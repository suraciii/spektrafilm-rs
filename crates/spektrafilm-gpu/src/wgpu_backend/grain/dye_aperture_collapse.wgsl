// Preserve the CPU's factor-by-factor linear-light average and log10.
struct Params { dims: vec4<u32> } // fine width, output width/height, factor
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> fine: array<f32>;
@group(0) @binding(2) var<storage, read_write> output: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let i = gid.x + gid.y * grid.x * 256u;
    if (i >= p.dims.y * p.dims.z) { return; }
    let x = i % p.dims.y;
    let y = i / p.dims.y;
    var sum = vec3<f32>(0.0);
    // Compensated accumulation keeps a factor-24 reduction from losing the
    // low-order exposure contributions. It does not change the box law.
    var correction = vec3<f32>(0.0);
    for (var dy = 0u; dy < p.dims.w; dy++) {
        for (var dx = 0u; dx < p.dims.w; dx++) {
            let s = ((y * p.dims.w + dy) * p.dims.x + x * p.dims.w + dx) * 3u;
            let value = pow(vec3<f32>(10.0), vec3<f32>(fine[s], fine[s + 1u], fine[s + 2u]));
            let adjusted = value - correction;
            let next = sum + adjusted;
            correction = (next - sum) - adjusted;
            sum = next;
        }
    }
    let result = log(sum / f32(p.dims.w * p.dims.w)) / log(10.0);
    output[i * 3u] = result.x;
    output[i * 3u + 1u] = result.y;
    output[i * 3u + 2u] = result.z;
}
