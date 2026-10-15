// Dye grain: the device Poisson count law, shared by every kernel that draws
// counts. There is one law: the decision kernel (`dye_decisions.wgsl`) and the
// production G4 shader concatenate this file ahead of their own entry points
// (`concat!(include_str!("dye_count.wgsl"), ...)` in `dye_decisions.rs` and
// `dye.rs`), so the test evidence over these functions covers the production
// source. This file has no entry point, no bindings and no workgroup size:
// those belong to each kernel.
//
// The device evaluates Knuth's multiplication below 30 and Hörmann's PTRS
// above -- the reference's own law and sampling domain, in the device's own
// arithmetic precision and from the device's own words.
// `specs/film-grain/v3/design.md#device-count-law` owns that contract: Dye's
// appearance is a function of the realized count statistics, not of the
// reference's individual draws, so the device is not required to reproduce
// them, and the CPU stays the reference for *statistics* rather than for
// words. What the device must not do is change the law: the reference's exact
// Poisson, never a normal approximation.

/// Largest number of words one decision may consume before the kernel reports
/// failure instead of guessing. The reference loops unboundedly; the cap is far
/// above the largest supported rate's expected consumption.
const MAX_DRAWS: u32 = 512u;
/// Beyond this boundary f32 cannot represent every adjacent integer count.
/// The conservative proposal guard reports failure instead of rounding.
const COUNT_LIMIT: f32 = 16777216.0;
/// The largest rate this device law realizes. Must match `DYE_MAX_RATE` in
/// `dye_decisions.rs`.
///
/// This statistical envelope is distinct from the exact-integer domain.
/// Its probe rationale and device acceptance requirements live with the host
/// constant; the actual adapter must pass certification before field use.
const MAX_RATE: f32 = 65536.0;
/// The sentinel written when a decision cannot be realized exactly: a rate the
/// device does not support, a proposal outside the exact f32 integer range, or
/// a word budget exhausted after `MAX_DRAWS` words. A truncated or rounded
/// count is a wrong count, not a slow one.
const DECISION_FAILED: u32 = 0xFFFFFFFFu;

// ── coordinate-addressed words ──
// The reference draws from a SplitMix64 chain, which needs 64-bit integers.
// Dye does not: it is coordinate-based, so the address alone can fix a value.
// Each draw mixes the address's two words with the draw index and finalizes
// with a 32-bit avalanche. Crop bounds, tile size, traversal order, dispatch
// shape and workgroup size never enter the address.

fn mix(x: u32) -> u32 {
    var h = x;
    h = (h ^ (h >> 16u)) * 0x7FEB352Du;
    h = (h ^ (h >> 15u)) * 0x846CA68Bu;
    return h ^ (h >> 16u);
}

/// Address of one `(cell, channel and layer, seed)` stream: a coordinate
/// identity, not a position in a traversal.
/// Distinct `(channel, layer)` tags can collide, because folding the tag into
/// the coordinate is not a joint injection of the pair -- the colliding cell is
/// about `2^31` cells away, which no field spans, so the addresses stay
/// distinct over every realizable field.
fn address(i: i32, j: i32, tag: u32, seed_lo: u32, seed_hi: u32) -> vec2<u32> {
    return vec2<u32>(
        mix(u32(i) ^ (tag * 0x9E3779B9u)),
        mix(u32(j) ^ (seed_lo * 0x85EBCA6Bu) ^ (seed_hi * 0xC2B2AE35u)),
    );
}

/// The `k`-th uniform in `[0, 1)` of one address. Twenty-four mantissa bits
/// exactly: a value that needs more cannot be represented in f32, and rounding
/// one would bias the comparison it feeds.
fn uniform(addr: vec2<u32>, k: u32) -> f32 {
    let h = mix(addr.x ^ (addr.y * 0x9E3779B9u) ^ (k * 0x85EBCA6Bu));
    return f32(h >> 8u) * 5.960464477539063e-8;
}

// ── the count law ──

/// `ln(k!)` for the small integer counts PTRS proposes. Exact factorials below
/// the table's end; Stirling's series with the first correction above it.
const LN_FACT_LEN: u32 = 13u;
const LN_FACT: array<f32, LN_FACT_LEN> = array<f32, LN_FACT_LEN>(
    0.0,
    0.0,
    0.6931471824645996,
    1.7917594909667969,
    3.178053855895996,
    4.787491798400879,
    6.579251289367676,
    8.525161743164062,
    10.604602813720703,
    12.801827430725098,
    15.104412078857422,
    17.502307891845703,
    19.987214111328125,
);

