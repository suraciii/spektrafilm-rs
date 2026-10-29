struct MeterChunk {
  row: u32, first: u32, count: u32, tap_first: u32,
  tap_count: u32, page_first: u32, page_len: u32, pad: u32,
}
@group(0) @binding(0) var<uniform> p: MeterParams;
@group(0) @binding(1) var<uniform> c: MeterChunk;
@group(0) @binding(2) var<storage, read> accumulated: array<f32>;
@group(0) @binding(3) var<storage, read_write> luminance: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x + gid.y * 65535u * 256u;
  if i >= c.count { return; }
  var rgb = vec3<f32>(accumulated[i * 3u], accumulated[i * 3u + 1u], accumulated[i * 3u + 2u]);
  if p.decode != 0u {
    rgb = vec3<f32>(decode_one(rgb.x, p.cctf_kind), decode_one(rgb.y, p.cctf_kind), decode_one(rgb.z, p.cctf_kind));
  }
  luminance[c.row * p.cols + c.first + i] = p.y_row.x * rgb.x + p.y_row.y * rgb.y + p.y_row.z * rgb.z;
}
