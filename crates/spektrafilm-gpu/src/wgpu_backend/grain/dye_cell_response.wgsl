// Formed-cell CMY -> linear route response. Subtract the film base offset
// before optics; fractional readout must not subtract it a second time.
struct Params {
    dims: vec4<u32>, // cells, wavelengths, print=0/scan=1, unused
    controls: vec4<f32>, // normalization, print exposure scale
    preflash: vec4<f32>,
    mins: vec4<f32>,
    matrix: mat3x3<f32>,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> formed: array<f32>;
// Per wavelength: CMY density (3), base, illuminant, response weights (3).
@group(0) @binding(2) var<storage, read> spectrum: array<f32>;
@group(0) @binding(3) var<storage, read_write> linear: array<f32>;
@group(0) @binding(4) var<storage, read_write> status: atomic<u32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let i = gid.x + gid.y * grid.x * 256u;
    if (i >= p.dims.x) { return; }
    let b = i * 3u;
    let cmy = vec3<f32>(formed[b], formed[b + 1u], formed[b + 2u]) - p.mins.xyz;
    var raw = vec3<f32>(0.0);
    for (var wl = 0u; wl < p.dims.y; wl++) {
        let s = wl * 8u;
        let d = dot(cmy, vec3<f32>(spectrum[s], spectrum[s + 1u], spectrum[s + 2u])) + spectrum[s + 3u];
        let light = pow(10.0, -d) * spectrum[s + 4u];
        raw += light * vec3<f32>(spectrum[s + 5u], spectrum[s + 6u], spectrum[s + 7u]);
    }
    var result: vec3<f32>;
    if (p.dims.z == 0u) {
        raw = raw * p.controls.x + p.preflash.xyz;
        // Match expose_linear_calibrated: retain inner floor/log/exp,
        // post-exposure scale, outer floor/log, then the linear conversion.
        let inner = log(max(raw, vec3<f32>(0.0)) + vec3<f32>(1e-10)) / log(10.0);
        let recovered = exp(inner * log(10.0)) * p.controls.y;
        let outer = log(max(recovered, vec3<f32>(0.0)) + vec3<f32>(1e-10)) / log(10.0);
        result = pow(vec3<f32>(10.0), outer);
    } else {
        result = p.matrix * (raw / p.controls.x);
    }
    if (any((bitcast<vec3<u32>>(result) & vec3<u32>(0x7fffffffu)) >= vec3<u32>(0x7f800000u))) {
        atomicOr(&status, 8u);
    }
    linear[b] = result.x;
    linear[b + 1u] = result.y;
    linear[b + 2u] = result.z;
}
