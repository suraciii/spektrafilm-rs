// Scanner PCHIP LUT: per-pixel 3-D monotone-cubic Hermite interpolation of
// a log10-XYZ lattice, then the post-LUT carry to output RGB.
//
// CPU equivalent: `scan_spectral_via_lut` in `stages::scanning`, which
// normalizes the input onto the LUT axes, calls `pchip_interp` from
// `spektrafilm_math::pchip3d`, and then applies
// `10^log_xyz → white/black luminance remap → CAT → XYZ→RGB`.
//
// The host uploads the same prepared lattice the reference interpolation
// uses: corner values, per-axis monotone slopes, and per-cell corner-hull
// bounds. The shader performs the same Hermite walk — a tensor-product
// cubic along each axis with the orthogonal slopes linearly mixed — and the
// same soft monotonicity clamp to the cell's corner hull.
//
// The post-LUT step is not an approximation of the CPU order: the CPU
// evaluates `10^x` on each interpolated log-XYZ component, applies the
// luminance remap as a single scalar scale on the pre-CAT XYZ (skipped when
// disabled), and then applies the two matrices in sequence.

struct Params {
    n_pixels: u32,
    size: u32,
    size_m1: u32,
    bw_enable: u32,
    data_min: vec3<f32>,
    data_scale: vec3<f32>,
    bw: vec2<f32>,
    cat: mat3x3<f32>,
    xyz_to_rgb: mat3x3<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;    // [H*W*3]
@group(0) @binding(2) var<storage, read> lut: array<f32>;      // [size^3 * 3]
@group(0) @binding(3) var<storage, read> slope_x: array<f32>;
@group(0) @binding(4) var<storage, read> slope_y: array<f32>;
@group(0) @binding(5) var<storage, read> slope_z: array<f32>;
@group(0) @binding(6) var<storage, read> cell_min: array<f32>;
@group(0) @binding(7) var<storage, read> cell_max: array<f32>;
@group(0) @binding(8) var<storage, read_write> output: array<f32>;

fn lut_index(i: u32, j: u32, k: u32, c: u32) -> u32 {
    return ((i * params.size + j) * params.size + k) * 3u + c;
}

fn cell_index(i: u32, j: u32, k: u32, c: u32) -> u32 {
    return ((i * params.size_m1 + j) * params.size_m1 + k) * 3u + c;
}

fn hermite(y0: f32, y1: f32, m0: f32, m1: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    return h00 * y0 + h10 * m0 + h01 * y1 + h11 * m1;
}

fn linear_mix(v0: f32, v1: f32, t: f32) -> f32 {
    return v0 + t * (v1 - v0);
}

fn bilinear_mix(v00: f32, v10: f32, v01: f32, v11: f32, tx: f32, ty: f32) -> f32 {
    let vx0 = linear_mix(v00, v10, tx);
    let vx1 = linear_mix(v01, v11, tx);
    return linear_mix(vx0, vx1, ty);
}

// LUT-axis coordinate in [0, size-1] to (base index, fraction).
fn base_fraction(coord: f32) -> vec2<f32> {
    let upper = f32(params.size - 1u);
    let c = clamp(coord, 0.0, upper);
    if c >= upper {
        return vec2<f32>(upper - 1.0, 1.0);
    }
    let base = floor(c);
    return vec2<f32>(base, c - base);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= params.n_pixels {
        return;
    }
    let base = pixel * 3u;

    // Normalize the input onto the LUT axes, matching the reference
    // `(x - xmin) * scale / (xmax - xmin)` followed by `* (size - 1)`.
    let r = base_fraction((input[base] - params.data_min.x) * params.data_scale.x);
    let g = base_fraction((input[base + 1u] - params.data_min.y) * params.data_scale.y);
    let b = base_fraction((input[base + 2u] - params.data_min.z) * params.data_scale.z);

    let i = u32(r.x);
    let j = u32(g.x);
    let k = u32(b.x);
    let tr = r.y;
    let tg = g.y;
    let tb = b.y;

    var log_xyz = vec3<f32>(0.0, 0.0, 0.0);
    for (var c = 0u; c < 3u; c++) {
        // Hermite along x at the four (y, z) corner positions of the cell.
        let v000 = hermite(
            lut[lut_index(i, j, k, c)],
            lut[lut_index(i + 1u, j, k, c)],
            slope_x[lut_index(i, j, k, c)],
            slope_x[lut_index(i + 1u, j, k, c)],
            tr,
        );
        let v010 = hermite(
            lut[lut_index(i, j + 1u, k, c)],
            lut[lut_index(i + 1u, j + 1u, k, c)],
            slope_x[lut_index(i, j + 1u, k, c)],
            slope_x[lut_index(i + 1u, j + 1u, k, c)],
            tr,
        );
        let v001 = hermite(
            lut[lut_index(i, j, k + 1u, c)],
            lut[lut_index(i + 1u, j, k + 1u, c)],
            slope_x[lut_index(i, j, k + 1u, c)],
            slope_x[lut_index(i + 1u, j, k + 1u, c)],
            tr,
        );
        let v011 = hermite(
            lut[lut_index(i, j + 1u, k + 1u, c)],
            lut[lut_index(i + 1u, j + 1u, k + 1u, c)],
            slope_x[lut_index(i, j + 1u, k + 1u, c)],
            slope_x[lut_index(i + 1u, j + 1u, k + 1u, c)],
            tr,
        );

        // Mix orthogonal-axis slopes linearly along x.
        let sy00 = linear_mix(
            slope_y[lut_index(i, j, k, c)],
            slope_y[lut_index(i + 1u, j, k, c)],
            tr,
        );
        let sy10 = linear_mix(
            slope_y[lut_index(i, j + 1u, k, c)],
            slope_y[lut_index(i + 1u, j + 1u, k, c)],
            tr,
        );
        let sy01 = linear_mix(
            slope_y[lut_index(i, j, k + 1u, c)],
            slope_y[lut_index(i + 1u, j, k + 1u, c)],
            tr,
        );
        let sy11 = linear_mix(
            slope_y[lut_index(i, j + 1u, k + 1u, c)],
            slope_y[lut_index(i + 1u, j + 1u, k + 1u, c)],
            tr,
        );

        // Hermite along y at the two z positions.
        let vz0 = hermite(v000, v010, sy00, sy10, tg);
        let vz1 = hermite(v001, v011, sy01, sy11, tg);

        // Bilinear mix of slope_z to the (tr, tg) point at both z corners.
        let sz0 = bilinear_mix(
            slope_z[lut_index(i, j, k, c)],
            slope_z[lut_index(i + 1u, j, k, c)],
            slope_z[lut_index(i, j + 1u, k, c)],
            slope_z[lut_index(i + 1u, j + 1u, k, c)],
            tr,
            tg,
        );
        let sz1 = bilinear_mix(
            slope_z[lut_index(i, j, k + 1u, c)],
            slope_z[lut_index(i + 1u, j, k + 1u, c)],
            slope_z[lut_index(i, j + 1u, k + 1u, c)],
            slope_z[lut_index(i + 1u, j + 1u, k + 1u, c)],
            tr,
            tg,
        );

        // Final Hermite along z, then clamp to the cell corner hull.
        let interp = hermite(vz0, vz1, sz0, sz1, tb);
        let mn = cell_min[cell_index(i, j, k, c)];
        let mx = cell_max[cell_index(i, j, k, c)];
        log_xyz[c] = clamp(interp, mn, mx);
    }

    // Post-LUT: 10^log_xyz, the white/black luminance remap on the
    // pre-CAT XYZ, then the two matrices in the reference's order.
    var xyz = vec3<f32>(
        pow(10.0, log_xyz.x),
        pow(10.0, log_xyz.y),
        pow(10.0, log_xyz.z),
    );
    if params.bw_enable != 0u {
        // Mirrors ColorReference::xyz_scale —
        // clip(m*Y + q, 0, 1)/(Y + 1e-10) on the pre-CAT scan XYZ.
        let scale = clamp(params.bw.x * xyz.y + params.bw.y, 0.0, 1.0) / (xyz.y + 1e-10);
        xyz = xyz * scale;
    }
    let adapted = params.cat * xyz;
    let rgb = params.xyz_to_rgb * adapted;

    output[base] = rgb.x;
    output[base + 1u] = rgb.y;
    output[base + 2u] = rgb.z;
}
