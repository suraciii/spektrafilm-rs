struct Params {
    n: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> base: array<f32>;
@group(0) @binding(2) var<storage, read> mask: array<f32>;
@group(0) @binding(3) var<storage, read> halo: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = gid.x + gid.y * grid.x * 256u;
    if index >= p.n { return; }
    let base_index = index * 3u;
    // One guard for all channels: subtracting per-channel masks can invert a
    // red-dominant response into a green fringe at a neutral source edge.
    let guard = max(mask[base_index], max(mask[base_index + 1u], mask[base_index + 2u]));
    for (var channel = 0u; channel < 3u; channel++) {
        let addition = max(halo[base_index + channel] - guard, 0.0);
        base[base_index + channel] += addition;
    }
}
