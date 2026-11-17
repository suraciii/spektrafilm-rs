// The final Dye density: the developed record density is split into the
// profile's density layers, each layer modulates its own record with the
// correlated field and count, and the layer densities sum back to the target.
//
//   t_c = sum_l t_c,l
//   rel   = white * strength[c] * taper(t_c,l)
//   wi    = sqrt(rho) * shared_l + sqrt(1-rho) * own_l
//   e     = 1 + rel * wi
//   lambda = (t_c,l + fog) * e * area / q
//   count  = Poisson(lambda)
//   D      = sum_l (q * count / area - fog)
//
// The count is drawn exactly: inversion below thirty events, PTRS above,
// matching the host sampler. A monochrome profile reads the first record in
// every lane.

struct P {
    w: u32,
    h: u32,
    seed: u32,
    expectation: u32,
    white: f32,
    rho: f32,
    taper_peak: f32,
    uniformity: f32,
    q: f32,
    area: f32,
    fog: f32,
    ox: u32,
    oy: u32,
    monochrome: u32,
    layer_count: u32,
    layer_rows: u32,
    layer_sign: f32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
    pad3: u32,
    pad4: u32,
    pad5: u32,
    pad6: u32,
    strength: vec4<f32>,
    dmax: array<vec4<f32>, 3>,
};
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> density: array<f32>;
@group(0) @binding(2) var<storage, read> shared_field: array<f32>;
@group(0) @binding(3) var<storage, read> f_r: array<f32>;
@group(0) @binding(4) var<storage, read> f_g: array<f32>;
@group(0) @binding(5) var<storage, read> f_b: array<f32>;
@group(0) @binding(6) var<storage, read_write> out: array<f32>;
@group(0) @binding(7) var<storage, read> layer_data: array<f32>;

fn axis_value(row: u32, channel: u32) -> f32 {
    return layer_data[row * 4u + channel];
}

fn curve_value(row: u32, layer: u32, channel: u32) -> f32 {
    let base = p.layer_rows * 4u;
    return layer_data[base + (row * 3u + layer) * 4u + channel];
}

fn interpolate_layer(density_value: f32, channel: u32, layer: u32) -> f32 {
    if (p.layer_rows < 2u) { return density_value; }
    let last = p.layer_rows - 1u;
    let x = density_value * p.layer_sign;
    let first = axis_value(0u, channel) * p.layer_sign;
    if (x <= first) { return curve_value(0u, layer, channel); }
    let last_value = axis_value(last, channel) * p.layer_sign;
    if (x >= last_value) { return curve_value(last, layer, channel); }
    var lo = 0u;
    var hi = last;
    loop {
        if (lo + 1u >= hi) { break; }
        let mid = (lo + hi) / 2u;
        if (axis_value(mid, channel) * p.layer_sign <= x) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let x0 = axis_value(lo, channel) * p.layer_sign;
    let x1 = axis_value(hi, channel) * p.layer_sign;
    let a = curve_value(lo, layer, channel);
    if (x1 <= x0) { return a; }
    let t = (x - x0) / (x1 - x0);
    let b = curve_value(hi, layer, channel);
    return a + t * (b - a);
}

fn hash_u32(x: u32) -> u32 {
    var v = x * 747796405u + 2891336453u;
    v = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
    return (v >> 22u) ^ v;
}

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

// Coordinate-addressed standard normal over the same samples as the host
// `normal01_coordinate`: ordinals 1 and 2.
fn normal01(seed: u32, stream: u32, x: u32, y: u32) -> f32 {
    let u1 = max(poisson_uniform(seed, stream, x, y, 1u), 1e-12);
    let u2 = poisson_uniform(seed, stream, x, y, 2u);
    return sqrt(-2.0 * log(u1)) * cos(6.283185307179586 * u2);
}

// Above this rate both executors draw the normal deviation instead of the
// exact Poisson count. Keep in sync with `dye::LARGE_RATE_THRESHOLD`.
const LARGE_RATE_THRESHOLD: f32 = 1.0e9;

// Exact Poisson sampling, matching the host `poisson_exact`: inversion below 30,
// PTRS above. A normal approximation keeps the mean and variance at large rates but
// changes the distribution once the rate is small, which a heavily oversampled
// geometry reaches.
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

// Normalized layer variance envelope. p is the developed fraction of the
// sampled layer peak; the asymptotic zero is at p = 1 / uniformity, above
// the sampled peak, so the measured peak keeps a positive residual.
fn taper(t: f32, dmax: f32) -> f32 {
    let pd = clamp(t / dmax, 0.0, 1.0 / p.uniformity);
    let shape = sqrt(max(pd * (1.0 - p.uniformity * pd), 0.0));
    return shape / p.taper_peak;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= p.w || y >= p.h) { return; }
    let i = y * p.w + x;
    let n = p.w * p.h;
    let a = sqrt(p.rho);
    let b = sqrt(1.0 - p.rho);
    let mono = p.monochrome != 0u;
    let layers = max(p.layer_count, 1u);
    for (var c = 0u; c < 3u; c++) {
        let t = density[i * 3u + c];
        if (p.expectation != 0u || p.q <= 0.0) {
            out[i * 3u + c] = t;
            continue;
        }
        var values = array<f32, 3>(0.0, 0.0, 0.0);
        var sum = 0.0;
        if (layers > 1u) {
            for (var layer = 0u; layer < layers; layer++) {
                values[layer] = interpolate_layer(t, c, layer);
                sum = sum + values[layer];
            }
        } else {
            values[0] = t;
            sum = t;
        }
        var realized = 0.0;
        for (var layer = 0u; layer < layers; layer++) {
            var layer_density = 0.0;
            if (t > 0.0) {
                if (sum > 0.0) {
                    layer_density = values[layer] * t / sum;
                } else {
                    // Match the host: keep a positive target when the
                    // profile's layer curves are all zero at this density.
                    layer_density = t / f32(layers);
                }
            }
            let offset = layer * n + i;
            var own = array<f32, 3>(f_r[offset], f_g[offset], f_b[offset]);
            let own_c = select(own[c], own[0], mono);
            let strength = select(p.strength[c], 1.0, mono);
            let s = shared_field[offset];
            let wi = a * s + b * own_c;
            let rel = p.white * strength * taper(layer_density, p.dmax[layer][c]);
            let e = max(1.0 + rel * wi, 0.02);
            let expected = (layer_density + p.fog) * e;
            let lambda = max(expected * p.area / p.q, 0.0);
            let stream = select(100u + layer * 4u + c, 100u + layer * 4u, mono);
            if (lambda > LARGE_RATE_THRESHOLD) {
                let variance = max(p.q * expected / p.area, 0.0);
                let z = normal01(p.seed, stream, x + p.ox, y + p.oy);
                realized = realized + expected - p.fog + sqrt(variance) * z;
                continue;
            }
            let count = poisson_exact(lambda, p.seed, stream, x + p.ox, y + p.oy);
            realized = realized + p.q * count / p.area - p.fog;
        }
        out[i * 3u + c] = realized;
    }
}
