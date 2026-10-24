// LCMS default RGB_8 optimizer, translated from cmsopt.c.
// Copyright (c) 1998-2026 Marti Maria Saguer
//
// Permission is hereby granted, free of charge, to any person obtaining
// a copy of this software and associated documentation files (the
// "Software"), to deal in the Software without restriction, including
// without limitation the rights to use, copy, modify, merge, publish,
// distribute, sublicense, and/or sell copies of the Software, and to
// permit persons to whom the Software is furnished to do so, subject to
// the following conditions:
//
// The above copyright notice and this permission notice shall be included
// in all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
// MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
// IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
// CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
// TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
// SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//
// Concatenate f64_math, viewer_icc_eval, this file, then viewer_icc_apply.
// All numeric tables are generated on the device.
//
// Scratch offsets are u32 words. The optimizer owns [0, 262144); evaluator
// caches follow it. Runtime dispatches prepare_main modes 6,7,0,1,2,3,4,5 in
// separate ordered dispatches, with workgroup counts ceil(curves/64),1,1,
// 257,1,64,562,1. Modes and tables are selected without host readback.
const ICC_OPT_FIRST: u32 = 64u;       // three 256-entry signed fixed14 tables
const ICC_OPT_SECOND: u32 = 832u;     // three 16385-entry widened byte tables
const ICC_OPT_RAMP: u32 = 50000u;     // three 4096-entry u16 gray/join tables
const ICC_OPT_INVERSE: u32 = 62288u;  // three 4096-entry u16 inverse tables
const ICC_OPT_CLUT: u32 = 74576u;     // 33^3 nodes, three interleaved u16s
// Header: mode (0 identity,1 joined,2 matrix,3 prelinearized,4 resampled),
// first/last shaper stage indices, matrix stage index, skip-identity-matrix.
// Header [16,28) contains nine matrix and three offset signed fixed14 words.
// Header [32,44) is reserved for evaluator BPC double coefficients.

fn icc_opt_word(value: f64) -> u32 {
    return u32(floor(clamp(value + f64(0.5), f64(0.0), f64(65535.0))));
}

fn icc_opt_byte(value: u32) -> u32 {
    return ((value * 65281u + 8388608u) >> 24u) & 255u;
}

fn icc_opt_fixed(value: u32) -> u32 {
    return value + (value + 32767u) / 65535u;
}

fn icc_opt_table16(base: u32, value: u32) -> u32 {
    if value == 65535u { return scratch[base + 4095u]; }
    let position = icc_opt_fixed(value * 4095u);
    let cell = position >> 16u;
    let rest = position & 65535u;
    let low = scratch[base + cell];
    let high = scratch[base + cell + 1u];
    // Unsigned wrapping arithmetic is intentional: LCMS LinearInterp casts
    // the signed table difference to u32 before multiplying.
    return ((((high - low) * rest + 32768u) >> 16u) + low) & 65535u;
}

fn icc_opt_stage(index: u32) -> u32 {
    return words[1u] + index * 4u;
}

fn icc_opt_curve(stage: u32, channel: u32) -> u32 {
    return words[words[icc_opt_stage(stage) + 3u] + channel];
}

fn icc_opt_join_eval(value: f32) -> array<f32, 16> {
    var input: array<f32, 16>;
    input[0] = value;
    input[1] = value;
    input[2] = value;
    if scratch[4u] != 0u {
        let first = scratch[1u];
        let last = scratch[2u];
        return eval_pipeline_range(eval_pipeline_range(input, first, first + 1u), last, last + 1u);
    }
    return eval_pipeline(input);
}

