@group(0) @binding(1) var<storage, read> axes: array<f32>;
@group(0) @binding(2) var<storage, read> source: array<f32>;
@group(0) @binding(3) var<storage, read_write> corrected: array<f32>;
fn lens_fetch(x: u32, y: u32, c: u32) -> f32 {
    return source[(y * lens.dimensions.x + x) * 3u + c];
}
fn lens_weight(value: f32, x: vec2<f32>, y: vec2<f32>) -> vec2<f32> {
    return lens_ds_mul(lens_ds_mul(vec2(value, 0.0), x), y);
}
fn lens_sample(p: vec2<f32>, c: u32) -> f32 {
    // Native/scipy nearest-edge extension; NaN coordinates yield zero.
    if p.x != p.x || p.y != p.y { return 0.0; }
    let q = clamp(p, vec2(0.0), vec2<f32>(lens.dimensions.xy - vec2(1u)));
    let x0 = u32(q.x); let y0 = u32(q.y);
    let x1 = min(x0 + 1u, lens.dimensions.x - 1u);
    let y1 = min(y0 + 1u, lens.dimensions.y - 1u);
    let dx = lens_ds_add(vec2(q.x, 0.0), vec2(-f32(x0), 0.0));
    let dy = lens_ds_add(vec2(q.y, 0.0), vec2(-f32(y0), 0.0));
    let nx = lens_ds_add(vec2(1.0, 0.0), -dx);
    let ny = lens_ds_add(vec2(1.0, 0.0), -dy);
    var sum = lens_weight(lens_fetch(x0, y0, c), nx, ny);
    sum = lens_ds_add(sum, lens_weight(lens_fetch(x1, y0, c), dx, ny));
    sum = lens_ds_add(sum, lens_weight(lens_fetch(x0, y1, c), nx, dy));
    return lens_round(lens_ds_add(sum, lens_weight(lens_fetch(x1, y1, c), dx, dy)));
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let i = id.x + id.y * groups.x * 256u;
    let width = lens.dimensions.x;
    if i >= width * lens.dimensions.y { return; }
    let x = i % width; let y = i / width;
    let initial = vec2(axes[x], axes[width + y]);
    let p = lens_project(initial * lens.mapping.w);
    var previous = p;
    var next = p;
    if lens.flags.w != 0u && (lens.dimensions.z == 1u || lens.dimensions.z == 3u) {
        previous = lens_project(vec2(axes[max(x, 1u) - 1u], initial.y) * lens.mapping.w);
        next = lens_project(vec2(axes[min(x + 1u, width - 1u)], initial.y) * lens.mapping.w);
    }
    for (var c = 0u; c < 3u; c++) {
        let distorted = lens_combined_distort(p, previous, next, x, y, c);
        corrected[i * 3u + c] = lens_sample(lens_pixel(lens_tca(distorted, c)), c);
    }
}
