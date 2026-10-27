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
    ox: u32,
    oy: u32,
    monochrome: u32,
    strength: vec4<f32>,
};
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> density: array<f32>;
@group(0) @binding(2) var<storage, read> shared_field: array<f32>;
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

// Exact Poisson sampling, matching the host `poisson_exact`: inversion below 30,
// PTRS above. A normal approximation would keep the mean and variance at large rates
// but changes the distribution once the rate is small, which a heavily oversampled
// geometry reaches.
// ln Gamma(z) for z >= 1 by the Lanczos approximation; WGSL has no lgamma.
fn log_gamma(z: f32) -> f32 {
    let g = 7.0;
    let c = array<f32, 9>(
        0.99999999999980993,
        676.5203681218851,
        -1259.1392167224028,
        771.32342877765313,
        -176.61502916214059,
        12.507343278686905,
        -0.13857109526572012,
        9.9843695780195716e-6,
        1.5056327351493116e-7,
    );
    var zz = z - 1.0;
    var x = c[0];
    for (var i = 1u; i < 9u; i++) {
        x = x + c[i] / (zz + f32(i));
    }
    let tt = zz + g + 0.5;
    return 0.5 * log(2.0 * 3.141592653589793) + (zz + 0.5) * log(tt) - tt + log(x);
}

fn poisson_uniform(seed: u32, stream: u32, x: u32, y: u32, n: u32) -> f32 {
    let h = hash_u32(x ^ hash_u32(y ^ hash_u32(n ^ hash_u32(seed ^ hash_u32(stream)))));
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

fn poisson_exact(lambda: f32, seed: u32, stream: u32, x: u32, y: u32) -> f32 {
    if (lambda <= 0.0) { return 0.0; }
    if (lambda < 30.0) {
        let limit = exp(-lambda);
        var product = 1.0;
        var count = 0.0;
        var n = 0u;
        loop {
            n = n + 1u;
            product = product * poisson_uniform(seed, stream, x, y, n);
            if (product <= limit) { return count; }
            count = count + 1.0;
            if (count > 4096.0) { return count; }
        }
        return count;
    }
    let slam = sqrt(lambda);
    let loglam = log(lambda);
    let b = 0.931 + 2.53 * slam;
    let a = -0.059 + 0.02483 * b;
    let inv_alpha = 1.1239 + 1.1328 / (b - 3.4);
    let vr = 0.9277 - 3.6224 / (b - 2.0);
    let log_inv_alpha = log(inv_alpha);
    var n = 0u;
    loop {
        n = n + 1u;
        let u = poisson_uniform(seed, stream, x, y, n) - 0.5;
        n = n + 1u;
        let v = poisson_uniform(seed, stream, x, y, n);
        let us = 0.5 - abs(u);
        if (us <= 0.0) { continue; }
        let k = floor((2.0 * a / us + b) * u + lambda + 0.43);
        if (k < 0.0) { continue; }
        if (us < 0.013 && v > us) { continue; }
        if (us >= 0.07 && v <= vr) { return k; }
        let lhs = log(v) + log_inv_alpha - log(a / (us * us) + b);
        let rhs = -lambda + k * loglam - log_gamma(k + 1.0);
        if (lhs <= rhs) { return k; }
        if (n > 512u) { return max(lambda, 0.0); }
    }
    return max(lambda, 0.0);
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
    let s = shared_field[i];
    for (var c = 0u; c < 3u; c++) {
        let t = density[i * 3u + c];
        // One effective record: every lane reads the first record's field, strength
        // and count draw, so a monochrome realization is identical in all three.
        let mono = p.monochrome != 0u;
        let own_c = select(own[c], own[0], mono);
        let strength = select(p.strength[c], 1.0, mono);
        let wi = a * s + b * own_c;
        let rel = p.white * strength * taper(t);
        let e = 1.0 + rel * wi;
        if (p.expectation != 0u) {
            out[i * 3u + c] = t;
            continue;
        }
        let lambda = max((t + p.fog) * e * p.area / p.q, 0.0);
        let stream = select(100u + c, 100u, mono);
        let count = poisson_exact(lambda, p.seed, stream, x + p.ox, y + p.oy);
        out[i * 3u + c] = p.q * count / p.area - p.fog;
    }
}
