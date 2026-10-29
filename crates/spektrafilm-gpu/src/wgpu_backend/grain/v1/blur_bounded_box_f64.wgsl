// Reflected box pass over binary64 storage, with f32 box arithmetic.
struct Params {
  count: u32, lines: u32, stride: u32, channel: u32,
  radius: u32, mode: u32, initialize: u32, axis_major: u32,
  a: f32, b1: f32, b2: f32, b3: f32,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f64>;
@group(0) @binding(2) var<storage, read_write> destination: array<f64>;
@group(0) @binding(3) var<storage, read> kernel: array<f32>;
@group(0) @binding(4) var<storage, read_write> state: array<f32>;
fn index(coordinate: u32, line: u32, length: u32) -> u32 {
  return select(line * length + coordinate, coordinate * p.lines + line, p.axis_major != 0u) * p.stride + p.channel;
}
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let line = gid.x;
  if line >= p.lines { return; }
  for (var j = 0u; j < p.count; j++) {
    var sum = 0.0;
    for (var k = 0u; k < 2u * p.radius + 1u; k++) {
      sum = sum + f32(source[index(j + k, line, p.count + 2u * p.radius)]);
    }
    destination[index(j, line, p.count)] = f64(sum * kernel[0]);
  }
}
