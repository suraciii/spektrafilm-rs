struct Params {
    n: u32,
    size: u32,
    shaper_size: u32,
    clamp_domain: u32,
    domain_min: vec4<f32>,
    domain_span: vec4<f32>,
    strength: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;
@group(0) @binding(2) var<storage, read> original: array<f32>;
@group(0) @binding(3) var<storage, read> table: array<f32>;
@group(0) @binding(4) var<storage, read> shaper: array<f32>;
@group(0) @binding(5) var<storage, read_write> output: array<f32>;

fn component(v: vec3<f32>, axis: u32) -> f32 {
    if axis == 0u { return v.x; }
    if axis == 1u { return v.y; }
    return v.z;
}

fn shaper_value(value: f32, channel: u32) -> f32 {
    if p.shaper_size == 0u { return value; }
    if p.shaper_size == 1u { return shaper[channel]; }
    let position = clamp(value, 0.0, 1.0) * f32(p.shaper_size - 1u);
    let low = u32(floor(position));
    let high = min(low + 1u, p.shaper_size - 1u);
    let f = position - f32(low);
    let a = shaper[low * 3u + channel];
    let b = shaper[high * 3u + channel];
    return a + f * (b - a);
}

fn lattice(r: u32, g: u32, b: u32) -> vec3<f32> {
    let base = ((r * p.size + g) * p.size + b) * 3u;
    return vec3<f32>(table[base], table[base + 1u], table[base + 2u]);
}

fn fail_output(index: u32) {
    let base = index * 3u;
    output[base] = bitcast<f32>(0x7fc00000u);
    output[base + 1u] = bitcast<f32>(0x7fc00000u);
    output[base + 2u] = bitcast<f32>(0x7fc00000u);
}

fn inside(value: vec3<f32>) -> bool {
    let high = p.domain_min.xyz + p.domain_span.xyz;
    return all(value >= p.domain_min.xyz) && all(value <= high);
}

fn enforce(value: vec3<f32>) -> vec3<f32> {
    let high = p.domain_min.xyz + p.domain_span.xyz;
    if p.clamp_domain != 0u {
        return clamp(value, p.domain_min.xyz, high);
    }
    return value;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = gid.x + gid.y * grid.x * 256u;
    if index >= p.n { return; }
    let base = index * 3u;
    let source = vec3<f32>(input[base], input[base + 1u], input[base + 2u]);
    var value = source;
    if !inside(value) {
        if p.clamp_domain != 0u {
            value = enforce(value);
        } else {
            fail_output(index);
            return;
        }
    }
    if p.shaper_size != 0u {
        value = vec3<f32>(
            shaper_value(value.x, 0u),
            shaper_value(value.y, 1u),
            shaper_value(value.z, 2u),
        );
    }
    if !inside(value) {
        if p.clamp_domain != 0u {
            value = enforce(value);
        } else {
            fail_output(index);
            return;
        }
    }

    var lut = lattice(0u, 0u, 0u);
    if p.size > 1u {
        let normalized = clamp((value - p.domain_min.xyz) / p.domain_span.xyz, vec3<f32>(0.0), vec3<f32>(1.0));
        let q = normalized * f32(p.size - 1u);
        let lo = min(vec3<u32>(floor(q)), vec3<u32>(p.size - 1u));
        let f = q - floor(q);
        var order = array<u32, 3>(0u, 1u, 2u);
        // Descending fractional order, matching the reference tetrahedral walk.
        if component(f, order[1]) > component(f, order[0]) {
            let t = order[0]; order[0] = order[1]; order[1] = t;
        }
        if component(f, order[2]) > component(f, order[1]) {
            let t = order[1]; order[1] = order[2]; order[2] = t;
        }
        if component(f, order[1]) > component(f, order[0]) {
            let t = order[0]; order[0] = order[1]; order[1] = t;
        }
        let hi = min(lo + vec3<u32>(1u), vec3<u32>(p.size - 1u));
        var idx = lo;
        lut = lattice(idx.x, idx.y, idx.z) * (1.0 - component(f, order[0]));
        for (var step = 0u; step < 3u; step++) {
            let axis = order[step];
            if axis == 0u { idx.x = hi.x; }
            if axis == 1u { idx.y = hi.y; }
            if axis == 2u { idx.z = hi.z; }
            var weight = component(f, axis);
            if step < 2u {
                weight -= component(f, order[step + 1u]);
            }
            lut += lattice(idx.x, idx.y, idx.z) * weight;
        }
    }
    let original_rgb = vec3<f32>(original[base], original[base + 1u], original[base + 2u]);
    let result = mix(original_rgb, lut, p.strength);
    output[base] = result.x;
    output[base + 1u] = result.y;
    output[base + 2u] = result.z;
}
