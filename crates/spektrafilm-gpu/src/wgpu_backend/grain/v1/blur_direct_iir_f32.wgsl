// Young-van Vliet recursive Gaussian, one separable direction.
//
// One invocation owns one whole row (horizontal) or column (vertical), so the
// forward and backward recursions keep the reference's serial state order. The
// state starts from the first sample and the backward pass re-reads the stored
// f32 forward endpoint, as upstream 28bf883 fast_gaussian_filter.py does.
// Coefficients and recurrence history use f32 on devices without shader f64.
// Storage bindings cover one image-sized RGB region from an aligned byte
// offset; `source_base`/`destination_base` carry the residual sample offset.

struct Params {
  width: u32,
  height: u32,
  channel: u32,
  vertical: u32,
  a: f32,
  b1: f32,
  b2: f32,
  b3: f32,
  grid_width: u32,
  source_base: u32,
  destination_base: u32,
  _pad: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> destination: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let line = gid.x + gid.y * p.grid_width * 64u;
  let line_count = select(p.height, p.width, p.vertical != 0u);
  if line >= line_count { return; }
  let length = select(p.width, p.height, p.vertical != 0u);
  let x0 = select(0u, line, p.vertical != 0u);
  let y0 = select(line, 0u, p.vertical != 0u);
  let initial = f32(source[p.source_base + (y0 * p.width + x0) * 3u + p.channel]);
  var w1 = initial;
  var w2 = initial;
  var w3 = initial;
  for (var j = 0u; j < length; j = j + 1u) {
    let x = select(j, line, p.vertical != 0u);
    let y = select(line, j, p.vertical != 0u);
    let offset = (y * p.width + x) * 3u + p.channel;
    let value = p.a * f32(source[p.source_base + offset])
      + p.b1 * w1 + p.b2 * w2 + p.b3 * w3;
    destination[p.destination_base + offset] = f32(value);
    w3 = w2;
    w2 = w1;
    w1 = value;
  }
  let last_x = select(length - 1u, line, p.vertical != 0u);
  let last_y = select(line, length - 1u, p.vertical != 0u);
  let last = f32(destination[p.destination_base + (last_y * p.width + last_x) * 3u + p.channel]);
  w1 = last;
  w2 = last;
  w3 = last;
  for (var j = length; j > 0u; j = j - 1u) {
    let coordinate = j - 1u;
    let x = select(coordinate, line, p.vertical != 0u);
    let y = select(line, coordinate, p.vertical != 0u);
    let index = p.destination_base + (y * p.width + x) * 3u + p.channel;
    let value = p.a * f32(destination[index]) + p.b1 * w1 + p.b2 * w2 + p.b3 * w3;
    destination[index] = f32(value);
    w3 = w2;
    w2 = w1;
    w1 = value;
  }
}
