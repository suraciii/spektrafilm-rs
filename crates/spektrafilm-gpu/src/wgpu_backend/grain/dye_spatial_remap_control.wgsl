// KNOWN-BAD control of G3: conservative remap with the overlap weights left
// un-normalized (no division by the cell width). A device run of this shader
// must fail the same independent oracle comparison that accepts
// `dye_spatial_remap.wgsl`, and must reproduce the CPU's `RemapNorm::Raw`
// control values. Keep the body otherwise byte-identical to the production
// pass so the altered law is the only difference.

const WORKGROUP: u32 = 256u;

@group(0) @binding(1) var<storage, read> source_layers: array<f32>;
@group(0) @binding(2) var<storage, read_write> cell_layers: array<f32>;

// Second rectangle's start relative to the cell start: `x * P - i * h`.
fn pixel_gap(x: i32, cell: i32) -> vec2<u32> {
    return fx_sub(
        fx_signed_mul(x, p.pitches.zw),
        fx_signed_mul(cell, p.pitches.xy));
}

// First source pixel of the conservative window of one cell. The estimate
// `cell * h / P` is split into an exact integer part and a small fractional
// part on the host, so its error stays far below one pixel; the window covers
// that error and the exact overlap weight zeroes the rest.
fn pixel_window_start(cell: i32) -> i32 {
    let frac = f32(cell) * p.fracs.x + f32(cell) * p.fracs.y;
    return cell * i32(p.ratios.x) + i32(floor(frac - 1.0));
}

@compute @workgroup_size(WORKGROUP)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    let index = gid.x + gid.y * grid.x * WORKGROUP;
    let ext_cols = p.cell_rect.z;
    let ext_rows = p.cell_rect.w;
    if (index >= ext_cols * ext_rows) { return; }
    let cx = index % ext_cols;
    let cy = index / ext_cols;
    let i = bitcast<i32>(p.cell_rect.x) + i32(cx);
    let j = bitcast<i32>(p.cell_rect.y) + i32(cy);
    let px0 = pixel_window_start(i);
    let py0 = pixel_window_start(j);
    let span = p.ratios.x + 5u;
    var acc: array<vec3<f32>, 3>;
    for (var wy = 0u; wy < span; wy = wy + 1u) {
        let y = py0 + i32(wy);
        // Broken law: raw overlap in um, not divided by the cell width.
        let weight_y = overlap_weight(pixel_gap(y, j), p.widths.w, 1.0);
        if (weight_y == 0.0) { continue; }
        let sy = u32(clamp(y, 0, i32(p.dims.y) - 1) - i32(p.source_rect.y));
        for (var wx = 0u; wx < span; wx = wx + 1u) {
            let x = px0 + i32(wx);
            // Broken law: raw overlap in um, not divided by the cell width.
            let weight_x = overlap_weight(pixel_gap(x, i), p.widths.w, 1.0);
            if (weight_x == 0.0) { continue; }
            let sx = u32(clamp(x, 0, i32(p.dims.x) - 1) - i32(p.source_rect.x));
            let weight = weight_x * weight_y;
            let base = (sy * p.source_rect.z + sx) * 9u;
            for (var c = 0u; c < 3u; c = c + 1u) {
                acc[c] = acc[c] + weight * vec3<f32>(
                    source_layers[base + c * 3u],
                    source_layers[base + c * 3u + 1u],
                    source_layers[base + c * 3u + 2u]);
            }
        }
    }
    let out = index * 9u;
    for (var c = 0u; c < 3u; c = c + 1u) {
        cell_layers[out + c * 3u] = acc[c].x;
        cell_layers[out + c * 3u + 1u] = acc[c].y;
        cell_layers[out + c * 3u + 2u] = acc[c].z;
    }
}
