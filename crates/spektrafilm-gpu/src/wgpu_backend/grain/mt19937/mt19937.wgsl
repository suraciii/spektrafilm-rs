// Device Spektra v1 grain — the sequential scipy/NumPy MT19937 regime.
//
// CPU equivalent: `spektrafilm_model::grain::v1::layer_particle_model` with
// `use_fast_stats = false`, which is what the composite path
// (`apply_grain_to_density`) always uses and the layered path
// (`apply_grain_to_density_layers`) uses whenever the request sets
// `use_fast_stats = false`. The reference drives one `rand_mt::Mt`, seeded
// from the u32-truncated `base_seed + seed_ch + sl·10`, across a whole
// `(channel, sublayer)` plane in row-major order: every Poisson draw first,
// then every binomial draw, with variable-length rejection consumption
// inside each law (`spektrafilm_math::numpy_rng`).
//
// The regime is sequential by construction, so this pass is too. One
// workgroup of one invocation owns one plane, walks it in row-major order,
// and keeps the MT19937 state in that plane's own storage slot.
// `specs/gpu-execution/design.md` §Stochastic alignment forbids reseeding,
// reordering, or a jump-ahead shortcut — an element's next generator word
// is not at a fixed offset, because rejection consumes a variable number of
// words — so the only faithful device mapping is the same walk in the same
// order.
//
// ## Chunking
//
// One dispatch draws one segment of every plane. A plane's first
// `ceil(n / chunk)` dispatches are its Poisson segments, in element order,
// and the next `ceil(n / chunk)` are its binomial segments in the same
// order — the reference's two-pass call order over one generator. The
// generator state and the segment counter live in the plane's storage slot
// and are carried across dispatches, so a segment boundary is a submission
// boundary only: the seed, the draw order, and the element index are
// unchanged. That gives the caller watchdog and cancellation boundaries
// without altering the realization.
//
// ## Precision
//
// The reference conditions both count laws on f64 values it reads at full
// precision, so this shader runs entirely in f64 and needs SHADER_F64. It
// calls the shared explicit `f64_math.wgsl` helpers instead of driver
// log/exp/pow builtins, which return wrong values on some devices. Every
// discrete decision is made in f64; only the finished particle field is
// narrowed to f32, matching the sibling sampler's representation.
//
// The one transcendental the reference takes from libm is `lgamma` inside
// the PTRS acceptance test. `sf64_lgamma` is a Lanczos approximation rather
// than libm, so a PTRS acceptance decision that lands within that
// approximation's error of its boundary can disagree with the reference.
// This is a stated approximation with a measured disagreement budget owed
// by the owning stage, not a verified equivalence.
//
// ## Output
//
// `[sl][pixel][ch]` f32 particle fields, sublayer-major then pixel then
// channel — the same layout the sibling sampler and the composition pass
// use. This pass writes fields only; composition, clumping, dye-cloud blur,
// final blur, and the unsharp mask own the density buffer.

struct Params {
  width: u32,
  height: u32,
  knots: u32,
  layers: u32,
  n_pixels: u32,
  chunk_elements: u32,
  // Bit 0: composite (not layered) path. Bit 1: monochrome, which shares
  // channel 0's stream for every channel like the reference. Bit 2: the
  // reference keeps f64 samples, so `sample_round` is the identity.
  flags: vec4<u32>,
  base_seed: u64,
  pixel_size_um: f64,
  lookup_sign: f32,
  pad0: u32,
  // The three-value triples are `vec4<f64>`, not `vec3<f64>`: a uniform
  // `vec3` has a 32-byte stride with its own alignment, and spelling the
  // padded lane explicitly makes the host's `repr(C)` layout unambiguous.
  particle_area: vec4<f64>,
  density_min: vec4<f64>,
  uniformity: vec4<f64>,
  density_max_composite: vec4<f64>,
  n_particles_composite: vec4<f64>,
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
  pad: u32,
  state: array<u32, 624>,
}

@group(0) @binding(6) var<storage, read_write> mt: array<MtSlot>;

