// Reset the twelve selection histograms before one digit pass.

@group(0) @binding(1) var<storage, read_write> setup_histogram: array<u32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
  let index = gid.x + gid.y * grid.x * 256u;
  if (index >= SETUP_TARGETS * SETUP_BINS) { return; }
  setup_histogram[index] = 0u;
}
