// Modeled scanner receiver capture: transmitted light integrated through
// the receiver's normalized band weights, assigned to the output space.
//
// CPU equivalent: `ScanReceiver::capture` / `capture_f64` in
// `scan_receiver.rs`. Per pixel and per wavelength:
//   d     = cmy · channel_density[wl] + base_density[wl]
//   light = 10^-d                       (NaN density → 0)
//   reading[c] += light * weights[wl][c]
// then `rgb = to_output · reading`.
//
// The weights arrive already multiplied by the illuminant and normalized so
// each band sums to one, exactly as the reference resolves them, so the
// shader performs no renormalization.

struct Params {
    n_pixels: u32,
    n_wavelengths: u32,
    has_base: u32,
    _pad: u32,
    to_output: mat3x3<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> density_cmy: array<f32>;    // [H*W*3]
@group(0) @binding(2) var<storage, read> weights: array<f32>;        // [N_WL*3]
@group(0) @binding(3) var<storage, read> channel_density: array<f32>; // [N_WL*3]
@group(0) @binding(4) var<storage, read> base_density: array<f32>;   // [N_WL]
@group(0) @binding(5) var<storage, read_write> output: array<f32>;   // [H*W*3]

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= params.n_pixels {
        return;
    }
    let base = pixel * 3u;
    let cmy_r = density_cmy[base];
    let cmy_g = density_cmy[base + 1u];
    let cmy_b = density_cmy[base + 2u];

    var reading = vec3<f32>(0.0, 0.0, 0.0);
    for (var wl = 0u; wl < params.n_wavelengths; wl++) {
        let cd_base = wl * 3u;
        // Same NaN handling as the reference: a nonfinite dye row or base
        // density contributes no light at all.
        let cd_r = channel_density[cd_base];
        let cd_g = channel_density[cd_base + 1u];
        let cd_b = channel_density[cd_base + 2u];
        let bd = base_density[wl];
        if cd_r != cd_r || cd_g != cd_g || cd_b != cd_b || bd != bd {
            continue;
        }
        let d = cmy_r * cd_r + cmy_g * cd_g + cmy_b * cd_b + bd;
        let light = pow(10.0, -d);
        let w_base = wl * 3u;
        reading.x += light * weights[w_base];
        reading.y += light * weights[w_base + 1u];
        reading.z += light * weights[w_base + 2u];
    }

    let rgb = params.to_output * reading;
    output[base] = rgb.x;
    output[base + 1u] = rgb.y;
    output[base + 2u] = rgb.z;
}