fn ln_fact(k: f32) -> f32 {
    if (k < f32(LN_FACT_LEN)) {
        return LN_FACT[u32(k)];
    }
    let z = k + 1.0;
    // Stirling: (z - 1/2) ln z - z + ln(sqrt(2 pi)) + 1/(12 z).
    return (z - 0.5) * log(z) - z + 0.9189385175704956 + 1.0 / (12.0 * z);
}

/// `log(1 + t) - t` without the cancellation of subtracting `t` from
/// `log(1 + t)` near zero, where the two agree to most of their bits and the
/// difference carries the rejection test's whole signal at the top of the rate
/// range. WGSL has no `log1p`, so for `|t| <= 0.1` the alternating series
/// `log(1 + t) - t = t^2 (-1/2 + t/3 - t^2/4 + t^3/5 - ...)` is summed by
/// Horner with seven terms -- its truncation is below `2e-10` there, and it
/// never calls `log` -- while outside that band the direct form is already
/// accurate because `|t|` is then large against `log(1 + t)`.
fn log1p_minus_t(t: f32) -> f32 {
    if (abs(t) <= 0.1) {
        let s = -0.5
            + t * (0.3333333333333333
            + t * (-0.25
            + t * (0.2
            + t * (-0.16666666666666666
            + t * (0.14285714285714285
            + t * -0.125)))));
        return t * t * s;
    }
    return log(1.0 + t) - t;
}

/// The right-hand side of the PTRS rejection test: the exact Poisson
/// log-probability `-lambda + k log(lambda) - ln(k!)`, the same equation the
/// reference decides on. Below the factorial table the terms cannot cancel at
/// these magnitudes, so the equation itself is already stable there. Above it
/// the same Stirling factorial `ln_fact` evaluates is arranged so that no two
/// large words subtract: with `z = k + 1` and `t = (lambda - z) / z`,
///
///     -lambda + k log(lambda) - ln(k!)
///       = k (log1p(t) - t) - t - log(z)/2 - log(sqrt(2 pi)) - 1/(12 z),
///
/// an identity in exact arithmetic. The factorial form at the top of the rate
/// range subtracts two words of magnitude `k log(lambda) ~ 7e5` to reach a
/// result of magnitude single digits -- an erratic f32 error of some `0.03`
/// nats that the rejection boundary, only `0.08` nats wide, sees directly --
/// and multiplies the device `log`'s error by `k`. This form keeps `log` at
/// coefficient `1/2`, and in the accepted band `|t| <= 0.1` reaches the value
/// through `log1p_minus_t`'s series without calling `log` at all.
fn ptrs_rhs(lam: f32, loglam: f32, k: f32) -> f32 {
    if (k < f32(LN_FACT_LEN)) {
        return -lam + k * loglam - ln_fact(k);
    }
    let z = k + 1.0;
    let t = (lam - z) / z;
    return k * log1p_minus_t(t) - t - 0.5 * log(z) - 0.9189385175704956 - 1.0 / (12.0 * z);
}

/// Knuth's multiplication: multiply uniforms until the product drops to
/// `exp(-lambda)`. Returns `(count, words used)`.
fn knuth(lam: f32, addr: vec2<u32>) -> vec2<u32> {
    let limit = exp(-lam);
    var product = 1.0;
    for (var k: u32 = 0u; k < MAX_DRAWS; k = k + 1u) {
        product = product * uniform(addr, k);
        if (product <= limit) {
            return vec2<u32>(k, k + 1u);
        }
    }
    return vec2<u32>(DECISION_FAILED, MAX_DRAWS);
}

/// The PTRS proposal constants at one rate.
struct PtrsConsts {
    a: f32,
    b: f32,
    inv_alpha: f32,
    vr: f32,
}

fn ptrs_consts(lam: f32) -> PtrsConsts {
    let slam = sqrt(lam);
    let b = 0.931 + 2.53 * slam;
    let a = -0.059 + 0.02483 * b;
    let inv_alpha = 1.1239 + 1.1328 / (b - 3.4);
    let vr = 0.9277 - 3.6224 / (b - 2.0);
    return PtrsConsts(a, b, inv_alpha, vr);
}

