// The scanner optics integrate transmission, never density.
// Count is pixels; each invocation keeps its three channels in one page.
struct Params { count: u32, inverse: u32, _pad: vec2<u32> }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> image: array<f32>;
// Range-reduced binary32 series, matching the scanner's spectral math.
fn transmission_exp(x: f32) -> f32 {
    if (bitcast<u32>(x) & 0x7fffffffu) > 0x7f800000u { return x; }
    if x < -103.97208 { return 0.0; }
    if x > 88.72284 { return bitcast<f32>(0x7f800000u); }
    let scaled = x * 1.44269504089;
    let exponent = i32(select(ceil(scaled - 0.5), floor(scaled + 0.5), scaled >= 0.0));
    let r = (x - f32(exponent) * 0.693145751953125) - f32(exponent) * 0.00000142860682;
    var term = 1.0;
    var sum = 1.0;
    for (var n = 1u; n <= 10u; n++) { term *= r / f32(n); sum += term; }
    if exponent > 0i { for (var n = 0i; n < exponent; n++) { sum *= 2.0; } }
    else { for (var n = 0i; n > exponent; n--) { sum *= 0.5; } }
    return sum;
}
fn transmission_log(value: f32) -> f32 {
    let bits = bitcast<u32>(value);
    if (bits & 0x7fffffffu) > 0x7f800000u { return value; }
    if value < 0.0 { return bitcast<f32>(0x7fc00000u); }
    if value == 0.0 { return bitcast<f32>(0xff800000u); }
    if bits == 0x7f800000u { return value; }
    var x = value;
    var exponent = 0i;
    loop { if x >= 0.70710678 { break; } x *= 2.0; exponent--; }
    loop { if x < 1.41421356 { break; } x *= 0.5; exponent++; }
    let z = (x - 1.0) / (x + 1.0);
    let z2 = z * z;
    var term = z;
    var sum = z;
    for (var n = 3u; n <= 13u; n += 2u) { term *= z2; sum += term / f32(n); }
    return f32(exponent) * 0.693145751953125 + (2.0 * sum + f32(exponent) * 0.00000142860682);
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let i = gid.x + gid.y * grid.x * 256u;
    if i >= p.count { return; }
    for(var c=0u;c<3u;c++) {
        let sample=i*3u+c;
        if p.inverse == 0u { image[sample] = transmission_exp(-image[sample] * 2.302585092994046); }
        else { image[sample] = -transmission_log(image[sample]) / 2.302585092994046; }
    }
}
