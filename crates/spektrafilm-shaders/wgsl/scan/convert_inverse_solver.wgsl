// Convert-film inverse scan: the bounded Gauss-Newton solve that maps scanned
// RGB back to film CMY density, per pixel.
//
// CPU equivalent: `Converter::invert` in `stages/converting.rs`, with the
// forward model of `Converter::forward_jacobian` evaluated over the same
// wavelength tables the reference integrates. Every bounded,
// image-independent quantity is resolved on the host and arrives as a
// parameter; the shader performs no re-derivation:
//
//   goal[c]     = (calibration · decode(rgb))[c] * gain
//   cmy         = clamp(cmy_max * 0.5 + seed_inverse · (seed_log_rgb -
//                 log10(max(goal[c], 1e-12))), 0, cmy_max)
//   repeat 8 times:
//     rgb, jac  = forward_jacobian(cmy)
//     residual  = rgb - goal
//     break when max |residual| < 1e-6
//     solve (JᵀJ + 1e-7·I) δ = Jᵀ residual by partial-pivot elimination
//     cmy       = clamp(cmy - δ, 0, cmy_max)
//
// The forward model integrates the transmitted light over the wavelength
// grid: `density = base + cmy · dye`, `light = illuminant · 10^-density`,
// `xyz = Σ light · cmf / normalization`, and the analytic derivative
// `dxyz[ch][k] = -ln(10) · Σ light · dye[ch] · cmf[k] / normalization`.
//
// Precision: the reference runs the solve in f64, and the residual break
// at 1e-6 plus the clamped update are discrete decisions that f32 rounding
// would move, so the whole solver runs in f64 (the same explicit
// `f64_math.wgsl` arithmetic the Spektra v1 pass uses: the driver's
// extended log/exp builtins are unusable on some devices, and `10^-x` is
// `sf64_exp(-x · ln 10)`).
//
// Layout: no function-local f64 arrays. Some Vulkan drivers (Mesa llvmpipe)
// lose the device on a dynamically indexed local constant `array<f64, N>`
// inside a nested loop, so the 3×3 normal equations, the 3-vector residual,
// and the Jacobian columns are all named scalars / vectors and the pivoting
// is written out. Storage and uniform buffers are unaffected.

// The residual break and the damping are the reference's own constants; the
// iteration count arrives as a parameter so the host states it explicitly.
const RESIDUAL_BREAK: f64 = 1e-6;
const DAMPING: f64 = 1e-7;
const PIVOT_EPSILON: f64 = 1e-14;

