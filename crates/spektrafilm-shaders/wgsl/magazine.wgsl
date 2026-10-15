// Bundled magazine print appearance on the device (pipe architecture S4).
// A faithful port of `magazine_print_color::map_pixel`: CIELAB D50 tone and
// selective chroma finishing with a 13-step gamut-chroma bisection, applied
// to the display-referred portion of the signal while out-of-range residual
// survives. Tone knots/slopes and the Lab transform matrices arrive as
// uniforms so `magazine_print_color.rs` stays the single source of truth.
//
// Input state: the buffer after the output-transfer tail — encoded exactly
// when `encoded` is set. In that mode the appearance decodes the transfer
// state for its own domain and restores it afterwards; in linear mode it
// processes the samples directly.
//
// Requires the shared CCTF functions from cctf_lib.wgsl prepended at the
// call site (source concatenation).

struct Params {
    n_pixels: u32,
    cctf_kind: u32,
    encoded: u32,
    _pad0: u32,
    strength_pad: vec4<f32>,  // x = strength
    // Destination RGB ↔ CIELAB D50, column-major as three vec4 columns.
    to_col0: vec4<f32>,
    to_col1: vec4<f32>,
    to_col2: vec4<f32>,
    from_col0: vec4<f32>,
    from_col1: vec4<f32>,
    from_col2: vec4<f32>,
    // Tone curve: 10 input knots, 10 output knots, 10 Hermite slopes.
    tin0: vec4<f32>, tin1: vec4<f32>, tin2: vec4<f32>,
    tout0: vec4<f32>, tout1: vec4<f32>, tout2: vec4<f32>,
    slo0: vec4<f32>, slo1: vec4<f32>, slo2: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> data: array<f32>;

// CIELAB piecewise breakpoint and slope constant (6/29, 29/3 cubed).
const LAB_EPSILON: f32 = 216.0 / 24389.0;
const LAB_KAPPA: f32 = 24389.0 / 27.0;
const D50: vec3<f32> = vec3<f32>(0.96422, 1.0, 0.82521);
const GAMUT_STEPS: u32 = 13u;

fn tone_in(i: u32) -> f32 {
    let a = array<f32, 10>(
        params.tin0.x, params.tin0.y, params.tin0.z, params.tin0.w,
        params.tin1.x, params.tin1.y, params.tin1.z, params.tin1.w,
        params.tin2.x, params.tin2.y,
    );
    return a[i];
}

fn tone_out(i: u32) -> f32 {
    let a = array<f32, 10>(
        params.tout0.x, params.tout0.y, params.tout0.z, params.tout0.w,
        params.tout1.x, params.tout1.y, params.tout1.z, params.tout1.w,
        params.tout2.x, params.tout2.y,
    );
    return a[i];
}

fn tone_slope(i: u32) -> f32 {
    let a = array<f32, 10>(
        params.slo0.x, params.slo0.y, params.slo0.z, params.slo0.w,
        params.slo1.x, params.slo1.y, params.slo1.z, params.slo1.w,
        params.slo2.x, params.slo2.y,
    );
    return a[i];
}

// Monotone cubic Hermite curve: the last knot at or below l picks the
// segment, matching `TONE_IN.partition_point(..) - 1`.
fn tone_eval(l: f32) -> f32 {
    // NaN (`l != l`) or ±inf guard, matching the reference's
    // `!l.is_finite()` branch.
    if l != l || abs(l) > 1.0e30 {
        return tone_out(0u);
    }
    if l <= tone_in(0u) {
        return tone_out(0u);
    }
    if l >= tone_in(9u) {
        return tone_out(9u);
    }
    var i = 0u;
    loop {
        if i >= 8u { break; }
        if l < tone_in(i + 1u) { break; }
        i = i + 1u;
    }
    let h = tone_in(i + 1u) - tone_in(i);
    let t = (l - tone_in(i)) / h;
    let t2 = t * t;
    let t3 = t2 * t;
    return (2.0 * t3 - 3.0 * t2 + 1.0) * tone_out(i)
        + (t3 - 2.0 * t2 + t) * h * tone_slope(i)
        + (-2.0 * t3 + 3.0 * t2) * tone_out(i + 1u)
        + (t3 - t2) * h * tone_slope(i + 1u);
}

// Gaussian weight on the shortest signed hue distance.
fn hue_weight(hue: f32, center_deg: f32, width_deg: f32) -> f32 {
    let center = radians(center_deg);
    let width = radians(width_deg);
    let d = atan2(sin(hue - center), cos(hue - center));
    return exp(-0.5 * (d / width) * (d / width));
}

fn xyz_to_lab(xyz: vec3<f32>) -> vec3<f32> {
    var f = vec3<f32>(0.0);
    for (var c: i32 = 0; c < 3; c = c + 1) {
        let t = xyz[c] / D50[c];
        if t > LAB_EPSILON {
            f[c] = pow(t, 1.0 / 3.0);
        } else {
            f[c] = (LAB_KAPPA * t + 16.0) / 116.0;
        }
    }
    return vec3<f32>(116.0 * f.y - 16.0, 500.0 * (f.x - f.y), 200.0 * (f.y - f.z));
}

fn ssmooth(x: f32) -> f32 {
    let t = clamp(x, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}
fn lab_to_xyz(lab: vec3<f32>) -> vec3<f32> {
    let fy = (lab.x + 16.0) / 116.0;
    let fx = fy + lab.y / 500.0;
    let fz = fy - lab.z / 200.0;
    var xyz = vec3<f32>(0.0);
    let fs = array<f32, 3>(fx, fy, fz);
    for (var c: i32 = 0; c < 3; c = c + 1) {
        let f = fs[c];
        if f * f * f > LAB_EPSILON {
            xyz[c] = f * f * f;
        } else {
            xyz[c] = (116.0 * f - 16.0) / LAB_KAPPA;
        }
    }
    return xyz * D50;
}

fn appearance(lab: vec3<f32>) -> vec3<f32> {
    let l = lab.x;
    let a = lab.y;
    let b = lab.z;
    let chroma = length(vec2<f32>(a, b));
    let hue = atan2(b, a);
    let chroma_weight = ssmooth((chroma - 20.0) / 28.0);
    let mid_weight = ssmooth((l - 8.0) / 15.0) * (1.0 - ssmooth((l - 78.0) / 18.0));
    let red = hue_weight(hue, 35.0, 25.0) * chroma_weight * mid_weight;
    let blue = hue_weight(hue, -65.0, 29.0) * chroma_weight * mid_weight;
    let green = hue_weight(hue, 140.0, 30.0) * chroma_weight * mid_weight;
    let mapped_l = tone_eval(l) - 4.0 * red - 3.5 * blue;
    let chroma_scale = 1.0 + 0.22 * red + 0.10 * blue - 0.04 * green;
    let highlight = ssmooth((l - 75.0) / 25.0) * (1.0 - ssmooth(chroma / 18.0));
    return vec3<f32>(
        mapped_l,
        a * chroma_scale + 0.12 * highlight,
        b * chroma_scale + 0.7 * highlight,
    );
}

fn to_rgb(lab: vec3<f32>) -> vec3<f32> {
    let m = mat3x3<f32>(params.from_col0.xyz, params.from_col1.xyz, params.from_col2.xyz);
    return m * lab_to_xyz(lab);
}

fn in_gamut(rgb: vec3<f32>) -> bool {
    return all(rgb >= vec3<f32>(0.0)) && all(rgb <= vec3<f32>(1.0));
}

// Largest chroma scale at fixed mapped lightness and hue that fits the
// destination gamut (13 bisection steps, matching the reference).
fn gamut_chroma_scale(lab: vec3<f32>) -> f32 {
    if in_gamut(to_rgb(lab)) {
        return 1.0;
    }
    var low = 0.0;
    var high = 1.0;
    for (var s: u32 = 0u; s < GAMUT_STEPS; s = s + 1u) {
        let mid = 0.5 * (low + high);
        if in_gamut(to_rgb(vec3<f32>(lab.x, lab.y * mid, lab.z * mid))) {
            low = mid;
        } else {
            high = mid;
        }
    }
    return low;
}

// Headroom decode/encode: pure-power spaces mirror the sign for negative
// residual; piecewise spaces pass the value through like the reference.
fn decode_headroom(v: f32) -> f32 {
    if v < 0.0 && (params.cctf_kind == 5u || params.cctf_kind == 6u) {
        return -decode_one(-v, params.cctf_kind);
    }
    return decode_one(v, params.cctf_kind);
}

fn encode_headroom(v: f32) -> f32 {
    if v < 0.0 && (params.cctf_kind == 5u || params.cctf_kind == 6u) {
        return -encode_one(-v, params.cctf_kind);
    }
    return encode_one(v, params.cctf_kind);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels {
        return;
    }
    let base = idx * 3u;
    var linear = vec3<f32>(data[base], data[base + 1u], data[base + 2u]);
    if params.encoded == 1u {
        linear = vec3<f32>(
            decode_headroom(linear.x),
            decode_headroom(linear.y),
            decode_headroom(linear.z),
        );
    }
    let base_rgb = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let residual = linear - base_rgb;
    let to_m = mat3x3<f32>(params.to_col0.xyz, params.to_col1.xyz, params.to_col2.xyz);
    let mapped = appearance(xyz_to_lab(to_m * base_rgb));
    let scale = gamut_chroma_scale(mapped);
    let mapped_rgb = to_rgb(vec3<f32>(mapped.x, mapped.y * scale, mapped.z * scale));
    var out = base_rgb + params.strength_pad.x * (mapped_rgb - base_rgb) + residual;
    if params.encoded == 1u {
        out = vec3<f32>(
            max(encode_headroom(out.x), 0.0),
            max(encode_headroom(out.y), 0.0),
            max(encode_headroom(out.z), 0.0),
        );
    }
    data[base] = out.x;
    data[base + 1u] = out.y;
    data[base + 2u] = out.z;
}
