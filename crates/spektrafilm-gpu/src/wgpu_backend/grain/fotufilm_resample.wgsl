// Conservative developed-density resampling over globally addressed source pages.
// Each page contributes in ascending source order to a retained f64 accumulator.
// Clamped edge weights are merged before normalization, as axis_spans does.
struct ResampleParams {
  origin_x: f64,
  origin_y: f64,
  pitch_x: f64,
  pitch_y: f64,
  source_pitch: f64,
  pad_align: u32,
  pad_align2: u32,
  source_width: u32,
  source_height: u32,
  output_width: u32,
  output_height: u32,
  output_x: u32,
  output_y: u32,
  page_first: u32,
  page_pixels: u32,
}
@group(0) @binding(0) var<uniform> params: ResampleParams;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> accumulated: array<f64>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;
fn overlap(lo: f64, hi: f64, index: i32, last: i32) -> f64 {
  let cell_lo = select(f64(index) * params.source_pitch, f64(-1.0e300), index <= 0);
  let cell_hi = select(f64(index + 1) * params.source_pitch, f64(1.0e300), index >= last);
  return max(min(hi, cell_hi) - max(lo, cell_lo), f64(0.0));
}
fn axis_total(lo: f64, hi: f64, last: i32) -> f64 {
  var total: f64 = 0.0;
  var merged: f64 = 0.0;
  var previous = -1;
  for (var i = i32(floor(lo / params.source_pitch)) - 1;
       i <= i32(ceil(hi / params.source_pitch)) + 1; i = i + 1) {
    let w = overlap(lo, hi, i, last);
    if (w <= 0.0) { continue; }
    let clamped = clamp(i, 0, last);
    if (previous != clamped) {
      total = total + merged;
      merged = 0.0;
      previous = clamped;
    }
    merged = merged + w;
  }
  return total + merged;
}
fn axis_weight(lo: f64, hi: f64, index: i32, last: i32) -> f64 {
  var weight: f64 = 0.0;
  let first = i32(floor(lo / params.source_pitch)) - 1;
  let end = i32(ceil(hi / params.source_pitch)) + 1;
  // Only merge virtual indices visited by axis_spans. Outside the source,
  // every virtual edge cell overlaps the whole interval; including indices
  // before first/after end would count extra copies and amplify density.
  let a = max(select(index, first, index == 0), first);
  let b = min(select(index, end, index == last), end);
  for (var i = a; i <= b; i = i + 1) {
    weight = weight + overlap(lo, hi, i, last);
  }
  return weight;
}
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (gid.x >= params.output_width || gid.y >= params.output_height) { return; }
  let x0 = params.origin_x + f64(params.output_x + gid.x) * params.pitch_x;
  let x1 = x0 + params.pitch_x;
  let y0 = params.origin_y + f64(params.output_y + gid.y) * params.pitch_y;
  let y1 = y0 + params.pitch_y;
  let last_x = i32(params.source_width) - 1;
  let last_y = i32(params.source_height) - 1;
  let total_x = axis_total(x0, x1, last_x);
  let total_y = axis_total(y0, y1, last_y);
  let first_x = clamp(i32(floor(x0 / params.source_pitch)) - 1, 0, last_x);
  let end_x = clamp(i32(ceil(x1 / params.source_pitch)) + 1, 0, last_x);
  let first_y = clamp(i32(floor(y0 / params.source_pitch)) - 1, 0, last_y);
  let end_y = clamp(i32(ceil(y1 / params.source_pitch)) + 1, 0, last_y);
  let out = (gid.y * params.output_width + gid.x) * 3u;
  var sum = vec3<f64>(accumulated[out], accumulated[out + 1u], accumulated[out + 2u]);
  for (var y = first_y; y <= end_y; y = y + 1) {
    let wy = axis_weight(y0, y1, y, last_y) / total_y;
    if (wy <= 0.0) { continue; }
    for (var x = first_x; x <= end_x; x = x + 1) {
      let pixel = u32(y) * params.source_width + u32(x);
      if (pixel < params.page_first || pixel - params.page_first >= params.page_pixels) { continue; }
      let wx = axis_weight(x0, x1, x, last_x) / total_x;
      if (wx <= 0.0) { continue; }
      let i = (pixel - params.page_first) * 3u;
      // Match the CPU's x_weight * y_weight * density evaluation order.
      sum = sum + wx * wy * vec3<f64>(f64(source[i]), f64(source[i + 1u]), f64(source[i + 2u]));
    }
  }
  accumulated[out] = sum.x;
  accumulated[out + 1u] = sum.y;
  accumulated[out + 2u] = sum.z;
}
@compute @workgroup_size(8, 8)
fn finalize(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (gid.x >= params.output_width || gid.y >= params.output_height) { return; }
  let i = (gid.y * params.output_width + gid.x) * 3u;
  output[i] = f32(accumulated[i]);
  output[i + 1u] = f32(accumulated[i + 1u]);
  output[i + 2u] = f32(accumulated[i + 2u]);
}
