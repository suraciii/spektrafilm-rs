// Valid-sample census for scanner auto-setup: one atomic per complete finite
// decoded RGB sample, the exact population the reference keeps.

@group(0) @binding(2) var<storage, read_write> setup_count: array<atomic<u32>>;

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
  atomicAdd(&setup_count[0], 1u);
}
