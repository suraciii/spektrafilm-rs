// Scanner slide points: per-channel scanned-density rescaling through the
// black/white point window, with linear tangent extrapolation outside the
// calibrated capture range.
//
// CPU equivalent: `SlidePoints::map` / `apply` in `stages::scanning`. Per
// channel, with the host-resolved constants `clear`, `dense`, the endpoint
// output/tangent pairs, and the point map (`start`, `span`, `black`,
// `scale`):
//   value >= clear[c]  → endpoint_output[0][c] + (value - clear[c]) * slope
//   value <= dense[c]  → endpoint_output[1][c] + (value - dense[c]) * slope
//   otherwise          → clear[c] * 10^points.apply(log10(value/clear[c]))
// where `points.apply(t) = start + (t - start - black*span) * scale`, and a
// channel whose window is the default passes its value through unchanged.

// Every three-component member is a `vec4<f32>` so the WGSL uniform
// layout matches the host `#[repr(C)]` struct word for word: a `vec3<f32>`
// aligns to 16 bytes in the uniform address space, which the host's
// `[f32; 4]` does not, and the two layouts would disagree. The fourth
// component is never read.
struct Params {
    n_pixels: u32,
    identity_mask: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    clear: vec4<f32>,
    dense: vec4<f32>,
    endpoint_output_0: vec4<f32>,
    endpoint_output_1: vec4<f32>,
    endpoint_slope_0: vec4<f32>,
    endpoint_slope_1: vec4<f32>,
    start: f32,
    span: f32,
    black: vec4<f32>,
    scale: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;          // [H*W*3]
@group(0) @binding(2) var<storage, read_write> output: array<f32>;   // [H*W*3]

fn point_apply(t: f32, channel: u32) -> f32 {
    // `start + (t - start - black*span) * scale`
    return params.start
        + (t - params.start - params.black[channel] * params.span) * params.scale[channel];
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= params.n_pixels {
        return;
    }
    let base = pixel * 3u;
    for (var c = 0u; c < 3u; c++) {
        let value = input[base + c];
        // A default window is an exact passthrough, kept out of the
        // arithmetic path so the identity costs no rounding.
        if (params.identity_mask & (1u << c)) != 0u {
            output[base + c] = value;
            continue;
        }
        var mapped: f32;
        if value >= params.clear[c] {
            mapped = params.endpoint_output_0[c] + (value - params.clear[c]) * params.endpoint_slope_0[c];
        } else if value <= params.dense[c] {
            mapped = params.endpoint_output_1[c] + (value - params.dense[c]) * params.endpoint_slope_1[c];
        } else {
            mapped = params.clear[c] * pow(10.0, point_apply(log(value / params.clear[c]) / log(10.0), c));
        }
        output[base + c] = mapped;
    }
}
