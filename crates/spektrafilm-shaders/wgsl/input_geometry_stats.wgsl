// Global cropped-source range, carried across every input page/window.
// Comparisons ignore NaN just like the reference fold from infinities.
struct Params { count: u32, first: u32, _pad0: u32, _pad1: u32, }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> minimum: array<f32>;
@group(0) @binding(3) var<storage, read_write> maximum: array<f32>;
@compute @workgroup_size(1)
fn main() {
    var lo = bitcast<f32>(0x7f800000u);
    var hi = bitcast<f32>(0xff800000u);
    if p.first == 0u { lo = minimum[0]; hi = maximum[0]; }
    for (var i = 0u; i < p.count; i = i + 1u) {
        if source[i] < lo { lo = source[i]; }
        if source[i] > hi { hi = source[i]; }
    }
    minimum[0] = lo;
    maximum[0] = hi;
}
