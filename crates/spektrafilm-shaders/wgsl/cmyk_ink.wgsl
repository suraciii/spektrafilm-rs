// CMYK ink style (cmyk_ink_v1) on the device (pipe architecture S4).
// A faithful port of `cmyk::map_pixel`: the fixed Lab grade
// L' = max(4, 50 + (L - 50) * 1.03 - 0.28 * C * s), a' = 1.40 a,
// b' = 1.40 b, with s = smoothstep((L - 8) / 37). No tone curve, no hue
// masks, no gamut bisection, and no output nonnegative clamp — signed
// headroom survives to the image writer. The Lab transform matrices
// arrive as uniforms so `cmyk.rs` stays the single source of truth.
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
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> data: array<f32>;

// CIELAB piecewise breakpoint and slope constant (6/29, 29/3 cubed).
const LAB_EPSILON: f32 = 216.0 / 24389.0;
const LAB_KAPPA: f32 = 24389.0 / 27.0;
const D50: vec3<f32> = vec3<f32>(0.96422, 1.0, 0.82521);

// cmyk_ink_v1 grade constants (specs/cmyk-style).
const K_L: f32 = 0.28;
const K_C: f32 = 1.40;
const CONTRAST: f32 = 1.03;
const SMOOTHSTEP_LOW: f32 = 8.0;
const SMOOTHSTEP_HIGH: f32 = 45.0;
const L_FLOOR: f32 = 4.0;

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

fn ssmooth(x: f32) -> f32 {
    let t = clamp(x, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Full-strength cmyk_ink_v1 appearance in CIELAB D50.
fn appearance(lab: vec3<f32>) -> vec3<f32> {
    let l = lab.x;
    let a = lab.y;
    let b = lab.z;
    let chroma = length(vec2<f32>(a, b));
    let s = ssmooth((l - SMOOTHSTEP_LOW) / (SMOOTHSTEP_HIGH - SMOOTHSTEP_LOW));
    let mapped_l = max(L_FLOOR, 50.0 + (l - 50.0) * CONTRAST - K_L * chroma * s);
    return vec3<f32>(mapped_l, K_C * a, K_C * b);
}

fn to_rgb(lab: vec3<f32>) -> vec3<f32> {
    let m = mat3x3<f32>(params.from_col0.xyz, params.from_col1.xyz, params.from_col2.xyz);
    return m * lab_to_xyz(lab);
}

// Headroom decode/encode: every space uses sign-preserving powers, so
// finite signed headroom stays finite and no output clamp is applied.
fn decode_headroom(v: f32) -> f32 {
    return decode_one(v, params.cctf_kind);
}

fn encode_headroom(v: f32) -> f32 {
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
    let to_m = mat3x3<f32>(params.to_col0.xyz, params.to_col1.xyz, params.to_col2.xyz);
    let mapped_rgb = to_rgb(appearance(xyz_to_lab(to_m * linear)));
    var out = linear + params.strength_pad.x * (mapped_rgb - linear);
    if params.encoded == 1u {
        out = vec3<f32>(
            encode_headroom(out.x),
            encode_headroom(out.y),
            encode_headroom(out.z),
        );
    }
    data[base] = out.x;
    data[base + 1u] = out.y;
    data[base + 2u] = out.z;
}
