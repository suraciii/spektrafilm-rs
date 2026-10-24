// Separable preview triangle. Global support and normalization remain intact
// when gathered taps are split across storage windows.
struct Params {
    src_extent: u32, dst_extent: u32, position: u32, vertical: u32,
    count: u32, first_tap: u32, taps: u32, left: u32,
    right: u32, _pad: u32,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;
fn triangle(x: f32) -> f32 {
    if abs(x) < 1.0 { return 1.0 - abs(x); }
    return 0.0;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= p.count { return; }
    let ratio = f32(p.src_extent) / f32(p.dst_extent);
    let sratio = max(ratio, 1.0);
    let centre = (f32(p.position) + 0.5) * ratio - 0.5;
    var sum: f32 = 0.0;
    for (var i = p.left; i < p.right; i = i + 1u) {
        sum = sum + triangle((f32(i) - centre) / sratio);
    }
    for (var c = 0u; c < 3u; c = c + 1u) {
        var acc: f32 = 0.0;
        if p.first_tap != p.left { acc = out[idx * 3u + c]; }
        for (var t = 0u; t < p.taps; t = t + 1u) {
            let weight = triangle((f32(p.first_tap + t) - centre) / sratio) / sum;
            acc = acc + source[(t * p.count + idx) * 3u + c] * weight;
        }
        // The vertical intermediate is not clamped by image::imageops.
        if p.vertical == 0u && p.first_tap + p.taps == p.right {
            if acc > 1.0 { acc = 1.0; }
            if acc < 0.0 { acc = 0.0; }
        }
        out[idx * 3u + c] = acc;
    }
}
