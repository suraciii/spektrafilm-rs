// Reads the developed film density in a band out to the readout grid.
//
// The law lives on the film lattice; a readout cell is a film-coordinate rectangle.
// Finer than the lattice it interpolates its centre, coarser it integrates the cells
// it covers, which is what makes a cropped readout and a full-frame read agree where
// they overlap.

struct P {
    cols: u32,
    rows: u32,
    window_x: u32,
    window_y: u32,
    window_w: u32,
    window_h: u32,
    grid_x: u32,
    grid_y: u32,
    source_pitch: f32,
    pitch_x: f32,
    pitch_y: f32,
    origin_x: f32,
    origin_y: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
};
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read> src: array<f32>;
@group(0) @binding(2) var<storage, read_write> dst: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= p.cols || y >= p.rows) { return; }
    let gx = f32(p.grid_x + x);
    let gy = f32(p.grid_y + y);
    let fx0 = (p.origin_x + gx * p.pitch_x) / p.source_pitch;
    let fy0 = (p.origin_y + gy * p.pitch_y) / p.source_pitch;
    let fx1 = fx0 + p.pitch_x / p.source_pitch;
    let fy1 = fy0 + p.pitch_y / p.source_pitch;
    let i = (y * p.cols + x) * 3u;
    let span_x = fx1 - fx0;
    let span_y = fy1 - fy0;
    var out = vec3<f32>(0.0);
    if (span_x >= 1.0 || span_y >= 1.0) {
        var acc = vec3<f32>(0.0);
        var weight = 0.0;
        var sy = i32(floor(fy0));
        let sy_end = i32(ceil(fy1));
        while (sy < sy_end) {
            let wy = max(min(fy1, f32(sy) + 1.0) - max(fy0, f32(sy)), 0.0);
            var sx = i32(floor(fx0));
            let sx_end = i32(ceil(fx1));
            while (sx < sx_end) {
                let wx = max(min(fx1, f32(sx) + 1.0) - max(fx0, f32(sx)), 0.0);
                let w = wx * wy;
                if (w > 0.0) {
                    let cx = clamp(u32(sx) - p.window_x, 0u, p.window_w - 1u);
                    let cy = clamp(u32(sy) - p.window_y, 0u, p.window_h - 1u);
                    let j = (cy * p.window_w + cx) * 3u;
                    acc += w * vec3<f32>(src[j], src[j + 1u], src[j + 2u]);
                    weight += w;
                }
                sx += 1;
            }
            sy += 1;
        }
        out = acc / max(weight, 1e-6);
    } else {
        let sx = clamp((fx0 + fx1) * 0.5 - 0.5, 0.0, f32(p.window_w) - 1.0);
        let sy = clamp((fy0 + fy1) * 0.5 - 0.5, 0.0, f32(p.window_h) - 1.0);
        let x0 = u32(floor(sx));
        let y0 = u32(floor(sy));
        let x1 = min(x0 + 1u, p.window_w - 1u);
        let y1 = min(y0 + 1u, p.window_h - 1u);
        let wx = sx - f32(x0);
        let wy = sy - f32(y0);
        let a = vec3<f32>(src[(y0 * p.window_w + x0) * 3u], src[(y0 * p.window_w + x0) * 3u + 1u], src[(y0 * p.window_w + x0) * 3u + 2u]);
        let b = vec3<f32>(src[(y0 * p.window_w + x1) * 3u], src[(y0 * p.window_w + x1) * 3u + 1u], src[(y0 * p.window_w + x1) * 3u + 2u]);
        let c = vec3<f32>(src[(y1 * p.window_w + x0) * 3u], src[(y1 * p.window_w + x0) * 3u + 1u], src[(y1 * p.window_w + x0) * 3u + 2u]);
        let d = vec3<f32>(src[(y1 * p.window_w + x1) * 3u], src[(y1 * p.window_w + x1) * 3u + 1u], src[(y1 * p.window_w + x1) * 3u + 2u]);
        let top = a + (b - a) * wx;
        let bottom = c + (d - c) * wx;
        out = top + (bottom - top) * wy;
    }
    dst[i] = out.r;
    dst[i + 1u] = out.g;
    dst[i + 2u] = out.b;
}
