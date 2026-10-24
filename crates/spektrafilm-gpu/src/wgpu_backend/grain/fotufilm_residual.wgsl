// Fotufilm pitch statistics and residual readout — the device realization of
// `Bank::pitch_tables` and `Bank::residual` in
// `spektrafilm_model::grain::fotufilm::tiles`.
//
// The pitch pass measures the mean density and adjacent-level correlation of
// a record's bank over the actual readout footprint, at the 128×128 probe
// lattice the reference samples. The residual pass then samples the grain
// residual at every output pixel: it evaluates the complete periodic
// texel-area integral of the chosen bank placements, subtracts the measured
// mean, and normalizes by the retained variance.
//
// The periodic integral is the four-point prefix sum over a signed period
// decomposition (`periodic_sum`), so an arbitrary finite rectangle integrates
// exactly without a margin clip — the implementation's complete-periodic
// extension of the source's 96-micrometer sampler. The box's denominator is
// the requested area, never the retained block coverage.
//
// Placement identity is per block: `block_light` hashes the block coordinates
// into a periodic offset and one of eight orientations. The device reproduces
// the same hash and the same seam weights, so a pixel's residual depends only
// on its absolute bank coordinates, the footprint, and the seed — never on
// dispatch order, workgroup size, or chunking.
//
// The density fields are base-10 logarithms of transmittance, exactly as the
// reference's `log10` forms them. The shared `sf64_log` is a natural log — the
// software adapter's native f64 `log` is not reliable — so each log is scaled
// by `1 / ln(10)`; the residual's normalization divides by the measured
// variance.

// Both constants are f64 because every use is binary64 arithmetic — the
// periodic decomposition, the block offsets, and the seam weights all divide
// or multiply an f64 value. 256.0 and 128.0 are exact in either width.
const TILE: f64 = 256.0;
const BLOCK: f64 = 128.0;
const PROBE: u32 = 128u;
const LEVELS: u32 = 17u;

/// `1 / ln(10)`: the shared `sf64_log` is a natural log, while the reference's
/// `log10` divides by `ln(10)`. The density fields are base-10 logarithms of
/// transmittance, so every log below is scaled by this factor.
const LOG10_LN: f64 = 0.4342944819032518;

struct PitchParams {
  footprint_x: f64,
  footprint_y: f64,
  record: u32,
  pad0: u32,
}

@group(0) @binding(0) var<uniform> pitch_params: PitchParams;
// [level][257*257] centered prefix sums, one record.
@group(0) @binding(1) var<storage, read> sums: array<f32>;
// One f64 per level: the tile's mean transmittance.
@group(0) @binding(2) var<storage, read> means: array<f64>;
// [probe][level] probe densities of this record.
@group(0) @binding(3) var<storage, read_write> density: array<f64>;
// 16 adjacent-level correlations of this record.
@group(0) @binding(4) var<storage, read_write> correlation: array<f64>;
// Probe mean density is not tile mean transmittance.
@group(0) @binding(5) var<storage, read_write> probe_means: array<f64>;

/// The four-point periodic prefix sum, in the reference's exact order.
fn inside(x: f64, y: f64, sums_offset: u32) -> f64 {
  let n = TILE;
  let ix = min(u32(floor(x)), u32(n) - 1u);
  let iy = min(u32(floor(y)), u32(n) - 1u);
  let a = x - f64(ix);
  let b = y - f64(iy);
  let base = sums_offset + iy * (u32(n) + 1u) + ix;
  let s00 = f64(sums[base]);
  let s10 = f64(sums[base + 1u]);
  let s01 = f64(sums[base + u32(n) + 1u]);
  let s11 = f64(sums[base + u32(n) + 2u]);
  return s00 + (s10 - s00) * a + (s01 - s00) * b + (s11 - s10 - s01 + s00) * a * b;
}

fn periodic_sum(x: f64, y: f64, sums_offset: u32) -> f64 {
  let n = TILE;
  let qx = floor(x / n);
  let qy = floor(y / n);
  let rx = x - qx * n;
  let ry = y - qy * n;
  return inside(rx, ry, sums_offset)
    + qx * inside(n, ry, sums_offset)
    + qy * inside(rx, n, sums_offset)
    + qx * qy * inside(n, n, sums_offset);
}

/// Complete periodic texel-area light integral of one rectangle.
fn rectangle_light(x0: f64, x1: f64, y0: f64, y1: f64, sums_offset: u32, mean: f64) -> f64 {
  let area = (x1 - x0) * (y1 - y0);
  let s = periodic_sum(x1, y1, sums_offset) - periodic_sum(x0, y1, sums_offset)
    - periodic_sum(x1, y0, sums_offset) + periodic_sum(x0, y0, sums_offset);
  return mean + s / area;
}

/// The source's PCG hash of a block placement.
fn pcg(v: u32) -> u32 {
  let s = v * 747796405u + 2891336453u;
  let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
  return (w >> 22u) ^ w;
}

