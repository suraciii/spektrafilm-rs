// Parsed LittleCMS stages. Concatenate f64_math.wgsl before this module.
// Curve-cache preparation (one invocation per curve occurrence) must complete
// before any optimizer or image dispatch. No profile-dependent sampling is host work.
struct Params { n: u32, mode: u32, stage_count: u32, pad: u32 }
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read> words: array<u32>;
@group(0) @binding(3) var<storage, read> values: array<f64>;
@group(0) @binding(4) var<storage, read_write> scratch: array<u32>;
@group(0) @binding(5) var<storage, read_write> output: array<f32>;
@group(0) @binding(6) var<storage, read_write> bpc_coefficients: array<f64>;

const icc_eval_cache_start: u32 = 262144u;
const icc_eval_cache_stride: u32 = 8192u;

fn icc_eval_saturate(v: f64) -> u32 {
    let d = v + 0.5;
    if d <= 0.0 { return 0u; }
    if d >= 65535.0 { return 65535u; }
    return u32(d);
}

fn icc_eval_clamp(v: f32) -> f32 {
    if v < 1.0e-9 || v != v { return 0.0; }
    return min(v, 1.0);
}

// C pow, unlike the signed-power utility, does not extend noninteger powers
// to negative arguments. Preserve integer powers and negative zero here.
fn icc_eval_pow(x: f64, y: f64) -> f64 {
    if y == 0.0 || x == 1.0 { return f64(1.0); }
    if x == 0.0 {
        if y > 0.0 { return x; }
        return 1.0 / abs(x);
    }
    if x < 0.0 {
        if floor(y) != y { return (x - x) / (x - x); }
        let result = sf64_exp(y * sf64_log(-x));
        if abs(y) < 9007199254740992.0 && (abs(y) - 2.0 * floor(abs(y) / 2.0)) == 1.0 {
            return -result;
        }
        return result;
    }
    return sf64_exp(y * sf64_log(x));
}

