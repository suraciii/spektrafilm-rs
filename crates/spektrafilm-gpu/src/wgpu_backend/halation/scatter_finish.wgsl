// Halation scatter blend.
//
// Per pixel and channel c:
//   result[i,c] = (1 - amount[c]) * result[i,c] + amount[c] * scatter[i,c]
//
// `amount` is per channel: the reference skips a channel whose core and tail
// widths are both non-positive, and a zero amount here reproduces that exactly.
// `result` is read-modify-written in place.

struct Params {
    n_pixels: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    amount: vec4<f32>, // .xyz used (per channel), .w padding
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> scatter: array<f32>;
@group(0) @binding(2) var<storage, read_write> result: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels { return; }
    let base = idx * 3u;
    let sa = params.amount.xyz;
    for (var c = 0u; c < 3u; c++) {
        let r = result[base + c];
        result[base + c] = (1.0 - sa[c]) * r + sa[c] * scatter[base + c];
    }
}
