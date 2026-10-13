// Printing stage: film CMY density → print log-exposure via spectral path.
//
// For each pixel:
//   1. Spectral density from CMY + channel_density + base_density
//   2. Light transmittance = 10^(-density) * enlarger_illuminant
//   3. Raw = sum(light * print_sensitivity) per channel
//   4. Normalize by midgray factor
//   5. log10(raw)

struct Params {
    width: u32,
    height: u32,
    n_wavelengths: u32,
    normalization_factor: f32,
    // Post-inner-log linear multiplier: print exposure × BW correction.
    exposure_scale: f32,
    // Spatially-constant preflash exposure (per channel), added before log10.
    preflash: vec3<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> density_cmy: array<f32>;        // [H*W*3]
@group(0) @binding(2) var<storage, read> channel_density: array<f32>;    // [N_WL*3]
@group(0) @binding(3) var<storage, read> base_density: array<f32>;       // [N_WL]
@group(0) @binding(4) var<storage, read> illuminant: array<f32>;         // [N_WL]
@group(0) @binding(5) var<storage, read> sensitivity: array<f32>;        // [N_WL*3]
@group(0) @binding(6) var<storage, read_write> output: array<f32>;       // [H*W*3]

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel_idx = gid.x + gid.y * grid.x * 256u;
    let total_pixels = params.width * params.height;
    if pixel_idx >= total_pixels {
        return;
    }

    let base = pixel_idx * 3u;
    let cmy_r = density_cmy[base];
    let cmy_g = density_cmy[base + 1u];
    let cmy_b = density_cmy[base + 2u];

    var raw = vec3<f32>(0.0, 0.0, 0.0);

    for (var wl = 0u; wl < params.n_wavelengths; wl++) {
        let cd_base = wl * 3u;
        // CPU side guarantees no NaN values are uploaded — see wgpu_backend.rs
        // (`sanitize_f32` replaces NaN with 0 to match Python's `density_to_light` semantics).
        // Metal's compiler runs in fast-math mode and optimizes away `x != x` NaN checks,
        // so we cannot rely on in-shader NaN detection.
        let cd_r = channel_density[cd_base];
        let cd_g = channel_density[cd_base + 1u];
        let cd_b = channel_density[cd_base + 2u];
        let bd = base_density[wl];
        let d = cmy_r * cd_r + cmy_g * cd_g + cmy_b * cd_b + bd;
        let light = pow(10.0, -d) * illuminant[wl];
        raw.x += light * sensitivity[cd_base];
        raw.y += light * sensitivity[cd_base + 1u];
        raw.z += light * sensitivity[cd_base + 2u];
    }

    raw = raw * params.normalization_factor + params.preflash;

    // Preserve CPU `expose_calibrated`: inner floor/log, exp10, post-scale,
    // then outer floor/log. Keeping these stages explicit also preserves
    // behavior when the post-scale is not one.
    let inner = vec3<f32>(
        log(max(raw.x, 0.0) + 1e-10) / log(10.0),
        log(max(raw.y, 0.0) + 1e-10) / log(10.0),
        log(max(raw.z, 0.0) + 1e-10) / log(10.0),
    );
    let recovered = exp(inner * log(10.0)) * params.exposure_scale;
    output[base] = log(max(recovered.x, 0.0) + 1e-10) / log(10.0);
    output[base + 1u] = log(max(recovered.y, 0.0) + 1e-10) / log(10.0);
    output[base + 2u] = log(max(recovered.z, 0.0) + 1e-10) / log(10.0);
}
