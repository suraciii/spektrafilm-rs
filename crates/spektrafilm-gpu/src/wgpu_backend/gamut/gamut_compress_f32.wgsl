@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel_idx = gid.x + gid.y * grid.x * 256u;
    if pixel_idx >= params.n_pixels {
        return;
    }
    let base = pixel_idx * 3u;
    let rgb = vec3<f32>(img[base], img[base + 1u], img[base + 2u]);
    var out: vec3<f32>;
    if params.mode == 0u {
        out = compress_aces_rgc(rgb);
    } else {
        out = compress_perceptual(rgb);
    }
    img[base] = out.x;
    img[base + 1u] = out.y;
    img[base + 2u] = out.z;
}
