// Separable Gaussian pass of the Dye grain law's correlated field.
//
// The field is white noise filtered by a Gaussian; the noise is generated here
// from the pixel coordinates and the stream, so no noise buffer is stored. Taps
// are normalised on the host; `use_noise` selects the first pass (no source
// field) from the following ones.

struct P {
    w: u32,
    h: u32,
    radius: u32,
    axis: u32,
    stream: u32,
    seed: u32,
    use_noise: u32,
    pad: u32,
    ox: u32,
    oy: u32,
    pad2: u32,
    pad3: u32,
};
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> taps: array<f32>;
@group(0) @binding(2) var<storage, read> src: array<f32>;
@group(0) @binding(3) var<storage, read_write> dst: array<f32>;

fn hash_u32(x: u32) -> u32 {
    var v = x * 747796405u + 2891336453u;
    v = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
    return (v >> 22u) ^ v;
}

fn uniform01(seed: u32, stream: u32, x: u32, y: u32) -> f32 {
    let h = hash_u32(x ^ hash_u32(y ^ hash_u32(seed ^ hash_u32(stream))));
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

fn normal01(seed: u32, stream: u32, x: u32, y: u32) -> f32 {
    let u1 = max(uniform01(seed, stream, x, y), 1e-6);
    let u2 = uniform01(seed, stream ^ 0x9e3779b9u, y, x);
    return sqrt(-2.0 * log(u1)) * cos(6.283185307 * u2);
}

fn sample(x: u32, y: u32) -> f32 {
    if (p.use_noise != 0u) {
        return normal01(p.seed, p.stream, x + p.ox, y + p.oy);
    }
    return src[y * p.w + x];
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= p.w || y >= p.h) { return; }
    var acc = 0.0;
    for (var t = 0u; t <= 2u * p.radius; t++) {
        let off = i32(t) - i32(p.radius);
        var sx = i32(x);
        var sy = i32(y);
        if (p.axis == 0u) { sx = clamp(sx + off, 0, i32(p.w) - 1); } else { sy = clamp(sy + off, 0, i32(p.h) - 1); }
        acc = acc + taps[t] * sample(u32(sx), u32(sy));
    }
    dst[y * p.w + x] = acc;
}
