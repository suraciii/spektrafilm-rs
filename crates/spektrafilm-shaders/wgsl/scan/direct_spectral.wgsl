// Neutral scanner reference: binary64 spectral reduction, log/exp floor,
// then separate chromatic adaptation and output RGB transforms.
struct Params {header:vec4<u32>,normalization:vec2<f64>,cat:array<vec4<f64>,3>,rgb:array<vec4<f64>,3>}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> input:array<f32>;
@group(0) @binding(2) var<storage,read> dye:array<f64>;
@group(0) @binding(3) var<storage,read> base:array<f64>;
@group(0) @binding(4) var<storage,read> lamp:array<f64>;
@group(0) @binding(5) var<storage,read> cmfs:array<f64>;
@group(0) @binding(6) var<storage,read_write> output:array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let pixel=gid.x+gid.y*grid.x*256u;
    if pixel>=p.header.x{return;}
    let b=pixel*3u;
    let cmy=vec3<f64>(f64(input[b]),f64(input[b+1u]),f64(input[b+2u]));
    var xyz=vec3<f64>(0.0);
    for(var wl=0u;wl<p.header.y;wl++) {
        let i=wl*3u;
        let density=(cmy.x*dye[i]+cmy.y*dye[i+1u]+cmy.z*dye[i+2u])+base[wl];
        let light=sf64_exp(-density*2.302585092994046)*lamp[wl];
        xyz.x+=light*cmfs[i];xyz.y+=light*cmfs[i+1u];xyz.z+=light*cmfs[i+2u];
    }
    let inv=1.0/p.normalization.x;
    for(var c=0u;c<3u;c++) {
        let log_xyz=sf64_log(max(xyz[c]*inv,f64(0.0))+1e-10)/2.302585092994046;
        xyz[c]=sf64_exp(log_xyz*2.302585092994046);
    }
    let a=vec3<f64>(
        p.cat[0].x*xyz.x+p.cat[0].y*xyz.y+p.cat[0].z*xyz.z,
        p.cat[1].x*xyz.x+p.cat[1].y*xyz.y+p.cat[1].z*xyz.z,
        p.cat[2].x*xyz.x+p.cat[2].y*xyz.y+p.cat[2].z*xyz.z);
    output[b]=f32(p.rgb[0].x*a.x+p.rgb[0].y*a.y+p.rgb[0].z*a.z);
    output[b+1u]=f32(p.rgb[1].x*a.x+p.rgb[1].y*a.y+p.rgb[1].z*a.z);
    output[b+2u]=f32(p.rgb[2].x*a.x+p.rgb[2].y*a.y+p.rgb[2].z*a.z);
}
