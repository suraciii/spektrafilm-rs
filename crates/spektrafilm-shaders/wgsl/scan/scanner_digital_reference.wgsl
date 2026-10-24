// Digital Reference conversion: the prepared reference model applied to a
// developed negative density image.
//
// CPU equivalent: `DigitalReference::convert` in `digital_reference.rs`,
// including `capture`, `coordinates`, and `tone`.
//
// Per pixel:
//   1. capture:    log10(sum_i 10^(log_weights[i][c] - density_i)), where
//                  density_i = base[i] + cmy · dyes[i]. A wavelength whose
//                  base or dye row is NaN contributes nothing (the host
//                  marks it with a NaN weight row).
//   2. coordinates: optical = clear - capture; monochrome takes the mean of
//                  the three, colour applies the common separation inverse.
//   3. response:   sign(x) * x^2 / (x^2 + 41/9) on x = coordinate / anchor,
//                  scaled by the per-channel gain.
//   4. output:     linear sRGB → output space.
//
// The capture runs in binary64 because the reference's log-sum-exp must
// stay finite for extreme densities; the shared explicit f64 helpers avoid
// the driver's broken extended log/exp builtins. Every constant is
// host-resolved, so the shader neither refits nor renormalizes.
//
// The uniform struct is a single `[f32; 4]` head and everything else is a
// storage buffer: `vec4<f64>` and `mat3x3<f64>` in a uniform block would put
// the host's `[f64; 4]`/`[[f64; 3]; 3]` members at offsets the WGSL layout
// does not agree with, while storage buffers are tightly packed.

struct Params {
    n_pixels: u32,
    n_wavelengths: u32,
    monochrome: u32,
    _pad0: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
// Tightly packed f64 storage: `[f64; 3]` clear, `[f64; 3]` anchor, `[f64; 3]`
// gains, then the two 3×3 matrices row-major.
@group(0) @binding(1) var<storage, read> model: array<f64>;
@group(0) @binding(2) var<storage, read> density_cmy: array<f32>;   // [H*W*3]
@group(0) @binding(3) var<storage, read> log_weights: array<f64>;   // [N_WL*3]
@group(0) @binding(4) var<storage, read> dyes: array<f64>;          // [N_WL*3]
@group(0) @binding(5) var<storage, read> base: array<f64>;          // [N_WL]
@group(0) @binding(6) var<storage, read_write> output: array<f32>;  // [H*W*3]

const MODEL_CLEAR: u32 = 0u;
const MODEL_ANCHOR: u32 = 3u;
const MODEL_GAINS: u32 = 6u;
const MODEL_INVERSE: u32 = 9u;   // 9 values
const MODEL_TO_OUTPUT: u32 = 18u; // 9 values

/// `10^x` for the log-sum-exp terms, evaluated through the shared explicit
/// binary64 exponential so the driver's broken native `exp` is not used.
/// `x * ln(10)` is the reference's `10.0f64.powf(x)`.
fn pow10_f64(x: f64) -> f64 {
    return sf64_exp(x * 2.302585092994046);
}

/// One channel's base-10 log-sum-exp, mirroring the reference's
/// `DigitalReference::capture`: the maximum exponent, then
/// `maximum + log10(sum(10^(e - maximum)))`.
fn capture_channel(cmy: vec3<f64>, channel: u32) -> f64 {
    var maximum: f64 = -1.0e300;
    for (var wl = 0u; wl < params.n_wavelengths; wl = wl + 1u) {
        let base_d = base[wl];
        if base_d != base_d {
            continue;
        }
        let dye_base = wl * 3u;
        let dye_r = dyes[dye_base];
        let dye_g = dyes[dye_base + 1u];
        let dye_b = dyes[dye_base + 2u];
        if dye_r != dye_r || dye_g != dye_g || dye_b != dye_b {
            continue;
        }
        let weight = log_weights[dye_base + channel];
        if weight != weight {
            continue;
        }
        let exponent = weight - (base_d + cmy.x * dye_r + cmy.y * dye_g + cmy.z * dye_b);
        if exponent > maximum {
            maximum = exponent;
        }
    }
    if maximum <= -1.0e300 {
        // Every wavelength is missing: the reference's all `-inf` exponents
        // reduce to this same degenerate value.
        return maximum;
    }
    var sum: f64 = 0.0;
    for (var wl = 0u; wl < params.n_wavelengths; wl = wl + 1u) {
        let base_d = base[wl];
        if base_d != base_d {
            continue;
        }
        let dye_base = wl * 3u;
        let dye_r = dyes[dye_base];
        let dye_g = dyes[dye_base + 1u];
        let dye_b = dyes[dye_base + 2u];
        if dye_r != dye_r || dye_g != dye_g || dye_b != dye_b {
            continue;
        }
        let weight = log_weights[dye_base + channel];
        if weight != weight {
            continue;
        }
        sum = sum + pow10_f64(weight - (base_d + cmy.x * dye_r + cmy.y * dye_g + cmy.z * dye_b) - maximum);
    }
    return maximum + sf64_log(sum) / 2.302585092994046;
}

/// The common signed response: `sign(x) * x^2 / (x^2 + 41/9)`, with the
/// reciprocal form outside |x| = 1 so no square overflows. The sign comes
/// from `x` itself, so no untyped literal decides the f64 type.
fn reference_tone(x: f64) -> f64 {
    let a = abs(x);
    let ratio = select(
        1.0 / (1.0 + (41.0 / 9.0) / a / a),
        a * a / (a * a + 41.0 / 9.0),
        a > 1.0,
    );
    return (x / a) * ratio;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= params.n_pixels {
        return;
    }
    let base = pixel * 3u;
    let cmy = vec3<f64>(
        f64(density_cmy[base]),
        f64(density_cmy[base + 1u]),
        f64(density_cmy[base + 2u]),
    );
    var capture: vec3<f64>;
    for (var c = 0u; c < 3u; c = c + 1u) {
        capture[c] = capture_channel(cmy, c);
    }
    let optical = vec3<f64>(
        model[MODEL_CLEAR] - capture.x,
        model[MODEL_CLEAR + 1u] - capture.y,
        model[MODEL_CLEAR + 2u] - capture.z,
    );
    var coordinates: vec3<f64>;
    if params.monochrome != 0u {
        let mean = (optical.x + optical.y + optical.z) / 3.0;
        coordinates = vec3<f64>(mean, mean, mean);
    } else {
        // The separation inverse acts on the optical capture.
        for (var r = 0u; r < 3u; r = r + 1u) {
            coordinates[r] = model[MODEL_INVERSE + r * 3u] * optical.x
                + model[MODEL_INVERSE + r * 3u + 1u] * optical.y
                + model[MODEL_INVERSE + r * 3u + 2u] * optical.z;
        }
    }
    var response: vec3<f64>;
    for (var c = 0u; c < 3u; c = c + 1u) {
        response[c] = reference_tone(coordinates[c] / model[MODEL_ANCHOR + c])
            * model[MODEL_GAINS + c];
    }
    for (var r = 0u; r < 3u; r = r + 1u) {
        output[base + r] = f32(model[MODEL_TO_OUTPUT + r * 3u] * response.x
            + model[MODEL_TO_OUTPUT + r * 3u + 1u] * response.y
            + model[MODEL_TO_OUTPUT + r * 3u + 2u] * response.z);
    }
}
