// One digit of the exact 64-bit selection used by scanner auto-setup.
//
// Each of the twelve targets (luma p05/p50/p95 lower and upper neighbours,
// then R/G/B p50 lower and upper) owns one 65536-bin histogram over the next
// 16 key bits. A target's earlier digits are its prefix; a key contributes
// only to targets whose prefix it still matches. No value is approximated:
// the four passes over 64 key bits resolve the exact order statistic.

@group(0) @binding(2) var<storage, read_write> setup_histogram: array<atomic<u32>>;
@group(0) @binding(3) var<storage, read_write> setup_targets: array<vec4<u32>>;

fn contribute(t: u32, c0: u64, c1: u64, c2: u64, lum: u64) {
  let rank_meta = setup_targets[t];
  var key = lum;
  if (rank_meta.x == 0u) {
    key = c0;
  } else if (rank_meta.x == 1u) {
    key = c1;
  } else if (rank_meta.x == 2u) {
    key = c2;
  }
  let prefix = (u64(rank_meta.z) << 32u) | u64(rank_meta.w);
  if (!setup_prefix_matches(key, prefix, p.shift)) { return; }
  let bin = setup_digit(key, p.shift);
  atomicAdd(&setup_histogram[t * SETUP_BINS + bin], 1u);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
  let index = gid.x + gid.y * grid.x * 256u;
  if (index >= p.chunk_end - p.chunk_start) { return; }
  let pixel = p.chunk_start + index;
  if (pixel >= p.width * p.height) { return; }
  let base = pixel * 3u;
  let raw_r = source_value(base);
  let raw_g = source_value(base + 1u);
  let raw_b = source_value(base + 2u);
  if (!setup_finite(raw_r) || !setup_finite(raw_g) || !setup_finite(raw_b)) { return; }
  let r = setup_input(raw_r);
  let g = setup_input(raw_g);
  let b = setup_input(raw_b);
  if (!setup_finite(r) || !setup_finite(g) || !setup_finite(b)) { return; }

  let positive_r = max(r, p.min_sample);
  let positive_g = max(g, p.min_sample);
  let positive_b = max(b, p.min_sample);
  let c0 = setup_order_key(setup_log2(positive_r));
  let c1 = setup_order_key(setup_log2(positive_g));
  let c2 = setup_order_key(setup_log2(positive_b));
  var lumin = p.luma_r * positive_r + p.luma_g * positive_g + p.luma_b * positive_b;
  if (lumin != lumin) { lumin = p.min_sample; }
  lumin = max(lumin, p.min_sample);
  let lum = setup_order_key(setup_log2(lumin));

  contribute(0u, c0, c1, c2, lum);
  contribute(1u, c0, c1, c2, lum);
  contribute(2u, c0, c1, c2, lum);
  contribute(3u, c0, c1, c2, lum);
  contribute(4u, c0, c1, c2, lum);
  contribute(5u, c0, c1, c2, lum);
  contribute(6u, c0, c1, c2, lum);
  contribute(7u, c0, c1, c2, lum);
  contribute(8u, c0, c1, c2, lum);
  contribute(9u, c0, c1, c2, lum);
  contribute(10u, c0, c1, c2, lum);
  contribute(11u, c0, c1, c2, lum);
}
