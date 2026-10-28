// The preview has at most 65536 samples. Sequential binary32 accumulation
// preserves reference row-major order, including the cell and ring order.
@group(0) @binding(0) var<uniform> p: MeterParams;
@group(0) @binding(1) var<storage, read> grid: array<f32>;
@group(0) @binding(2) var<storage, read_write> result: array<f32>;
@group(0) @binding(3) var<storage, read> matrix_weights: array<f32>;
fn average() -> f32 {
  var sum: f32 = 0.0;
  for (var i = 0u; i < p.cols * p.rows; i = i + 1u) { sum = sum + grid[i]; }
  return sum / f32(p.cols * p.rows);
}
fn order_key(value: f32) -> u32 {
  let bits = bitcast<u32>(value);
  let sign_bit = u32(1u) << 31u;
  if (bits & sign_bit) != u32(0u) { return ~bits; }
  return bits | sign_bit;
}
fn sorted_value(index: u32) -> f32 {
  var lo = u32(0u);
  var hi = ~u32(0u);
  loop {
    if lo >= hi { break; }
    let probe = lo + (hi - lo) / u32(2u);
    var count = 0u;
    for (var i = 0u; i < p.cols * p.rows; i = i + 1u) {
      if order_key(grid[i]) <= probe { count = count + 1u; }
    }
    if count > index { hi = probe; } else { lo = probe + u32(1u); }
  }
  let sign_bit = u32(1u) << 31u;
  if (lo & sign_bit) != u32(0u) { return bitcast<f32>(lo & ~sign_bit); }
  return bitcast<f32>(~lo);
}
fn meter() -> f32 {
  let n = p.cols * p.rows;
  let maximum = max(p.cols, p.rows);
  if p.pattern == 0u { return average(); }
  if p.pattern == 1u {
    let high = sorted_value(n / 2u);
    if n % 2u != 0u { return high; }
    return 0.5 * (sorted_value(n / 2u - 1u) + high);
  }
  if p.pattern == 4u {
    let height = p.rows / 5u;
    let width = p.cols / 5u;
    if height == 0u || width == 0u { return f32(0.0); }
    var weight_sum: f32 = 0.0;
    for (var cell = 0u; cell < 25u; cell = cell + 1u) { weight_sum = weight_sum + matrix_weights[cell]; }
    var metered: f32 = 0.0;
    for (var cell = 0u; cell < 25u; cell = cell + 1u) {
      var sum: f32 = 0.0;
      for (var y = (cell / 5u) * height; y < (cell / 5u + 1u) * height; y = y + 1u) {
        for (var x = (cell % 5u) * width; x < (cell % 5u + 1u) * width; x = x + 1u) { sum = sum + grid[y * p.cols + x]; }
      }
      let mean = sum / f32(width * height);
      metered = metered + mean * matrix_weights[cell] / weight_sum;
    }
    return metered;
  }
  if p.pattern == 5u {
    var weighted: f32 = 0.0;
    var total: f32 = 0.0;
    for (var ring = 0u; ring < 3u; ring = ring + 1u) {
      let lower = select(select(f32(0.25), f32(0.05), ring == 1u), f32(0.0), ring == 0u);
      let upper = select(select(f32(0.50), f32(0.25), ring == 1u), f32(0.05), ring == 0u);
      let ring_weight = select(select(f32(0.20), f32(0.30), ring == 1u), f32(0.50), ring == 0u);
      var sum: f32 = 0.0;
      var count = 0u;
      for (var y = 0u; y < p.rows; y = y + 1u) {
        let ny = norm_coord(y, p.rows, maximum);
        for (var x = 0u; x < p.cols; x = x + 1u) {
          let nx = norm_coord(x, p.cols, maximum);
          let radius = sqrt(nx * nx + ny * ny);
          if radius >= lower && radius < upper { sum = sum + grid[y * p.cols + x]; count = count + 1u; }
        }
      }
      if count > 0u { weighted = weighted + ring_weight * (sum / f32(count)); total = total + ring_weight; }
    }
    if total > 0.0 { return weighted / total; }
    return average();
  }
  if p.pattern != 2u && p.pattern != 3u && p.pattern != 6u { return p.mid_grey; }
  var weighted: f32 = 0.0;
  var total: f32 = 0.0;
  for (var y = 0u; y < p.rows; y = y + 1u) {
    let ny = norm_coord(y, p.rows, maximum);
    for (var x = 0u; x < p.cols; x = x + 1u) {
      let nx = norm_coord(x, p.cols, maximum);
      let value = grid[y * p.cols + x];
      var weight: f32 = 1.0;
      if p.pattern == 2u { weight = exp(-(nx * nx + ny * ny) * (f32(1.0) / (2.0 * 0.2 * 0.2))); }
      if p.pattern == 3u && sqrt(nx * nx + ny * ny) >= 0.15 { weight = f32(0.0); }
      if p.pattern == 6u { weight = value * value; }
      weighted = weighted + value * weight;
      total = total + weight;
    }
  }
  if (p.pattern == 3u && total == 0.0) || (p.pattern == 6u && total < 1e-12) { return average(); }
  return weighted / total;
}
@compute @workgroup_size(1)
fn main() {
  let exposure = meter() / p.mid_grey;
  var ev = -log2(exposure);
  if abs(ev) > 3.4028234663852886e38 { ev = f32(0.0); }
  result[0] = ev;
}
