struct Params {
  width: u32,
  height: u32,
  layers: u32,
  micro_active: u32,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> fields: array<f32>;
@group(0) @binding(2) var<storage, read> clump: array<f64>;
@group(0) @binding(3) var<storage, read_write> density: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let n = p.width * p.height;
  let i = gid.x + gid.y * 65535u * 256u;
  if i >= n { return; }
  for (var ch = 0u; ch < 3u; ch = ch + 1u) {
    var sum = fields[i * 3u + ch];
    for (var sl = 1u; sl < p.layers; sl = sl + 1u) {
      sum = sum + fields[(sl * n + i) * 3u + ch];
    }
    // Upstream multiplies its f32 density array in-place by a f64 clump.
    if p.micro_active != 0u { sum = f32(f64(sum) * clump[i * 3u + ch]); }
    density[i * 3u + ch] = sum;
  }
}
