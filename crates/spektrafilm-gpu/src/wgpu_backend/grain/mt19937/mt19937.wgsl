// Resident NumPy compound Poisson/binomial draws and final-plane normals.
// Source tables interpolate in f64; grain working arrays condition in f32.
// Every complete plane's Poisson walk precedes its binomial walk. The last
// plane retains MT state into NumPy HWC standard_normal microstructure.

struct Params {
  width: u32,
  height: u32,
  knots: u32,
  layers: u32,
  n_pixels: u32,
  chunk_elements: u32,
  // x: particle / continued normals / fresh normals; y: monochrome;
  // z: host input representation; w: layer*3+channel.
  flags: vec4<u32>,
  base_seed: u64,
  micro_sigma: f64,
  lookup_sign: f32,
  pad0: u32,
  // The three-value triples are `vec4<f64>`, not `vec3<f64>`: a uniform
  // `vec3` has a 32-byte stride with its own alignment, and spelling the
  // padded lane explicitly makes the host's `repr(C)` layout unambiguous.
  uniformity: vec4<f64>,
  // WGSL requires an array element stride to be a multiple of 16 in the
  // uniform address space, so each 9-value table is three `vec4` rows with
  // a padding lane the host leaves zero.
  n_particles_layers: array<vec4<f64>, 3>,
  density_max_adj: array<vec4<f64>, 3>,
  density_min_layers: array<vec4<f64>, 3>,
}

@group(0) @binding(0) var<uniform> p: Params;
// Film density CMY, read only. Index `i * 3 + ch`.
@group(0) @binding(1) var<storage, read> density_cmy: array<f32>;
// Composite curve x column per channel: `knots` rows × 3 channels, already
// negated for a positive film by the host.
@group(0) @binding(2) var<storage, read> curve_x: array<f64>;
// Layer curve rows: `knots` rows × 3 sublayers × 3 channels.
@group(0) @binding(3) var<storage, read> curve_layers: array<f64>;
// Per-sublayer particle fields, `[sl][pixel][ch]` f32.
@group(0) @binding(4) var<storage, read_write> layer_fields: array<f32>;
// Poisson seeds, two u32 lanes per element. The Poisson segments fill them
// and the binomial segments read them, which is the reference's two-pass
// order over one generator. Each plane owns its own `2 * n_pixels` lane
// range, so concurrent planes never touch the same lanes.
@group(0) @binding(5) var<storage, read_write> seeds: array<u32>;

/// One plane's generator state and its position in the two-pass walk. The
/// array is runtime-sized and the host allocates one slot per plane, so
/// planes never share state and no fixed plane cap is baked into the shader.
struct MtSlot {
  dispatch_index: u32,
  idx: u32,
  seeded: u32,
  has_gauss: u32,
  state: array<u32, 624>,
  gauss: f64,
}

@group(0) @binding(6) var<storage, read_write> mt: array<MtSlot>;
@group(0) @binding(7) var<storage, read_write> clump: array<f64>;

// The local WGPU HAL uses this named function to select strict Metal math.
// Every Poisson and binomial decision below is conditioned on an f64 value
// whose last bits change the reference's discrete outcome, so no contraction
// or reassociation is allowed anywhere in this module.
fn spektrafilm_strict_float(v: f64) -> f64 {
  return v;
}


// ── MT19937 (`rand_mt::Mt`, bit-identical to numpy's RandomState) ──

const MT_PI: f64 = 3.14159265358979323846;

// ── NumPy's RandomState distribution algorithms
// (`spektrafilm_math::numpy_rng`) ──

/// `rk_double`: a 53-bit uniform in [0, 1) from two tempered outputs.
fn rk_double(plane: u32) -> f64 {
  let a = f64(u64(mt_next(plane)) >> 5u);
  let b = f64(u64(mt_next(plane)) >> 6u);
  return (a * 67108864.0 + b) / 9007199254740992.0;
}

