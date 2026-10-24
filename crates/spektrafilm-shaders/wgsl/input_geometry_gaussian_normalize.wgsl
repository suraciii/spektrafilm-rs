// Carry the reference's sequential kernel sum, then normalize each window.
struct Params { count: u32, mode: u32, first: u32, _pad: u32, }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> weights: array<f64>;
@group(0) @binding(2) var<storage, read_write> state: array<f64>;
@compute @workgroup_size(1)
fn main() {
    if p.mode == 0u {
        var sum: f64 = 0.0lf;
        if p.first != 0u { sum = state[0]; }
        for (var i = 0u; i < p.count; i = i + 1u) { sum = sum + weights[i]; }
        state[0] = sum;
    } else {
        for (var i = 0u; i < p.count; i = i + 1u) { weights[i] = weights[i] / state[0]; }
    }
}
