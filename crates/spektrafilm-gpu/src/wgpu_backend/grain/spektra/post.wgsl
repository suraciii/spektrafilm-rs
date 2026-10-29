// Original f32-array unsharp operation. The two Python-float exponents narrow
// independently; each power narrows before the f32 quotient is evaluated.
struct Params {
  dimensions: vec4<u32>,
  controls: vec4<f32>, // amount, 1+amount, padding
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> density: array<f32>;
@group(0) @binding(2) var<storage, read> blurred: array<f32>;
@group(0) @binding(3) var<storage, read_write> sharpened: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
  let pixel = gid.x + gid.y * grid.x * 256u;
  if pixel >= p.dimensions.x { return; }
  for (var channel = 0u; channel < 3u; channel++) {
    let index = pixel * 3u + channel;
    let input_density = max(density[index], 0.0);
    var value: f32 = 0.0;
    if input_density > 0.0 {
      let blur_density = max(blurred[index], 1e-6);
      let numerator = f32(sf64_exp(f64(p.controls.y) * sf64_log(f64(max(input_density, 1e-6)))));
      let denominator = f32(sf64_exp(f64(p.controls.x) * sf64_log(f64(blur_density))));
      value = numerator / denominator;
    }
    sharpened[index] = value;
  }
}
