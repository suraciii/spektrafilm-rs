struct ColorPass {
    out_width:u32, out_height:u32, iwidth:u32, iheight:u32,
    flip:u32, colors:u32, adaptation_active:u32, mix_green:u32,
    tint:f32, pad1:f32, pad2:f32, pad3:f32,
    camera_to_aces:array<vec4<f32>,3>, adaptation:array<vec4<f32>,3>,
    aces_to_xyz:array<vec4<f32>,3>, xyz_to_aces:array<vec4<f32>,3>,
};
@group(0) @binding(0) var<uniform> p: ColorPass;
@group(0) @binding(1) var<storage, read> source: array<u32>;
@group(0) @binding(2) var<storage, read_write> destination: array<u32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id:vec3<u32>, @builtin(num_workgroups) grid:vec3<u32>) {
    let i=id.x+id.y*grid.x*256u;
    if (i >= p.iwidth*p.iheight) { return; }
    var camera = vec4<f32>(f32(source[i*4u]),f32(source[i*4u+1u]),f32(source[i*4u+2u]),f32(source[i*4u+3u]));
    var colors = p.colors;
    if (p.mix_green != 0u) {
        camera.y = f32((source[i*4u+1u]+source[i*4u+3u]) >> 1u);
        colors = 3u;
    }
    if (colors < 4u) { camera.w = 0.0; }
    for (var c=0u;c<3u;c++) {
        let m = p.camera_to_aces[c];
        // Preserve the native left-associated sum rather than a dot reduction.
        var value = m.x*camera.x + m.y*camera.y + m.z*camera.z;
        if (colors == 4u) { value += m.w*camera.w; }
        destination[i*4u+c] = raw_clip(i32(value));
    }
}
