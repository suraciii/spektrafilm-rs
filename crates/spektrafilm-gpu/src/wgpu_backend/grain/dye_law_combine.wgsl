// Weighted sum of the octave fields: the multi-scale field before the band limit.

struct P { w: u32, h: u32, pad0: u32, pad1: u32, w0: f32, w1: f32, w2: f32, pad2: f32 };
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> o0: array<f32>;
@group(0) @binding(2) var<storage, read> o1: array<f32>;
@group(0) @binding(3) var<storage, read> o2: array<f32>;
@group(0) @binding(4) var<storage, read_write> dst: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.y * p.w + gid.x;
    if (gid.x >= p.w || gid.y >= p.h) { return; }
    dst[i] = p.w0 * o0[i] + p.w1 * o1[i] + p.w2 * o2[i];
}
