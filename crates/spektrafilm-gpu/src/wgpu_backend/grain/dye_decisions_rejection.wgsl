// The placed numerical-rejection probe: one placed `(lam, u, v)` trial per
// invocation, run through the production law's own trial body and rejection
// equation, and through the pre-repair control equation beside it.
//
// The three files concatenate as `dye_count.wgsl`,
// `dye_count_control_cancelled.wgsl` (whose `cancelled_rhs` is the control
// column), then this one (`REJECTION_SHADER` in `dye_decisions.rs`). The host
// compares each state, each verdict and each right-hand side against an
// independent f64 oracle of the exact factorial equation, so the evidence is
// the device's own arithmetic at the placed boundary, not a distribution.
// Only tests compile it.

/// Must match `dispatch_linear`'s assumption in `wgpu_backend/mod.rs`; a
/// smaller size silently computes only a fraction of the places.
const WORKGROUP: u32 = 256u;

/// One placed trial: the rate, the pair's raw uniform `u01` (the law's
/// centred `u` is `u01 - 0.5`) and the pair's second uniform `v`.
struct Place {
    lam: f32,
    u01: f32,
    v: f32,
    pad: u32,
}

@group(0) @binding(0) var<storage, read> places: array<Place>;
// Eight words per place: the trial's state, the stable verdict, the control
// verdict, then `proposal`, `lhs`, `rhs_stable` and `rhs_control` as raw f32
// bits, then padding.
@group(0) @binding(1) var<storage, read_write> results: array<u32>;

@compute @workgroup_size(WORKGROUP)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    let index = gid.x + gid.y * grid.x * WORKGROUP;
    if (index >= arrayLength(&places)) { return; }
    let p = places[index];
    let lam = p.lam;
    let loglam = log(lam);
    let c = ptrs_consts(lam);
    let pair = ptrs_pair(c, lam, p.u01 - 0.5, p.v);
    var verdict_stable = 0u;
    var verdict_control = 0u;
    var rhs_stable = 0.0;
    var rhs_control = 0.0;
    if (pair.state == PTRS_SHORTCUT) {
        verdict_stable = 1u;
        verdict_control = 1u;
    } else if (pair.state == PTRS_LOG_TEST) {
        rhs_stable = ptrs_rhs(lam, loglam, pair.proposal);
        rhs_control = cancelled_rhs(lam, loglam, pair.proposal);
        if (pair.lhs <= rhs_stable) { verdict_stable = 1u; }
        if (pair.lhs <= rhs_control) { verdict_control = 1u; }
    }
    let base = index * 8u;
    results[base] = pair.state;
    results[base + 1u] = verdict_stable;
    results[base + 2u] = verdict_control;
    results[base + 3u] = bitcast<u32>(pair.proposal);
    results[base + 4u] = bitcast<u32>(pair.lhs);
    results[base + 5u] = bitcast<u32>(rhs_stable);
    results[base + 6u] = bitcast<u32>(rhs_control);
    results[base + 7u] = 0u;
}
