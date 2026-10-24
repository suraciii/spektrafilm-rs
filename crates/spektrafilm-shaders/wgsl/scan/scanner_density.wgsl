// Render immutable reconstructed exposures; tone defines linear sRGB once.
@group(0) @binding(1) var<storage, read> input: array<f64>;
@group(0) @binding(4) var<storage, read_write> output: array<f32>;

fn exp_m1(x: f64) -> f64 {
    if abs(x) > 0.5 { return sf64_exp(x) - 1.0; }
    var term = x;
    var sum = x;
    for (var n = 2u; n <= 24u; n++) { term *= x / f64(n); sum += term; }
    return sum;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= p.header.x { return; }
    var x = vec3<f64>(input[3u*pixel],input[3u*pixel+1u],input[3u*pixel+2u]);
    if p.flags.y != 0u {
        let stops = (x - vec3<f64>(p.operator_terms.x)) / 0.3010299956639812;
        let mean = ((stops.x + stops.y) + stops.z) / 3.0;
        let operator_shift = p.operator_terms.y * mean + p.operator_terms.z;
        x = vec3<f64>(p.operator_terms.x) + 0.3010299956639812 * (stops + vec3<f64>(operator_shift) + p.color.xyz);
        if !finite64(mean) || !finite64(operator_shift) { atomicOr(&status,16u); }
    }
    var linear_srgb = vec3<f64>(0.0);
    for (var c = 0u; c < 3u; c++) {
        let e = x[c] / 0.3010299956639812 - p.camera.x + p.brightness[c];
        let z = p.contrast[c] * e;
        if !finite64(x[c]) || !finite64(e) || !finite64(z) { atomicOr(&status,16u); }
        var t: f64;
        if z >= 0.0 { t = 1.0 / (1.0 + sf64_exp(-z)); }
        else { let a = sf64_exp(z); t = a / (1.0 + a); }
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
        let result = f32(value);
        if !finite64(value) || (bitcast<u32>(result) & 0x7f800000u) == 0x7f800000u { atomicOr(&status,1u); }
        output[pixel*3u+c] = result;
    }
}
