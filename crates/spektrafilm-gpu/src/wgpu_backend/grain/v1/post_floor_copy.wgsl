struct Params {
  dimensions: vec4<u32>,
  density_min: vec4<f32>,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> input_density: array<f32>;
@group(0) @binding(2) var<storage, read_write> output_density: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let pixel = gid.x + gid.y * 65535u * 256u;
  if pixel >= p.dimensions.x { return; }
  for (var channel = 0u; channel < 3u; channel = channel + 1u) {
    output_density[pixel * 3u + channel] = input_density[pixel * 3u + channel] - p.density_min[channel];
  }
}
