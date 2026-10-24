// CPU exponential_filter_channel: result starts at zero, then += amp * component.
struct Params {
    n_pixels: u32,
    scale: f32,
    clear_first: u32,
    _pad: u32,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<f32>;
@group(0) @binding(2) var<storage, read_write> dst: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= params.n_pixels { return; }
    for (var c = 0u; c < 3u; c += 1u) {
        let i = pixel * 3u + c;
        var previous = 0.0;
        if params.clear_first == 0u { previous = dst[i]; }
        dst[i] = previous + params.scale * src[i];
    }
}
