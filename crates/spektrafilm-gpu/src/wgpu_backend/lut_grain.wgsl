struct Params {
    n: u32,
    width: u32,
    height: u32,
    seed_lo: u32,
    seed_hi: u32,
    first_pixel: u32,
    amount: f32,
    size: f32,
    shadows: f32,
    midtones: f32,
    highlights: f32,
    chroma: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
    pad3: f32,
}

// The LUT grain hash is the reference's 64-bit mix. WGSL has no required
// 64-bit integer type, so the shape is kept as two 32-bit limbs. The final
// 53-bit fraction is rounded through f32; image samples themselves are f32.

struct U64 {
    lo: u32,
    hi: u32,
}

fn mul32(a: u32, b: u32) -> U64 {
    let a0 = a & 0xffffu;
    let a1 = a >> 16u;
    let b0 = b & 0xffffu;
    let b1 = b >> 16u;
    let p0 = a0 * b0;
    let p1 = a0 * b1;
    let p2 = a1 * b0;
    let p3 = a1 * b1;
    let t0_lo = p0 & 0xffffu;
    let t0_hi = p0 >> 16u;
    let t1 = (p1 & 0xffffu) + (p2 & 0xffffu);
    let t1_lo = t1 & 0xffffu;
    let t1_hi = (p1 >> 16u) + (p2 >> 16u) + (t1 >> 16u);
    let sum1 = t0_hi + t1_lo;
    let mid_lo = sum1 & 0xffffu;
    let carry1 = sum1 >> 16u;
    let sum2 = t1_hi + (p3 & 0xffffu) + carry1;
    let mid_hi = sum2 & 0xffffu;
    let carry2 = sum2 >> 16u;
    return U64(
        t0_lo | (mid_lo << 16u),
        mid_hi | (((p3 >> 16u) + carry2) << 16u),
    );
}

fn mul64(a: U64, b: U64) -> U64 {
    let p0 = mul32(a.lo, b.lo);
    let p1 = mul32(a.lo, b.hi);
    let p2 = mul32(a.hi, b.lo);
    return U64(p0.lo, p0.hi + p1.lo + p2.lo);
}

fn xor64(a: U64, b: U64) -> U64 {
    return U64(a.lo ^ b.lo, a.hi ^ b.hi);
}

fn add64(a: U64, b: U64) -> U64 {
    let lo = a.lo + b.lo;
    let carry = select(0u, 1u, lo < a.lo);
    return U64(lo, a.hi + b.hi + carry);
}

fn shr33(a: U64) -> U64 {
    return U64(a.hi >> 1u, 0u);
}

fn shr11(a: U64) -> U64 {
    return U64((a.lo >> 11u) | (a.hi << 21u), a.hi >> 11u);
}

fn hash_noise(value_in: U64) -> f32 {
    var value = value_in;
    value = xor64(value, shr33(value));
    value = mul64(value, U64(0xed558ccdu, 0xff51afd7u));
    value = xor64(value, shr33(value));
    value = mul64(value, U64(0x1a85ec53u, 0xc4ceb9feu));
    value = xor64(value, shr33(value));
    let fraction = shr11(value);
    return (f32(fraction.hi) * 4294967296.0 + f32(fraction.lo)) / 9007199254740992.0 * 2.0 - 1.0;
}

fn value_noise(x: f32, y: f32, seed: U64) -> f32 {
    let xi = i32(floor(x));
    let yi = i32(floor(y));
    let fx = x - floor(x);
    let fy = y - floor(y);
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let y_term = mul64(U64(bitcast<u32>(yi), 0u), U64(0x7f4a7c15u, 0x9e3779b9u));
    let x0 = U64(bitcast<u32>(xi), 0u);
    let x1 = U64(bitcast<u32>(xi + 1), 0u);
    let y0 = y_term;
    let y1 = add64(y_term, U64(0x7f4a7c15u, 0x9e3779b9u));
    let h00 = hash_noise(xor64(xor64(seed, x0), y0));
    let h10 = hash_noise(xor64(xor64(seed, x1), y0));
    let h01 = hash_noise(xor64(xor64(seed, x0), y1));
    let h11 = hash_noise(xor64(xor64(seed, x1), y1));
    let nx0 = h00 * (1.0 - sx) + h10 * sx;
    let nx1 = h01 * (1.0 - sx) + h11 * sx;
    return nx0 * (1.0 - sy) + nx1 * sy;
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;
@group(0) @binding(2) var<storage, read_write> output: array<f32>;

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
    let mono = value_noise(x, y, U64(p.seed_lo, p.seed_hi));
    let chroma = value_noise(
        x,
        y,
        xor64(U64(p.seed_lo, p.seed_hi), U64(0x61a5f5d3u, 0x51ed270bu)),
    ) * p.chroma;
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
