// Layered fast-statistics particle and clumping fields. Spatial filtering,
// layer composition, mass-conserving USM, and floor subtraction are separate
// resident passes. The sequential NumPy sampler lives in grain/mt19937.
//
// Binary64 conditioning preserves reference count-law decisions; f32 casts
// occur only at the Scalar boundaries selected by p.flags.x. The host pads
// binary64 vectors and rows to four lanes. p.flags.y retains monochrome seed
// identity; p.lookup_sign applies to both the curve axis and lookup query.

struct Params {
  width: u32,
  height: u32,
  knots: u32,
  pad0: u32,
  base_seed: u64,
  pixel_size_um: f64,
  particle_area: vec3<f64>,
  particle_scale_layers: vec3<f64>,
  density_min: vec3<f64>,
  uniformity: vec3<f64>,
  // WGSL requires an array element stride to be a multiple of 16 in the
  // uniform address space, so each 9-value table is three `vec4` rows with a
  // padding lane the host leaves zero.
  density_min_layers: array<vec4<f64>, 3>,
  density_max_adj: array<vec4<f64>, 3>,
  n_particles: array<vec4<f64>, 3>,
  micro_sigma: f64,
  micro_blur_px: f64,
  grain_blur: f32,
  usm_sigma: f32,
  usm_amount: f32,
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
@group(0) @binding(5) var<storage, read_write> clump_field: array<f32>;

// The local WGPU HAL uses this named function to select strict Metal math.
// Every Poisson and binomial decision below is conditioned on an f64 value
// whose last bits change the reference's discrete outcome, so no contraction
// or reassociation is allowed anywhere in this module.
fn spektrafilm_strict_float(v: f64) -> f64 {
  return v;
}

fn sample_round(value: f64) -> f64 {
  if p.flags.x != 0u { return value; }
  return f64(f32(value));
}

// ── the `fast_stats` RNG (`spektrafilm_math::stats::FastStatsRng`) ──

fn u64h(hi: u32, lo: u32) -> u64 {
  return (u64(hi) << 32u) | u64(lo);
}

// The reference constants. WGSL `const` is a const-expression context that
// cannot call a function, and an abstract-int literal cannot hold a value
// above i64, so each function builds them once from the helper above.
fn golden() -> u64 { return u64h(0x9E3779B9u, 0x7F4A7C15u); }
fn mix1() -> u64 { return u64h(0xBF58476Du, 0x1CE4E5B9u); }
fn mix2() -> u64 { return u64h(0x94D049BBu, 0x133111EBu); }
const TAG_POISSON: u64 = u64(0u);
const TAG_BINOMIAL: u64 = u64(1u);
const TAG_MICRO: u64 = u64(2u);

/// Independent stream for element `index` of an array draw, tagged so the
/// Poisson pass and the binomial pass over one element do not replay each
/// other's uniforms.
fn stream_state(base_seed: u64, tag: u64, index: u64) -> u64 {
  var z = (base_seed * golden()) ^ (tag * mix1()) ^ (index * mix2());
  z = (z ^ (z >> 30u)) * mix1();
  z = (z ^ (z >> 27u)) * mix2();
  return z ^ (z >> 31u);
}

fn next_u64(state: ptr<function, u64>) -> u64 {
  *state = *state + golden();
  var z = *state;
  z = (z ^ (z >> 30u)) * mix1();
  z = (z ^ (z >> 27u)) * mix2();
  return z ^ (z >> 31u);
}

/// Uniform double in [0, 1) with 53-bit resolution (numpy `rk_double`).
fn rand(state: ptr<function, u64>) -> f64 {
  let w = next_u64(state);
  return f64(w >> 11u) * (1.0 / 9007199254740992.0);
}

/// Standard normal via Marsaglia polar rejection. Each element owns exactly
/// one stream and performs exactly one normal draw on it, so the host's cached
/// spare variate is never read a second time and needs no device state.
fn randn(state: ptr<function, u64>) -> f64 {
  // naga rejects a `return` from inside `loop`, so the rejection loop exits
  // through the condition like the reference's `while True` with a break.
  var result: f64 = 0.0;
  var done = false;
  loop {
    let u = 2.0 * rand(state) - 1.0;
    let v = 2.0 * rand(state) - 1.0;
    let s = u * u + v * v;
    if s > 0.0 && s < 1.0 {
      let factor = sqrt(spektrafilm_strict_float(-2.0 * sf64_log(s) / s));
      result = u * factor;
      done = true;
    }
    if done {
      break;
    }
  }
  return result;
}

// ── `fast_poisson`: λ ≤ 0 → 0; λ < 30 → Knuth multiplication of uniforms;
// otherwise the rounded normal approximation clamped at 0. ──
fn fast_poisson(state: ptr<function, u64>, lam: f64) -> i64 {
  if lam <= 0.0 {
    return i64(0);
  }
  if lam < 30.0 {
    let l = sf64_exp(spektrafilm_strict_float(-lam));
    var prob: f64 = 1.0;
    var k = i64(0);
    loop {
      if prob <= l {
        break;
      }
      k = k + i64(1);
      prob = prob * rand(state);
    }
    return k - i64(1);
  }
  let sample = lam + sqrt(lam) * randn(state);
  let rounded = i64(sf64_round(spektrafilm_strict_float(sample)));
  return select(rounded, i64(0), rounded < i64(0));
}

// ── `fast_binomial`: p ≤ 0 → 0; p ≥ 1 → n; n < 25 → direct Bernoulli counting;
// variance > 10 → rounded normal approximation clamped to [0, n]; otherwise the
// inverse-CDF walk. ──
fn fast_binomial(state: ptr<function, u64>, n: i64, p: f64) -> i64 {
  if p <= 0.0 {
    return i64(0);
  }
  if p >= 1.0 {
    return n;
  }
  if n < i64(25) {
    var count = i64(0);
    var i = i64(0);
    loop {
      if i >= n {
        break;
      }
      if rand(state) < p {
        count = count + i64(1);
      }
      i = i + i64(1);
    }
    return count;
  }
  let nf = f64(n);
  let mean = nf * p;
  let variance = nf * p * (1.0 - p);
  if variance > 10.0 {
    let approx = mean + sqrt(variance) * randn(state);
    let rounded = i64(sf64_round(spektrafilm_strict_float(approx)));
    return clamp(rounded, i64(0), n);
  }
  let u = rand(state);
  var cdf: f64 = 0.0;
  var prob: f64 = sf64_powi(spektrafilm_strict_float(1.0 - p), u32(n));
  var k = i64(0);
  loop {
    if cdf >= u || k > n {
      break;
    }
    cdf = cdf + prob;
    if k < n {
      prob = prob * (((nf - f64(k)) / (f64(k) + 1.0)) * (p / (1.0 - p)));
    }
    k = k + i64(1);
  }
  return k - i64(1);
}

// ── `add_micro_structure`'s clumping: `fast_lognormal_from_mean_std` with
// linear-space mean 1 and standard deviation `micro_sigma`, per channel from
// its own tagged stream (`base_seed ^ 42` folded with the channel). ──
fn clumping(state: ptr<function, u64>, m: f64, s: f64) -> f64 {
  // WGSL has no if-expression; the reference's `if/else` value is built here.
  var mu_sigma = vec2<f64>(0.0, 0.0);
  if m > 0.0 {
    let sigma2 = sf64_log(1.0 + (s * s) / (m * m));
    mu_sigma = vec2<f64>(sf64_log(m) - sigma2 / 2.0, sqrt(sigma2));
  }
  if mu_sigma.y < 1e-6 {
    return sf64_exp(mu_sigma.x);
  }
  return sf64_exp(mu_sigma.x + mu_sigma.y * randn(state));
}

// One composite-curve bracket for `x` in channel `ch`'s x column. Matches
// `interp_density_cmy_layers`: below the first knot clamps to bracket 0 with
// `t = 0`, at or above the last knot clamps to the final bracket with `t = 1`,
// and otherwise the low bracket is the last knot whose x is `<= x`
// (`partition_point(|v| v <= x)`). The x column is non-decreasing, so the
// search is a binary search.
fn bracket(x: f64, ch: u32) -> vec2<f64> {
  let k = p.knots;
  let first = sample_round(curve_x[ch]);
  let last = sample_round(curve_x[(k - 1u) * 3u + ch]);
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
    if sample_round(curve_x[mid * 3u + ch]) <= x {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  let xa = sample_round(curve_x[lo * 3u + ch]);
  let xb = sample_round(curve_x[hi * 3u + ch]);
  let dx = sample_round(xb - xa);
  var t: f64 = 0.0;
  if dx != 0.0 {
    let inverse = sample_round(1.0 / dx);
    t = sample_round(sample_round(x - xa) * inverse);
  }
  return vec2<f64>(f64(lo), t);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let n = p.tail_padding.y;
  let i = gid.x + gid.y * 65535u * 256u;
  if i >= n { return; }
  let global_i = p.tail_padding.x + i;
  let base_seed = p.base_seed;
  let micro_base = base_seed ^ u64(42u);

  for (var ch = 0u; ch < 3u; ch = ch + 1u) {
    let gu = p.uniformity[ch];
    let x = f64(density_cmy[i * 3u + ch]) * f64(p.lookup_sign);
    if p.flags.z == 0u {
    for (var sl = 0u; sl < 3u; sl = sl + 1u) {
      // The tables are three `vec4` rows; lane `ch` holds this channel and the
      // padding lane is unused, so the row is `sl` and the lane is `ch`.
      let dmax = p.density_max_adj[sl][ch];
      let npp = p.n_particles[sl][ch];
      let od_particle = dmax / npp;

      // Sublayer density: interpolate the composite density against this
      // channel's curve column, then read this sublayer's curve column at the
      // same bracket.
      let b = bracket(x, ch);
      let lo = u32(b.x);
      let t = b.y;
      let y0 = sample_round(curve_layers[(lo * 3u + sl) * 3u + ch]);
      let y1 = sample_round(curve_layers[((lo + 1u) * 3u + sl) * 3u + ch]);
      let interpolated = sample_round(y0 + sample_round(t * sample_round(y1 - y0)));
      // The reference's `density_sl` is a `Scalar` (f32) sum of the f32
      // interpolated plane value and the f32 per-sublayer floor, so the sum
      // itself is an f32 rounding step even when the products below stay f64.
      let dmin_layer_scalar = f32(p.density_min_layers[sl][ch]);
      var density_sl: f64 = f64(f32(interpolated) + dmin_layer_scalar);
      if p.flags.x != 0u {
        density_sl = interpolated + p.density_min_layers[sl][ch];
      }

      let prob = clamp(density_sl / dmax, f64(1e-6), 1.0 - f64(1e-6));
      let saturation = 1.0 - prob * gu * (1.0 - 1e-6);
      let lam = npp / saturation;

      let seed_ch = select(ch, 0u, p.flags.y != 0u);
      let seed = base_seed + u64(seed_ch) + u64(sl) * u64(10u);
      var poisson_state = stream_state(seed, TAG_POISSON, u64(global_i));
      let seeds = fast_poisson(&poisson_state, lam);
      var binomial_state = stream_state(seed, TAG_BINOMIAL, u64(global_i));
      let developed = fast_binomial(&binomial_state, seeds, prob);
      layer_fields[(sl * n + i) * 3u + ch] = f32(f64(developed) * od_particle * saturation);
    }
    }

    var value: f64 = 1.0;
    if p.flags.w != 0u && p.micro_sigma > 0.05 {
      var clump_state = stream_state(micro_base, u64(ch), u64(global_i));
      value = clumping(&clump_state, 1.0, p.micro_sigma);
    }
    clump_field[i * 3u + ch] = f32(value);
  }
}
