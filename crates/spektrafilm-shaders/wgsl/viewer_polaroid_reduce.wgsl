// Polaroid preparation: source maximum, density statistics, or one further
// statistics reduction level. No image values or statistics leave the device.
struct Params {
    count: u32,
    mode: u32,
    _pad0: u32,
    _pad1: u32,
}
struct Statistics {
    density_neutral: vec4<f32>,
    chroma: vec4<f32>,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read> statistics_in: array<Statistics>;
@group(0) @binding(3) var<storage, read_write> statistics_out: array<Statistics>;
@group(0) @binding(4) var<storage, read_write> density: array<f32>;

var<workgroup> density_maxima: array<vec4<f32>, 256>;
var<workgroup> chroma_maxima: array<vec4<f32>, 256>;

// Rust f32::max/min ignore a single NaN, including the seeded reductions in
// PolaroidState::new. Keep that behavior instead of relying on GPU NaN min/max.
fn numeric_max(a: f32, b: f32) -> f32 {
    if a != a { return b; }
    if b != b { return a; }
    return max(a, b);
}
fn numeric_min(a: f32, b: f32) -> f32 {
    if a != a { return b; }
    if b != b { return a; }
    return min(a, b);
}
fn maxima(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(numeric_max(a.x, b.x), numeric_max(a.y, b.y),
                     numeric_max(a.z, b.z), numeric_max(a.w, b.w));
}

@compute @workgroup_size(256)
fn main(
    @builtin(workgroup_id) group_id: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    // Merge one page's reduced record into the full-image statistics. A
    // separate dispatch orders this update without atomics or host samples.
    if p.mode == 3u {
        if group_id.x == 0u && group_id.y == 0u && lane == 0u {
            statistics_out[0].density_neutral = maxima(
                statistics_out[0].density_neutral, statistics_in[0].density_neutral);
            statistics_out[0].chroma = maxima(statistics_out[0].chroma, statistics_in[0].chroma);
        }
        return;
    }
    let group = group_id.x + group_id.y * grid.x;
    let index = group * 256u + lane;
    var dmax = vec4<f32>(0.0);
    var cmax = vec4<f32>(0.0);
    if index < p.count {
        if p.mode == 0u {
            let offset = index * 3u;
            let value = numeric_max(numeric_max(numeric_max(0.0,
                source[offset]), source[offset + 1u]), source[offset + 2u]);
            dmax.x = value;
        } else if p.mode == 1u {
            let divisor = select(1.0, 255.0, statistics_in[0].density_neutral.x > 1.0);
            let offset = index * 3u;
            let rgb = vec3<f32>(source[offset], source[offset + 1u], source[offset + 2u]);
            let d = clamp(-log(pow(clamp(rgb / divisor, vec3<f32>(1e-4),
                vec3<f32>(1.0)), vec3<f32>(2.2))), vec3<f32>(0.0), vec3<f32>(6.0));
            density[offset] = d.x;
            density[offset + 1u] = d.y;
            density[offset + 2u] = d.z;
            let neutral = numeric_min(numeric_min(numeric_min(bitcast<f32>(0x7f800000u), d.x), d.y), d.z);
            dmax = maxima(vec4<f32>(0.0), vec4<f32>(d, neutral));
            cmax = maxima(vec4<f32>(0.0), vec4<f32>(d - neutral, 0.0));
        } else {
            dmax = maxima(vec4<f32>(0.0), statistics_in[index].density_neutral);
            cmax = maxima(vec4<f32>(0.0), statistics_in[index].chroma);
        }
    }
    density_maxima[lane] = dmax;
    chroma_maxima[lane] = cmax;
    workgroupBarrier();
    for (var stride = 128u; stride > 0u; stride = stride / 2u) {
        if lane < stride {
            density_maxima[lane] = maxima(density_maxima[lane], density_maxima[lane + stride]);
            chroma_maxima[lane] = maxima(chroma_maxima[lane], chroma_maxima[lane + stride]);
        }
        workgroupBarrier();
    }
    if lane == 0u && group * 256u < p.count {
        statistics_out[group].density_neutral = density_maxima[0];
        statistics_out[group].chroma = chroma_maxima[0];
    }
}
