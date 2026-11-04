// Spektra statistical PCG particle fields. Curve interpolation uses f64
// tables, then floor addition, conditioning and count products use f32 exactly
// like the reference working arrays. NumPy clumping continues from the final
// plane's MT19937 seed/state.

struct Params {
  width: u32,
  height: u32,
  knots: u32,
  layers: u32,
  base_seed: u64,
  pad_seed: u64,
  uniformity: vec3<f64>,
  // WGSL requires an array element stride to be a multiple of 16 in the
  // uniform address space, so each 9-value table is three `vec4` rows with a
  // padding lane the host leaves zero.
  density_min_layers: array<vec4<f64>, 3>,
  density_max_adj: array<vec4<f64>, 3>,
  n_particles: array<vec4<f64>, 3>,
  lookup_sign: f32,
  flags: vec4<u32>,
  tail_padding: vec4<u32>,
}

@group(0) @binding(0) var<uniform> p: Params;
// Film-density input remains unchanged until the final composition pass.
@group(0) @binding(1) var<storage, read> density_cmy: array<f32>;
// Composite curve x column per channel: `knots` rows × 3 channels, already
// negated for a positive film by the host.
@group(0) @binding(2) var<storage, read> curve_x: array<f64>;
// Layer curve rows: `knots` rows × 3 sublayers × 3 channels.
@group(0) @binding(3) var<storage, read> curve_layers: array<f64>;
@group(0) @binding(4) var<storage, read_write> layer_fields: array<f32>;

// The local WGPU HAL uses this named function to select strict Metal math.
// Every Poisson and binomial decision below is conditioned on an f64 value
// whose last bits change the reference's discrete outcome, so no contraction
// or reassociation is allowed anywhere in this module.
fn spektrafilm_strict_float(v: f64) -> f64 {
  return v;
}

