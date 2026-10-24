struct Params {
    frame: vec4<u32>, // width, height, candidate limit, capacity
    basis: vec4<f32>, // basis[0].xy, basis[1].xy
    geometry: vec4<f32>, // stamp radius, unused
}
struct Centers { count: u32, unused: array<u32, 3>, points: array<vec2<f32>> }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> centers: Centers;
// Rust f32::total_cmp, including the ordering of signed zero.
fn ordered(value: f32) -> u32 {
    let bits = bitcast<u32>(value);
    return select(bits ^ 0x80000000u, ~bits, (bits & 0x80000000u) != 0u);
}
fn before(a: vec2<f32>, b: vec2<f32>) -> bool {
    let ax = ordered(a.x);
    let bx = ordered(b.x);
    return ax < bx || (ax == bx && ordered(a.y) < ordered(b.y));
}
@compute @workgroup_size(1)
fn main() {
    var count = 0u;
    let limit = i32(p.frame.z);
    for (var i = -limit; i <= limit; i++) {
        for (var j = -limit; j <= limit; j++) {
            let x = f32(p.frame.x) * 0.5 + f32(i) * p.basis.x + f32(j) * p.basis.z;
            let y = f32(p.frame.y) * 0.5 + f32(i) * p.basis.y + f32(j) * p.basis.w;
            if x >= -p.geometry.x && x <= f32(p.frame.x) + p.geometry.x
                && y >= -p.geometry.x && y <= f32(p.frame.y) + p.geometry.x {
                let point = vec2<f32>(x, y);
                // Stable insertion preserves the CPU's i-major/j-minor generation order for ties.
                var k = count;
                loop {
                    if k == 0u { break; }
                    if !before(point, centers.points[k - 1u]) { break; }
                    centers.points[k] = centers.points[k - 1u];
                    k--;
                }
                centers.points[k] = point;
                count++;
            }
        }
    }
    centers.count = count;
}
