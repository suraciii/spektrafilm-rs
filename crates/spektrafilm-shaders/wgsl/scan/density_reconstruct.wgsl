@group(0) @binding(1) var<storage, read> input: array<f32>;
@group(0) @binding(4) var<storage, read_write> output: array<f32>;
@group(0) @binding(6) var<storage, read> setup_state: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= p.header.x { return; }
    for (var c = 0u; c < 3u; c++) {
        let d = input[pixel*3u+c];
        var x = estimate(d,c);
        if p.flags.x != 0u { x += setup_state[5u+c]; }
        if !finite32(d) || !finite32(x) { atomicOr(&status,1u); }
        output[pixel*3u+c] = x;
    }
}
