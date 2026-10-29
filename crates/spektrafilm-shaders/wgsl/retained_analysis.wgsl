@group(0) @binding(2) var<storage, read_write> counts: array<atomic<u32>>;

// Integer thresholds avoid overflow and floating-point spatial rounding.
fn spatial_bin(x: u32) -> u32 {
    let width = p.x1 - p.x0;
    let q = width / p.spatial;
    let r = width % p.spatial;
    var lo = 0u;
    var hi = p.spatial;
    loop {
        if lo + 1u >= hi { break; }
        let mid = (lo + hi) / 2u;
        let fraction = r * mid;
        let threshold = q * mid + fraction / p.spatial + select(0u, 1u, 2u * (fraction % p.spatial) > p.spatial);
        if x >= threshold { lo = mid; } else { hi = mid; }
    }
    return lo;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.count { return; }
    let selected = p.start + id.x;
    let region_width = p.x1 - p.x0;
    let x = p.x0 + selected % region_width;
    let y = p.y0 + selected / region_width;
    let i = y * p.width + x - p.page_first;
    let sample = analysis_sample(i);
    if !sample.valid { atomicAdd(&counts[p.totals_offset + 1u], 1u); return; }
    atomicAdd(&counts[p.totals_offset], 1u);
    if any(sample.linear < vec3<f32>(0.0)) { atomicAdd(&counts[p.totals_offset + 2u], 1u); }
    if any(sample.linear > vec3<f32>(1.0)) { atomicAdd(&counts[p.totals_offset + 3u], 1u); }
    let bins = vec4<u32>(signal_bin(sample.tone), signal_bin(sample.rgb.x), signal_bin(sample.rgb.y), signal_bin(sample.rgb.z));
    for (var channel = 0u; channel < 4u; channel++) {
        atomicAdd(&counts[channel * 256u + bins[channel]], 1u);
    }
    let col = spatial_bin(x - p.x0);
    atomicAdd(&counts[p.column_offset + col], 1u);
    if p.mode == 1u {
        for (var channel = 0u; channel < 4u; channel++) {
            atomicAdd(&counts[1024u + (channel * p.spatial + col) * 256u + bins[channel]], 1u);
        }
    } else if p.mode == 2u {
        let extent = 0.4 / f32(p.zoom);
        if abs(sample.ab.x) > extent || abs(sample.ab.y) > extent {
            atomicAdd(&counts[p.totals_offset + 4u], 1u);
        } else {
            let uv = (sample.ab / extent + vec2<f32>(1.0)) * 128.0;
            let bx = min(255u, u32(floor(uv.x)));
            let by = min(255u, u32(floor(uv.y)));
            atomicAdd(&counts[1024u + by * 256u + bx], 1u);
        }
    }
}
