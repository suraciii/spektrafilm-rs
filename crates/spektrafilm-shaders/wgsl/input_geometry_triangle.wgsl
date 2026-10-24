// Separable preview triangle over retained image pages. Source pages are
// visited in order; normalization covers the full global tap support.
struct Params {
    src_width: u32, src_height: u32, dst_width: u32, dst_height: u32,
    source_first: u32, source_count: u32, output_first: u32, output_count: u32,
    vertical: u32, flags: u32, _pad0: u32, _pad1: u32,
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
    if idx >= p.output_count { return; }
    let pixel = p.output_first + idx;
    let x = pixel % p.dst_width;
    let y = pixel / p.dst_width;
    let vertical = p.vertical != 0u;
    let extent = select(p.src_width, p.src_height, vertical);
    let output_extent = select(p.dst_width, p.dst_height, vertical);
    let position = select(x, y, vertical);
    let base = select(y * p.src_width, x, vertical);
    let stride = select(1u, p.src_width, vertical);
    let ratio = f32(extent) / f32(output_extent);
    let support = max(ratio, 1.0);
    let support_centre = (f32(position) + 0.5) * ratio;
    let centre = support_centre - 0.5;
    let left_value = floor(support_centre - support);
    // Clamp after integer conversion, as the host/reference do. Comparing
    // against f32(extent) would round large extents and change boundary taps.
    var left = extent - 1u;
    if left_value < 4294967296.0 {
        left = min(u32(max(left_value, 0.0)), extent - 1u);
    }
    let right_value = ceil(support_centre + support);
    var right = extent;
    if right_value < 4294967296.0 {
        right = clamp(u32(max(right_value, 0.0)), left + 1u, extent);
    }

    // Intersect this pixel's ordered taps with the currently bound source page.
    // Ceil division avoids visiting a pixel in an adjacent column or row.
    var first = left;
    if p.source_first > base {
        let distance = p.source_first - base;
        first = max(first, distance / stride + select(0u, 1u, distance % stride != 0u));
    }
    let source_end = p.source_first + p.source_count;
    var end = left;
    if source_end > base {
        let distance = source_end - base;
        end = min(right, distance / stride + select(0u, 1u, distance % stride != 0u));
    }
    var sum: f32 = 0.0;
    if first < end {
        for (var t = left; t < right; t = t + 1u) {
            sum = sum + triangle((f32(t) - centre) / support);
        }
    }
    for (var c = 0u; c < 3u; c = c + 1u) {
        var acc: f32 = 0.0;
        if (p.flags & 1u) == 0u { acc = out[idx * 3u + c]; }
        for (var t = first; t < end; t = t + 1u) {
            let weight = triangle((f32(t) - centre) / support) / sum;
            let source_pixel = base + t * stride - p.source_first;
            acc = acc + source[source_pixel * 3u + c] * weight;
        }
        // Only the completed horizontal pass clamps, as in image::imageops.
        if !vertical && (p.flags & 2u) != 0u {
            if acc > 1.0 { acc = 1.0; }
            if acc < 0.0 { acc = 0.0; }
        }
        out[idx * 3u + c] = acc;
    }
}
