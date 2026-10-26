// G2 of the device path: per-pixel density to per-pixel layer targets.
//
// Faithful to the CPU in `crates/spektrafilm-model/src/grain/v3.rs`:
//
//  - `bracket` clamps at both ends and otherwise takes
//    `searchsorted(side='right') - 1`, so the low knot is the last one that is
//    `<= x` and the fraction uses that bracket's `inv_dx`,
//  - a layer's weight is `(value + frac_l * min).max(0)` and only for
//    `cap_l > 0`; `value` is the raw layer curve at the bracket's two knots,
//  - the total that `allocate` distributes is `(x + min).clamp(0, cap_total)`,
//    while the bracket lookup uses the unclamped `x`,
//  - `allocate` is the saturating loop: a round that retires a layer leaves
//    the remaining layers at zero and redistributes what is left. Three
//    layers bound it to three rounds.
//
// The tables are laid out per channel as `[knots K][inv_dx K-1][layer 0 K]
// [layer 1 K][layer 2 K]`, so a channel's knot count is data, not a constant.
// `inv_dx` holds one spacing per bracket, and a bracket is chosen with
// `low <= K-2`, so its block is one shorter than the knot block.

const WORKGROUP: u32 = 256u;

struct LayerParams {
    source: vec4<u32>,
    region: vec4<u32>, // width, height, monochrome, validation-only
}

@group(0) @binding(0) var<uniform> params: LayerParams;
@group(0) @binding(1) var<storage, read> tables: array<Table>;
@group(0) @binding(2) var<storage, read> curves: array<f32>;
@group(0) @binding(3) var<storage, read> densities: array<f32>;
@group(0) @binding(4) var<storage, read_write> targets: array<f32>;
@group(0) @binding(5) var<storage, read_write> status: atomic<u32>;

fn knot_at(table: Table, index: u32) -> f32 {
    return curves[table.offset + index];
}

fn inv_dx_at(table: Table, index: u32) -> f32 {
    return curves[table.offset + table.knot_count + index];
}

fn layer_at(table: Table, layer: u32, index: u32) -> f32 {
    let base = table.offset + table.knot_count + (table.knot_count - 1u);
    return curves[base + layer * table.knot_count + index];
}

/// `(low, t)` of the CPU's bracket lookup. The knots carry the
/// positive-film sign, so the query is multiplied by the table's sign first,
/// exactly as the CPU's `Channel::bracket` does.
fn bracket(table: Table, x: f32) -> vec2<f32> {
    let query = x * table.lookup_sign;
    let k = table.knot_count;
    if (query <= knot_at(table, 0u)) {
        return vec2<f32>(0.0, 0.0);
    }
    if (query >= knot_at(table, k - 1u)) {
        return vec2<f32>(f32(k - 2u), 1.0);
    }
    // Last index whose knot is `<= x` is `partition_point(..) - 1`.
    var low = 0u;
    var high = k;
    loop {
        if (high - low <= 1u) { break; }
        let mid = (low + high) / 2u;
        if (knot_at(table, mid) <= query) { low = mid; } else { high = mid; }
    }
    return vec2<f32>(f32(low), (query - knot_at(table, low)) * inv_dx_at(table, low));
}

// Retired layers retain their capacity; only still-active provisional offers
// are cleared before redistributing the remaining total.
fn allocate(total_target: f32, capacity: vec4<f32>, weights: vec4<f32>) -> vec3<f32> {
    let total = capacity.x + capacity.y + capacity.z;
    var out = vec3<f32>(0.0);
    if (total_target <= 0.0) { return out; }
    if (total_target >= total) { return capacity.xyz; }
    var unsaturated = capacity.xyz > vec3<f32>(0.0);
    var remaining = total_target;
    for (var round = 0u; round < 3u; round = round + 1u) {
        if (remaining <= 0.0 || !any(unsaturated)) { break; }
        var share = select(vec3<f32>(0.0), max(weights.xyz, vec3<f32>(0.0)), unsaturated);
        var scale = max(share.x, max(share.y, share.z));
        if (scale <= 0.0) {
            share = select(vec3<f32>(0.0), capacity.xyz, unsaturated);
            scale = max(share.x, max(share.y, share.z));
            if (scale <= 0.0) { break; }
        }
        // Normalize before summing and multiplying: finite coefficients do
        // not imply that their raw sum or remaining*weight is representable.
        share = share / scale;
        let sum = share.x + share.y + share.z;
        let offer = remaining * (share / sum);
        let capped = unsaturated & (offer >= capacity.xyz);
        out = select(out, offer, unsaturated);
        out = select(out, capacity.xyz, capped);
        if (!any(capped)) { break; }
        unsaturated = unsaturated & !capped;
        out = select(out, vec3<f32>(0.0), unsaturated);
        remaining = total_target - (out.x + out.y + out.z);
    }
    return out;
}