/// One block's light: periodic offset, orientation, and complete rectangle.
///
/// `turn` is the block hash's low three bits; the reference applies the two
/// mirror folds and the transpose in that order.
///
/// The return is f32, matching the reference's `rectangle_light(..) as f32
/// as f64`: the tile's prefix sums are f32, so the reference narrows the
/// integral to f32 before widening it back for the caller's f64 arithmetic.
fn block_light(rect: vec4<f64>, bx: i32, by: i32, seed: u32, r: u32, sums_offset: u32, mean: f64) -> f32 {
  let g = 0x9E3779B9u;
  let h = pcg(u32(bx) ^ pcg(u32(by) ^ pcg(seed ^ (200u + r) * g)));
  let ox = f64(h % 256u) + 64.0;
  let oy = f64((h >> 8u) % 256u) + 64.0;
  let turn = (h >> 16u) & 7u;
  var x0 = rect.x - f64(bx) * BLOCK;
  var x1 = rect.y - f64(bx) * BLOCK;
  var y0 = rect.z - f64(by) * BLOCK;
  var y1 = rect.w - f64(by) * BLOCK;
  if ((turn & 1u) != 0u) {
    let t = x0;
    x0 = BLOCK - x1;
    x1 = BLOCK - t;
  }
  if ((turn & 2u) != 0u) {
    let t = y0;
    y0 = BLOCK - y1;
    y1 = BLOCK - t;
  }
  if ((turn & 4u) != 0u) {
    let tx = x0;
    let tx1 = x1;
    // Preserve both x boundaries before transposing the rectangle.
    x0 = y0;
    x1 = y1;
    y0 = tx;
    y1 = tx1;
  }
  return f32(rectangle_light(x0 + ox, x1 + ox, y0 + oy, y1 + oy, sums_offset, mean));
}

/// The source's seam weight of a block center against a 128-texel block.
///
/// The clamp bounds are named f64 values: an abstract-float literal would
/// infer f32, which cannot clamp an f64 argument.
fn weight(c: f64, b: i32) -> f64 {
  let start = f64(b) * BLOCK;
  let lo = f64(0.0);
  let hi = f64(1.0);
  return clamp((c - start + 16.0) / 32.0, lo, hi) * clamp((start + 144.0 - c) / 32.0, lo, hi);
}

// The pitch pass: 128×128 probes per level, exactly the reference's lattice.
@compute @workgroup_size(8, 8)
fn pitch(@builtin(global_invocation_id) gid: vec3<u32>) {
  let x = gid.x;
  let y = gid.y;
  if (x >= PROBE || y >= PROBE) { return; }
  let px = f64((x * 7919u) % 4096u) * pitch_params.footprint_x;
  let py = f64((y * 104729u) % 4096u) * pitch_params.footprint_y;
  let rect = vec4<f64>(
    px, px + pitch_params.footprint_x,
    py, py + pitch_params.footprint_y,
  );
  let bx = i32(floor((px + pitch_params.footprint_x / 2.0) / BLOCK));
  let by = i32(floor((py + pitch_params.footprint_y / 2.0) / BLOCK));
  let base = (y * PROBE + x) * LEVELS;
  for (var k = 0u; k < LEVELS; k = k + 1u) {
    let sums_offset = k * 257u * 257u;
    let light = block_light(rect, bx, by, 0x5A3F1E1Du, pitch_params.record,
      sums_offset, means[k]);
    density[base + k] = LOG10_LN * (f64(0.0) - sf64_log(f64(max(light, 1e-9))));
  }
}

// Adjacent-level correlation over the probe lattice, one thread per pair.
@compute @workgroup_size(1)
fn correlate(@builtin(global_invocation_id) gid: vec3<u32>) {
  let k = gid.x;
  if (k >= LEVELS) { return; }
  // The statistics are f64: the probe densities are stored in f64, so the
  // means and the covariance accumulate in binary64.
  var mean_a: f64 = 0.0;
  var mean_b: f64 = 0.0;
  for (var i = 0u; i < PROBE * PROBE; i = i + 1u) {
    mean_a = mean_a + density[i * LEVELS + k];
    if (k + 1u < LEVELS) {
      mean_b = mean_b + density[i * LEVELS + k + 1u];
    }
  }
  mean_a = mean_a / f64(PROBE * PROBE);
  mean_b = mean_b / f64(PROBE * PROBE);
  probe_means[k] = mean_a;
  if (k + 1u >= LEVELS) { return; }
  var aa: f64 = 0.0;
  var bb: f64 = 0.0;
  var ab: f64 = 0.0;
  for (var i = 0u; i < PROBE * PROBE; i = i + 1u) {
    let a = density[i * LEVELS + k] - mean_a;
    let b = density[i * LEVELS + k + 1u] - mean_b;
    aa = aa + a * a;
    bb = bb + b * b;
    ab = ab + a * b;
  }
  // The correlation is a Pearson coefficient clamped to [-1, 1]; the bounds
  // are named f64 so the clamp's abstract literals match the f64 argument.
  let lo = f64(-1.0);
  let hi = f64(1.0);
  if (aa > 0.0 && bb > 0.0) {
    correlation[k] = clamp(ab / sqrt(aa * bb), lo, hi);
  } else {
    correlation[k] = hi;
  }
}

