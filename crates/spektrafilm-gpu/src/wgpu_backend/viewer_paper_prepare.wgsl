// The scalar loop order, clamped edges and bilinear endpoints match decorations.rs.
struct Params {
    source: vec4<u32>, // source width/height, tile width/height
    rotated: vec4<u32>, // rotated width/height, blur radius, unused
    geometry: vec4<f32>, // sigma, sin(angle), cos(angle), unused
}
struct Bounds { xmin: atomic<u32>, xmax: atomic<u32>, ymin: atomic<u32>, ymax: atomic<u32> }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> destination: array<f32>;
@group(0) @binding(3) var<storage, read_write> kernel: array<f32>;
@group(0) @binding(4) var<storage, read_write> bounds: Bounds;
fn index_of(id: vec3<u32>) -> u32 { return id.x + id.y * 65535u * 256u; }
@compute @workgroup_size(1)
fn make_kernel() {
    let radius = i32(p.rotated.z);
    var sum = 0.0;
    for (var i = -radius; i <= radius; i++) {
        let w = exp(-f32(i * i) / (2.0 * p.geometry.x * p.geometry.x));
        kernel[u32(i + radius)] = w;
        sum += w;
    }
    for (var i = 0u; i < p.rotated.z * 2u + 1u; i++) { kernel[i] /= sum; }
}
@compute @workgroup_size(256)
fn blur_horizontal(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = index_of(id);
    if n >= p.source.x * p.source.y { return; }
    let x = n % p.source.x;
    let y = n / p.source.x;
    let radius = i32(p.rotated.z);
    var value = 0.0;
    for (var i = -radius; i <= radius; i++) {
        let sx = u32(clamp(i32(x) + i, 0, i32(p.source.x) - 1));
        value += source[y * p.source.x + sx] * kernel[u32(i + radius)];
    }
    destination[n] = value;
}
@compute @workgroup_size(256)
fn blur_vertical(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = index_of(id);
    if n >= p.source.x * p.source.y { return; }
    let x = n % p.source.x;
    let y = n / p.source.x;
    let radius = i32(p.rotated.z);
    var value = 0.0;
    for (var i = -radius; i <= radius; i++) {
        let sy = u32(clamp(i32(y) + i, 0, i32(p.source.y) - 1));
        value += source[sy * p.source.x + x] * kernel[u32(i + radius)];
    }
    destination[n] = value;
}
fn sample_alpha(width: u32, height: u32, x: f32, y: f32) -> f32 {
    if x < 0.0 || y < 0.0 || x > f32(width - 1u) || y > f32(height - 1u) { return 0.0; }
    let x0 = u32(floor(x));
    let y0 = u32(floor(y));
    let x1 = min(x0 + 1u, width - 1u);
    let y1 = min(y0 + 1u, height - 1u);
    let a = x - f32(x0);
    let b = y - f32(y0);
    return (source[y0 * width + x0] * (1.0 - a) + source[y0 * width + x1] * a) * (1.0 - b)
        + (source[y1 * width + x0] * (1.0 - a) + source[y1 * width + x1] * a) * b;
}
@compute @workgroup_size(256)
fn resample_tile(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = index_of(id);
    if n >= p.source.z * p.source.w { return; }
    let x = f32(n % p.source.z) * f32(p.source.x - 1u) / f32(max(p.source.z - 1u, 1u));
    let y = f32(n / p.source.z) * f32(p.source.y - 1u) / f32(max(p.source.w - 1u, 1u));
    destination[n] = sample_alpha(p.source.x, p.source.y, x, y);
}
@compute @workgroup_size(256)
fn rotate_tile(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = index_of(id);
    if n >= p.rotated.x * p.rotated.y { return; }
    let x = n % p.rotated.x;
    let y = n / p.rotated.x;
    let dx = f32(x) - f32(p.rotated.x - 1u) / 2.0;
    let dy = f32(y) - f32(p.rotated.y - 1u) / 2.0;
    let sx = p.geometry.z * dx - p.geometry.y * dy + f32(p.source.z - 1u) / 2.0;
    let sy = p.geometry.y * dx + p.geometry.z * dy + f32(p.source.w - 1u) / 2.0;
    let a = sample_alpha(p.source.z, p.source.w, sx, sy);
    destination[n] = a;
    if a > 1e-4 {
        atomicMin(&bounds.xmin, x);
        atomicMax(&bounds.xmax, x);
        atomicMin(&bounds.ymin, y);
        atomicMax(&bounds.ymax, y);
    }
}
