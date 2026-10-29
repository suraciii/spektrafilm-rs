// Raw-bit rectangular gather/scatter. The image binding is one aligned physical
// view (or a bounded bulk-copy adapter); scratch contains only the target channel.
struct Params {
  count: u32,
  columns: u32,
  image_start: u32,
  image_row_step: u32,
  scratch_start: i32,
  scratch_row_step: i32,
  scratch_col_step: i32,
  scatter: u32,
  sample_words: u32,
  _pad0: u32,
  _pad1: u32,
  _pad2: u32,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> image: array<u32>;
@group(0) @binding(2) var<storage, read_write> scratch: array<u32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
  let idx = gid.x + gid.y * grid.x * 256u;
  if idx >= p.count { return; }
  let row = idx / p.columns;
  let col = idx % p.columns;
  let image_index = p.image_start + row * p.image_row_step + col * 3u * p.sample_words;
  let scratch_index = u32(p.scratch_start + i32(row) * p.scratch_row_step + i32(col) * p.scratch_col_step);
  if p.scatter != 0u {
    image[image_index] = scratch[scratch_index];
  } else {
    scratch[scratch_index] = image[image_index];
  }
}
