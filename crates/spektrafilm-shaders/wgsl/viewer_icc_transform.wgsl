// GUI viewer ICC display transform: the device port of `apply_display_profile`
// in `crates/spektrafilm-gui/src/display/colortransform.rs`.
//
// The viewer's ICC conversion maps encoded sRGB viewing pixels through the
// selected display profile. It is disposable viewer data: it never becomes a
// retained or saved sample, and the source pixels are never modified.
//
// This kernel reproduces the exact algorithm the reference evaluates for an
// RGB8 → RGB8 matrix-shaper transform, which is the class every display
// profile the viewer can select belongs to. The reference's own optimizer
// reduces that transform to two lookup tables and a fixed-point matrix:
//
//   1. the 8-bit input selects an entry from a 256-entry first-shaper table,
//      which holds the source transfer curve scaled to 1.14 fixed point;
//   2. the matrix runs in 1.14 fixed point with a rounding bias, exactly as
//      the reference's fused matrix-shaper routine computes it;
//   3. the result is clipped to the 1.14 range;
//   4. the clipped index selects an entry from a 16385-entry second-shaper
//      table, which holds the destination inverse transfer curve already
//      quantized to the 8-bit output raster.
//
// Both tables and the matrix are resolved on the host from the profiles' own
// tags, so the device evaluates the same values rather than re-deriving them.
// Every arithmetic step below is integer or a table lookup, so the device
// reproduces the reference bit for bit instead of approximating it: there is
// no floating-point policy to disagree about.
//
// A profile whose resolved pipeline is not this class is declined by the host,
// which keeps the reference implementation. A device port must not approximate
// a pipeline it cannot reproduce.

struct Params {
    n_pixels: u32,
    /// Whether the request carries a matrix stage. A matrix-free request is a
    /// curve-to-curve join, which the reference also produces.
    matrix: u32,
    // Matrix in 1.14 fixed point, row-major. Used only when `matrix` is set.
    mat00: i32,
    mat01: i32,
    mat02: i32,
    mat10: i32,
    mat11: i32,
    mat12: i32,
    mat20: i32,
    mat21: i32,
    mat22: i32,
    // Offset in 1.14 fixed point. Used only when `matrix` is set.
    off0: i32,
    off1: i32,
    off2: i32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;        // [N*3] 0..=1
@group(0) @binding(2) var<storage, read> shaper1: array<i32>;      // [3*256]
@group(0) @binding(3) var<storage, read> shaper2: array<u32>;      // [3*16385]
@group(0) @binding(4) var<storage, read_write> out: array<f32>;    // [N*3] 0..=1

// The 1.14 range the reference clips to, and the rounding bias it adds.
const FIXED14_MAX: i32 = 16384;
const FIXED14_BIAS: i32 = 0x2000;

// The 8-bit input value the reference's first shaper indexes with.
fn to_input_byte(v: f32) -> u32 {
    // The viewer quantizes by truncating clamped RGB to bytes before LCMS.
    let scaled = f32(v) * 255.0;
    if scaled <= 0.0 {
        return 0u;
    }
    if scaled >= 255.0 {
        return 255u;
    }
    return u32(scaled);
}

// One channel through the reference's fused matrix-shaper evaluation.
fn transform_channel(base_in: u32, c: u32) -> u32 {
    let first = shaper1[c * 256u + base_in];
    if params.matrix == 0u {
        // No matrix: the first shaper's value is already the second shaper's
        // input domain, so clip and index directly.
        var v = first;
        if v < 0 {
            v = 0;
        }
        if v > FIXED14_MAX {
            v = FIXED14_MAX;
        }
        return shaper2[c * 16385u + u32(v)];
    }
    return first;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels {
        return;
    }
    let base = idx * 3u;
    let ri = to_input_byte(input[base]);
    let gi = to_input_byte(input[base + 1u]);
    let bi = to_input_byte(input[base + 2u]);
    var out_bytes: vec3<u32>;
    if params.matrix == 0u {
        out_bytes = vec3<u32>(
            transform_channel(ri, 0u),
            transform_channel(gi, 1u),
            transform_channel(bi, 2u),
        );
    } else {
        // The three first-shaper values, in 1.14 fixed point.
        let r = shaper1[0u * 256u + ri];
        let g = shaper1[1u * 256u + gi];
        let b = shaper1[2u * 256u + bi];
        // The matrix in 1.14 fixed point, with the reference's rounding bias.
        // The products stay inside i32: 1.14 values are at most 32768 and the
        // matrix entries are at most 32768, so a product is at most ~2^30 and
        // the three-term sum with the offset stays inside i32.
        let l0 = (params.mat00 * r + params.mat01 * g + params.mat02 * b + params.off0 + FIXED14_BIAS) >> 14;
        let l1 = (params.mat10 * r + params.mat11 * g + params.mat12 * b + params.off1 + FIXED14_BIAS) >> 14;
        let l2 = (params.mat20 * r + params.mat21 * g + params.mat22 * b + params.off2 + FIXED14_BIAS) >> 14;
        // Clip to the 1.14 range, then index the second shaper.
        let c0 = clamp(l0, 0, FIXED14_MAX);
        let c1 = clamp(l1, 0, FIXED14_MAX);
        let c2 = clamp(l2, 0, FIXED14_MAX);
        out_bytes = vec3<u32>(
            shaper2[0u * 16385u + u32(c0)],
            shaper2[1u * 16385u + u32(c1)],
            shaper2[2u * 16385u + u32(c2)],
        );
    }
    // The second shaper already holds the 8-bit output widened back to the
    // 16-bit domain, exactly as the reference stores it. Scale to 0..1 the
    // same way the host does after the reference transform.
    out[base] = f32(out_bytes.x & 0xFFu) / 255.0;
    out[base + 1u] = f32(out_bytes.y & 0xFFu) / 255.0;
    out[base + 2u] = f32(out_bytes.z & 0xFFu) / 255.0;
}
