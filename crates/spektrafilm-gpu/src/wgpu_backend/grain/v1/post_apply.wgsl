struct Params {
  dimensions: vec4<u32>,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> sharpened: array<f32>;
@group(0) @binding(2) var<storage, read> sums: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let pixel = gid.x + gid.y * 65535u * 256u;
  if pixel >= p.dimensions.x { return; }
  for (var channel = 0u; channel < 3u; channel = channel + 1u) {
    var value = sharpened[pixel * 3u + channel];
    let input_sum = sums[channel];
    let sharpened_sum = sums[channel + 3u];
    if sharpened_sum > 0.0 {
      value = value * (input_sum / sharpened_sum);
    }
    output[pixel * 3u + channel] = value;
  }
}
