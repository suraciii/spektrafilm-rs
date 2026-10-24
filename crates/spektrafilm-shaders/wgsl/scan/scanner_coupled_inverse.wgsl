// Coupled negative-to-positive interpretation: the host-resolved inverse of
// the sampled physical scanner response, applied per pixel.
//
// CPU equivalent: `CoupledInverse::estimate_density_with_anchor` and
// `interpret_coupled_capture` in `stages::scanning`.
//
// The host resolves the whole inverse — the interior tetrahedra and boundary
// faces, their median-split BVH trees, the film's monotone axes, and the tone
// constants — and uploads every one of them verbatim. The device therefore
// builds nothing and orders nothing of its own: it reproduces the reference's
// traversal exactly, so "first tetra that contains the query" and "nearest
// boundary face" are the host's answers, not an approximation of them.
//
// Per pixel:
//   1. query:      q = asinh(capture / scale) per channel, with the host's
//                  asymptotic fallback for a nonfinite ratio.
//   2. containing: descend the interior tree, pruning by the node bounds with
//                  the host's 1e-12 tolerance, left child before right, and
//                  return the first containing tetra's interpolated density.
//                  `contains` uses weights >= -1e-9 and sum <= 1 + 1e-9.
//   3. nearest:    otherwise descend the boundary tree, pruning when a node's
//                  squared bounds distance exceeds the best so far, visiting
//                  the nearer child first, and keep the closest anchor on a
//                  triangle (including its vertex and edge Voronoi regions)
//                  under the host's `(distance, index)` tie-break.
//   4. axes:       the film's monotone axes recover relative log-exposure,
//                  anchored at the face's density with the axes' slopes.
//   5. tone:       the conversion tone curve, anchored linearly at a
//                  boundary face and evaluated directly inside the domain.
//
// Binary64 throughout: the reference is an f64 model whose conditioning and
// containment tolerances are meaningful only at that precision, and the
// shared explicit f64 helpers replace the driver's broken extended
// log/exp builtins. Every intermediate is a scalar or a `vec3<f64>` — no
// dynamically indexed local f64 array, which the driver's loop unroller
// cannot handle and would lose the device to; the traversal stacks are
// `i32`, which it handles.
//
// The scalar head is the uniform block and every f64 array is a tightly
// packed storage buffer, because a uniform block's `vec4<f64>` members would
// not sit at the offsets the host's `[f64; 4]` arrays occupy.

