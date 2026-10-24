// One ABI for preview sampling and reduction; values retain reference precision.
struct MeterParams {
  src_width: u32, src_height: u32, cols: u32, rows: u32,
  radius_y: u32, radius_x: u32, cctf_kind: u32, pattern: u32,
  sigma_y: f64, sigma_x: f64, factor_y: f64, factor_x: f64,
  y_row: vec4<f64>,
  mid_grey: f64, decode: u32, pad: u32, tail: vec4<u32>,
}
fn mirror_index(i: i32, n: u32) -> u32 {
  if n <= 1u { return 0u; }
  let period = 2i * i32(n) - 2i;
  var j = i % period;
  if j < 0i { j = j + period; }
  return u32(select(j, period - j, j >= i32(n)));
}
fn tap_weight(dx: i32, sigma: f64) -> f64 {
  if sigma <= 0.0 { return select(f64(0.0), f64(1.0), dx == 0i); }
  return sf64_exp(-0.5 / (sigma * sigma) * f64(dx) * f64(dx));
}
fn source_position(k: u32, factor: f64, size: u32) -> u32 {
  return min(u32(floor((f64(k) + 0.5) * factor)), size - 1u);
}
fn norm_coord(i: u32, dim: u32, maximum: u32) -> f64 {
  return (f64(i) / f64(dim) - 0.5) * (f64(dim) / f64(maximum));
}
fn meter_spow(x: f64, power: f64) -> f64 {
  if x == 0.0 || x != x { return x; }
  return sign(x) * sf64_exp(power * sf64_log(abs(x)));
}
fn meter_decode(v: f64, kind: u32) -> f64 {
  switch kind {
    case 0u: { return v; }
    case 1u: {
      if v > 0.155251141552511 { return sf64_exp((v * 17.52 - 9.72) * 0.69314718055994530942); }
      return (v - 0.0729055341958355) / 10.5402377416545;
    }
    case 2u: {
      if v <= 12.92 * 0.0031308 { return v / 12.92; }
      return meter_spow((v + 0.055) / 1.055, 2.4);
    }
    case 3u: {
      if v < 0.03125 { return v / 16.0; }
      return meter_spow(v, 1.8);
    }
    case 4u: {
      if v < 1.099 * meter_spow(f64(0.018), f64(0.45)) - 0.099 { return v / 4.5; }
      return meter_spow((v + 0.099) / 1.099, f64(1.0) / 0.45);
    }
    case 5u: { return meter_spow(v, f64(563.0) / 256.0); }
    default: { return meter_spow(v, f64(2.6)); }
  }
}
