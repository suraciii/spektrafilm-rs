// Fotufilm transmittance, tile, and centered running sums — the device
// realization of the capacity law and `TileLevel::from_light` in
// `spektrafilm_model::grain::fotufilm::tiles`.
//
// The source applies the capacity law to each of the 768×768 sample texels:
// a dye's demand is `-LN_10 * capacity * (1 - exp(-demand))`, a silver's
// transmittance mixes its uncovered accumulator against the fully exposed
// limit. It then reduces the log-transmittance lattice to the 256×256
// canonical tile by a 3×3 box average of `exp`, and builds the tile's centered
// running sums — the prefix sums the residual pass integrates.
//
// Every transcendental here is binary32, exactly as the source evaluates it:
// `exp` and `ln` on `f32` values. Only `TileLevel::from_light`'s mean and
// running sums are binary64, and those accumulate one row per work item in the
// source's ascending order.
//
// The 3×3 box reduction is its own module, `fotufilm_downsample.wgsl`, because
// its binding layout differs: this module's binding 1 is the tile the sums
// stage reads, while the reduction needs the full log-light lattice and a
// separate tile output, since an in-place reduction would let one work item
// overwrite a texel another still reads.

const SAMPLE: u32 = 768u;
const TILE: u32 = 256u;
const LN_10: f32 = 2.302585093;

struct CapacityParams {
  capacity: f32,
  silver: u32,
  demand_offset: u32,
  uncovered_offset: u32,
  log_offset: u32,
  pad0: u32,
  pad1: u32,
  pad2: u32,
}

@group(0) @binding(0) var<uniform> capacity_params: CapacityParams;
// One level's demand lattice: a dye's blurred deposits, a silver's Gaussian.
@group(0) @binding(1) var<storage, read> demand: array<f32>;
// Silver only: the log uncovered-area lattice.
@group(0) @binding(2) var<storage, read> uncovered: array<f32>;
// The level's accumulated log-transmittance lattice.
@group(0) @binding(3) var<storage, read_write> log_light: array<f32>;

@compute @workgroup_size(8, 8)
fn transmittance(@builtin(global_invocation_id) gid: vec3<u32>) {
  let x = gid.x;
  let y = gid.y;
  if (x >= SAMPLE || y >= SAMPLE) { return; }
  let i = y * SAMPLE + x;
  let demand = demand[capacity_params.demand_offset + i];
  let d = capacity_params.capacity * (1.0 - exp(-demand));
  if (capacity_params.silver == 0u) {
    log_light[capacity_params.log_offset + i] = log_light[capacity_params.log_offset + i]
      - LN_10 * d;
  } else {
    let u = exp(uncovered[capacity_params.uncovered_offset + i]);
    let full = exp(-LN_10 * capacity_params.capacity);
    let passed = u * exp(-LN_10 * d) + (1.0 - u) * full;
    log_light[capacity_params.log_offset + i] = log_light[capacity_params.log_offset + i]
      + log(max(passed, 1e-12));
  }
}

struct SumsParams {
  tile_offset: u32,
  sums_offset: u32,
  mean_offset: u32,
  pad0: u32,
}

@group(0) @binding(0) var<uniform> sums_params: SumsParams;
@group(0) @binding(1) var<storage, read> tile: array<f32>;
@group(0) @binding(2) var<storage, read_write> sums: array<f32>;
// One f64 per level: the tile's mean transmittance, the source's
// `TileLevel::mean_light`. Written once by the level's first row and read by
// the pitch and residual passes.
@group(0) @binding(3) var<storage, read_write> level_means: array<f64>;

// One invocation preserves the reference's y-major column accumulation and
// f64 running sums. The zero border is supplied by cleared device storage.
@compute @workgroup_size(1)
fn running_sums(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (gid.x != 0u) { return; }
  var mean: f64 = 0.0;
  for (var i = 0u; i < TILE * TILE; i = i + 1u) {
    mean = mean + f64(tile[sums_params.tile_offset + i]);
  }
  mean = mean / f64(TILE * TILE);
  level_means[sums_params.mean_offset] = f64(f32(mean));
  var columns = array<f64, TILE>();
  for (var y = 0u; y < TILE; y = y + 1u) {
    var run: f64 = 0.0;
    for (var x = 0u; x < TILE; x = x + 1u) {
      columns[x] = columns[x] + (f64(tile[sums_params.tile_offset + y * TILE + x]) - mean);
      run = run + columns[x];
      sums[sums_params.sums_offset + (y + 1u) * (TILE + 1u) + x + 1u] = f32(run);
    }
  }
}
