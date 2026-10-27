struct Params {
  dimensions: vec4<u32>,
  density_min_0: f64,
  density_min_1: f64,
  density_min_2: f64,
  _pad: f64,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> input_density: array<f32>;
@group(0) @binding(2) var<storage, read_write> output_density: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let pixel = gid.x + gid.y * 65535u * 256u;
  if pixel >= p.dimensions.x { return; }
  let minimum = vec3<f64>(p.density_min_0, p.density_min_1, p.density_min_2);
  for (var channel = 0u; channel < 3u; channel = channel + 1u) {
    let value = input_density[pixel * 3u + channel];
    if p.dimensions.y != 0u {
      output_density[pixel * 3u + channel] = max(value, 0.0);
    } else {
      output_density[pixel * 3u + channel] = f32(f64(value) - minimum[channel]);
    }
  }
}