// The residual pass: one output pixel's grain sample.
struct ResidualParams {
  origin_x: f64,
  origin_y: f64,
  pitch_x: f64,
  pitch_y: f64,
  footprint_x: f64,
  footprint_y: f64,
  width: u32,
  height: u32,
  seed: u32,
  record: u32,
  // Alignment pad: the f64 group below needs 8-byte alignment.
  pad_align: u32,
  pad_align2: u32,
  d_min: f64,
  d_max: f64,
  monochrome: u32,
  residual_offset: u32,
  pad0: u32,
  pad1: u32,
}

@group(0) @binding(0) var<uniform> residual_params: ResidualParams;
@group(0) @binding(1) var<storage, read> residual_sums: array<f32>;
@group(0) @binding(2) var<storage, read> residual_means: array<f64>;
@group(0) @binding(3) var<storage, read> residual_correlation: array<f64>;
@group(0) @binding(4) var<storage, read> density_in: array<f32>;
@group(0) @binding(5) var<storage, read_write> residual_out: array<f64>;
@group(0) @binding(6) var<storage, read> residual_probe_means: array<f64>;

@compute @workgroup_size(8, 8)
fn residual(@builtin(global_invocation_id) gid: vec3<u32>) {
  let x = gid.x;
  let y = gid.y;
  if (x >= residual_params.width || y >= residual_params.height) { return; }
  let center = vec2<f64>(
    residual_params.origin_x + (f64(x) + 0.5) * residual_params.pitch_x,
    residual_params.origin_y + (f64(y) + 0.5) * residual_params.pitch_y,
  );
  let gross = f64(density_in[(y * residual_params.width + x) * 3u + residual_params.record]);
  // The level bracket's bounds are named f64 values: the abstract literals
  // `1e-6`, `0.0`, and `1.0` would infer f32 and cannot bind against the f64
  // arguments. The 16.0 scales the unit interval onto the 17 levels.
  let span = max(residual_params.d_max - residual_params.d_min, f64(1e-6));
  let lo = f64(0.0);
  let hi = f64(1.0);
  let t = clamp((gross - residual_params.d_min) / span, lo, hi) * 16.0;
  let k = min(u32(t), 15u);
  let w = t - f64(k);
  // The retained variance is floored at `1e-4` before the square root, so a
  // fully correlated pair still divides. The floor is a named f64 value.
  let kept = sqrt(max(
    sf64_powi(1.0 - w, 2u) + w * w
      + 2.0 * w * (1.0 - w) * residual_correlation[k],
    f64(1e-4)));
  let rect = vec4<f64>(
    center.x - residual_params.footprint_x / 2.0,
    center.x + residual_params.footprint_x / 2.0,
    center.y - residual_params.footprint_y / 2.0,
    center.y + residual_params.footprint_y / 2.0,
  );
  // Both accumulators are f64: `weight`, `block_light`'s inputs, and the
  // retained variance are binary64, and `residual_out` stores f64.
  var grain: f64 = 0.0;
  var norm: f64 = 0.0;
  for (var by = i32(floor((center.y - 16.0) / BLOCK));
    by <= i32(floor((center.y + 16.0) / BLOCK)); by = by + 1) {
    for (var bx = i32(floor((center.x - 16.0) / BLOCK));
      bx <= i32(floor((center.x + 16.0) / BLOCK)); bx = bx + 1) {
      let seam = weight(center.x, bx) * weight(center.y, by);
      if (seam <= 0.0) { continue; }
      // `block_light` returns the reference's f32-narrowed light; the log is
      // the reference's `log10`, formed from the shared natural log, so the
      // whole expression stays binary64. `1e-9` is the reference's
      // `.max(1e-9)` floor.
      let a = LOG10_LN * (f64(0.0) - sf64_log(f64(max(block_light(rect, bx, by,
        residual_params.seed, residual_params.record, k * 257u * 257u, residual_means[k]),
        1e-9)))) - residual_probe_means[k];
      let b = LOG10_LN * (f64(0.0) - sf64_log(f64(max(block_light(rect, bx, by,
        residual_params.seed, residual_params.record, (k + 1u) * 257u * 257u,
        residual_means[k + 1u]), 1e-9)))) - residual_probe_means[k + 1u];
      grain = grain + seam * (a + (b - a) * w) / kept;
      norm = norm + seam * seam;
    }
  }
  // The seam weights are normalized by their own L2 norm, floored at `1e-12`
  // so an empty footprint cannot divide by zero.
  let scale = sqrt(max(norm, f64(1e-12)));
  residual_out[residual_params.residual_offset + y * residual_params.width + x] =
    grain / scale;
}
