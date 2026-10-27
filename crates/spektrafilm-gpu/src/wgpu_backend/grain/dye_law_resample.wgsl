// Fills a band window with the developed density read at the readout grid's cell
// centres. The source is the flat copy of the full-source raster; the grid may
// crop, scale, or both, so the mapping is bilinear with the host's own weights.

struct P {
    cols: u32,
    rows: u32,
    origin_x: u32,
    origin_y: u32,
    source_w: u32,
    source_h: u32,
    grid_w: u32,
    grid_h: u32,
    fx: f32,
    fy: f32,
    pad0: f32,
    pad1: f32,
};
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> src: array<f32>;
@group(0) @binding(2) var<storage, read_write> dst: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= p.cols || y >= p.rows) { return; }
    let gx = f32(p.origin_x + x);
    let gy = f32(p.origin_y + y);
    let sx = clamp((gx + 0.5) * p.fx - 0.5, 0.0, f32(p.source_w) - 1.0);
    let sy = clamp((gy + 0.5) * p.fy - 0.5, 0.0, f32(p.source_h) - 1.0);
    let x0 = u32(floor(sx));
    let y0 = u32(floor(sy));
    let x1 = min(x0 + 1u, p.source_w - 1u);
    let y1 = min(y0 + 1u, p.source_h - 1u);
    let wx = sx - f32(x0);
    let wy = sy - f32(y0);
    let i = (y * p.cols + x) * 3u;
    for (var c = 0u; c < 3u; c++) {
        let p00 = src[(y0 * p.source_w + x0) * 3u + c];
        let p10 = src[(y0 * p.source_w + x1) * 3u + c];
        let p01 = src[(y1 * p.source_w + x0) * 3u + c];
        let p11 = src[(y1 * p.source_w + x1) * 3u + c];
        let top = p00 + (p10 - p00) * wx;
        let bottom = p01 + (p11 - p01) * wx;
        dst[i + c] = top + (bottom - top) * wy;
    }
}
