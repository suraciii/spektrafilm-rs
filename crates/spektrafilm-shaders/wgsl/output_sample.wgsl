// Saving sample conversion — the writer's independent color and sample
// conversion (`specs/gpu-execution/spec.md`: saving primaries and transfer,
// output clipping, and requested bit-depth quantization are device sample
// conversion).
//
// The device evaluates the same order the host reference uses: decode the
// source transfer when requested, apply the source→destination primaries
// matrix, encode the destination transfer when requested, then convert to the
// writer's sample type. Integer formats clip to [0,1] and truncate after
// scaling; binary16 output rounds ties-to-even directly from the device value
// and preserves zeros, subnormals, infinities and NaN payloads; binary32
// output keeps unclipped headroom.
//
// Output words pack several samples so the terminal readback is the writer's
// payload, not a second conversion pass:
//   U8    4 samples per word (little-endian byte order within the word)
//   U16   2 samples per word
//   Half  2 samples per word
//   F32   1 sample per word (bit pattern in the word)
//
// Requires the shared CCTF functions from cctf_lib.wgsl prepended at the call
// site (source concatenation).

struct Params {
    /// Total RGB samples: `width * height * 3`.
    n_samples: u32,
    src_cctf: u32,
    decode: u32,
    dst_cctf: u32,
    encode: u32,
    /// 0 U8, 1 U16, 2 Half, 3 F32.
    kind: u32,
    /// Samples packed into one output word.
    lanes: u32,
    n_words: u32,
    // Source→destination matrix, column-major as three vec4 columns.
    col0: vec4<f32>,
    col1: vec4<f32>,
    col2: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> samples: array<f32>;
@group(0) @binding(2) var<storage, read_write> words: array<u32>;

// Direct binary32 → binary16 rounding with ties-to-even, matching numpy's
// `astype(float16)` and the writer's native reference conversion. NaN keeps
// its sign and quiet payload, and both infinities and subnormals survive.
fn f32_to_half_bits(value: f32) -> u32 {
    let bits = bitcast<u32>(value);
    let sign = (bits >> 16u) & 0x8000u;
    let exponent_bits = (bits >> 23u) & 0xffu;
    let fraction = bits & 0x7fffffu;
    if exponent_bits == 0xffu {
        if fraction == 0u {
            return sign | 0x7c00u;
        }
        return sign | 0x7c00u | ((fraction >> 13u) | 0x0200u);
    }
    let exponent = i32(exponent_bits) - 127;
    if exponent < -25 {
        return sign;
    }
    if exponent > 15 {
        return sign | 0x7c00u;
    }
    let significand = fraction | 0x800000u;
    var shift = 13u;
    if exponent < -14 {
        shift = u32(-1 - exponent);
    }
    let rounded = significand >> shift;
    let remainder = significand & ((1u << shift) - 1u);
    let midpoint = 1u << (shift - 1u);
    var magnitude_bits = rounded;
    if remainder > midpoint || (remainder == midpoint && (rounded & 1u) == 1u) {
        magnitude_bits = rounded + 1u;
    }
    var magnitude: u32;
    if exponent < -14 {
        magnitude = magnitude_bits;
    } else {
        magnitude = (u32(exponent + 15) << 10u) + magnitude_bits - 1024u;
    }
    return sign | magnitude;
}

fn quantize(value: f32, scale: f32) -> u32 {
    if value != value {
        return 0u;
    }
    return u32(clamp(value, 0.0, 1.0) * scale);
}

// The pinned saving no-op: same space and transfer state. The reference clones
// the pixels there instead of re-running the colour transform (~2e-5 of sRGB
// round-trip drift, and NaN must not spread across channels through a matrix
// multiply). The device reproduces the clone exactly and only clips/packs.
fn pass_through() -> bool {
    if params.decode != 0u || params.encode != 0u {
        return false;
    }
    return params.col0.x == 1.0 && params.col0.y == 0.0 && params.col0.z == 0.0
        && params.col1.x == 0.0 && params.col1.y == 1.0 && params.col1.z == 0.0
        && params.col2.x == 0.0 && params.col2.y == 0.0 && params.col2.z == 1.0;
}

fn converted_sample(sample: u32) -> f32 {
    if pass_through() {
        return samples[sample];
    }
    let pixel = sample / 3u;
    let channel = sample % 3u;
    let base = pixel * 3u;
    var rgb = vec3<f32>(samples[base], samples[base + 1u], samples[base + 2u]);
    if params.decode != 0u {
        rgb = vec3<f32>(
            decode_one(rgb.x, params.src_cctf),
            decode_one(rgb.y, params.src_cctf),
            decode_one(rgb.z, params.src_cctf),
        );
    }
    let m = mat3x3<f32>(params.col0.xyz, params.col1.xyz, params.col2.xyz);
    let converted = m * rgb;
    var value = converted.x;
    if channel == 1u {
        value = converted.y;
    } else if channel == 2u {
        value = converted.z;
    }
    if params.encode != 0u {
        value = encode_one(value, params.dst_cctf);
    }
    return value;
}

fn sample_word(value: f32) -> u32 {
    if params.kind == 0u {
        return quantize(value, 255.0);
    }
    if params.kind == 1u {
        return quantize(value, 65535.0);
    }
    if params.kind == 2u {
        return f32_to_half_bits(value);
    }
    return bitcast<u32>(value);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let word = gid.x + gid.y * grid.x * 256u;
    if word >= params.n_words {
        return;
    }
    let first = word * params.lanes;
    var packed = 0u;
    for (var lane = 0u; lane < params.lanes; lane = lane + 1u) {
        let sample = first + lane;
        if sample >= params.n_samples {
            continue;
        }
        let bits = sample_word(converted_sample(sample));
        if params.kind == 3u {
            packed = bits;
        } else if params.kind == 0u {
            packed = packed | (bits << (8u * lane));
        } else {
            packed = packed | (bits << (16u * lane));
        }
    }
    words[word] = packed;
}
