// RGB light leak: a tinted additive spill from a gap on the frame edge.
//
// The field is a wide band with a soft Gaussian core and a short feather
// inward. Seeded value noise streaks along the gap, roughens its edge, and
// slowly varies its intensity, so the result reads as one irregular edge stain
// rather than a uniform tube or a symmetric spindle. The noise is the same
// integer lattice used by the Film camera-light-leak field, but the geometry
// is frame-relative and this stage reads no film data.

struct Params {
    n: u32,
    first_pixel: u32,
    image_width: u32,
    image_height: u32,
    gain: f32,
    tint_r: f32,
    tint_g: f32,
    tint_b: f32,
    pos_x: f32,
    pos_y: f32,
    angle_deg: f32,
    width: f32,
    spread: f32,
    texture_amount: f32,
    texture_scale: f32,
    seed: u32,
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> pixels: array<f32>;

const PI: f32 = 3.141592653589793;

fn hash2(x: u32, y: u32, seed: u32) -> f32 {
    var h = seed ^ 0x9E3779B9u;
    h = (h ^ (x * 0xC2B2AE3Du)) << 15u | (h ^ (x * 0xC2B2AE3Du)) >> 17u;
    h = h * 0x85EBCA77u;
    h = (h ^ (y * 0x27D4EB2Fu)) << 13u | (h ^ (y * 0x27D4EB2Fu)) >> 19u;
    h = h * 0x165667B1u;
    h = h ^ (h >> 15u);
    h = h * 0x9E377F1Bu;
    h = h ^ (h >> 13u);
    return f32(h >> 8u) / 16777216.0;
}

fn smoothstep1(t: f32) -> f32 {
    let c = clamp(t, 0.0, 1.0);
    return c * c * (3.0 - 2.0 * c);
}

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let x0 = floor(x);
    let y0 = floor(y);
    let fx = smoothstep1(x - x0);
    let fy = smoothstep1(y - y0);
    let ix = i32(x0);
    let iy = i32(y0);
    let n00 = hash2(u32(ix), u32(iy), seed);
    let n10 = hash2(u32(ix) + 1u, u32(iy), seed);
    let n01 = hash2(u32(ix), u32(iy) + 1u, seed);
    let n11 = hash2(u32(ix) + 1u, u32(iy) + 1u, seed);
    let top = n00 + (n10 - n00) * fx;
    let bottom = n01 + (n11 - n01) * fx;
    return top + (bottom - top) * fy;
}

fn streak_noise(x: f32, y: f32, seed: u32) -> f32 {
    var sum = 0.0;
    var norm = 0.0;
    var amp = 1.0;
    var fx = x;
    var fy = y;
    for (var octave = 0u; octave < 3u; octave = octave + 1u) {
        sum = sum + amp * value_noise(fx, fy, seed + octave);
        norm = norm + amp;
        amp = amp * 0.5;
        fx = fx * 2.0;
        fy = fy * 2.0;
    }
    return sum / norm;
}

fn seeded_band_structure(along: f32, seed: u32) -> vec2<f32> {
    let width = smoothstep1((streak_noise(along * 3.7, 0.37, seed + 0x9E3779B9u) - 0.2) / 0.6);
    let intensity = smoothstep1((streak_noise(along * 0.73, 0.73, seed + 0x85EBCA77u) - 0.2) / 0.6);
    return vec2<f32>(width, intensity);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = gid.x + gid.y * grid.x * 256u;
    if index >= p.n { return; }

    let pixel = index + p.first_pixel;
    let image_width = f32(p.image_width);
    let image_height = f32(p.image_height);
    let aspect = image_width / image_height;
    let x = f32(pixel % p.image_width);
    let y = f32(pixel / p.image_width);
    let frame_x = (2.0 * (x + 0.5) / image_width - 1.0) * aspect;
    let frame_y = 2.0 * (y + 0.5) / image_height - 1.0;

    let angle = p.angle_deg * PI / 180.0;
    let direction = vec2<f32>(cos(angle), sin(angle));
    let center = vec2<f32>(p.pos_x * aspect, p.pos_y);
    let offset = vec2<f32>(frame_x, frame_y) - center;
    let along = dot(offset, direction);
    let across = dot(offset, vec2<f32>(-direction.y, direction.x));
    let closest = clamp(along, -p.width, p.width);
    let edge = offset - direction * closest;
    let distance = length(edge);

    let texture_on = p.texture_amount > 0.0 && p.texture_scale > 0.0;
    var width_variation = 0.0;
    var band_envelope = 1.0;
    if texture_on {
        let slow = seeded_band_structure(along, p.seed);
        width_variation = p.texture_amount * 0.30 * (slow.x * 2.0 - 1.0);
        band_envelope = 1.0 - p.texture_amount * 0.85 * (1.0 - slow.y);
    }
    let local_spread = max(p.spread * (1.0 + width_variation), 1.0e-6);
    let q = distance / local_spread;
    let falloff = exp(-q * q);

    var modulation = 1.0;
    if texture_on {
        let texture_scale = 0.25 / p.texture_scale;
        let streaks = streak_noise(
            along / (texture_scale * 3.0),
            across / texture_scale,
            p.seed,
        );
        modulation = 1.0 - p.texture_amount + p.texture_amount * streaks;
    }

    let amplitude = p.gain * falloff * modulation * band_envelope;
    let base = index * 3u;
    pixels[base] = source[base] + amplitude * p.tint_r;
    pixels[base + 1u] = source[base + 1u] + amplitude * p.tint_g;
    pixels[base + 2u] = source[base + 2u] + amplitude * p.tint_b;
}
