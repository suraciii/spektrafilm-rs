// Shared geometry of the Dye spatial passes (G3 remap, G5 readout).
//
// Overlap weights are differences of large film coordinates: `origin +
// index * pitch` and `cell * pitch` are both tens of thousands of
// micrometers apart while the local difference is a few. Evaluated in f32
// they cancel to roughly 2e-3 um, three orders of magnitude above the
// weight precision the readout needs. The host therefore packs every origin
// and pitch as a signed 64-bit fixed-point value in 2^-42 um, and the
// boundary differences are computed in two-word integer arithmetic here:
// exact for every validated geometry, with one final rounding of the small
// local difference to f32. Plain compensated float residuals are not used;
// they do not survive compiler contraction on every driver.
//
// The same global expression is evaluated for every band and every crop, so
// banding and cropping cannot move a value beyond that final rounding. The
// plan (`DyePlan`) validates every product and sum below against the
// representable range before any dispatch, and rejects geometry that exceeds
// it with an actionable error.
//
// Word order is (high, low), two's complement.

const FX_SCALE: f32 = 2.2737367544323206e-13; // 2^-42 um

struct Params {
    origin: vec4<u32>,      // x0 (hi, lo), y0 (hi, lo)
    footprint: vec4<u32>,   // pitch x (hi, lo), pitch y (hi, lo)
    pitches: vec4<u32>,     // cell (hi, lo), source (hi, lo)
    dims: vec4<u32>,        // source width, source height, output width, output rows
    source_rect: vec4<u32>, // window x, window y, window cols, window rows
    cell_rect: vec4<u32>,   // cell x (i32 bits), cell y (i32 bits), ext cols, ext rows
    tile: vec4<u32>,        // radius d, cols, rows, output y
    ratios: vec4<u32>,      // cell/source q, pitch x/cell q, pitch y/cell q, spare
    phase: vec4<u32>,       // origin x/cell (i32 bits), origin y/cell (i32 bits), spare, spare
    fracs: vec4<f32>,       // cell/source r hi, r lo, origin x frac, origin y frac
    fracs2: vec4<f32>,      // pitch x/cell r hi, r lo, pitch y/cell r hi, r lo
    widths: vec4<f32>,      // pitch x, pitch y, cell, source (um)
    mins: vec4<f32>,        // density_min per channel
}

@group(0) @binding(0) var<uniform> p: Params;

fn fx_add(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let lo = a.y + b.y;
    return vec2<u32>(a.x + b.x + select(0u, 1u, lo < a.y), lo);
}

fn fx_neg(a: vec2<u32>) -> vec2<u32> {
    let lo = ~a.y + 1u;
    return vec2<u32>(~a.x + select(0u, 1u, lo == 0u), lo);
}

fn fx_sub(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let lo = a.y - b.y;
    return vec2<u32>(a.x - b.x - select(0u, 1u, a.y < b.y), lo);
}

// Exact 32x32 -> 64 product.
fn mul32(a: u32, b: u32) -> vec2<u32> {
    let a0 = a & 0xffffu;
    let a1 = a >> 16u;
    let b0 = b & 0xffffu;
    let b1 = b >> 16u;
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 16u) + (p01 & 0xffffu) + (p10 & 0xffffu);
    return vec2<u32>(
        p11 + (p01 >> 16u) + (p10 >> 16u) + (mid >> 16u),
        (mid << 16u) | (p00 & 0xffffu));
}

// `m * a` with `m >= 0`; the plan bounds the product below 2^63.
fn fx_mul(m: u32, a: vec2<u32>) -> vec2<u32> {
    let lo = mul32(m, a.y);
    let hi = mul32(m, a.x);
    return vec2<u32>(hi.y + lo.x, lo.y);
}

fn fx_signed_mul(m: i32, a: vec2<u32>) -> vec2<u32> {
    let negative = m < 0;
    let product = fx_mul(select(u32(m), u32(-m), negative), a);
    return select(product, fx_neg(product), negative);
}

// The single rounding of a lattice difference, in um.
fn fx_f32(v: vec2<u32>) -> f32 {
    let negative = (v.x & 0x80000000u) != 0u;
    let mag = select(v, fx_neg(v), negative);
    let scaled = f32(mag.x) * 4294967296.0 + f32(mag.y);
    return select(scaled, -scaled, negative) * FX_SCALE;
}

// Overlap of the cell `[0, cell)` with the second rectangle `[u, u + gap)`,
// normalized by `norm`. `u` is the second rectangle's start relative to the
// cell start: exact on the lattice, so weights sum to one over the covered
// rectangles and constants survive the transfer.
fn overlap_weight(u: vec2<u32>, gap: f32, norm: f32) -> f32 {
    let start = fx_f32(u);
    let cell = p.widths.z;
    return max(0.0, min(start + gap, cell) - max(start, 0.0)) / norm;
}
