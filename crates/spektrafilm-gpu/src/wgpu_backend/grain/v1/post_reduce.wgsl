// One invocation sums up to 256 six-value items for one channel, so the
// 256-pixel workgroup stride is folded without workgroup memory. The host
// repeats the pass until one item remains.
struct Params {
  n_items: u32,
  output_groups: u32,
  padding: vec2<u32>,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> input_sums: array<f64>;
@group(0) @binding(2) var<storage, read_write> output_sums: array<f64>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let channel = gid.x;
  if channel >= 3u { return; }
  var density_sum: f64 = 0.0;
  var sharpen_sum: f64 = 0.0;
  if p.output_groups != 0u {
    density_sum = output_sums[channel];
    sharpen_sum = output_sums[channel + 3u];
  }
  for (var item = 0u; item < p.n_items; item = item + 1u) {
    density_sum = density_sum + input_sums[item * 6u + channel];
    sharpen_sum = sharpen_sum + input_sums[item * 6u + channel + 3u];
  }
  output_sums[channel] = density_sum;
  output_sums[channel + 3u] = sharpen_sum;
}