// cmsgamma.c DefaultEvalParametricFn, including its deliberately asymmetric
// discontinuity and near-zero rules. Values remain double until stage output.
fn icc_eval_parametric(t: i32, p: u32, r: f64) -> f64 {
    let g = values[p];
    // Param arrays in the parsed ABI always have ten entries.
    let a = values[p + 1u];
    let b = values[p + 2u];
    let c = values[p + 3u];
    let d = values[p + 4u];
    let e = values[p + 5u];
    let f = values[p + 6u];
    switch t {
        case 1, -1: {
            if r < 0.0 { if abs(g - 1.0) < 0.0001 { return r; } return f64(0.0); }
            if t == -1 {
                if abs(g) < 0.0001 { return f64(f32(1.0e22)); }
                return icc_eval_pow(r, 1.0 / g);
            }
            return icc_eval_pow(r, g);
        }
        case 2: {
            if abs(a) < 0.0001 { return f64(0.0); }
            if r >= -b / a { let v = a * r + b; if v > 0.0 { return icc_eval_pow(v, g); } }
            return f64(0.0);
        }
        case -2: {
            if abs(g) < 0.0001 || abs(a) < 0.0001 || r < 0.0 { return f64(0.0); }
            return max((icc_eval_pow(r, 1.0 / g) - b) / a, f64(0.0));
        }
        case 3: {
            if abs(a) < 0.0001 { return f64(0.0); }
            if r >= max(-b / a, f64(0.0)) {
                let v = a * r + b;
                if v > 0.0 { return icc_eval_pow(v, g) + c; }
                return f64(0.0);
            }
            return c;
        }
        case -3: {
            if abs(g) < 0.0001 || abs(a) < 0.0001 { return f64(0.0); }
            if r >= c {
                if r - c > 0.0 { return (icc_eval_pow(r - c, 1.0 / g) - b) / a; }
                return f64(0.0);
            }
            return -b / a;
        }
        case 4: {
            if r < d { return r * c; }
            let v = a * r + b;
            if v > 0.0 { return icc_eval_pow(v, g); }
            return f64(0.0);
        }
        case -4: {
            let v = a * d + b;
            var disc = f64(0.0);
            if v >= 0.0 { disc = icc_eval_pow(v, g); }
            if r >= disc {
                if abs(g) < 0.0001 || abs(a) < 0.0001 { return f64(0.0); }
                return (icc_eval_pow(r, 1.0 / g) - b) / a;
            }
            if abs(c) < 0.0001 { return f64(0.0); }
            return r / c;
        }
        case 5: {
            if r < d { return r * c + f; }
            let v = a * r + b;
            if v > 0.0 { return icc_eval_pow(v, g) + e; }
            return e;
        }
        case -5: {
            if r >= c * d + f {
                if r - e < 0.0 || abs(g) < 0.0001 || abs(a) < 0.0001 { return f64(0.0); }
                return (icc_eval_pow(r - e, 1.0 / g) - b) / a;
            }
            if abs(c) < 0.0001 { return f64(0.0); }
            return (r - f) / c;
        }
        case 6: {
            let v = a * r + b;
            if g == 1.0 { return v + c; }
            if v < 0.0 { return c; }
            return icc_eval_pow(v, g) + c;
        }
        case -6: {
            if abs(g) < 0.0001 || abs(a) < 0.0001 || r - c < 0.0 { return f64(0.0); }
            return (icc_eval_pow(r - c, 1.0 / g) - b) / a;
        }
        case 7: {
            let v = b * icc_eval_pow(r, g) + c;
            if v <= 0.0 { return d; }
            return a * (sf64_log(v) / 2.3025850929940456840179914546844) + d;
        }
        case -7: {
            if abs(g) < 0.0001 || abs(a) < 0.0001 || abs(b) < 0.0001 { return f64(0.0); }
            return icc_eval_pow((icc_eval_pow(f64(10.0), (r - d) / a) - c) / b, 1.0 / g);
        }
        case 8: { return g * icc_eval_pow(a, b * r + c) + d; }
        case -8: {
            let v = r - d;
            if v < 0.0 || abs(g) < 0.0001 || abs(b) < 0.0001 { return f64(0.0); }
            return (sf64_log(v / g) / sf64_log(a) - c) / b;
        }
        case 108: {
            if abs(g) < 0.0001 { return f64(0.0); }
            return icc_eval_pow(1.0 - icc_eval_pow(1.0 - r, 1.0 / g), 1.0 / g);
        }
        case -108: { return 1.0 - icc_eval_pow(1.0 - icc_eval_pow(r, g), g); }
        case 109, -109: {
            let correction = 0.5 / (1.0 / (1.0 + sf64_exp(-g)) - 0.5);
            if t == 109 {
                return correction * (1.0 / (1.0 + sf64_exp(-g * (2.0 * r - 1.0))) - 0.5) + 0.5;
            }
            let v = (r - 0.5) / correction;
            return (-sf64_log(1.0 / (v + 0.5) - 1.0) / g + 1.0) / 2.0;
        }
        default: { return f64(0.0); }
    }
}

fn icc_eval_analytic_inverse(offset: u32) -> bool {
    return words[offset] != 0u && words[offset + 1u] == 1u
        && bitcast<i32>(words[words[offset + 4u]]) > 0;
}

fn icc_eval_segmented(offset: u32, r: f64, inverse: bool) -> f64 {
    let count = words[offset + 1u];
    let start = words[offset + 4u];
    for (var j = count; j > 0u; j -= 1u) {
        let s = start + (j - 1u) * 6u;
        let x0 = values[words[s + 1u]];
        let x1 = values[words[s + 2u]];
        if (r > x0 && r <= x1) || inverse {
            let t = bitcast<i32>(words[s]);
            if t != 0 {
                let result = icc_eval_parametric(select(t, -t, inverse), words[s + 3u], r);
                // EvalSegmentedFn maps either sign of infinity to PLUS_INF.
                if abs(result) > 1.7976931348623157e308 { return f64(f32(1.0e22)); }
                return result;
            }
            let n = words[s + 5u];
            let table = words[s + 4u];
            // C first casts the numerator to float, then divides by double.
            let v = icc_eval_clamp(f32(f64(f32(r - x0)) / (x1 - x0)));
            if v == 1.0 || n == 1u { return values[table + n - 1u]; }
            let pos = v * f32(n - 1u);
            let lo = u32(floor(pos));
            let hi = u32(ceil(pos));
            let a = f32(values[table + lo]);
            let b = f32(values[table + hi]);
            return f64(a + (b - a) * (pos - f32(lo)));
        }
    }
    return f64(f32(-1.0e22));
}

