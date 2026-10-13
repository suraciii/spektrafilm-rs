// Shared per-channel CCTF encode/decode, matching
// `spektrafilm_math::colorspace::{cctf_encode, cctf_decode}`.
//
// cctf_kind: 0 Linear, 1 ACEScct, 2 sRGB, 3 ProPhoto, 4 Rec2020,
//            5 Adobe RGB (1998), 6 Gamma 2.6
//
// Negative handling mirrors the reference exactly: the piecewise spaces use
// sign-preserving powers; the pure-power spaces produce NaN for negative
// inputs, exactly like the CPU `powf`.
//
// Consumed via source concatenation (WGSL has no include mechanism):
// concat!(include_str!(cctf_lib.wgsl), include_str!(<node>.wgsl)).

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
