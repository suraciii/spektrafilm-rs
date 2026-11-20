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

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let index = gid.x + gid.y * grid.x * 256u;
    if index >= p.n { return; }

    let pixel = index + p.first_pixel;
    let x = f32(pixel % p.image_width);
    let y = f32(pixel / p.image_width);
    let image_width = f32(p.image_width);
    let image_height = f32(p.image_height);
    let aspect = image_width / image_height;
    let frame_x = (2.0 * (x + 0.5) / image_width - 1.0) * aspect;
    let frame_y = 2.0 * (y + 0.5) / image_height - 1.0;

    let angle = p.angle_deg * PI / 180.0;
    let sin_angle = sin(angle);
    let cos_angle = cos(angle);
    let dx = frame_x - p.pos_x * aspect;
    let dy = frame_y - p.pos_y;
    let along = cos_angle * dx + sin_angle * dy;
    let across = -sin_angle * dx + cos_angle * dy;
    let falloff = exp(-(along * along) / (2.0 * p.width * p.width)
        - (across * across) / (2.0 * p.spread * p.spread));

    var modulation = 1.0;
    if p.texture_amount > 0.0 && p.texture_scale > 0.0 {
        let phase = f32(p.seed % 1048576u) / 1048576.0 * 2.0 * PI;
        let u = along / p.width * p.texture_scale + phase;
        let v = across / p.spread * p.texture_scale * 1.37 - phase * 0.73;
        let pattern = 0.5 + 0.5 * sin(u) * sin(v);
        modulation = 1.0 - p.texture_amount + p.texture_amount * pattern;
    }

    let amplitude = p.gain * falloff * modulation;
    let base = index * 3u;
    pixels[base] = source[base] + amplitude * p.tint_r;
    pixels[base + 1u] = source[base + 1u] + amplitude * p.tint_g;
    pixels[base + 2u] = source[base + 2u] + amplitude * p.tint_b;
}
