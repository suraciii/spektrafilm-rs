// Acceptance control for `dye_count.wgsl`'s law: the same words, the same
// entry, and the count law the specification forbids.
//
// `specs/film-grain/v3/design.md#device-count-law` accepts the device kernel on
// statistical evidence, so the evidence is only worth something if a build with
// a different law fails it. This module is concatenated after `dye_count.wgsl`
// (whose shared words this control draws from and whose real `count_of` it
// never calls), so the control differs from the production kernel in exactly
// one place: `broken_count_of` replaces the whole count law with a rounded
// normal. Above the switch it never enters Hörmann's transformed rejection,
// and below it never enters Knuth's product loop. A rounded normal has the
// same mean and variance as Poisson, so the moment checks pass by construction
// where the rates are large; what fails is the empty-cell probability below
// the switch and the shape check on both sides of it.
//
// It is not part of the library build; only `dye_decisions.rs`'s tests compile
// and run it.

const WORKGROUP: u32 = 256u;

/// Twelve uniforms summed and centred: an Irwin-Hall normal approximation,
/// scaled to unit variance. Rounded to an integer, it matches Poisson's first
/// two moments and not its shape.
fn normal_approximation(lam: f32, addr: vec2<u32>) -> vec2<u32> {
    var sum = 0.0;
    for (var k: u32 = 0u; k < 12u; k = k + 1u) {
        sum = sum + uniform(addr, k);
    }
    let sample = lam + sqrt(lam) * (sum - 6.0);
    return vec2<u32>(u32(max(round(sample), 0.0)), 12u);
}

fn broken_count_of(lam: f32, addr: vec2<u32>) -> vec2<u32> {
    if (lam == 0.0) {
        return vec2<u32>(0u, 0u);
    }
    // The whole law is replaced, not just the branch above the switch: a
    // normal that replaced only the upper branch would keep every count in the
    // lower branch, where the empty-cell probability makes the difference
    // unmistakable.
    return normal_approximation(lam, addr);
}

struct Case {
    lam: f32,
    seed_lo: u32,
    seed_hi: u32,
    tag: u32,
    i: i32,
    j: i32,
    pad0: u32,
    pad1: u32,
}

@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> counts: array<u32>;
@group(0) @binding(2) var<storage, read_write> words_used: array<u32>;

@compute @workgroup_size(WORKGROUP)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    let index = gid.x + gid.y * grid.x * WORKGROUP;
    if (index >= arrayLength(&cases)) { return; }
    let item = cases[index];
    let addr = address(item.i, item.j, item.tag, item.seed_lo, item.seed_hi);
    let out = broken_count_of(item.lam, addr);
    counts[index] = out.x;
    words_used[index] = out.y;
}
