// Output transfer encoding/decoding for the pipe tail (pipe architecture
// S4). Matches `spektrafilm_math::colorspace::{cctf_encode, cctf_decode}`
// per channel; encode additionally applies the stored same-space RGB
// roundtrip matrix (`rgb_to_rgb_identity`, the rounded-inverse product the
// CPU reference multiplies through), decode is per-channel only.
//
// mode 0 = encode (linear native → encoded, matrix + cctf)
// mode 1 = decode (encoded → linear, cctf only)
// cctf_kind: 0 Linear, 1 ACEScct, 2 sRGB, 3 ProPhoto, 4 Rec2020,
//            5 Adobe RGB (1998), 6 Gamma 2.6
//
// Negative handling mirrors the reference exactly: the piecewise spaces use
// sign-preserving powers; the pure-power spaces produce NaN for negative
// inputs, exactly like the CPU `powf`.

struct Params {
    n_pixels: u32,
    mode: u32,
    cctf_kind: u32,
    _pad: u32,
    // Same-space roundtrip matrix, column-major as three vec4 columns.
    col0: vec4<f32>,
    col1: vec4<f32>,
    col2: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> data: array<f32>;

fn spow(x: f32, p: f32) -> f32 {
    return sign(x) * pow(abs(x), p);
}

fn encode_one(v: f32, kind: u32) -> f32 {
    if kind == 0u { return v; }
    if kind == 1u {
        if v <= 0.0078125 {
            return 10.5402377416545 * v + 0.0729055341958355;
        }
        return (log2(v) + 9.72) / 17.52;
    }
    if kind == 2u {
        if v <= 0.0031308 {
            return 12.92 * v;
        }
        return 1.055 * spow(v, 1.0 / 2.4) - 0.055;
    }
    if kind == 3u {
        if 1.0 / 512.0 > v {
            return v * 16.0;
        }
        return spow(v, 1.0 / 1.8);
    }
    if kind == 4u {
        let a = 1.099;
        let b = 0.018;
        if v < b {
            return v * 4.5;
        }
        return a * spow(v, 0.45) - (a - 1.0);
    }
    if kind == 5u {
        return pow(v, 1.0 / (563.0 / 256.0));
    }
    return pow(v, 1.0 / 2.6);
}

fn decode_one(v: f32, kind: u32) -> f32 {
    if kind == 0u { return v; }
    if kind == 1u {
        if v > 0.155251141552511 {
            return exp2(v * 17.52 - 9.72);
        }
        return (v - 0.0729055341958355) / 10.5402377416545;
    }
    if kind == 2u {
        if 12.92 * 0.0031308 >= v {
            return v / 12.92;
        }
        return spow((v + 0.055) / 1.055, 2.4);
    }
    if kind == 3u {
        if v < 16.0 * (1.0 / 512.0) {
            return v / 16.0;
        }
        return spow(v, 1.8);
    }
    if kind == 4u {
        let a = 1.099;
        let b = 0.018;
        if v < a * pow(b, 0.45) - (a - 1.0) {
            return v / 4.5;
        }
        return spow((v + (a - 1.0)) / a, 1.0 / 0.45);
    }
    if kind == 5u {
        return pow(v, 563.0 / 256.0);
    }
    return pow(v, 2.6);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels {
        return;
    }
    let base = idx * 3u;
    if params.mode == 0u {
        let m = mat3x3<f32>(params.col0.xyz, params.col1.xyz, params.col2.xyz);
        let rgb = vec3<f32>(data[base], data[base + 1u], data[base + 2u]);
        let native = m * rgb;
        data[base] = encode_one(native.x, params.cctf_kind);
        data[base + 1u] = encode_one(native.y, params.cctf_kind);
        data[base + 2u] = encode_one(native.z, params.cctf_kind);
    } else {
        data[base] = decode_one(data[base], params.cctf_kind);
        data[base + 1u] = decode_one(data[base + 1u], params.cctf_kind);
        data[base + 2u] = decode_one(data[base + 2u], params.cctf_kind);
    }
}
