// Prepare the optional halation source response once per pixel.
// The base film raw stays in the input buffer; only the blur source is
// compressed, using the post-scatter per-pixel channel peak.
struct Params {
    n_pixels: u32,
    compression: f32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;
@group(0) @binding(2) var<storage, read_write> output: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels { return; }
    let base = idx * 3u;
    let source = vec3<f32>(input[base], input[base + 1u], input[base + 2u]);
    let peak = max(0.0, max(source.x, max(source.y, source.z)));
    // Keep sources at or above the fixed anchor unchanged. Compression only
    // lifts weaker sources toward the brightest source response.
    let denominator = select(
        (1.0 - params.compression) + params.compression * peak,
        1.0,
        peak > 1.0,
    );
    let response = source / denominator;
    output[base] = response.x;
    output[base + 1u] = response.y;
    output[base + 2u] = response.z;
}
