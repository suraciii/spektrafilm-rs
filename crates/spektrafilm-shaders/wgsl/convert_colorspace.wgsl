// Input-route colour management — GPU port of the input-only route's terminal
// conversion in `spektrafilm-core/src/pipeline/execution.rs`.
//
// The input route runs no film stage, so its whole numerical content is this
// conversion: decode the source transfer, apply the source→destination
// matrix, encode the destination transfer. Doing it here keeps the route's
// only image arithmetic on the device.
//
// Requires the shared CCTF functions from cctf_lib.wgsl prepended at the
// call site (source concatenation).

struct Params {
    n_pixels: u32,
    /// Source CCTF discriminant, or a sentinel when the request supplies
    /// linear samples and no decode applies.
    src_cctf: u32,
    decode: u32,
    dst_cctf: u32,
    encode: u32,
    // Source→destination matrix, column-major as three vec4 columns.
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
    var rgb = vec3<f32>(data[base], data[base + 1u], data[base + 2u]);
    if params.decode != 0u {
        rgb = vec3<f32>(
            decode_one(rgb.x, params.src_cctf),
            decode_one(rgb.y, params.src_cctf),
            decode_one(rgb.z, params.src_cctf),
        );
    }
    let m = mat3x3<f32>(params.col0.xyz, params.col1.xyz, params.col2.xyz);
    let converted = m * rgb;
    if params.encode != 0u {
        data[base] = encode_one(converted.x, params.dst_cctf);
        data[base + 1u] = encode_one(converted.y, params.dst_cctf);
        data[base + 2u] = encode_one(converted.z, params.dst_cctf);
    } else {
        data[base] = converted.x;
        data[base + 1u] = converted.y;
        data[base + 2u] = converted.z;
    }
}