struct Params {
    n_pixels: u32,
    n_tetras: u32,
    n_faces: u32,
    knots: u32,
    interior_nodes_offset: u32,
    boundary_nodes_offset: u32,
    tetras_offset: u32,
    faces_offset: u32,
    density_offset: u32,
    exposure_offset: u32,
    interior_items_offset: u32,
    boundary_items_offset: u32,
    n_interior_nodes: u32,
    n_boundary_nodes: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
// One tightly packed f64 storage buffer holds every read-only resolved
// constant, in this order: the 18 tone constants, the interior nodes, the
// boundary nodes, the tetras, the faces, the interior item permutation, the
// boundary item permutation, the axis density knots, and the axis exposure
// knots. Both trees' item permutations are stored as the f64 bit patterns of
// their i32 values, so the device reads them back with `bitcast`. The whole
// inverse is packed into one binding because a stage may bind only a handful
// of storage buffers, and the resolved inverse is not large enough to
// warrant the extra bindings.
@group(0) @binding(1) var<storage, read> packed: array<f64>;
@group(0) @binding(2) var<storage, read> capture_rgb: array<f32>;  // [H*W*3]
@group(0) @binding(3) var<storage, read_write> output: array<f32>; // [H*W*3]

/// One tetra's 24-value row, at its packed offset.
fn tetra_at(index: u32, lane: u32) -> f64 {
    return packed[params.tetras_offset + index * TETRA_STRIDE + lane];
}

/// One face's 45-value row, at its packed offset.
fn face_at(index: u32, lane: u32) -> f64 {
    return packed[params.faces_offset + index * FACE_STRIDE + lane];
}

// Offsets into `packed`, resolved on the host.
const K_START: u32 = 0u;
const K_SPAN: u32 = 1u;
const K_BLACK: u32 = 2u;
const K_SCALE_WINDOW: u32 = 5u;
const K_SCALE: u32 = 8u;
const K_CONTRAST: u32 = 11u;
const K_DMAX: u32 = 12u;
const K_BRIGHTNESS_EV: u32 = 13u;
const K_BALANCE_EV: u32 = 14u;
const K_CAMERA_EV: u32 = 17u;
const CONSTANTS_LEN: u32 = 18u;
// `PACK_INTERIOR_NODES` and the rest are supplied by the uniform block, so
// the host can place each section without the shader hard-coding a length.

const TETRA_STRIDE: u32 = 24u;
const FACE_STRIDE: u32 = 45u;
const NODE_STRIDE: u32 = 10u;
const STACK_DEPTH: i32 = 160;
const LN_2: f64 = 0.69314718055994530942;
const LN_10: f64 = 2.30258509299404568402;
// WGSL rejects unary minus on a const identifier, so the negated ln(10)
// the tone's `expm1` needs is its own constant.
const NEG_LN_10: f64 = -2.30258509299404568402;

// The host's error path returns no image at all, so a device that reaches it
// must not silently produce a finite value. A quiet NaN cannot be written as
// a literal (WGSL const-folds `sqrt(-1)`), so it is derived at runtime.
fn f64_quiet_nan() -> f64 {
    var zero: f64 = 0.0;
    return zero / zero;
}
const LOG10_2: f64 = 0.30102999566398119521;
// WGSL rejects unary minus on a const identifier, so the bounds tolerance
// is written inline as `1e-12` and the containment tolerance is subtracted
// from zero at use.
const CONTAINS_TOLERANCE: f64 = 1e-9;

/// The reference's `asinh(capture / scale)`, with the asymptotic fallback
/// for a nonfinite ratio: `sign * (ln|capture| - ln(scale) + ln 2)`. The sign
/// comes from `capture / |capture|`, so no untyped literal decides the f64
/// type.
fn query_coordinate(capture: f64, scale: f64) -> f64 {
    let ratio = capture / scale;
    if ratio == ratio && abs(ratio) < 1.0e308 {
        return sf64_log(ratio + sqrt(ratio * ratio + 1.0));
    }
    let magnitude = abs(capture);
    return (capture / magnitude) * (sf64_log(magnitude) - sf64_log(scale) + LN_2);
}

// Node readers: both trees live in `packed`, each at its own section
// offset, and both store `left`/`right` as the f64 bit patterns of their
// i32 values so one buffer serves both.
fn interior_min(node_index: u32) -> vec3<f64> {
    let offset = params.interior_nodes_offset + node_index * NODE_STRIDE;
    return vec3<f64>(packed[offset], packed[offset + 1u], packed[offset + 2u]);
}

fn interior_max(node_index: u32) -> vec3<f64> {
    let offset = params.interior_nodes_offset + node_index * NODE_STRIDE;
    return vec3<f64>(packed[offset + 3u], packed[offset + 4u], packed[offset + 5u]);
}

fn interior_left(node_index: u32) -> i32 {
    return i32(packed[params.interior_nodes_offset + node_index * NODE_STRIDE + 6u]);
}

fn interior_right(node_index: u32) -> i32 {
    return i32(packed[params.interior_nodes_offset + node_index * NODE_STRIDE + 7u]);
}

fn interior_start(node_index: u32) -> u32 {
    return u32(packed[params.interior_nodes_offset + node_index * NODE_STRIDE + 8u]);
}

fn interior_end(node_index: u32) -> u32 {
    let raw = packed[params.interior_nodes_offset + node_index * NODE_STRIDE + 9u];
    return select(0u, u32(raw), i32(raw) >= 0);
}

fn boundary_min(node_index: u32) -> vec3<f64> {
    let offset = params.boundary_nodes_offset + node_index * NODE_STRIDE;
    return vec3<f64>(packed[offset], packed[offset + 1u], packed[offset + 2u]);
}

fn boundary_max(node_index: u32) -> vec3<f64> {
    let offset = params.boundary_nodes_offset + node_index * NODE_STRIDE;
    return vec3<f64>(packed[offset + 3u], packed[offset + 4u], packed[offset + 5u]);
}

fn boundary_left(node_index: u32) -> i32 {
    return i32(packed[params.boundary_nodes_offset + node_index * NODE_STRIDE + 6u]);
}

fn boundary_right(node_index: u32) -> i32 {
    return i32(packed[params.boundary_nodes_offset + node_index * NODE_STRIDE + 7u]);
}

fn boundary_start(node_index: u32) -> u32 {
    return u32(packed[params.boundary_nodes_offset + node_index * NODE_STRIDE + 8u]);
}

fn boundary_end(node_index: u32) -> u32 {
    return u32(packed[params.boundary_nodes_offset + node_index * NODE_STRIDE + 9u]);
}

// The item permutations are stored as f64 bit patterns of their i32 values.
fn interior_item(index: u32) -> u32 {
    return u32(bitcast<i32>(packed[params.interior_items_offset + index]));
}

fn boundary_item(index: u32) -> u32 {
    return u32(bitcast<i32>(packed[params.boundary_items_offset + index]));
}

/// The reference's `Box3::contains` tolerance, on the interior tree.
fn interior_contains(point: vec3<f64>, node_index: u32) -> bool {
    let lo = interior_min(node_index);
    let hi = interior_max(node_index);
    return point.x >= lo.x - 1e-12
        && point.x <= hi.x + 1e-12
        && point.y >= lo.y - 1e-12
        && point.y <= hi.y + 1e-12
        && point.z >= lo.z - 1e-12
        && point.z <= hi.z + 1e-12;
}

/// The reference's `Box3::distance2`: the squared distance from the query to
/// the box, zero inside, on the boundary tree.
fn boundary_distance2(point: vec3<f64>, node_index: u32) -> f64 {
    let lo = boundary_min(node_index);
    let hi = boundary_max(node_index);
    let dx = point.x - clamp(point.x, lo.x, hi.x);
    let dy = point.y - clamp(point.y, lo.y, hi.y);
    let dz = point.z - clamp(point.z, lo.z, hi.z);
    return dx * dx + dy * dy + dz * dz;
}

/// Closest point on a triangle, including its vertex and edge Voronoi
/// regions — the reference's `closest_triangle`, transcribed in the same
/// branch order so the returned anchor is bit-comparable.
fn closest_triangle(p: vec3<f64>, a: vec3<f64>, b: vec3<f64>, c: vec3<f64>) -> vec3<f64> {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / (d4 - d3 + d5 - d6));
    }
    let denominator = 1.0 / (va + vb + vc);
    return a + ab * (vb * denominator) + ac * (vc * denominator);
}

