// The count-decision entry over the cancelled rejection control: the same
// case layout and entry as `dye_decisions.wgsl`, reading counts from
// `dye_count_control_cancelled.wgsl`'s law instead of the production one.
//
// The three files concatenate as `dye_count.wgsl`,
// `dye_count_control_cancelled.wgsl`, then this one (`CANCELLED_SHADER` in
// `dye_decisions.rs`). Only tests compile it: it exists so the pooled
// five-replicate screen can be shown to reject the equation the repair
// replaced, at the top of the rate range.

/// Must match `dispatch_linear`'s assumption in `wgpu_backend/mod.rs`; a
/// smaller size silently computes only a fraction of the cases.
const WORKGROUP: u32 = 256u;

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
    let out = cancelled_count_of(item.lam, addr);
    counts[index] = out.x;
    words_used[index] = out.y;
}
