// Shared scanner/enlarger spectral lattice generator. The source halo stays
// binary64 until the common monotone-slope preparation consumes it.
struct Params {
    size:u32, n:u32, n_wl:u32, flags:u32,
    origin:vec4<u32>, extent:vec4<u32>,
    minimum:vec4<f64>, maximum:vec4<f64>, preflash:vec4<f64>, normalization:vec4<f64>,
}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> cd:array<f64>;
@group(0) @binding(2) var<storage,read> bd:array<f64>;
@group(0) @binding(3) var<storage,read> illuminant:array<f64>;
@group(0) @binding(4) var<storage,read> sensitivity:array<f64>;
@group(0) @binding(5) var<storage,read_write> output:array<f64>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let i=gid.x+gid.y*grid.x*256u;
    if i>=p.n{return;}
    let q=p.origin.xyz+vec3<u32>(i/(p.extent.y*p.extent.z),(i/p.extent.z)%p.extent.y,i%p.extent.z);
    var d=p.minimum.xyz+(p.maximum.xyz-p.minimum.xyz)*vec3<f64>(q)/f64(p.size-1u);
    if (p.flags&1u)!=0u {d=vec3<f64>(vec3<f32>(d));}
    var raw=vec3<f64>(0.0);
    for(var wl=0u;wl<p.n_wl;wl++) {
        let b=wl*3u;
        let density=d.x*cd[b]+d.y*cd[b+1u]+d.z*cd[b+2u]+bd[wl];
        var light=sf64_exp(-density*f64(2.302585092994046))*illuminant[wl];
        // density_to_light zeros NaN light, not the projection coefficients.
        if light!=light {light=f64(0.0);}
        raw.x+=light*sensitivity[b];raw.y+=light*sensitivity[b+1u];raw.z+=light*sensitivity[b+2u];
    }
    raw=raw*p.normalization.x+p.preflash.xyz;
    for(var c=0u;c<3u;c++) {
        var value=raw[c];
        if value!=value || value<f64(0.0) {value=f64(0.0);}
        value=sf64_log(value+f64(1e-10))/f64(2.302585092994046);
        if (p.flags&2u)!=0u {value=f64(f32(value));}
        output[i*3u+c]=value;
    }
}
