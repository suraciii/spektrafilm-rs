// The core supplies the response and the complete forward/inverse Oklab
// matrix pairing. Input and output are linear RGB in the target space.
struct Params {
    n_pixels: u32,
    expand_enabled: u32,
    black: f32,
    inverse_span: f32,
    tone: vec4<f32>,
    color: vec4<f32>,
    to_srgb: mat3x3<f32>,
    from_srgb: mat3x3<f32>,
    rgb_to_lms: mat3x3<f32>,
    lms_to_lab: mat3x3<f32>,
    lab_to_lms: mat3x3<f32>,
    lms_to_rgb: mat3x3<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> data: array<f32>;

fn expand_channel(x: f32) -> f32 {
    // The shared transfer library works on positive inputs; explicitly extend
    // its sRGB curve symmetrically for negative working coordinates.
    let encoded = sign(x) * encode_one(abs(x), 2u);
    let mapped = clamp((encoded - params.black) * params.inverse_span, 0.0, 1.0);
    return decode_one(mapped, 2u);
}

fn tone_window(l: f32) -> f32 {
    if l <= 0.0 || l >= 1.0 { return 0.0; }
    let t = l * (1.0 - l);
    return 16.0 * t * t;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels { return; }
    let base = idx * 3u;
    var rgb = params.to_srgb * vec3<f32>(data[base], data[base + 1u], data[base + 2u]);
    if params.expand_enabled != 0u {
        rgb = vec3<f32>(expand_channel(rgb.x), expand_channel(rgb.y), expand_channel(rgb.z));
    }
    if any(params.tone != vec4<f32>(0.0)) {
        let lms = params.rgb_to_lms * rgb;
        let roots = sign(lms) * pow(abs(lms), vec3<f32>(1.0 / 3.0));
        var lab = params.lms_to_lab * roots;
        let l1 = lab.x + params.tone.y * (2.0 * lab.x - 1.0) * tone_window(lab.x);
        lab.x = l1 + params.tone.x * tone_window(l1);
        let chroma = length(lab.yz);
        let relative_chroma = chroma / params.color.x;
        let warm = params.tone.z * tone_window(lab.x) / (1.0 + relative_chroma * relative_chroma);
        lab.y = lab.y + warm * params.color.y;
        lab.z = lab.z + warm * params.color.z;
        let adjusted_chroma = length(lab.yz);
        let relative_adjusted_chroma = adjusted_chroma / params.color.x;
        let saturation = exp(params.tone.w / (1.0 + relative_adjusted_chroma * relative_adjusted_chroma));
        lab.y = lab.y * saturation;
        lab.z = lab.z * saturation;
        let inverse_lms = params.lab_to_lms * lab;
        rgb = params.lms_to_rgb * (inverse_lms * inverse_lms * inverse_lms);
    }
    let out = params.from_srgb * rgb;
    data[base] = out.x;
    data[base + 1u] = out.y;
    data[base + 2u] = out.z;
}
