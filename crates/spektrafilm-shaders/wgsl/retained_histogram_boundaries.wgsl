// Match the CPU binary64 OETF and nearest-bin rule over positive f32 words.
// Each invocation finds one exact transition; no sampled LUT is uploaded.
@group(0) @binding(0) var<storage, read_write> boundaries: array<u32>;

fn histogram_srgb_encode(value: f64) -> f64 {
    if value <= 0.0031308 { return value * 12.92; }
    return 1.055 * sf64_pow(value, f64(1.0) / f64(2.4)) - 0.055;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if index >= 63u { return; }
    var low = 0u;
    var high = bitcast<u32>(1.0f);
    while low < high {
        let middle = low + (high - low) / 2u;
        let encoded = histogram_srgb_encode(f64(bitcast<f32>(middle)));
        if u32(sf64_round(encoded * 63.0)) > index { high = middle; }
        else { low = middle + 1u; }
    }
    boundaries[index] = low;
}
