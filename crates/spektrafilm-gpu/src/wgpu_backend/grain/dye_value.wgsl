// Coordinate-addressed value-noise field for the Dye grain law.
//
// The four outputs are the shared field and the three record-specific fields.
// Every output is unit variance; the apply pass mixes them with sqrt(rho) and
// sqrt(1-rho), applies the density taper, and draws the conditional count.
//
// The hash and cell size match the CPU reference. Coordinates are global
// source pixels, so overlapping bands reproduce the same field.

struct P {
    w: u32,
    h: u32,
    seed: u32,
    ox: u32,
    oy: u32,
    monochrome: u32,
    pad0: u32,
    pad1: u32,
    layer_count: u32,
    base_l0: f32,
    base_l1: f32,
    base_l2: f32,
    field_scale: f32,
    fine_scale: f32,
    fine_weight: f32,
    pad2: u32,
};
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read_write> shared_field: array<f32>;
@group(0) @binding(2) var<storage, read_write> own_r: array<f32>;
@group(0) @binding(3) var<storage, read_write> own_g: array<f32>;
@group(0) @binding(4) var<storage, read_write> own_b: array<f32>;

fn hash(x0: u32, y0: u32, seed: u32) -> f32 {
    var x = x0 * 0x9e3779b1u
        + y0 * 0x85ebca6bu
        + seed * 0xc2b2ae35u;
    x = x ^ (x >> 15u);
    x = x * 0x2c1b3c6du;
    x = x ^ (x >> 12u);
    x = x * 0x297a2d39u;
    x = x ^ (x >> 15u);
    return f32(x) * (2.0 / 4294967295.0) - 1.0;
}

fn smoothstep01(t: f32) -> f32 {
    let v = clamp(t, 0.0, 1.0);
    return v * v * (3.0 - 2.0 * v);
}

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let ix = floor(x);
    let iy = floor(y);
    let fx = smoothstep01(x - ix);
    let fy = smoothstep01(y - iy);
    let x0 = bitcast<u32>(i32(ix));
    let y0 = bitcast<u32>(i32(iy));
    let x1 = bitcast<u32>(i32(ix) + 1);
    let y1 = bitcast<u32>(i32(iy) + 1);
    let a = hash(x0, y0, seed);
    let b = hash(x1, y0, seed);
    let c = hash(x0, y1, seed);
    let d = hash(x1, y1, seed);
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    return top + (bottom - top) * fy;
}

fn field_one(x: f32, y: f32, stream: u32, base_px: f32) -> f32 {
    let seed = p.seed ^ (stream * 0x9e3779b9u);
    return value_noise(x / base_px, y / base_px, seed) * p.field_scale;
}

fn field(x: f32, y: f32, stream: u32, base_px: f32) -> f32 {
    let base = field_one(x, y, stream, base_px);
    if (p.fine_weight <= 0.0) { return base; }
    let fine = field_one(x, y, stream ^ 0x40000000u, base_px * p.fine_scale);
    return base * sqrt(1.0 - p.fine_weight) + fine * sqrt(p.fine_weight);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= p.w || y >= p.h) { return; }
    let i = y * p.w + x;
    let n = p.w * p.h;
    let gx = f32(x + p.ox);
    let gy = f32(y + p.oy);
    let layers = max(p.layer_count, 1u);
    for (var layer = 0u; layer < layers; layer++) {
        let offset = layer * n + i;
        let layer_stream = layer * 4u;
        let base_px = select(select(p.base_l0, p.base_l1, layer == 1u), p.base_l2, layer == 2u);
        shared_field[offset] = field(gx, gy, 0u, base_px);
        own_r[offset] = field(gx, gy, layer_stream + 1u, base_px);
        own_g[offset] = field(gx, gy, select(layer_stream + 2u, layer_stream + 1u, p.monochrome != 0u), base_px);
        own_b[offset] = field(gx, gy, select(layer_stream + 3u, layer_stream + 1u, p.monochrome != 0u), base_px);
    }
}