/// The reference's `Tetra::contains`, one tetra.
fn tetra_contains(tetra_index: u32, query: vec3<f64>) -> bool {
    let offset = params.tetras_offset + tetra_index * TETRA_STRIDE;
    let origin = vec3<f64>(packed[offset], packed[offset + 1u], packed[offset + 2u]);
    let inv0 = vec3<f64>(packed[offset + 3u], packed[offset + 4u], packed[offset + 5u]);
    let inv1 = vec3<f64>(packed[offset + 6u], packed[offset + 7u], packed[offset + 8u]);
    let inv2 = vec3<f64>(packed[offset + 9u], packed[offset + 10u], packed[offset + 11u]);
    let delta = query - origin;
    let w0 = dot(inv0, delta);
    let w1 = dot(inv1, delta);
    let w2 = dot(inv2, delta);
    return w0 >= 0.0 - CONTAINS_TOLERANCE
        && w1 >= 0.0 - CONTAINS_TOLERANCE
        && w2 >= 0.0 - CONTAINS_TOLERANCE
        && w0 + w1 + w2 <= 1.0 + CONTAINS_TOLERANCE;
}

/// The reference's `Tetra::density`, one tetra.
fn tetra_density(tetra_index: u32, query: vec3<f64>) -> vec3<f64> {
    let offset = params.tetras_offset + tetra_index * TETRA_STRIDE;
    let origin = vec3<f64>(packed[offset], packed[offset + 1u], packed[offset + 2u]);
    let inv0 = vec3<f64>(packed[offset + 3u], packed[offset + 4u], packed[offset + 5u]);
    let inv1 = vec3<f64>(packed[offset + 6u], packed[offset + 7u], packed[offset + 8u]);
    let inv2 = vec3<f64>(packed[offset + 9u], packed[offset + 10u], packed[offset + 11u]);
    let delta = query - origin;
    let w0 = dot(inv0, delta);
    let w1 = dot(inv1, delta);
    let w2 = dot(inv2, delta);
    let density0 = vec3<f64>(packed[offset + 12u], packed[offset + 13u], packed[offset + 14u]);
    // `edges[j]` starts at `offset + 15 + j * 3`, and `edges[j][c]` is the
    // c-th channel's density edge of corner j.
    return vec3<f64>(
        density0.x + w0 * packed[offset + 15u] + w1 * packed[offset + 18u] + w2 * packed[offset + 21u],
        density0.y + w0 * packed[offset + 16u] + w1 * packed[offset + 19u] + w2 * packed[offset + 22u],
        density0.z + w0 * packed[offset + 17u] + w1 * packed[offset + 20u] + w2 * packed[offset + 23u],
    );
}