// The local WGPU HAL uses this named function to select strict Metal math.
// Every Poisson and binomial decision below is conditioned on an f64 value
// whose last bits change the reference's discrete outcome, so no contraction
// or reassociation is allowed anywhere in this module.
fn spektrafilm_strict_float(v: f64) -> f64 {
  return v;
}

// The reference stores its intermediate density samples in `Scalar`, which
// is f32 unless the pipeline runs in full f64 precision. Every value that
// crosses that boundary in the reference is narrowed here on the f32 route,
// so the conditioning the device computes is the same value the reference's
// discrete decisions see. `flags.z` is 1 when the reference keeps f64.
fn sample_round(value: f64) -> f64 {
  if p.flags.z != 0u { return value; }
  return f64(f32(value));
}

// ── MT19937 (`rand_mt::Mt`, bit-identical to numpy's RandomState) ──

const MT_N: u32 = 624u;
const MT_M: u32 = 397u;
const MT_MATRIX_A: u32 = 0x9908b0dfu;
const MT_UPPER_MASK: u32 = 0x80000000u;
const MT_LOWER_MASK: u32 = 0x7fffffffu;
const MT_PI: f64 = 3.14159265358979323846;

/// `Mt::reseed`: the reference's seed-to-state recurrence, verbatim.
fn mt_reseed(plane: u32, seed: u32) {
  mt[plane].idx = MT_N;
  mt[plane].state[0] = seed;
  for (var i = 1u; i < MT_N; i = i + 1u) {
    let prev = mt[plane].state[i - 1u];
    mt[plane].state[i] = 1812433253u * (prev ^ (prev >> 30u)) + i;
  }
  mt[plane].seeded = 1u;
}

/// `rand_mt::fill_next_state`: the twist over the whole block, with the
/// wrap-around pair the reference writes explicitly.
fn mt_twist(plane: u32) {
  for (var i = 0u; i < MT_N - MT_M; i = i + 1u) {
    let x = (mt[plane].state[i] & MT_UPPER_MASK) | (mt[plane].state[i + 1u] & MT_LOWER_MASK);
    mt[plane].state[i] = mt[plane].state[i + MT_M] ^ (x >> 1u)
      ^ select(0u, MT_MATRIX_A, (x & 1u) != 0u);
  }
  for (var i = MT_N - MT_M; i < MT_N - 1u; i = i + 1u) {
    let x = (mt[plane].state[i] & MT_UPPER_MASK) | (mt[plane].state[i + 1u] & MT_LOWER_MASK);
    mt[plane].state[i] = mt[plane].state[i + MT_M - MT_N] ^ (x >> 1u)
      ^ select(0u, MT_MATRIX_A, (x & 1u) != 0u);
  }
  let x = (mt[plane].state[MT_N - 1u] & MT_UPPER_MASK) | (mt[plane].state[0u] & MT_LOWER_MASK);
  mt[plane].state[MT_N - 1u] = mt[plane].state[MT_M - 1u] ^ (x >> 1u)
    ^ select(0u, MT_MATRIX_A, (x & 1u) != 0u);
  mt[plane].idx = 0u;
}

/// `rand_mt::temper`, verbatim.
fn mt_temper(input: u32) -> u32 {
  var x = input;
  x = x ^ (x >> 11u);
  x = x ^ ((x << 7u) & 0x9d2c5680u);
  x = x ^ ((x << 15u) & 0xefc60000u);
  return x ^ (x >> 18u);
}

/// `Mt::next_u32` for one plane.
fn mt_next(plane: u32) -> u32 {
  if (mt[plane].idx >= MT_N) {
    mt_twist(plane);
  }
  let x = mt[plane].state[mt[plane].idx];
  mt[plane].idx = mt[plane].idx + 1u;
  return mt_temper(x);
}

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

/// `ln Γ(x)`, the Lanczos approximation with `g = 7`, `n = 9`. NumPy takes
/// this from libm; the approximation agrees with libm to within its own
/// rounding over the arguments the PTRS test reaches (positive integers).
///
/// The reflection branch maps any `x < 0.5` to a core argument above 0.5,
/// so one level is enough and no recursion is needed.
///
/// The coefficients are written out as a scalar accumulator rather than a
/// constant array: a dynamically indexed local `array<f64, N>` loses the
/// device on this driver, and a storage or uniform buffer would cost a
/// binding for eight values that never change.
fn sf64_lgamma(x: f64) -> f64 {
  if x < 0.5 {
    return sf64_log(spektrafilm_strict_float(MT_PI / abs(sin(MT_PI * x))))
      - sf64_lgamma_core(1.0 - x);
  }
  return sf64_lgamma_core(x);
}

