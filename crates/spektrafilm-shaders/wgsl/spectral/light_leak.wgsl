// In-place camera light leak: stray light that reached the film record during
// exposure, added to linear raw exposure before development.
//
// The field is the host module's field: a compact Gaussian falloff from an
// aperture segment on the frame perimeter, modulated by a seeded three-octave
// value noise stretched along the aperture. The lattice hash is 32-bit wrapping
// integer arithmetic, so the device and the host evaluate the same lattice
// values.
//
// One thread per pixel; it updates that pixel's three records. Coordinates
// come from the pixel's index in the frame, so the field does not depend on
// how the frame is chunked across dispatches.

struct Params {
    // The pointwise machinery patches the first two words per chunk with the
    // chunk's pixel count and the chunk's first item in the frame; the raster
    // extent follows them.
    n_pixels: u32,
    first_pixel: u32,
    width: u32,
    height: u32,
    center_x: f32,
    center_y: f32,
    direction_x: f32,
    direction_y: f32,
    half: f32,
    spread: f32,
    texture_scale: f32,
    texture_amount: f32,
    seed: u32,
    scale: f32,
    spectrum_r: f32,
    spectrum_g: f32,
    spectrum_b: f32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> data: array<f32>;

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

fn distance_to_segment(px: f32, py: f32) -> f32 {
    let ax = px - params.center_x;
    let ay = py - params.center_y;
    let along = clamp(
        ax * params.direction_x + ay * params.direction_y,
        -params.half,
        params.half,
    );
    let ex = ax - along * params.direction_x;
    let ey = ay - along * params.direction_y;
    return sqrt(ex * ex + ey * ey);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let idx = gid.x + gid.y * grid.x * 256u;
    if idx >= params.n_pixels {
        return;
    }
    // The chunk-local index locates the data; the coordinates are the
    // pixel's place in the frame.
    let pixel = params.first_pixel + idx;
    let px = f32(pixel % params.width);
    let py = f32(pixel / params.width);
    let d = distance_to_segment(px, py);
    let q = d / params.spread;
    // One spread of penumbra beyond one and a half spreads, then nothing: the
    // camera cavity shadows the film away from the gap.
    let falloff = exp(-q * q) * (1.0 - smoothstep1(q - 1.5));
    let ax = px - params.center_x;
    let ay = py - params.center_y;
    let along = ax * params.direction_x + ay * params.direction_y;
    let across = -ax * params.direction_y + ay * params.direction_x;
    let streaks = streak_noise(
        along / (params.texture_scale * 3.0),
        across / params.texture_scale,
        params.seed,
    );
    let texture = clamp(params.texture_amount, 0.0, 1.0);
    let weight = falloff * (1.0 - texture + texture * streaks);
    let base = idx * 3u;
    data[base] = data[base] + params.scale * params.spectrum_r * weight;
    data[base + 1u] = data[base + 1u] + params.scale * params.spectrum_g * weight;
    data[base + 2u] = data[base + 2u] + params.scale * params.spectrum_b * weight;
}
