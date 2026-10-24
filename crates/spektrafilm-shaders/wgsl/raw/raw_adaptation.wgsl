struct Adaptation { matrix:array<vec4<f32>,3>, count:u32, enabled:u32, tint:f32, pad:u32 };
@group(0) @binding(0) var<uniform> p:Adaptation;
@group(0) @binding(1) var<storage,read_write> pixels:array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>, @builtin(num_workgroups) grid:vec3<u32>) {
    let i=id.x+id.y*grid.x*256u;
    if (i>=p.count) { return; }
    var rgb=vec3<f32>(pixels[i*3u],pixels[i*3u+1u],pixels[i*3u+2u]);
    if (p.enabled!=0u) {
        let xyz=vec3<f32>(0.9525523959*rgb.x+0.0000936786*rgb.z,0.3439664498*rgb.x+0.7281660966*rgb.y-0.0721325464*rgb.z,1.0088251844*rgb.z);
        let adapted=vec3<f32>(dot(p.matrix[0].xyz,xyz),dot(p.matrix[1].xyz,xyz),dot(p.matrix[2].xyz,xyz));
        rgb=vec3<f32>(1.0498110175*adapted.x-0.0000974845*adapted.z,-0.4959030231*adapted.x+1.3733130458*adapted.y+0.0982400361*adapted.z,0.9912520182*adapted.z);
    }
    rgb.y*=p.tint;
    pixels[i*3u]=rgb.x; pixels[i*3u+1u]=rgb.y; pixels[i*3u+2u]=rgb.z;
}
