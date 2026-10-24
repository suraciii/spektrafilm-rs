// Fotufilm formation — the pinned crystal coating, drawn exactly as the
// reference's `light` in `spektrafilm_model::grain::fotufilm::tiles`.
//
// The reference walks a coating grid of `cells × cells` cells, one per
// `0.256 mm / cells` square. For each cell it draws a Poisson count by
// counting the 48 cumulative thresholds below one uniform, then deposits each
// drawn crystal's mark into a 768×768 periodic texel lattice: a dye crystal
// scatters into the four texels around its position with bilinear weights, a
// silver crystal scatters a Gaussian of radius `4.2 sigma` and records its
// uncovered square as a log-area accumulator.
//
// Stream identity is per cell: `key(seed, stream, cx, cy)` hashes the cell
// coordinates with the source's PCG, and `uniform(k, i)` derives each word in
// draw order. The device reproduces both bit for bit, so a crystal's position,
// activation draw and mark are the reference's.
//
// One work item owns one coating cell and deposits that cell's crystals into a
// private region of the band's scratch lattice. The region is the cell's own
// texel span widened by the sublayer's mark margin, so a crystal never
// deposits outside it. `reduce` then adds the band's regions into the shared
// demand lattice, visiting the band's cells in ascending order — the
// reference's `for cy { for cx { for c } }` — so a texel's additions arrive in
// the reference's order: a cell's crystals accumulate into its own region in
// ascending crystal order, and the regions are added in ascending cell order.
//
// `light` re-runs the coating for each of the 17 bank levels because the
// developing fraction is gross-dependent. The host therefore runs one
// formation pass per level with that level's gross density.
//
// The crystal arithmetic is f32, exactly as the reference's: the mark is a
// binary32 running mean of four `-ln(u)` samples, and the deposits are
// binary32 products. A silver crystal's uncovered square needs
// `population::occupied_area`, whose silver branch is ported below in binary64
// because the reference evaluates it in binary64.

const SAMPLE: u32 = 768u;
/// Poisson count thresholds per coating cell, the model's `N_COUNTS`.
const THRESHOLD_COUNT: u32 = 48u;
/// The bounded crystal budget: the count law has exactly 48 thresholds.
const MAX_CRYSTALS: u32 = 48u;
const GOLDEN: u32 = 0x9E3779B9u;
/// The model's `PI`, the binary64 literal `std::f64::consts::PI` names.
const PI: f64 = 3.141592653589793;
/// The model's Euler constant, stored as a decimal literal in the source.
const EULER: f64 = 0.5772156649;

struct FormationParams {
  cells: u32,
  record: u32,
  sublayer: u32,
  seed: u32,
  first_row: u32,
  rows: u32,
  silver: u32,
  margin: u32,
  region_w: u32,
  max_crystals: u32,
  cells_in_band: u32,
  // Alignment pad: the f64 group below needs 8-byte alignment.
  pad_align: u32,
  // This bank level's gross density and the record's density bounds, as the
  // model reports them: `forming` divides by the record's own span.
  gross: f64,
  d_min: f64,
  d_max: f64,
  cell_texels: f32,
  sigma: f32,
  resolved: f32,
  edge: f32,
  pad0: u32,
  pad1: u32,
  pad2: u32,
  pad3: u32,
}

@group(0) @binding(0) var<uniform> params: FormationParams;
// 48 cumulative Poisson thresholds of this sublayer's coating cell.
@group(0) @binding(1) var<storage, read> thresholds: array<f32>;
// 129 developing fractions, in f64 exactly as the model stores them.
@group(0) @binding(2) var<storage, read> forming: array<f64>;
// [cell][region] deposit scratch of this band.
@group(0) @binding(3) var<storage, read_write> deposit_scratch: array<f32>;
// Silver only: the same layout for the log uncovered-area accumulator.
@group(0) @binding(4) var<storage, read_write> uncovered_scratch: array<f32>;
// One bounded execution-status word, never a numerical intermediate.
@group(0) @binding(5) var<storage, read_write> status: atomic<u32>;

fn pcg(v: u32) -> u32 {
  let s = v * 747796405u + 2891336453u;
  let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
  return (w >> 22u) ^ w;
}

fn key(seed: u32, stream: u32, x: u32, y: u32) -> u32 {
  return pcg(x ^ pcg(y ^ pcg(seed ^ stream * GOLDEN)));
}

fn uniform(k: u32, i: u32) -> f32 {
  return (f32(pcg(k ^ i * GOLDEN) >> 8u) + 0.5) * (1.0 / 16777216.0);
}

fn wrap(v: i32, n: u32) -> u32 {
  let m = i32(n);
  var r = v % m;
  if (r < 0) { r = r + m; }
  return u32(r);
}

