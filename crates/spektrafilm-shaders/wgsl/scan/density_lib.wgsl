// Shared calibrated axes and ABI for post-optics setup and positive rendering.
// Coefficients and calibrated axes use binary32 throughout the scan path.
struct Params {
    header: vec4<u32>,
    offsets: vec4<u32>,
    counts: vec4<u32>,
    flags: vec4<u32>,
    contrast: vec4<f32>,
    dmax: vec4<f32>,
    brightness: vec4<f32>,
    denominator: vec4<f32>,
    camera: vec4<f32>,
    setup: vec4<f32>,
    operator_terms: vec4<f32>,
    color: vec4<f32>,
    destination: array<vec4<f32>, 3>,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(2) var<storage, read> density: array<f32>;
@group(0) @binding(3) var<storage, read> exposure: array<f32>;
@group(0) @binding(5) var<storage, read_write> status: atomic<u32>;

fn finite32(value: f32) -> bool {
    // Integer classification is not subject to driver fast-math NaN folding.
    return (bitcast<u32>(value) & 0x7f800000u) != 0x7f800000u;
}
fn estimate(d: f32, c: u32) -> f32 {
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
fn neutral_density(x: f32, c: u32) -> f32 {
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
