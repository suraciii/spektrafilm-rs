// Gathered taps retain ascending global kernel order across dispatches.
struct Params { count: u32, taps: u32, first_tap: u32, _pad: u32, }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f64>;
@group(0) @binding(2) var<storage, read> weights: array<f64>;
@group(0) @binding(3) var<storage, read_write> out: array<f64>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= p.count { return; }
    for (var c = 0u; c < 3u; c = c + 1u) {
        var sum: f64 = 0.0lf;
        if p.first_tap != 0u { sum = out[idx * 3u + c]; }
        for (var t = 0u; t < p.taps; t = t + 1u) {
            sum = sum + weights[t] * source[(t * p.count + idx) * 3u + c];
        }
        out[idx * 3u + c] = sum;
    }
}
