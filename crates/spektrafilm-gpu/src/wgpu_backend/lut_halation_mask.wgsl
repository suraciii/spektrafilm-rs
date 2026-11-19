struct Params {
    n: u32,
    low: f32,
    high: f32,
    transition: f32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> mask: array<f32>;

fn smoothstep01(edge0: f32, edge1: f32, value: f32) -> f32 {
    if edge1 <= edge0 { return select(0.0, 1.0, value >= edge1); }
    let t = clamp((value - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = gid.x + gid.y * grid.x * 256u;
    if index >= p.n { return; }
    let base = index * 3u;
    let peak = max(source[base], max(source[base + 1u], source[base + 2u]));
    let transition = max(p.transition, 1.0e-30);
    let rise = smoothstep01(p.low, p.low + transition, peak);
    let fall = 1.0 - smoothstep01(p.high + transition, p.high + 2.0 * transition, peak);
    let gate = rise * fall;
    let span = max(1.0 - p.low, 1.0e-30);
    mask[base] = gate * max(source[base] - p.low, 0.0) / span;
    mask[base + 1u] = gate * max(source[base + 1u] - p.low, 0.0) / span;
    mask[base + 2u] = gate * max(source[base + 2u] - p.low, 0.0) / span;
}
