// DIR coupler final subtract with the reversal receiver Langmuir.
//
// CPU equivalent: in `compute_exposure_correction`, after the spatial
// diffusion blend of the correction:
//   if receiver_k: correction = langmuir(correction, receiver_k, receiver_ref)
//   result = log_raw - correction
//
// The receiver responds to the inhibitor *arriving after* diffusion, so
// the saturation lives here rather than in the matmul pass. An all-zero
// mask (negative film) reduces to a plain `dst -= src`; a channel with a
// non-finite knee stays linear (CPU `k.is_infinite()` short-circuit) and
// its sanitized `receiver_k` is never read.

struct Params {
    n_pixels: u32,
    source_base: u32,
    destination_base: u32,
    _pad2: u32,
    // Amplitude-matched receiver knee, reference and activity per
    // channel (`.w` padding).
    receiver_k: vec4<f32>,
    receiver_ref: vec4<f32>,
    receiver_mask: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<f32>;
@group(0) @binding(2) var<storage, read_write> dst: array<f32>;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels { return; }
    let base = idx * 3u;

    var c: vec3<f32> = vec3<f32>(src[params.source_base + base], src[params.source_base + base + 1u], src[params.source_base + base + 2u]);
    // g(C) = C (Kr + C_ref) / (Kr + C) — amplitude-matched at C_ref.
    let k = params.receiver_k.xyz;
    let r = params.receiver_ref.xyz;
    let saturated = c * (k + r) / (k + c);
    let mask = params.receiver_mask.xyz;
    c = vec3<f32>(
        select(c.x, saturated.x, mask.x > 0.5),
        select(c.y, saturated.y, mask.y > 0.5),
        select(c.z, saturated.z, mask.z > 0.5),
    );

    dst[params.destination_base + base] = dst[params.destination_base + base] - c.x;
    dst[params.destination_base + base + 1u] = dst[params.destination_base + base + 1u] - c.y;
    dst[params.destination_base + base + 2u] = dst[params.destination_base + base + 2u] - c.z;
}
