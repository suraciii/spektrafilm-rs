//! Grain V2 accepts and returns native encoded RGB. Working-domain images and
//! generated grain use half storage boundaries. The noise arithmetic is shared
//! with WGSL; original OpenCL device transcendental results can still differ.
use spektrafilm_math::{
    grain::{fast_blur_weights, optical_weights},
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
    /// Profile metadata: 0 = Negative, 1 = Positive.
    ///
    /// Dehancer does not pass this field to the grain kernels. It remains
    /// separate from `resolution_type`, which selects Film Resolution.
    pub film_type: u32,
    /// Host Film Resolution implementation: 0 = OpticalResolution, 1 = FastBlur.
    pub resolution_type: u32,
    pub size: f32,
    pub amount: f32,
    pub shadows: f32,
    pub midtones: f32,
    pub highlights: f32,
    pub resolution_factor: f32,
    pub seed: u32,
    /// Explicit host timer; `None` derives the canonical phase from `seed`.
    pub timer: Option<f32>,
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
            // Bundled profile metadata is type=0 (Negative); Film Resolution
            // remains a separate profile field.
            film_type: 0,
            resolution_type: 1,
            size: p.scale,
            amount: p.amount,
            shadows: p.shadows,
            midtones: p.midtones,
            highlights: p.highlights,
            resolution_factor: p.resolution_factor,
            seed: 0,
            timer: None,
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
    /// Resolve the host timer while retaining the seed-based default.
    pub fn resolved_timer(self) -> f32 {
        self.timer
            .unwrap_or_else(|| spektrafilm_math::grain::seeded_phase(self.seed))
    }
    pub fn resampler_scale(self) -> f32 {
        1.0 + (self.size - 1.0) / 47.0 * 1.5
    }
    /// Film Resolution radius in output pixels; Noise deliberately returns zero.
    pub fn resolution_radius(self, width: u32, height: u32) -> f32 {
        if self.mode == GrainV2Mode::Noise {
            return 0.;
        }
        let gsf = (5200.0 / width.max(1) as f32).max(3100.0 / height.max(1) as f32);
        let a = self.amount.clamp(0., 1.);
        let s = 1.0 + (self.size - 1.0) / 47.0;
        s * (1.0 - self.resolution_factor.clamp(0., 100.) / 100.) / gsf
            * if self.resolution_type == 1 { 1.2 } else { 1.6 }
            * (0.7 * a * a + 0.3 * a + 0.05)
    }
}
#[inline]
fn hash(mut x: u32) -> u32 {
    x = x.wrapping_add(x << 10);
    x ^= x >> 6;
    x = x.wrapping_add(x << 3);
    x ^= x >> 11;
    x.wrapping_add(x << 15)
}
#[inline]
fn random(v: [f32; 4]) -> f32 {
    let h =
        hash(v[0].to_bits() ^ hash(v[1].to_bits()) ^ hash(v[2].to_bits()) ^ hash(v[3].to_bits()));
    f32::from_bits((h & 0x007fffff) | 0x3f800000) - 1.0
}
#[inline]
fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
#[inline]
fn fract(v: f32) -> f32 {
    v - v.floor()
}
#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6. - 15.) + 10.)
}
// The vendor grad4 computes an adjusted local p3 but returns p itself. Keep
// that observable return value while naming the helper after the source kernel.
fn grad4(j: f32, ip: [f32; 4]) -> [f32; 4] {
    let px = (fract(j * ip[0]) * 7.).floor() * ip[2] - 1.;
    let py = (fract(j * ip[1]) * 7.).floor() * ip[2] - 1.;
    let pz = (fract(j * ip[2]) * 7.).floor() * ip[2] - 1.;
    [px, py, pz, 1.5 - (px.abs() + py.abs() + pz.abs())]
}
fn snoise(timer: f32, v: [f32; 4]) -> [f32; 4] {
    let r = random(v);
    let ip = random(v.map(|x| mix(r, timer, timer * x)));
    grad4(0.5, [ip; 4])
}
// Independent binary32 trig with split constants and ordinary multiply/add.
// The small-range products are exact before cancellation; neither backend
// needs fused multiply-add for that reduction. Large finite inputs use the
// binary expansion of mathematical 2/pi instead of a rounded f32 period.
fn trig_reduce(value: f32) -> (f32, u32) {
    let x = value.abs();
    if x <= std::f32::consts::FRAC_PI_4 {
        return (x, 0);
    }
    if x < 8192.0 {
        let q = x.mul_add(0.6366197723675814, 0.5).floor();
        let r = (x - q * 1.5703125) - q * 0.00048351287841796875;
        let r = r - q * 2.384185791015625e-7;
        return (r - q * 7.549789415861596e-8, q as u32 & 3);
    }
    let bits = x.to_bits();
    if bits & 0x7f800000 == 0x7f800000 {
        return (f32::NAN, 0);
    }
    let mantissa = (bits & 0x007fffff) | 0x00800000;
    let two_over_pi = [
        0xdebbc561u32,
        0xfe5163ab,
        0x3c439041,
        0xdb629599,
        0xf534ddc0,
        0xfc2757d1,
        0x4e441529,
        0xa2f9836e,
    ];
    let mut product = [0u32; 9];
    let mut carry = 0u64;
    for i in 0..8 {
        let v = mantissa as u64 * two_over_pi[i] as u64 + carry;
        product[i] = v as u32;
        carry = v >> 32;
    }
    product[8] = carry as u32;
    let shift = 406 - (bits >> 23);
    let extract = |bit: u32| {
        let i = (bit / 32) as usize;
        let offset = bit % 32;
        let mut word = product[i] >> offset;
        if offset != 0 && i < 8 {
            word |= product[i + 1] << (32 - offset);
        }
        word
    };
    let round_up = (extract(shift - 1) & 1) as f32;
    let quadrant = ((extract(shift) & 3) + round_up as u32) & 3;
    let hi = (extract(shift - 24) & 0x00ffffff) as f32 * 5.960464477539063e-8 - round_up;
    let lo = (extract(shift - 48) & 0x00ffffff) as f32 * 3.552713678800501e-15;
    let r = hi * 1.570796251296997;
    let tail = hi.mul_add(1.570796251296997, -r);
    let tail = hi.mul_add(7.549789415861596e-8, tail);
    (lo.mul_add(1.5707963267948966, tail) + r, quadrant)
}
#[inline]
fn trig_polynomial(r: f32, cosine: bool) -> f32 {
    let z = r * r;
    if cosine {
        let p = (1.0f32 / 479001600.0) * z - 1.0 / 3628800.0;
        let p = p * z + 1.0 / 40320.0;
        let p = p * z - 1.0 / 720.0;
        let p = p * z + 1.0 / 24.0;
        let p = p * z - 0.5;
        z * p + 1.0
    } else {
        let p = (1.0f32 / 6227020800.0) * z - 1.0 / 39916800.0;
        let p = p * z + 1.0 / 362880.0;
        let p = p * z - 1.0 / 5040.0;
        let p = p * z + 1.0 / 120.0;
        let p = p * z - 1.0 / 6.0;
        (r * z) * p + r
    }
}
#[inline]
fn grain_sin(value: f32) -> f32 {
    let (r, q) = trig_reduce(value);
    let result = trig_polynomial(r, q & 1 != 0);
    if (q & 2 != 0) ^ value.is_sign_negative() {
        -result
    } else {
        result
    }
}
#[inline]
fn grain_cos(value: f32) -> f32 {
    let (r, q) = trig_reduce(value);
    let result = trig_polynomial(r, q & 1 == 0);
    if (q + 1) & 2 != 0 { -result } else { result }
}
fn rnm(tc: [f32; 2], timer: f32) -> [f32; 4] {
    let n = grain_sin((tc[0] + timer) * 12.9898 + (tc[1] + timer) * 78.233) * 43758.5453;
    [n, n * 1.2154, n * 1.3453, n * 1.3647].map(|v| fract(v) * 2. - 1.)
}
fn pnoise(p: [f32; 3], timer: f32, texel: f32) -> f32 {
    let pi = p.map(|v| texel * v.floor() + 0.5 * texel);
    let pf = p.map(fract);
    let mut n = [[[0.; 2]; 2]; 2];
    for x in 0..2 {
        for y in 0..2 {
            let perm = rnm([pi[0] + x as f32 * texel, pi[1] + y as f32 * texel], timer)[3];
            for z in 0..2 {
                let g = rnm([perm, pi[2] + z as f32 * texel], timer);
                n[x][y][z] = (g[0] * 4. - 1.) * (pf[0] - x as f32)
                    + (g[1] * 4. - 1.) * (pf[1] - y as f32)
                    + (g[2] * 4. - 1.) * (pf[2] - z as f32);
            }
        }
    }
    let ux = fade(pf[0]);
    let uy = fade(pf[1]);
    let uz = fade(pf[2]);
    mix(
        mix(
            mix(n[0][0][0], n[1][0][0], ux),
            mix(n[0][1][0], n[1][1][0], ux),
            uy,
        ),
        mix(
            mix(n[0][0][1], n[1][0][1], ux),
            mix(n[0][1][1], n[1][1][1], ux),
            uy,
        ),
        uz,
    )
}
fn rotated(pos: [f32; 2], angle: f32, aspect: f32) -> [f32; 2] {
    let x = (pos[0] * 2. - 1.) * aspect;
    let y = pos[1] * 2. - 1.;
    let sine = grain_sin(angle);
    let cosine = grain_cos(angle);
    [
        (x * cosine - y * sine) / aspect * 0.5 + 0.5,
        (y * cosine + x * sine) * 0.5 + 0.5,
    ]
}
fn color_phase(rgb: [f32; 3], timer: f32) -> f32 {
    let sn = snoise(timer, [rgb[0], rgb[1], rgb[2], 1.]);
    0.01 * timer + 0.99 * (sn[0] * 0.25 + sn[1] * 0.25 + sn[2] * 0.25 + sn[3] * 0.25)
}
fn generator(
    pos: [f32; 2],
    size: [f32; 2],
    luma: f32,
    rgb: [f32; 3],
    p: GrainV2Params,
    digital: bool,
    phase: f32,
) -> [f32; 3] {
    let timer = if digital {
        color_phase(rgb, phase)
    } else {
        phase
    };
    let scale = p.size.clamp(0.5, 1.4);
    let scaled_size = size.map(|v| v * scale);
    let coords = if digital {
        pos
    } else {
        [pos[0] / scaled_size[0], pos[1] / scaled_size[1]]
    };
    let mut angles = [1.425, 3.892, 5.835].map(|a| a * p.rotation * scale);
    if p.clustered && !digital {
        let noise = snoise(timer, [coords[0], timer, coords[1], timer]);
        angles = [noise[0], noise[1], noise[2]].map(|v| v * p.rotation);
    }
    let mult = if digital {
        let den = (1. + (p.size - 1.) / 47.) * 2.4 * (size[0] / 1920.).max(size[1] / 1080.);
        size.map(|v| v / den)
    } else {
        scaled_size.map(|v| v / p.cluster_size / scale)
    };
    let mut n = [0.; 3];
    for c in 0..3 {
        let angle = if digital {
            timer + [1.425, 3.892, 5.835][c]
        } else {
            angles[c]
        };
        let aspect = if digital {
            size[0] / size[1]
        } else {
            scaled_size[0] / scaled_size[1]
        };
        let q = rotated(coords, angle, aspect);
        let v = [q[0] * mult[0], q[1] * mult[1], c as f32];
        let texel = if digital {
            p.cluster_size / 256.
        } else {
            1. / 256. / p.cluster_size
        };
        n[c] = pnoise(v, timer, texel);
        if c == 0 && !digital {
            n[c] = mix(n[c], pnoise([v[0], v[1], 1.], timer * 0.5, texel), luma);
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
                sum[c] += v[c];
            }
        }
    }
    sum.map(|v| v * (1. / 9.))
}
// Match sampled_color's size-changing branch, including its -0.1 pixel offset.
fn sample_grain_source(source: &ImageBuf, size: [f32; 2], x: i32, y: i32) -> [f32; 3] {
    if size == [source.width as f32, source.height as f32] {
        return source.get(x as u32, y as u32).map(to_f32);
    }
    let px = x as f32 / (size[0] - 1.) * (source.width - 1) as f32 - 0.1;
    let py = y as f32 / (size[1] - 1.) * (source.height - 1) as f32 - 0.1;
    let ix = px.floor() as i32;
    let iy = py.floor() as i32;
    let at = |dx: i32, dy: i32| {
        source
            .get(
                (ix + dx).clamp(0, source.width as i32 - 1) as u32,
                (iy + dy).clamp(0, source.height as i32 - 1) as u32,
            )
            .map(to_f32)
    };
    let a = at(0, 0);
    let b = at(1, 0);
    let c = at(0, 1);
    let d = at(1, 1);
    std::array::from_fn(|i| {
        mix(
            mix(a[i], b[i], px - px.floor()),
            mix(c[i], d[i], px - px.floor()),
            py - py.floor(),
        )
    })
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
fn resolution_sample(source: &ImageBuf, x: f32, y: f32) -> [f32; 3] {
    let x = x.clamp(0., source.width.saturating_sub(1) as f32) - 0.1;
    let y = y.clamp(0., source.height.saturating_sub(1) as f32) - 0.1;
    let low_x = x.floor() as i32;
    let low_y = y.floor() as i32;
    let x0 = low_x.clamp(0, source.width as i32 - 1) as u32;
    let y0 = low_y.clamp(0, source.height as i32 - 1) as u32;
    let x1 = (low_x + 1).clamp(0, source.width as i32 - 1) as u32;
    let y1 = (low_y + 1).clamp(0, source.height as i32 - 1) as u32;
    let fx = (x - x.floor()).clamp(0., 1.);
    let fy = (y - y.floor()).clamp(0., 1.);
    let a = source.get(x0, y0).map(to_f32);
    let b = source.get(x1, y0).map(to_f32);
    let c = source.get(x0, y1).map(to_f32);
    let d = source.get(x1, y1).map(to_f32);
    std::array::from_fn(|i| mix(mix(a[i], b[i], fx), mix(c[i], d[i], fx), fy))
}

fn fast_blur_pass(source: &ImageBuf, radius: f32, horizontal: bool) -> ImageBuf {
    let weights = fast_blur_weights(radius);
    let mut output = source.clone();
    for y in 0..source.height {
        for x in 0..source.width {
            let mut value = [0.; 3];
            for &[weight, offset] in &weights {
                let (px, py) = if horizontal {
                    (x as f32 + offset, y as f32)
                } else {
                    (x as f32, y as f32 + offset)
                };
                let (nx, ny) = if horizontal {
                    (x as f32 - offset, y as f32)
                } else {
                    (x as f32, y as f32 - offset)
                };
                // FastBlur's line kernel resets an out-of-range sample to center.
                let px = if px < 0. || px > source.width as f32 - 1. {
                    x as f32
                } else {
                    px
                };
                let nx = if nx < 0. || nx > source.width as f32 - 1. {
                    x as f32
                } else {
                    nx
                };
                let py = if py < 0. || py > source.height as f32 - 1. {
                    y as f32
                } else {
                    py
                };
                let ny = if ny < 0. || ny > source.height as f32 - 1. {
                    y as f32
                } else {
                    ny
                };
                let positive = resolution_sample(source, px, py);
                let negative = resolution_sample(source, nx, ny);
                for c in 0..3 {
                    value[c] += weight * (positive[c] + negative[c]);
                }
            }
            output.set(
                x,
                y,
                value.map(|v| from_f32(half::f16::from_f32(v).to_f32())),
            );
        }
    }
    output
}

fn optical_pass(source: &ImageBuf, radius: f32, horizontal: bool, round_half: bool) -> ImageBuf {
    let weights = optical_weights(radius);
    let half = weights.len() / 2;
    let mut output = source.clone();
    for y in 0..source.height {
        for x in 0..source.width {
            let mut value = [0.; 3];
            for i in 0..half {
                let (px, py) = if horizontal {
                    (x as i32 + i as i32, y as i32)
                } else {
                    (x as i32, y as i32 + i as i32)
                };
                let (nx, ny) = if horizontal {
                    (px - half as i32, py)
                } else {
                    (px, py - half as i32)
                };
                let px = px.clamp(0, source.width as i32 - 1) as u32;
                let py = py.clamp(0, source.height as i32 - 1) as u32;
                let nx = nx.clamp(0, source.width as i32 - 1) as u32;
                let ny = ny.clamp(0, source.height as i32 - 1) as u32;
                let positive = source.get(px, py).map(to_f32);
                let negative = source.get(nx, ny).map(to_f32);
                for c in 0..3 {
                    value[c] += positive[c] * weights[i + half] + negative[c] * weights[i];
                }
            }
            output.set(
                x,
                y,
                if round_half {
                    value.map(|v| from_f32(half::f16::from_f32(v).to_f32()))
                } else {
                    value.map(from_f32)
                },
            );
        }
    }
    output
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
    let phase = p.resolved_timer();
    let optical = p.resolution_type == 0;
    let radius = p.resolution_radius(input.width, input.height);
    // Photo Grain preserves the caller's native encoded RGB domain; only the
    // half-storage boundary is applied before filtering and composition.
    let mut source = input.clone();
    source
        .data
        .iter_mut()
        .for_each(|value| *value = from_f32(half::f16::from_f32(to_f32(*value)).to_f32()));
    // Analogue's generator samples the original working image, while its
    // composition uses the Film Resolution result. Noise is single-pass and
    // consumes the prepared input directly.
    let original = (radius > 0. && p.mode == GrainV2Mode::Analogue).then(|| source.clone());
    if radius > 0. {
        if optical {
            // OpticalResolution retains float precision between H and V.
            source = optical_pass(&source, radius, true, false);
            source = optical_pass(&source, radius, false, true);
        } else {
            source = fast_blur_pass(&source, radius, true);
            source = fast_blur_pass(&source, radius, false);
        }
    }
    let gsf = (5200. / input.width as f32).max(3100. / input.height as f32);
    let size = [
        (input.width as f32 * gsf).floor(),
        (input.height as f32 * gsf).floor(),
    ];
    let mut out = source.clone();
    for y in 0..input.height {
        for x in 0..input.width {
            let rgb = source.get(x, y).map(to_f32);
            let luma = rgb[0] * 0.2125 + rgb[1] * 0.7154 + rgb[2] * 0.0721;
            let uv = [
                x as f32 * (1. / (input.width - 1).max(1) as f32),
                y as f32 * (1. / (input.height - 1).max(1) as f32),
            ];
            let g = if p.mode == GrainV2Mode::Noise {
                generator(
                    uv,
                    [input.width as f32, input.height as f32],
                    luma,
                    rgb,
                    p,
                    true,
                    phase,
                )
            } else {
                let gx = (uv[0] * (size[0] / p.resampler_scale())) as i32;
                let gy = (uv[1] * (size[1] / p.resampler_scale())) as i32;
                resample_3x3(
                    |a, b| {
                        let a = a.clamp(0, size[0] as i32 - 1);
                        let b = b.clamp(0, size[1] as i32 - 1);
                        let grain_rgb =
                            sample_grain_source(original.as_ref().unwrap_or(&source), size, a, b);
                        let grain_luma =
                            grain_rgb[0] * 0.2125 + grain_rgb[1] * 0.7154 + grain_rgb[2] * 0.0721;
                        generator(
                            [a as f32, b as f32],
                            size,
                            grain_luma,
                            grain_rgb,
                            p,
                            false,
                            phase,
                        )
                        .map(|value| half::f16::from_f32(value).to_f32())
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
                // Composition remains in the native encoded domain.
                let composed = half::f16::from_f32((h * 0.5 + b * 0.5).clamp(0., 1.)).to_f32();
                result[c] = composed;
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
    fn matches_normalized_external_kernel_outputs() {
        // External 7.4.1 OpenCL execution, with only trigonometry, phase
        // reduction and contraction normalized to our portable arithmetic.
        // These are output values, not proprietary source. Native PoCL output
        // differs; provenance and that limitation are recorded in docs.
        // The historical fixture input was linear; encode it here solely to
        // preserve the external encoded-domain test contract.
        let historical_bt709 = |value: f32| {
            if value < 0.018 {
                value * 4.5
            } else {
                1.099 * value.powf(0.45) - 0.099
            }
        };
        let input = ImageBuf::from_data(
            37,
            19,
            (0..37 * 19 * 3)
                .map(|v| {
                    from_f32(half::f16::from_f32(historical_bt709((v % 17) as f32 / 17.)).to_f32())
                })
                .collect(),
        );
        for (mode, fixture) in [
            (
                GrainV2Mode::Analogue,
                include_str!("fixtures/analogue_normalized_half.txt"),
            ),
            (
                GrainV2Mode::Noise,
                include_str!("fixtures/noise_normalized_half.txt"),
            ),
        ] {
            let mut params = GrainV2Params::for_profile(0);
            params.seed = 5489;
            params.mode = mode;
            params.resolution_factor = 100.;
            let actual = apply_cpu(&input, params);
            let expected: Vec<_> = fixture
                .split_whitespace()
                .map(|v| half::f16::from_bits(u16::from_str_radix(v, 16).unwrap()).to_f32())
                .collect();
            assert_eq!(actual.data.len(), expected.len());
            for (index, (&value, reference)) in actual.data.iter().zip(expected).enumerate() {
                let delta = (to_f32(value) - reference).abs();
                assert!(delta <= 1e-6, "{mode:?} sample {index}: delta={delta}");
            }
        }
    }
    #[test]
    fn explicit_timer_overrides_seed_and_changes_phase() {
        let image = ImageBuf::from_data(
            4,
            4,
            (0..4 * 4 * 3)
                .map(|v| from_f32((v % 11) as f32 / 11.0))
                .collect(),
        );
        let mut seeded = GrainV2Params::for_profile(0);
        seeded.mode = GrainV2Mode::Noise;
        seeded.resolution_factor = 100.0;
        seeded.seed = 42;
        let mut explicit = seeded;
        explicit.seed = 777;
        explicit.timer = Some(spektrafilm_math::grain::seeded_phase(42));
        assert_eq!(
            apply_cpu(&image, seeded).data,
            apply_cpu(&image, explicit).data
        );

        explicit.timer = Some(0.25);
        assert_ne!(
            apply_cpu(&image, seeded).data,
            apply_cpu(&image, explicit).data
        );
    }

    #[test]
    fn grad4_matches_vendor_return_value() {
        assert_eq!(grad4(0.5, [0.25, 0.75, 0.5, 0.0]), [-1.0, 0.0, -0.5, 0.0]);
        assert_eq!(grad4(0.5, [0.75; 4]), [0.5, 0.5, 0.5, 0.0]);
    }

    #[test]
    fn resample_3x3_averages_clamped_neighborhood() {
        let value = resample_3x3(
            |x, y| {
                let x = x.clamp(0, 1) as f32;
                let y = y.clamp(0, 1) as f32;
                [x, y, 1.0]
            },
            0,
            0,
        );
        assert!((value[0] - 1.0 / 3.0).abs() < 1e-6);
        assert!((value[1] - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(value[2], 1.0);
    }

    #[test]
    fn overlay_matches_photoshop_midpoint_and_endpoints() {
        assert_eq!(overlay(0.5, 0.0), 0.0);
        assert_eq!(overlay(0.5, 1.0), 1.0);
        assert!((overlay(0.25, 0.75) - 0.375).abs() < 1e-6);
        assert!((overlay(0.75, 0.25) - 0.625).abs() < 1e-6);
    }

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
    fn resolution_type_selects_analogue_path_and_noise_bypasses_resolution() {
        let weights = fast_blur_weights(0.5);
        assert_eq!(weights.len(), 1);
        assert!((weights[0][0] - 0.5).abs() < 1e-6);
        assert!((weights[0][1] - 0.021735).abs() < 1e-5);
        let image = ImageBuf::from_data(
            192,
            108,
            (0..192 * 108)
                .flat_map(|i| [from_f32(if i % 192 < 96 { 0.1 } else { 0.8 }); 3])
                .collect(),
        );

        let mut analogue = GrainV2Params::default();
        analogue.mode = GrainV2Mode::Analogue;
        analogue.size = 48.;
        analogue.amount = 1.;
        analogue.resolution_factor = 0.;
        analogue.resolution_type = 0;
        let optical = apply_cpu(&image, analogue);
        analogue.resolution_type = 1;
        let fast = apply_cpu(&image, analogue);
        let edge = |img: &ImageBuf| to_f32(img.get(96, 54)[0]) - to_f32(img.get(95, 54)[0]);
        assert!((edge(&optical) - edge(&fast)).abs() > 1e-3);
        analogue.film_type = 1;
        assert_eq!(apply_cpu(&image, analogue).data, fast.data);

        let mut noise = analogue;
        noise.mode = GrainV2Mode::Noise;
        noise.resolution_type = 0;
        noise.resolution_factor = 0.;
        let optical_noise = apply_cpu(&image, noise);
        noise.resolution_type = 1;
        noise.resolution_factor = 100.;
        let fast_noise = apply_cpu(&image, noise);
        assert_eq!(noise.resolution_radius(image.width, image.height), 0.);
        assert_eq!(optical_noise.data, fast_noise.data);
        noise.film_type = 0;
        assert_eq!(apply_cpu(&image, noise).data, fast_noise.data);
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
    fn optical_subpixel_radius_is_identity() {
        let image = ImageBuf::from_data(
            9,
            7,
            (0..9 * 7 * 3)
                .map(|i| from_f32(half::f16::from_f32((i % 17) as f32 / 17.).to_f32()))
                .collect(),
        );
        let horizontal = optical_pass(&image, 0.5, true, false);
        let output = optical_pass(&horizontal, 0.5, false, true);
        assert_eq!(output.data, image.data);
    }

    #[test]
    fn noise_scale_preserves_relative_grain_size() {
        let mut p = GrainV2Params::default();
        p.mode = GrainV2Mode::Noise;
        let phase = spektrafilm_math::grain::seeded_phase(p.seed);
        let a = generator([0.37, 0.61], [1920., 1080.], 0.5, [0.5; 3], p, true, phase);
        let b = generator([0.37, 0.61], [3840., 2160.], 0.5, [0.5; 3], p, true, phase);
        assert!(a.iter().zip(b).all(|(x, y)| (*x - y).abs() < 1e-6));
    }
    #[test]
    fn middle_gray_responds_to_midtones_not_shadows() {
        let image = ImageBuf::from_data(64, 32, vec![from_f32(0.5); 64 * 32 * 3]);
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
            "encoded middle gray must not select the shadow bell"
        );
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
            for resolution_type in [0, 1] {
                for mode in [GrainV2Mode::Analogue, GrainV2Mode::Noise] {
                    for control in [0., 1.] {
                        let mut params = GrainV2Params::for_profile(profile);
                        params.resolution_type = resolution_type;
                        params.mode = mode;
                        params.seed = 42;
                        params.amount = control;
                        params.shadows = control;
                        params.midtones = control;
                        params.highlights = control;
                        params.color = control;
                        if profile == 0
                            && resolution_type == 0
                            && mode == GrainV2Mode::Noise
                            && control == 1.
                        {
                            params.timer = Some(0.25);
                        }
                        let cpu = apply_cpu(&image, params);
                        let gpu_params = spektrafilm_gpu::GrainV2GpuParams {
                            mode: params.mode as u32,
                            resolution_type: params.resolution_type,
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
                            timer: params.timer,
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
                        assert!(
                            max_error < 5e-3,
                            "CPU/GPU Grain V2 drift: {max_error}, profile={profile}, resolution_type={resolution_type}, mode={mode:?}, control={control}"
                        );
                    }
                }
            }
        }
    }
}