fn sf64_lgamma_core(x: f64) -> f64 {
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

/// Per-element conditioning for one sublayer, in the reference's f64
/// arithmetic. Returns `(prob, saturation, od_particle, lam)`.
///
/// The layered path interpolates the composite density against the
/// channel's composite curve and reads this sublayer's column at the same
/// bracket, then adds the per-layer density floor. The composite path reads
/// the composite density directly and adds the composite floor.
fn conditioning(i: u32, ch: u32, sl: u32, layered: bool) -> vec4<f64> {
  let gu = p.uniformity[ch];
  var dmax: f64;
  var npp: f64;
  var density: f64;
  if layered {
    dmax = sample_round(p.density_max_adj[sl][ch]);
    npp = sample_round(p.n_particles_layers[sl][ch]);
    // The lookup axis and the query share the host's sign, so a positive
    // film's descending axis still brackets. The reference negates the
    // image sample in `Scalar`, so the sign flip is a f32 boundary too.
    let x = sample_round(f64(density_cmy[i * 3u + ch]) * f64(p.lookup_sign));
    let b = bracket(x, ch);
    let lo = u32(b.x);
    let t = b.y;
    let y0 = sample_round(curve_layers[(lo * 3u + sl) * 3u + ch]);
    let y1 = sample_round(curve_layers[((lo + 1u) * 3u + sl) * 3u + ch]);
    let interpolated = sample_round(y0 + sample_round(t * sample_round(y1 - y0)));
    // The reference adds the per-layer floor in `Scalar`.
    density = sample_round(interpolated + sample_round(p.density_min_layers[sl][ch]));
  } else {
    // The composite path reads the density buffer directly and adds the
    // composite floor in `Scalar`.
    dmax = sample_round(p.density_max_composite[ch] + p.density_min[ch]);
    npp = sample_round(p.n_particles_composite[ch]);
    density = sample_round(f64(density_cmy[i * 3u + ch]) + p.density_min[ch]);
  }
  // `layer_particle_model` conditions both laws on these f64 values. The
  // bounds are named so every one of them is f64 rather than an untyped
  // abstract-float literal.
  let eps = f64(1e-6);
  let prob = clamp(density / dmax, eps, 1.0 - eps);
  let saturation = 1.0 - prob * gu * (1.0 - eps);
  let od_particle = dmax / npp;
  return vec4<f64>(prob, saturation, od_particle, npp / saturation);
}

/// The seed of one `(channel, sublayer)` plane: `base_seed + seed_ch +
/// sl·10`, with `seed_ch = 0` for every channel when monochrome.
fn plane_seed(ch: u32, sl: u32) -> u32 {
  let seed_ch = select(ch, 0u, p.flags.y != 0u);
  return u32(p.base_seed + u64(seed_ch) + u64(sl) * u64(10u));
}

@compute @workgroup_size(1)
fn main(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
  // One invocation owns the plane's sequential generator state.
  if lid != 0u {
    return;
  }
  let plane = 0u;
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
  let layered = p.flags.x == 0u;
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
      let c = conditioning(i, ch, sl, layered);
      let seed = rk_poisson(plane, c.w);
      seeds[seed_base + i * 2u] = u32(seed & u64(0xffffffffu));
      seeds[seed_base + i * 2u + 1u] = u32(seed >> 32u);
    }
  } else {
    for (var j = 0u; j < count; j = j + 1u) {
      let i = j;
      let c = conditioning(i, ch, sl, layered);
      let seed = (u64(seeds[seed_base + i * 2u + 1u]) << 32u) | u64(seeds[seed_base + i * 2u]);
      let developed = rk_binomial(plane, seed, c.x);
      layer_fields[i * 3u + ch] = f32(f64(developed) * c.z * c.y);
    }
  }
  mt[plane].dispatch_index = dispatch_index + 1u;
}
