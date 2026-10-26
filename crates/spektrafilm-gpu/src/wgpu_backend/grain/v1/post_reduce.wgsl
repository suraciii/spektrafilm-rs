// Exact NumPy float32 pairwise leaves, carried across arbitrary storage windows.
// Host recursion splits at (n/2)&!7 until <=128; combine adds left and right.
struct Params {
  count: u32,
  first: u32,
  total: u32,
  combine: u32,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> density: array<f32>;
@group(0) @binding(2) var<storage, read> sharpened: array<f32>;
@group(0) @binding(3) var<storage, read_write> lanes: array<f32>;
@group(0) @binding(4) var<storage, read_write> sums: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let channel = gid.x;
  if channel >= 3u { return; }
  if p.combine != 0u {
    sums[channel] = density[channel] + sums[channel];
    sums[channel + 3u] = density[channel + 3u] + sums[channel + 3u];
    return;
  }
  var d: array<f32, 8>;
  var s: array<f32, 8>;
  if p.first != 0u {
    for (var k = 0u; k < 8u; k++) {
      d[k] = lanes[channel * 16u + k];
      s[k] = lanes[channel * 16u + k + 8u];
    }
  }
  var density_sum: f32 = -0.0;
  var sharpen_sum: f32 = -0.0;
  if p.total < 8u {
    if p.first != 0u {
      density_sum = sums[channel];
      sharpen_sum = sums[channel + 3u];
    }
    for (var j = 0u; j < p.count; j++) {
      density_sum = density_sum + max(density[j * 3u + channel], 0.0);
      sharpen_sum = sharpen_sum + sharpened[j * 3u + channel];
    }
  } else {
    let end = p.total - p.total % 8u;
    for (var j = 0u; j < p.count && p.first + j < end; j++) {
      let coordinate = p.first + j;
      let k = coordinate % 8u;
      let dv = max(density[j * 3u + channel], 0.0);
      let sv = sharpened[j * 3u + channel];
      if coordinate < 8u { d[k] = dv; s[k] = sv; }
      else { d[k] = d[k] + dv; s[k] = s[k] + sv; }
    }
    if p.first <= end && p.first + p.count >= end {
      density_sum = ((d[0] + d[1]) + (d[2] + d[3])) + ((d[4] + d[5]) + (d[6] + d[7]));
      sharpen_sum = ((s[0] + s[1]) + (s[2] + s[3])) + ((s[4] + s[5]) + (s[6] + s[7]));
    } else if p.first > end {
      density_sum = sums[channel];
      sharpen_sum = sums[channel + 3u];
    }
    for (var j = 0u; j < p.count; j++) {
      if p.first + j >= end {
        density_sum = density_sum + max(density[j * 3u + channel], 0.0);
        sharpen_sum = sharpen_sum + sharpened[j * 3u + channel];
      }
    }
    for (var k = 0u; k < 8u; k++) {
      lanes[channel * 16u + k] = d[k];
      lanes[channel * 16u + k + 8u] = s[k];
    }
  }
  if p.total < 8u || p.first + p.count >= p.total - p.total % 8u {
    sums[channel] = density_sum;
    sums[channel + 3u] = sharpen_sum;
  }
}
