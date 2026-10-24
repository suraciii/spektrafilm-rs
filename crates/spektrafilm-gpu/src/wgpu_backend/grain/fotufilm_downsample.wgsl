// Fotufilm 3x3 box reduction — the device realization of the log-light
// reduction in `light`, `spektrafilm_model::grain::fotufilm::tiles`.
//
// The source reduces the 768×768 log-transmittance lattice to the 256×256
// canonical tile by averaging nine `exp` values in ascending dy then dx and
// dividing by nine, exactly as its final loop does. The device performs the
// same per-tile-texel map, in the same order, with binary32 `exp` as the
// source evaluates it.
//
// The reduction is out of place: the log-light lattice stays intact, and the
// tile is a separate output. The reference reads the full lattice, and an
// in-place map would let one work item overwrite a texel another still reads.

const SAMPLE: u32 = 768u;
const TILE: u32 = 256u;

// Binding 0 is this stage's uniform block, kept so every Fotufilm stage
// declares one; the reduction reads no field of it.
@group(0) @binding(0) var<uniform> params: vec4<u32>;
// The log-transmittance lattice.
@group(0) @binding(1) var<storage, read> log_light: array<f32>;
// The 256×256 canonical tile this pass writes.
@group(0) @binding(2) var<storage, read_write> tile: array<f32>;

// The 3×3 box average from 768 to 256: the source's `t / 9.0` of nine
// `exp` values, in ascending dy then dx.
@compute @workgroup_size(8, 8)
fn downsample(@builtin(global_invocation_id) gid: vec3<u32>) {
  let x = gid.x;
  let y = gid.y;
  if (x >= TILE || y >= TILE) { return; }
  var t = 0.0;
  for (var dy = 0u; dy < 3u; dy = dy + 1u) {
    for (var dx = 0u; dx < 3u; dx = dx + 1u) {
      t = t + exp(log_light[(3u * y + dy) * SAMPLE + 3u * x + dx]);
    }
  }
  tile[y * TILE + x] = t / 9.0;
}
