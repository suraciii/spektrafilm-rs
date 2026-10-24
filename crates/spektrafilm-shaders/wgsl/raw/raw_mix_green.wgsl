struct MixPass{count:u32,pad0:u32,pad1:u32,pad2:u32};
@group(0) @binding(0) var<uniform> p:MixPass;
@group(0) @binding(1) var<storage,read_write> image:array<u32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) grid:vec3<u32>){
    let i=id.x+id.y*grid.x*256u;if(i>=p.count){return;}
    image[i*4u+1u]=(image[i*4u+1u]+image[i*4u+3u])>>1u;
}
