// One ABI for preview sampling and reduction.
struct MeterParams {
  src_width: u32, src_height: u32, cols: u32, rows: u32,
  radius_y: u32, radius_x: u32, cctf_kind: u32, pattern: u32,
  sigma_y: f32, sigma_x: f32, factor_y: f32, factor_x: f32,
  y_row: vec4<f32>,
  mid_grey: f32, decode: u32, pad: u32, pad2: u32, tail: vec4<u32>,
}
fn mirror_index(i: i32, n: u32) -> u32 {
  if n <= 1u { return 0u; }
  let period = 2i * i32(n) - 2i;
  var j = i % period;
  if j < 0i { j = j + period; }
  return u32(select(j, period - j, j >= i32(n)));
}
fn tap_weight(dx: i32, sigma: f32) -> f32 {
  if sigma <= 0.0 { return select(0.0, 1.0, dx == 0i); }
  return exp(-0.5 / (sigma * sigma) * f32(dx) * f32(dx));
}
fn source_position(k: u32, dim: u32, size: u32) -> u32 {
  // floor((k + 0.5) * size / dim), without overflow or f32 rounding.
  let quotient = size / dim;
  let remainder = size % dim;
  return k * quotient + quotient / 2u +
    ((quotient % 2u) * dim + (2u * k + 1u) * remainder) / (2u * dim);
}
fn norm_coord(i: u32, dim: u32, maximum: u32) -> f32 {
  return (f32(i) / f32(dim) - 0.5) * (f32(dim) / f32(maximum));
}
