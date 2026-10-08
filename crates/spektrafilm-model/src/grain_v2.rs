//! Grain V2 accepts and returns linear scanner RGB; composition uses a fixed
//! display transfer internally. Integer gradient/phase hashing and Film
//! Resolution FIR are compatibility choices, not bit-exact Dehancer output.
use spektrafilm_math::{
    image::ImageBuf,
    precision::{from_f32, to_f32},
};

pub const CANVAS_WIDTH: u32 = 5200;
pub const CANVAS_HEIGHT: u32 = 3100;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GrainV2Mode {
    #[default]
    Analogue,
    Noise,
}
#[derive(Debug, Clone, Copy)]
pub struct GrainV2Profile {
    pub name: &'static str,
    pub scale: f32,
    pub amount: f32,
    pub shadows: f32,
    pub midtones: f32,
    pub highlights: f32,
    pub color: f32,
    pub resolution_factor: f32,
}
const fn profile(name: &'static str, values: [f32; 7]) -> GrainV2Profile {
    GrainV2Profile {
        name,
        scale: values[0],
        amount: values[1] / 100.0,
        shadows: values[2] / 100.0,
        midtones: values[3] / 100.0,
        highlights: values[4] / 100.0,
        color: values[5] / 100.0,
        resolution_factor: values[6],
    }
}
pub const PROFILE_NAMES: [&str; 12] = [
    "8mm50", "8mm250", "8mm500", "16mm50", "16mm250", "16mm500", "35mm50", "35mm250", "35mm500",
    "65mm50", "65mm250", "65mm500",
];
pub const PROFILES: [GrainV2Profile; 12] = [
    profile("8mm50", [40., 25., 30., 50., 50., 50., 50.]),
    profile("8mm250", [48., 50., 25., 45., 65., 65., 70.]),
    profile("8mm500", [48., 70., 30., 45., 65., 90., 75.]),
    profile("16mm50", [25., 25., 30., 55., 65., 65., 65.]),
    profile("16mm250", [25., 45., 30., 55., 65., 65., 75.]),
    profile("16mm500", [25., 70., 30., 45., 65., 75., 80.]),
    profile("35mm50", [8., 30., 30., 55., 65., 65., 70.]),
    profile("35mm250", [12., 35., 30., 55., 65., 65., 75.]),
    profile("35mm500", [16., 40., 35., 55., 65., 70., 80.]),
    profile("65mm50", [1., 10., 30., 45., 55., 65., 90.]),
    profile("65mm250", [2., 15., 30., 55., 65., 65., 90.]),
    profile("65mm500", [3., 25., 25., 55., 65., 80., 100.]),
];
pub fn profile_index(name: &str) -> Option<usize> {
    PROFILE_NAMES.iter().position(|&n| n == name)
}
#[derive(Debug, Clone, Copy)]
pub struct GrainV2Params {
    pub mode: GrainV2Mode,
    pub profile: usize,
    /// Host grainResolutionType: 0 = Negative (optical), 1 = Positive (fast box).
    pub film_type: u32,
    pub size: f32,
    pub amount: f32,
    pub shadows: f32,
    pub midtones: f32,
    pub highlights: f32,
    pub resolution_factor: f32,
    pub seed: u32,
    pub color: f32,
    pub cluster_size: f32,
    pub rotation: f32,
    pub colored: bool,
    pub clustered: bool,
}
impl Default for GrainV2Params {
    fn default() -> Self {
        Self::for_profile(7)
    }
}
impl GrainV2Params {
    pub fn for_profile(index: usize) -> Self {
        let p = PROFILES[index.min(11)];
        Self {
            mode: GrainV2Mode::Analogue,
            profile: index.min(11),
            // Host grainResolutionType: Negative = 0 (optical),
            // Positive = 1 (fast box).
            film_type: 1,
            size: p.scale,
            amount: p.amount,
            shadows: p.shadows,
            midtones: p.midtones,
            highlights: p.highlights,
            resolution_factor: p.resolution_factor,
            seed: 0,
            color: p.color,
            cluster_size: 1.6,
            rotation: 1.,
            colored: true,
            clustered: true,
        }
    }
    pub fn profile_data(self) -> GrainV2Profile {
        PROFILES[self.profile.min(11)]
    }
    pub fn profile_name(self) -> &'static str {
        self.profile_data().name
    }
    pub fn resampler_scale(self) -> f32 {
        1.0 + (self.size - 1.0) / 47.0 * 1.5
    }
    /// Radius in output pixels; source FastBlur is approximated by a fractional
    /// separable box FIR, and optical mode uses a Gaussian FIR.
    pub fn resolution_radius(self, width: u32, height: u32) -> f32 {
        let gsf = (5200.0 / width.max(1) as f32).max(3100.0 / height.max(1) as f32);
        let a = self.amount.clamp(0., 1.);
        let s = 1.0 + (self.size - 1.0) / 47.0;
        if self.mode == GrainV2Mode::Noise {
            return s * (1. - self.resolution_factor.clamp(0., 100.) / 100.) / gsf
                * 1.87
                * effective_control(a);
        }
        s * (1.0 - self.resolution_factor.clamp(0., 100.) / 100.) / gsf
            * if self.film_type == 1 { 1.2 } else { 1.6 }
            * (0.7 * a * a + 0.3 * a + 0.05)
    }
}
#[inline]
fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^ (x >> 16)
}
#[inline]
fn unit(x: u32) -> f32 {
    (x >> 8) as f32 / 16777216.0
}
#[inline]
fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6. - 15.) + 10.)
}
/// Independent integer gradient hash: unlike a sine hash, small differences
/// in CPU/GPU transcendental rounding do not select unrelated gradients.
fn pnoise(p: [f32; 3], seed: u32) -> f32 {
    let cell = p.map(|v| v.floor() as i32);
    let f = p.map(|v| v - v.floor());
    let u = f.map(fade);
    let mut sum = 0.;
    for z in 0..2 {
        for y in 0..2 {
            for x in 0..2 {
                let h = hash(
                    (cell[0] + x) as u32
                        ^ ((cell[1] + y) as u32).wrapping_mul(0x9e3779b9)
                        ^ ((cell[2] + z) as u32).wrapping_mul(0x85ebca6b)
                        ^ seed,
                );
                let g = [
                    unit(h) * 2. - 1.,
                    unit(hash(h)) * 2. - 1.,
                    unit(hash(h ^ 0x51ed270b)) * 2. - 1.,
                ];
                let d =
                    g[0] * (f[0] - x as f32) + g[1] * (f[1] - y as f32) + g[2] * (f[2] - z as f32);
                sum += d
                    * if x == 0 { 1. - u[0] } else { u[0] }
                    * if y == 0 { 1. - u[1] } else { u[1] }
                    * if z == 0 { 1. - u[2] } else { u[2] };
            }
        }
    }
    sum
}
fn encode_display(value: f32) -> f32 {
    if value < 0.018 {
        value * 4.5
    } else {
        1.099 * value.powf(0.45) - 0.099
    }
}
fn decode_display(value: f32) -> f32 {
    if value < 0.081 {
        value / 4.5
    } else {
        ((value + 0.099) / 1.099).powf(1. / 0.45)
    }
}
fn rotated(pos: [f32; 2], angle: f32, aspect: f32) -> [f32; 2] {
    let x = (pos[0] - 0.5) * aspect;
    let y = pos[1] - 0.5;
    [
        (x * angle.cos() - y * angle.sin()) / aspect + 0.5,
        x * angle.sin() + y * angle.cos() + 0.5,
    ]
}
// Content-only phase: identical RGB has identical phase regardless of position.
// A continuous integer-gradient field avoids hashing backend-dependent float bits.
fn color_phase(rgb: [f32; 3], seed: u32) -> f32 {
    let timer = (seed & 65535) as f32 / 65536.;
    timer * 0.01 + pnoise(rgb, seed) * 0.99
}
fn generator(
    pos: [f32; 2],
    size: [f32; 2],
    luma: f32,
    rgb: [f32; 3],
    p: GrainV2Params,
    digital: bool,
) -> [f32; 3] {
    let timer = if digital {
        color_phase(rgb, p.seed)
    } else {
        (p.seed & 65535) as f32 / 65536.
    };
    let scale = p.size.clamp(0.5, 1.4);
    let mut angles = [1.425, 3.892, 5.835].map(|a| a * scale * p.rotation);
    if p.clustered && !digital {
        // Hash virtual texels, not a smooth full-frame rotation field. The latter
        // folds globally rotated coordinates into coherent ridges in flat skies.
        let x = (pos[0] * size[0] + 0.5).floor() as u32;
        let y = (pos[1] * size[1] + 0.5).floor() as u32;
        let h = hash(x ^ y.wrapping_mul(0x9e3779b9) ^ p.seed);
        angles = [h, hash(h), hash(h ^ 0x51ed270b)].map(|v| (unit(v) * 2. - 1.) * p.rotation);
    }
    let coords = if digital { pos } else { pos.map(|v| v / scale) };
    let den = if digital {
        (1. + (p.size - 1.) / 47.) * 2.4 * (size[0] / 1920.).max(size[1] / 1080.)
    } else {
        p.cluster_size.max(0.01)
    };
    let mut n = [0.; 3];
    for c in 0..3 {
        let angle = if digital {
            timer + [1.425, 3.892, 5.835][c]
        } else {
            angles[c]
        };
        let q = rotated(coords, angle, size[0] / size[1]);
        let v = [q[0] * size[0] / den, q[1] * size[1] / den, timer + c as f32];
        n[c] = pnoise(v, p.seed);
        if c == 0 && !digital {
            n[c] = mix(n[c], pnoise([v[0], v[1], timer * 0.5 + 1.], p.seed), luma);
        }
    }
    for c in 1..3 {
        n[c] = mix(
            n[0],
            n[c],
            if p.colored { p.color.clamp(0., 1.) } else { 0. },
        );
    }
    n.map(|v| v + 0.5)
}
/// Nine-tap clamped texture sampling; the callback permits focused tests.
pub fn resample_3x3(mut sample: impl FnMut(i32, i32) -> [f32; 3], x: i32, y: i32) -> [f32; 3] {
    let mut sum = [0.; 3];
    for dy in -1..=1 {
        for dx in -1..=1 {
            let v = sample(x + dx, y + dy);
            for c in 0..3 {
                sum[c] += v[c] / 9.;
            }
        }
    }
    sum
}
fn overlay(b: f32, g: f32) -> f32 {
    if b < 0.5 {
        2. * b * g
    } else {
        1. - 2. * (1. - b) * (1. - g)
    }
    .clamp(0., 1.)
}
fn opacity(v: f32, c: f32) -> f32 {
    (-0.5 * ((v - c) * 5.).powi(2)).exp()
}
fn weight(d: i32, r: f32, optical: bool) -> f32 {
    if optical {
        (-0.5 * (d as f32 / r.max(0.001)).powi(2)).exp()
    } else {
        (r + 1. - (d.abs() as f32)).clamp(0., 1.)
    }
}
#[inline]
fn effective_control(value: f32) -> f32 {
    let t = value.clamp(0., 1.);
    0.12 * t * t + 0.68 * t + 0.2
}
pub fn apply_cpu(input: &ImageBuf, p: GrainV2Params) -> ImageBuf {
    if input.width == 0 || input.height == 0 {
        return input.clone();
    }
    let optical = p.film_type == 0;
    let radius = p.resolution_radius(input.width, input.height);
    let reach = if optical {
        (radius * 3.).ceil()
    } else {
        radius.ceil()
    } as i32;
    // Fixed display working transfer; destination primaries remain unchanged.
    let mut source = input.clone();
    source
        .data
        .iter_mut()
        .for_each(|value| *value = from_f32(encode_display(to_f32(*value))));
    if radius > 0. {
        let mut tmp = source.clone();
        for axis in 0..2 {
            for y in 0..input.height {
                for x in 0..input.width {
                    let mut value = [0.; 3];
                    let mut total = 0.;
                    for d in -reach..=reach {
                        let w = weight(d, radius, optical);
                        let xx = (x as i32 + if axis == 0 { d } else { 0 })
                            .clamp(0, input.width as i32 - 1)
                            as u32;
                        let yy = (y as i32 + if axis == 1 { d } else { 0 })
                            .clamp(0, input.height as i32 - 1)
                            as u32;
                        let v = source.get(xx, yy).map(to_f32);
                        for c in 0..3 {
                            value[c] += v[c] * w;
                        }
                        total += w;
                    }
                    tmp.set(x, y, value.map(|v| from_f32(v / total)));
                }
            }
            std::mem::swap(&mut source, &mut tmp);
        }
    }
    let gsf = (5200. / input.width as f32).max(3100. / input.height as f32);
    let size = [input.width as f32 * gsf, input.height as f32 * gsf];
    let mut out = source.clone();
    for y in 0..input.height {
        for x in 0..input.width {
            let rgb = source.get(x, y).map(to_f32);
            let luma = rgb[0] * 0.2125 + rgb[1] * 0.7154 + rgb[2] * 0.0721;
            let uv = [
                x as f32 / (input.width - 1).max(1) as f32,
                y as f32 / (input.height - 1).max(1) as f32,
            ];
            let g = if p.mode == GrainV2Mode::Noise {
                generator(
                    uv,
                    [input.width as f32, input.height as f32],
                    luma,
                    rgb,
                    p,
                    true,
                )
            } else {
                let gx = (uv[0] * size[0] / p.resampler_scale()) as i32;
                let gy = (uv[1] * size[1] / p.resampler_scale()) as i32;
                resample_3x3(
                    |a, b| {
                        generator(
                            [
                                a.clamp(0, size[0] as i32 - 1) as f32 / size[0],
                                b.clamp(0, size[1] as i32 - 1) as f32 / size[1],
                            ],
                            size,
                            luma,
                            rgb,
                            p,
                            false,
                        )
                    },
                    gx,
                    gy,
                )
            };
            let a_raw = p.amount.clamp(0., 1.);
            let a = effective_control(a_raw)
                * if p.mode == GrainV2Mode::Noise {
                    0.5
                } else {
                    1.
                };
            let ws = effective_control(p.shadows) * a * opacity(luma, 0.) * 2.;
            let wm = effective_control(p.midtones) * a * opacity(luma, 0.5);
            let wh = effective_control(p.highlights) * a * opacity(luma, 1.) * 2.;
            let mut result = [0.; 3];
            for c in 0..3 {
                let b = rgb[c];
                let s = mix(b, overlay(b.max(0.).powf(0.8), g[c]), ws);
                let m = mix(s, overlay(s, g[c]), wm);
                let h = mix(m, overlay(m - 0.2, g[c]), wh);
                result[c] = decode_display(mix(b, h, 0.5));
            }
            out.set(x, y, result.map(from_f32));
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_amount_still_runs_pipeline() {
        let i = ImageBuf::from_data(2, 2, vec![0.4; 12]);
        let mut p = GrainV2Params::default();
        p.amount = 0.;
        let out = apply_cpu(&i, p);
        assert!(out.data.iter().all(|v| to_f32(*v).is_finite()));
        assert_ne!(out.data, i.data);
    }

    #[test]
    fn film_types_select_distinct_resolution_paths() {
        let image = ImageBuf::from_data(
            192,
            108,
            (0..192 * 108)
                .flat_map(|i| [from_f32(if i % 192 < 96 { 0.1 } else { 0.8 }); 3])
                .collect(),
        );
        for mode in [GrainV2Mode::Analogue, GrainV2Mode::Noise] {
            let mut params = GrainV2Params::default();
            params.mode = mode;
            params.size = 48.;
            params.amount = 1.;
            params.resolution_factor = 0.;
            params.film_type = 0;
            let negative = apply_cpu(&image, params);
            params.film_type = 1;
            let positive = apply_cpu(&image, params);
            let edge = |img: &ImageBuf| to_f32(img.get(96, 54)[0]) - to_f32(img.get(95, 54)[0]);
            assert!((edge(&negative) - edge(&positive)).abs() > 1e-3, "{mode:?}");
        }
    }
    #[test]
    fn modes_resolution_and_color() {
        let i = ImageBuf::from_data(
            8,
            6,
            (0..144).map(|v| from_f32((v % 11) as f32 / 11.)).collect(),
        );
        let mut p = GrainV2Params::default();
        let a = apply_cpu(&i, p);
        assert_eq!(a.data, apply_cpu(&i, p).data);
        p.mode = GrainV2Mode::Noise;
        let b = apply_cpu(&i, p);
        assert_ne!(a.data, b.data);
        p.resolution_factor = 0.;
        p.film_type = 0;
        assert!(apply_cpu(&i, p).data.iter().all(|v| v.is_finite()));
        p.colored = false;
        assert!(apply_cpu(&i, p).data.iter().all(|v| v.is_finite()));
    }
    #[test]
    fn noise_film_resolution_reduces_edge_contrast() {
        let image = ImageBuf::from_data(
            192,
            108,
            (0..192 * 108)
                .flat_map(|i| [from_f32(if i % 192 < 96 { 0.1 } else { 0.8 }); 3])
                .collect(),
        );
        let mut p = GrainV2Params::default();
        p.mode = GrainV2Mode::Noise;
        p.size = 48.;
        p.shadows = 0.;
        p.midtones = 0.;
        p.highlights = 0.;
        p.resolution_factor = 100.;
        let sharp = apply_cpu(&image, p);
        p.resolution_factor = 0.;
        let blurred = apply_cpu(&image, p);
        let edge = |img: &ImageBuf| to_f32(img.get(96, 54)[0]) - to_f32(img.get(95, 54)[0]);
        assert!(edge(&blurred) < edge(&sharp) * 0.98);
    }

    #[test]
    fn noise_scale_preserves_relative_grain_size() {
        let mut p = GrainV2Params::default();
        p.mode = GrainV2Mode::Noise;
        let a = generator([0.37, 0.61], [1920., 1080.], 0.5, [0.5; 3], p, true);
        let b = generator([0.37, 0.61], [3840., 2160.], 0.5, [0.5; 3], p, true);
        assert!(a.iter().zip(b).all(|(x, y)| (*x - y).abs() < 1e-6));
    }
    #[test]
    fn middle_gray_responds_to_midtones_not_shadows() {
        let image = ImageBuf::from_data(64, 32, vec![from_f32(0.18); 64 * 32 * 3]);
        let mut params = GrainV2Params::default();
        params.resolution_factor = 100.;
        params.highlights = 0.;
        params.shadows = 0.;
        params.midtones = 1.;
        let mid = apply_cpu(&image, params);
        params.shadows = 1.;
        params.midtones = 0.;
        let shadow = apply_cpu(&image, params);
        let variance = |img: &ImageBuf| {
            let mean = img.data.iter().map(|v| to_f32(*v)).sum::<f32>() / img.data.len() as f32;
            img.data
                .iter()
                .map(|v| (to_f32(*v) - mean).powi(2))
                .sum::<f32>()
                / img.data.len() as f32
        };
        assert!(
            variance(&mid) > variance(&shadow) * 2.,
            "linear middle gray must not select the shadow bell"
        );
    }

    #[test]
    fn noise_phase_distinguishes_equal_luma_colors() {
        let a = [0.6, 0.4, 0.3];
        let b = [0.4, 0.4 + 0.2 * 0.2125 / 0.7154, 0.3];
        let luma = |v: [f32; 3]| v[0] * 0.2125 + v[1] * 0.7154 + v[2] * 0.0721;
        assert!((luma(a) - luma(b)).abs() < 1e-6);
        assert!((color_phase(a, 42) - color_phase(b, 42)).abs() > 1e-3);
    }

    #[test]
    fn clustered_sky_has_no_coherent_directional_ridges() {
        let (w, h) = (1024u32, 576u32);
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for _ in 0..w {
                data.extend([from_f32(0.25 + 0.55 * y as f32 / (h - 1) as f32); 3]);
            }
        }
        let mut p = GrainV2Params::default();
        p.seed = 42;
        let out = apply_cpu(&ImageBuf::from_data(w, h, data), p);
        let mut r: Vec<f32> = out
            .data
            .chunks_exact(3)
            .map(|v| (to_f32(v[0]) + to_f32(v[1]) + to_f32(v[2])) / 3.)
            .collect();
        for row in r.chunks_exact_mut(w as usize) {
            let mean = row.iter().sum::<f32>() / w as f32;
            row.iter_mut().for_each(|v| *v -= mean);
        }
        let mut squared = 0.;
        let mut count = 0;
        for by in (0..h as usize - 32).step_by(32) {
            for bx in (0..w as usize - 32).step_by(32) {
                let (mut energy, mut dx, mut dy) = (0., 0., 0.);
                for y in by..by + 31 {
                    for x in bx..bx + 31 {
                        let i = y * w as usize + x;
                        energy += r[i] * r[i];
                        dx += r[i] * r[i + 1];
                        dy += r[i] * r[i + w as usize];
                    }
                }
                squared += ((dx - dy) / energy).powi(2);
                count += 1;
            }
        }
        let directional_rms = (squared / count as f32).sqrt();
        assert!(
            directional_rms < 0.10,
            "coherent sky ridges: {directional_rms}"
        );
    }
    #[test]
    fn gpu_matches_cpu_reference_when_adapter_is_available() {
        use spektrafilm_gpu::wgpu_backend::WgpuBackend;
        let Some(gpu) = WgpuBackend::new() else {
            eprintln!("Grain V2 GPU parity skipped: no adapter");
            return;
        };
        let image = ImageBuf::from_data(
            37,
            19,
            (0..37 * 19 * 3)
                .map(|v| from_f32((v % 17) as f32 / 17.0))
                .collect(),
        );
        for profile in 0..PROFILES.len() {
            for film_type in [0, 1] {
                for mode in [GrainV2Mode::Analogue, GrainV2Mode::Noise] {
                    for control in [0., 1.] {
                        let mut params = GrainV2Params::for_profile(profile);
                        params.film_type = film_type;
                        params.mode = mode;
                        params.seed = 42;
                        params.amount = control;
                        params.shadows = control;
                        params.midtones = control;
                        params.highlights = control;
                        params.color = control;
                        let cpu = apply_cpu(&image, params);
                        let gpu_params = spektrafilm_gpu::GrainV2GpuParams {
                            mode: params.mode as u32,
                            film_type: params.film_type,
                            amount: params.amount,
                            shadows: params.shadows,
                            midtones: params.midtones,
                            highlights: params.highlights,
                            raw_scale: params.size,
                            cluster_size: params.cluster_size,
                            rotation: params.rotation,
                            color: params.color,
                            resolution_factor: params.resolution_factor,
                            seed: params.seed,
                            colored: params.colored,
                            clustered: params.clustered,
                        };
                        let gpu_image = gpu.grain_v2_gpu(&image, &gpu_params);
                        assert!(
                            cpu.data.iter().all(|v| to_f32(*v).is_finite()),
                            "CPU Grain V2 output contains non-finite samples"
                        );
                        assert!(
                            gpu_image.data.iter().all(|v| to_f32(*v).is_finite()),
                            "GPU Grain V2 output contains non-finite samples"
                        );
                        let max_error = cpu
                            .data
                            .iter()
                            .zip(&gpu_image.data)
                            .map(|(a, b)| (to_f32(*a) - to_f32(*b)).abs())
                            .fold(0.0f32, f32::max);
                        assert!(max_error < 5e-3, "CPU/GPU Grain V2 drift: {max_error}");
                    }
                }
            }
        }
    }
}
