// Gaussian antialias weights for the spline resize's downscale prefilter.
//
// `rescale_spline3`'s `axis` builds `k[x] = exp(-0.5/s² · x²)` for
// `x ∈ [-radius, radius]` and normalizes the kernel before filtering. The
// weights are shared by every line of an axis, so the device computes them
// once per axis pass into a storage buffer instead of once per line: the
// arithmetic is the reference's own, evaluated with the explicit binary64
// `exp` (`f64_math.wgsl`), because a driver's extended `exp` is unusable on
// some devices.

struct Params {
    /// `-0.5 / (sigma * sigma)`, resolved on the host in binary64.
    c: f64,
    radius: u32,
    count: u32,
    first: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> weights: array<f64>;

@compute @workgroup_size(256)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= p.count {
        return;
    }
    let x = f64(p.first + idx) - f64(p.radius);
    weights[idx] = sf64_exp(p.c * x * x);
}
