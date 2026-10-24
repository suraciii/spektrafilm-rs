// LabScan scanner finish: the gradation curve, the shadow cast, and the
// three fixed Gaussian hue corrections, in Oklab.
//
// CPU equivalent: `finish::apply` / `finish_pixel` in
// `stages::scanning/finish.rs`, adapted from FotufilmCore's
// `LabScanFinish.swift`. The constants below are the same fixed values, and
// the two conversion matrices are resolved on the host from the output
// colour space, so the device port evaluates the same per-pixel arithmetic.
//
// `chromatic == 0` runs the gradation only, preserving channel neutrality
// for a monochrome scan.

const HIGHLIGHT_LIFT: f32 = 0.006;
const BLACK_PULL: f32 = 0.019;
const SHADOW_CAST_0: f32 = -0.045;
const SHADOW_CAST_1: f32 = -0.033;
const CAST_SATURATION: f32 = 0.1;
const CORRECTION_FADE_START: f32 = 0.08;
const CORRECTION_FADE_END: f32 = 0.20;

struct Correction {
    hue: f32,
    width: f32,
    chroma: f32,
    turn: f32,
}

const CORRECTIONS: array<Correction, 3> = array<Correction, 3>(
    Correction(45.0, 35.0, 0.97, 2.3),
    Correction(135.0, 35.0, 0.89, 5.5),
    Correction(240.0, 35.0, 1.05, -6.4),
);

struct Params {
    n_pixels: u32,
    chromatic: u32,
    _pad0: u32,
    _pad1: u32,
    strength: f32,
    _pad2: f32,
    _pad3: f32,
    _pad4: u32,
    output_to_srgb: mat3x3<f32>,
    srgb_to_output: mat3x3<f32>,
    weights: vec3<f32>,
    _pad5: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<f32>;          // [H*W*3]
@group(0) @binding(2) var<storage, read_write> output: array<f32>;   // [H*W*3]

fn clampv(value: f32, low: f32, high: f32) -> f32 {
    return clamp(value, low, high);
}

fn oklab(rgb: vec3<f32>) -> vec3<f32> {
    let l = pow(0.4122214708 * rgb.x + 0.5363325363 * rgb.y + 0.0514459929 * rgb.z, 1.0 / 3.0);
    let m = pow(0.2119034982 * rgb.x + 0.6806995451 * rgb.y + 0.1073969566 * rgb.z, 1.0 / 3.0);
    let s = pow(0.0883024619 * rgb.x + 0.2817188376 * rgb.y + 0.6299787005 * rgb.z, 1.0 / 3.0);
    return vec3<f32>(
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    );
}

fn linear_from_oklab(lab: vec3<f32>) -> vec3<f32> {
    let l = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
    let m = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
    let s = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
    let l3 = l * l * l;
    let m3 = m * m * m;
    let s3 = s * s * s;
    return vec3<f32>(
        4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
        -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
        -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3,
    );
}

/// The luminance gradation curve: a cube-root level lift and black pull.
fn graded(rgb: vec3<f32>, strength: f32) -> vec3<f32> {
    let luminance = params.weights.x * rgb.x + params.weights.y * rgb.y + params.weights.z * rgb.z;
    if luminance <= 1e-6 || !(luminance == luminance) {
        return rgb;
    }
    let level = pow(luminance, 1.0 / 3.0);
    let upper = clampv((level - 0.55) / 0.45, 0.0, 1.0);
    let lifted = level
        + strength * (HIGHLIGHT_LIFT * sin(3.141592653589793 * upper)
            - BLACK_PULL * clampv(1.0 - level / 0.35, 0.0, 1.0));
    let lifted_target = max(lifted, 0.0);
    let scale = lifted_target * lifted_target * lifted_target / luminance;
    return rgb * scale;
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) grid: vec3<u32>) {
    let pixel = gid.x + gid.y * grid.x * 256u;
    if pixel >= params.n_pixels {
        return;
    }
    let base = pixel * 3u;
    let strength = params.strength;
    let rgb = vec3<f32>(input[base], input[base + 1u], input[base + 2u]);
    let toned = graded(rgb, strength);
    if params.chromatic == 0u {
        output[base] = toned.x;
        output[base + 1u] = toned.y;
        output[base + 2u] = toned.z;
        return;
    }

    var lab = oklab(params.output_to_srgb * toned);
    let level = lab.x;
    // The simple core API has no per-frame floor; LabScanFinish's default
    // floor of zero makes the crossover onset fully active.
    let onset = 1.0;
    let shadow = clampv((0.70 - level) / 0.40, 0.0, 1.0)
        * clampv((level - 0.04) / 0.16, 0.0, 1.0)
        * onset;
    let chroma = sqrt(lab.y * lab.y + lab.z * lab.z);
    let saturation = chroma / max(level, 1e-3);
    let neutral = level * exp(-saturation / CAST_SATURATION);
    let cast_amount = strength * neutral * shadow;
    lab.y += cast_amount * SHADOW_CAST_0;
    lab.z += cast_amount * SHADOW_CAST_1;

    let chroma2 = sqrt(lab.y * lab.y + lab.z * lab.z);
    var hue = atan2(lab.z, lab.y) * 180.0 / 3.141592653589793;
    var gain = 1.0;
    var turn = 0.0;
    for (var i = 0u; i < 3u; i++) {
        let correction = CORRECTIONS[i];
        var distance = (hue - correction.hue) % 360.0;
        if distance > 180.0 {
            distance -= 360.0;
        }
        if distance < -180.0 {
            distance += 360.0;
        }
        let ratio = distance / correction.width;
        let weight = exp(-0.5 * ratio * ratio);
        gain *= 1.0 + strength * (correction.chroma - 1.0) * weight;
        turn += strength * correction.turn * weight;
    }
    let saturated = clampv(
        (chroma2 - CORRECTION_FADE_START) / (CORRECTION_FADE_END - CORRECTION_FADE_START),
        0.0,
        1.0,
    );
    let smoothstep = saturated * saturated * (3.0 - 2.0 * saturated);
    let fade = clampv(level / 0.25, 0.0, 1.0)
        * clampv((1.0 - level) / 0.08, 0.0, 1.0)
        * (1.0 - smoothstep);
    let corrected_chroma = chroma2 * (1.0 + (gain - 1.0) * fade);
    let corrected_hue = (hue + turn * fade) * 3.141592653589793 / 180.0;
    lab.y = corrected_chroma * cos(corrected_hue);
    lab.z = corrected_chroma * sin(corrected_hue);
    let finished = params.srgb_to_output * linear_from_oklab(lab);

    output[base] = finished.x;
    output[base + 1u] = finished.y;
    output[base + 2u] = finished.z;
}
