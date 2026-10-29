// Direct scanner reference: binary32 spectral reduction, log/exp floor,
// then separate chromatic adaptation and output RGB transforms.
struct Params {header:vec4<u32>,normalization:vec4<f32>,cat:array<vec4<f32>,3>,rgb:array<vec4<f32>,3>}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> input:array<f32>;
@group(0) @binding(2) var<storage,read> dye:array<f32>;
@group(0) @binding(3) var<storage,read> base:array<f32>;
@group(0) @binding(4) var<storage,read> lamp:array<f32>;
@group(0) @binding(5) var<storage,read> cmfs:array<f32>;
@group(0) @binding(6) var<storage,read_write> output:array<f32>;

// Range reduction and series mirror f64_math; no driver log/pow is used.
// In a 100,000-point binary32 arithmetic sweep, exp10 over [-4, 1]
// differed by at most 3 ulp and log10 over [1e-5, 2] by at most 4 ulp
// from f64 results rounded to f32; device contraction may differ.
fn scan_log10(value:f32)->f32 {
    if value != value || value >= bitcast<f32>(0x7f800000u) { return value; }
    if value <= 0.0 { return bitcast<f32>(0xff800000u); }
    var x=value;
    var exponent=0i;
    loop { if x>=0.70710678 { break; } x=x*2.0; exponent--; }
    loop { if x<1.41421356 { break; } x=x*0.5; exponent++; }
    let z=(x-1.0)/(x+1.0);
    let z2=z*z;
    var term=z;
    var sum=z;
    for(var k=3u;k<=13u;k+=2u) { term=term*z2; sum=sum+term/f32(k); }
    return f32(exponent)*0.30102999566 + (2.0*sum)*0.4342944819;
}
fn scan_exp10(value:f32)->f32 {
    if value != value { return value; }
    if value < -45.0 { return 0.0; }
    if value > 38.53184 { return bitcast<f32>(0x7f800000u); }
    let scaled=value*3.32192809489;
    let exponent=i32(select(ceil(scaled-0.5),floor(scaled+0.5),scaled>=0.0));
    let r=(value-f32(exponent)*0.301025390625)*2.302585093 - f32(exponent)*0.000010603225;
    var term=1.0;
    var sum=1.0;
    for(var k=1u;k<=10u;k++) { term=term*r/f32(k); sum=sum+term; }
    if exponent>0i { for(var k=0i;k<exponent;k++) { sum=sum*2.0; } }
    else { for(var k=0i;k>exponent;k--) { sum=sum*0.5; } }
    return sum;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let pixel=gid.x+gid.y*grid.x*256u;
    if pixel>=p.header.x{return;}
    let b=pixel*3u;
    let cmy=vec3<f32>(input[b],input[b+1u],input[b+2u]);
    var xyz=vec3<f32>(0.0);
    for(var wl=0u;wl<p.header.y;wl++) {
        let i=wl*3u;
        let density=(cmy.x*dye[i]+cmy.y*dye[i+1u]+cmy.z*dye[i+2u])+base[wl];
        let light=scan_exp10(-density)*lamp[wl];
        xyz.x+=light*cmfs[i];xyz.y+=light*cmfs[i+1u];xyz.z+=light*cmfs[i+2u];
    }
    let inv=1.0/p.normalization.x;
    for(var c=0u;c<3u;c++) {
        let log_xyz=scan_log10(max(xyz[c]*inv,0.0)+1e-10);
        xyz[c]=scan_exp10(log_xyz);
    }
    let a=vec3<f32>(
        p.cat[0].x*xyz.x+p.cat[0].y*xyz.y+p.cat[0].z*xyz.z,
        p.cat[1].x*xyz.x+p.cat[1].y*xyz.y+p.cat[1].z*xyz.z,
        p.cat[2].x*xyz.x+p.cat[2].y*xyz.y+p.cat[2].z*xyz.z);
    output[b]=p.rgb[0].x*a.x+p.rgb[0].y*a.y+p.rgb[0].z*a.z;
    output[b+1u]=p.rgb[1].x*a.x+p.rgb[1].y*a.y+p.rgb[1].z*a.z;
    output[b+2u]=p.rgb[2].x*a.x+p.rgb[2].y*a.y+p.rgb[2].z*a.z;
}