/// The reference's `CoupledInverse::find`: descend the uploaded interior
/// tree, prune by the node bounds, visit the left child before the right,
/// and return the first containing tetra, or -1 when none contains the
/// query. The stack is `i32` because the driver's loop unroller handles
/// integer but not binary64 constant arrays.
fn find_tetra(query: vec3<f64>) -> i32 {
    var stack: array<i32, 160>;
    var top: i32 = 0;
    stack[0] = 0;
    while top >= 0 {
        let node_index = u32(stack[top]);
        top = top - 1;
        if !interior_contains(query, node_index) {
            continue;
        }
        let left = interior_left(node_index);
        if left < 0i {
            let start = interior_start(node_index);
            let end = interior_end(node_index);
            for (var k = start; k < end; k = k + 1u) {
                let tetra_index = interior_item(k);
                if tetra_contains(tetra_index, query) {
                    return i32(tetra_index);
                }
            }
        } else {
            // LIFO: push the right sibling first so the left child pops next.
            top = top + 1;
            stack[top] = interior_right(node_index);
            top = top + 1;
            stack[top] = left;
        }
    }
    return -1;
}

/// The nearest boundary face and its anchor, mirroring `CoupledInverse::
/// nearest`: descend the uploaded boundary tree, prune when a node's squared
/// bounds distance exceeds the best so far, visit the nearer child first, and
/// keep the closest anchor under the host's `(distance, index)` tie-break.
struct NearestFace {
    distance: f64,
    index: i32,
    anchor: vec3<f64>,
}

fn nearest_face(query: vec3<f64>) -> NearestFace {
    var best_distance: f64 = 1.0e300;
    var best_index: i32 = -1;
    var best_anchor = vec3<f64>(0.0, 0.0, 0.0);
    var stack: array<i32, 160>;
    var top: i32 = 0;
    stack[0] = 0;
    while top >= 0 {
        let node_index = u32(stack[top]);
        top = top - 1;
        if boundary_distance2(query, node_index) > best_distance {
            continue;
        }
        let left = boundary_left(node_index);
        if left < 0i {
            let start = boundary_start(node_index);
            let end = boundary_end(node_index);
            for (var k = start; k < end; k = k + 1u) {
                let face_index = i32(boundary_item(k));
                let offset = params.faces_offset + u32(face_index) * FACE_STRIDE;
                let a = vec3<f64>(packed[offset], packed[offset + 1u], packed[offset + 2u]);
                let b = vec3<f64>(packed[offset + 3u], packed[offset + 4u], packed[offset + 5u]);
                let c = vec3<f64>(packed[offset + 6u], packed[offset + 7u], packed[offset + 8u]);
                let anchor = closest_triangle(query, a, b, c);
                let difference = query - anchor;
                let distance = dot(difference, difference);
                // The host compares `(distance, index)` tuples, so an equal
                // distance keeps the smaller index.
                if distance < best_distance
                    || (distance == best_distance
                        && (best_index < 0 || face_index < best_index)) {
                    best_distance = distance;
                    best_index = face_index;
                    best_anchor = anchor;
                }
            }
        } else {
            let right = boundary_right(node_index);
            let a = boundary_distance2(query, u32(left));
            let b = boundary_distance2(query, u32(right));
            // LIFO: push the farther child first so the nearer pops next.
            if a <= b {
                top = top + 1;
                stack[top] = right;
                top = top + 1;
                stack[top] = left;
            } else {
                top = top + 1;
                stack[top] = left;
                top = top + 1;
                stack[top] = right;
            }
        }
    }
    return NearestFace(best_distance, best_index, best_anchor);
}

/// The denominator of both tone value and slope, mirroring `positive_tone`.
fn tone_denominator() -> f64 {
    return -sf64_exp(NEG_LN_10 * packed[K_DMAX]) + 1.0;
}

