// Convert one preview pixel to film raw through the render's prepared TC
// table and adaptation matrix, then write the mean raw channel value.
struct MeterChunk {
  row: u32, first: u32, count: u32, tap_first: u32,
  tap_count: u32, page_first: u32, page_len: u32, pad: u32,
}
struct RawParams {
  lut_size: u32,
  _pad0: u32,
  _pad1: u32,
  _pad2: u32,
  // Source RGB → CAT-adapted XYZ, stored as three column vectors.
  cols: array<vec4<f32>, 3>,
}
@group(0) @binding(0) var<uniform> p: MeterParams;
@group(0) @binding(1) var<uniform> c: MeterChunk;
@group(0) @binding(2) var<storage, read> accumulated: array<f32>;
@group(0) @binding(3) var<storage, read_write> raw_field: array<f32>;
@group(0) @binding(4) var<storage, read> tc_lut: array<f32>;
@group(0) @binding(5) var<uniform> r: RawParams;

fn mat_vec(v: vec3<f32>) -> vec3<f32> {
  return r.cols[0].xyz * v.x + r.cols[1].xyz * v.y + r.cols[2].xyz * v.z;
}

fn mitchell(t: f32) -> f32 {
  let at = abs(t);
  let b = 1.0 / 3.0;
  let c = 1.0 / 3.0;
  if at < 1.0 {
    let t2 = at * at;
    let t3 = t2 * at;
    return ((12.0 - 9.0 * b - 6.0 * c) * t3
      + (-18.0 + 12.0 * b + 6.0 * c) * t2
      + (6.0 - 2.0 * b)) / 6.0;
  } else if at < 2.0 {
    let t2 = at * at;
    let t3 = t2 * at;
    return ((-b - 6.0 * c) * t3
      + (6.0 * b + 30.0 * c) * t2
      + (-12.0 * b - 48.0 * c) * at
      + (8.0 * b + 24.0 * c)) / 6.0;
  }
  return 0.0;
}

fn reflect_index(i: i32, n: i32) -> u32 {
  var idx = i;
  if idx < 0 { idx = -idx; }
  let period = 2 * (n - 1);
  if period > 0 {
    idx = idx % period;
    if idx >= n { idx = period - idx; }
  } else {
    idx = 0;
  }
  return u32(clamp(idx, 0, n - 1));
}

fn sample_lut_bicubic(lut_x: f32, lut_y: f32) -> vec3<f32> {
  let size_i = i32(r.lut_size);
  let max_xy = f32(size_i - 1);
  let xf = clamp(lut_x, 0.0, max_xy);
  let yf = clamp(lut_y, 0.0, max_xy);
  var xi = i32(floor(xf));
  var yi = i32(floor(yf));
  if xi >= size_i - 1 { xi = size_i - 2; }
  if yi >= size_i - 1 { yi = size_i - 2; }
  let fx = xf - f32(xi);
  let fy = yf - f32(yi);

  let wx0 = mitchell(fx + 1.0);
  let wx1 = mitchell(fx);
  let wx2 = mitchell(fx - 1.0);
  let wx3 = mitchell(fx - 2.0);
  let wy0 = mitchell(fy + 1.0);
  let wy1 = mitchell(fy);
  let wy2 = mitchell(fy - 1.0);
  let wy3 = mitchell(fy - 2.0);

  var sum = vec3<f32>(0.0, 0.0, 0.0);
  var weight_sum = 0.0;
  for (var dy = 0i; dy < 4i; dy++) {
    let sy = reflect_index(yi + dy - 1i, size_i);
    var wy: f32;
    if dy == 0 { wy = wy0; } else if dy == 1 { wy = wy1; } else if dy == 2 { wy = wy2; } else { wy = wy3; }
    for (var dx = 0i; dx < 4i; dx++) {
      let sx = reflect_index(xi + dx - 1i, size_i);
      var wx: f32;
      if dx == 0 { wx = wx0; } else if dx == 1 { wx = wx1; } else if dx == 2 { wx = wx2; } else { wx = wx3; }
      let w = wx * wy;
      weight_sum += w;
      let lut_base = (sy * u32(size_i) + sx) * 3u;
      sum.x += w * tc_lut[lut_base];
      sum.y += w * tc_lut[lut_base + 1u];
      sum.z += w * tc_lut[lut_base + 2u];
    }
  }
  if weight_sum != 0.0 { sum /= weight_sum; }
  return sum;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x + gid.y * 65535u * 256u;
  if i >= c.count { return; }
  var rgb = vec3<f32>(accumulated[i * 3u], accumulated[i * 3u + 1u], accumulated[i * 3u + 2u]);
  if p.decode != 0u {
    rgb = vec3<f32>(decode_one(rgb.x, p.cctf_kind), decode_one(rgb.y, p.cctf_kind), decode_one(rgb.z, p.cctf_kind));
  }
  let xyz = mat_vec(rgb);
  let b = xyz.x + xyz.y + xyz.z;
  if b <= 1e-10 {
    raw_field[c.row * p.cols + c.first + i] = 0.0;
    return;
  }
  let xc = xyz.x / b;
  let yc = xyz.y / b;
  let omx = max(1.0 - xc, 1e-10);
  let tx = clamp((1.0 - xc) * (1.0 - xc), 0.0, 1.0);
  let ty = clamp(yc / omx, 0.0, 1.0);
  let scale = f32(r.lut_size - 1u);
  let lookup = sample_lut_bicubic(ty * scale, tx * scale);
  raw_field[c.row * p.cols + c.first + i] = (lookup.x + lookup.y + lookup.z) / 3.0 * b;
}
