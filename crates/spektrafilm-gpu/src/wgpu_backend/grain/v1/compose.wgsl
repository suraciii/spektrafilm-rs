struct Params {
  width: u32,
  height: u32,
  layers: u32,
  micro_active: u32,
  scale: f32,
  pad0: u32,
  pad1: u32,
  pad2: u32,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> fields: array<f32>;
@group(0) @binding(2) var<storage, read> clump: array<f32>;
@group(0) @binding(3) var<storage, read_write> density: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let n = p.width * p.height;
  let i = gid.x + gid.y * 65535u * 256u;
  if i >= n { return; }
  for (var ch = 0u; ch < 3u; ch = ch + 1u) {
    var sum: f32 = 0.0;
    for (var sl = 0u; sl < p.layers; sl = sl + 1u) {
      sum = sum + fields[(sl * n + i) * 3u + ch];
    }
    sum = sum * p.scale;
    if p.micro_active != 0u { sum = sum * clump[i * 3u + ch]; }
    density[i * 3u + ch] = sum;
  }
}
