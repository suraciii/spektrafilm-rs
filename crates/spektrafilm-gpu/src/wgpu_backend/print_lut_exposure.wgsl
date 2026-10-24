struct Params { n:u32, scale:f32, pad0:u32, pad1:u32 }
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var<storage,read_write> image:array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
    let i=gid.x+gid.y*grid.x*256u;if i>=p.n{return;}
    for(var c=0u;c<3u;c++){
        let j=i*3u+c;
        let recovered=exp(image[j]*log(10.0))*p.scale;
        image[j]=log(max(recovered,0.0)+1e-10)/log(10.0);
    }
}
