// PCG XSL RR 128/64. Limbs are little-endian; arithmetic wraps modulo 2^128.
// Jump-ahead is on device, so blocks reproduce one uninterrupted NumPy stream.
struct Params { count: u32, first_pixel: u32, frame_count: u32, unused: u32 }
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read_write> random: array<f32>;
const INITIAL = vec4<u32>(0xa69561e3u, 0x09585eb7u, 0x5996452du, 0x1aa1b534u);
const MULTIPLIER = vec4<u32>(0x9fccf645u, 0x4385df64u, 0x1fc65da4u, 0x2360ed05u);
const INCREMENT = vec4<u32>(0x447873a9u, 0x588133bcu, 0x3af71a82u, 0x418ddadbu);
fn add128(a: vec4<u32>, b: vec4<u32>) -> vec4<u32> {
    var result = vec4<u32>(0u);
    var carry = 0u;
    for (var i = 0u; i < 4u; i++) {
        let sum = a[i] + b[i];
        let with_carry = sum + carry;
        carry = select(0u, 1u, sum < a[i] || with_carry < sum);
        result[i] = with_carry;
    }
    return result;
}
fn mul_wide(a: u32, b: u32) -> vec2<u32> {
    let a0 = a & 65535u;
    let b0 = b & 65535u;
    let t0 = a0 * b0;
    let t1 = (a >> 16u) * b0 + (t0 >> 16u);
    let t2 = a0 * (b >> 16u) + (t1 & 65535u);
    return vec2<u32>((t2 << 16u) + (t0 & 65535u), (a >> 16u) * (b >> 16u) + (t1 >> 16u) + (t2 >> 16u));
}
fn mul128(a: vec4<u32>, b: vec4<u32>) -> vec4<u32> {
    var result = vec4<u32>(0u);
    for (var i = 0u; i < 4u; i++) {
        for (var j = 0u; j < 4u - i; j++) {
            let product = mul_wide(a[i], b[j]);
            var term = vec4<u32>(0u);
            term[i + j] = product.x;
            if i + j + 1u < 4u { term[i + j + 1u] = product.y; }
            result = add128(result, term);
        }
    }
    return result;
}
fn advance(delta: u32) -> vec4<u32> {
    var d = delta;
    var multiplier = MULTIPLIER;
    var increment = INCREMENT;
    var state = INITIAL;
    loop {
        if d == 0u { break; }
        if (d & 1u) != 0u { state = add128(mul128(state, multiplier), increment); }
        increment = mul128(add128(multiplier, vec4<u32>(1u, 0u, 0u, 0u)), increment);
        multiplier = mul128(multiplier, multiplier);
        d >>= 1u;
    }
    return state;
}
fn output(state: vec4<u32>) -> vec2<u32> {
    let value = state.xy ^ state.zw;
    let rotation = state.w >> 26u;
    let low = select(value.x, value.y, rotation >= 32u);
    let high = select(value.y, value.x, rotation >= 32u);
    let shift = rotation & 31u;
    if shift == 0u { return vec2<u32>(low, high); }
    return vec2<u32>((low >> shift) | (high << (32u - shift)), (high >> shift) | (low << (32u - shift)));
}
fn fill_array(first: u32, stream_offset: u32, destination_offset: u32) {
    let frame_index = p.first_pixel + first;
    let low = frame_index + stream_offset;
    let high = select(0u, 1u, low < frame_index);
    // Dividing the 33-bit draw index by two produces a 32-bit PCG step.
    var state = advance(((low >> 1u) | (high << 31u)) + 1u);
    var words = output(state);
    var upper = (low & 1u) != 0u;
    let end = min(p.count - first, 256u);
    for (var i = 0u; i < end; i++) {
        let word = select(words.x, words.y, upper);
        random[destination_offset + first + i] = f32(word >> 8u) * (1.0 / 16777216.0);
        if upper {
            state = add128(mul128(state, MULTIPLIER), INCREMENT);
            words = output(state);
        }
        upper = !upper;
    }
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let block = id.x + id.y * 65535u * 256u;
    if block >= (p.count + 255u) / 256u { return; }
    let first = block * 256u;
    // Both page-local arrays are slices of the two full-frame arrays. Their
    // starts can have different word parity when the full frame has odd N.
    fill_array(first, 0u, 0u);
    fill_array(first, p.frame_count, p.count);
}
