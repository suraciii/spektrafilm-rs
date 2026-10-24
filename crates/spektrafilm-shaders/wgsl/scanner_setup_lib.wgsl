// Scanner auto-setup reduction, shared half.
//
// The reference measures one complete decoded input frame
// (`spektrafilm_core::scanner_setup::from_image`): decode the input transfer,
// keep the pixels whose decoded RGB is finite, take the binary logarithm of
// each positive-clamped channel and of the luminance, then read the exact
// interpolated quantiles of those complete populations. This device port
// keeps the same population, the same ordering (binary64 total order through
// a monotone 64-bit key), and the same interpolation; only the bounded
// controls return to the host.
//
// Only bounded, image-independent constants arrive from the host: the target
// luminance placement, the clamp bounds, the flat-span threshold, the sample
// floor, and the three luminance weights. Everything image-dependent — the
// decoded samples, the quantile positions, the interpolation, and the
// balance/density/contrast interpretation — runs here.
//
// Requires `f64_math.wgsl` (sf64_log/sf64_exp) and a source accessor file
// that declares binding 1 and `source_value(index)`. Source concatenation is
// the WGSL include mechanism.

const SETUP_BINS: u32 = 65536u;
const SETUP_TARGETS: u32 = 12u;

struct SetupParams {
  chunk_start: u32,
  chunk_end: u32,
  width: u32,
  height: u32,
  shift: u32,
  cctf_kind: u32,
  decode: u32,
  n_targets: u32,
  target_log2: f64,
  max_density_ev: f64,
  max_balance_ev: f64,
  contrast_min: f64,
  contrast_max: f64,
  flat_span: f64,
  min_sample: f64,
  pad0: f64,
  luma_r: f64,
  luma_g: f64,
  luma_b: f64,
  pad1: f64,
}

@group(0) @binding(0) var<uniform> p: SetupParams;

// The reference's `spow`: sign-preserving power at finite signed headroom.
// `sf64_pow` composes the validated explicit binary64 routines; the driver's
// native transcendental builtins are not used.
fn setup_spow(x: f64, power: f64) -> f64 {
  return sf64_pow(x, power);
}

// `spektrafilm_math::colorspace::cctf_decode` for the same samples.
fn setup_decode(v: f64, kind: u32) -> f64 {
  switch kind {
    case 0u: { return v; }
    case 1u: {
      if v > 0.155251141552511 {
        return sf64_exp((v * 17.52 - 9.72) * 0.69314718055994530942);
      }
      return (v - 0.0729055341958355) / 10.5402377416545;
    }
    case 2u: {
      if 12.92 * 0.0031308 >= v { return v / 12.92; }
      return setup_spow((v + 0.055) / 1.055, 2.4);
    }
    case 3u: {
      if v < 16.0 * (1.0 / 512.0) { return v / 16.0; }
      return setup_spow(v, 1.8);
    }
    case 4u: {
      let a: f64 = f64(1.099);
      if v < a * setup_spow(0.018, 0.45) - (a - 1.0) { return v / 4.5; }
      return setup_spow((v + (a - 1.0)) / a, 1.0 / 0.45);
    }
    case 5u: { return setup_spow(v, 563.0 / 256.0); }
    default: { return setup_spow(v, 2.6); }
  }
}

// `f64::is_finite`: infinite and NaN samples are excluded exactly like the
// reference's `all(|v| v.is_finite())` mask.
fn setup_finite(v: f64) -> bool {
  return abs(v) <= 1.7976931348623157e308;
}

fn setup_input(v: f64) -> f64 {
  if p.decode == 0u { return v; }
  return setup_decode(v, p.cctf_kind);
}

// `f64::log2` as the validated base-2 logarithm: ln(v) / ln(2).
fn setup_log2(v: f64) -> f64 {
  return sf64_log(v) * 1.4426950408889634074;
}

// Monotone map from binary64 total order to unsigned 64-bit order. The
// reference sorts with `f64::total_cmp`, so the key must preserve that order
// exactly, including negative samples and every mantissa bit.
fn setup_order_key(value: f64) -> u64 {
  let bits = bitcast<u64>(value);
  let sign_bit = u64(1u) << 63u;
  if (bits & sign_bit) != u64(0u) { return ~bits; }
  return bits | sign_bit;
}

fn setup_key_value(key: u64) -> f64 {
  let sign_bit = u64(1u) << 63u;
  if (key & sign_bit) != u64(0u) { return bitcast<f64>(key & ~sign_bit); }
  return bitcast<f64>(~key);
}

fn setup_digit(key: u64, shift: u32) -> u32 {
  if shift >= 48u { return u32((key >> 48u) & u64(0xFFFF)); }
  return u32((key >> shift) & u64(0xFFFF));
}

// Whether a key still matches the prefix selected by earlier passes. The
// prefix holds the already chosen digits at their own shifts.
fn setup_prefix_matches(key: u64, prefix: u64, shift: u32) -> bool {
  if shift >= 48u { return true; }
  let upper = shift + 16u;
  return (key >> upper) == (prefix >> upper);
}

// Quantile rank positions, exactly as `quantile(sorted, fraction)` sizes the
// population: `position = fraction * (n - 1)`, with `floor`/`ceil` neighbours.
fn setup_position(n: u32, fraction: f64) -> f64 {
  return fraction * f64(n - 1u);
}

fn setup_lower(position: f64) -> u32 {
  return u32(floor(position));
}

fn setup_upper(position: f64) -> u32 {
  return u32(ceil(position));
}
