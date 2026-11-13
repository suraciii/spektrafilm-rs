// GUI viewer full-source composition: the device port of
// `DisplaySource::full_source_view` in
// `crates/spektrafilm-gui/src/display/worker.rs`.
//
// The view samples the full-resolution source at every visible
// viewport pixel, applies the display colour transform, and — when a profile
// is selected — the ICC conversion. Doing that on the host is image
// arithmetic on the host, so a device route must not download the source and
// resample it on the CPU. This kernel reads the source image read-only and
// writes only the bounded viewport texture.
//
// Filtering follows the host's mapping and source-cell coverage:
//   * the viewport pixel centre maps to the source coordinate through the
//     image rectangle, with the host's `- 0.5` pixel-centre convention;
//   * minification uses exact source-cell overlap; otherwise the separable
//     interpolation kernel and edge clamp are evaluated in f32;
//   * the display transform is the same decode → display matrix → sRGB
//     encode → clamp the viewer applies;
//   * the ICC tables, when present, are the resolved profile pipeline.
//
// Nothing here approximates the geometry, the kernel, or the colour
// transform. A viewport pixel outside the image rectangle is transparent,
// exactly as the host leaves it.

struct Params {
    // Viewport texture size in pixels.
    tex_width: u32,
    tex_height: u32,
    // Source image size in pixels.
    src_width: u32,
    src_height: u32,
    // The viewport rectangle the kernel maps through, in points.
    rect_min_x: f32,
    rect_min_y: f32,
    rect_width: f32,
    rect_height: f32,
    // The image rectangle the source is fitted into, in points.
    image_min_x: f32,
    image_min_y: f32,
    image_width: f32,
    image_height: f32,
    // Interpolation mode discriminant, matching the host's `Interpolation`.
    interpolation: u32,
    // Display transform: source CCTF discriminant and decode flag.
    src_cctf: u32,
    decode: u32,
    // Whether the display transform is enabled at all. When it is not, the
    // host views raw output-space values and no colour transform applies.
    transform: u32,
    // Display matrix, column-major as three vec4 columns.
    col0: vec4<f32>,
    col1: vec4<f32>,
    col2: vec4<f32>,
    // ICC pipeline presence and shape.
    icc: u32,
    icc_matrix: u32,
    icc_mat00: i32,
    icc_mat01: i32,
    icc_mat02: i32,
    icc_mat10: i32,
    icc_mat11: i32,
    icc_mat12: i32,
    icc_mat20: i32,
    icc_mat21: i32,
    icc_mat22: i32,
    icc_off0: i32,
    icc_off1: i32,
    icc_off2: i32,
    footprint_x: f32, footprint_y: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;      // [W*H*3]
@group(0) @binding(2) var<storage, read> icc_first: array<i32>;   // [3*256]
@group(0) @binding(3) var<storage, read> icc_second: array<u32>;  // [3*16385]
@group(0) @binding(4) var<storage, read_write> out: array<f32>;   // [TW*TH*3]

// Interpolation radius per mode, matching the host's `Interpolation::radius`.
// An if-chain rather than a switch keeps the kernel on the same construct the
// repo's other per-mode shaders use.
fn radius(mode: u32) -> i32 {
    if mode <= 1u {
        return 1; // nearest, linear
    }
    if mode <= 3u {
        return 2; // cubic, spline16
    }
    if mode == 4u {
        return 3; // spline36
    }
    return 4; // lanczos, blackman
}

// Per-mode kernel weight, matching the host's `Interpolation::weight`.
fn weight(mode: u32, x: f32) -> f32 {
    let a = abs(x);
    if a >= f32(radius(mode)) {
        return 0.0;
    }
    if mode == 0u {
        // nearest
        if a < 0.5 {
            return 1.0;
        }
        return 0.0;
    }
    if mode == 1u {
        return 1.0 - a; // linear
    }
    if mode == 2u {
        // cubic
        if a < 1.0 {
            return (4.0 - 6.0 * a * a + 3.0 * a * a * a) / 6.0;
        }
        let y = 2.0 - a;
        return y * y * y / 6.0;
    }
    if mode == 3u {
        // spline16
        if a < 1.0 {
            return ((a - 1.8) * a - 0.2) * a + 1.0;
        }
        let y = a - 1.0;
        return ((-y / 3.0 + 0.8) * y - 7.0 / 15.0) * y;
    }
    if mode == 4u {
        // spline36
        if a < 1.0 {
            return ((13.0 / 11.0 * a - 453.0 / 209.0) * a - 3.0 / 209.0) * a + 1.0;
        }
        if a < 2.0 {
            let y = a - 1.0;
            return ((-6.0 / 11.0 * y + 270.0 / 209.0) * y - 156.0 / 209.0) * y;
        }
        let y = a - 2.0;
        return ((y / 11.0 - 45.0 / 209.0) * y + 26.0 / 209.0) * y;
    }
    // lanczos (5) and blackman (6)
    if a < 1e-7 {
        return 1.0;
    }
    let p = 3.141592653589793 * a;
    let q = p / 4.0;
    let window = select(
        0.42 + 0.5 * cos(q) + 0.08 * cos(2.0 * q),
        sin(q) / q,
        mode == 5u,
    );
    return sin(p) / p * window;
}

// Source sample at integer coordinates, with the host's edge clamp.
fn source_pixel(x: i32, y: i32, c: u32) -> f32 {
    let cx = clamp(x, 0, i32(params.src_width) - 1);
    let cy = clamp(y, 0, i32(params.src_height) - 1);
    let base = (u32(cy) * params.src_width + u32(cx)) * 3u;
    return transform_pixel(vec3<f32>(source[base], source[base + 1u], source[base + 2u]))[c];
}

// Snap a coordinate that landed within one f32 ulp of an image edge back to
// the exact edge before the half-open containment test.
fn edge_snap(value: f32, lo: f32, hi: f32) -> f32 {
    let tolerance = 2.0 * 1.1920929e-7 * max(1.0, max(abs(value), abs(lo)));
    if abs(value - lo) <= tolerance {
        return lo;
    }
    if abs(value - hi) <= tolerance {
        return hi;
    }
    return value;
}

// floor(x + 0.5), with half coordinates rounding up. A plain
// floor(x + 0.5) can compile to round(), whose ties-to-even rule selects the
// wrong cell on exact half coordinates.
fn nearest_coord(x: f32) -> i32 {
    let shifted = x + 0.5;
    let base = floor(shifted);
    let frac = shifted - base;
    // Devices may round the shifted coordinate one ulp below an exact
    // integer. Snap such near-boundary values up so the host and device
    // select the same cell.
    let tolerance = 2.0 * 1.1920929e-7 * max(1.0, abs(shifted));
    if frac >= 1.0 - tolerance {
        return i32(base) + 1;
    }
    return i32(base);
}

// Separable interpolation at a continuous source coordinate, matching the
// host's `sample_f32` tap for tap.
//
// The host builds a per-axis weight array and then accumulates
// `wx[i] * wy[j] * pixel(...)` in row-major tap order. This kernel evaluates
// each weight at the same coordinate and accumulates in the same order, so
// the two agree bit for bit while the kernel holds no local array. That
// matters on drivers whose pipeline build mishandles a dynamically indexed
// local array: the values, not the storage, are what the host defines.
fn sample_source(x: f32, y: f32, c: u32) -> f32 {
    let mode = params.interpolation;
    if mode == 0u {
        return source_pixel(nearest_coord(x), nearest_coord(y), c);
    }
    let r = radius(mode);
    let bx = i32(floor(x));
    let by = i32(floor(y));
    let taps = 2u * u32(r);
    var out = 0.0;
    var j = 0u;
    loop {
        if j >= taps {
            break;
        }
        let wy = weight(mode, y - f32(by + i32(j) - r + 1));
        var i = 0u;
        loop {
            if i >= taps {
                break;
            }
            let wx = weight(mode, x - f32(bx + i32(i) - r + 1));
            out = out + wx * wy * source_pixel(bx + i32(i) - r + 1, by + i32(j) - r + 1, c);
            i = i + 1u;
        }
        j = j + 1u;
    }
    return out;
}

fn compensated(sum: f32, correction: f32, value: f32) -> vec2<f32> {
    let next = sum + value;
    if (bitcast<u32>(next) & 0x7f800000u) == 0x7f800000u { return vec2<f32>(next, 0.0); }
    var error: f32;
    if abs(sum) >= abs(value) { error = (sum - next) + value; }
    else { error = (value - next) + sum; }
    return vec2<f32>(next, correction + error);
}

// Average transformed source cells over the destination pixel's footprint.
fn sample_area(x: f32, y: f32, fx: f32, fy: f32, c: u32) -> f32 {
    let lo_x = (x + 0.5) - fx / 2.0;
    let lo_y = (y + 0.5) - fy / 2.0;
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
            chunk = compensated(chunk.x, chunk.y, wx * wy * source_pixel(ix, iy, c));
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

// --- Display transform, shared with the viewer's display-transform kernel. ---

fn spow(x: f32, p: f32) -> f32 {
    if x != x {
        return x;
    }
    if x == 0.0 {
        return x;
    }
    return sign(x) * pow(abs(x), p);
}

fn encode_one(v: f32, kind: u32) -> f32 {
    if kind == 0u { return v; }
    if kind == 1u {
        if v <= 0.0078125 {
            return 10.5402377416545 * v + 0.0729055341958355;
        }
        return (log2(v) + 9.72) / 17.52;
    }
    if kind == 2u {
        if v <= 0.0031308 {
            return 12.92 * v;
        }
        return 1.055 * spow(v, 1.0 / 2.4) - 0.055;
    }
    if kind == 3u {
        if 1.0 / 512.0 > v {
            return v * 16.0;
        }
        return spow(v, 1.0 / 1.8);
    }
    if kind == 4u {
        let a = 1.099;
        let b = 0.018;
        if v < b {
            return v * 4.5;
        }
        return a * spow(v, 0.45) - (a - 1.0);
    }
    if kind == 5u {
        return spow(v, 1.0 / (563.0 / 256.0));
    }
    return spow(v, 1.0 / 2.6);
}

fn decode_one(v: f32, kind: u32) -> f32 {
    if kind == 0u { return v; }
    if kind == 1u {
        if v > 0.155251141552511 {
            return exp2(v * 17.52 - 9.72);
        }
        return (v - 0.0729055341958355) / 10.5402377416545;
    }
    if kind == 2u {
        if 12.92 * 0.0031308 >= v {
            return v / 12.92;
        }
        return spow((v + 0.055) / 1.055, 2.4);
    }
    if kind == 3u {
        if v < 16.0 * (1.0 / 512.0) {
            return v / 16.0;
        }
        return spow(v, 1.8);
    }
    if kind == 4u {
        let a = 1.099;
        let b = 0.018;
        if v < a * pow(b, 0.45) - (a - 1.0) {
            return v / 4.5;
        }
        return spow((v + (a - 1.0)) / a, 1.0 / 0.45);
    }
    if kind == 5u {
        return spow(v, 563.0 / 256.0);
    }
    return spow(v, 2.6);
}

// --- ICC, shared with the viewer's ICC-transform kernel. ---

// The reference's fused matrix-shaper evaluation, reproduced exactly: the
// 8-bit input indexes the first shaper, the matrix runs in 1.14 fixed point
// with the reference's rounding bias, the result is clipped to the 1.14
// range, and the second shaper already holds the quantized output byte.
const ICC_FIXED14_MAX: i32 = 16384;
const ICC_FIXED14_BIAS: i32 = 0x2000;

fn icc_input_byte(v: f32) -> u32 {
    let scaled = f32(v) * 255.0;
    if scaled <= 0.0 {
        return 0u;
    }
    if scaled >= 255.0 {
        return 255u;
    }
    return u32(scaled);
}

fn icc_channel(c: u32, first: i32) -> u32 {
    if params.icc_matrix == 0u {
        var v = first;
        if v < 0 {
            v = 0;
        }
        if v > ICC_FIXED14_MAX {
            v = ICC_FIXED14_MAX;
        }
        return icc_second[c * 16385u + u32(v)];
    }
    return u32(first);
}

fn transform_pixel(value: vec3<f32>) -> vec3<f32> {
    var rgb = value;
    if params.transform != 0u {
        if params.decode != 0u {
            rgb = vec3<f32>(
                decode_one(rgb.x, params.src_cctf),
                decode_one(rgb.y, params.src_cctf),
                decode_one(rgb.z, params.src_cctf),
            );
        }
        let m = mat3x3<f32>(params.col0.xyz, params.col1.xyz, params.col2.xyz);
        rgb = m * rgb;
        rgb = vec3<f32>(
            encode_one(rgb.x, 2u),
            encode_one(rgb.y, 2u),
            encode_one(rgb.z, 2u),
        );
        rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    if params.icc != 0u {
        let ri = icc_input_byte(rgb.x);
        let gi = icc_input_byte(rgb.y);
        let bi = icc_input_byte(rgb.z);
        let r0 = icc_first[0u * 256u + ri];
        let g0 = icc_first[1u * 256u + gi];
        let b0 = icc_first[2u * 256u + bi];
        var out_bytes: vec3<u32>;
        if params.icc_matrix == 0u {
            out_bytes = vec3<u32>(
                icc_channel(0u, r0),
                icc_channel(1u, g0),
                icc_channel(2u, b0),
            );
        } else {
            let l0 = (params.icc_mat00 * r0 + params.icc_mat01 * g0 + params.icc_mat02 * b0 + params.icc_off0 + ICC_FIXED14_BIAS) >> 14;
            let l1 = (params.icc_mat10 * r0 + params.icc_mat11 * g0 + params.icc_mat12 * b0 + params.icc_off1 + ICC_FIXED14_BIAS) >> 14;
            let l2 = (params.icc_mat20 * r0 + params.icc_mat21 * g0 + params.icc_mat22 * b0 + params.icc_off2 + ICC_FIXED14_BIAS) >> 14;
            let c0 = clamp(l0, 0, ICC_FIXED14_MAX);
            let c1 = clamp(l1, 0, ICC_FIXED14_MAX);
            let c2 = clamp(l2, 0, ICC_FIXED14_MAX);
            out_bytes = vec3<u32>(
                icc_second[0u * 16385u + u32(c0)],
                icc_second[1u * 16385u + u32(c1)],
                icc_second[2u * 16385u + u32(c2)],
            );
        }
        // The second shaper holds the output byte widened to the 16-bit
        // domain, exactly as the reference stores it.
        rgb = vec3<f32>(
            f32(out_bytes.x & 0xFFu) / 255.0,
            f32(out_bytes.y & 0xFFu) / 255.0,
            f32(out_bytes.z & 0xFFu) / 255.0,
        );
    }
    return rgb;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    let n_pixels = params.tex_width * params.tex_height;
    if idx >= n_pixels {
        return;
    }
    let tx = idx % params.tex_width;
    let ty = idx / params.tex_width;
    let base = idx * 3u;
    // Viewport pixel centre in points, with the host's convention.
    let px = params.rect_min_x + (f32(tx) + 0.5) * params.rect_width / f32(params.tex_width);
    let py = params.rect_min_y + (f32(ty) + 0.5) * params.rect_height / f32(params.tex_height);
    // Outside the image rectangle the host leaves the pixel transparent, and
    // the caller never reads it.
    let image_max_x = params.image_min_x + params.image_width;
    let image_max_y = params.image_min_y + params.image_height;
    let px_edge = edge_snap(px, params.image_min_x, image_max_x);
    let py_edge = edge_snap(py, params.image_min_y, image_max_y);
    if px_edge < params.image_min_x || px_edge >= image_max_x
        || py_edge < params.image_min_y || py_edge >= image_max_y {
        out[base] = 0.0;
        out[base + 1u] = 0.0;
        out[base + 2u] = 0.0;
        return;
    }
    // Map the point into the source's continuous pixel grid.
    let uvx = (px_edge - params.image_min_x) / params.image_width;
    let uvy = (py_edge - params.image_min_y) / params.image_height;
    let sx = uvx * f32(params.src_width) - 0.5;
    let sy = uvy * f32(params.src_height) - 0.5;
    let fx = max(1.0, params.footprint_x);
    let fy = max(1.0, params.footprint_y);
    var rgb: vec3<f32>;
    if params.interpolation != 0u && (fx > 1.0 || fy > 1.0) {
        rgb = vec3<f32>(
            sample_area(sx, sy, fx, fy, 0u),
            sample_area(sx, sy, fx, fy, 1u),
            sample_area(sx, sy, fx, fy, 2u),
        );
    } else {
        rgb = vec3<f32>(
            sample_source(sx, sy, 0u),
            sample_source(sx, sy, 1u),
            sample_source(sx, sy, 2u),
        );
    }
    out[base] = rgb.x;
    out[base + 1u] = rgb.y;
    out[base + 2u] = rgb.z;
}
