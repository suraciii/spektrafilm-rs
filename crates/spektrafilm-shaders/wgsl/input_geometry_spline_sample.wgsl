// Final spline-rescale readout: the reference's 4×4 cubic gather.
//
// `rescale_spline3` resolves one tap set per output index — half-pixel centers,
// mirror extension, the cubic weights evaluated in a fixed order — and then
// accumulates the sixteen samples as `data · yw[a] · xw[b]` in ascending `a`
// then `b`, clamping the result to the input interval. Every step is reproduced
// here, including the clamp bounds measured on the device.

struct Params {
    src_width: u32,
    src_height: u32,
    out_width: u32,
    out_height: u32,
    first_pixel: u32,
    count: u32,
    first_tap: u32,
    taps: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> data: array<f64>;
@group(0) @binding(2) var<storage, read> minimum: array<f32>;
@group(0) @binding(3) var<storage, read> maximum: array<f32>;
@group(0) @binding(4) var<storage, read_write> accumulator: array<f64>;
@group(0) @binding(5) var<storage, read_write> out: array<f32>;


/// Reference `rem_euclid` on the tap coordinate, then the mirror fold.
fn fold(x_in: f64, n: u32) -> f64 {
    if n <= 1u {
        return 0.0lf;
    }
    let period = f64(2u * n - 2u);
    // `f64::rem_euclid`: the truncated remainder (IEEE `fmod`, which is exact)
    // adjusted into `[0, period)`. Written out because the remainder operator
    // is not available for binary64 on every device.
    var x = x_in - period * trunc(x_in / period);
    if x < 0.0lf {
        x = x + abs(period);
    }
    if x > f64(n - 1u) {
        x = period - x;
    }
    return x;
}


@compute @workgroup_size(256)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= p.count { return; }
    let pixel = p.first_pixel + idx;
    let ox = pixel % p.out_width;
    let oy = pixel / p.out_width;

    let x = fold(
        (f64(ox) + 0.5) * (f64(p.src_width) / f64(p.out_width)) - 0.5,
        p.src_width,
    );
    let y = fold(
        (f64(oy) + 0.5) * (f64(p.src_height) / f64(p.out_height)) - 0.5,
        p.src_height,
    );
    let fx = x - floor(x);
    let fy = y - floor(y);
    let gx = 1.0 - fx;
    let gy = 1.0 - fy;
    let wx0 = gx * gx * gx / 6.0;
    let wx1 = (fx * fx * (fx - 2.0) * 3.0 + 4.0) / 6.0;
    let wx2 = (gx * gx * (gx - 2.0) * 3.0 + 4.0) / 6.0;
    let wx3 = 1.0 - wx0 - wx1 - wx2;
    let wy0 = gy * gy * gy / 6.0;
    let wy1 = (fy * fy * (fy - 2.0) * 3.0 + 4.0) / 6.0;
    let wy2 = (gy * gy * (gy - 2.0) * 3.0 + 4.0) / 6.0;
    let wy3 = 1.0 - wy0 - wy1 - wy2;
    let xw = array<f64, 4>(wx0, wx1, wx2, wx3);
    let yw = array<f64, 4>(wy0, wy1, wy2, wy3);
    for (var c = 0u; c < 3u; c = c + 1u) {
        var value: f64 = 0.0lf;
        if p.first_tap != 0u { value = accumulator[idx * 3u + c]; }
        for (var t = 0u; t < p.taps; t = t + 1u) {
            let tap = p.first_tap + t;
            value = value + (data[(t * p.count + idx) * 3u + c] * yw[tap / 4u]) * xw[tap % 4u];
        }
        accumulator[idx * 3u + c] = value;
        if p.first_tap + p.taps != 16u { continue; }
        let lo = f64(minimum[0]);
        let hi = f64(maximum[0]);
        if value < lo {
            value = lo;
        }
        if value > hi {
            value = hi;
        }
        out[idx * 3u + c] = f32(value);
    }
}