/// The point-map applied to a log-exposure, mirroring `positive_tone`'s
/// `PointMap::apply`. A default window is an exact passthrough.
fn point_apply(log_e: f64, channel: u32) -> f64 {
    if packed[K_BLACK + channel] == 0.0 && packed[K_SCALE_WINDOW + channel] == 1.0 {
        return log_e;
    }
    return packed[K_START]
        + (log_e - packed[K_START] - packed[K_BLACK + channel] * packed[K_SPAN])
            * packed[K_SCALE_WINDOW + channel];
}

/// The conversion tone curve, mirroring `positive_tone(...).0`.
fn tone_value(log_e: f64, channel: u32) -> f64 {
    let mapped = point_apply(log_e, channel);
    let e = mapped / LOG10_2 - packed[K_CAMERA_EV] + packed[K_BRIGHTNESS_EV]
        + packed[K_BALANCE_EV + channel];
    let z = packed[K_CONTRAST] * e;
    // Numerically stable logistic: the reference branches on the sign of z.
    // Both branches are f64 expressions, so `select` infers f64.
    let t = select(
        sf64_exp(z) / (sf64_exp(z) + 1.0),
        1.0 / (sf64_exp(-z) + 1.0),
        z >= 0.0,
    );
    let density = packed[K_DMAX] * (1.0 - t);
    let positive = sf64_exp(NEG_LN_10 * density) - 1.0;
    return (positive + tone_denominator()) / tone_denominator();
}

/// The tone derivative, mirroring `positive_tone`'s second return value.
fn tone_slope(log_e: f64, channel: u32) -> f64 {
    let mapped = point_apply(log_e, channel);
    let e = mapped / LOG10_2 - packed[K_CAMERA_EV] + packed[K_BRIGHTNESS_EV]
        + packed[K_BALANCE_EV + channel];
    let z = packed[K_CONTRAST] * e;
    let t = select(
        sf64_exp(z) / (sf64_exp(z) + 1.0),
        1.0 / (sf64_exp(-z) + 1.0),
        z >= 0.0,
    );
    let density = packed[K_DMAX] * (1.0 - t);
    // d t/dz = t(1-t); d z/d e = contrast; d e/d log_e = window / log10(2).
    return LN_10 * packed[K_DMAX] * packed[K_CONTRAST] * t * (1.0 - t)
        * sf64_exp(NEG_LN_10 * density)
        / (tone_denominator() * LOG10_2)
        * packed[K_SCALE_WINDOW + channel];
}

/// Piecewise-linear axis estimate, mirroring `DensityAxis::estimate`.
/// `.x` is the estimated log-exposure, `.y` the interval's slope.
fn axis_estimate(d: f64, channel: u32) -> vec2<f64> {
    var lower = 0u;
    var upper = params.knots;
    while lower < upper {
        let mid = (lower + upper) / 2u;
        if packed[params.density_offset + mid * 3u + channel] < d {
            lower = mid + 1u;
        } else {
            upper = mid;
        }
    }
    let idx = clamp(lower, 1u, params.knots - 1u);
    let lo = idx - 1u;
    let d_lo = packed[params.density_offset + lo * 3u + channel];
    let d_hi = packed[params.density_offset + idx * 3u + channel];
    let e_lo = packed[params.exposure_offset + lo * 3u + channel];
    let e_hi = packed[params.exposure_offset + idx * 3u + channel];
    let fraction = (d - d_lo) / (d_hi - d_lo);
    return vec2<f64>(
        e_lo + fraction * (e_hi - e_lo),
        (e_hi - e_lo) / (d_hi - d_lo),
    );
}

