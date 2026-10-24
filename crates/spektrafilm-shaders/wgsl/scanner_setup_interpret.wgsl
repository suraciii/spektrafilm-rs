// Interpretation half of scanner auto-setup: interpolate the resolved order
// statistics exactly as `quantile` does, then derive the bounded density,
// channel balance and contrast controls.
//
// Order-statistic pairs: luma p05 (0,1), p50 (2,3), p95 (4,5); R p50 (6,7),
// G p50 (8,9), B p50 (10,11).

@group(0) @binding(1) var<storage, read> setup_count: array<u32>;
@group(0) @binding(2) var<storage, read> setup_keys: array<u64>;
@group(0) @binding(3) var<storage, read_write> setup_values: array<f64>;

// `f64::clamp` passes NaN through; a clamp built from min/max would silently
// replace it, so the non-finite rejection the reference performs must see it.
fn setup_clamp(value: f64, low: f64, high: f64) -> f64 {
  if value != value { return value; }
  return clamp(value, low, high);
}

fn quantile_at(n: u32, fraction: f64, lower_target: u32, upper_target: u32) -> f64 {
  let position = fraction * f64(n - 1u);
  let lower = floor(position);
  let lo = setup_key_value(setup_keys[lower_target]);
  let hi = setup_key_value(setup_keys[upper_target]);
  return lo * (1.0 - position + lower) + hi * (position - lower);
}

@compute @workgroup_size(1)
fn main() {
  let n = setup_count[0];
  if (n == 0u) {
    for (var i = 0u; i < 5u; i = i + 1u) { setup_values[i] = f64(0.0); }
    return;
  }
  let median_r = quantile_at(n, 0.5, 6u, 7u);
  let median_g = quantile_at(n, 0.5, 8u, 9u);
  let median_b = quantile_at(n, 0.5, 10u, 11u);
  let neutral = (median_r + median_g + median_b) / 3.0;
  var balance_r = setup_clamp(neutral - median_r, -p.max_balance_ev, p.max_balance_ev);
  var balance_g = setup_clamp(neutral - median_g, -p.max_balance_ev, p.max_balance_ev);
  var balance_b = -(balance_r + balance_g);
  if (abs(balance_b) > p.max_balance_ev) {
    let scale = p.max_balance_ev / abs(balance_b);
    balance_r = balance_r * scale;
    balance_g = balance_g * scale;
    balance_b = balance_b * scale;
  }
  let luma_50 = quantile_at(n, 0.5, 2u, 3u);
  let luma_05 = quantile_at(n, 0.05, 0u, 1u);
  let luma_95 = quantile_at(n, 0.95, 4u, 5u);
  let density = setup_clamp(p.target_log2 - luma_50, -p.max_density_ev, p.max_density_ev);
  let span = luma_95 - luma_05;
  var contrast = 4.0 / span;
  if (span <= p.flat_span) { contrast = 1.0; }
  contrast = setup_clamp(contrast, p.contrast_min, p.contrast_max);
  setup_values[0] = density;
  setup_values[1] = balance_r;
  setup_values[2] = balance_g;
  setup_values[3] = balance_b;
  setup_values[4] = contrast;
}
