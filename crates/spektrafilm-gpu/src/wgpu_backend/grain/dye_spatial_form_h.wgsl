// G5a of the Dye device path: horizontal formation over the extended cell
// rows. `horizontal[row][ci] = sum_t taps[t] * cells[row][ci + 2*d - t]`,
// which is the CPU's cell-integrated tap law: tap `t` carries offset `d - t`
// and the taps are the integrated Gaussian coefficients, not point samples.
// The extended row is the band's formation halo, so an artificial band
// boundary is never clamped or reflected.

const WORKGROUP: u32 = 256u;

@group(0) @binding(1) var<storage, read> cells: array<f32>;
@group(0) @binding(2) var<storage, read_write> horizontal: array<f32>;
@group(0) @binding(3) var<storage, read> taps: array<f32>;

@compute @workgroup_size(WORKGROUP)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    let index = gid.x + gid.y * grid.x * WORKGROUP;
    let cols = p.tile.y;
    if (index >= cols * p.cell_rect.w) { return; }
    let ci = index % cols;
    let row = index / cols;
    let d = p.tile.x;
    let components = select(3u, 1u, (p.ratios.w & 8u) != 0u);
    for (var c = 0u; c < components; c++) {
        var acc = 0.0;
        for (var t = 0u; t <= 2u * d; t++) {
            let src = (row * p.cell_rect.z + (ci + 2u * d - t)) * components + c;
            acc = acc + taps[t] * cells[src];
        }
        horizontal[index * components + c] = acc;
    }
}
