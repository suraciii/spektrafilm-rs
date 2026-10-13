// Per-pixel sRGB transfer function encode/decode.
// mode 0 = encode (linear -> sRGB), mode 1 = decode (sRGB -> linear).
// Matches CPU srgb_encode/srgb_decode exactly.

struct Params {
    width: u32,
    height: u32,
    mode: u32, // 0 = encode, 1 = decode
    _pad: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;   // [H*W*3]
@group(0) @binding(2) var<storage, read_write> output: array<f32>; // [H*W*3]

fn srgb_encode(x: f32) -> f32 {
    if x <= 0.0031308 {
        return 12.92 * x;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

fn srgb_decode(x: f32) -> f32 {
    if x <= 0.04045 {
        return x / 12.92;
    }
    return pow((x + 0.055) / 1.055, 2.4);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel_idx = gid.x + gid.y * grid.x * 256u;
    let total_pixels = params.width * params.height;
    if pixel_idx >= total_pixels {
        return;
    }

    let base = pixel_idx * 3u;
    for (var c = 0u; c < 3u; c++) {
        let x = input[base + c];
        if params.mode == 0u {
            output[base + c] = srgb_encode(x);
        } else {
            output[base + c] = srgb_decode(x);
        }
    }
}