// Spektra statistical PCG realization.
//
// Selected only by `film_render.grain.spektra_sampling = statistical_pcg`.
// Each pixel owns a PCG32 stream; Poisson and binomial use the reference's
// exact branches with f32 working arithmetic. The default MT19937 and
// fast-statistics paths remain in `spektra.wgsl` and are not changed.
//
fn bracket(x: f64, ch: u32) -> vec2<f64> {
  let k = p.knots;
  let first = curve_x[ch];
  let last = curve_x[(k - 1u) * 3u + ch];
  if x <= first {
    return vec2<f64>(0.0, 0.0);
  }
  if x >= last {
    return vec2<f64>(f64(k - 2u), 1.0);
  }
  var lo = 0u;
  var hi = k - 1u;
  loop {
    if hi - lo <= 1u {
      break;
    }
    let mid = (lo + hi) / 2u;
    if curve_x[mid * 3u + ch] <= x {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  let xa = curve_x[lo * 3u + ch];
  let xb = curve_x[hi * 3u + ch];
  let dx = xb - xa;
  var t: f64 = 0.0;
  if dx != 0.0 {
    let inverse = 1.0 / dx;
    t = (x - xa) * inverse;
  }
  return vec2<f64>(f64(lo), t);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let n = p.tail_padding.y;
  let i = gid.x + gid.y * 65535u * 256u;
  if i >= n { return; }
  let global_i = p.tail_padding.x + i;
  let channels = select(3u, 1u, p.flags.y != 0u);
  for (var ch = 0u; ch < channels; ch = ch + 1u) {
    let gu = f32(p.uniformity[ch]);
    let x = f64(density_cmy[i * 3u + ch]) * f64(p.lookup_sign);
    let b = bracket(x, ch);
    let lo = u32(b.x);
    for (var sl = 0u; sl < p.layers; sl = sl + 1u) {
      let dmax = f32(p.density_max_adj[sl][ch]);
      let npp = f32(p.n_particles[sl][ch]);
      let od = f32(p.density_max_adj[sl][ch] / p.n_particles[sl][ch]);
      var interpolated: f32;
      if x <= curve_x[ch] {
        interpolated = f32(curve_layers[sl * 3u + ch]);
      } else if x >= curve_x[(p.knots - 1u) * 3u + ch] {
        interpolated = f32(curve_layers[((p.knots - 1u) * 3u + sl) * 3u + ch]);
      } else {
        let y0 = curve_layers[(lo * 3u + sl) * 3u + ch];
        let y1 = curve_layers[((lo + 1u) * 3u + sl) * 3u + ch];
        interpolated = f32(y0 + b.y * (y1 - y0));
      }
      let density_sl = f32(sf64_from_f32(interpolated) + p.density_min_layers[sl][ch]);
      let prob = clamp(sf32_div(density_sl, dmax), 1e-6f, 1.0f - 1e-6f);
      let saturation = sf32_sub(1.0f, sf32_mul(sf32_mul(prob, gu), 1.0f - 1e-6f));
      let lam = sf32_div(npp, saturation);
      let layer_seed = u32(p.base_seed) + ch + sl * 10u;
      var rng = Pcg(
        pcg_splitmix(layer_seed) ^ pcg_splitmix(global_i),
        (pcg_splitmix(global_i ^ 0x9e3779b9u) << 1u) | 1u,
      );
      let seeds = pcg_poisson(f32(lam), &rng);
      let developed = pcg_binomial(seeds, f32(prob), &rng);
      let particle = developed * od;
      let value = particle * saturation;
      layer_fields[(sl * n + i) * 3u + ch] = value;
      if p.flags.y != 0u {
        layer_fields[(sl * n + i) * 3u + 1u] = value;
        layer_fields[(sl * n + i) * 3u + 2u] = value;
      }
    }
  }
}

// ── f32 PCG32 exact branch (Negs-style coordinate address) ────────────────
// Same Poisson/binomial rejection branches as the reference, but each pixel
// owns its stream and the working arithmetic is f32.

fn pcg_splitmix(x0: u32) -> u32 {
  var z = x0;
  z = (z ^ (z >> 16u)) * 0x85ebca6bu;
  z = (z ^ (z >> 13u)) * 0xc2b2ae35u;
  return z ^ (z >> 16u);
}
struct Pcg { state: u32, inc: u32 }
fn pcg_next(rng: ptr<function, Pcg>) -> u32 {
  let s = (*rng).state;
  (*rng).state = s * 747796405u + (*rng).inc;
  let word = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
  return (word >> 22u) ^ word;
}
fn pcg_uniform(rng: ptr<function, Pcg>) -> f32 {
  return (f32(pcg_next(rng) >> 8u) + 0.5) * (1.0 / 16777216.0);
}
fn pcg_log_factorial(k: f32) -> f32 {
  if k < 8.0 {
    var r = 1.0;
    for (var i = 2.0; i <= k; i += 1.0) { r *= i; }
    return log(r);
  }
  let x = k + 1.0;
  return (x - 0.5) * log(x) - x + 0.91893853320467274
    + 1.0 / (12.0 * x) - 1.0 / (360.0 * x * x * x);
}
fn pcg_poisson(lam: f32, rng: ptr<function, Pcg>) -> f32 {
  if lam <= 0.0 { return 0.0; }
  if lam < 10.0 {
    let enlam = exp(-lam);
    var x = 0.0;
    var prod = 1.0;
    loop {
      prod *= pcg_uniform(rng);
      if prod > enlam { x += 1.0; } else { return x; }
    }
  }
  let slam = sqrt(lam);
  let loglam = log(lam);
  let b = 0.931 + 2.53 * slam;
  let a = -0.059 + 0.02483 * b;
  let invalpha = 1.1239 + 1.1328 / (b - 3.4);
  let vr = 0.9277 - 3.6224 / (b - 2.0);
  loop {
    let u = pcg_uniform(rng) - 0.5;
    let v = pcg_uniform(rng);
    let us = 0.5 - abs(u);
    let k = floor((2.0 * a / us + b) * u + lam + 0.43);
    if us >= 0.07 && v <= vr { return k; }
    if k < 0.0 || (us < 0.013 && v > us) { continue; }
    let lhs = log(v) + log(invalpha) - log(a / (us * us) + b);
    let rhs = -lam + k * loglam - pcg_log_factorial(k);
    if lhs <= rhs { return k; }
  }
  return 0.0;
}
fn pcg_binomial_inversion(n: f32, p: f32, rng: ptr<function, Pcg>) -> f32 {
  let q = 1.0 - p;
  let qn = exp(n * log(q));
  let np = n * p;
  let bound = floor(min(n, np + 10.0 * sqrt(np * q + 1.0)));
  var x = 0.0;
  var px = qn;
  var u = pcg_uniform(rng);
  loop {
    if u <= px { return x; }
    x += 1.0;
    if x > bound { x = 0.0; px = qn; u = pcg_uniform(rng); }
    else { u -= px; px = ((n - x + 1.0) * p * px) / (x * q); }
  }
  return 0.0;
}
fn pcg_stirling(x: f32) -> f32 {
  let x2 = x * x;
  return (13680.0 - (462.0 - (132.0 - (99.0 - 140.0 / x2) / x2) / x2) / x2) / x / 166320.0;
}
fn pcg_binomial_btpe(n: f32, p: f32, rng: ptr<function, Pcg>) -> f32 {
  let r = p;
  let q = 1.0 - r;
  let fm = n * r + r;
  let m = floor(fm);
  let p1 = floor(2.195 * sqrt(n * r * q) - 4.6 * q) + 0.5;
  let xm = m + 0.5;
  let xl = xm - p1;
  let xr = xm + p1;
  let c = 0.134 + 20.5 / (15.3 + m);
  let a1 = (fm - xl) / (fm - xl * r);
  let laml = a1 * (1.0 + a1 / 2.0);
  let a2 = (xr - fm) / (xr * q);
  let lamr = a2 * (1.0 + a2 / 2.0);
  let p2 = p1 * (1.0 + 2.0 * c);
  let p3 = p2 + c / laml;
  let p4 = p3 + c / lamr;
  let nrq = n * r * q;
  loop {
    let u = pcg_uniform(rng) * p4;
    var v = pcg_uniform(rng);
    var y: f32;
    if u <= p1 {
      return floor(xm - p1 * v + u);
    } else if u <= p2 {
      let x = xl + (u - p1) / c;
      v = v * c + 1.0 - abs(m - x + 0.5) / p1;
      if v > 1.0 { continue; }
      y = floor(x);
    } else if u <= p3 {
      y = floor(xl + log(v) / laml);
      if y < 0.0 { continue; }
      v *= (u - p2) * laml;
    } else {
      y = floor(xr - log(v) / lamr);
      if y > n { continue; }
      v *= (u - p3) * lamr;
    }
    let k = abs(y - m);
    if k <= 20.0 || k >= nrq / 2.0 - 1.0 {
      let s = r / q;
      let a = s * (n + 1.0);
      var f = 1.0;
      if m < y {
        for (var i = m + 1.0; i <= y; i += 1.0) { f *= a / i - s; }
      } else if m > y {
        for (var i = y + 1.0; i <= m; i += 1.0) { f /= a / i - s; }
      }
      if v > f { continue; }
      return y;
    }
    let rho = (k / nrq) * ((k * (k / 3.0 + 0.625) + 0.16666666666666666) / nrq + 0.5);
    let t = -k * k / (2.0 * nrq);
    let a_log = log(v);
    if a_log < t - rho { return y; }
    if a_log > t + rho { continue; }
    let x1 = y + 1.0;
    let f1 = m + 1.0;
    let z = n - m + 1.0;
    let w = n - y + 1.0;
    let bound = xm * log(f1 / x1) + (n - m + 0.5) * log(z / w)
      + (y - m) * log(w * r / (x1 * q))
      + pcg_stirling(f1) + pcg_stirling(z) + pcg_stirling(x1) + pcg_stirling(w);
    if a_log > bound { continue; }
    return y;
  }
  return 0.0;
}
fn pcg_binomial(n: f32, p: f32, rng: ptr<function, Pcg>) -> f32 {
  if n <= 0.0 { return 0.0; }
  if p <= 0.5 {
    if p * n <= 30.0 { return pcg_binomial_inversion(n, p, rng); }
    return pcg_binomial_btpe(n, p, rng);
  }
  let q = 1.0 - p;
  if q * n <= 30.0 { return n - pcg_binomial_inversion(n, q, rng); }
  return n - pcg_binomial_btpe(n, q, rng);
}
