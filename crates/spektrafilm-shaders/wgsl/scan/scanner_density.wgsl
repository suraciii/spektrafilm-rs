// Render immutable reconstructed exposures; tone defines linear sRGB once.
@group(0) @binding(1) var<storage, read> input: array<f32>;
@group(0) @binding(4) var<storage, read_write> output: array<f32>;

// Reduce by ln(2) before the series to avoid driver transcendental narrowing.
fn tone_exp(x: f32) -> f32 {
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
fn exp_m1(x: f32) -> f32 {
    if abs(x) > 0.5 { return tone_exp(x) - 1.0; }
    var term = x;
    var sum = x;
    for (var n = 2u; n <= 10u; n++) { term *= x / f32(n); sum += term; }
    return sum;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= p.header.x { return; }
    var x = vec3<f32>(input[3u*pixel],input[3u*pixel+1u],input[3u*pixel+2u]);
    if p.flags.y != 0u {
        let stops = (x - vec3<f32>(p.operator_terms.x)) / 0.3010299956639812;
        let mean = ((stops.x + stops.y) + stops.z) / 3.0;
        let operator_shift = p.operator_terms.y * mean + p.operator_terms.z;
        x = vec3<f32>(p.operator_terms.x) + 0.3010299956639812 * (stops + vec3<f32>(operator_shift) + p.color.xyz);
        if !finite32(mean) || !finite32(operator_shift) { atomicOr(&status,16u); }
    }
    var linear_srgb = vec3<f32>(0.0);
    for (var c = 0u; c < 3u; c++) {
        let e = x[c] / 0.3010299956639812 - p.camera.x + p.brightness[c];
        let z = p.contrast[c] * e;
        if !finite32(x[c]) || !finite32(e) || !finite32(z) { atomicOr(&status,16u); }
        var t: f32;
        if z >= 0.0 { t = 1.0 / (1.0 + tone_exp(-z)); }
        else { let a = tone_exp(z); t = a / (1.0 + a); }
        // Form density first, like CPU positive_tone: multiplying LN_10 by
        // a valid huge dmax before the zero shoulder would overflow to NaN.
        let tone_density = p.dmax[c] * (1.0 - t);
        let positive = exp_m1(-2.302585092994046 * tone_density);
        linear_srgb[c] = (positive + p.denominator[c]) / p.denominator[c];
    }
    for (var c = 0u; c < 3u; c++) {
        var value = linear_srgb[c];
        // Identity bypass matches CPU and preserves exact signed/headroom data.
        if p.flags.z != 0u {
            value = (p.destination[c].x * linear_srgb.x + p.destination[c].y * linear_srgb.y) + p.destination[c].z * linear_srgb.z;
        }
        if !finite32(value) { atomicOr(&status,1u); }
        output[pixel*3u+c] = value;
    }
}
