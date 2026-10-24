// Fotufilm compose — the device realization of `Bank::apply_image`'s colour
// mix and scaling in `spektrafilm_model::grain::fotufilm::tiles`.
//
// Each record's residual is scaled by `amount * layers[r] / size`, where
// `size` is the compensated grain scale and `softness` widens the footprint.
// The three records are then mixed: `own * grain[r] + shared * mean`, with
// the source's variance-compensated `shared`, and a monochrome stock uses
// only its single record. The result is added to the resampled density.

struct ComposeParams {
  amount: f64,
  size: f64,
  own: f64,
  // `layers` is a `vec4<f64>` rather than `array<f64, 3>`: a uniform array's
  // element stride must be a multiple of 16, and spelling the padded lane
  // explicitly keeps the host's `repr(C)` layout unambiguous.
  layers: vec4<f64>,
  width: u32,
  height: u32,
  records: u32,
  monochrome: u32,
}

@group(0) @binding(0) var<uniform> compose_params: ComposeParams;
// `records` slices of one f64 per output pixel.
@group(0) @binding(1) var<storage, read> residual: array<f64>;
@group(0) @binding(2) var<storage, read> density: array<f32>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let x = gid.x;
  let y = gid.y;
  if (x >= compose_params.width || y >= compose_params.height) { return; }
  let pixel = y * compose_params.width + x;
  let i = pixel * 3u;
  var grain = array<f64, 3>();
  for (var r = 0u; r < 3u; r = r + 1u) {
    if (compose_params.monochrome == 1u && r != 1u) { continue; }
    if (r >= compose_params.records) { continue; }
    grain[r] = residual[u32(r) * compose_params.width * compose_params.height + pixel]
      * compose_params.amount * compose_params.layers[r] / compose_params.size;
  }
  let mean = (grain[0] + grain[1] + grain[2]) / 3.0;
  // `shared` is a reserved WGSL keyword, so the variance-compensated shared
  // weight is named `shared_weight`.
  let shared_weight = sqrt(3.0 - 2.0 * compose_params.own * compose_params.own) - compose_params.own;
  for (var r = 0u; r < 3u; r = r + 1u) {
    let d = select(compose_params.own * grain[r] + shared_weight * mean, grain[1],
      compose_params.monochrome == 1u);
    output[i + r] = f32(f64(density[i + r]) + d);
  }
}