/// One proposal pair's verdict before the rejection test's comparison, which
/// needs the caller's equation: the four exits the reference's trial has.
const PTRS_RETRY: u32 = 0u;
const PTRS_FAILED: u32 = 1u;
const PTRS_SHORTCUT: u32 = 2u;
const PTRS_LOG_TEST: u32 = 3u;

struct PtrsPair {
    state: u32,
    proposal: f32,
    lhs: f32,
}

/// The trial body the law and its acceptance controls share: given one pair's
/// centred uniform `u` and its second uniform `v`, everything up to the
/// `lhs <= rhs` comparison, in the reference's own order. `lhs` is meaningful
/// only in `PTRS_LOG_TEST`; the caller supplies the rejection equation and
/// compares, so a control can replace that one equation and nothing else.
fn ptrs_pair(c: PtrsConsts, lam: f32, u: f32, v: f32) -> PtrsPair {
    let us = 0.5 - abs(u);
    if (us == 0.0) {
        return PtrsPair(PTRS_RETRY, 0.0, 0.0);
    }
    let proposal = floor((2.0 * c.a / us + c.b) * u + lam + 0.43);
    if (proposal < 0.0) {
        return PtrsPair(PTRS_RETRY, proposal, 0.0);
    }
    // The reference rejects the squeeze before it tests the proposal
    // against its own exact-integer bound, so a proposal this device could
    // not represent must not fail the case while the reference would have
    // rejected it and drawn again.
    if (us < 0.013 && v > us) {
        return PtrsPair(PTRS_RETRY, proposal, 0.0);
    }
    if (proposal >= COUNT_LIMIT) {
        return PtrsPair(PTRS_FAILED, 0.0, 0.0);
    }
    if (us >= 0.07 && v <= c.vr) {
        return PtrsPair(PTRS_SHORTCUT, proposal, 0.0);
    }
    let lhs = log(v) + log(c.inv_alpha) - log(c.a / (us * us) + c.b);
    return PtrsPair(PTRS_LOG_TEST, proposal, lhs);
}

/// Hörmann's transformed rejection with squeeze, the same algorithm the
/// reference runs. Returns `(count, words used)`.
fn ptrs(lam: f32, addr: vec2<u32>) -> vec2<u32> {
    let loglam = log(lam);
    let c = ptrs_consts(lam);
    var k: u32 = 0u;
    for (var guard: u32 = 0u; guard < MAX_DRAWS; guard = guard + 1u) {
        if (k + 2u > MAX_DRAWS) { break; }
        let u = uniform(addr, k) - 0.5;
        k = k + 1u;
        let v = uniform(addr, k);
        k = k + 1u;
        let pair = ptrs_pair(c, lam, u, v);
        if (pair.state == PTRS_RETRY) { continue; }
        if (pair.state == PTRS_FAILED) {
            return vec2<u32>(DECISION_FAILED, k);
        }
        if (pair.state == PTRS_SHORTCUT) {
            return vec2<u32>(u32(pair.proposal), k);
        }
        if (pair.lhs <= ptrs_rhs(lam, loglam, pair.proposal)) {
            return vec2<u32>(u32(pair.proposal), k);
        }
    }
    return vec2<u32>(DECISION_FAILED, k);
}

/// One count from one stream, dispatching on the rate exactly as the reference
/// does. Returns `(count, words used)`.
///
/// Zero rate is zero count. Any rate the device cannot realize faithfully --
/// negative, NaN, above `MAX_RATE` -- fails its case with `DECISION_FAILED`
/// instead of returning a rounded, truncated or differently distributed count.
fn count_of(lam: f32, addr: vec2<u32>) -> vec2<u32> {
    if (lam == 0.0) {
        return vec2<u32>(0u, 0u);
    }
    // `!(lam > 0.0)` is also true for NaN, which no other comparison sees.
    if (!(lam > 0.0) || lam > MAX_RATE) {
        return vec2<u32>(DECISION_FAILED, 0u);
    }
    if (lam < 30.0) {
        return knuth(lam, addr);
    }
    return ptrs(lam, addr);
}