/// The reference's `forming`, evaluated for this level's gross density.
///
/// `forming` forms `t` in binary64, indexes with the truncated value, and
/// narrows the interpolated result to f32 only at the end; the comparison
/// against a crystal's `develop` draw is then f32 against f32.
fn developing_fraction() -> f32 {
  let span = max(params.d_max - params.d_min, f64(1e-6));
  let t = clamp((params.gross - params.d_min) / span, f64(0.0), f64(1.0)) * 128.0;
  let i = min(u32(t), 127u);
  let f = t - f64(i);
  return f32(forming[i] * (1.0 - f) + forming[i + 1u] * f);
}

/// The reference's `population::occupied_area` silver branch: `2*PI*E1(peak)`.
///
/// The reference keeps the series below six and the asymptotic expansion above
/// it, with Euler's constant as a decimal literal; both are ported in binary64
/// so a silver crystal's uncovered square is the reference's.
fn occupied_area(peak: f64) -> f64 {
  var ein: f64 = 0.0;
  if (peak < 6.0) {
    var term = peak;
    var sum: f64 = 0.0;
    for (var k = 1; k <= 60; k = k + 1) {
      sum = sum + term / f64(k);
      term = term * (-peak / f64(k + 1));
      if (abs(term) < 1e-14) { break; }
    }
    ein = sum;
  } else {
    ein = sf64_exp(-peak) / peak
      * (1.0 - 1.0 / peak + 2.0 / sf64_powi(peak, 2u) - 6.0 / sf64_powi(peak, 3u))
      + sf64_log(peak) + EULER;
  }
  return 2.0 * PI * ein;
}

/// One cell's region origin along one axis: the cell's own edge, less the
/// margin the widest mark of this sublayer can reach beyond it.
fn region_origin(cell: u32) -> i32 {
  return i32(floor(f32(cell) * params.cell_texels)) - i32(params.margin);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let cell = gid.x;
  if (cell >= params.cells_in_band) { return; }
  let cy = params.first_row + cell / params.cells;
  let cx = cell % params.cells;
  let stream_count = params.record * 8u + params.sublayer * 2u;
  let count_key = key(params.seed, stream_count + 1u, cx, cy);
  let u = uniform(count_key, 0u);
  var count = 0u;
  for (var t = 0u; t < THRESHOLD_COUNT; t = t + 1u) {
    if (thresholds[t] < u) { count = count + 1u; }
  }
  if (count > params.max_crystals) {
    atomicMax(&status, 1u);
    return;
  }
  let k = key(params.seed, stream_count, cx, cy);
  let fraction = developing_fraction();
  let region = params.region_w * params.region_w;
  let base = cell * region;
  // The scratch is freshly allocated device memory, so the work item that owns
  // a region clears it before depositing. Clearing is not part of the law: an
  // untouched texel contributes nothing to the reduction.
  for (var i = 0u; i < region; i = i + 1u) {
    deposit_scratch[base + i] = 0.0;
    uncovered_scratch[base + i] = 0.0;
  }
  let x0 = region_origin(cx);
  let y0 = region_origin(cy);
  for (var c = 0u; c < count; c = c + 1u) {
    let i = c * 7u;
    let px = (f32(cx) + uniform(k, i)) * params.cell_texels;
    let py = (f32(cy) + uniform(k, i + 1u)) * params.cell_texels;
    let develop = uniform(k, i + 2u);
    // The mark is the binary32 running mean of four `-ln(u)` samples, with the
    // reference's `1e-7` floor on each draw.
    var mark = 0.0;
    for (var j = 0u; j < 4u; j = j + 1u) {
      mark = mark - log(max(uniform(k, i + 3u + j), 1e-7));
    }
    mark = mark / 4.0;
    if (develop >= fraction) { continue; }
    let peak = params.edge * mark;
    if (params.silver == 0u) {
      // Bilinear scatter into the four texels around the position, with the
      // reference's `px - 0.5` centring and its `(dy, dx)` tap order.
      let uu = px - 0.5;
      let vv = py - 0.5;
      let ix = i32(floor(uu));
      let iy = i32(floor(vv));
      let ax = uu - f32(ix);
      let ay = vv - f32(iy);
      for (var dy = 0u; dy < 2u; dy = dy + 1u) {
        let wy = select(1.0 - ay, ay, dy == 1u);
        for (var dx = 0u; dx < 2u; dx = dx + 1u) {
          let wx = select(1.0 - ax, ax, dx == 1u);
          let tx = ix + i32(dx) - x0;
          let ty = iy + i32(dy) - y0;
          if (tx < 0 || ty < 0 || tx >= i32(params.region_w) || ty >= i32(params.region_w)) {
            atomicMax(&status, 2u);
            return;
          }
          let index = base + u32(ty) * params.region_w + u32(tx);
          deposit_scratch[index] = deposit_scratch[index] + peak * wx * wy;
        }
      }
    } else {
      if (params.resolved > 0.0) {
        let reach = 4.2 * params.sigma;
        let xa = i32(floor(px - reach));
        let xb = i32(ceil(px + reach));
        let ya = i32(floor(py - reach));
        let yb = i32(ceil(py + reach));
        for (var y = ya; y <= yb; y = y + 1) {
          let dy = f32(y) + 0.5 - py;
          let row = peak * params.resolved * exp(-dy * dy / (2.0 * params.sigma * params.sigma));
          if (row < 1e-5) { continue; }
          for (var x = xa; x <= xb; x = x + 1) {
            let dx = f32(x) + 0.5 - px;
            let tx = x - x0;
            let ty = y - y0;
            if (tx < 0 || ty < 0 || tx >= i32(params.region_w) || ty >= i32(params.region_w)) {
              atomicMax(&status, 2u);
              return;
            }
            let index = base + u32(ty) * params.region_w + u32(tx);
            deposit_scratch[index] = deposit_scratch[index]
              + row * exp(-dx * dx / (2.0 * params.sigma * params.sigma));
          }
        }
      }
      if (params.resolved < 1.0) {
        // The uncovered square spans `sigma * sqrt(occupied_area(peak))`. The
        // model owns that integral; the port above evaluates the reference's
        // own silver branch for this crystal's mark.
        let side = params.sigma * sqrt(f32(occupied_area(f64(peak))));
        let left = px - side / 2.0;
        let right = px + side / 2.0;
        let top = py - side / 2.0;
        let bottom = py + side / 2.0;
        for (var y = i32(floor(top)); y <= i32(floor(bottom)); y = y + 1) {
          let oy = min(bottom, f32(y) + 1.0) - max(top, f32(y));
          if (oy <= 0.0) { continue; }
          for (var x = i32(floor(left)); x <= i32(floor(right)); x = x + 1) {
            let ox = min(right, f32(x) + 1.0) - max(left, f32(x));
            if (ox > 0.0) {
              let tx = x - x0;
              let ty = y - y0;
              if (tx < 0 || ty < 0 || tx >= i32(params.region_w) || ty >= i32(params.region_w)) {
                atomicMax(&status, 2u);
                return;
              }
              let index = base + u32(ty) * params.region_w + u32(tx);
              uncovered_scratch[index] = uncovered_scratch[index]
                + log(max(1.0 - (1.0 - params.resolved) * ox * oy, 1e-6));
            }
          }
        }
      }
    }
  }
}

