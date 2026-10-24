// Identity channel: the reference returns a copy for sigma <= 0.
//
// Source and destination bind one image-sized RGB region each; the base
// offsets address the requested samples inside those aligned bindings, so
// this copies only the requested channel and leaves every other channel
// untouched.

struct Params {
  width: u32,
  height: u32,
  channel: u32,
  grid_width: u32,
  source_base: u32,
  destination_base: u32,
  _pad0: u32,
  _pad1: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> destination: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let pixel = gid.x + gid.y * p.grid_width * 256u;
  if pixel >= p.width * p.height { return; }
  let offset = pixel * 3u + p.channel;
  destination[p.destination_base + offset] = source[p.source_base + offset];
}
