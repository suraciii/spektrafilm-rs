// Band limit and normalisation: field = (combined - blurred) * scale.
// The scale is the exact reciprocal of the effective kernel's L2 norm, computed on
// the host, so no global reduction is needed on the device.

struct P { w: u32, h: u32, pad0: u32, pad1: u32, scale: f32, pad2: f32, pad3: f32, pad4: f32 };
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> combined: array<f32>;
@group(0) @binding(2) var<storage, read> blurred: array<f32>;
@group(0) @binding(3) var<storage, read_write> dst: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.y * p.w + gid.x;
    if (gid.x >= p.w || gid.y >= p.h) { return; }
    dst[i] = (combined[i] - blurred[i]) * p.scale;
}
