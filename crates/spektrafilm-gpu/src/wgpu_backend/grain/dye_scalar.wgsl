// Scalar record G3/G4: exact source overlap and one certified Poisson draw.
// Concatenated after dye_count.wgsl and dye_spatial_params.wgsl.
struct RecordParams {
    seed: vec2<u32>,
    density_per_event: f32,
    expectation: u32,
}
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> cells: array<f32>;
@group(0) @binding(3) var<uniform> r: RecordParams;
@group(0) @binding(4) var<storage, read_write> status: atomic<u32>;
fn pixel_gap(x: i32, start: vec2<u32>) -> vec2<u32> {
    return fx_sub(fx_signed_mul(x, p.pitches.zw), start);
}
fn pixel_window_start(cell: i32) -> i32 {
    let frac = f32(cell) * p.fracs.x + f32(cell) * p.fracs.y;
    return cell * i32(p.ratios.x) + i32(floor(frac - 1.0));
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = gid.x + gid.y * grid.x * 256u;
    if (index >= p.cell_rect.z * p.cell_rect.w) { return; }
    let i = bitcast<i32>(p.cell_rect.x) + i32(index % p.cell_rect.z);
    let j = bitcast<i32>(p.cell_rect.y) + i32(index / p.cell_rect.z);
    let cell_x = fx_signed_mul(i, p.pitches.xy);
    let cell_y = fx_signed_mul(j, p.pitches.xy);
    let px0 = pixel_window_start(i);
    let py0 = pixel_window_start(j);
    let span = p.ratios.x + 5u;
    var density = 0.0;
    for (var wy = 0u; wy < span; wy++) {
        let y = py0 + i32(wy);
        let weight_y = overlap_weight(pixel_gap(y, cell_y), p.widths.w, p.widths.z);
        if (weight_y == 0.0) { continue; }
        let sy = u32(clamp(y, 0, i32(p.dims.y) - 1) - i32(p.source_rect.y));
        for (var wx = 0u; wx < span; wx++) {
            let x = px0 + i32(wx);
            let weight_x = overlap_weight(pixel_gap(x, cell_x), p.widths.w, p.widths.z);
            if (weight_x == 0.0) { continue; }
            let sx = u32(clamp(x, 0, i32(p.dims.x) - 1) - i32(p.source_rect.x));
            let value = source[(sy * p.source_rect.z + sx) * 3u];
            if ((bitcast<u32>(value) & 0x7fffffffu) >= 0x7f800000u || value < 0.0) {
                atomicOr(&status, 32u);
                continue;
            }
            let contribution = weight_x * weight_y * value;
            if (value > 0.0 && contribution == 0.0) { atomicOr(&status, 2u); }
            density += contribution;
        }
    }
    let rate = density / r.density_per_event;
    var result = density;
    if ((bitcast<u32>(rate) & 0x7fffffffu) >= 0x7f800000u || rate > MAX_RATE || (density > 0.0 && rate == 0.0)) {
        atomicOr(&status, 2u);
        result = 0.0;
    } else if (r.expectation == 0u && rate > 0.0) {
        let draw = count_of(rate, address(i, j, 0u, r.seed.x, r.seed.y));
        if (draw.x == DECISION_FAILED) {
            atomicOr(&status, 4u);
            result = 0.0;
        } else {
            result = f32(draw.x) * r.density_per_event;
        }
    }
    cells[index] = result;
}
