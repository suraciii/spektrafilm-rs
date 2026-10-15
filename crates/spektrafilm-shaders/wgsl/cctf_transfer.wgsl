// Output transfer encoding/decoding for the pipe tail (pipe architecture
// S4). Matches `spektrafilm_math::colorspace::{cctf_encode, cctf_decode}`
// per channel; encode additionally applies the stored same-space RGB
// roundtrip matrix (`rgb_to_rgb_identity`, the rounded-inverse product the
// CPU reference multiplies through), decode is per-channel only.
//
// mode 0 = encode (linear native → encoded, matrix + cctf)
// mode 1 = decode (encoded → linear, cctf only)
//
// Requires the shared CCTF functions from cctf_lib.wgsl prepended at the
// call site (source concatenation).

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
