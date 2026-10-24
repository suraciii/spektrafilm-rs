// Per-channel `dst[i,c] = clear_first ? scale[c] * src[i,c]
//                                   : dst[i,c] + scale[c] * src[i,c]`.
//
// Used by halation's scatter accumulation (tail components) and final
// additive step. The amplitude `a_tot[c] = halation_strength[c] *
// halation_amount` differs per channel: Portra is (0.05, 0.015, 0.0), so
// averaging the scalar produces a uniform cool halo around bright edges.

struct Params {
    n_pixels: u32,
    clear_first: u32,
    _pad1: u32,
    _pad2: u32,
    scale: vec4<f32>, // .xyz used (per channel), .w padding
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<f32>;
@group(0) @binding(2) var<storage, read_write> dst: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels { return; }
    let base = idx * 3u;
    let s = params.scale.xyz;
    if params.clear_first != 0u {
        dst[base]      = s.x * src[base];
        dst[base + 1u] = s.y * src[base + 1u];
        dst[base + 2u] = s.z * src[base + 2u];
    } else {
        dst[base]      = dst[base]      + s.x * src[base];
        dst[base + 1u] = dst[base + 1u] + s.y * src[base + 1u];
        dst[base + 2u] = dst[base + 2u] + s.z * src[base + 2u];
    }
}
