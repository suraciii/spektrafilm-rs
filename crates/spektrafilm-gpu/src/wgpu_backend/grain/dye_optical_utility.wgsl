// Un-floored log10 at the exact-cell optical probe boundary.
struct Params { dims: vec4<u32>, padding: vec4<u32> }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;
@group(0) @binding(2) var<storage, read_write> output: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let i = gid.x + gid.y * grid.x * 256u;
    if (i >= p.dims.y * p.dims.z) { return; }
    let b = i * 3u;
    let value = log(vec3<f32>(input[b], input[b + 1u], input[b + 2u])) / log(10.0);
    output[b] = value.x;
    output[b + 1u] = value.y;
    output[b + 2u] = value.z;
}