struct ReduceParams {
  cells: u32,
  first_row: u32,
  rows: u32,
  margin: u32,
  region_w: u32,
  // The first band row of this chunk, and how many rows it owns. A chunk
  // never spans more than the periodic lattice, so each output row is written
  // by exactly one work item of the chunk.
  chunk_first: i32,
  chunk_rows: u32,
  cell_texels: f32,
  pad0: u32,
  pad1: u32,
  pad2: u32,
}

@group(0) @binding(0) var<uniform> reduce_params: ReduceParams;
// [cell][region] deposit scratch of this band.
@group(0) @binding(1) var<storage, read> scratch: array<f32>;
// [cell][region] uncovered scratch of this band.
@group(0) @binding(2) var<storage, read> reduce_uncovered_scratch: array<f32>;
// The shared demand lattice this chunk reduces into.
@group(0) @binding(3) var<storage, read_write> field: array<f32>;
// The shared uncovered lattice this chunk reduces into.
@group(0) @binding(4) var<storage, read_write> uncovered: array<f32>;

fn reduce_region_origin(cell: u32) -> i32 {
  return i32(floor(f32(cell) * reduce_params.cell_texels)) - i32(reduce_params.margin);
}

// One work item per band row of the chunk. It visits the band's cells in
// ascending order — the reference's `for cy { for cx }` — and adds each cell's
// region row into the shared lattice, wrapping both axes. A cell's region
// overlaps only the rows it spans, so a work item visits the few cells whose
// region covers its row.
@compute @workgroup_size(64)
fn reduce(@builtin(global_invocation_id) gid: vec3<u32>) {
  let chunk_row = gid.x;
  if (chunk_row >= reduce_params.chunk_rows) { return; }
  let y = reduce_params.chunk_first + i32(chunk_row);
  let out_row = wrap(y, SAMPLE) * SAMPLE;
  let cells = reduce_params.rows * reduce_params.cells;
  let region = reduce_params.region_w * reduce_params.region_w;
  for (var cell = 0u; cell < cells; cell = cell + 1u) {
    let cy = reduce_params.first_row + cell / reduce_params.cells;
    let ty = y - reduce_region_origin(cy);
    if (ty < 0 || ty >= i32(reduce_params.region_w)) { continue; }
    let cx = cell % reduce_params.cells;
    let x_origin = reduce_region_origin(cx);
    let base = cell * region + u32(ty) * reduce_params.region_w;
    for (var rx = 0u; rx < reduce_params.region_w; rx = rx + 1u) {
      let index = out_row + wrap(x_origin + i32(rx), SAMPLE);
      let value = scratch[base + rx];
      // Adding zero is the identity, so skipping an untouched region texel is
      // the reference's own result rather than an approximation of it.
      if (value != 0.0) {
        field[index] = field[index] + value;
      }
      let cover = reduce_uncovered_scratch[base + rx];
      if (cover != 0.0) {
        uncovered[index] = uncovered[index] + cover;
      }
    }
  }
}
