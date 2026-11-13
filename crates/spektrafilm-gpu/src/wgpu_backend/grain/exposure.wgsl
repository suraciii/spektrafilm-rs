// Exposure Grain: reconstructed post-scan encoded-RGB gradient noise.
// Binding order: params, source page, T1, T2, T3, perm, perm2, octave weights, output page.
struct Params {
 dims: vec4<u32>,      // width, height, page pixels, 0
 coeff: vec4<f32>,     // shadow, midtone, highlight amplitude, frequency gain
 shape: vec4<f32>,     // roughness effective, size_high (fy), colorvar, size_low (fx + amp)
 page: vec4<u32>,      // first absolute pixel
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage,read> source_rgb: array<f32>;
@group(0) @binding(2) var<storage,read> t1: array<f32>;
@group(0) @binding(3) var<storage,read> t2: array<f32>;
@group(0) @binding(4) var<storage,read> t3: array<f32>;
@group(0) @binding(5) var<storage,read> perm: array<u32>;
@group(0) @binding(6) var<storage,read> perm2: array<u32>;
@group(0) @binding(7) var<storage,read> octave_weights: array<f32>;
@group(0) @binding(8) var<storage,read_write> output_rgb: array<f32>;

const OCT: array<f32, 5> = array<f32, 5>(23.1, 8.15, 57.0, 69.2, 117.4);

fn smoothstep01(v: f32) -> f32 {
 let x = clamp(v, 0.0, 1.0);
 return (3.0 - 2.0 * x) * x * x;
}

fn noise_rgb_at(x: f32, y: f32, base: u32, base_perm: u32) -> vec3<f32> {
 var acc = vec3<f32>(0.0, 0.0, 0.0);
 let mask = 8191u;
 for (var o: u32 = 0u; o < 5u; o = o + 1u) {
  let frequency = f32(1u << o);
  let px = x * (1.0 / p.shape.w) * frequency + OCT[o];
  let py = y * (1.0 / p.shape.y) + OCT[o];
  let ix = floor(px);
  let iy = floor(py);
  let fx = px - ix;
  let fy = py - iy;
  let tx = smoothstep01(fx);
  let ty = smoothstep01(fy);
  let ux = u32(i32(ix)) & mask;
  let ux1 = (ux + 1u) & mask;
  let uy = u32(i32(iy)) & mask;
  let uy1 = (uy + 1u) & mask;
  let h0 = perm2[base_perm + ux];
  let h1 = perm2[base_perm + ux1];
  let c00 = perm[base_perm + (h0 ^ uy)];
  let c01 = perm[base_perm + (h0 ^ uy1)];
  let c10 = perm[base_perm + (h1 ^ uy)];
  let c11 = perm[base_perm + (h1 ^ uy1)];
  for (var ch: u32 = 0u; ch < 3u; ch = ch + 1u) {
   let s = ch;
   let v00 = fx * t1[base + c00 + s] + fy * t2[base + c00 + s] + t3[base + c00 + s];
   let v01 = fx * t1[base + c01 + s] + (fy - 1.0) * t2[base + c01 + s] + t3[base + c01 + s];
   let v10 = (fx - 1.0) * t1[base + c10 + s] + fy * t2[base + c10 + s] + t3[base + c10 + s];
   let v11 = (fx - 1.0) * t1[base + c11 + s] + (fy - 1.0) * t2[base + c11 + s] + t3[base + c11 + s];
   let a = v00 + (v10 - v00) * tx;
   let b = v01 + (v11 - v01) * tx;
   acc[ch] = acc[ch] + octave_weights[o] * (a + (b - a) * ty);
  }
 }
 let amp = abs(p.shape.w);
 return clamp(acc, vec3<f32>(-amp), vec3<f32>(amp));
}

fn noise_rgb(x: f32, y: f32) -> vec3<f32> {
 return noise_rgb_at(x, y, 0u, 0u);
}

fn noise_rgb_color(x: f32, y: f32) -> vec3<f32> {
 return noise_rgb_at(x, y, 516u, 8192u);
}

fn zone_weights(luma: f32) -> vec3<f32> {
 let mid = 0.424672424793243;
 let high = 0.849344849586487;
 let lo = select(1.0, smoothstep01((luma + 0.5) / 0.5), luma < 0.0);
 let middle = smoothstep01(luma / mid);
 let hi = smoothstep01((luma - mid) / (high - mid));
 let last = smoothstep01((luma - 1.0) / 0.5);
 return vec3<f32>(lo - middle, middle - hi, hi - last);
}

fn shaped(value: f32, rough: f32) -> f32 {
 if (abs(rough - 0.5) < 1e-5) { return value; }
 let t = (1.0 - 0.5) * rough / (rough - 0.5);
 let a = t * t - t;
 let b = t;
 let denom = 1.0 - abs(value) - b;
 let mapped = select(1.0, clamp(a / denom + b, 0.0, 1.0), denom != 0.0);
 return select(-mapped, mapped, value >= 0.0);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
 let local = gid.x + gid.y * grid.x * 256u;
 if (local >= p.dims.z) { return; }
 let i = local * 3u;
 let absolute = p.page.x + local;
 let x = f32(absolute % p.dims.x);
 let y = f32(absolute / p.dims.x);
 let rgb = vec3<f32>(source_rgb[i], source_rgb[i + 1u], source_rgb[i + 2u]);
 let luma = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
 let weights = zone_weights(luma);
 let displacement = dot(weights, vec3<f32>(p.coeff.x, p.coeff.y, p.coeff.z));
 let base = displacement * p.coeff.w;
 let fields = noise_rgb(x, y);
 let base_field = shaped(fields.x, p.shape.x);
 let out_luma = clamp(luma + base * base_field, 0.0, 1.0);
 let delta = out_luma - luma;
 // Luma pass: achromatic grain delta reaches every channel.
 var out = clamp(rgb + vec3<f32>(delta), vec3<f32>(0.0), vec3<f32>(1.0));
 // Color pass (binary 0x1418d8070): a second generator supplies three
 // chromatic noise fields; per-zone amplitude 0.4 (shadow/midtone) and
 // 0.8 (highlight), then frequency gain.  Raw noise, no S-curve.
 if (p.shape.z > 0.0) {
  let n = noise_rgb_color(x, y);
  let k1 = 0.4 * p.shape.z;
  let k2 = 0.8 * p.shape.z;
  let color_gain = (weights.x * p.coeff.x + weights.y * p.coeff.y) * k1
      + weights.z * p.coeff.z * k2;
  let delta_c = color_gain * p.coeff.w;
  out = clamp(out + n * delta_c, vec3<f32>(0.0), vec3<f32>(1.0));
 }
 output_rgb[i] = out.r;
 output_rgb[i + 1u] = out.g;
 output_rgb[i + 2u] = out.b;
}
