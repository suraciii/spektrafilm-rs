// Device realization of `apply_multiplicative_unsharp_mask`. One workgroup
// covers a segment of one channel; lane `l` sums 32 consecutive pixels of that
// segment and writes one f64 pair. The f32 cast below is the reference's
// `Scalar` store; the mass sums keep the unrounded f64 value.
struct Params {
  dimensions: vec4<u32>, // n_pixels, groups_per_channel, padding
  controls: vec4<f32>,   // USM amount, padding
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> density: array<f32>;
@group(0) @binding(2) var<storage, read> blurred: array<f32>;
@group(0) @binding(3) var<storage, read_write> sharpened: array<f32>;
@group(0) @binding(4) var<storage, read_write> partial_sums: array<f64>;

const PIXELS_PER_LANE: u32 = 32u;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let channel = gid.x % 3u;
  let lane = gid.x / 3u;
  if lane * 32u >= p.dimensions.x { return; }
  let entry = lane * 6u;
  let amount = f64(p.controls.x);
  var density_sum: f64 = 0.0;
  var sharpen_sum: f64 = 0.0;
  if p.dimensions.y != 0u {
    density_sum = partial_sums[entry + channel];
    sharpen_sum = partial_sums[entry + channel + 3u];
  }
  for (var pixel = lane * 32u; pixel < min(p.dimensions.x, (lane + 1u) * 32u); pixel = pixel + 1u) {
      let input_density = max(f64(density[pixel * 3u + channel]), 0.0lf);
      var sharpened_value: f64 = 0.0;
      if input_density > 0.0 {
        let blur_density = max(f64(blurred[pixel * 3u + channel]), 1e-6lf);
        // The reference evaluates `density^(1+amount) / blur^amount` as two
        // powers; keeping the quotient of two explicit exponentials preserves
        // that structure instead of forming the log difference.
        sharpened_value = sf64_exp((1.0 + amount) * sf64_log(input_density))
          / sf64_exp(amount * sf64_log(blur_density));
      }
      sharpened[pixel * 3u + channel] = f32(sharpened_value);
      density_sum = density_sum + input_density;
      sharpen_sum = sharpen_sum + sharpened_value;
  }
  partial_sums[entry + channel] = density_sum;
  partial_sums[entry + channel + 3u] = sharpen_sum;
}
