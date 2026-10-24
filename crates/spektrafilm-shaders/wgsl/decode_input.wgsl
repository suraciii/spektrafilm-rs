// Input transfer decoding (pipe architecture: input preparation). Decodes the
// uploaded input samples in place with the input colour space's CCTF, exactly
// like `spektrafilm_math::colorspace::cctf_decode` on the host.
//
// This is the chain-head counterpart of `cctf_transfer.wgsl`: the same
// per-channel transfer function, without the same-space roundtrip matrix that
// belongs to output encoding.
//
// Requires the shared CCTF functions from cctf_lib.wgsl prepended at the
// call site (source concatenation).

struct Params {
    n_pixels: u32,
    cctf_kind: u32,
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
    data[base] = decode_one(data[base], params.cctf_kind);
    data[base + 1u] = decode_one(data[base + 1u], params.cctf_kind);
    data[base + 2u] = decode_one(data[base + 2u], params.cctf_kind);
}
