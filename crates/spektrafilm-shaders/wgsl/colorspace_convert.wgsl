// Per-pixel 3x3 matrix multiply: RGB in, RGB out.
// Matches CPU: out[c] = sum_k matrix[c][k] * rgb[k]
// Matrix passed as 9 consecutive floats in row-major order (Rust [[f32;3];3]).

struct Params {
    width: u32,
    height: u32,
    m00: f32, m01: f32, m02: f32,
    m10: f32, m11: f32, m12: f32,
    m20: f32, m21: f32, m22: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;   // [H*W*3]
@group(0) @binding(2) var<storage, read_write> output: array<f32>; // [H*W*3]

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel_idx = gid.x + gid.y * grid.x * 256u;
    let total_pixels = params.width * params.height;
    if pixel_idx >= total_pixels {
        return;
    }

    let base = pixel_idx * 3u;
    let r = input[base];
    let g = input[base + 1u];
    let b = input[base + 2u];

    output[base]       = params.m00 * r + params.m01 * g + params.m02 * b;
    output[base + 1u] = params.m10 * r + params.m11 * g + params.m12 * b;
    output[base + 2u] = params.m20 * r + params.m21 * g + params.m22 * b;
}
