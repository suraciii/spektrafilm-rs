// Block sums of one target histogram (256 bins per block), for the exact
// prefix scan that locates each quantile rank.

@group(0) @binding(1) var<storage, read> setup_histogram: array<u32>;
@group(0) @binding(2) var<storage, read_write> setup_partial: array<u32>;

var<workgroup> block_sum: array<u32, 256>;

@compute @workgroup_size(256)
fn main(@builtin(local_invocation_index) lid: u32, @builtin(workgroup_id) wid: vec3<u32>) {
  let group = wid.x;
  if (group >= SETUP_TARGETS * (SETUP_BINS / 256u)) { return; }
  block_sum[lid] = setup_histogram[group * 256u + lid];
  workgroupBarrier();
  var step = 128u;
  loop {
    if (step == 0u) { break; }
    if (lid < step) { block_sum[lid] = block_sum[lid] + block_sum[lid + step]; }
    workgroupBarrier();
    step = step / 2u;
  }
  if (lid == 0u) { setup_partial[group] = block_sum[0]; }
}