// Auxiliary black-point stages follow the main descriptor range. They are not
// applied to image samples, but their authored curves need the same device cache.
fn icc_eval_descriptor_count() -> u32 {
    var count = words[0];
    for (var s = 0u; s < words[0]; s += 1u) {
        let d = words[1] + s * 4u;
        if words[d] == 9u {
            let payload = words[d + 3u];
            count = max(count, words[payload + 1u] + words[payload + 2u]);
        }
    }
    return count;
}

fn icc_eval_curve_count() -> u32 {
    var count = 0u;
    for (var s = 0u; s < icc_eval_descriptor_count(); s += 1u) {
        let d = words[1] + s * 4u;
        if words[d] == 1u { count += words[d + 2u]; }
    }
    return count;
}

fn icc_eval_curve_offset(ordinal: u32) -> u32 {
    var remaining = ordinal;
    for (var s = 0u; s < icc_eval_descriptor_count(); s += 1u) {
        let d = words[1] + s * 4u;
        if words[d] == 1u {
            let n = words[d + 2u];
            if remaining < n { return words[words[d + 3u] + remaining]; }
            remaining -= n;
        }
    }
    return 0u;
}

fn icc_eval_curve_cache(offset: u32) -> u32 {
    let count = icc_eval_curve_count();
    for (var i = 0u; i < count; i += 1u) {
        if icc_eval_curve_offset(i) == offset { return icc_eval_cache_start + icc_eval_cache_stride * i; }
    }
    return icc_eval_cache_start;
}

fn icc_eval_forward_count(offset: u32) -> u32 {
    if words[offset + 3u] != 0u { return words[offset + 3u]; }
    if words[offset + 1u] == 1u {
        let s = words[offset + 4u];
        if bitcast<i32>(words[s]) == 1 && abs(values[words[s + 3u]] - 1.0) < 0.001 { return 2u; }
    }
    return 4096u;
}

fn icc_eval_forward_value(offset: u32, cache: u32, index: u32) -> u32 {
    if words[offset + 3u] != 0u { return u32(values[words[offset + 2u] + index]); }
    return scratch[cache + index];
}

fn eval_curve_table_count(offset: u32) -> u32 {
    if words[offset] == 0u { return icc_eval_forward_count(offset); }
    // cmsBuildParametricToneCurve uses 4096 for every negative parametric type.
    return 4096u;
}

fn eval_curve_table_value(offset: u32, index: u32) -> u32 {
    let cache = icc_eval_curve_cache(offset);
    if words[offset] != 0u { return scratch[cache + 4096u + index]; }
    return icc_eval_forward_value(offset, cache, index);
}

// Sequential per curve: cmsReverseToneCurveEx intentionally carries its last
// slope/intercept through samples for which GetInterval finds no interval.
fn icc_eval_prepare_curve(ordinal: u32) {
    if ordinal >= icc_eval_curve_count() { return; }
    let offset = icc_eval_curve_offset(ordinal);
    let cache = icc_eval_cache_start + icc_eval_cache_stride * ordinal;
    let n = icc_eval_forward_count(offset);
    if words[offset + 3u] == 0u {
        for (var i = 0u; i < n; i += 1u) {
            scratch[cache + i] = icc_eval_saturate(icc_eval_segmented(offset, f64(i) / f64(n - 1u), false) * 65535.0);
        }
    }
    if words[offset] == 0u { return; }
    if icc_eval_analytic_inverse(offset) {
        for (var i = 0u; i < 4096u; i += 1u) {
            scratch[cache + 4096u + i] = icc_eval_saturate(icc_eval_segmented(offset, f64(i) / 4095.0, true) * 65535.0);
        }
        return;
    }
    let first = icc_eval_forward_value(offset, cache, 0u);
    let last = icc_eval_forward_value(offset, cache, n - 1u);
    let ascending_search = first < last;
    let ascending = first <= last;
    var a = f64(0.0);
    var b = f64(0.0);
    for (var i = 0u; i < 4096u; i += 1u) {
        let y = f64(i) * 65535.0 / 4095.0;
        var interval = -1;
        for (var k = 0u; k + 1u < n; k += 1u) {
            let j = select(k, n - 2u - k, ascending_search);
            let x1 = f64(icc_eval_forward_value(offset, cache, j));
            let x2 = f64(icc_eval_forward_value(offset, cache, j + 1u));
            if y >= min(x1, x2) && y <= max(x1, x2) { interval = i32(j); break; }
        }
        if interval >= 0 {
            let j = u32(interval);
            let x1 = f64(icc_eval_forward_value(offset, cache, j));
            let x2 = f64(icc_eval_forward_value(offset, cache, j + 1u));
            let y1 = f64(j) * 65535.0 / f64(n - 1u);
            let y2 = f64(j + 1u) * 65535.0 / f64(n - 1u);
            if x1 == x2 {
                scratch[cache + 4096u + i] = icc_eval_saturate(select(y1, y2, ascending));
                continue;
            }
            a = (y2 - y1) / (x2 - x1);
            b = y2 - a * x2;
        }
        scratch[cache + 4096u + i] = icc_eval_saturate(a * y + b);
    }
}