fn icc_opt_classify() {
    scratch[0u] = 3u;
    scratch[4u] = 0u;
    if params.stage_count == 0u {
        scratch[0u] = 0u;
        return;
    }
    var curves_only = true;
    var count = 0u;
    var selected_stages: array<u32, 4>;
    for (var i = 0u; i < params.stage_count; i++) {
        let kind = words[icc_opt_stage(i)];
        if kind == 9u && icc_eval_bpc_empty() { continue; }
        if count < 4u { selected_stages[count] = i; }
        count++;
        if kind != 1u { curves_only = false; }
    }
    if curves_only {
        scratch[0u] = 1u;
        return;
    }
    if count != 3u && count != 4u { return; }
    if words[icc_opt_stage(selected_stages[0u])] != 1u || words[icc_opt_stage(selected_stages[count - 1u])] != 1u { return; }
    for (var i = 1u; i < count - 1u; i++) {
        let stage = icc_opt_stage(selected_stages[i]);
        if words[stage] != 2u || words[stage + 1u] != 3u || words[stage + 2u] != 3u { return; }
    }
    let payload1 = words[icc_opt_stage(selected_stages[1u]) + 3u];
    let matrix1 = words[payload1];
    var offset = words[payload1 + 1u];
    var matrix: array<f64, 9>;
    for (var i = 0u; i < 9u; i++) { matrix[i] = values[matrix1 + i]; }
    if count == 4u {
        if offset != 0xffffffffu { return; }
        let payload2 = words[icc_opt_stage(selected_stages[2u]) + 3u];
        let matrix2 = words[payload2];
        offset = words[payload2 + 1u];
        var product: array<f64, 9>;
        for (var row = 0u; row < 3u; row++) {
            for (var column = 0u; column < 3u; column++) {
                product[row * 3u + column] = values[matrix2 + row * 3u] * matrix[column]
                    + values[matrix2 + row * 3u + 1u] * matrix[3u + column]
                    + values[matrix2 + row * 3u + 2u] * matrix[6u + column];
            }
        }
        matrix = product;
    }
    var identity = offset == 0xffffffffu;
    for (var row = 0u; row < 3u; row++) {
        for (var column = 0u; column < 3u; column++) {
            let i = row * 3u + column;
            let expected = select(f64(0.0), f64(1.0), row == column);
            if abs(matrix[i] - expected) >= f64(1.0) / f64(65535.0) { identity = false; }
            scratch[16u + i] = bitcast<u32>(i32(floor(matrix[i] * 16384.0lf + 0.5lf)));
        }
        var off: i32 = 0;
        if offset != 0xffffffffu { off = i32(floor(values[offset + row] * 16384.0lf + 0.5lf)); }
        scratch[25u + row] = bitcast<u32>(off);
    }
    scratch[1u] = selected_stages[0u];
    scratch[2u] = selected_stages[count - 1u];
    scratch[3u] = selected_stages[1u];
    scratch[0u] = select(2u, 1u, identity);
    scratch[4u] = u32(identity);
}

fn icc_opt_sample(index: u32) {
    let mode = scratch[0u];
    if mode == 2u {
        for (var channel = 0u; channel < 3u; channel++) {
            if index < 256u {
                let x = f32(f64(index) / 255.0lf);
                let y = eval_curve(icc_opt_curve(scratch[1u], channel), x);
                var fixed = 2147483647i;
                if y < 131072.0 { fixed = i32(floor(f64(y) * 16384.0lf + 0.5lf)); }
                scratch[ICC_OPT_FIRST + channel * 256u + index] = bitcast<u32>(fixed);
            }
            if index < 16385u {
                let y = eval_curve(icc_opt_curve(scratch[2u], channel), f32(index) / 16384.0);
                let word = icc_opt_word(f64(clamp(y, 0.0, 1.0)) * 65535.0lf);
                scratch[ICC_OPT_SECOND + channel * 16385u + index] = icc_opt_byte(word) * 257u;
            }
        }
        return;
    }
    if (mode != 1u && mode != 3u) || index >= 4096u { return; }
    var y = icc_opt_join_eval(f32(f64(index) / 4095.0lf));
    for (var channel = 0u; channel < 3u; channel++) {
        scratch[ICC_OPT_RAMP + channel * 4096u + index] = icc_opt_word(f64(y[channel]) * 65535.0lf);
    }
}

fn icc_opt_slope(base: u32) {
    let descending = scratch[base] > scratch[base + 4095u];
    let begin = select(f64(0.0), f64(65535.0), descending);
    let end = 65535.0lf - begin;
    var value = f64(scratch[base + 82u]);
    var slope = (value - begin) / 82.0lf;
    var beta = value - slope * 82.0lf;
    for (var i = 0u; i < 82u; i++) { scratch[base + i] = icc_opt_word(f64(i) * slope + beta); }
    value = f64(scratch[base + 4013u]);
    slope = (end - value) / 82.0lf;
    beta = value - slope * 4013.0lf;
    for (var i = 4013u; i < 4096u; i++) { scratch[base + i] = icc_opt_word(f64(i) * slope + beta); }
}

