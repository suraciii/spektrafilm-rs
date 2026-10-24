// Per-pixel constant multiply for a retained image buffer.
//
// The reference applies the metered exposure scale to the uploaded samples
// (`apply_autoexposure`), so this pass multiplies every sample by one
// image-independent scalar and nothing else.

struct Params {
    n_pixels: u32,
    scale: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> data: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels {
        return;
    }
    let base = idx * 3u;
    data[base] = data[base] * params.scale;
    data[base + 1u] = data[base + 1u] * params.scale;
    data[base + 2u] = data[base + 2u] * params.scale;
}
