// Ordered compensated sums persist across raster windows.
struct SetupState {
    sum_n: f32, sum_u: f32, sum_v: f32, nmin: f32,
    nmax: f32, offset0: f32, offset1: f32, offset2: f32,
    correction_n: f32, correction_u: f32, correction_v: f32,
    prefix0: u32, prefix1: u32, rank0: u32, rank1: u32, bit: u32,
    chunk_n: f32, chunk_u: f32, chunk_v: f32,
    chunk_correction_n: f32, chunk_correction_u: f32, chunk_correction_v: f32,
}
@group(0) @binding(1) var<storage, read> input: array<f32>;
@group(0) @binding(4) var<storage, read_write> s: SetupState;
// One 65536-bin histogram per rank, accumulated over all windows.
@group(0) @binding(6) var<storage, read_write> hist: array<u32, 131072>;

fn order_key(value: f32) -> u32 {
    let bits = bitcast<u32>(value);
    if (bits & 0x80000000u) != 0u { return ~bits; }
    return bits | 0x80000000u;
}
fn key_value(key: u32) -> f32 {
    if (key & 0x80000000u) != 0u { return bitcast<f32>(key & 0x7fffffffu); }
    return bitcast<f32>(~key);
}
fn neutral(pixel: u32) -> f32 {
    return ((input[pixel * 3u] + input[pixel * 3u + 1u]) + input[pixel * 3u + 2u]) / 3.0;
}
// Compensated accumulation is bounded to 1024 pixels per chunk; chunk totals
// are themselves accumulated with compensation across the whole frame.
fn compensated(sum: f32, correction: f32, value: f32) -> vec2<f32> {
    let next = sum + value;
    var error: f32;
    if abs(sum) >= abs(value) { error = (sum - next) + value; }
    else { error = (value - next) + sum; }
    return vec2<f32>(next, correction + error);
}
fn flush_chunk() {
    let n = compensated(s.sum_n, s.correction_n, s.chunk_n + s.chunk_correction_n);
    let u = compensated(s.sum_u, s.correction_u, s.chunk_u + s.chunk_correction_u);
    let v = compensated(s.sum_v, s.correction_v, s.chunk_v + s.chunk_correction_v);
    s.sum_n = n.x; s.correction_n = n.y;
    s.sum_u = u.x; s.correction_u = u.y;
    s.sum_v = v.x; s.correction_v = v.y;
    s.chunk_n = 0.0; s.chunk_u = 0.0; s.chunk_v = 0.0;
    s.chunk_correction_n = 0.0; s.chunk_correction_u = 0.0; s.chunk_correction_v = 0.0;
}
@compute @workgroup_size(1)
fn main() {
    // header: bounded count, operation, global first pixel, whole-frame count.
    switch p.header.y {
        case 0u: {
            for (var i = 0u; i < p.header.x; i++) {
                let d = vec3<f32>(input[3u*i], input[3u*i+1u], input[3u*i+2u]);
                if !finite32(d.x) || !finite32(d.y) || !finite32(d.z) { atomicOr(&status, 2u); continue; }
                let x = vec3<f32>(estimate(d.x,0u), estimate(d.y,1u), estimate(d.z,2u));
                if !finite32(x.x) || !finite32(x.y) || !finite32(x.z) { atomicOr(&status, 1u); continue; }
                let mean_x = ((x.x + x.y) + x.z) / 3.0;
                let r = d - vec3<f32>(neutral_density(mean_x,0u), neutral_density(mean_x,1u), neutral_density(mean_x,2u));
                let u = (r.x - r.y) / 1.4142135623730951;
                let v = ((r.x + r.y) - 2.0*r.z) / 2.449489742783178;
                let n = neutral(i);
                let n_sum = compensated(s.chunk_n, s.chunk_correction_n, n);
                let u_sum = compensated(s.chunk_u, s.chunk_correction_u, u);
                let v_sum = compensated(s.chunk_v, s.chunk_correction_v, v);
                s.chunk_n = n_sum.x; s.chunk_correction_n = n_sum.y;
                s.chunk_u = u_sum.x; s.chunk_correction_u = u_sum.y;
                s.chunk_v = v_sum.x; s.chunk_correction_v = v_sum.y;
                if ((p.header.z + i) & 1023u) == 1023u { flush_chunk(); }
                if p.header.z + i == 0u { s.nmin = n; s.nmax = n; }
                else { s.nmin = min(s.nmin,n); s.nmax = max(s.nmax,n); }
            }
        }
        case 1u: {
            // Two 16-bit digits select both exact binary32 order statistics.
            // The first digit has no prefix; avoid shifting a u32 by 32.
            let first = s.bit == 16u;
            for (var i = 0u; i < p.header.x; i++) {
                let key = order_key(neutral(i));
                let digit = (key >> s.bit) & 65535u;
                if first || (key >> 16u) == (s.prefix0 >> 16u) { hist[digit] += 1u; }
                if first || (key >> 16u) == (s.prefix1 >> 16u) { hist[65536u + digit] += 1u; }
            }
        }
        case 2u: {
            flush_chunk();
            // Compute the percentile rank in integers to avoid f32 rounding for
            // large frames; the next sample is used even at an integral rank.
            s.rank0 = (p.header.w - 1u) / 20u;
            s.rank1 = min(s.rank0 + 1u, p.header.w - 1u);
            s.prefix0 = 0u; s.prefix1 = 0u; s.bit = 16u;
            for (var i = 0u; i < 131072u; i++) { hist[i] = 0u; }
            if !finite32(s.sum_n + s.correction_n) || !finite32(s.sum_u + s.correction_u) || !finite32(s.sum_v + s.correction_v) { atomicOr(&status, 4u); }
        }
        case 3u: {
            var cum0 = 0u; var digit0 = 65535u;
            for (var d = 0u; d < 65536u; d++) {
                let c = hist[d];
                if s.rank0 >= cum0 + c { cum0 += c; } else { digit0 = d; s.rank0 -= cum0; break; }
            }
            var cum1 = 0u; var digit1 = 65535u;
            for (var d = 0u; d < 65536u; d++) {
                let c = hist[65536u + d];
                if s.rank1 >= cum1 + c { cum1 += c; } else { digit1 = d; s.rank1 -= cum1; break; }
            }
            s.prefix0 |= digit0 << s.bit;
            s.prefix1 |= digit1 << s.bit;
            for (var i = 0u; i < 131072u; i++) { hist[i] = 0u; }
            if s.bit >= 16u { s.bit -= 16u; }
        }
        case 4u: {
            let fraction = f32((p.header.w - 1u) % 20u) / 20.0;
            let low = key_value(s.prefix0);
            let high = key_value(s.prefix1);
            let n5 = low + fraction * (high - low);
            var w = vec3<f32>(0.33, 0.30, 0.35);
            if s.nmax - n5 >= 1.8 { w = vec3<f32>(0.30, 0.35, 0.29); }
            let count = f32(p.header.w);
            let en = ((w.x * ((s.sum_n + s.correction_n)/count) + w.y*s.nmax) + w.z*s.nmin) / ((w.x+w.y)+w.z);
            let dx = 0.75 * (p.setup.x - en) / p.setup.y;
            let cast_u = (s.sum_u + s.correction_u) / count;
            let cast_v = (s.sum_v + s.correction_v) / count;
            let weight = max(1.0 - 40.0*(cast_u*cast_u + cast_v*cast_v), 0.2);
            let u = -0.9*weight*cast_u/p.setup.y;
            let v = -0.9*weight*cast_v/p.setup.y;
            s.offset0 = dx + u/1.4142135623730951 + v/2.449489742783178;
            s.offset1 = dx - u/1.4142135623730951 + v/2.449489742783178;
            s.offset2 = dx - 2.0*v/2.449489742783178;
            if !finite32(s.offset0) || !finite32(s.offset1) || !finite32(s.offset2) { atomicOr(&status, 8u); }
        }
        default: {}
    }
}
