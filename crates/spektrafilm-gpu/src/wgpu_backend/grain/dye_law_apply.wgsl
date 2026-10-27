// The final Dye density: the record density t is modulated by the correlated
// field and by the event count, per channel, and the mean stays exactly t.
//
//   rel   = white * strength[c] * taper(t)          (relative density amplitude)
//   wi    = sqrt(rho) * shared + sqrt(1-rho) * own  (record coupling)
//   e     = 1 + rel * wi
//   count = (t + fog) * e * area / q                (expected events)
//   D     = q * (count + sqrt(count) * z) / area - fog
//
// The Poisson draw is the normal approximation: with q = 1.5e-3 um^2 and a 25 um^2
// cell the expected count is in the thousands, where the two agree to well under a
// part in a thousand.

struct P {
    w: u32,
    h: u32,
    seed: u32,
    expectation: u32,
    white: f32,
    rho: f32,
    taper_peak: f32,
    taper_floor: f32,
    dmax: f32,
    uniformity: f32,
    q: f32,
    area: f32,
    fog: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
    strength: vec4<f32>,
};
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> density: array<f32>;
@group(0) @binding(2) var<storage, read> shared: array<f32>;
@group(0) @binding(3) var<storage, read> f_r: array<f32>;
@group(0) @binding(4) var<storage, read> f_g: array<f32>;
@group(0) @binding(5) var<storage, read> f_b: array<f32>;
@group(0) @binding(6) var<storage, read_write> out: array<f32>;

fn hash_u32(x: u32) -> u32 {
    var v = x * 747796405u + 2891336453u;
    v = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
    return (v >> 22u) ^ v;
}

fn uniform01(stream: u32, x: u32, y: u32) -> f32 {
    let h = hash_u32(x ^ hash_u32(y ^ hash_u32(p.seed ^ hash_u32(stream))));
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

fn normal01(stream: u32, x: u32, y: u32) -> f32 {
    let u1 = max(uniform01(stream, x, y), 1e-6);
    let u2 = uniform01(stream ^ 0x9e3779b9u, y, x);
    return sqrt(-2.0 * log(u1)) * cos(6.283185307 * u2);
}

fn taper(t: f32) -> f32 {
    let pd = clamp(t / p.dmax, 0.0, 1.0);
    let sat = 1.0 - pd * p.uniformity;
    let shape = sqrt(max(sat * pd * (1.0 - pd), 0.0));
    return max(shape / p.taper_peak, p.taper_floor);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= p.w || y >= p.h) { return; }
    let i = y * p.w + x;
    let a = sqrt(p.rho);
    let b = sqrt(1.0 - p.rho);
    let own = array<f32, 3>(f_r[i], f_g[i], f_b[i]);
    let s = shared[i];
    for (var c = 0u; c < 3u; c++) {
        let t = density[i * 3u + c];
        let wi = a * s + b * own[c];
        let rel = p.white * p.strength[c] * taper(t);
        let e = 1.0 + rel * wi;
        if (p.expectation != 0u) {
            out[i * 3u + c] = t;
            continue;
        }
        let lambda = max((t + p.fog) * e * p.area / p.q, 0.0);
        let z = normal01(100u + c, x, y);
        let count = lambda + sqrt(lambda) * z;
        out[i * 3u + c] = p.q * count / p.area - p.fog;
    }
}
