struct MeterChunk {
  row: u32, first: u32, count: u32, tap_first: u32,
  tap_count: u32, page_first: u32, page_len: u32, axis: u32,
}
@group(0) @binding(0) var<uniform> p: MeterParams;
@group(0) @binding(1) var<uniform> c: MeterChunk;
@group(0) @binding(2) var<storage, read> gathered: array<f64>;
@group(0) @binding(3) var<storage, read_write> accumulated: array<f64>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x + gid.y * 65535u * 256u;
  if i >= c.count { return; }
  let radius = i32(select(p.radius_y, p.radius_x, c.axis != 0u));
  let sigma = select(p.sigma_y, p.sigma_x, c.axis != 0u);
  var norm: f64 = 0.0;
  for (var d = -radius; d <= radius; d = d + 1i) { norm = norm + tap_weight(d, sigma); }
  for (var ch = 0u; ch < 3u; ch = ch + 1u) {
    var sum: f64 = 0.0;
    if c.tap_first != 0u { sum = accumulated[i * 3u + ch]; }
    for (var tap = 0u; tap < c.tap_count; tap = tap + 1u) {
      let d = i32(c.tap_first + tap) - radius;
      sum = sum + (tap_weight(d, sigma) / norm) * gathered[(tap * c.count + i) * 3u + ch];
    }
    accumulated[i * 3u + ch] = sum;
  }
}
