// Identical density interpolation arithmetic with one owner per global
// interval. Coefficient pages overlap one row; image windows have no halos.
struct Params {
    n_pixels: u32,
    total_rows: u32,
    first_row: u32,
    page_rows: u32,
    uniform_grid: u32,
    gamma_inv: f32,
    first_x: f32,
    last_x: f32,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> log_raw: array<f32>;
@group(0) @binding(2) var<storage, read> log_exposure: array<f32>;
@group(0) @binding(3) var<storage, read> density_curves: array<f32>;
@group(0) @binding(4) var<storage, read_write> output: array<f32>;
fn channel_value(index: u32, channel: u32) {
    let xq = log_raw[index];
    let first = params.first_row;
    let end = first + params.page_rows;
    let xa0 = params.first_x * params.gamma_inv;
    let xa_last = params.last_x * params.gamma_inv;
    if xq <= xa0 || params.total_rows == 1u {
        if first == 0u { output[index] = density_curves[channel]; }
        return;
    }
    if xq >= xa_last {
        if end == params.total_rows {
            output[index] = density_curves[(params.page_rows - 1u) * 3u + channel];
        }
        return;
    }
    if xq != xq {
        if first == 0u { output[index] = xq; }
        return;
    }
    if params.uniform_grid != 0u {
        let step = (xa_last - xa0) / f32(params.total_rows - 1u);
        let t = (xq - xa0) / step;
        let i = min(u32(floor(t)), params.total_rows - 2u);
        if i < first || i + 1u >= end { return; }
        let local = i - first;
        let frac = t - f32(i);
        let y0 = density_curves[local * 3u + channel];
        let y1 = density_curves[(local + 1u) * 3u + channel];
        output[index] = y0 + frac * (y1 - y0);
        return;
    }
    let page_first = log_exposure[0] * params.gamma_inv;
    let page_last = log_exposure[params.page_rows - 1u] * params.gamma_inv;
    if xq < page_first || xq >= page_last { return; }
    var lo = 0u;
    var hi = params.page_rows;
    loop {
        if lo + 1u >= hi { break; }
        let mid = (lo + hi) / 2u;
        if log_exposure[mid] * params.gamma_inv <= xq { lo = mid; }
        else { hi = mid; }
    }
    let xa_lo = log_exposure[lo] * params.gamma_inv;
    let xa_hi = log_exposure[lo + 1u] * params.gamma_inv;
    let dx = xa_hi - xa_lo;
    let frac = select(0.0, (xq - xa_lo) / dx, dx != 0.0);
    let y0 = density_curves[lo * 3u + channel];
    let y1 = density_curves[(lo + 1u) * 3u + channel];
    output[index] = y0 + frac * (y1 - y0);
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= params.n_pixels { return; }
    for (var c = 0u; c < 3u; c += 1u) { channel_value(pixel * 3u + c, c); }
}