fn icc_eval_lerp16(rest: u32, low: u32, high: u32) -> u32 {
    // Unsigned arithmetic is intentional: LCMS multiplies the signed difference
    // as uint32 and takes the low 16 bits after its rounding shift.
    return (low + (((high - low) * rest + 32768u) >> 16u)) & 65535u;
}

fn eval_curve16(offset: u32, v: u32) -> u32 {
    let n = eval_curve_table_count(offset);
    if v == 65535u || n == 1u { return eval_curve_table_value(offset, n - 1u); }
    let product = (n - 1u) * v;
    let fixed = product + (product + 32767u) / 65535u;
    let cell = fixed >> 16u;
    return icc_eval_lerp16(fixed & 65535u, eval_curve_table_value(offset, cell), eval_curve_table_value(offset, cell + 1u));
}

fn eval_curve(offset: u32, v: f32) -> f32 {
    if icc_eval_analytic_inverse(offset) { return f32(icc_eval_segmented(offset, f64(v), true)); }
    if words[offset] != 0u || words[offset + 1u] == 0u {
        return f32(f64(eval_curve16(offset, icc_eval_saturate(f64(v) * 65535.0))) / 65535.0);
    }
    return f32(icc_eval_segmented(offset, f64(v), false));
}

// Select the same tetrahedron as LCMS, retaining coefficient order by axis.
fn icc_eval_tetra_coeff(corners: array<f32, 8>, r: vec3<f32>) -> vec4<f32> {
    let c = corners;
    let c0 = c[0];
    if r.x >= r.y && r.y >= r.z { return vec4(c0, c[4]-c0, c[6]-c[4], c[7]-c[6]); }
    if r.x >= r.z && r.z >= r.y { return vec4(c0, c[4]-c0, c[7]-c[5], c[5]-c[4]); }
    if r.z >= r.x && r.x >= r.y { return vec4(c0, c[5]-c[1], c[7]-c[5], c[1]-c0); }
    if r.y >= r.x && r.x >= r.z { return vec4(c0, c[6]-c[2], c[2]-c0, c[7]-c[6]); }
    if r.y >= r.z && r.z >= r.x { return vec4(c0, c[7]-c[3], c[2]-c0, c[3]-c[2]); }
    return vec4(c0, c[7]-c[3], c[3]-c[1], c[1]-c0);
}

fn icc_eval_tetra16(corners: array<u32, 8>, r: vec3<u32>) -> u32 {
    var local_corners = corners;
    // Every authored u16 value and difference is exactly representable in f32.
    // Only coefficient selection uses float; all fixed-point products remain int.
    var c: array<f32, 8>;
    for (var i = 0u; i < 8u; i += 1u) { c[i] = f32(local_corners[i]); }
    let coeff = icc_eval_tetra_coeff(c, vec3<f32>(r));
    let rest = i32(coeff.y) * i32(r.x) + i32(coeff.z) * i32(r.y) + i32(coeff.w) * i32(r.z) + 32769;
    return u32(i32(coeff.x) + ((rest + (rest >> 16u)) >> 16u)) & 65535u;
}

