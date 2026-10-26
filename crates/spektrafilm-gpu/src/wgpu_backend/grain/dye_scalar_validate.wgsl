// Validate raw device words: floating-point flush-to-zero must not hide an
// invalid lane difference or turn an unrepresentable positive target into zero.
struct Params { pixels: vec4<u32>, }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<u32>;
@group(0) @binding(2) var<storage, read_write> status: atomic<u32>;
fn canonical(bits: u32) -> u32 {
    return select(bits, 0u, (bits & 0x7fffffffu) == 0u);
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let i = gid.x + gid.y * grid.x * 256u;
    if (i >= p.pixels.x) { return; }
    let b = i * 3u;
    for (var lane = 0u; lane < 3u; lane++) {
        let bits = source[b + lane];
        let magnitude = bits & 0x7fffffffu;
        if (magnitude >= 0x7f800000u || ((bits & 0x80000000u) != 0u && magnitude != 0u)
            || canonical(bits) != canonical(source[b])) {
            atomicOr(&status, 32u);
        }
        if (magnitude != 0u && magnitude < 0x00800000u) { atomicOr(&status, 2u); }
    }
}
