// Nine-layer density evaluation, in the same layer and channel order as
// PolaroidState::new/frame. Coefficients come from the caller's scalar timing.
struct Params {
    header: vec4<u32>,
    coefficients: array<vec4<f32>, 3>,
}
struct Statistics {
    density_neutral: vec4<f32>,
    chroma: vec4<f32>,
}
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> density: array<f32>;
@group(0) @binding(2) var<storage, read> statistics: array<Statistics>;
@group(0) @binding(3) var<storage, read_write> output: array<f32>;

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
fn coefficient(layer: u32) -> f32 {
    return p.coefficients[layer / 4u][layer % 4u];
}

@compute @workgroup_size(256)
fn main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
) {
    let index = gid.x + gid.y * grid.x * 256u;
    if index >= p.header.x { return; }
    let offset = index * 3u;
    let d = vec3<f32>(density[offset], density[offset + 1u], density[offset + 2u]);
    let n = numeric_min(numeric_min(numeric_min(bitcast<f32>(0x7f800000u), d.x), d.y), d.z);
    let ch = d - n;
    let dl = d / (statistics[0].density_neutral.xyz + vec3<f32>(1e-4));
    let cl = ch / (statistics[0].chroma.xyz + vec3<f32>(1e-4));
    let shadow = clamp(d.x * 0.34 + d.y * 0.39 + d.z * 0.27, 0.0, 1.75) / 1.75;
    let h = 1.0 - shadow;
    let strength = numeric_max(numeric_max(numeric_max(0.0, cl.x), cl.y), cl.z);
    let warmth = clamp((ch.y + 1.25 * ch.z - 0.85 * ch.x)
        / (ch.x + ch.y + ch.z + 1e-4), 0.0, 1.0);
    let silver = clamp(0.24 + 0.46 * shadow + 0.30 * n
        / (statistics[0].density_neutral.w + 1e-4), 0.0, 1.0);
    var layers: array<vec3<f32>, 9>;
    layers[0] = vec3<f32>(n);
    layers[1] = vec3<f32>(0.0, 0.0, ch.z);
    layers[2] = vec3<f32>(0.0, ch.y, 0.0);
    layers[3] = vec3<f32>(ch.x, 0.0, 0.0);
    layers[4] = (0.30 + 0.70 * h) * vec3<f32>(0.04, 0.08, 0.15);
    layers[5] = clamp(silver * vec3<f32>(0.18, 0.19, 0.20)
        + cl.yzx * vec3<f32>(0.03, 0.02, 0.04), vec3<f32>(0.0), vec3<f32>(0.28));
    layers[6] = (h * vec3<f32>(0.10, 0.06, 0.02)
        + (vec3<f32>(1.0) - dl.zxy) * vec3<f32>(0.13, 0.07, 0.18)
        + cl.yzx * vec3<f32>(0.04, 0.02, 0.06)) * (0.40 + 0.60 * h);
    layers[7] = (0.24 + 0.42 * h + 0.34 * numeric_max(warmth, strength))
        * vec3<f32>(0.02, 0.08, 0.19);
    layers[8] = h * vec3<f32>(0.03, 0.06, 0.11);
    var developed = vec3<f32>(0.0);
    for (var layer = 0u; layer < 9u; layer = layer + 1u) {
        developed = developed + coefficient(layer) * layers[layer];
    }
    let rgb = exp(-clamp(developed, vec3<f32>(0.0), vec3<f32>(8.0)) / 2.2);
    output[offset] = rgb.x;
    output[offset + 1u] = rgb.y;
    output[offset + 2u] = rgb.z;
}
