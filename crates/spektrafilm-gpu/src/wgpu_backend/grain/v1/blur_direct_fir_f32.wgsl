// Reference FIR Gaussian, one separable direction.
//
// Storage bindings cover one image-sized RGB region starting at an aligned
// byte offset; `source_base`/`destination_base` carry the residual sample
// offset into those bindings, so arbitrary layer offsets stay valid.
// Boundaries use the reference's half-sample symmetric reflection, folded
// repeatedly so a kernel wider than the image still matches the CPU.

struct Params {
  width: u32,
  height: u32,
  radius: u32,
  channel: u32,
  vertical: u32,
  grid_width: u32,
  source_base: u32,
  destination_base: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read> kernel: array<f32>;
@group(0) @binding(3) var<storage, read_write> destination: array<f32>;

fn reflect_index(index: i32, length: i32) -> u32 {
  let period = 2 * length;
  let folded = ((index % period) + period) % period;
  return u32(select(period - 1 - folded, folded, folded < length));
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let pixel = gid.x + gid.y * p.grid_width * 256u;
  if pixel >= p.width * p.height { return; }
  let x = pixel % p.width;
  let y = pixel / p.width;
  let axis_length = select(i32(p.width), i32(p.height), p.vertical != 0u);
  let coordinate = select(i32(x), i32(y), p.vertical != 0u);
  var sum: f32 = 0.0;
  let kernel_size = 2u * p.radius + 1u;
  for (var k = 0u; k < kernel_size; k = k + 1u) {
    let delta = i32(k) - i32(p.radius);
    let reflected = reflect_index(coordinate + delta, axis_length);
    let sx = select(reflected, x, p.vertical != 0u);
    let sy = select(y, reflected, p.vertical != 0u);
    let index = p.source_base + (sy * p.width + sx) * 3u + p.channel;
    sum = sum + f32(source[index]) * kernel[k];
  }
  destination[p.destination_base + pixel * 3u + p.channel] = f32(sum);
}