fn icc_eval_clut(v: array<f32, 16>, payload: u32, inputs: u32, outputs: u32) -> array<f32, 16> {
    var local_input = v;
    let table = words[payload];
    let is_float = (words[payload + 1u] & 1u) != 0u;
    // Native >3D interpolation always uses tetrahedral in the last three axes.
    let trilinear = inputs == 3u && (words[payload + 1u] & 256u) != 0u;
    var low: array<u32, 16>;
    var step: array<u32, 16>;
    var fraction: array<f32, 16>;
    var fixed_fraction: array<u32, 16>;
    var stride = outputs;
    for (var j = inputs; j > 0u; j -= 1u) {
        let d = j - 1u;
        let samples = words[payload + 2u + d];
        if is_float {
            let clamped = icc_eval_clamp(local_input[d]);
            let pos = clamped * f32(samples - 1u);
            let cell = u32(floor(pos));
            low[d] = cell * stride;
            step[d] = select(stride, 0u, clamped >= 1.0 || samples == 1u);
            fraction[d] = pos - f32(cell);
        } else {
            let input = icc_eval_saturate(f64(local_input[d]) * 65535.0);
            let product = input * (samples - 1u);
            let pos = product + (product + 32767u) / 65535u;
            low[d] = (pos >> 16u) * stride;
            step[d] = select(stride, 0u, input == 65535u || samples == 1u);
            fixed_fraction[d] = pos & 65535u;
        }
        stride *= samples;
    }
    let tail = min(inputs, 3u);
    let prefix = inputs - tail;
    var result: array<f32, 16>;
    // An explicit depth-first reduction replaces native C recursive evaluators.
    // It stores one pending result per axis, never the exponential corner set.
    for (var channel = 0u; channel < outputs; channel += 1u) {
        var pending_float: array<f32, 16>;
        var pending_fixed: array<u32, 16>;
        for (var leaf = 0u; leaf < (1u << prefix); leaf += 1u) {
            var base = table + channel;
            for (var d = 0u; d < prefix; d += 1u) {
                base += low[d] + ((leaf >> (prefix - 1u - d)) & 1u) * step[d];
            }
            var cf: array<f32, 8>;
            var ci: array<u32, 8>;
            for (var corner = 0u; corner < (1u << tail); corner += 1u) {
                var index = base;
                for (var t = 0u; t < tail; t += 1u) {
                    let d = prefix + t;
                    index += low[d] + ((corner >> (tail - 1u - t)) & 1u) * step[d];
                }
                if is_float { cf[corner] = f32(values[index]); }
                else { ci[corner] = u32(values[index]); }
            }
            var rf: f32 = 0.0;
            var ri = 0u;
            if tail == 3u && !trilinear {
                if is_float {
                    let r = vec3(fraction[prefix], fraction[prefix + 1u], fraction[prefix + 2u]);
                    let c = icc_eval_tetra_coeff(cf, r);
                    rf = c.x + c.y * r.x + c.z * r.y + c.w * r.z;
                } else {
                    ri = icc_eval_tetra16(ci, vec3(fixed_fraction[prefix], fixed_fraction[prefix + 1u], fixed_fraction[prefix + 2u]));
                }
            } else {
                // Native bilinear/trilinear reduces x, then y, then z (not z first).
                for (var axis = 0u; axis < tail; axis += 1u) {
                    let span = 1u << (tail - 1u - axis);
                    for (var c = 0u; c < span; c += 1u) {
                        if is_float { cf[c] = cf[c] + (cf[c + span] - cf[c]) * fraction[prefix + axis]; }
                        else { ci[c] = icc_eval_lerp16(fixed_fraction[prefix + axis], ci[c], ci[c + span]); }
                    }
                }
                rf = cf[0];
                ri = ci[0];
            }
            var bits = leaf;
            var level = 0u;
            loop {
                if (bits & 1u) == 0u {
                    pending_float[level] = rf;
                    pending_fixed[level] = ri;
                    break;
                }
                let d = prefix - 1u - level;
                if is_float { rf = pending_float[level] + (rf - pending_float[level]) * fraction[d]; }
                else { ri = icc_eval_lerp16(fixed_fraction[d], pending_fixed[level], ri); }
                bits >>= 1u;
                level += 1u;
            }
        }
        if is_float { result[channel] = pending_float[prefix]; }
        else { result[channel] = f32(f64(pending_fixed[prefix]) / 65535.0); }
    }
    return result;
}

fn icc_eval_lab_f(t: f64) -> f64 {
    let ratio = f64(24.0) / f64(116.0);
    let limit = ratio * ratio * ratio;
    if t <= limit { return (f64(841.0) / f64(108.0)) * t + (f64(16.0) / f64(116.0)); }
    return icc_eval_pow(t, f64(1.0) / f64(3.0));
}

fn icc_eval_lab_inverse(t: f64) -> f64 {
    if t <= f64(24.0) / f64(116.0) { return (f64(108.0) / f64(841.0)) * (t - (f64(16.0) / f64(116.0))); }
    return t * t * t;
}

