// G5c of the Dye device path: exact rectangle readout of the formed field
// and the single base subtraction.
//
// One thread per output pixel of the band. The footprint `[origin + k*pitch,
// origin + (k+1)*pitch)` in global film coordinates integrates the formed
// cell averages through exact fractional rectangle overlap, divided by the
// footprint width, so subcell footprints and noninteger ratios are covered.
// The formed rectangle holds every cell the footprint can touch, so no edge
// extension happens at an artificial band boundary. `density_min` is
// subtracted once, after the weighted sum.

const WORKGROUP: u32 = 256u;

@group(0) @binding(1) var<storage, read> formed: array<f32>;
@group(0) @binding(2) var<storage, read_write> output: array<f32>;

// First cell of the conservative window of one footprint, from the host's
// split of `pitch / cell` and of the origin phase. The window covers the
// estimate error and the exact overlap weight zeroes the rest.
fn cell_window_start(k: u32, q: u32, r_hi: f32, r_lo: f32, origin_i: i32, origin_frac: f32) -> i32 {
    let frac = origin_frac + f32(k) * r_hi + f32(k) * r_lo;
    return origin_i + i32(k) * i32(q) + i32(floor(frac - 1.0));
}

@compute @workgroup_size(WORKGROUP)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    let index = gid.x + gid.y * grid.x * WORKGROUP;
    let width = p.dims.z;
    if (index >= width * p.dims.w) { return; }
    let px = index % width;
    let py = index / width;
    let kx = p.phase.z + px;
    let ky = p.tile.w + py;
    let ix0 = cell_window_start(
        kx, p.ratios.y, p.fracs2.x, p.fracs2.y,
        bitcast<i32>(p.phase.x), p.fracs.z);
    let iy0 = cell_window_start(
        ky, p.ratios.z, p.fracs2.z, p.fracs2.w,
        bitcast<i32>(p.phase.y), p.fracs.w);
    let span_x = p.ratios.y + 5u;
    let span_y = p.ratios.z + 5u;
    // Footprint starts on the lattice: `origin + k * pitch`.
    let base_x = fx_add(p.origin.xy, fx_mul(kx, p.footprint.xy));
    let base_y = fx_add(p.origin.zw, fx_mul(ky, p.footprint.zw));
    // Inner formed cell `(0, 0)` in global cell coordinates.
    let inner_x = bitcast<i32>(p.cell_rect.x) + i32(p.tile.x);
    let inner_y = bitcast<i32>(p.cell_rect.y) + i32(p.tile.x);
    // The conservative window above is deliberately wider than the band's
    // formed rectangle (`WINDOW_MARGIN` is 3, the window carries 5 spare
    // cells), so its last rows and columns fall outside the buffer. Their
    // overlap weight is zero, but the index would still be evaluated:
    // `u32(i - inner)` on a negative difference wraps to ~4.29e9 and the
    // read runs far past the committed resource. D3D12 turns that into an
    // access violation (the Windows package job died in this test with exit
    // code 2173); Vulkan and Metal happened to tolerate it. Clamp the loops
    // to the rectangle instead: the skipped cells contribute nothing.
    let lo_x = u32(max(0, inner_x - ix0));
    let lo_y = u32(max(0, inner_y - iy0));
    let hi_x = min(span_x, u32(max(0, inner_x + i32(p.tile.y) - ix0)));
    let hi_y = min(span_y, u32(max(0, inner_y + i32(p.tile.z) - iy0)));
    var acc = vec3<f32>(0.0);
    for (var sy = lo_y; sy < hi_y; sy = sy + 1u) {
        let j = iy0 + i32(sy);
        let weight_y = overlap_weight(
            fx_sub(base_y, fx_signed_mul(j, p.pitches.xy)),
            p.widths.y, p.widths.y);
        if (weight_y == 0.0) { continue; }
        let ly = u32(j - inner_y);
        // Reduce one axis at a time. A coarse preview footprint can cover
        // tens of thousands of cells; one flat f32 sum loses constant mass.
        var row = vec3<f32>(0.0);
        for (var sx = lo_x; sx < hi_x; sx = sx + 1u) {
            let i = ix0 + i32(sx);
            let weight_x = overlap_weight(
                fx_sub(base_x, fx_signed_mul(i, p.pitches.xy)),
                p.widths.x, p.widths.x);
            if (weight_x == 0.0) { continue; }
            let lx = u32(i - inner_x);
            let src = (ly * p.tile.y + lx) * 3u;
            row = row + weight_x * vec3<f32>(
                formed[src], formed[src + 1u], formed[src + 2u]);
        }
        acc = acc + weight_y * row;
    }
    let dst = select(((p.tile.w + py) * p.phase.w + kx) * 3u,
        index * 3u, (p.ratios.w & 1u) != 0u);
    if ((p.ratios.w & 2u) != 0u) {
        acc = acc + vec3<f32>(output[dst], output[dst + 1u], output[dst + 2u]);
    }
    if ((p.ratios.w & 4u) == 0u) { acc = acc - p.mins.xyz; }
    output[dst] = acc.x;
    output[dst + 1u] = acc.y;
    output[dst + 2u] = acc.z;
}
