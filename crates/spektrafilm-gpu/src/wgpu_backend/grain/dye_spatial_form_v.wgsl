// G5b of the Dye device path: vertical formation over the inner cell rows.
// `formed[k][ci] = sum_t taps[t] * horizontal[k + 2*d - t][ci]`, the vertical
// half of the same cell-integrated tap law. The horizontal buffer already
// holds the band's full halo, so no band boundary is clamped here either.

const WORKGROUP: u32 = 256u;

@group(0) @binding(1) var<storage, read> horizontal: array<f32>;
@group(0) @binding(2) var<storage, read_write> formed: array<f32>;
@group(0) @binding(3) var<storage, read> taps: array<f32>;

@compute @workgroup_size(WORKGROUP)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    let index = gid.x + gid.y * grid.x * WORKGROUP;
    let cols = p.tile.y;
    if (index >= cols * p.tile.z) { return; }
    let ci = index % cols;
    let row = index / cols;
    let d = p.tile.x;
    let components = select(3u, 1u, (p.ratios.w & 8u) != 0u);
    for (var c = 0u; c < components; c++) {
        var acc = 0.0;
        for (var t = 0u; t <= 2u * d; t++) {
            let src = ((row + 2u * d - t) * cols + ci) * components + c;
            acc = acc + taps[t] * horizontal[src];
        }
        formed[index * components + c] = acc;
    }
}
