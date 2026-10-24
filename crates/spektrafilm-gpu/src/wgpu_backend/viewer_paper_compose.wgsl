struct Params {
    frame: vec4<u32>, // width, height, rotated width, page pixel count
    stamp: vec4<u32>, // xmin, ymin, stamp width, stamp height
    page: vec4<u32>, // absolute first pixel, unused
}
struct Centers { count: u32, unused: array<u32, 3>, points: array<vec2<f32>> }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> rotated: array<f32>;
@group(0) @binding(2) var<storage, read> centers: Centers;
@group(0) @binding(3) var<storage, read> random: array<f32>;
@group(0) @binding(4) var<storage, read_write> rgb: array<f32>;
// WGSL round uses ties-to-even, matching the CPU stamp origins and 8-bit quantization.
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = id.x + id.y * 65535u * 256u;
    if n >= p.frame.w { return; }
    let absolute = p.page.x + n;
    let x = i32(absolute % p.frame.x);
    let y = i32(absolute / p.frame.x);
    var trans = 1.0;
    for (var c = 0u; c < centers.count; c++) {
        let origin = vec2<i32>(round(centers.points[c] - 0.5 * vec2<f32>(p.stamp.zw)));
        let dx = x - origin.x;
        let dy = y - origin.y;
        if dx >= 0 && dy >= 0 && dx < i32(p.stamp.z) && dy < i32(p.stamp.w) {
            trans *= 1.0 - rotated[(u32(dy) + p.stamp.y) * p.frame.z + u32(dx) + p.stamp.x];
        }
    }
    // The second random array starts after every draw of the first full frame,
    // not after a per-pixel pair (including for odd frame pixel counts).
    let g = 0.88 * random[n] + 0.12 * pow(random[p.frame.w + n], 10.0);
    let paper = 0.30 + 0.70 * trans;
    let variation = 1.0 + (2.0 * g - 1.0) * paper * 0.07 * 0.25;
    // Rust f32::powi(6): exponentiation by squaring, not transcendental pow.
    let g2 = g * g;
    let g4 = g2 * g2;
    let lift = (g2 * g4) * paper * 0.42 * 0.25;
    let base = (215.0 / 255.0 + trans * 27.0 / 255.0) * variation;
    let value = round(clamp(base + (1.0 - base) * lift, 0.0, 1.0) * 255.0) / 255.0;
    rgb[n * 3u] = value;
    rgb[n * 3u + 1u] = value;
    rgb[n * 3u + 2u] = value;
}
