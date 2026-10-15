// Acceptance control for `dye_count.wgsl`'s rejection equation: the same
// coordinate-addressed words, the same trial body, the same branch at thirty
// and the same factorial term, with the one equation this repair replaced.
//
// `ptrs_rhs` rearranges the exact Poisson log-probability `-lambda + k
// log(lambda) - ln(k!)` so that no two large f32 words cancel; `cancelled_rhs`
// keeps the factorial form's own arrangement, verbatim. At the top of the rate
// range that form loses the rejection signal to f32 cancellation -- the
// observed systematic variance bias of the first hardware acceptance run --
// so the placed probe must measure its decisions against an independent f64
// oracle, and the top-rate control test must see the equation exposed: by the
// pooled counts where the adapter's arithmetic rounds the cancellation into a
// systematic count bias, by its word consumption where it does not.
//
// This module is concatenated after `dye_count.wgsl` (whose words and trial
// body it draws from and whose real `count_of` it never calls). It is not part
// of the library build; only `dye_decisions.rs`'s tests compile and run it.

/// The pre-repair rejection equation, verbatim: the factorial term in its
/// cancelling arrangement, `ln_fact` and all. Identical to `ptrs_rhs` in
/// exact arithmetic and the computation the repair replaced.
fn cancelled_rhs(lam: f32, loglam: f32, k: f32) -> f32 {
    return -lam + k * loglam - ln_fact(k);
}

/// The whole count law with `cancelled_rhs` in the rejection test. The loop
/// calls the shared `ptrs_pair` trial body, so this differs from the
/// production `ptrs` in exactly one place: the rejection equation.
fn cancelled_ptrs(lam: f32, addr: vec2<u32>) -> vec2<u32> {
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
        if (pair.lhs <= cancelled_rhs(lam, loglam, pair.proposal)) {
            return vec2<u32>(u32(pair.proposal), k);
        }
    }
    return vec2<u32>(DECISION_FAILED, k);
}

/// `count_of` with the cancelled rejection law, for the full-law control.
fn cancelled_count_of(lam: f32, addr: vec2<u32>) -> vec2<u32> {
    if (lam == 0.0) {
        return vec2<u32>(0u, 0u);
    }
    if (!(lam > 0.0) || lam > MAX_RATE) {
        return vec2<u32>(DECISION_FAILED, 0u);
    }
    if (lam < 30.0) {
        return knuth(lam, addr);
    }
    return cancelled_ptrs(lam, addr);
}
