// Spektra tagged fast-statistics particle fields. Curve interpolation uses
// f64 tables, then floor addition, conditioning and count products use f32
// exactly like upstream's grain working arrays. NumPy clumping is generated
// separately from the final plane's MT19937 seed/state.

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

fn round_even(value: f64) -> f64 {
  let low = floor(value);
  let delta = value - low;
  if delta < 0.5 { return low; }
  if delta > 0.5 { return low + 1.0; }
  return select(low + 1.0, low, low == floor(low * 0.5) * 2.0);
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
  let rounded = i64(round_even(spektrafilm_strict_float(sample)));
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
    let rounded = i64(round_even(spektrafilm_strict_float(approx)));
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
      prob = (prob * ((nf - f64(k)) / (f64(k) + 1.0))) * (p / (1.0 - p));
    }
    k = k + i64(1);
  }
  return k - i64(1);
}

// One composite-curve bracket for `x` in channel `ch`'s x column. Matches
// `interp_density_cmy_layers`: below the first knot clamps to bracket 0 with
// `t = 0`, at or above the last knot clamps to the final bracket with `t = 1`,
// and otherwise the low bracket is the last knot whose x is `<= x`
// (`partition_point(|v| v <= x)`). The x column is non-decreasing, so the
// search is a binary search.
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
      let seed = p.base_seed + u64(ch) + u64(sl) * u64(10u);
      var poisson_state = stream_state(seed, TAG_POISSON, u64(global_i));
      let seeds = fast_poisson(&poisson_state, sf64_from_f32(lam));
      var binomial_state = stream_state(seed, TAG_BINOMIAL, u64(global_i));
      let developed = fast_binomial(&binomial_state, seeds, sf64_from_f32(prob));
      let particle = sf32_mul(f32(developed), od);
      let value = sf32_mul(particle, saturation);
      layer_fields[(sl * n + i) * 3u + ch] = value;
      if p.flags.y != 0u {
        layer_fields[(sl * n + i) * 3u + 1u] = value;
        layer_fields[(sl * n + i) * 3u + 2u] = value;
      }
    }
  }
}