struct Params {
    n_pixels: u32,
    n_wl: u32,
    iterations: u32,
    cctf_kind: u32,
    gain: f64,
    normalization: f64,
    // WGSL pads a vec3<f64> to a 4-lane row in the uniform address space;
    // the host uploads the same padded rows and the shader reads lanes 0..2.
    seed_log_rgb: vec4<f64>,
    seed_inverse: array<vec4<f64>, 3>,
    xyz_to_rgb: array<vec4<f64>, 3>,
    calibration: array<vec4<f64>, 3>,
    cmy_max: vec4<f64>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;           // [H*W*3]
@group(0) @binding(2) var<storage, read> dye: array<f64>;            // [N_WL*3]
@group(0) @binding(3) var<storage, read> base: array<f64>;           // [N_WL]
@group(0) @binding(4) var<storage, read> illuminant: array<f64>;     // [N_WL]
@group(0) @binding(5) var<storage, read> cmfs: array<f64>;           // [N_WL*3]
@group(0) @binding(6) var<storage, read_write> output: array<f32>;   // [H*W*3]

// The reference's `LN_10` and `10^-x` as the reference evaluates them.
const LN10: f64 = 2.302585092994046;
const NEG_LN10: f64 = -2.302585092994046;

fn pow10(value: f64) -> f64 {
    // The reference's `10.0f64.powf(-density)`; the explicit exp keeps the
    // driver's extended pow out of the decision path.
    return sf64_exp(value * LN10);
}

/// `forward_jacobian`: the transmitted-light integral and its analytic
/// derivative. `rgb` is the predicted linear RGB; `j0`, `j1`, `j2` are
/// `∂rgb/∂cmy[ch]`, so the caller reads `jk[c]` as the reference's
/// `jac[c][k]`.
struct Forward {
    rgb: vec3<f64>,
    j0: vec3<f64>,
    j1: vec3<f64>,
    j2: vec3<f64>,
}

fn forward_jacobian(cmy: vec3<f64>) -> Forward {
    var xyz = vec3<f64>(0.0, 0.0, 0.0);
    // dxyz[k] for ch = 0..2, as named vectors (no local array).
    var d0 = vec3<f64>(0.0, 0.0, 0.0);
    var d1 = vec3<f64>(0.0, 0.0, 0.0);
    var d2 = vec3<f64>(0.0, 0.0, 0.0);
    let n = params.n_wl;
    for (var wl = 0u; wl < n; wl = wl + 1u) {
        let base_wl = base[wl];
        let dye_base = wl * 3u;
        let dye_r = dye[dye_base];
        let dye_g = dye[dye_base + 1u];
        let dye_b = dye[dye_base + 2u];
        // A nonfinite table entry contributes no light at all, exactly as
        // the reference's `if !light.is_finite() { continue; }`.
        if base_wl != base_wl || dye_r != dye_r || dye_g != dye_g || dye_b != dye_b {
            continue;
        }
        let density = base_wl + cmy.x * dye_r + cmy.y * dye_g + cmy.z * dye_b;
        let light = illuminant[wl] * pow10(density * -1.0);
        if light != light {
            continue;
        }
        let cmf_base = wl * 3u;
        let cmf = vec3<f64>(cmfs[cmf_base], cmfs[cmf_base + 1u], cmfs[cmf_base + 2u]);
        xyz = xyz + light * cmf;
        // `dxyz[ch][k] = -ln(10) · light · dye[ch] · cmf[k]`: the dye factor
        // is the derivative channel's scalar, replicated across k.
        let scale = light * NEG_LN10;
        d0 = d0 + scale * vec3<f64>(dye_r, dye_r, dye_r) * cmf;
        d1 = d1 + scale * vec3<f64>(dye_g, dye_g, dye_g) * cmf;
        d2 = d2 + scale * vec3<f64>(dye_b, dye_b, dye_b) * cmf;
    }
    // The reference divides both the integral and every derivative row by
    // the normalization, so the Jacobian is the normalized gradient and not
    // the raw one.
    let inv_norm = 1.0 / params.normalization;
    xyz = xyz * inv_norm;
    d0 = d0 * inv_norm;
    d1 = d1 * inv_norm;
    d2 = d2 * inv_norm;
    // rgb = xyz_to_rgb · xyz, and jk = xyz_to_rgb · dk.
    let row0 = vec3<f64>(params.xyz_to_rgb[0][0], params.xyz_to_rgb[0][1], params.xyz_to_rgb[0][2]);
    let row1 = vec3<f64>(params.xyz_to_rgb[1][0], params.xyz_to_rgb[1][1], params.xyz_to_rgb[1][2]);
    let row2 = vec3<f64>(params.xyz_to_rgb[2][0], params.xyz_to_rgb[2][1], params.xyz_to_rgb[2][2]);
    // Explicit left-to-right sums keep the reference's accumulation order;
    // `dot` is not guaranteed to group the same way.
    var out: Forward;
    out.rgb = vec3<f64>(
        row0.x * xyz.x + row0.y * xyz.y + row0.z * xyz.z,
        row1.x * xyz.x + row1.y * xyz.y + row1.z * xyz.z,
        row2.x * xyz.x + row2.y * xyz.y + row2.z * xyz.z,
    );
    out.j0 = vec3<f64>(
        row0.x * d0.x + row0.y * d0.y + row0.z * d0.z,
        row1.x * d0.x + row1.y * d0.y + row1.z * d0.z,
        row2.x * d0.x + row2.y * d0.y + row2.z * d0.z,
    );
    out.j1 = vec3<f64>(
        row0.x * d1.x + row0.y * d1.y + row0.z * d1.z,
        row1.x * d1.x + row1.y * d1.y + row1.z * d1.z,
        row2.x * d1.x + row2.y * d1.y + row2.z * d1.z,
    );
    out.j2 = vec3<f64>(
        row0.x * d2.x + row0.y * d2.y + row0.z * d2.z,
        row1.x * d2.x + row1.y * d2.y + row1.z * d2.z,
        row2.x * d2.x + row2.y * d2.y + row2.z * d2.z,
    );
    return out;
}

/// `solve3`: partial-pivot Gaussian elimination with back substitution over
/// nine named coefficients. A pivot below the tolerance yields a zero
/// update, as in the reference.
fn solve3(a00: f64, a01: f64, a02: f64,
          a10: f64, a11: f64, a12: f64,
          a20: f64, a21: f64, a22: f64,
          b0: f64, b1: f64, b2: f64) -> vec3<f64> {
    var m00 = a00; var m01 = a01; var m02 = a02;
    var m10 = a10; var m11 = a11; var m12 = a12;
    var m20 = a20; var m21 = a21; var m22 = a22;
    var v0 = b0; var v1 = b1; var v2 = b2;

    // ── column 0: pivot over rows 0..2 ──────────────────────────────────
    // The reference scans `row` upward and compares against the running
    // pivot, so the comparison target follows the selection.
    var p = 0u;
    if abs(m10) > abs(m00) {
        p = 1u;
    }
    let pivot0 = select(m00, m10, p == 1u);
    if abs(m20) > abs(pivot0) {
        p = 2u;
    }
    let pivot_value = select(select(m00, m10, p == 1u), m20, p == 2u);
    if abs(pivot_value) < PIVOT_EPSILON {
        return vec3<f64>(0.0, 0.0, 0.0);
    }
    if p == 1u {
        let t00 = m00; m00 = m10; m10 = t00;
        let t01 = m01; m01 = m11; m11 = t01;
        let t02 = m02; m02 = m12; m12 = t02;
        let tv = v0; v0 = v1; v1 = tv;
    } else if p == 2u {
        let t00 = m00; m00 = m20; m20 = t00;
        let t01 = m01; m01 = m21; m21 = t01;
        let t02 = m02; m02 = m22; m22 = t02;
        let tv = v0; v0 = v2; v2 = tv;
    }
    // Elimination rows 1 and 2 read the (already swapped) pivot row m0*.
    let f1 = m10 / m00;
    m10 = m10 - f1 * m00;
    m11 = m11 - f1 * m01;
    m12 = m12 - f1 * m02;
    v1 = v1 - f1 * v0;
    let f2 = m20 / m00;
    m20 = m20 - f2 * m00;
    m21 = m21 - f2 * m01;
    m22 = m22 - f2 * m02;
    v2 = v2 - f2 * v0;

    // ── column 1: pivot over rows 1..2 ──────────────────────────────────
    if abs(m21) > abs(m11) {
        let t10 = m10; m10 = m20; m20 = t10;
        let t11 = m11; m11 = m21; m21 = t11;
        let t12 = m12; m12 = m22; m22 = t12;
        let tv = v1; v1 = v2; v2 = tv;
    }
    if abs(m11) < PIVOT_EPSILON {
        return vec3<f64>(0.0, 0.0, 0.0);
    }
    let f3 = m21 / m11;
    m21 = m21 - f3 * m11;
    m22 = m22 - f3 * m12;
    v2 = v2 - f3 * v1;

    // ── column 2 ────────────────────────────────────────────────────────
    if abs(m22) < PIVOT_EPSILON {
        return vec3<f64>(0.0, 0.0, 0.0);
    }

    // Back substitution, last row first, summing the tail in ascending k
    // order exactly as the reference does.
    let x2 = (v2 - 0.0) / m22;
    let x1 = (v1 - m12 * x2) / m11;
    let x0 = (v0 - (m01 * x1 + m02 * x2)) / m00;
    return vec3<f64>(x0, x1, x2);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= params.n_pixels {
        return;
    }
    let base_index = pixel * 3u;
    // The reference decodes the input transfer before solving, so the device
    // applies the same `cctf_decode` to the uploaded samples. A linear input
    // space (kind 0) and a disabled decode both leave the samples untouched,
    // matching the reference exactly in both cases.
    var rgb = vec3<f64>(f64(input[base_index]), f64(input[base_index + 1u]), f64(input[base_index + 2u]));
    if params.cctf_kind != 0u {
        rgb = vec3<f64>(
            f64(decode_one(input[base_index], params.cctf_kind)),
            f64(decode_one(input[base_index + 1u], params.cctf_kind)),
            f64(decode_one(input[base_index + 2u], params.cctf_kind)),
        );
    }

    // goal = (calibration · rgb) * gain, matching the reference's
    // column-oriented accumulation.
    let gain = params.gain;
    let goal = vec3<f64>(
        (rgb.x * params.calibration[0][0]
            + rgb.y * params.calibration[1][0]
            + rgb.z * params.calibration[2][0]) * gain,
        (rgb.x * params.calibration[0][1]
            + rgb.y * params.calibration[1][1]
            + rgb.z * params.calibration[2][1]) * gain,
        (rgb.x * params.calibration[0][2]
            + rgb.y * params.calibration[1][2]
            + rgb.z * params.calibration[2][2]) * gain,
    );

    // Seed: `cmy_max * 0.5 + seed_inverse · (seed_log_rgb - log10(goal))`,
    // clamped to the bounded box.
    // The reference's seed is `seed_log_rgb - target.log10()`, so the
    // logarithm is base 10: `sf64_log` is natural, and the ratio below is
    // the reference's own `log10`.
    let log_delta = vec3<f64>(
        params.seed_log_rgb[0] - sf64_log(max(goal.x, f64(1e-12))) / 2.302585092994046,
        params.seed_log_rgb[1] - sf64_log(max(goal.y, f64(1e-12))) / 2.302585092994046,
        params.seed_log_rgb[2] - sf64_log(max(goal.z, f64(1e-12))) / 2.302585092994046,
    );
    var cmy = vec3<f64>(0.0, 0.0, 0.0);
    cmy.x = clamp(
        params.cmy_max[0] * 0.5
            + params.seed_inverse[0][0] * log_delta.x
            + params.seed_inverse[0][1] * log_delta.y
            + params.seed_inverse[0][2] * log_delta.z,
        f64(0.0),
        params.cmy_max[0],
    );
    cmy.y = clamp(
        params.cmy_max[1] * 0.5
            + params.seed_inverse[1][0] * log_delta.x
            + params.seed_inverse[1][1] * log_delta.y
            + params.seed_inverse[1][2] * log_delta.z,
        f64(0.0),
        params.cmy_max[1],
    );
    cmy.z = clamp(
        params.cmy_max[2] * 0.5
            + params.seed_inverse[2][0] * log_delta.x
            + params.seed_inverse[2][1] * log_delta.y
            + params.seed_inverse[2][2] * log_delta.z,
        f64(0.0),
        params.cmy_max[2],
    );

    for (var iteration = 0u; iteration < params.iterations; iteration = iteration + 1u) {
        let f = forward_jacobian(cmy);
        let r0 = f.rgb.x - goal.x;
        let r1 = f.rgb.y - goal.y;
        let r2 = f.rgb.z - goal.z;
        if max(max(abs(r0), abs(r1)), abs(r2)) < RESIDUAL_BREAK {
            break;
        }
        // Normal equations: jtj[col][row] = Σ_c jac[col][c] · jac[row][c],
        // jtr[col] = Σ_c jac[col][c] · residual[c]. `jk` is column k, so
        // these are the inner products of the Jacobian columns, accumulated
        // in the reference's channel order.
        let a00 = (f.j0.x * f.j0.x + f.j0.y * f.j0.y + f.j0.z * f.j0.z) + DAMPING;
        let a01 = f.j0.x * f.j1.x + f.j0.y * f.j1.y + f.j0.z * f.j1.z;
        let a02 = f.j0.x * f.j2.x + f.j0.y * f.j2.y + f.j0.z * f.j2.z;
        let a11 = (f.j1.x * f.j1.x + f.j1.y * f.j1.y + f.j1.z * f.j1.z) + DAMPING;
        let a12 = f.j1.x * f.j2.x + f.j1.y * f.j2.y + f.j1.z * f.j2.z;
        let a22 = (f.j2.x * f.j2.x + f.j2.y * f.j2.y + f.j2.z * f.j2.z) + DAMPING;
        let jtr0 = f.j0.x * r0 + f.j0.y * r1 + f.j0.z * r2;
        let jtr1 = f.j1.x * r0 + f.j1.y * r1 + f.j1.z * r2;
        let jtr2 = f.j2.x * r0 + f.j2.y * r1 + f.j2.z * r2;
        // jtj is symmetric, so the transposed columns read the same.
        let delta = solve3(
            a00, a01, a02,
            a01, a11, a12,
            a02, a12, a22,
            jtr0, jtr1, jtr2,
        );
        cmy = vec3<f64>(
            clamp(cmy.x - delta.x, f64(0.0), params.cmy_max[0]),
            clamp(cmy.y - delta.y, f64(0.0), params.cmy_max[1]),
            clamp(cmy.z - delta.z, f64(0.0), params.cmy_max[2]),
        );
    }

    output[base_index] = f32(cmy.x);
    output[base_index + 1u] = f32(cmy.y);
    output[base_index + 2u] = f32(cmy.z);
}
