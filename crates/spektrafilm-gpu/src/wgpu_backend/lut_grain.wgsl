struct Params {
    n: u32,
    width: u32,
    height: u32,
    seed: u32,
    first_pixel: u32,
    amount: f32,
    size: f32,
    shadows: f32,
    midtones: f32,
    highlights: f32,
    chroma: f32,
    pad0: f32,
    pad1: f32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;
@group(0) @binding(2) var<storage, read_write> output: array<f32>;

fn hash_noise(value_in: u32) -> f32 {
    var value = value_in;
    value ^= value >> 16u;
    value = value * 0x7feb352du;
    value ^= value >> 15u;
    value = value * 0x846ca68bu;
    value ^= value >> 16u;
    return f32(value) / f32(0xffffffffu) * 2.0 - 1.0;
}

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let xi = i32(floor(x));
    let yi = i32(floor(y));
    let fx = x - floor(x);
    let fy = y - floor(y);
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let h00 = hash_noise(seed ^ bitcast<u32>(xi) ^ (bitcast<u32>(yi) * 0x9e3779b9u));
    let h10 = hash_noise(seed ^ bitcast<u32>(xi + 1) ^ (bitcast<u32>(yi) * 0x9e3779b9u));
    let h01 = hash_noise(seed ^ bitcast<u32>(xi) ^ (bitcast<u32>(yi + 1) * 0x9e3779b9u));
    let h11 = hash_noise(seed ^ bitcast<u32>(xi + 1) ^ (bitcast<u32>(yi + 1) * 0x9e3779b9u));
    let nx0 = h00 * (1.0 - sx) + h10 * sx;
    let nx1 = h01 * (1.0 - sx) + h11 * sx;
    return nx0 * (1.0 - sy) + nx1 * sy;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = gid.x + gid.y * grid.x * 256u;
    if index >= p.n { return; }
    let base = index * 3u;
    let rgb = vec3<f32>(input[base], input[base + 1u], input[base + 2u]);
    let luma = clamp(0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z, 0.0, 1.0);
    let weight = p.shadows * (1.0 - luma) * (1.0 - luma)
        + p.midtones * 4.0 * luma * (1.0 - luma)
        + p.highlights * luma * luma;
    let scale = max(p.size, 1.0e-30);
    let global = p.first_pixel + index;
    let x = f32(global % p.width) / scale;
    let y = f32(global / p.width) / scale;
    let mono = value_noise(x, y, p.seed);
    let chroma = value_noise(x, y, p.seed ^ 0x61a5f5d3u) * p.chroma;
    let amount = p.amount * weight;
    let result = vec3<f32>(
        rgb.x + amount * (mono + chroma * 0.5),
        rgb.y + amount * mono,
        rgb.z + amount * (mono - chroma * 0.5),
    );
    output[base] = result.x;
    output[base + 1u] = result.y;
    output[base + 2u] = result.z;
}
