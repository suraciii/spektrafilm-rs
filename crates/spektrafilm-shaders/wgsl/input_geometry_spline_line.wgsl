// Whole-line spline recurrence, streamed through bounded windows. State is
// [closed-form first, zi, previous forward/backward value].
struct Params { len: u32, count: u32, mode: u32, first: u32, zn: f64, }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> a: array<f64>;
@group(0) @binding(2) var<storage, read> b: array<f64>;
@group(0) @binding(3) var<storage, read_write> out: array<f64>;
@group(0) @binding(4) var<storage, read_write> state: array<f64>;
@compute @workgroup_size(1)
fn main() {
    let z = 0.0lf - 0.267949192431122706472553658494127633lf;
    let gain = (1.0lf - z) * (1.0lf - 1.0lf / z);
    if p.mode == 0u {
        state[0] = a[0] * gain + p.zn * (b[0] * gain);
        state[1] = z;
    } else if p.mode == 1u {
        var first = state[0];
        var zi = state[1];
        for (var i = 0u; i < p.count; i = i + 1u) {
            first = first + zi * (a[i] * gain + p.zn * (b[i] * gain));
            zi = zi * z;
        }
        state[0] = first;
        state[1] = zi;
    } else if p.mode == 2u {
        var previous = state[2];
        for (var i = 0u; i < p.count; i = i + 1u) {
            var value = a[i] * gain;
            if p.first + i == 0u {
                value = state[0] / (1.0lf - p.zn * p.zn);
            } else {
                value = value + z * previous;
            }
            out[i] = value;
            previous = value;
        }
        state[2] = previous;
    } else if p.mode == 3u {
        let value = (z * a[0] + b[0]) * z / (z * z - 1.0lf);
        out[0] = value;
        state[2] = value;
    } else {
        var previous = state[2];
        for (var i = p.count; i > 0u; i = i - 1u) {
            let value = z * (previous - a[i - 1u]);
            out[i - 1u] = value;
            previous = value;
        }
        state[2] = previous;
    }
}