/// The host's `(capture - scale * sinh(anchor)) / hypot(scale, scale *
/// sinh(anchor))`, one channel.
fn raw_delta(capture: f64, scale: f64, anchor: f64) -> f64 {
    // `sinh` and `hypot` are not part of the explicit binary64 helper set the
    // other passes use, and the driver's extended forms are not reliable, so
    // both are composed from the validated `sf64_exp`.
    let raw_anchor = scale * (sf64_exp(anchor) - sf64_exp(0.0 - anchor)) * 0.5;
    return (capture - raw_anchor)
        / sqrt(scale * scale + raw_anchor * raw_anchor);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= params.n_pixels {
        return;
    }
    let base = pixel * 3u;
    let capture = vec3<f64>(
        f64(capture_rgb[base]),
        f64(capture_rgb[base + 1u]),
        f64(capture_rgb[base + 2u]),
    );
    let query = vec3<f64>(
        query_coordinate(capture.x, packed[K_SCALE]),
        query_coordinate(capture.y, packed[K_SCALE + 1u]),
        query_coordinate(capture.z, packed[K_SCALE + 2u]),
    );

    // Containing tetra: the uploaded tree is the host's, so the first hit is
    // the host's answer.
    let tetra_index = find_tetra(query);
    if tetra_index >= 0 {
        let density_out = tetra_density(u32(tetra_index), query);
        for (var c = 0u; c < 3u; c = c + 1u) {
            output[base + c] = f32(tone_value(axis_estimate(density_out[c], c).x, c));
        }
        return;
    }

    // Nearest boundary face, on the host's tie-break.
    let nearest = nearest_face(query);
    if nearest.index < 0 {
        // The host reports the same failure for an empty boundary.
        for (var c = 0u; c < 3u; c = c + 1u) {
            output[base + c] = f32(f64_quiet_nan());
        }
        return;
    }
    let offset = params.faces_offset + u32(nearest.index) * FACE_STRIDE;
    let a = vec3<f64>(packed[offset], packed[offset + 1u], packed[offset + 2u]);
    let b = vec3<f64>(packed[offset + 3u], packed[offset + 4u], packed[offset + 5u]);
    let c_pt = vec3<f64>(packed[offset + 6u], packed[offset + 7u], packed[offset + 8u]);
    // The anchor's barycentric coordinates on the triangle.
    let ab = b - a;
    let ac = c_pt - a;
    let ap = nearest.anchor - a;
    let aa = dot(ab, ab);
    let bb = dot(ac, ac);
    let cc = dot(ab, ac);
    let determinant = aa * bb - cc * cc;
    let v = (bb * dot(ap, ab) - cc * dot(ap, ac)) / determinant;
    let w = (aa * dot(ap, ac) - cc * dot(ap, ab)) / determinant;
    let bary = vec3<f64>(1.0 - v - w, v, w);
    // Anchor density, mixed by the same barycentric weights the host uses.
    var anchor_density = vec3<f64>(0.0, 0.0, 0.0);
    for (var c = 0u; c < 3u; c = c + 1u) {
        anchor_density[c] = bary.x * packed[offset + 9u + c]
            + bary.y * packed[offset + 12u + c]
            + bary.z * packed[offset + 15u + c];
    }
    // The raw-capture delta at the anchor drives the tangent continuation.
    let delta = vec3<f64>(
        raw_delta(capture.x, packed[K_SCALE], nearest.anchor.x),
        raw_delta(capture.y, packed[K_SCALE + 1u], nearest.anchor.y),
        raw_delta(capture.z, packed[K_SCALE + 2u], nearest.anchor.z),
    );
    for (var c = 0u; c < 3u; c = c + 1u) {
        let anchor_axis = axis_estimate(anchor_density[c], c);
        // The continuation's density: the anchor density plus the averaged
        // Jacobian's tangent displacement along the capture delta.
        var displacement: f64 = 0.0;
        for (var k = 0u; k < 3u; k = k + 1u) {
            // `jacobian[v][c][k]` is vertex v's c-th density edge along
            // tangent direction k; the host averages it over the vertices.
            // The uploaded layout is the host's row-major `[v][c][k]`, so
            // vertex v sits at `v * 9 + c * 3 + k`.
            let jacobian_index = offset + 18u + (c * 3u + k);
            displacement = displacement
                + (bary.x * packed[jacobian_index]
                    + bary.y * packed[jacobian_index + 9u]
                    + bary.z * packed[jacobian_index + 18u])
                    * delta[k];
        }
        let continued_density = anchor_density[c] + displacement;
        let continued_axis = axis_estimate(continued_density, c);
        // The anchored exposure: the axis estimate at the continued density,
        // referenced back through the anchor's slope.
        let exposure = anchor_axis.x
            + anchor_axis.y * (continued_density - anchor_density[c]);
        let value = tone_value(exposure, c);
        let slope = tone_slope(exposure, c);
        // The tone's own continuation between the anchor and the estimate.
        output[base + c] = f32(value + slope * (continued_axis.x - exposure));
    }
}
