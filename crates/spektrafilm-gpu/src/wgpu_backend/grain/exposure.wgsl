// Exposure Grain: reconstructed post-scan encoded-RGB value noise.
// Binding order: params, source page, T1, T2, T3, perm, perm2, octave weights, output page.
struct Params {
 dims: vec4<u32>,      // width, height, page pixels, 0
 coeff: vec4<f32>,     // shadow, midtone, highlight amplitude, frequency gain
 shape: vec4<f32>,     // roughness effective, size_low, 0, 0
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

const OCT: array<f32, 8> = array<f32, 8>(23.1, 8.15, 57.0, 69.2, 117.4, 138.6, 198.6, 278.85);

fn smoothstep01(v: f32) -> f32 {
 let x = clamp(v, 0.0, 1.0);
 return (3.0 - 2.0 * x) * x * x;
}

fn corner(ix: i32, iy: i32) -> f32 {
 let mask = 8191u;
 let h0 = perm2[u32(ix) & mask];
 let h1 = perm2[(u32(ix) + 1u) & mask];
 let i0 = u32(iy) & mask;
 let i1 = (u32(iy) + 1u) & mask;
 let c00 = perm[h0 ^ i0];
 let c01 = perm[h0 ^ i1];
 let c10 = perm[h1 ^ i0];
 let c11 = perm[h1 ^ i1];
 // The caller needs all four values; keep this function for the single value
 // used by the table tests and inline the four-corner sum in noise().
 return t1[c00] + t3[c00] + t2[c00]
   + t1[c01] + t3[c01] + t2[c01]
   + t1[c10] + t3[c10] + t2[c10]
   + t1[c11] + t3[c11] + t2[c11];
}

fn noise(x: f32, y: f32) -> f32 {
 var acc = 0.0;
 let mask = 8191u;
 for (var o: u32 = 0u; o < 5u; o = o + 1u) {
  let px = x * (1.0 / p.shape.y) * f32(1u << o) + OCT[o];
  let py = y * (1.0 / p.shape.z) + OCT[o];
  let ix = floor(px);
  let iy = floor(py);
  let tx = smoothstep01(px - ix);
  let ty = smoothstep01(py - iy);
  let ux = u32(i32(ix)) & mask;
  let ux1 = (ux + 1u) & mask;
  let uy = u32(i32(iy)) & mask;
  let uy1 = (uy + 1u) & mask;
  let h0 = perm2[ux];
  let h1 = perm2[ux1];
  let c00 = perm[h0 ^ uy];
  let c01 = perm[h0 ^ uy1];
  let c10 = perm[h1 ^ uy];
  let c11 = perm[h1 ^ uy1];
  let v00 = t1[c00] + t3[c00] + t2[c00];
  let v01 = t1[c01] + t3[c01] + t2[c01];
  let v10 = t1[c10] + t3[c10] + t2[c10];
  let v11 = t1[c11] + t3[c11] + t2[c11];
  let a = v00 + (v10 - v00) * tx;
  let b = v01 + (v11 - v01) * tx;
  acc = acc + octave_weights[o] * (a + (b - a) * ty);
 }
 return clamp(acc, -p.shape.y, p.shape.y);
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
 let grain = shaped(noise(x, y), p.shape.x);
 let out_luma = clamp(luma + displacement * p.coeff.w * grain, 0.0, 1.0);
 let delta = out_luma - luma;
 output_rgb[i] = clamp(rgb.r + delta, 0.0, 1.0);
 output_rgb[i + 1u] = clamp(rgb.g + delta, 0.0, 1.0);
 output_rgb[i + 2u] = clamp(rgb.b + delta, 0.0, 1.0);
}
