// One ordered accumulator is carried across bounded raster windows. No
// tree reduction: changing physical pages must not change CPU raster-order sums.
struct SetupState {
    sum_n: f64, sum_u: f64, sum_v: f64, nmin: f64,
    nmax: f64, offset0: f64, offset1: f64, offset2: f64,
    prefix0: u64, prefix1: u64,
    rank0: u32, rank1: u32, zeros0: u32, zeros1: u32,
    bit: u32, pad0: u32, pad1: u32, pad2: u32,
}
@group(0) @binding(1) var<storage, read> input: array<f32>;
@group(0) @binding(4) var<storage, read_write> s: SetupState;
// One 256-bin digit histogram per rank. Counts are integers, so the result does
// not depend on window order: eight 8-bit digits replace sixty-four 1-bit rounds.
@group(0) @binding(6) var<storage, read_write> hist: array<u32, 131072>;

fn order_key(value: f64) -> u64 {
    let bits = bitcast<u64>(value);
    let sign_bit = u64(1u) << 63u;
    if (bits & sign_bit) != u64(0u) { return ~bits; }
    return bits | sign_bit;
}
fn key_value(key: u64) -> f64 {
    let sign_bit = u64(1u) << 63u;
    if (key & sign_bit) != u64(0u) { return bitcast<f64>(key & ~sign_bit); }
    return bitcast<f64>(~key);
}
fn neutral(pixel: u32) -> f64 {
    return ((f64(input[pixel * 3u]) + f64(input[pixel * 3u + 1u])) + f64(input[pixel * 3u + 2u])) / 3.0;
}
@compute @workgroup_size(1)
fn main() {
    // header: bounded count, operation, global first pixel, whole-frame count.
    switch p.header.y {
        case 0u: {
            for (var i = 0u; i < p.header.x; i++) {
                let d = vec3<f64>(f64(input[3u*i]), f64(input[3u*i+1u]), f64(input[3u*i+2u]));
                if !finite64(d.x) || !finite64(d.y) || !finite64(d.z) { atomicOr(&status, 2u); continue; }
                let x = vec3<f64>(estimate(d.x,0u), estimate(d.y,1u), estimate(d.z,2u));
                if !finite64(x.x) || !finite64(x.y) || !finite64(x.z) { atomicOr(&status, 1u); continue; }
                let mean_x = ((x.x + x.y) + x.z) / 3.0;
                let r = d - vec3<f64>(neutral_density(mean_x,0u), neutral_density(mean_x,1u), neutral_density(mean_x,2u));
                let u = (r.x - r.y) / 1.4142135623730951;
                let v = ((r.x + r.y) - 2.0*r.z) / 2.449489742783178;
                let n = neutral(i);
                s.sum_n += n;
                s.sum_u += u;
                s.sum_v += v;
                if p.header.z + i == 0u { s.nmin = n; s.nmax = n; }
                else { s.nmin = min(s.nmin,n); s.nmax = max(s.nmax,n); }
            }
        }
        case 1u: {
            // Exact two order statistics by binary64 total-order radix selection,
            // eight bits per round. Counts and chosen prefixes never leave the
            // device. Both ranks are processed together, including duplicate
            // samples and signed zero.
            let hi0 = s.prefix0 >> s.bit;
            let hi1 = s.prefix1 >> s.bit;
            for (var i = 0u; i < p.header.x; i++) {
                let key = order_key(neutral(i));
                let hi = key >> s.bit;
                if hi == hi0 { hist[u32((key >> s.bit) & u64(65535u))] = hist[u32((key >> s.bit) & u64(65535u))] + 1u; }
                if hi == hi1 { hist[65536u + u32((key >> s.bit) & u64(65535u))] = hist[65536u + u32((key >> s.bit) & u64(65535u))] + 1u; }
            }
        }
        case 2u: {
            let position = f64(p.header.w - 1u) * 0.05;
            s.rank0 = u32(floor(position));
            // CPU reads the next tail value even when interpolation is zero.
            s.rank1 = min(s.rank0 + 1u, p.header.w - 1u);
            s.prefix0 = u64(0u); s.prefix1 = u64(0u);
            s.zeros0 = 0u; s.zeros1 = 0u; s.bit = 48u;
            for (var i = 0u; i < 131072u; i++) { hist[i] = 0u; }
            if !finite64(s.sum_n) || !finite64(s.sum_u) || !finite64(s.sum_v) { atomicOr(&status, 4u); }
        }
        case 3u: {
            // Walk the digit bins in ascending order and stop at the bin that
            // holds each rank, exactly as the one-bit form skipped zero groups.
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
            s.prefix0 |= u64(digit0) << s.bit;
            s.prefix1 |= u64(digit1) << s.bit;
            for (var i = 0u; i < 131072u; i++) { hist[i] = 0u; }
            if s.bit >= 16u { s.bit -= 16u; }
        }
        case 4u: {
            let position = f64(p.header.w - 1u) * 0.05;
            let low = key_value(s.prefix0);
            let high = key_value(s.prefix1);
            let n5 = low + (position - floor(position)) * (high - low);
            var w = vec3<f64>(0.33, 0.30, 0.35);
            if s.nmax - n5 >= 1.8 { w = vec3<f64>(0.30, 0.35, 0.29); }
            let count = f64(p.header.w);
            let en = ((w.x * (s.sum_n/count) + w.y*s.nmax) + w.z*s.nmin) / ((w.x+w.y)+w.z);
            let dx = 0.75 * (p.setup.x - en) / p.setup.y;
            let cast_u = s.sum_u / count;
            let cast_v = s.sum_v / count;
            let weight = max(f64(1.0) - f64(40.0)*(cast_u*cast_u + cast_v*cast_v), f64(0.2));
            let u = -0.9*weight*cast_u/p.setup.y;
            let v = -0.9*weight*cast_v/p.setup.y;
            s.offset0 = dx + u/1.4142135623730951 + v/2.449489742783178;
            s.offset1 = dx - u/1.4142135623730951 + v/2.449489742783178;
            s.offset2 = dx - 2.0*v/2.449489742783178;
            if !finite64(s.offset0) || !finite64(s.offset1) || !finite64(s.offset2) { atomicOr(&status, 8u); }
        }
        default: {}
    }
}
