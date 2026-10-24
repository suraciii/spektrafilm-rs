// Exclusive block offsets per target: `offsets[t * 256 + b]` is the number of
// keys in blocks before `b`. One thread per target walks its 256 blocks in
// order, preserving the exact integer prefix.

@group(0) @binding(1) var<storage, read> setup_partial: array<u32>;
@group(0) @binding(2) var<storage, read_write> setup_offsets: array<u32>;

@compute @workgroup_size(12)
fn main(@builtin(local_invocation_index) tid: u32) {
  if (tid >= SETUP_TARGETS) { return; }
  var running = 0u;
  for (var b = 0u; b < SETUP_BINS / 256u; b = b + 1u) {
    let index = tid * (SETUP_BINS / 256u) + b;
    setup_offsets[index] = running;
    running = running + setup_partial[index];
  }
}
