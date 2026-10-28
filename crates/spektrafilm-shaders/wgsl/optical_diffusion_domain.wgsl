// Print exposure domains around the optical PSF.
struct Params { dims:vec4<u32>, controls:vec4<f32> }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> input:array<f32>;
@group(0) @binding(2) var<storage,read_write> output:array<f32>;
// Reduce the argument before exponentiation, retaining the scale in log2
// form so an extreme finite exposure does not overflow on host conversion.
fn scaled_pow10(x:f32, log_scale:f32, sign:f32) -> f32 {
    if sign == 0.0 { return 0.0; }
    let exponent = x * 3.321928094887362 + log_scale;
    let whole = floor(exponent);
    return sign * exp2(exponent - whole) * exp2(whole);
}
fn log10_positive(x:f32) -> f32 {
    let bits = bitcast<u32>(x);
    let exponent = f32(i32((bits >> 23u) & 255u) - 127);
    let mantissa = bitcast<f32>((bits & 0x7fffffu) | 0x3f800000u);
    return (exponent + log2(mantissa)) * 0.3010299956639812;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let pixel=gid.x+gid.y*grid.x*256u;
    if pixel>=p.dims.x {return;}
    for(var c=0u;c<3u;c++) {
        let i=pixel*3u+c;
        if p.dims.y==0u {output[i]=scaled_pow10(input[i],p.controls.x,p.controls.y);}
        else {output[i]=log10_positive(max(input[i],0.0)+1e-10);}
    }
}
