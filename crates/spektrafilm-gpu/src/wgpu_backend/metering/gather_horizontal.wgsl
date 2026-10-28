struct MeterChunk {
  row: u32, first: u32, count: u32, tap_first: u32,
  tap_count: u32, page_first: u32, page_len: u32, pad: u32,
}
@group(0) @binding(0) var<uniform> p: MeterParams;
@group(0) @binding(1) var<uniform> c: MeterChunk;
@group(0) @binding(2) var<storage, read> source: array<f32>;
@group(0) @binding(3) var<storage, read_write> gathered: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x + gid.y * 65535u * 256u;
  if i >= c.count * c.tap_count { return; }
  let col = c.first + i % c.count;
  let tap = c.tap_first + i / c.count;
  let sx = i32(source_position(col, p.cols, p.src_width));
  let x = mirror_index(sx + i32(tap) - i32(p.radius_x), p.src_width);
  if x < c.page_first || x - c.page_first >= c.page_len { return; }
  for (var ch = 0u; ch < 3u; ch = ch + 1u) {
    gathered[i * 3u + ch] = source[(x - c.page_first) * 3u + ch];
  }
}