fn layer_target(table: Table, x: f32) -> vec3<f32> {
    let total = clamp(x + table.min, 0.0, table.cap_total);
    let bracket_ = bracket(table, x);
    let low = u32(bracket_.x);
    let t = bracket_.y;
    var weights = vec4<f32>(0.0);
    weights.x = select(0.0, (layer_at(table, 0u, low) + t * (layer_at(table, 0u, low + 1u) - layer_at(table, 0u, low))) + table.frac.x * table.min, table.cap.x > 0.0);
    weights.y = select(0.0, (layer_at(table, 1u, low) + t * (layer_at(table, 1u, low + 1u) - layer_at(table, 1u, low))) + table.frac.y * table.min, table.cap.y > 0.0);
    weights.z = select(0.0, (layer_at(table, 2u, low) + t * (layer_at(table, 2u, low + 1u) - layer_at(table, 2u, low))) + table.frac.z * table.min, table.cap.z > 0.0);
    weights = vec4<f32>(max(weights.x, 0.0), max(weights.y, 0.0), max(weights.z, 0.0), 0.0);
    return allocate(total, table.cap, weights);
}

fn validate_source(source_index: u32) {
    let first_density = densities[source_index];
    for (var channel = 0u; channel < 3u; channel = channel + 1u) {
        let table = tables[channel];
        let density = densities[source_index + channel];
        // Check original lanes, never record-0 substitutions. Explicit IEEE
        // finiteness checks prevent NaN and infinity from passing the envelope.
        if ((bitcast<u32>(density) & 0x7f800000u) == 0x7f800000u ||
            !(density >= -table.min - 1e-6 && density <= table.cap_total - table.min + 1e-6)) {
            atomicOr(&status, 1u);
        }
        if (params.region.z != 0u && !(abs(density - first_density) <= 1e-6)) {
            atomicOr(&status, 16u);
        }
    }
}

@compute @workgroup_size(WORKGROUP)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    let index = gid.x + gid.y * grid.x * WORKGROUP;
    let pixels = params.region.x * params.region.y;
    if (index >= pixels) { return; }
    let x = index % params.region.x + params.source.z;
    let y = index / params.region.x + params.source.w;
    let source_index = (y * params.source.x + x) * 3u;
    if (params.region.w != 0u) {
        validate_source(source_index);
        return;
    }
    // BW pages were checked in full before band-local allocation. Color keeps
    // its existing band-local checks and allocation arithmetic.
    if (params.region.z == 0u) {
        validate_source(source_index);
    }
    let first_density = densities[source_index];
    for (var channel = 0u; channel < 3u; channel = channel + 1u) {
        let table = tables[channel];
        let source_density = densities[source_index + channel];
        let density = select(source_density, first_density, params.region.z != 0u);
        let layers = layer_target(table, density);
        let total = clamp(density + table.min, 0.0, table.cap_total);
        let sum = layers.x + layers.y + layers.z;
        if (!all(layers >= vec3<f32>(0.0)) || !all(layers <= table.cap.xyz) ||
            !(abs(sum - total) <= max(1e-6, abs(total) * 2e-6))) {
            atomicOr(&status, 2u);
        }
        // `c * 3 + l`: the channel-major order of the CPU's `[[f64; 3]; 3]`.
        targets[index * 9u + channel * 3u + 0u] = layers.x;
        targets[index * 9u + channel * 3u + 1u] = layers.y;
        targets[index * 9u + channel * 3u + 2u] = layers.z;
    }
}
