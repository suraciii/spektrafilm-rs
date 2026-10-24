// Exact print exposure domains around the optical PSF.
struct Params { dims:vec4<u32>, controls:vec4<f64> }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> input:array<f32>;
@group(0) @binding(2) var<storage,read_write> output:array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let pixel=gid.x+gid.y*grid.x*256u;
    if pixel>=p.dims.x {return;}
    for(var c=0u;c<3u;c++) {
        let i=pixel*3u+c;
        if p.dims.y==0u {output[i]=f32(sf64_exp(f64(input[i])*2.302585092994046)*p.controls.x);}
        else {output[i]=f32(sf64_log(max(f64(input[i]),f64(0.0))+1e-10)/2.302585092994046);}
    }
}