fn icc_opt_decide() {
    let mode = scratch[0u];
    if mode != 1u && mode != 3u { return; }
    var suitable = true;
    var linear = true;
    for (var channel = 0u; channel < 3u; channel++) {
        let base = ICC_OPT_RAMP + channel * 4096u;
        if mode == 3u { icc_opt_slope(base); }
        let descending = scratch[base] > scratch[base + 4095u];
        var zeros = 0u;
        var poles = 0u;
        for (var i = 0u; i < 4096u; i++) {
            let value = scratch[base + i];
            zeros += u32(value == 0u);
            poles += u32(value == 65535u);
            let identity = icc_opt_word(f64(i) * 65535.0lf / 4095.0lf);
            if abs(i32(value) - i32(identity)) > 15i { linear = false; }
            if i > 0u {
                let previous = i32(scratch[base + i - 1u]);
                let delta = i32(value) - previous;
                if (descending && delta > 2i) || (!descending && delta < -2i) { suitable = false; }
            }
        }
        if zeros > 204u || poles > 204u { suitable = false; }
    }
    if mode == 1u {
        if linear { scratch[0u] = 0u; }
        return;
    }
    // LCMS rejects linearization when the original final curves squeeze a
    // wide range to an endpoint. Inspect their own estimated/authored tables,
    // not the gray ramp (which has already undergone slope limiting).
    if params.stage_count > 0u {
        let last = params.stage_count - 1u;
        if words[icc_opt_stage(last)] == 1u {
            for (var channel = 0u; channel < 3u; channel++) {
                let curve = icc_opt_curve(last, channel);
                let count = eval_curve_table_count(curve);
                var zeros = 0u;
                var poles = 0u;
                for (var i = 0u; i < count; i++) {
                    let value = eval_curve_table_value(curve, i);
                    zeros += u32(value == 0u);
                    poles += u32(value == 65535u);
                }
                if (zeros != 1u || poles != 1u) && (zeros > count / 20u || poles > count / 20u) { suitable = false; }
            }
        }
    }
    if !suitable { scratch[0u] = 4u; }
}

fn icc_opt_reverse(index: u32) {
    if scratch[0u] != 3u || index >= 4096u { return; }
    let y = f64(index) * 65535.0lf / 4095.0lf;
    for (var channel = 0u; channel < 3u; channel++) {
        let base = ICC_OPT_RAMP + channel * 4096u;
        let ascending = scratch[base] < scratch[base + 4095u];
        // Slope-limited endpoints cover the complete interval, so every y
        // has an interval and LCMS's unmatched-interval a/b carry is unused.
        // Search direction and plateau endpoint choice match GetInterval.
        var interval = 0u;
        for (var step = 0u; step < 4095u; step++) {
            let j = select(step, 4094u - step, ascending);
            let lo = f64(scratch[base + j]);
            let hi = f64(scratch[base + j + 1u]);
            if y >= min(lo, hi) && y <= max(lo, hi) { interval = j; break; }
        }
        let x1 = f64(scratch[base + interval]);
        let x2 = f64(scratch[base + interval + 1u]);
        let y1 = f64(interval) * 65535.0lf / 4095.0lf;
        let y2 = f64(interval + 1u) * 65535.0lf / 4095.0lf;
        var value = select(y1, y2, ascending);
        if x1 != x2 {
            let a = (y2 - y1) / (x2 - x1);
            let b = y2 - a * x2;
            value = a * y + b;
        }
        scratch[ICC_OPT_INVERSE + channel * 4096u + index] = icc_opt_word(value);
    }
}