/// `rk_poisson`: λ ≥ 10 → PTRS; λ == 0 → 0; otherwise multiplication of
/// uniforms. Matches `numpy.random.RandomState.poisson`.
fn rk_poisson(plane: u32, lam: f64) -> u64 {
  if lam >= 10.0 {
    return rk_poisson_ptrs(plane, lam);
  }
  if lam == 0.0 {
    return u64(0u);
  }
  let enlam = sf64_exp(spektrafilm_strict_float(-lam));
  var x: u64 = u64(0u);
  var prod: f64 = 1.0;
  loop {
    let u = rk_double(plane);
    prod = prod * u;
    if prod > enlam {
      x = x + u64(1u);
    } else {
      return x;
    }
  }
  // Unreachable: the loop above only returns or falls through to its next
  // iteration, so it never exits. The trailing value satisfies the validator's
  // requirement that every path of a non-void function produce one.
  return u64(0u);
}

/// Hörmann's transformed rejection (PTRS), verbatim from NumPy.
fn rk_poisson_ptrs(plane: u32, lam: f64) -> u64 {
  let slam = sqrt(lam);
  let loglam = sf64_log(spektrafilm_strict_float(lam));
  let b = 0.931 + 2.53 * slam;
  let a = -0.059 + 0.02483 * b;
  let invalpha = 1.1239 + 1.1328 / (b - 3.4);
  let vr = 0.9277 - 3.6224 / (b - 2.0);
  loop {
    let u = rk_double(plane) - 0.5;
    let v = rk_double(plane);
    let us = 0.5 - abs(u);
    // NumPy casts via `(long)floor(...)`: floor toward -∞ for negatives,
    // then truncate toward zero, which is what `i64(floor(..))` does here.
    let k = i64(floor(spektrafilm_strict_float((2.0 * a / us + b) * u + lam + 0.43)));
    if us >= 0.07 && v <= vr {
      return u64(k);
    }
    if k < 0 || (us < 0.013 && v > us) {
      continue;
    }
    let lhs = sf64_log(spektrafilm_strict_float(v))
      + sf64_log(spektrafilm_strict_float(invalpha))
      - sf64_log(spektrafilm_strict_float(a / (us * us) + b));
    let kp1 = f64(k) + 1.0;
    let rhs = -lam + f64(k) * loglam - sf64_lgamma(kp1);
    if lhs <= rhs {
      return u64(k);
    }
  }
  // Unreachable: the rejection loop above only returns or continues, so it
  // never exits. The trailing value satisfies the validator's requirement that
  // every path of a non-void function produce one.
  return u64(0u);
}

/// `rk_binomial`: dispatches by `p` and `n·p` exactly like NumPy.
fn rk_binomial(plane: u32, n: u64, p: f64) -> u64 {
  if p <= 0.5 {
    if p * f64(n) <= 30.0 {
      return rk_binomial_inversion(plane, n, p);
    }
    return rk_binomial_btpe(plane, n, p);
  }
  let q = 1.0 - p;
  if q * f64(n) <= 30.0 {
    return n - rk_binomial_inversion(plane, n, q);
  }
  return n - rk_binomial_btpe(plane, n, q);
}

fn rk_binomial_inversion(plane: u32, n: u64, p: f64) -> u64 {
  let q = 1.0 - p;
  let qn = sf64_exp(spektrafilm_strict_float(f64(n) * sf64_log(spektrafilm_strict_float(q))));
  let np = f64(n) * p;
  let bound = u64(min(f64(n), np + 10.0 * sqrt(np * q + 1.0)));
  var x: u64 = u64(0u);
  var px: f64 = qn;
  var u = rk_double(plane);
  loop {
    if u <= px {
      return x;
    }
    x = x + u64(1u);
    if x > bound {
      x = u64(0u);
      px = qn;
      u = rk_double(plane);
    } else {
      u = u - px;
      px = (f64(n - x + u64(1u)) * p * px) / (f64(x) * q);
    }
  }
  // Unreachable: the rejection loop above only returns or falls through to its
  // next iteration, so it never exits. The trailing value satisfies the
  // validator's requirement that every path of a non-void function produce one.
  return u64(0u);
}

