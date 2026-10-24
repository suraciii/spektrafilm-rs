// Widen retained RGB f32 samples to the binary64 working values the
// spline-rescale reference evaluates (`rescale_spline3` preprocesses to
// float64). Widening is exact, so no arithmetic difference is introduced.

struct Params {
    n_values: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f64>;

@compute @workgroup_size(256)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= p.n_values {
        return;
    }
    out[idx] = f64(source[idx]);
}
