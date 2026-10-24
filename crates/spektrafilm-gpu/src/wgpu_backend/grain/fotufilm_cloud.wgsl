// Fotufilm dye cloud — the device realization of `cloud_term` in
// `spektrafilm_model::grain::fotufilm::tiles`.
//
// A dye crystal's mark becomes a dye cloud. The source blurs the deposit
// lattice with four Gaussian terms whose spreads are `s * sigma` and whose
// weights sum to one. A term is either blurred directly, or — when its coarse
// factor divides the sample edge — prefiltered with a `factor`-wide tent,
// blurred at the coarse lattice, and upsampled with bilinear interpolation.
// The device reproduces both branches.
//
// The direct blur is separable: a work item owns one output texel and
// accumulates `2 * radius + 1` taps. The radius is the source's
// `ceil(3.5 * sigma)` with a floor of one, and the taps are normalized by
// their own sum, so the kernel is the source's.
//
// The tent prefilter is one pass of the coarse branch: a work item owns one
// coarse cell and accumulates `3 * factor` taps of the source's own tent,
// `max(0, 1 - |(t + 0.5)/factor - 1.5|)`, centered on the coarse cell.
//
// Transcendentals are binary32, exactly as the source evaluates them: the
// kernel and the tent are f32 values, and only the blur's sigma arithmetic
// widens.

const SAMPLE: u32 = 768u;

struct BlurParams {
  n: u32,
  radius: i32,
  vertical: u32,
  scale: f32,
  sigma: f32,
  // Coarse factor of the tent pass; zero for a plain blur.
  factor: u32,
  pad0: u32,
  pad1: u32,
}

@group(0) @binding(0) var<uniform> blur_params: BlurParams;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> target_field: array<f32>;

// Normalized taps prepared once per term, not once per output texel.
@group(0) @binding(3) var<storage, read_write> kernel: array<f32>;

@compute @workgroup_size(1)
fn prepare_kernel(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (gid.x != 0u) { return; }
  let radius = blur_params.radius;
  let sigma = blur_params.sigma;
  var sum = 0.0;
  for (var t = -radius; t <= radius; t = t + 1) {
    let w = exp(-f32(t * t) / (2.0 * sigma * sigma));
    kernel[u32(t + radius)] = w;
    sum = sum + w;
  }
  for (var t = 0; t <= 2 * radius; t = t + 1) {
    kernel[u32(t)] = kernel[u32(t)] / sum;
  }
}
fn wrap(v: i32, n: i32) -> i32 {
  var r = v % n;
  if (r < 0) { r = r + n; }
  return r;
}

/// One separable pass of the source's `blur`: normalized Gaussian taps, with
/// the source's radius and normalization, over a periodic lattice.
///
/// The vertical pass reads the horizontal pass's output, and the scale is
/// applied by the second pass, exactly as the source applies it after the
/// separable pair.
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let x = gid.x;
  let y = gid.y;
  let n = i32(blur_params.n);
  if (x >= u32(n) || y >= u32(n)) { return; }
  let radius = blur_params.radius;
  // The kernel entry preserves the reference's f32 tap generation, sum and
  // normalization order; both separable passes read those exact same taps.
  var acc = 0.0;
  for (var t = -radius; t <= radius; t = t + 1) {
    let w = kernel[u32(t + radius)];
    let s = wrap(i32(x) + t, n);
    let v = wrap(i32(y) + t, n);
    let value = source[
      select(v * n + i32(x), i32(y) * n + s, blur_params.vertical == 0u)
    ];
    acc = acc + w * value;
  }
  target_field[i32(y) * n + i32(x)] = acc * blur_params.scale;
}

/// One coarse cell's tent prefilter, the source's own tap count and weights.
///
/// The horizontal pass reduces `SAMPLE` source texels into `m` coarse cells
/// along x; the vertical pass reduces the `m` coarse cells of each source row
/// into the `m × m` coarse lattice along y.
@compute @workgroup_size(8, 8)
fn tent(@builtin(global_invocation_id) gid: vec3<u32>) {
  let x = gid.x;
  let y = gid.y;
  let factor = i32(blur_params.factor);
  let m = i32(blur_params.n);
  let horizontal = blur_params.vertical == 0u;
  let height = select(m, i32(SAMPLE), horizontal);
  if (x >= u32(m) || y >= u32(height)) { return; }
  var acc = 0.0;
  for (var t = 0; t < 3 * factor; t = t + 1) {
    // The source's own tent: `max(0, 1 - |(t + 0.5)/factor - 1.5|)`.
    let w = max(1.0 - abs((f32(t) + 0.5) / f32(factor) - 1.5), 0.0);
    if (w == 0.0) { continue; }
    let index = wrap(factor * select(i32(y), i32(x), horizontal) - factor + t, i32(SAMPLE));
    let value = source[select(index * m + i32(x), i32(y) * i32(SAMPLE) + index, horizontal)];
    acc = acc + w * value;
  }
  target_field[i32(y) * m + i32(x)] = acc;
}

// Both branches add a cloud term to a separate, initially zero demand lattice.
// Every term reads the same original deposit lattice, not the evolving sum.
@compute @workgroup_size(8, 8)
fn accumulate(@builtin(global_invocation_id) gid: vec3<u32>) {
  let x = gid.x;
  let y = gid.y;
  if (x >= SAMPLE || y >= SAMPLE) { return; }
  let i = y * SAMPLE + x;
  target_field[i] = target_field[i] + source[i];
}

@compute @workgroup_size(8, 8)
fn upsample_add(@builtin(global_invocation_id) gid: vec3<u32>) {
  let x = gid.x;
  let y = gid.y;
  if (x >= SAMPLE || y >= SAMPLE) { return; }
  let f = f32(blur_params.factor);
  let m = i32(blur_params.n);
  let u = (f32(x) + 0.5) / f - 0.5;
  let v = (f32(y) + 0.5) / f - 0.5;
  let ix = i32(floor(u));
  let iy = i32(floor(v));
  let ax = u - f32(ix);
  let ay = v - f32(iy);
  let a = (source[wrap(iy, m) * m + wrap(ix, m)] * (1.0 - ax)
    + source[wrap(iy, m) * m + wrap(ix + 1, m)] * ax) * (1.0 - ay);
  let b = (source[wrap(iy + 1, m) * m + wrap(ix, m)] * (1.0 - ax)
    + source[wrap(iy + 1, m) * m + wrap(ix + 1, m)] * ax) * ay;
  let i = y * SAMPLE + x;
  target_field[i] = target_field[i] + blur_params.scale / (f * f) * (a + b);
}
