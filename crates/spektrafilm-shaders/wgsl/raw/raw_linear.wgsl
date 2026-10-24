// LibRaw 0.22.2 misc_demosaic.cpp: border_interpolate / lin_interpolate.
// Only measured channels are read; interpolated channels never feed this pass.
struct DemosaicPass { colors: u32, stage: u32, top: u32, left: u32 };
@group(0) @binding(0) var<uniform> frame: RawFrame;
@group(0) @binding(1) var<uniform> settings: DemosaicPass;
@group(0) @binding(2) var<storage, read_write> image: array<u32>;
@group(0) @binding(3) var<storage, read> cfa: array<u32>;
fn dm_color(y: i32, x: i32) -> u32 {
    let yy = u32((y % i32(frame.cfa_rows) + i32(frame.cfa_rows)) % i32(frame.cfa_rows));
    let xx = u32((x % i32(frame.cfa_columns) + i32(frame.cfa_columns)) % i32(frame.cfa_columns));
    return raw_color_from_code(cfa[yy * frame.cfa_columns + xx], frame);
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let p = gid.x + gid.y * grid.x * 256u;
    if (p >= raw_pixel_count(frame)) { return; }
    let y = i32(p / frame.width); let x = i32(p % frame.width);
    let f = dm_color(y, x);
    let border = i32(settings.stage);
    let edge = x < border || y < border || x >= i32(frame.width)-border || y >= i32(frame.height)-border;
    // stage==0 is the weighted linear interior; other stages fill the border.
    if (settings.stage == 0u && (x == 0 || y == 0 || x == i32(frame.width)-1 || y == i32(frame.height)-1)) { return; }
    if (settings.stage != 0u && !edge) { return; }
    var sums: array<u32,4>; var weights: array<u32,4>;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let yy = y+dy; let xx = x+dx;
            if (yy < 0 || xx < 0 || yy >= i32(frame.height) || xx >= i32(frame.width)) { continue; }
            let c = dm_color(yy,xx);
            var weight = 1u;
            if (settings.stage == 0u) {
                if (c == f) { continue; }
                weight = 1u << (u32(dy == 0) + u32(dx == 0));
            }
            sums[c] += image[(u32(yy)*frame.width+u32(xx))*4u+c] * weight;
            weights[c] += weight;
        }
    }
    for (var c=0u; c<settings.colors; c++) {
        if (c == f) { continue; }
        if (settings.stage == 0u) {
            var multiplier = 0u;
            if (weights[c] != 0u) { multiplier = 256u / weights[c]; }
            image[p*4u+c] = (sums[c]*multiplier)>>8u;
        } else if (weights[c] != 0u) {
            image[p*4u+c] = sums[c]/weights[c];
        }
    }
}
