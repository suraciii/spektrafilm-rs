// Dye grain: the decision entry point over the shared device count law.
//
// The law, its constants and its coordinate-addressed words live in
// `dye_count.wgsl` and are concatenated ahead of this file
// (`concat!(include_str!("dye_count.wgsl"), include_str!("dye_decisions.wgsl"))`
// in `dye_decisions.rs`); the production G4 shader concatenates the same file,
// so the law tested here is the law production runs. This file adds only the
// decision case layout and its entry point.

/// Must match `dispatch_linear`'s assumption in `wgpu_backend/mod.rs`; a
/// smaller size silently computes only a fraction of the cases.
const WORKGROUP: u32 = 256u;

// ── entry point ──
// One case per invocation: a rate, a stream address, and the seed the address
// mixes in.

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
    // `dispatch_linear` spreads a long case list over a second grid dimension
    // once the workgroup count passes 65535, so the index is the same
    // `gid.x + gid.y * workgroups.x * WORKGROUP` used by `grain_v2.wgsl`.
    // Reading only `gid.x` would repeat row zero and leave the tail at zero,
    // which is a wrong count rather than a visible failure.
    let index = gid.x + gid.y * grid.x * WORKGROUP;
    if (index >= arrayLength(&cases)) { return; }
    let item = cases[index];
    let addr = address(item.i, item.j, item.tag, item.seed_lo, item.seed_hi);
    let out = count_of(item.lam, addr);
    counts[index] = out.x;
    words_used[index] = out.y;
}
