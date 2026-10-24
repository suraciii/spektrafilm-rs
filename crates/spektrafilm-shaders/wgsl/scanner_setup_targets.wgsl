// Exact quantile rank targets.
//
// Pairs per population: luma p05, p50, p95, then R, G and B p50. The first
// entry of a pair is `floor(position)` and the second `ceil(position)`, so the
// interpolation the reference performs needs only these two order statistics.

@group(0) @binding(1) var<storage, read> setup_count: array<u32>;
@group(0) @binding(2) var<storage, read_write> setup_targets: array<vec4<u32>>;

fn write_pair(base: u32, field: u32, fraction: f64, span: f64) {
  let position = fraction * span;
  setup_targets[base] = vec4<u32>(field, u32(floor(position)), 0u, 0u);
  setup_targets[base + 1u] = vec4<u32>(field, u32(ceil(position)), 0u, 0u);
}

@compute @workgroup_size(1)
fn main() {
  let n = setup_count[0];
  if (n == 0u) {
    for (var t = 0u; t < SETUP_TARGETS; t = t + 1u) {
      setup_targets[t] = vec4<u32>(0u, 0u, 0u, 0u);
    }
    return;
  }
  let span = f64(n - 1u);
  write_pair(0u, 3u, 0.05, span);
  write_pair(2u, 3u, 0.5, span);
  write_pair(4u, 3u, 0.95, span);
  write_pair(6u, 0u, 0.5, span);
  write_pair(8u, 1u, 0.5, span);
  write_pair(10u, 2u, 0.5, span);
}
