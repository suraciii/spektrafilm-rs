// Inclusive 65536-bin prefix per target: `prefix[t * 65536 + b]` is the number
// of keys whose selected digit is at most `b`. A workgroup scans one 256-bin
// block and adds its exclusive base, so the result is the exact count.

@group(0) @binding(1) var<storage, read> setup_histogram: array<u32>;
@group(0) @binding(2) var<storage, read> setup_offsets: array<u32>;
@group(0) @binding(3) var<storage, read_write> setup_prefix: array<u32>;

var<workgroup> block_scan: array<u32, 256>;

@compute @workgroup_size(256)
fn main(@builtin(local_invocation_index) lid: u32, @builtin(workgroup_id) wid: vec3<u32>) {
  let group = wid.x;
  if (group >= SETUP_TARGETS * (SETUP_BINS / 256u)) { return; }
  let index = group * 256u + lid;
  block_scan[lid] = setup_histogram[index];
  workgroupBarrier();
  var step = 1u;
  loop {
    if (step >= 256u) { break; }
    var add = 0u;
    if (lid >= step) { add = block_scan[lid - step]; }
    workgroupBarrier();
    block_scan[lid] = block_scan[lid] + add;
    workgroupBarrier();
    step = step * 2u;
  }
  setup_prefix[index] = block_scan[lid] + setup_offsets[group];
}
