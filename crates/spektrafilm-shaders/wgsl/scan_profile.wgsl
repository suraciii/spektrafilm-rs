// Scan-stage appearance in linear sRGB. The core supplies the forward and
// inverse working-space matrices and the per-channel knots. Matrix, if
// present, runs before the curves; strength blends in linear sRGB.
struct Params {
    n_pixels: u32,
    strength: f32,
    has_matrix: u32,
    _pad: u32,
    to_srgb: mat3x3<f32>,
    from_srgb: mat3x3<f32>,
    matrix: mat3x3<f32>,
    knots: array<vec4<f32>, 27>,
    counts: vec4<u32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> data: array<f32>;

fn sample_curve(channel: u32, x: f32) -> f32 {
    let count = params.counts[channel];
    let first = params.knots[channel * 9u];
    if x <= first.x { return first.y; }
    var previous = first;
    for (var index = 1u; index < 9u; index = index + 1u) {
        if index >= count { break; }
        let current = params.knots[channel * 9u + index];
        if x <= current.x {
            let span = current.x - previous.x;
            if span <= 0.0 { return current.y; }
            return mix(previous.y, current.y, (x - previous.x) / span);
        }
        previous = current;
    }
    return previous.y;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels { return; }
    let base = idx * 3u;
    let source = params.to_srgb * vec3<f32>(data[base], data[base + 1u], data[base + 2u]);
    var transformed = source;
    if params.has_matrix != 0u {
        transformed = params.matrix * source;
    }
    transformed = vec3<f32>(
        sample_curve(0u, transformed.x),
        sample_curve(1u, transformed.y),
        sample_curve(2u, transformed.z),
    );
    let mixed = mix(source, transformed, params.strength);
    let out = params.from_srgb * mixed;
    data[base] = out.x;
    data[base + 1u] = out.y;
    data[base + 2u] = out.z;
}
