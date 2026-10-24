// Output mix — GPU port of the final blend in
// `spektrafilm-core/src/pipeline/output_mix.rs` (`compose_output_mix`).
//
// The processed result carries the output transfer when the caller requested
// encoding; the prepared source is always linear output primaries. The blend
// decodes the result, lerps against the linear source, and re-encodes — the
// exact reference order, so the device result matches the host sample for
// sample.
//
// Requires the shared CCTF functions from cctf_lib.wgsl prepended at the
// call site (source concatenation).

struct Params {
    n_pixels: u32,
    /// 1 when the operands carry encoded samples, 0 when linear.
    encoded: u32,
    cctf_kind: u32,
    amount: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> result: array<f32>;
@group(0) @binding(2) var<storage, read> source: array<f32>;

fn blend_one(r: f32, s: f32) -> f32 {
    // The prepared source is linear output primaries; only the processed
    // result carries the output transfer, so only it is decoded. The
    // reference blend is `(1 - amount) * source + amount * decode(result)`
    // re-encoded, and it never touches the source's transfer.
    if params.encoded == 0u {
        return (1.0 - params.amount) * s + params.amount * r;
    }
    let rl = decode_one(r, params.cctf_kind);
    return encode_one((1.0 - params.amount) * s + params.amount * rl, params.cctf_kind);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels {
        return;
    }
    let base = idx * 3u;
    result[base] = blend_one(result[base], source[base]);
    result[base + 1u] = blend_one(result[base + 1u], source[base + 1u]);
    result[base + 2u] = blend_one(result[base + 2u], source[base + 2u]);
}
