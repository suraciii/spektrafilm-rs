fn ds_add(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let s = a.x + b.x;
    let v = s - a.x;
    let e = (a.x - (s - v)) + (b.x - v) + a.y + b.y;
    let hi = s + e;
    return vec2<f32>(hi, e - (hi - s));
}
fn ds_mul_int(a: vec2<f32>, b: f32) -> vec2<f32> {
    let p = a.x * b;
    // Dekker product residual, with exactly representable integer b.
    let ca = 4097.0 * a.x;
    let ah = ca - (ca - a.x);
    let al = a.x - ah;
    let cb = 4097.0 * b;
    let bh = cb - (cb - b);
    let bl = b - bh;
    let e = ((ah * bh - p) + ah * bl + al * bh) + al * bl + a.y * b;
    let hi = p + e;
    return vec2<f32>(hi, e - (hi - p));
}
fn shr_jam(a: vec2<u32>, n: u32) -> vec2<u32> {
    if (n == 0u) { return a; }
    if (n < 32u) {
        let lost = a.x << (32u - n);
        return vec2<u32>((a.x >> n) | (a.y << (32u - n)) | u32(lost != 0u), a.y >> n);
    }
    if (n == 32u) { return vec2<u32>(a.y | u32(a.x != 0u), 0u); }
    if (n < 64u) {
        let lost = a.x | (a.y << (64u - n));
        return vec2<u32>((a.y >> (n - 32u)) | u32(lost != 0u), 0u);
    }
    return vec2<u32>(u32((a.x | a.y) != 0u), 0u);
}
fn add64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let lo = a.x + b.x;
    return vec2<u32>(lo, a.y + b.y + u32(lo < a.x));
}
// Software binary64 positive addition, round-to-nearest ties-to-even. This
// preserves rc += step's cumulative rounding instead of replacing it by x*step.
fn add_double(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    if ((a.x | a.y) == 0u) { return b; }
    var large = a;
    var small = b;
    if (a.y < b.y || (a.y == b.y && a.x < b.x)) { large = b; small = a; }
    var exponent = (large.y >> 20u) & 2047u;
    let small_exponent = (small.y >> 20u) & 2047u;
    let lm = vec2<u32>(large.x << 3u, ((large.y & 1048575u) | 1048576u) << 3u | (large.x >> 29u));
    let sm = vec2<u32>(small.x << 3u, ((small.y & 1048575u) | 1048576u) << 3u | (small.x >> 29u));
    var total = add64(lm, shr_jam(sm, exponent - small_exponent));
    if ((total.y & 16777216u) != 0u) { total = shr_jam(total, 1u); exponent++; }
    let round_bits = total.x & 7u;
    var mantissa = vec2<u32>((total.x >> 3u) | (total.y << 29u), total.y >> 3u);
    if (round_bits > 4u || (round_bits == 4u && (mantissa.x & 1u) != 0u)) {
        mantissa = add64(mantissa, vec2<u32>(1u, 0u));
        if ((mantissa.y & 2097152u) != 0u) { mantissa = shr_jam(mantissa, 1u); exponent++; }
    }
    return vec2<u32>(mantissa.x, (mantissa.y & 1048575u) | (exponent << 20u));
}
fn floor_double(a: vec2<u32>) -> u32 {
    let exponent = i32((a.y >> 20u) & 2047u) - 1023;
    if (exponent < 0) { return 0u; }
    let mantissa = vec2<u32>(a.x, (a.y & 1048575u) | 1048576u);
    let shift = u32(52 - exponent);
    if (shift >= 32u) { return mantissa.y >> (shift - 32u); }
    return (mantissa.x >> shift) | (mantissa.y << (32u - shift));
}
