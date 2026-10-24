// Separable negative-to-positive interpretation: invert each channel's
// calibrated capture curve to relative log-exposure, then evaluate the
// conversion tone curve.
//
// CPU equivalent: `interpret_negative_capture` + `PositiveChannel::estimate`
// + `positive_tone` in `stages::scanning`.
//
// The host resolves the pooled monotone inverse knots, the per-channel
// endpoint capture/output/tangent triplets, and the tone constants, so the
// device port performs the same three steps:
//   1. invert: `d = -log10(value / clear)`, then the same
//      `partition_point(|x| x < d)` binary search over the ascending density
//      knots with the reference's linear interpolation;
//   2. tone: `positive_tone(log_e, channel)` — the logistic response under
//      the point map, with its analytic derivative;
//   3. endpoint branches above the clear-side capture and below the
//      dense-side capture use the same tangent extrapolation the reference
//      derives from that derivative at the first and last knot.

// Every three-component member is a `vec4<f32>` so the WGSL uniform layout
// matches the host `#[repr(C)]` struct word for word: a `vec3<f32>` aligns
// to 16 bytes in the uniform address space, which the host's `[f32; 4]`
// does not, and the two layouts would disagree. The fourth component is
// never read.
struct Params {
    n_pixels: u32,
    knots: u32,
    identity_mask: u32,
    _pad0: u32,
    start: f32,
    span: f32,
    black: vec4<f32>,
    scale: vec4<f32>,
    clear: vec4<f32>,
    endpoint_capture_0: vec4<f32>,
    endpoint_capture_1: vec4<f32>,
    endpoint_output_0: vec4<f32>,
    endpoint_output_1: vec4<f32>,
    endpoint_slope_0: vec4<f32>,
    endpoint_slope_1: vec4<f32>,
    contrast: f32,
    dmax: f32,
    brightness_ev: f32,
    balance_ev: vec4<f32>,
    camera_ev: f32,
    ln10: f32,
    log10_2: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;          // [H*W*3]
@group(0) @binding(2) var<storage, read> density: array<f32>;        // [knots*3]
@group(0) @binding(3) var<storage, read> exposure: array<f32>;       // [knots*3]
@group(0) @binding(4) var<storage, read_write> output: array<f32>;   // [H*W*3]

// WGSL has no `expm1` builtin, so the reference's `expm1(x)` is evaluated
// as `exp(x) - 1` — the same expression, not a different function. The
// reference itself uses this form for the positive branch.
fn expm1_f32(x: f32) -> f32 {
    return exp(x) - 1.0;
}

fn point_apply(t: f32, channel: u32) -> f32 {
    // A default per-channel window is an exact passthrough, kept out of the
    // arithmetic so the identity costs no rounding.
    if params.black[channel] == 0.0 && params.scale[channel] == 1.0 {
        return t;
    }
    return params.start
        + (t - params.start - params.black[channel] * params.span) * params.scale[channel];
}

/// Piecewise-linear inverse of the channel's calibration knots, mirroring
/// `PositiveChannel::estimate`: `partition_point(|x| x < d)` clamped into
/// `1..knots-1`, then linear interpolation between the bracketing knots.
fn invert(d: f32, channel: u32) -> f32 {
    var lower = 0u;
    var upper = params.knots;
    while lower < upper {
        let mid = (lower + upper) / 2u;
        if density[mid * 3u + channel] < d {
            lower = mid + 1u;
        } else {
            upper = mid;
        }
    }
    var idx = clamp(lower, 1u, params.knots - 1u);
    let lo = idx - 1u;
    let d_lo = density[lo * 3u + channel];
    let d_hi = density[idx * 3u + channel];
    let fraction = (d - d_lo) / (d_hi - d_lo);
    return exposure[lo * 3u + channel]
        + fraction * (exposure[idx * 3u + channel] - exposure[lo * 3u + channel]);
}

/// The conversion tone curve and its derivative, mirroring
/// `positive_tone(log_e, channel, points, params)`.
fn tone(log_e: f32, channel: u32) -> vec2<f32> {
    let contrast = params.contrast;
    let dmax = params.dmax;
    let denominator = -expm1_f32(-params.ln10 * dmax);
    let mapped = point_apply(log_e, channel);
    let e = mapped / params.log10_2 - params.camera_ev + params.brightness_ev
        + params.balance_ev[channel];
    let z = contrast * e;
    // Numerically stable logistic: the reference branches on the sign of z.
    let t = select(exp(z) / (1.0 + exp(z)), 1.0 / (1.0 + exp(-z)), z >= 0.0);
    let density = dmax * (1.0 - t);
    let positive = expm1_f32(-params.ln10 * density);
    let value = (positive + denominator) / denominator;
    let derivative = params.ln10 * dmax * contrast * t * (1.0 - t)
        * exp(-params.ln10 * density)
        / (denominator * params.log10_2)
        * params.scale[channel];
    return vec2<f32>(value, derivative);
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
        var mapped: f32;
        if value >= params.endpoint_capture_0[c] {
            mapped = params.endpoint_output_0[c]
                + (value - params.endpoint_capture_0[c]) * params.endpoint_slope_0[c];
        } else if value <= params.endpoint_capture_1[c] {
            mapped = params.endpoint_output_1[c]
                + (value - params.endpoint_capture_1[c]) * params.endpoint_slope_1[c];
        } else {
            let d = -log(value / params.clear[c]) / params.ln10;
            mapped = tone(invert(d, c), c).x;
        }
        output[base + c] = mapped;
    }
}
