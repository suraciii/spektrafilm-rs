// GUI viewer display-colour transform: the device port of `display_rgb` in
// `crates/spektrafilm-gui/src/display/colortransform.rs`.
//
// The viewer's display transform maps native image samples to encoded sRGB
// viewing pixels: decode the source transfer when the caller says the source
// is encoded, apply the source space's display matrix, encode sRGB, and clamp
// to 0..=1 for the bounded display surface. The result is disposable viewer
// data — it never becomes a retained or saved sample, and the source pixels
// are never modified.
//
// This is not the saving conversion: the destination transfer is always sRGB
// and the matrix is the host's display matrix, exactly as the reference
// `display_rgb` builds it. A device path that substituted a different matrix
// or a different transfer function would change what the viewer shows, so
// none of the three steps is approximated here.

struct Params {
    n_pixels: u32,
    /// Source CCTF discriminant, or a sentinel when the request supplies
    /// linear samples and no decode applies.
    src_cctf: u32,
    decode: u32,
    clamp: u32,
    // Source → encoded sRGB matrix, column-major as three vec4 columns.
    col0: vec4<f32>,
    col1: vec4<f32>,
    col2: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;     // [N*3]
@group(0) @binding(2) var<storage, read_write> out: array<f32>; // [N*3]

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels {
        return;
    }
    let base = idx * 3u;
    var rgb = vec3<f32>(input[base], input[base + 1u], input[base + 2u]);
    if params.decode != 0u {
        rgb = vec3<f32>(
            decode_one(rgb.x, params.src_cctf),
            decode_one(rgb.y, params.src_cctf),
            decode_one(rgb.z, params.src_cctf),
        );
    }
    let m = mat3x3<f32>(params.col0.xyz, params.col1.xyz, params.col2.xyz);
    let encoded = m * rgb;
    var result = vec3<f32>(
        encode_one(encoded.x, 2u),
        encode_one(encoded.y, 2u),
        encode_one(encoded.z, 2u),
    );
    if params.clamp != 0u {
        result = clamp(result, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    out[base] = result.x;
    out[base + 1u] = result.y;
    out[base + 2u] = result.z;
}
