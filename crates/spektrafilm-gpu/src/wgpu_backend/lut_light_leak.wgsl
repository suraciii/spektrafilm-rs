// RGB light leak: a tinted additive spill from a gap on the frame edge.
//
// The aperture follows the same perimeter contract as the Film camera light
// leak: position walks clockwise around the frame edge, width runs along the
// tangent, spread falls inward, and angle tilts the segment. A warm shoulder
// and a narrower hot core burn toward white with exposure. Seeded texture adds
// fine streaks only; this stage reads no film data.

struct Params {
    n: u32,
    first_pixel: u32,
    image_width: u32,
    image_height: u32,
    gain: f32,
    tint_r: f32,
    tint_g: f32,
    tint_b: f32,
    position: f32,
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
const LEAK_SHOULDER_SCALE: f32 = 0.48;
const LEAK_CORE_RATIO: f32 = 0.55;
const LEAK_BURN_START: f32 = 1.2;
const LEAK_BURN_END: f32 = 2.4;
const LEAK_HOTSPOT_CENTER: f32 = 0.05;
const LEAK_HOTSPOT_WIDTH: f32 = 0.55;
const LEAK_HOTSPOT_FLOOR: f32 = 0.12;
const LEAK_CORE_BOOST: f32 = 0.35;
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

fn perimeter_point(t: f32, aspect: f32) -> vec4<f32> {
    let w = 2.0 * aspect;
    let h = 2.0;
    let perimeter = 2.0 * (w + h);
    var d = fract(t) * perimeter;
    if d < w {
        return vec4<f32>(-aspect + d, -1.0, 1.0, 0.0);
    }
    d = d - w;
    if d < h {
        return vec4<f32>(aspect, -1.0 + d, 0.0, 1.0);
    }
    d = d - h;
    if d < w {
        return vec4<f32>(aspect - d, 1.0, -1.0, 0.0);
    }
    d = d - w;
    return vec4<f32>(-aspect, 1.0 - d, 0.0, -1.0);
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
    let perimeter = perimeter_point(p.position, aspect);
    let center = perimeter.xy;
    let tangent = perimeter.zw;
    let direction = vec2<f32>(
        tangent.x * cos(angle) - tangent.y * sin(angle),
        tangent.x * sin(angle) + tangent.y * cos(angle),
    );
    let offset = vec2<f32>(frame_x, frame_y) - center;
    let along = dot(offset, direction);
    let across = dot(offset, vec2<f32>(-direction.y, direction.x));
    let closest = clamp(along, -0.5 * p.width, 0.5 * p.width);
    let edge = offset - direction * closest;
    let distance = length(edge);

    let texture_on = p.texture_amount > 0.0 && p.texture_scale > 0.0;
    let local_spread = max(p.spread, 1.0e-6);
    let q = distance / local_spread;
    let shoulder = exp(-q * q);
    let core_width = local_spread * LEAK_CORE_RATIO;
    let half = 0.5 * p.width;
    let aperture_t = select(0.0, clamp(along / half, -1.0, 1.0), half > 0.0);
    let hotspot = exp(
        -pow((aperture_t - LEAK_HOTSPOT_CENTER) / LEAK_HOTSPOT_WIDTH, 2.0),
    );
    let core = exp(-(distance / core_width) * (distance / core_width))
        * (LEAK_HOTSPOT_FLOOR + (1.0 - LEAK_HOTSPOT_FLOOR) * hotspot);

    var modulation = 1.0;
    if texture_on {
        let texture_scale = max(p.texture_scale, 1.0e-6);
        let streaks = streak_noise(
            along / (texture_scale * 3.0),
            across / texture_scale,
            p.seed,
        );
        modulation = 1.0 - 0.5 * p.texture_amount + 0.5 * p.texture_amount * streaks;
    }

    let amplitude = p.gain * LEAK_SHOULDER_SCALE * shoulder * modulation;
    let core_amplitude = p.gain * core * modulation;
    let burn = smoothstep1(
        (core_amplitude - LEAK_BURN_START) / (LEAK_BURN_END - LEAK_BURN_START),
    );
    let tint = vec3<f32>(p.tint_r, p.tint_g, p.tint_b);
    let hot = vec3<f32>(1.0, 0.92, 0.80);
    let core_orange = core_amplitude * LEAK_CORE_BOOST * (1.0 - burn);
    let base = index * 3u;
    pixels[base] = source[base] + amplitude * tint.r + core_orange * tint.r
        + core_amplitude * (hot.r - tint.r) * burn;
    pixels[base + 1u] = source[base + 1u] + amplitude * tint.g + core_orange * tint.g
        + core_amplitude * (hot.g - tint.g) * burn;
    pixels[base + 2u] = source[base + 2u] + amplitude * tint.b + core_orange * tint.b
        + core_amplitude * (hot.b - tint.b) * burn;
}
