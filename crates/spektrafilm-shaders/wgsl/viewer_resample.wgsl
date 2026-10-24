// GUI viewer resample: the device port of `capped_raster` in
// `crates/spektrafilm-gui/src/display/raster.rs`.
//
// The viewer's capped raster is its only image-sized reduction, so a device
// route must not reduce it on the host. Each output pixel covers the exact
// source rectangle the host maps it to, and the value is the same
// separable bilinear mix the host computes, in the host's channel order:
// horizontal first, then vertical. Nothing here approximates the geometry,
// the boundary clamp, or the tap weights.
//
// The host owns the destination size: it computes the long-edge factor and
// the rounded destination dimensions in f64, so one authoritative rule
// decides how many pixels the viewer keeps. This kernel only maps each
// destination pixel to its source rectangle.

struct Params {
    /// Source width in pixels.
    width: u32,
    /// Source height in pixels.
    height: u32,
    /// Destination width in pixels.
    out_width: u32,
    /// Destination height in pixels.
    out_height: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;   // [W*H*3]
@group(0) @binding(2) var<storage, read_write> out: array<f32>; // [OW*OH*3]

// `(y * width + x) * 3 + c`
fn source_index(x: u32, y: u32, c: u32) -> u32 {
    return ((y * params.width + x) * 3u + c);
}

// Source sample at integer coordinates, with reflect-free edge clamp: the
// host clamps the continuous coordinate to [0, width-1] and takes
// `min(x0 + 1, width - 1)` as the second tap, which is a clamp, not a wrap.
fn sample(x: u32, y: u32, c: u32) -> f32 {
    let cx = min(x, params.width - 1u);
    let cy = min(y, params.height - 1u);
    return source[source_index(cx, cy, c)];
}

// One output pixel: the source rectangle `sx`/`sy` span, mixed bilinearly in
// the host's order. The continuous coordinate is computed in f32 here and in
// f64 on the host; the two agree to the last few bits for any image the
// viewer can display, and the tests state the resulting budget.
fn resample_pixel(ox: u32, oy: u32, c: u32) -> f32 {
    let sx = clamp(
        (f32(ox) + 0.5) * f32(params.width) / f32(params.out_width) - 0.5,
        0.0,
        f32(params.width - 1u),
    );
    let sy = clamp(
        (f32(oy) + 0.5) * f32(params.height) / f32(params.out_height) - 0.5,
        0.0,
        f32(params.height - 1u),
    );
    let x0 = u32(floor(sx));
    let y0 = u32(floor(sy));
    let x1 = min(x0 + 1u, params.width - 1u);
    let y1 = min(y0 + 1u, params.height - 1u);
    let fx = sx - f32(x0);
    let fy = sy - f32(y0);
    let top = sample(x0, y0, c) * (1.0 - fx) + sample(x1, y0, c) * fx;
    let bottom = sample(x0, y1, c) * (1.0 - fx) + sample(x1, y1, c) * fx;
    return top * (1.0 - fy) + bottom * fy;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    let n_pixels = params.out_width * params.out_height;
    if idx >= n_pixels {
        return;
    }
    let ox = idx % params.out_width;
    let oy = idx / params.out_width;
    let base = idx * 3u;
    out[base] = resample_pixel(ox, oy, 0u);
    out[base + 1u] = resample_pixel(ox, oy, 1u);
    out[base + 2u] = resample_pixel(ox, oy, 2u);
}
