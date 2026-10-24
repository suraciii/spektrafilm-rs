// Shared calibrated axes and ABI for post-optics setup and positive rendering.
// All vectors of binary64 coefficients occupy 32 bytes in the uniform.
struct Params {
    header: vec4<u32>,
    offsets: vec4<u32>,
    counts: vec4<u32>,
    flags: vec4<u32>,
    contrast: vec4<f64>,
    dmax: vec4<f64>,
    brightness: vec4<f64>,
    denominator: vec4<f64>,
    camera: vec4<f64>,
    setup: vec4<f64>,
    operator_terms: vec4<f64>,
    color: vec4<f64>,
    destination: array<vec4<f64>, 3>,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(2) var<storage, read> density: array<f64>;
@group(0) @binding(3) var<storage, read> exposure: array<f64>;
@group(0) @binding(5) var<storage, read_write> status: atomic<u32>;

fn finite64(value: f64) -> bool {
    // Integer classification is not subject to driver fast-math NaN folding.
    return (bitcast<u64>(value) & u64(0x7ff0000000000000)) != u64(0x7ff0000000000000);
}
fn estimate(d: f64, c: u32) -> f64 {
    let offset = p.offsets[c];
    var lower = 0u;
    var upper = p.counts[c];
    while lower < upper {
        let mid = (lower + upper) / 2u;
        if density[offset + mid] < d { lower = mid + 1u; } else { upper = mid; }
    }
    let hi = offset + clamp(lower, 1u, p.counts[c] - 1u);
    let lo = hi - 1u;
    return exposure[lo] + (d - density[lo]) / (density[hi] - density[lo]) * (exposure[hi] - exposure[lo]);
}
fn neutral_density(x: f64, c: u32) -> f64 {
    let offset = p.offsets[c];
    var lower = 0u;
    var upper = p.counts[c];
    while lower < upper {
        let mid = (lower + upper) / 2u;
        if exposure[offset + mid] < x { lower = mid + 1u; } else { upper = mid; }
    }
    let hi = offset + clamp(lower, 1u, p.counts[c] - 1u);
    let lo = hi - 1u;
    let gamma = (density[hi] - density[lo]) / (exposure[hi] - exposure[lo]);
    return density[lo] + (x - exposure[lo]) * gamma;
}
