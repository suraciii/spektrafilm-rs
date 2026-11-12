// Coordinate-addressed value-noise field for the Dye grain law.
//
// The four outputs are the shared field and the three record-specific fields.
// Every output is unit variance; the apply pass mixes them with sqrt(rho) and
// sqrt(1-rho), applies the density taper, and draws the conditional count.
//
// The hash, octave sizes and weights match the CPU reference. Coordinates are
// global source pixels, so overlapping bands reproduce the same field.

struct P {
    w: u32,
    h: u32,
    seed: u32,
    ox: u32,
    oy: u32,
    monochrome: u32,
    pad0: u32,
    pad1: u32,
    base_px: f32,
    field_scale: f32,
    wa: f32,
    wb: f32,
    wc: f32,
    norm: f32,
};
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var<storage, read_write> shared_field: array<f32>;
@group(0) @binding(2) var<storage, read_write> own_r: array<f32>;
@group(0) @binding(3) var<storage, read_write> own_g: array<f32>;
@group(0) @binding(4) var<storage, read_write> own_b: array<f32>;

fn hash(x0: u32, y0: u32, seed: u32, octave: u32) -> f32 {
    var x = x0 * 0x9e3779b1u
        + y0 * 0x85ebca6bu
        + seed * 0xc2b2ae35u
        + octave * 0x27d4eb2fu;
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

fn value_noise(x: f32, y: f32, seed: u32, octave: u32) -> f32 {
    let ix = floor(x);
    let iy = floor(y);
    let fx = smoothstep01(x - ix);
    let fy = smoothstep01(y - iy);
    let x0 = bitcast<u32>(i32(ix));
    let y0 = bitcast<u32>(i32(iy));
    let x1 = bitcast<u32>(i32(ix) + 1);
    let y1 = bitcast<u32>(i32(iy) + 1);
    let a = hash(x0, y0, seed, octave);
    let b = hash(x1, y0, seed, octave);
    let c = hash(x0, y1, seed, octave);
    let d = hash(x1, y1, seed, octave);
    let top = a + (b - a) * fx;
    let bottom = c + (d - c) * fx;
    return top + (bottom - top) * fy;
}

fn field(x: f32, y: f32, stream: u32) -> f32 {
    let seed = p.seed ^ (stream * 0x9e3779b9u);
    let weights = array<f32, 3>(p.wa, p.wb, p.wc);
    var sum = 0.0;
    for (var octave = 0u; octave < 3u; octave++) {
        let weight = weights[octave];
        let size = p.base_px / f32(1u << octave);
        let offset = f32(octave) * 37.0;
        sum += weight * value_noise(x / size + offset, y / size + offset, seed, octave);
    }
    return sum / p.norm * p.field_scale;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= p.w || y >= p.h) { return; }
    let i = y * p.w + x;
    let gx = f32(x + p.ox);
    let gy = f32(y + p.oy);
    shared_field[i] = field(gx, gy, 0u);
    own_r[i] = field(gx, gy, 1u);
    own_g[i] = field(gx, gy, select(2u, 1u, p.monochrome != 0u));
    own_b[i] = field(gx, gy, select(3u, 1u, p.monochrome != 0u));
}