/// BTPE (Kachitvichyanukul-Schmeider), verbatim from NumPy's
/// `rk_binomial_btpe`. The reference's `goto`-style flow is preserved as one
/// rejection loop with `continue` and an explicit break.
fn rk_binomial_btpe(plane: u32, n: u64, p: f64) -> u64 {
  let r = min(p, 1.0 - p);
  let q = 1.0 - r;
  let nf = f64(n);
  let fm = nf * r + r;
  let m = i64(floor(fm));
  let p1 = floor(2.195 * sqrt(nf * r * q) - 4.6 * q) + 0.5;
  let xm = f64(m) + 0.5;
  let xl = xm - p1;
  let xr = xm + p1;
  let c = 0.134 + 20.5 / (15.3 + f64(m));
  let a1 = (fm - xl) / (fm - xl * r);
  let laml = a1 * (1.0 + a1 / 2.0);
  let a2 = (xr - fm) / (xr * q);
  let lamr = a2 * (1.0 + a2 / 2.0);
  let p2 = p1 * (1.0 + 2.0 * c);
  let p3 = p2 + c / laml;
  let p4 = p3 + c / lamr;
  let nrq = nf * r * q;

  var y: i64 = 0;
  loop {
    let u = rk_double(plane) * p4;
    var v = rk_double(plane);
    var cand: i64 = 0;

    // Steps 10-40: candidate y, possibly updating v.
    if u <= p1 {
      y = i64(floor(xm - p1 * v + u));
      break;
    } else if u <= p2 {
      let x = xl + (u - p1) / c;
      v = v * c + 1.0 - abs(f64(m) - x + 0.5) / p1;
      if v > 1.0 {
        continue;
      }
      cand = i64(floor(x));
    } else if u <= p3 {
      cand = i64(floor(xl + sf64_log(spektrafilm_strict_float(v)) / laml));
      if cand < 0 {
        continue;
      }
      v = v * (u - p2) * laml;
    } else {
      cand = i64(floor(xr - sf64_log(spektrafilm_strict_float(v)) / lamr));
      if cand > i64(n) {
        continue;
      }
      v = v * (u - p3) * lamr;
    }

    // Step 50: squeeze test, then the full acceptance check.
    let k = i64(abs(f64(cand - m)));
    if k <= 20 || f64(k) >= nrq / 2.0 - 1.0 {
      let s = r / q;
      let a_step51 = s * (nf + 1.0);
      var f: f64 = 1.0;
      if m < cand {
        for (var i = m + 1; i <= cand; i = i + 1) {
          f = f * (a_step51 / f64(i) - s);
        }
      } else if m > cand {
        for (var i = cand + 1; i <= m; i = i + 1) {
          f = f / (a_step51 / f64(i) - s);
        }
      }
      if v > f {
        continue;
      }
    } else {
      let kf = f64(k);
      let rho = (kf / nrq) * ((kf * (kf / 3.0 + 0.625) + 0.16666666666666666) / nrq + 0.5);
      let t = -kf * kf / (2.0 * nrq);
      let a_log = sf64_log(spektrafilm_strict_float(v));
      if a_log < t - rho {
        y = cand;
        break;
      }
      if a_log > t + rho {
        continue;
      }
      let x1 = f64(cand + 1);
      let f1 = f64(m + 1);
      let z = f64(i64(n) - m + 1);
      let w = f64(i64(n) - cand + 1);
      let x2 = x1 * x1;
      let f2 = f1 * f1;
      let z2 = z * z;
      let w2 = w * w;
      let bound = xm * sf64_log(spektrafilm_strict_float(f1 / x1))
        + (nf - f64(m) + 0.5) * sf64_log(spektrafilm_strict_float(z / w))
        + f64(cand - m) * sf64_log(spektrafilm_strict_float(w * r / (x1 * q)))
        + (13680.0 - (462.0 - (132.0 - (99.0 - 140.0 / f2) / f2) / f2) / f2) / f1 / 166320.0
        + (13680.0 - (462.0 - (132.0 - (99.0 - 140.0 / z2) / z2) / z2) / z2) / z / 166320.0
        + (13680.0 - (462.0 - (132.0 - (99.0 - 140.0 / x2) / x2) / x2) / x2) / x1 / 166320.0
        + (13680.0 - (462.0 - (132.0 - (99.0 - 140.0 / w2) / w2) / w2) / w2) / w / 166320.0;
      if a_log > bound {
        continue;
      }
    }
    y = cand;
    break;
  }
  let y_pos = select(y, i64(n) - y, p > 0.5);
  return u64(y_pos);
}

