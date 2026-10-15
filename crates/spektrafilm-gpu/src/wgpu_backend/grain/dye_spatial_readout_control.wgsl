// KNOWN-BAD control of G5c: exact rectangle readout with the overlap weights
// normalized by the cell width instead of the footprint width. A device run
// of this shader must fail the same independent oracle comparison that
// accepts `dye_spatial_readout.wgsl`, and must reproduce the CPU's
// `ReadoutNorm::Cell` control values. Keep the body otherwise
// byte-identical to the production pass so the altered law is the only
// difference.

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
    let kx = px;
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
    var acc = vec3<f32>(0.0);
    for (var sy = 0u; sy < span_y; sy = sy + 1u) {
        let j = iy0 + i32(sy);
        // Broken law: normalized by the cell width, not the footprint width.
        let weight_y = overlap_weight(
            fx_sub(base_y, fx_signed_mul(j, p.pitches.xy)),
            p.widths.y, p.widths.z);
        if (weight_y == 0.0) { continue; }
        let ly = u32(j - inner_y);
        for (var sx = 0u; sx < span_x; sx = sx + 1u) {
            let i = ix0 + i32(sx);
            // Broken law: normalized by the cell width, not the footprint width.
            let weight_x = overlap_weight(
                fx_sub(base_x, fx_signed_mul(i, p.pitches.xy)),
                p.widths.x, p.widths.z);
            if (weight_x == 0.0) { continue; }
            let lx = u32(i - inner_x);
            let src = (ly * p.tile.y + lx) * 3u;
            acc = acc + (weight_x * weight_y) * vec3<f32>(
                formed[src], formed[src + 1u], formed[src + 2u]);
        }
    }
    let dst = ((p.tile.w + py) * width + px) * 3u;
    output[dst] = acc.x - p.mins.x;
    output[dst + 1u] = acc.y - p.mins.y;
    output[dst + 2u] = acc.z - p.mins.z;
}