fn icc_eval_bpc_coefficient(index: u32) -> f64 {
    return bpc_coefficients[index];
}

fn icc_eval_store_bpc(index: u32, value: f64) {
    bpc_coefficients[index] = value;
}

fn icc_eval_bpc_empty() -> bool {
    var difference = f64(0.0);
    for (var i = 0u; i < 3u; i += 1u) { difference += abs(icc_eval_bpc_coefficient(i) - 1.0); }
    for (var i = 3u; i < 6u; i += 1u) { difference += abs(icc_eval_bpc_coefficient(i)); }
    return difference < 0.002;
}

fn eval_pipeline_range(v: array<f32, 16>, first: u32, end: u32) -> array<f32, 16> {
    var current = v;
    for (var stage = first; stage < end; stage += 1u) {
        let descriptor = words[1] + 4u * stage;
        let kind = words[descriptor];
        let inputs = words[descriptor + 1u];
        let outputs = words[descriptor + 2u];
        let payload = words[descriptor + 3u];
        var next = current;
        switch kind {
            case 1u: {
                for (var c = 0u; c < outputs; c += 1u) { next[c] = eval_curve(words[payload + c], current[c]); }
            }
            case 2u: {
                let matrix = words[payload];
                let offset = words[payload + 1u];
                // An empty BPC layer is absent in the native pipeline, so its
                // neighboring matrices are preoptimized before float evaluation.
                if inputs == 3u && outputs == 3u && offset == 0xffffffffu {
                    var fused: array<f64, 9>;
                    for (var i = 0u; i < 9u; i += 1u) { fused[i] = values[matrix + i]; }
                    var cursor = stage + 1u;
                    var joined = false;
                    loop {
                        if cursor >= end { break; }
                        let candidate = words[1] + cursor * 4u;
                        let candidate_kind = words[candidate];
                        if candidate_kind == 0u || (candidate_kind == 9u && icc_eval_bpc_empty()) {
                            cursor += 1u;
                            continue;
                        }
                        if candidate_kind != 2u || words[candidate + 1u] != 3u || words[candidate + 2u] != 3u { break; }
                        let candidate_payload = words[candidate + 3u];
                        if words[candidate_payload + 1u] != 0xffffffffu { break; }
                        let second = words[candidate_payload];
                        var product: array<f64, 9>;
                        for (var row = 0u; row < 3u; row += 1u) {
                            for (var col = 0u; col < 3u; col += 1u) {
                                product[row * 3u + col] = values[second + row * 3u] * fused[col]
                                    + values[second + row * 3u + 1u] * fused[3u + col]
                                    + values[second + row * 3u + 2u] * fused[6u + col];
                            }
                        }
                        fused = product;
                        joined = true;
                        stage = cursor;
                        cursor += 1u;
                    }
                    if joined {
                        var identity = true;
                        for (var row = 0u; row < 3u; row += 1u) {
                            for (var col = 0u; col < 3u; col += 1u) {
                                if abs(fused[row * 3u + col] - select(f64(0.0), f64(1.0), row == col)) >= f64(f32(0.00001)) { identity = false; }
                            }
                        }
                        if !identity {
                            for (var row = 0u; row < 3u; row += 1u) {
                                var sum = f64(0.0);
                                for (var col = 0u; col < 3u; col += 1u) { sum += f64(current[col]) * fused[row * 3u + col]; }
                                next[row] = f32(sum);
                            }
                        }
                        current = next;
                        continue;
                    }
                }
                for (var row = 0u; row < outputs; row += 1u) {
                    var sum = f64(0.0);
                    for (var col = 0u; col < inputs; col += 1u) { sum += f64(current[col]) * values[matrix + row * inputs + col]; }
                    if offset != 0xffffffffu { sum += values[offset + row]; }
                    next[row] = f32(sum);
                }
            }
            case 3u: { next = icc_eval_clut(current, payload, inputs, outputs); }
            case 4u: {
                let adjust = f64(65535.0) / f64(32768.0);
                let x = icc_eval_lab_f(f64(current[0]) * adjust / 0.9642);
                let y = icc_eval_lab_f(f64(current[1]) * adjust);
                let z = icc_eval_lab_f(f64(current[2]) * adjust / 0.8249);
                next[0] = f32((116.0 * y - 16.0) / 100.0);
                next[1] = f32((500.0 * (x - y) + 128.0) / 255.0);
                next[2] = f32((200.0 * (y - z) + 128.0) / 255.0);
            }
            case 5u: {
                let l = f64(current[0]) * 100.0;
                let a = f64(current[1]) * 255.0 - 128.0;
                let b = f64(current[2]) * 255.0 - 128.0;
                let y = (l + 16.0) / 116.0;
                let x = y + 0.002 * a;
                let z = y - 0.005 * b;
                let adjust = f64(65535.0) / f64(32768.0);
                next[0] = f32(icc_eval_lab_inverse(x) * 0.9642 / adjust);
                next[1] = f32(icc_eval_lab_inverse(y) / adjust);
                next[2] = f32(icc_eval_lab_inverse(z) * 0.8249 / adjust);
            }
            case 6u: {
                for (var c = 0u; c < outputs; c += 1u) { next[c] = f32(f64(current[c]) * (f64(65535.0) / f64(65280.0))); }
            }
            case 7u: {
                for (var c = 0u; c < outputs; c += 1u) { next[c] = f32(f64(current[c]) * (f64(65280.0) / f64(65535.0))); }
            }
            case 8u: {
                for (var c = 0u; c < outputs; c += 1u) { if current[c] < 0.0 { next[c] = 0.0; } }
            }
            case 9u: {
                if !icc_eval_bpc_empty() {
                    for (var c = 0u; c < 3u; c += 1u) {
                        next[c] = f32(f64(current[c]) * icc_eval_bpc_coefficient(c) + icc_eval_bpc_coefficient(c + 3u));
                    }
                }
            }
            default: {}
        }
        current = next;
    }
    return current;
}

