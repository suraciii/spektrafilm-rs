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
// Shared scanner/enlarger spectral lattice generator. The source halo stays
// binary32 until the common monotone-slope preparation consumes it.
struct Params {
    size:u32, n:u32, n_wl:u32, flags:u32,
    origin:vec4<u32>, extent:vec4<u32>,
    minimum:vec4<f32>, maximum:vec4<f32>, preflash:vec4<f32>, normalization:vec4<f32>,
}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> cd:array<f32>;
@group(0) @binding(2) var<storage,read> bd:array<f32>;
@group(0) @binding(3) var<storage,read> illuminant:array<f32>;
@group(0) @binding(4) var<storage,read> sensitivity:array<f32>;
@group(0) @binding(5) var<storage,read_write> output:array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=gid.x+gid.y*grid.x*256u;
    if i>=p.n{return;}
    let q=p.origin.xyz+vec3<u32>(i/(p.extent.y*p.extent.z),(i/p.extent.z)%p.extent.y,i%p.extent.z);
    var d=p.minimum.xyz+(p.maximum.xyz-p.minimum.xyz)*vec3<f32>(q)/f32(p.size-1u);
    var raw=vec3<f32>(0.0);
    for(var wl=0u;wl<p.n_wl;wl++) {
        let b=wl*3u;
        let density=d.x*cd[b]+d.y*cd[b+1u]+d.z*cd[b+2u]+bd[wl];
        var light=transmission_exp(-density*f32(2.302585092994046))*illuminant[wl];
        // density_to_light zeros NaN light, not the projection coefficients.
        if light!=light {light=f32(0.0);}
        raw.x+=light*sensitivity[b];raw.y+=light*sensitivity[b+1u];raw.z+=light*sensitivity[b+2u];
    }
    raw=raw*p.normalization.x+p.preflash.xyz;
    for(var c=0u;c<3u;c++) {
        var value=raw[c];
        if value!=value || value<f32(0.0) {value=f32(0.0);}
        value=transmission_log(value+f32(1e-10))/f32(2.302585092994046);
        output[i*3u+c]=value;
    }
}