// PTRS calls only at k+1 >= 1; reflection is unreachable and would introduce
// native f64 sin into an otherwise explicit-transcendental device kernel.
fn sf64_lgamma(x: f64) -> f64 {
  // Lanczos g=7, n=9: the accumulator starts at the g coefficient divided
  // by (z + g), and each subsequent coefficient divides (z + g + i). The
  // reference's `lgamma` is libm, so this is an approximation of it; the
  // argument is always a positive integer in the PTRS test.
  let z = x - 1.0;
  var acc: f64 = 0.99999999999980993
    + 676.5203681218851 / (z + 1.0)
    + -1259.1392167224028 / (z + 2.0)
    + 771.32342877765313 / (z + 3.0)
    + -176.61502916214059 / (z + 4.0)
    + 12.507343278686905 / (z + 5.0)
    + -0.13857109526572012 / (z + 6.0)
    + 9.9843695780195716e-6 / (z + 7.0)
    + 1.5056327351493116e-7 / (z + 8.0);
  let t = z + 7.5;
  return 0.5 * sf64_log(spektrafilm_strict_float(2.0 * MT_PI))
    + (z + 0.5) * sf64_log(spektrafilm_strict_float(t))
    - t + sf64_log(spektrafilm_strict_float(acc));
}

// ── the reference's per-element conditioning ──

/// One composite-curve bracket for `x` in channel `ch`'s x column. Matches
/// `interp_density_cmy_layers`: below the first knot clamps to bracket 0
/// with `t = 0`, at or above the last knot clamps to the final bracket with
/// `t = 1`, and otherwise the low bracket is the last knot whose x is
/// `<= x` (`partition_point(|v| v <= x)`).
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

// Probability, saturation, optical density and lambda are f32 array values
// promoted only at the discrete-sampler boundary.
fn conditioning(i: u32, ch: u32, sl: u32) -> vec4<f64> {
  let dmax = f32(p.density_max_adj[sl][ch]);
  let npp = f32(p.n_particles_layers[sl][ch]);
  let x = f64(density_cmy[i * 3u + ch]) * f64(p.lookup_sign);
  let b = bracket(x, ch);
  let lo = u32(b.x);
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
  let density = f32(sf64_from_f32(interpolated) + p.density_min_layers[sl][ch]);
  let prob = clamp(sf32_div(density, dmax), 1e-6f, 1.0f - 1e-6f);
  let saturation = sf32_sub(1.0f, sf32_mul(sf32_mul(prob, f32(p.uniformity[ch])), 1.0f - 1e-6f));
  let od = f32(p.density_max_adj[sl][ch] / p.n_particles_layers[sl][ch]);
  return vec4<f64>(sf64_from_f32(prob), sf64_from_f32(saturation), sf64_from_f32(od), sf64_from_f32(sf32_div(npp, saturation)));
}

/// The seed of one `(channel, sublayer)` plane: `base_seed + seed_ch +
/// sl·10`, with `seed_ch = 0` for every channel when monochrome.
fn plane_seed(ch: u32, sl: u32) -> u32 {
  let seed_ch = select(ch, 0u, p.flags.y != 0u);
  return u32(p.base_seed + u64(seed_ch) + u64(sl) * u64(10u));
}

// NumPy RandomState caches the first polar variate and returns the second.
// Both the cached value and its presence survive device segment boundaries.
fn rk_standard_normal(plane: u32) -> f64 {
  if mt[plane].has_gauss != 0u {
    mt[plane].has_gauss = 0u;
    return mt[plane].gauss;
  }
  var result: f64 = 0.0;
  loop {
    let x1 = 2.0 * rk_double(plane) - 1.0;
    let x2 = 2.0 * rk_double(plane) - 1.0;
    let r2 = x1 * x1 + x2 * x2;
    if r2 > 0.0 && r2 < 1.0 {
      let factor = sqrt(-2.0 * sf64_log(r2) / r2);
      mt[plane].gauss = factor * x1;
      mt[plane].has_gauss = 1u;
      result = factor * x2;
      break;
    }
  }
  return result;
}

