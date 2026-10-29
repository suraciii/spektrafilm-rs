// Full-support bounded rectangle; padding contains globally reflected samples.
struct Params { dims:vec4<u32>, controls:vec4<f32> }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read> halo:array<f32>;
@group(0) @binding(2) var<storage,read> taps:array<f32>;
@group(0) @binding(3) var<storage,read_write> output:array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>) {
    let pixel=gid.x+gid.y*grid.x*256u;
    if pixel>=p.dims.x*p.dims.y {return;}
    let x=pixel%p.dims.x;
    let y=pixel/p.dims.x;
    let k=p.dims.w;
    let radius=k/2u;
    for(var c=0u;c<3u;c++) {
        var sum=0.0;
        for(var ky=0u;ky<k;ky++) {
            for(var kx=0u;kx<k;kx++) {
                sum+=taps[c*k*k+ky*k+kx]*halo[((y+ky)*p.dims.z+x+kx)*3u+c];
            }
        }
        let center=halo[((y+radius)*p.dims.z+x+radius)*3u+c];
        output[pixel*3u+c]=(1.0-p.controls.x)*center+p.controls.x*sum;
    }
}
