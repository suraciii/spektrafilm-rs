// GUI viewer resample: the device port of `capped_raster` in
// `crates/spektrafilm-gui/src/display/raster.rs`.
//
// The viewer's capped raster is its only image-sized reduction, so a device
// route must not reduce it on the host. Each output pixel averages the exact
// overlapping source cells in row-major order, with edge-clamped taps.
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

// Clamp logical cell coordinates only when fetching the source.
fn sample(x: i32, y: i32, c: u32) -> f32 {
    let cx = u32(clamp(x, 0, i32(params.width) - 1));
    let cy = u32(clamp(y, 0, i32(params.height) - 1));
    return source[source_index(cx, cy, c)];
}

fn compensated(sum: f32, correction: f32, value: f32) -> vec2<f32> {
    let next = sum + value;
    if (bitcast<u32>(next) & 0x7f800000u) == 0x7f800000u { return vec2<f32>(next, 0.0); }
    var error: f32;
    if abs(sum) >= abs(value) { error = (sum - next) + value; }
    else { error = (value - next) + sum; }
    return vec2<f32>(next, correction + error);
}

fn resample_pixel(ox: u32, oy: u32, c: u32) -> f32 {
    if params.width == params.out_width && params.height == params.out_height {
        return sample(i32(ox), i32(oy), c);
    }
    let sx = (f32(ox) + 0.5) * f32(params.width) / f32(params.out_width) - 0.5;
    let sy = (f32(oy) + 0.5) * f32(params.height) / f32(params.out_height) - 0.5;
    let fx = max(1.0, f32(params.width) / f32(params.out_width));
    let fy = max(1.0, f32(params.height) / f32(params.out_height));
    let lo_x = (sx + 0.5) - fx / 2.0;
    let lo_y = (sy + 0.5) - fy / 2.0;
    let hi_x = lo_x + fx;
    let hi_y = lo_y + fy;
    var result = 0.0;
    var correction = 0.0;
    // Bound correction accumulation, then compensate the chunk totals too.
    var chunk = vec2<f32>(0.0);
    var chunk_count = 0u;
    var iy = i32(floor(lo_y));
    let end_y = i32(ceil(hi_y));
    let end_x = i32(ceil(hi_x));
    loop {
        if iy >= end_y { break; }
        let wy = max(0.0, min(hi_y, f32(iy + 1)) - max(lo_y, f32(iy))) / fy;
        var ix = i32(floor(lo_x));
        loop {
            if ix >= end_x { break; }
            let wx = max(0.0, min(hi_x, f32(ix + 1)) - max(lo_x, f32(ix))) / fx;
            chunk = compensated(chunk.x, chunk.y, wx * wy * sample(ix, iy, c));
            chunk_count += 1u;
            if chunk_count == 1024u {
                let total = compensated(result, correction, chunk.x + chunk.y);
                result = total.x;
                correction = total.y;
                chunk = vec2<f32>(0.0);
                chunk_count = 0u;
            }
            ix = ix + 1;
        }
        iy = iy + 1;
    }
    if chunk_count != 0u {
        let total = compensated(result, correction, chunk.x + chunk.y);
        result = total.x;
        correction = total.y;
    }
    return result + correction;
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