fn icc_opt_resample(index: u32) {
    let mode = scratch[0u];
    if (mode != 3u && mode != 4u) || index >= 35937u { return; }
    var input: array<f32, 16>;
    var coordinates = vec3<u32>(index / 1089u, (index / 33u) % 33u, index % 33u);
    for (var channel = 0u; channel < 3u; channel++) {
        let word = icc_opt_word(f64(coordinates[channel]) * 65535.0lf / 32.0lf);
        var value = f32(f64(word) / 65535.0lf);
        if mode == 3u {
            // A tabulated inverse tone curve's float evaluator quantizes
            // its f32 input to u16 before doing integer interpolation.
            let inverse_input = icc_opt_word(f64(value) * 65535.0lf);
            let inverse = icc_opt_table16(ICC_OPT_INVERSE + channel * 4096u, inverse_input);
            value = f32(f64(inverse) / 65535.0lf);
        }
        input[channel] = value;
    }
    var result = eval_pipeline(input);
    for (var channel = 0u; channel < 3u; channel++) {
        scratch[ICC_OPT_CLUT + index * 3u + channel] = icc_opt_word(f64(result[channel]) * 65535.0lf);
    }
}

// Six-tetrahedron integer interpolation, including LCMS's signed wrapping
// 0x8001 rounding correction. Used both by application and white repair.
fn icc_opt_tetra(input: vec3<u32>) -> vec3<u32> {
    var cell: vec3<u32>;
    var rest: vec3<i32>;
    var stride: vec3<u32>;
    let strides = vec3<u32>(3267u, 99u, 3u);
    for (var channel = 0u; channel < 3u; channel++) {
        let fixed = icc_opt_fixed(input[channel] * 32u);
        cell[channel] = fixed >> 16u;
        rest[channel] = i32(fixed & 65535u);
        stride[channel] = select(0u, strides[channel], rest[channel] != 0i);
    }
    let base = ICC_OPT_CLUT + cell.x * strides.x + cell.y * strides.y + cell.z * strides.z;
    var order = vec3<u32>(0u, 1u, 2u);
    if rest[order.x] < rest[order.y] { let t = order.x; order.x = order.y; order.y = t; }
    if rest[order.y] < rest[order.z] { let t = order.y; order.y = order.z; order.z = t; }
    if rest[order.x] < rest[order.y] { let t = order.x; order.x = order.y; order.y = t; }
    var result: vec3<u32>;
    for (var channel = 0u; channel < 3u; channel++) {
        let c0 = i32(scratch[base + channel]);
        var previous = c0;
        var offset = base + channel;
        var sum: i32 = 32769i;
        for (var i = 0u; i < 3u; i++) {
            let axis = order[i];
            offset += stride[axis];
            let current = i32(scratch[offset]);
            sum += (current - previous) * rest[axis];
            previous = current;
        }
        result[channel] = u32(c0 + ((sum + (sum >> 16u)) >> 16u)) & 65535u;
    }
    return result;
}

fn icc_opt_white_patch() {
    let mode = scratch[0u];
    if mode != 3u && mode != 4u { return; }
    // Current viewer uses perceptual intent and default flags: RGB white is
    // 65535 in every channel, with no absolute-intent white-fix suppression.
    var white = vec3<u32>(65535u);
    if mode == 3u {
        for (var channel = 0u; channel < 3u; channel++) {
            white[channel] = scratch[ICC_OPT_RAMP + channel * 4096u + 4095u];
        }
    }
    let obtained = icc_opt_tetra(white);
    var equal = true;
    for (var channel = 0u; channel < 3u; channel++) {
        let difference = 65535u - obtained[channel];
        if difference > 61440u { return; }
        if difference != 0u { equal = false; break; }
    }
    if equal { return; }
    var node: vec3<u32>;
    for (var channel = 0u; channel < 3u; channel++) {
        let position = f64(white[channel]) * 32.0lf / 65535.0lf;
        if position != floor(position) { return; }
        node[channel] = u32(position);
    }
    let base = ICC_OPT_CLUT + node.x * 3267u + node.y * 99u + node.z * 3u;
    for (var channel = 0u; channel < 3u; channel++) { scratch[base + channel] = 65535u; }
}

@compute @workgroup_size(64)
fn prepare_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    switch params.mode {
        case 6u: { if index < icc_eval_curve_count() { icc_eval_prepare_curve(index); } }
        case 7u: { if index == 0u { icc_eval_prepare_bpc(); } }
        case 0u: { if index == 0u { icc_opt_classify(); } }
        case 1u: { icc_opt_sample(index); }
        case 2u: { if index == 0u { icc_opt_decide(); } }
        case 3u: { icc_opt_reverse(index); }
        case 4u: { icc_opt_resample(index); }
        case 5u: { if index == 0u { icc_opt_white_patch(); } }
        default: {}
    }
}
