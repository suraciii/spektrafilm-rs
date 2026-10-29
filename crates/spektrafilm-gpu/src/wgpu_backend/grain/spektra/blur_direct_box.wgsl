struct Params {
  width: u32, height: u32, radius: u32, channel: u32,
  vertical: u32, grid_width: u32, source_base: u32, destination_base: u32,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> destination: array<f32>;

fn reflect_index(index: i32, length: i32) -> u32 {
  let period = 2 * length;
  let folded = ((index % period) + period) % period;
  return u32(select(period - 1 - folded, folded, folded < length));
}
fn sample(line: u32, coordinate: u32) -> f32 {
  let x = select(coordinate, line, p.vertical != 0u);
  let y = select(line, coordinate, p.vertical != 0u);
  return source[p.source_base + (y * p.width + x) * 3u + p.channel];
}
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let line = gid.x + gid.y * p.grid_width * 64u;
  let count = select(p.height, p.width, p.vertical != 0u);
  if line >= count { return; }
  let length = select(p.width, p.height, p.vertical != 0u);
  let radius = i32(p.radius);
  var sum = 0.0f;
  for (var offset = -radius; offset <= radius; offset = offset + 1) {
    sum = sum + sample(line, reflect_index(offset, i32(length)));
  }
  for (var j = 0u; j < length; j = j + 1u) {
    let x = select(j, line, p.vertical != 0u);
    let y = select(line, j, p.vertical != 0u);
    destination[p.destination_base + (y * p.width + x) * 3u + p.channel] = sum / f32(2u * p.radius + 1u);
    if j + 1u < length {
      sum = sum - sample(line, reflect_index(i32(j) - radius, i32(length)));
      sum = sum + sample(line, reflect_index(i32(j) + radius + 1, i32(length)));
    }
  }
}
