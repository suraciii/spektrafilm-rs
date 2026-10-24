// CPU DIR blends the completed exponential tail, not weighted components.
struct Params {
    n_pixels: u32,
    scale: f32,
    _pad0: u32,
    _pad1: u32,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> tail: array<f32>;
@group(0) @binding(2) var<storage, read_write> gaussian: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= params.n_pixels { return; }
    for (var c = 0u; c < 3u; c += 1u) {
        let i = pixel * 3u + c;
        gaussian[i] = (1.0 - params.scale) * gaussian[i] + params.scale * tail[i];
    }
}