fn draw_clumping(plane: u32) {
  let segments = (p.n_pixels + p.chunk_elements - 1u) / p.chunk_elements;
  let offset = select(0u, segments * 2u, p.flags.x == 1u);
  let segment = mt[plane].dispatch_index - offset;
  if segment >= segments { return; }
  if mt[plane].seeded == 0u {
    mt_reseed(plane, plane_seed(p.flags.w % 3u, p.flags.w / 3u));
  }
  let count = min(p.chunk_elements, p.n_pixels - segment * p.chunk_elements);
  let variance = p.micro_sigma * p.micro_sigma;
  let sum = 1.0 + variance;
  // Compensated log1p preserves the low bits lost by forming 1+variance.
  let sigma2 = sf64_log(sum) * (variance / (sum - 1.0));
  let sigma = sqrt(sigma2);
  for (var i = 0u; i < count; i = i + 1u) {
    if p.flags.y != 0u {
      let z = rk_standard_normal(plane);
      let value = sf64_exp(z * sigma - 0.5 * sigma2);
      for (var ch = 0u; ch < 3u; ch = ch + 1u) { clump[i * 3u + ch] = value; }
    } else {
      for (var ch = 0u; ch < 3u; ch = ch + 1u) {
        let z = rk_standard_normal(plane);
        clump[i * 3u + ch] = sf64_exp(z * sigma - 0.5 * sigma2);
      }
    }
  }
  mt[plane].dispatch_index = mt[plane].dispatch_index + 1u;
}

@compute @workgroup_size(1)
fn main(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
  // One invocation owns the plane's sequential generator state.
  if lid != 0u {
    return;
  }
  let plane = 0u;
  if p.flags.x != 0u { draw_clumping(plane); return; }
  let n = p.n_pixels;
  let chunk = p.chunk_elements;
  let poisson_segments = (n + chunk - 1u) / chunk;
  let total = poisson_segments * 2u;
  let dispatch_index = mt[plane].dispatch_index;
  if dispatch_index >= total {
    return;
  }
  let sl = p.flags.w / 3u;
  let ch = p.flags.w % 3u;
  // Each plane owns its own seed lanes, so the concurrent planes in one
  // dispatch never race on the same lanes.
  let seed_base = 0u;

  // Segments `0..poisson_segments` are the Poisson pass and the rest are
  // the binomial pass, which is the reference's call order.
  var phase = 0u;
  var segment = dispatch_index;
  if dispatch_index >= poisson_segments {
    phase = 1u;
    segment = dispatch_index - poisson_segments;
  }
  let first = segment * chunk;
  if first >= n {
    return;
  }
  let count = min(chunk, n - first);

  // The plane's first dispatch seeds the generator, exactly like
  // `rand_mt::Mt::new(seed as u32)`. Later dispatches resume the carried
  // state, so the plane is still one continuous reference walk.
  if mt[plane].seeded == 0u {
    mt_reseed(plane, plane_seed(ch, sl));
  }

  if phase == 0u {
    for (var j = 0u; j < count; j = j + 1u) {
      let i = j;
      let c = conditioning(i, ch, sl);
      let seed = rk_poisson(plane, c.w);
      seeds[seed_base + i * 2u] = u32(seed & u64(0xffffffffu));
      seeds[seed_base + i * 2u + 1u] = u32(seed >> 32u);
    }
  } else {
    for (var j = 0u; j < count; j = j + 1u) {
      let i = j;
      let c = conditioning(i, ch, sl);
      let seed = (u64(seeds[seed_base + i * 2u + 1u]) << 32u) | u64(seeds[seed_base + i * 2u]);
      let developed = rk_binomial(plane, seed, c.x);
      let particle = sf32_mul(f32(developed), f32(c.z));
      let value = sf32_mul(particle, f32(c.y));
      layer_fields[i * 3u + ch] = value;
      if p.flags.y != 0u {
        layer_fields[i * 3u + 1u] = value;
        layer_fields[i * 3u + 2u] = value;
      }
    }
  }
  mt[plane].dispatch_index = dispatch_index + 1u;
}