fn eval_pipeline(v: array<f32, 16>) -> array<f32, 16> {
    return eval_pipeline_range(v, 0u, words[0]);
}

// A separate dispatch after all curve caches: auxiliary destination input
// stages evaluate the authored darkest colorant without host numeric work.
fn icc_eval_prepare_bpc() {
    for (var i = 0u; i < 3u; i += 1u) {
        icc_eval_store_bpc(i, f64(1.0));
        icc_eval_store_bpc(i + 3u, f64(0.0));
    }
    for (var s = 0u; s < words[0]; s += 1u) {
        let descriptor = words[1] + s * 4u;
        if words[descriptor] != 9u { continue; }
        let payload = words[descriptor + 3u];
        var black = vec3<f64>(0.00336, 0.0034731, 0.00287);
        if words[payload] == 1u {
            var zero: array<f32, 16>;
            let first = words[payload + 1u];
            let xyz = eval_pipeline_range(zero, first, first + words[payload + 2u]);
            let adjust = f64(65535.0) / f64(32768.0);
            let x = icc_eval_lab_f(f64(xyz[0]) * adjust / 0.9642);
            let y = icc_eval_lab_f(f64(xyz[1]) * adjust);
            let z = icc_eval_lab_f(f64(xyz[2]) * adjust / 0.8249);
            // Native XYZ2Lab writes float before PackLabDoubleFromFloat.
            var l = f64(f32((116.0 * y - 16.0) / 100.0)) * 100.0;
            let a = f64(f32((500.0 * (x - y) + 128.0) / 255.0)) * 255.0 - 128.0;
            let b = f64(f32((200.0 * (y - z) + 128.0) / 255.0)) * 255.0 - 128.0;
            if l > 95.0 || l < 0.0 { l = 0.0; }
            else if l > 50.0 { l = 50.0; }
            let ly = (l + 16.0) / 116.0;
            black = vec3<f64>(icc_eval_lab_inverse(ly + 0.002 * a) * 0.9642,
                icc_eval_lab_inverse(ly), icc_eval_lab_inverse(ly - 0.005 * b) * 0.8249);
        }
        let white = vec3<f64>(0.9642, 1.0, 0.8249);
        for (var i = 0u; i < 3u; i += 1u) {
            // Retain ComputeBlackPointCompensation's operation order, including
            // the multiply/divide in b rather than substituting black directly.
            let denominator = -white[i];
            icc_eval_store_bpc(i, (black[i] - white[i]) / denominator);
            icc_eval_store_bpc(i + 3u, (-white[i] * black[i] / denominator) / (f64(65535.0) / f64(32768.0))); 
        }
        return;
    }
}
