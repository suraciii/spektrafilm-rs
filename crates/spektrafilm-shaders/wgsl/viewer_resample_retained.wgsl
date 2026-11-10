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
    origin_x:u32, origin_y:u32, window_width:u32, first:u32, count:u32,
    tile_lo_x: i32, tile_hi_x: i32, tile_y: i32, tile_rows: u32, accumulate: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;   // [W*H*3]
@group(0) @binding(2) var<storage, read_write> out: array<f32>; // [OW*OH*3]

// `(y * width + x) * 3 + c`
fn source_index(x: u32, y: u32, c: u32) -> u32 {
    return (((y - params.origin_y) * params.window_width + x - params.origin_x) * 3u + c);
}

// Clamp logical cell coordinates only when fetching from the retained window.
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

// Tiles partition logical taps in row-major order, including edge repeats.
fn resample_pixel(ox: u32, oy: u32, c: u32, initial: vec2<f32>) -> vec2<f32> {
    if params.tile_rows == 0u && params.width == params.out_width && params.height == params.out_height {
        return vec2<f32>(sample(i32(ox), i32(oy), c), 0.0);
    }
    let sx = (f32(ox) + 0.5) * f32(params.width) / f32(params.out_width) - 0.5;
    let sy = (f32(oy) + 0.5) * f32(params.height) / f32(params.out_height) - 0.5;
    let fx = max(1.0, f32(params.width) / f32(params.out_width));
    let fy = max(1.0, f32(params.height) / f32(params.out_height));
    let lo_x = (sx + 0.5) - fx / 2.0;
    let lo_y = (sy + 0.5) - fy / 2.0;
    let hi_x = lo_x + fx;
    let hi_y = lo_y + fy;
    var start_x = i32(floor(lo_x));
    var end_x = i32(ceil(hi_x));
    var iy = i32(floor(lo_y));
    var end_y = i32(ceil(hi_y));
    if params.tile_rows != 0u {
        start_x = max(start_x, params.tile_lo_x);
        end_x = min(end_x, params.tile_hi_x);
        iy = max(iy, params.tile_y);
        end_y = min(end_y, params.tile_y + i32(params.tile_rows));
    }
    var result = initial;
    // Chunk corrections stay local; compensated totals persist across tiles.
    var chunk = vec2<f32>(0.0);
    var chunk_count = 0u;
    loop {
        if iy >= end_y { break; }
        let wy = max(0.0, min(hi_y, f32(iy + 1)) - max(lo_y, f32(iy))) / fy;
        var ix = start_x;
        loop {
            if ix >= end_x { break; }
            let wx = max(0.0, min(hi_x, f32(ix + 1)) - max(lo_x, f32(ix))) / fx;
            chunk = compensated(chunk.x, chunk.y, wx * wy * sample(ix, iy, c));
            chunk_count += 1u;
            if chunk_count == 1024u {
                result = compensated(result.x, result.y, chunk.x + chunk.y);
                chunk = vec2<f32>(0.0);
                chunk_count = 0u;
            }
            ix = ix + 1;
        }
        iy = iy + 1;
    }
    if chunk_count != 0u {
        result = compensated(result.x, result.y, chunk.x + chunk.y);
    }
    return result;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    let n_pixels = params.count;
    if idx >= n_pixels {
        return;
    }
    let ox = (idx + params.first) % params.out_width;
    let oy = (idx + params.first) / params.out_width;
    let base = idx * 3u;
    for (var c = 0u; c < 3u; c++) {
        var initial = vec2<f32>(0.0);
        if params.tile_rows != 0u && (params.accumulate & 1u) != 0u {
            initial = vec2<f32>(out[base + c], out[base + 3u + c]);
        }
        let total = resample_pixel(ox, oy, c, initial);
        if params.tile_rows == 0u || (params.accumulate & 2u) != 0u {
            out[base + c] = total.x + total.y;
        } else {
            out[base + c] = total.x;
            out[base + 3u + c] = total.y;
        }
    }
}
