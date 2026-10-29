// Binary64 bounded Gaussian. Transport supplies true reflected FIR halos;
// recursive state carries across pages and initializes only at true endpoints.
struct Params {
  count: u32,
  lines: u32,
  stride: u32,
  channel: u32,
  radius: u32,
  mode: u32,
  initialize: u32,
  axis_major: u32,
  a: f64,
  b1: f64,
  b2: f64,
  b3: f64,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f64>;
@group(0) @binding(2) var<storage, read_write> destination: array<f64>;
@group(0) @binding(3) var<storage, read> kernel: array<f64>;
@group(0) @binding(4) var<storage, read_write> state: array<f64>;
fn index(coordinate: u32, line: u32, length: u32) -> u32 {
  return select(line * length + coordinate, coordinate * p.lines + line, p.axis_major != 0u) * p.stride + p.channel;
}
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let line = gid.x;
  if line >= p.lines { return; }
  if p.mode == 0u {
    for (var j = 0u; j < p.count; j++) {
      var sum: f64 = 0.0;
      for (var k = 0u; k < 2u * p.radius + 1u; k++) {
        sum = sum + source[index(j + k, line, p.count + 2u * p.radius)] * kernel[k];
      }
      destination[index(j, line, p.count)] = sum;
    }
    return;
  }
  let initial_coordinate = select(0u, p.count - 1u, p.mode == 2u);
  let initial = source[index(initial_coordinate, line, p.count)];
  var w1 = state[line * 3u];
  var w2 = state[line * 3u + 1u];
  var w3 = state[line * 3u + 2u];
  if p.initialize != 0u { w1 = initial; w2 = initial; w3 = initial; }
  for (var j = 0u; j < p.count; j++) {
    let coordinate = select(j, p.count - 1u - j, p.mode == 2u);
    let offset = index(coordinate, line, p.count);
    let value = p.a * source[offset] + p.b1 * w1 + p.b2 * w2 + p.b3 * w3;
    destination[offset] = value;
    w3 = w2;
    w2 = w1;
    w1 = value;
  }
  state[line * 3u] = w1;
  state[line * 3u + 1u] = w2;
  state[line * 3u + 2u] = w3;
}
