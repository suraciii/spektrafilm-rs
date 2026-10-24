// One digit of rank selection: find the smallest bin whose inclusive prefix
// exceeds the target rank, then carry the residual rank and the chosen digit
// into the next pass. After the last pass the accumulated prefix is the exact
// order statistic, written to `setup_keys`.

@group(0) @binding(1) var<storage, read> setup_prefix: array<u32>;
@group(0) @binding(2) var<storage, read_write> setup_targets: array<vec4<u32>>;
@group(0) @binding(3) var<storage, read_write> setup_keys: array<u64>;

@compute @workgroup_size(12)
fn main(@builtin(local_invocation_index) tid: u32) {
  if (tid >= p.n_targets) { return; }
  let rank_meta = setup_targets[tid];
  let rank = rank_meta.y;
  let base = tid * SETUP_BINS;
  var lo = 0u;
  var hi = SETUP_BINS - 1u;
  loop {
    if (lo >= hi) { break; }
    let mid = lo + (hi - lo) / 2u;
    if (setup_prefix[base + mid] > rank) { hi = mid; } else { lo = mid + 1u; }
  }
  var before = 0u;
  if (lo > 0u) { before = setup_prefix[base + lo - 1u]; }
  let residual = rank - before;
  var prefix = (u64(rank_meta.z) << 32u) | u64(rank_meta.w);
  prefix = prefix | (u64(lo) << p.shift);
  if (p.shift == 0u) {
    setup_keys[tid] = prefix;
  }
  setup_targets[tid] = vec4<u32>(rank_meta.x, residual, u32(prefix >> 32u), u32(prefix & u64(0xFFFFFFFF)));
}
