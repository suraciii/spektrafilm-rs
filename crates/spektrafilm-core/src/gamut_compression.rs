//! Output gamut compression matching pinned Python 0.3.4.
//! Perceptual tables target the destination's linear RGB cube and own whitepoint.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use crate::params::OutputGamutCompressParams;

use spektrafilm_math::colorspace::{resolve, spow, RgbColorSpace, SRGB};

// OkLab (Ottosson) matrices, exactly as colour-science stores them.
const M1_XYZ_TO_LMS: [[f64; 3]; 3] = [
    [0.8189330101, 0.3618667424, -0.1288597137],
    [0.0329845436, 0.9293118715, 0.0361456387],
    [0.0482003018, 0.2643662691, 0.633851707],
];
const M1_LMS_TO_XYZ: [[f64; 3]; 3] = [
    [1.2270138511035211, -0.5577999806518222, 0.2812561489664678],
    [-0.0405801784232806, 1.11225686961683, -0.07167667866560119],
    [-0.0763812845057069, -0.4214819784180127, 1.5861632204407947],
];
const M2_LMS_TO_LAB: [[f64; 3]; 3] = [
    [0.2104542553, 0.793617785, -0.0040720468],
    [1.9779984951, -2.428592205, 0.4505937099],
    [0.0259040371, 0.7827717662, -0.808675766],
];
const M2_LAB_TO_LMS: [[f64; 3]; 3] = [
    [0.9999999984505196, 0.3963377921737678, 0.21580375806075877],
    [
        1.0000000088817607,
        -0.10556134232365633,
        -0.0638541747717059,
    ],
    [
        1.0000000546724108,
        -0.08948418209496574,
        -1.2914855378640917,
    ],
];

// C_max(L, h) table grid dims, matching `_OKLCH_CMAX_TABLE_N_*`. The L-axis
// bounds and the bisection's chroma ceiling are per-space (see `build`).
const N_L: usize = 64;
const N_H: usize = 720;
const N_BISECT: usize = 18;

#[inline]
fn mat_vec(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

#[inline]
pub(crate) fn xyz_to_oklab(xyz: [f64; 3]) -> [f64; 3] {
    let lms = mat_vec(&M1_XYZ_TO_LMS, xyz);
    let lms_ = [lms[0].cbrt(), lms[1].cbrt(), lms[2].cbrt()];
    mat_vec(&M2_LMS_TO_LAB, lms_)
}

#[inline]
pub(crate) fn oklab_to_xyz(lab: [f64; 3]) -> [f64; 3] {
    let lms_ = mat_vec(&M2_LAB_TO_LMS, lab);
    let lms = [lms_[0].powi(3), lms_[1].powi(3), lms_[2].powi(3)];
    mat_vec(&M1_LMS_TO_XYZ, lms)
}

// Ottosson 2023 "Lr": a 1D monotonic remap of OkLab L so the lightness scale
// tracks CIELAB L* more closely. Lr(0)=0, Lr(1)=1.
const OKLRAB_K1: f64 = 0.206;
const OKLRAB_K2: f64 = 0.03;
const OKLRAB_K3: f64 = (1.0 + OKLRAB_K1) / (1.0 + OKLRAB_K2);

#[inline]
fn oklab_l_to_lr(l: f64) -> f64 {
    let t = OKLRAB_K3 * l - OKLRAB_K1;
    0.5 * (t + (t * t + 4.0 * OKLRAB_K2 * OKLRAB_K3 * l).sqrt())
}

#[inline]
fn oklrab_lr_to_l(lr: f64) -> f64 {
    (lr * (lr + OKLRAB_K1)) / (OKLRAB_K3 * (lr + OKLRAB_K2))
}

/// Smooth Reinhard knee on normalized distance: identity below `threshold`,
/// asymptotic at `limit` above it. Matches the ACES RGC v1.3 reference.
#[inline]
pub(crate) fn reinhard_knee(d: f64, threshold: f64, limit: f64, power: f64) -> f64 {
    if d > threshold {
        let scale = limit - threshold;
        let x = (d - threshold) / scale;
        let y = x / (1.0 + x.powf(power)).powf(1.0 / power);
        threshold + scale * y
    } else {
        d
    }
}

#[inline]
fn h_grid(j: usize) -> f64 {
    // linspace(-pi, pi, N_H, endpoint=False)
    -std::f64::consts::PI + (2.0 * std::f64::consts::PI) * (j as f64) / (N_H as f64)
}

/// Perceptual space the chroma reduction runs in.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Space {
    Oklch,
    /// OkLab with the Ottosson Lr-rebased lightness on the L axis.
    Oklrab,
    Jzazbz,
    Cam16ucs,
}

impl Space {
    #[inline]
    fn from_rgb(self, rgb: [f64; 3], cs: &RgbColorSpace, cam: &Cam16Viewing) -> [f64; 3] {
        let xyz = mat_vec(&cs.matrix_rgb_to_xyz, rgb);
        match self {
            Space::Oklch | Space::Oklrab => xyz_to_oklab(xyz),
            Space::Jzazbz => xyz_to_jzazbz(xyz),
            Space::Cam16ucs => xyz_to_cam16ucs(xyz, cam),
        }
    }
    #[inline]
    fn to_rgb(self, lab: [f64; 3], cs: &RgbColorSpace, cam: &Cam16Viewing) -> [f64; 3] {
        let xyz = match self {
            Space::Oklch | Space::Oklrab => oklab_to_xyz(lab),
            Space::Jzazbz => jzazbz_to_xyz(lab),
            Space::Cam16ucs => cam16ucs_to_xyz(lab, cam),
        };
        mat_vec(&cs.matrix_xyz_to_rgb, xyz)
    }
    /// Reconstruction lightness `L` → C_max-table lookup index. Identity except
    /// oklrab, which indexes the table by Ottosson's rebased `Lr`.
    #[inline]
    fn lookup_lightness(self, l: f64) -> f64 {
        match self {
            Space::Oklrab => oklab_l_to_lr(l),
            _ => l,
        }
    }
    /// C_max-table lookup index → reconstruction lightness `L` (for the bake).
    #[inline]
    fn recon_lightness(self, lookup: f64) -> f64 {
        match self {
            Space::Oklrab => oklrab_lr_to_l(lookup),
            _ => lookup,
        }
    }
    /// Pinned `(l_min, l_max, chroma_upper)` for each perceptual table.
    fn table_geometry(self) -> (f64, f64, f64) {
        match self {
            Space::Oklch | Space::Oklrab => (0.02, 1.0, 0.5),
            Space::Jzazbz => (0.002, 0.18, 0.3),
            Space::Cam16ucs => (1.0, 110.0, 150.0),
        }
    }
}

/// Which algorithm the compressor runs.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Off,
    /// ACES Reference Gamut Compression — per-channel knee in RGB, no table.
    AcesRgc,
    /// Perceptual chroma reduction against a `C_max(L, h)` table.
    Perceptual(Space),
}

/// Pre-computed output gamut compressor for a fixed output color space.
#[derive(Clone)]
pub struct OutputGamutCompress {
    kind: Kind,
    destination: &'static RgbColorSpace,
    cam: Cam16Viewing,
    knee: (f64, f64, f64),
    lightness: Option<(f64, f64, f64)>,
    /// Perceptual lightness of the output white (1.0 for OkLab, ~100 for CAM16-UCS).
    l_white: f64,
    /// Lightness-axis bounds of the `C_max` table grid.
    l_min: f64,
    l_max: f64,
    /// Destination-specific `C_max(L, h)`, row-major `[N_L][N_H]`.
    cmax: Arc<Vec<f64>>,
}

impl OutputGamutCompress {
    /// No-op (identity) compressor.
    pub fn identity() -> Self {
        Self {
            kind: Kind::Off,
            destination: &SRGB,
            cam: Cam16Viewing::new(SRGB.whitepoint_xyz()),
            knee: (0.0, 1.0, 1.0),
            lightness: None,
            l_white: 1.0,
            l_min: 0.0,
            l_max: 1.0,
            cmax: Arc::new(Vec::new()),
        }
    }

    pub fn is_active(&self) -> bool {
        self.kind != Kind::Off
    }

    /// Resolve and validate before building a destination-specific compressor.
    pub fn build(params: &OutputGamutCompressParams, output_color_space: &str) -> Result<Self, String> {
        let destination = resolve(output_color_space)?;
        let kind = match params.algorithm.as_str() {
            "off" => Kind::Off,
            "aces_rgc" => Kind::AcesRgc,
            "oklch" => Kind::Perceptual(Space::Oklch),
            "oklrab" => Kind::Perceptual(Space::Oklrab),
            "jzazbz" => Kind::Perceptual(Space::Jzazbz),
            "cam16ucs" => Kind::Perceptual(Space::Cam16ucs),
            other => return Err(format!("unknown output gamut compression algorithm {other:?}")),
        };
        let validate = |name: &str, values: [f32; 3]| -> Result<(f64, f64, f64), String> {
            let [t, l, p] = values.map(f64::from);
            if !t.is_finite() || !(0.0..1.0).contains(&t) {
                return Err(format!("{name} threshold must be finite and in [0, 1), got {t}"));
            }
            if !l.is_finite() || l <= 0.0 {
                return Err(format!("{name} limit must be finite and positive, got {l}"));
            }
            if !p.is_finite() || p <= 0.0 {
                return Err(format!("{name} power must be finite and positive, got {p}"));
            }
            Ok((t, l, p))
        };
        let knee = validate("output gamut knee", params.knee)?;
        let lightness = params.lightness_compression
            .map(|v| validate("output lightness compression", v)).transpose()?;
        let cam = Cam16Viewing::new(destination.whitepoint_xyz());
        let mut result = Self {
            kind,
            destination,
            cam,
            knee,
            lightness: if matches!(kind, Kind::Perceptual(_)) { lightness } else { None },
            l_white: 1.0,
            l_min: 0.0,
            l_max: 1.0,
            cmax: Arc::new(Vec::new()),
        };
        if let Kind::Perceptual(space) = kind {
            result.l_white = match space {
                Space::Oklch | Space::Oklrab => 1.0,
                Space::Jzazbz => xyz_to_jzazbz(destination.whitepoint_xyz())[0],
                Space::Cam16ucs => xyz_to_cam16ucs(destination.whitepoint_xyz(), &cam)[0],
            };
            let (l_min, l_max, _) = space.table_geometry();
            result.l_min = l_min;
            result.l_max = l_max;
            result.cmax = cmax_table_cached(space, destination, &cam);
        }
        Ok(result)
    }

    /// GPU inputs for sRGB's existing modes. An active compressor returning
    /// `None` requires CPU execution, including bypassing fused GPU passes.
    pub fn gpu_params(&self) -> Option<spektrafilm_gpu::GamutGpuParams<'_>> {
        if self.destination.name != "sRGB" {
            return None;
        }
        let mode = match self.kind {
            Kind::Off => return None,
            Kind::AcesRgc => 0,
            Kind::Perceptual(Space::Oklch) => 1,
            Kind::Perceptual(Space::Oklrab) => 2,
            Kind::Perceptual(Space::Cam16ucs) => 3,
            Kind::Perceptual(Space::Jzazbz) => return None,
        };
        Some(spektrafilm_gpu::GamutGpuParams {
            mode,
            knee: [self.knee.0 as f32, self.knee.1 as f32, self.knee.2 as f32],
            lightness: self
                .lightness
                .map(|(t, l, p)| [t as f32, l as f32, p as f32]),
            l_white: self.l_white as f32,
            l_min: self.l_min as f32,
            l_max: self.l_max as f32,
            cmax: &self.cmax,
            n_l: N_L as u32,
            n_h: N_H as u32,
        })
    }

    /// Bilinear `C_max(L, h)` lookup (L clamped, h wraps).
    #[inline]
    fn c_max_lookup(&self, l: f64, h: f64) -> f64 {
        let l = l.clamp(self.l_min, self.l_max);
        let h_step = h_grid(1) - h_grid(0);
        let h_idx = (h - h_grid(0)) / h_step;
        let h_floor = h_idx.floor();
        let h_lo = (h_floor as i64).rem_euclid(N_H as i64) as usize;
        let h_hi = (h_lo + 1) % N_H;
        let h_frac = h_idx - h_floor;

        let l_idx = (l - self.l_min) / (self.l_max - self.l_min) * ((N_L - 1) as f64);
        let l_lo = (l_idx.floor() as i64).clamp(0, (N_L - 2) as i64) as usize;
        let l_hi = l_lo + 1;
        let l_frac = l_idx - l_lo as f64;

        let t = &self.cmax;
        let v00 = t[l_lo * N_H + h_lo];
        let v01 = t[l_lo * N_H + h_hi];
        let v10 = t[l_hi * N_H + h_lo];
        let v11 = t[l_hi * N_H + h_hi];
        v00 * (1.0 - l_frac) * (1.0 - h_frac)
            + v01 * (1.0 - l_frac) * h_frac
            + v10 * l_frac * (1.0 - h_frac)
            + v11 * l_frac * h_frac
    }

    /// Compress a single linear output-RGB pixel. Identity when inactive.
    #[inline]
    pub fn compress(&self, rgb: [f64; 3]) -> [f64; 3] {
        let space = match self.kind {
            Kind::Off => return rgb,
            Kind::AcesRgc => return self.compress_aces_rgc(rgb),
            Kind::Perceptual(s) => s,
        };
        let lab = space.from_rgb(rgb, self.destination, &self.cam);
        // One-sided lightness compression first (so C_max is looked up at the
        // corrected L), normalized by the perceptual white.
        let mut l = lab[0];
        if let Some((t, lim, p)) = self.lightness {
            l = reinhard_knee(l / self.l_white, t, lim, p) * self.l_white;
        }
        let a = lab[1];
        let b = lab[2];
        let c = a.hypot(b);
        let h = b.atan2(a);
        // C_max is looked up at the lookup-lightness (Lr for oklrab); the
        // reconstruction below keeps the reconstruction lightness `l`.
        let c_max = self.c_max_lookup(space.lookup_lightness(l), h);
        let safe = c_max.max(1e-9);
        let d_comp = reinhard_knee(c / safe, self.knee.0, self.knee.1, self.knee.2);
        let c_new = d_comp * safe;
        space.to_rgb([l, c_new * h.cos(), c_new * h.sin()], self.destination, &self.cam)
    }

    /// ACES Reference Gamut Compression v1.3: per-channel Reinhard knee on the
    /// achromatic distance `d = (max - c)/max`. Mirrors `compress_rgb_aces_rgc`.
    #[inline]
    fn compress_aces_rgc(&self, rgb: [f64; 3]) -> [f64; 3] {
        let ach = rgb[0].max(rgb[1]).max(rgb[2]);
        if ach <= 1e-12 {
            return rgb;
        }
        let mut out = [0.0f64; 3];
        for c in 0..3 {
            let d = (ach - rgb[c]) / ach;
            let dc = reinhard_knee(d, self.knee.0, self.knee.1, self.knee.2);
            out[c] = ach * (1.0 - dc);
        }
        out
    }
}

// Safdar 2017 constants and matrices from the pinned colour-science runtime.
const JZ_M1: [[f64; 3]; 3] = [
    [0.41478972, 0.579999, 0.014648],
    [-0.20151, 1.120649, 0.0531008],
    [-0.0166008, 0.2648, 0.6684799],
];
const JZ_M1_INV: [[f64; 3]; 3] = [
    [1.9242264357876069, -1.0047923125953655, 0.037651404030617994],
    [0.35031676209499907, 0.7264811939316552, -0.06538442294808501],
    [-0.09098281098284754, -0.3127282905230739, 1.5227665613052603],
];
const JZ_M2: [[f64; 3]; 3] = [
    [0.5, 0.5, 0.0],
    [3.524, -4.066708, 0.542708],
    [0.199076, 1.096799, -1.295875],
];
const JZ_M2_INV: [[f64; 3]; 3] = [
    [1.0, 0.1386050432715393, 0.058047316156118994],
    [0.9999999999999999, -0.13860504327153927, -0.05804731615611874],
    [0.9999999999999998, -0.09601924202631892, -0.8118918960560387],
];
const JZ_B: f64 = 1.15;
const JZ_G: f64 = 0.66;
const JZ_D: f64 = -0.56;
const JZ_D0: f64 = 1.6295499532821565e-11;
const JZ_M_1: f64 = 0.1593017578125;
const JZ_M_2: f64 = 134.03437499999998;
const JZ_C1: f64 = 0.8359375;
const JZ_C2: f64 = 18.8515625;
const JZ_C3: f64 = 18.6875;
const JZ_Y_WHITE: f64 = 100.0;

fn xyz_to_jzazbz(xyz: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = xyz.map(|v| v * JZ_Y_WHITE);
    let lms = mat_vec(&JZ_M1, [JZ_B * x - (JZ_B - 1.0) * z, JZ_G * y - (JZ_G - 1.0) * x, z]);
    let pq = lms.map(|v| {
        let yp = spow(v / 10000.0, JZ_M_1);
        spow((JZ_C1 + JZ_C2 * yp) / (1.0 + JZ_C3 * yp), JZ_M_2)
    });
    let [iz, az, bz] = mat_vec(&JZ_M2, pq);
    [(1.0 + JZ_D) * iz / (1.0 + JZ_D * iz) - JZ_D0, az, bz]
}

fn jzazbz_to_xyz(jab: [f64; 3]) -> [f64; 3] {
    let [jz, az, bz] = jab;
    let iz = (jz + JZ_D0) / (1.0 + JZ_D - JZ_D * (jz + JZ_D0));
    let pq = mat_vec(&JZ_M2_INV, [iz, az, bz]);
    let lms = pq.map(|v| {
        let vp = spow(v, 1.0 / JZ_M_2);
        10000.0 * spow((vp - JZ_C1).max(0.0) / (JZ_C2 - JZ_C3 * vp), 1.0 / JZ_M_1)
    });
    let [xp, yp, z] = mat_vec(&JZ_M1_INV, lms);
    let x = (xp + (JZ_B - 1.0) * z) / JZ_B;
    let y = (yp + (JZ_G - 1.0) * x) / JZ_G;
    [x / JZ_Y_WHITE, y / JZ_Y_WHITE, z / JZ_Y_WHITE]
}

// ─────────────────────────────────────────────────────────────────────────
// CAM16-UCS (upstream's default `cam16ucs` method).
//
// Fixed L_A = 64 cd/m², Y_b = 20, Average surround. Adaptation and white
// response depend on the destination whitepoint; other coefficients are fixed.
// XYZ here is at Y=1; CIECAM16 works in the Y=100 domain, hence the ×/÷100.
// ─────────────────────────────────────────────────────────────────────────
const CAM16_M16: [[f64; 3]; 3] = [
    [0.401288, 0.650173, -0.051461],
    [-0.250268, 1.204414, 0.045854],
    [-0.002079, 0.048952, 0.953127],
];
const CAM16_M16I: [[f64; 3]; 3] = [
    [1.8620678550872327, -1.0112546305316843, 0.14918677544445175],
    [
        0.3875265432361372,
        0.6214474419314753,
        -0.008973985167612516,
    ],
    [
        -0.015841498849333863,
        -0.03412293802851557,
        1.0499644368778496,
    ],
];
const CAM16_F_L: f64 = 0.6839903845696502;
const CAM16_N: f64 = 0.2;
const CAM16_N_BB: f64 = 1.0003040045593807;
const CAM16_N_CB: f64 = 1.0003040045593807;
const CAM16_Z: f64 = 1.9272135954999579;
const CAM16_C: f64 = 0.69;
const CAM16_N_C: f64 = 1.0;
// Luo 2006 CAM16-UCS coefficients (K_L=1.0, c1, c2).
const UCS_C1: f64 = 0.007;
const UCS_C2: f64 = 0.0228;

#[derive(Clone, Copy)]
struct Cam16Viewing {
    d_rgb: [f64; 3],
    a_w: f64,
}

impl Cam16Viewing {
    fn new(white: [f64; 3]) -> Self {
        let rgb_w = mat_vec(&CAM16_M16, white.map(|v| v * 100.0));
        let d = (1.0 - (1.0 / 3.6) * ((-64.0f64 - 42.0) / 92.0).exp()).clamp(0.0, 1.0);
        let d_rgb = rgb_w.map(|v| d * 100.0 / v + 1.0 - d);
        let rgb_wc = std::array::from_fn(|i| rgb_w[i] * d_rgb[i]);
        let [r, g, b] = cam16_padc_forward(rgb_wc);
        Self { d_rgb, a_w: (2.0 * r + g + b / 20.0 - 0.305) * CAM16_N_BB }
    }
}

#[inline]
fn atan2_deg(y: f64, x: f64) -> f64 {
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

#[inline]
fn cam16_padc_forward(rgb: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0f64; 3];
    for i in 0..3 {
        let flr = (CAM16_F_L * rgb[i].abs() / 100.0).powf(0.42);
        out[i] = 400.0 * rgb[i].signum() * flr / (27.13 + flr) + 0.1;
    }
    out
}

#[inline]
fn cam16_padc_inverse(rgb: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0f64; 3];
    for i in 0..3 {
        let d = rgb[i] - 0.1;
        let base = (27.13 * d.abs()) / (400.0 - d.abs());
        out[i] = d.signum() * 100.0 / CAM16_F_L * base.powf(1.0 / 0.42);
    }
    out
}

/// XYZ (Y=1) → CAM16-UCS `(Jp, ap, bp)`.
fn xyz_to_cam16ucs(xyz: [f64; 3], cam: &Cam16Viewing) -> [f64; 3] {
    let xyz100 = [xyz[0] * 100.0, xyz[1] * 100.0, xyz[2] * 100.0];
    let rgb = mat_vec(&CAM16_M16, xyz100);
    let rgb_c = [
        rgb[0] * cam.d_rgb[0],
        rgb[1] * cam.d_rgb[1],
        rgb[2] * cam.d_rgb[2],
    ];
    let [ra, ga, ba] = cam16_padc_forward(rgb_c);
    let a = ra - 12.0 * ga / 11.0 + ba / 11.0;
    let b = (ra + ga - 2.0 * ba) / 9.0;
    let h = atan2_deg(b, a);
    let e_t = 0.25 * ((2.0 + h * std::f64::consts::PI / 180.0).cos() + 3.8);
    let a_resp = (2.0 * ra + ga + ba / 20.0 - 0.305) * CAM16_N_BB;
    let jj = 100.0 * spow(a_resp / cam.a_w, CAM16_C * CAM16_Z);
    let denom = ra + ga + 21.0 * ba / 20.0;
    let t = if denom != 0.0 {
        (50000.0 / 13.0) * CAM16_N_C * CAM16_N_CB * (e_t * (a * a + b * b).sqrt()) / denom
    } else {
        0.0
    };
    let cc = spow(t, 0.9) * spow(jj / 100.0, 0.5) * (1.64 - 0.29f64.powf(CAM16_N)).powf(0.73);
    let m = cc * CAM16_F_L.powf(0.25);
    let jp = (1.0 + 100.0 * UCS_C1) * jj / (1.0 + UCS_C1 * jj);
    let mp = (1.0 / UCS_C2) * (1.0 + UCS_C2 * m).ln();
    let hr = h * std::f64::consts::PI / 180.0;
    [jp, mp * hr.cos(), mp * hr.sin()]
}

/// CAM16-UCS `(Jp, ap, bp)` → XYZ (Y=1).
fn cam16ucs_to_xyz(jab: [f64; 3], cam: &Cam16Viewing) -> [f64; 3] {
    let [jp, ap, bp] = jab;
    let mp = (ap * ap + bp * bp).sqrt();
    let h = atan2_deg(bp, ap);
    let jj = jp / ((1.0 + 100.0 * UCS_C1) - UCS_C1 * jp);
    let m = ((UCS_C2 * mp).exp() - 1.0) / UCS_C2;
    let cc = m / CAM16_F_L.powf(0.25);
    let j_prime = jj.max(f64::EPSILON);
    let t = spow(cc / ((j_prime / 100.0).sqrt() * (1.64 - 0.29f64.powf(CAM16_N)).powf(0.73)), 1.0 / 0.9);
    let e_t = 0.25 * ((2.0 + h * std::f64::consts::PI / 180.0).cos() + 3.8);
    let a_resp = cam.a_w * spow(jj / 100.0, 1.0 / (CAM16_C * CAM16_Z));
    let p1 = if t != 0.0 {
        (50000.0 / 13.0) * CAM16_N_C * CAM16_N_CB * e_t / t
    } else {
        0.0
    };
    let p2 = a_resp / CAM16_N_BB + 0.305;
    let p3 = 21.0 / 20.0;
    let (mut a, mut b) = cam16_opponent_inverse(p1, p2, p3, h);
    // Achromatic guard, matching colour's `ab * np.where(t == 0, 0, 1)`:
    // when t == 0 the hue is undefined and the opponent inverse must be zeroed.
    if t == 0.0 {
        a = 0.0;
        b = 0.0;
    }
    let ra = (460.0 * p2 + 451.0 * a + 288.0 * b) / 1403.0;
    let ga = (460.0 * p2 - 891.0 * a - 261.0 * b) / 1403.0;
    let ba = (460.0 * p2 - 220.0 * a - 6300.0 * b) / 1403.0;
    let rgb_c = cam16_padc_inverse([ra, ga, ba]);
    let rgb = [
        rgb_c[0] / cam.d_rgb[0],
        rgb_c[1] / cam.d_rgb[1],
        rgb_c[2] / cam.d_rgb[2],
    ];
    let xyz100 = mat_vec(&CAM16_M16I, rgb);
    [xyz100[0] / 100.0, xyz100[1] / 100.0, xyz100[2] / 100.0]
}

/// Inverse opponent dimensions (CIECAM02/16 `opponent_colour_dimensions_inverse`).
fn cam16_opponent_inverse(p1: f64, p2: f64, p3: f64, h: f64) -> (f64, f64) {
    let hr = h * std::f64::consts::PI / 180.0;
    let s = hr.sin();
    let c = hr.cos();
    let nn = p2 * (2.0 + p3) * (460.0 / 1403.0);
    if s.abs() >= c.abs() {
        let p4 = if s != 0.0 { p1 / s } else { 0.0 };
        let b = nn
            / (p4 + (2.0 + p3) * (220.0 / 1403.0) * (c / s) - (27.0 / 1403.0)
                + p3 * (6300.0 / 1403.0));
        (b * (c / s), b)
    } else {
        let p5 = if c != 0.0 { p1 / c } else { 0.0 };
        let a = nn
            / (p5 + (2.0 + p3) * (220.0 / 1403.0)
                - ((27.0 / 1403.0) - p3 * (6300.0 / 1403.0)) * (s / c));
        (a, a * (s / c))
    }
}

/// Process-wide tables keyed by both perceptual algorithm and destination.
/// Builds occur outside the lock; concurrent duplicate builds are harmless.
fn cmax_table_cached(space: Space, destination: &'static RgbColorSpace, cam: &Cam16Viewing) -> Arc<Vec<f64>> {
    static CACHE: LazyLock<Mutex<HashMap<(Space, &'static str), Arc<Vec<f64>>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
    let key = (space, destination.name);
    let cache = &*CACHE;
    if let Some(table) = cache.lock().unwrap().get(&key) {
        return table.clone();
    }
    let table = Arc::new(build_cmax_table(space, destination, cam));
    cache.lock().unwrap().insert(key, table.clone());
    table
}

/// Bisect the max in-gamut chroma at each `(L, h)` grid node for the given
/// perceptual space, mirroring `_build_polar_perceptual_c_max_table`. Built
/// once per space (see `cmax_table_cached`).
fn build_cmax_table(space: Space, destination: &RgbColorSpace, cam: &Cam16Viewing) -> Vec<f64> {
    use rayon::prelude::*;
    let (l_min, l_max, chroma_upper) = space.table_geometry();
    let mut table = vec![0.0f64; N_L * N_H];
    table
        .par_chunks_exact_mut(N_H)
        .enumerate()
        .for_each(|(i, row)| {
            // Grid value is the lookup-lightness; map to reconstruction
            // lightness (Lr→L for oklrab) before the gamut check.
            let lookup_l = l_min + (l_max - l_min) * (i as f64) / ((N_L - 1) as f64);
            let l = space.recon_lightness(lookup_l);
            for (j, out) in row.iter_mut().enumerate() {
                let h = h_grid(j);
                let (cos_h, sin_h) = (h.cos(), h.sin());
                let mut lo = 0.0f64;
                let mut hi = chroma_upper;
                for _ in 0..N_BISECT {
                    let mid = (lo + hi) * 0.5;
                    let rgb = space.to_rgb([l, mid * cos_h, mid * sin_h], destination, cam);
                    let in_gamut = rgb.iter().all(|&v| v >= -1e-6 && v <= 1.0 + 1e-6);
                    if in_gamut {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                *out = lo;
            }
        });
    table
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn default_oklch_matches_fresh_python_edge_references() {
        let inputs = [
            [2.0, 0.0, 0.0],
            [0.0, 2.0, 2.0],
            [0.0, 0.0, 0.0],
            [1.2, -0.1, 0.4],
        ];
        // Independently generated against pinned upstream runtime 0.3.4.
        // Keep these values fixed: they guard the default algorithm and its
        // one-sided lightness compression, rather than repinning Rust output.
        let expected = [
            [1.000131672083069, 0.3335526009210533, 0.26771081930716234],
            [0.9608581941201273, 1.0002332007940766, 0.9981825436644369],
            [0.0, 0.0, 0.0],
            [0.9402427137627506, 0.0003375188741574854, 0.36596656377169123],
        ];
        let compressor = OutputGamutCompress::build(
            &OutputGamutCompressParams::default(),
            "sRGB",
        ).unwrap();
        for (rgb, want) in inputs.into_iter().zip(expected) {
            let got = compressor.compress(rgb);
            for i in 0..3 {
                assert!(
                    (got[i] - want[i]).abs() < 1e-6,
                    "{rgb:?}: {got:?}, expected {want:?}"
                );
            }
        }
    }
    #[test]
    fn jzazbz_forward_inverse_match_fresh_python_constants() {
        let cases = [
            ([0.0, 0.0, 0.0], [0.0, -4.33057640838877e-27, -4.96277727979887e-28], [0.0, 0.0, 0.0]),
            ([0.9504559270516716, 1.0, 1.0890577507598784], [0.16717342769906365, -0.0001403351730878002, -0.00010225282099334996], [0.9504559270517058, 1.0000000000000124, 1.089057750759923]),
            ([0.2, 0.1, -0.01], [0.0705700543382176, 0.08128666063727495, 0.09067046006630623], [0.20000000000001084, 0.09999999999999104, -0.009999999999997755]),
        ];
        for (xyz, jab, inverse) in cases {
            let forward = xyz_to_jzazbz(xyz);
            let back = jzazbz_to_xyz(jab);
            for i in 0..3 {
                assert!((forward[i] - jab[i]).abs() < 1e-12, "{xyz:?}: {forward:?}");
                assert!((back[i] - inverse[i]).abs() < 1e-11, "{jab:?}: {back:?}");
            }
        }
    }

    #[test]
    fn cam16_adapts_to_destination_whitepoint() {
        let references = [
            ("sRGB", [1.0228770275436545, 0.9852074782801457, 0.9285450586783286], 37.16907530221132),
            ("DCI-P3", [1.0379655182795853, 0.978366094618728, 1.04121606038871], 37.156162885738674),
            ("ProPhoto RGB", [1.0048858995308803, 0.9991651597854243, 1.1823902986704795], 37.17349142011997),
            ("ACES2065-1", [1.0181013144461637, 0.9889551394254175, 0.9923024021726741], 37.169941806273705),
        ];
        for (destination, d_rgb, a_w) in references {
            let cs = resolve(destination).unwrap();
            let cam = Cam16Viewing::new(cs.whitepoint_xyz());
            for i in 0..3 { assert!((cam.d_rgb[i] - d_rgb[i]).abs() < 1e-14); }
            assert!((cam.a_w - a_w).abs() < 1e-12);
            let xyz = cs.whitepoint_xyz();
            let back = cam16ucs_to_xyz(xyz_to_cam16ucs(xyz, &cam), &cam);
            for i in 0..3 { assert!((back[i] - xyz[i]).abs() < 1e-12); }
        }
    }

    #[test]
    fn disabled_lightness_matches_pinned_python() {
        let inputs = [[2.0, 0.0, 0.0], [0.01, 0.005, 0.02], [1.2, -0.1, 0.4]];
        let references: &[(&str, &str, [[f64; 3]; 3])] = &[
            ("oklch", "sRGB", [[0.9999502607939913, 0.3336117822943832, 0.2677783559617964], [0.00999680553911392, 0.005004552721989224, 0.01998010779258867], [0.921102605384875, 0.00779662847232664, 0.36339051119064136]]),
            ("oklch", "DCI-P3", [[0.9999939574793686, 0.3342470371354583, 0.31902542499990877], [0.009998618213796934, 0.0050019438310469, 0.01999011341157336], [0.9203567007330993, 0.007295735299738715, 0.3753692561192554]]),
            ("oklch", "ProPhoto RGB", [[0.999999951502701, 0.5725019013395553, 0.6331637913412947], [0.009998508276040994, 0.0050009746229878116, 0.01999501623114913], [0.969545666767737, 0.028419517885580396, 0.4264534798155361]]),
            ("oklch", "ACES2065-1", [[0.9999479628775944, 0.7791713540010654, 0.7705831044351357], [0.009999301926185694, 0.005000279466504453, 0.01999728030655998], [0.9778225259652394, 0.05055329579142336, 0.42122114356819285]]),
            ("oklrab", "sRGB", [[0.9998968385593685, 0.3336292099779188, 0.26779824591096096], [0.009996801426300956, 0.005004557660767545, 0.019980084755050587], [0.921101983884097, 0.007796870837573754, 0.3633904273823209]]),
            ("oklrab", "DCI-P3", [[0.999976691082727, 0.3342526483134597, 0.31903340372976663], [0.009998615563955811, 0.005001947558566613, 0.019990094453170053], [0.9203548906467645, 0.007296438303749417, 0.37536908934854984]]),
            ("oklrab", "ProPhoto RGB", [[0.9999952078629938, 0.5725047634891345, 0.6331685460073101], [0.009998501230392174, 0.005000979404664618, 0.01999499087077371], [0.96862667546514, 0.02893989144612738, 0.42656214158310957]]),
            ("oklrab", "ACES2065-1", [[0.9999914396462434, 0.7791321804137881, 0.7705235347806088], [0.009999298779052495, 0.005000280726432953, 0.01999726804544119], [0.9778057584263041, 0.05056487239336675, 0.4212227775645808]]),
            ("jzazbz", "sRGB", [[1.0001755660963343, 0.4238363873947827, 0.33677520485712037], [0.009995979181245155, 0.00501027961292606, 0.019967933094343273], [0.9768078151721247, 0.01524507950435017, 0.3871939758264947]]),
            ("jzazbz", "DCI-P3", [[1.0001462569943491, 0.4453081899264903, 0.4293914859029522], [0.00999857643525042, 0.005004881322520727, 0.019983234450790765], [0.9768638152802378, 0.020214232860892346, 0.4044213482794233]]),
            ("jzazbz", "ProPhoto RGB", [[0.9355488943551639, 0.9776190449279439, 1.2735237196610298], [0.009997436251877066, 0.005003415926877663, 0.019989283990374637], [0.9902184624441568, 0.09466866888116787, 0.48608410676205355]]),
            ("jzazbz", "ACES2065-1", [[1.150579472480684, 1.1623284547938184, 1.2429518912630804], [0.009997299434950733, 0.005003100010569582, 0.01998609308135035], [0.9952673151418769, 0.1573517463603509, 0.49242217142741124]]),
            ("cam16ucs", "sRGB", [[0.998862466762302, 0.3246094028643281, 0.25578338753138874], [0.009980054497212823, 0.005026279839882753, 0.019855856127262914], [0.878749761195602, 0.02028000337135383, 0.3481821566595246]]),
            ("cam16ucs", "DCI-P3", [[0.9988542207740082, 0.32794786237029644, 0.24207446887168632], [0.00997982591403333, 0.005026482013543773, 0.01984969490169732], [0.8732029749544876, 0.02107884135292239, 0.3484700548165006]]),
            ("cam16ucs", "ProPhoto RGB", [[0.9994396202316868, 0.5176060199136153, 0.4404041262255034], [0.009974476756004977, 0.005015475071365567, 0.01987672582051979], [0.9108547216735219, 0.037396026757958375, 0.38645957060697356]]),
            ("cam16ucs", "ACES2065-1", [[0.9999091662068789, 0.696453911904479, 0.6277447414374682], [0.009995947300312528, 0.005001038628580664, 0.019982950634259998], [0.9259541075641037, 0.050305732704408274, 0.4033039335268]]),
        ];
        for &(algorithm, destination, expected) in references {
            let params = OutputGamutCompressParams {
                algorithm: algorithm.into(), knee: [0.0, 1.0, 6.0], lightness_compression: None,
            };
            let comp = OutputGamutCompress::build(&params, destination).unwrap();
            for (rgb, want) in inputs.iter().zip(expected) {
                let got = comp.compress(*rgb);
                for i in 0..3 {
                    assert!((got[i] - want[i]).abs() < 1e-6,
                        "{algorithm}/{destination} {rgb:?}: {got:?} expected {want:?}");
                }
            }
        }
    }


    // Fresh Python probes from commit 3bb2c2d2801ff68b92019cf1dbcbb133d60832bc,
    // with knee/lightness values cast through np.float32 to match the params API.
    // Cases cover in-gamut, above-white red/cyan, black, darks and negative channels.
    #[test]
    fn all_algorithms_and_destinations_match_pinned_python() {
        let inputs = [
            [0.5, 0.4, 0.3], [2.0, 0.0, 0.0], [0.0, 2.0, 2.0],
            [0.0, 0.0, 0.0], [0.01, 0.005, 0.02], [1.2, -0.1, 0.4],
        ];
        let references: &[(&str, &str, [[f64; 3]; 6])] = &[
            ("off", "sRGB", [
                [0.5, 0.4, 0.3],
                [2.0, 0.0, 0.0],
                [0.0, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.01, 0.005, 0.02],
                [1.2, -0.1, 0.4],
            ]),
            ("off", "DCI-P3", [
                [0.5, 0.4, 0.3],
                [2.0, 0.0, 0.0],
                [0.0, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.01, 0.005, 0.02],
                [1.2, -0.1, 0.4],
            ]),
            ("off", "Display P3", [
                [0.5, 0.4, 0.3],
                [2.0, 0.0, 0.0],
                [0.0, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.01, 0.005, 0.02],
                [1.2, -0.1, 0.4],
            ]),
            ("off", "Adobe RGB (1998)", [
                [0.5, 0.4, 0.3],
                [2.0, 0.0, 0.0],
                [0.0, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.01, 0.005, 0.02],
                [1.2, -0.1, 0.4],
            ]),
            ("off", "ITU-R BT.2020", [
                [0.5, 0.4, 0.3],
                [2.0, 0.0, 0.0],
                [0.0, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.01, 0.005, 0.02],
                [1.2, -0.1, 0.4],
            ]),
            ("off", "ProPhoto RGB", [
                [0.5, 0.4, 0.3],
                [2.0, 0.0, 0.0],
                [0.0, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.01, 0.005, 0.02],
                [1.2, -0.1, 0.4],
            ]),
            ("off", "ACES2065-1", [
                [0.5, 0.4, 0.3],
                [2.0, 0.0, 0.0],
                [0.0, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.01, 0.005, 0.02],
                [1.2, -0.1, 0.4],
            ]),
            ("aces_rgc", "sRGB", [
                [0.5, 0.4000010666268463, 0.30013620807161223],
                [2.0, 0.21820256371932145, 0.21820256371932145],
                [0.21820256371932145, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.010025806953548249, 0.005403960473700402, 0.02],
                [1.2, 0.09255148433961402, 0.4111416191919668],
            ]),
            ("aces_rgc", "DCI-P3", [
                [0.5, 0.4000010666268463, 0.30013620807161223],
                [2.0, 0.21820256371932145, 0.21820256371932145],
                [0.21820256371932145, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.010025806953548249, 0.005403960473700402, 0.02],
                [1.2, 0.09255148433961402, 0.4111416191919668],
            ]),
            ("aces_rgc", "Display P3", [
                [0.5, 0.4000010666268463, 0.30013620807161223],
                [2.0, 0.21820256371932145, 0.21820256371932145],
                [0.21820256371932145, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.010025806953548249, 0.005403960473700402, 0.02],
                [1.2, 0.09255148433961402, 0.4111416191919668],
            ]),
            ("aces_rgc", "Adobe RGB (1998)", [
                [0.5, 0.4000010666268463, 0.30013620807161223],
                [2.0, 0.21820256371932145, 0.21820256371932145],
                [0.21820256371932145, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.010025806953548249, 0.005403960473700402, 0.02],
                [1.2, 0.09255148433961402, 0.4111416191919668],
            ]),
            ("aces_rgc", "ITU-R BT.2020", [
                [0.5, 0.4000010666268463, 0.30013620807161223],
                [2.0, 0.21820256371932145, 0.21820256371932145],
                [0.21820256371932145, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.010025806953548249, 0.005403960473700402, 0.02],
                [1.2, 0.09255148433961402, 0.4111416191919668],
            ]),
            ("aces_rgc", "ProPhoto RGB", [
                [0.5, 0.4000010666268463, 0.30013620807161223],
                [2.0, 0.21820256371932145, 0.21820256371932145],
                [0.21820256371932145, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.010025806953548249, 0.005403960473700402, 0.02],
                [1.2, 0.09255148433961402, 0.4111416191919668],
            ]),
            ("aces_rgc", "ACES2065-1", [
                [0.5, 0.4000010666268463, 0.30013620807161223],
                [2.0, 0.21820256371932145, 0.21820256371932145],
                [0.21820256371932145, 2.0, 2.0],
                [0.0, 0.0, 0.0],
                [0.010025806953548249, 0.005403960473700402, 0.02],
                [1.2, 0.09255148433961402, 0.4111416191919668],
            ]),
            ("oklch", "sRGB", [
                [0.49934573352277356, 0.3994604508168424, 0.2995407590112581],
                [0.9999165297202253, 0.32648626769775396, 0.2608847282277998],
                [0.5001302710890511, 1.0000925605225777, 0.989460142892127],
                [0.0, 0.0, 0.0],
                [0.00999680553911392, 0.005004552721989224, 0.01998010779258867],
                [0.921102605384875, 0.00779662847232664, 0.36339051119064136],
            ]),
            ("oklch", "DCI-P3", [
                [0.49941306213540515, 0.3995187716227326, 0.29959143354062334],
                [1.0000022982080385, 0.32660514600313983, 0.3099974348499589],
                [0.9271315349972798, 0.8387669686425365, 1.000453050898872],
                [0.0, 0.0, 0.0],
                [0.009998618213796934, 0.0050019438310469, 0.01999011341157336],
                [0.9203567007330993, 0.007295735299738715, 0.3753692561192554],
            ]),
            ("oklch", "Display P3", [
                [0.49929271382654167, 0.39939218465345494, 0.2994987516864163],
                [0.9998893125865398, 0.36424138086830665, 0.2865333372686254],
                [0.5248726834286841, 1.0000724910376422, 0.9858139518979456],
                [0.0, 0.0, 0.0],
                [0.009996151843016743, 0.005005983099803823, 0.019974979435627947],
                [0.9469415392229072, 0.011374748899876407, 0.3761898645659995],
            ]),
            ("oklch", "Adobe RGB (1998)", [
                [0.4990560785337207, 0.39917838778501075, 0.29932540076667513],
                [0.9999771312244045, 0.45479038637541147, 0.37718963370235736],
                [0.5588978309754336, 1.000296868836171, 0.9777890023298255],
                [0.0, 0.0, 0.0],
                [0.009996709117249748, 0.0050068983962796975, 0.01997365022654038],
                [0.9781741616509781, 0.03401568662142386, 0.4002712033385133],
            ]),
            ("oklch", "ITU-R BT.2020", [
                [0.49905150932492576, 0.3991838342453643, 0.29932684489149064],
                [0.9999765588196147, 0.426421584700942, 0.34181963746807115],
                [0.5501899283002919, 1.0003356790012914, 0.977902707983484],
                [0.0, 0.0, 0.0],
                [0.00999649618765399, 0.005005237245299915, 0.0199772979593309],
                [0.964152235503913, 0.024504972866060064, 0.3917491272775895],
            ]),
            ("oklch", "ProPhoto RGB", [
                [0.49853817001180306, 0.3987427166422221, 0.29897965231261064],
                [0.9998816340366617, 0.4578845686823861, 0.4789006865931586],
                [0.7960320110178604, 0.8328387159588693, 1.0848255863651304],
                [0.0, 0.0, 0.0],
                [0.009998508276040994, 0.0050009746229878116, 0.01999501623114913],
                [0.9694925057235474, 0.028317785764526582, 0.4263362210971101],
            ]),
            ("oklch", "ACES2065-1", [
                [0.4978660521531581, 0.3981548560442411, 0.2984486581297352],
                [0.9998986101460459, 0.5038803596462992, 0.44339646010666534],
                [0.5019056701262393, 1.0000449822413404, 0.9902887767993418],
                [0.0, 0.0, 0.0],
                [0.009999301926185694, 0.005000279466504453, 0.01999728030655998],
                [0.9772871025618578, 0.048484814919466226, 0.41953762238037196],
            ]),
            ("oklrab", "sRGB", [
                [0.49934573352343736, 0.39946045081670484, 0.29954075901042154],
                [0.9998143119786475, 0.32651963573551523, 0.26092249154218594],
                [0.4998450710876685, 1.0001915827711596, 0.9895569146558406],
                [0.0, 0.0, 0.0],
                [0.009996801426300956, 0.005004557660767545, 0.019980084755050587],
                [0.921101983884097, 0.007796870837573754, 0.3633904273823209],
            ]),
            ("oklrab", "DCI-P3", [
                [0.4994130622483394, 0.3995187716262398, 0.29959143318898385],
                [0.99993586807609, 0.32662675467062224, 0.31002784150811924],
                [0.8825234558289646, 0.8556293095720408, 1.008781737017845],
                [0.0, 0.0, 0.0],
                [0.009998615563955811, 0.005001947558566613, 0.019990094453170053],
                [0.9203548906467645, 0.007296438303749417, 0.37536908934854984],
            ]),
            ("oklrab", "Display P3", [
                [0.499292713807341, 0.39939218465802956, 0.29949875171139567],
                [0.999862393644534, 0.36425137348529124, 0.28654493697089534],
                [0.5245003884044274, 1.0002229814050467, 0.9859582620804624],
                [0.0, 0.0, 0.0],
                [0.009996148544221305, 0.005005988228208253, 0.019974957990148385],
                [0.9485009342282914, 0.010681054159839389, 0.3763421337208791],
            ]),
            ("oklrab", "Adobe RGB (1998)", [
                [0.49905607845163785, 0.39917838781542797, 0.2993254008930984],
                [1.0000283619743493, 0.4547636297789811, 0.37715792215526134],
                [0.5591249950574968, 1.0001671519519628, 0.9776692811127159],
                [0.0, 0.0, 0.0],
                [0.009996701653859652, 0.005006913456880949, 0.019973593407592007],
                [0.9782355975157431, 0.033978119203348346, 0.40027112864037245],
            ]),
            ("oklrab", "ITU-R BT.2020", [
                [0.4990515093313734, 0.39918383424300263, 0.29932684488172084],
                [0.9999644311412668, 0.4264273932038013, 0.34182649134987053],
                [0.5506236026342513, 1.0001023650314638, 0.977687404656365],
                [0.0, 0.0, 0.0],
                [0.009996494110314912, 0.005005240350053891, 0.01997728450184566],
                [0.9658491842286623, 0.02359667020279201, 0.39181380226074813],
            ]),
            ("oklrab", "ProPhoto RGB", [
                [0.4985381689104651, 0.3987427172990209, 0.2989796554397136],
                [0.9998954105921103, 0.45787637858564656, 0.4788884515211853],
                [0.7960320110178604, 0.8328387159588693, 1.0848255863651304],
                [0.0, 0.0, 0.0],
                [0.009998501230392174, 0.005000979404664618, 0.01999499087077371],
                [0.9685581742437659, 0.02884683302502178, 0.4264466916940157],
            ]),
            ("oklrab", "ACES2065-1", [
                [0.49786605221211205, 0.39815485598151956, 0.29844865794936054],
                [0.9999068326134015, 0.503873516744231, 0.4433880541657244],
                [0.5019054238893522, 1.0000452151642096, 0.9902889611184621],
                [0.0, 0.0, 0.0],
                [0.009999298779052495, 0.005000280726432953, 0.01999726804544119],
                [0.9772610504356003, 0.048502792638773405, 0.4195401597958421],
            ]),
            ("jzazbz", "sRGB", [
                [0.5000017364461872, 0.40002481988024574, 0.3000073833155131],
                [1.000143220384593, 0.4149606328965636, 0.3277633612000083],
                [0.6561291236699556, 1.0001210702326222, 0.9883311245004702],
                [0.0, 0.0, 0.0],
                [0.009995979181245155, 0.00501027961292606, 0.019967933094343273],
                [0.9768078151721247, 0.01524507950435017, 0.3871939758264947],
            ]),
            ("jzazbz", "DCI-P3", [
                [0.499996653624679, 0.3999997470083555, 0.3000089866280967],
                [1.000098962665164, 0.42948701934824834, 0.40922083897846107],
                [0.9256879950352853, 0.8423096207302826, 1.0032821511176244],
                [0.0, 0.0, 0.0],
                [0.00999857643525042, 0.005004881322520727, 0.019983234450790765],
                [0.9768638152802378, 0.020214232860892346, 0.4044213482794233],
            ]),
            ("jzazbz", "Display P3", [
                [0.4999991255233155, 0.400000235735381, 0.30000115634704333],
                [1.0001901464444727, 0.47851560779670727, 0.37622295645218323],
                [0.6646878027502237, 1.0002378240767156, 0.9846385080218973],
                [0.0, 0.0, 0.0],
                [0.00999609473981722, 0.005014152913092596, 0.019960146866164933],
                [0.9825649590680957, 0.03451920619773663, 0.40171123786433915],
            ]),
            ("jzazbz", "Adobe RGB (1998)", [
                [0.5000042694411769, 0.39999512841296, 0.3000006811042433],
                [1.0002060212021617, 0.5757731119116432, 0.47824981020882584],
                [0.6606262920047352, 1.0001716565234449, 0.9756981464198122],
                [0.0, 0.0, 0.0],
                [0.009997375480093058, 0.00501549696490165, 0.01995947201688051],
                [0.9894577546450531, 0.07661944277869509, 0.4279648620139722],
            ]),
            ("jzazbz", "ITU-R BT.2020", [
                [0.49999891908972693, 0.4000004915610714, 0.30000176123644684],
                [1.0001953357348008, 0.5723926190814433, 0.4650551540133733],
                [0.6551479741511795, 1.000209115596722, 0.9751045065843452],
                [0.0, 0.0, 0.0],
                [0.009996654509161535, 0.005013771469600918, 0.01996143127626533],
                [0.9888256337409373, 0.07106759637026412, 0.4261553103572509],
            ]),
            ("jzazbz", "ProPhoto RGB", [
                [0.4999954557523404, 0.40001298014442727, 0.30010447005163204],
                [1.000095234621163, 0.6056556201741263, 0.6337830614863217],
                [0.7764020525817525, 0.8113156364058667, 1.0568837760687833],
                [0.0, 0.0, 0.0],
                [0.009997436251877066, 0.005003415926877663, 0.019989283990374637],
                [0.9895845050040768, 0.0899886511902417, 0.4816555231981448],
            ]),
            ("jzazbz", "ACES2065-1", [
                [0.49993216184689626, 0.3999523837636473, 0.2999716189286772],
                [1.0000148233627666, 0.6372243446324849, 0.557636836603362],
                [0.48124810050375333, 1.0001150567599257, 0.9754895502270114],
                [0.0, 0.0, 0.0],
                [0.009997299434950733, 0.005003100010569582, 0.01998609308135035],
                [0.9927751537772631, 0.12425675936164195, 0.4691359062462376],
            ]),
            ("cam16ucs", "sRGB", [
                [0.49974636907090725, 0.3998127332147245, 0.2998389761855068],
                [0.9987900430928757, 0.32032525612697726, 0.25154805865211866],
                [0.4647665684577933, 0.9978350182092256, 0.9907180484971386],
                [0.0, 0.0, 0.0],
                [0.009980054497212823, 0.005026279839882753, 0.019855856127262914],
                [0.878749761195602, 0.02028000337135383, 0.3481821566595246],
            ]),
            ("cam16ucs", "DCI-P3", [
                [0.4997316482007821, 0.39977765586250746, 0.299823000413174],
                [0.9987848322679167, 0.3231250748787379, 0.2372321862845484],
                [0.4745741372490661, 0.9962303706614082, 0.9969446969070116],
                [0.0, 0.0, 0.0],
                [0.00997982591403333, 0.005026482013543773, 0.01984969490169732],
                [0.8732029749544876, 0.02107884135292239, 0.3484700548165006],
            ]),
            ("cam16ucs", "Display P3", [
                [0.4997164890340275, 0.399763516063939, 0.29981121303051994],
                [0.9990088687975526, 0.3559744449243264, 0.25695686085376657],
                [0.4942805567429199, 0.9979012854249923, 0.9893916136618576],
                [0.0, 0.0, 0.0],
                [0.009978341917191614, 0.0050353771948561206, 0.019824525448133536],
                [0.9003915293152127, 0.025774213873688254, 0.36258951792934163],
            ]),
            ("cam16ucs", "Adobe RGB (1998)", [
                [0.4995729480029829, 0.3996348418359395, 0.2997128003152345],
                [0.9994935311638684, 0.4488385151327933, 0.36822577175201704],
                [0.5384891360245982, 0.998874186121169, 0.9862871033642124],
                [0.0, 0.0, 0.0],
                [0.009982026691035342, 0.005037406953818647, 0.019833981932895515],
                [0.9438706987167721, 0.04951290193939527, 0.39689919644727084],
            ]),
            ("cam16ucs", "ITU-R BT.2020", [
                [0.4995707479703799, 0.3996409212160287, 0.29971286756398297],
                [0.9991374366806633, 0.4122024849421586, 0.3099937123322026],
                [0.5290802898675302, 0.9988481846064621, 0.9881172025327244],
                [0.0, 0.0, 0.0],
                [0.009976541156208647, 0.005033434712053331, 0.019826898008761574],
                [0.9217091455127547, 0.03441889285975853, 0.3815043836479474],
            ]),
            ("cam16ucs", "ProPhoto RGB", [
                [0.49931712467791123, 0.3993929696472845, 0.29952752381649456],
                [0.998822609582701, 0.434550264159677, 0.35015886726140555],
                [0.5465436896570529, 0.9945998396584808, 0.9968680810447876],
                [0.0, 0.0, 0.0],
                [0.009974476756004977, 0.005015475071365567, 0.01987672582051979],
                [0.9108547216735219, 0.037396026757958375, 0.38645957060697356],
            ]),
            ("cam16ucs", "ACES2065-1", [
                [0.4987539858371003, 0.39895397272145705, 0.29916132468519485],
                [0.9989578547925394, 0.4739407330518892, 0.37669405022607205],
                [0.5350084859318177, 0.9972348152958707, 0.9857742220531798],
                [0.0, 0.0, 0.0],
                [0.009995947300312528, 0.005001038628580664, 0.019982950634259998],
                [0.9258119346940561, 0.05021968423647454, 0.40318870623020664],
            ]),
        ];
        for &(algorithm, destination, expected) in references {
            let params = OutputGamutCompressParams {
                algorithm: algorithm.into(),
                knee: [0.0, 1.0, 6.0],
                lightness_compression: Some([0.7, 1.0, 2.2]),
            };
            let compressor = OutputGamutCompress::build(&params, destination).unwrap();
            for (rgb, want) in inputs.iter().zip(expected) {
                let got = compressor.compress(*rgb);
                for channel in 0..3 {
                    assert!((got[channel] - want[channel]).abs() < 1e-6,
                        "{algorithm} / {destination} at {rgb:?}: {got:?}, expected {want:?}");
                }
            }
        }
    }

    #[test]
    fn invalid_configuration_fails_before_processing() {
        let mut params = OutputGamutCompressParams::default();
        assert!(OutputGamutCompress::build(&params, "unknown").is_err());
        params.algorithm = "oklab".into();
        assert!(OutputGamutCompress::build(&params, "sRGB").is_err());
        params.algorithm = "off".into();
        for knee in [[-0.1, 1.0, 6.0], [1.0, 1.0, 6.0], [0.0, 0.0, 6.0],
            [0.0, 1.0, 0.0], [f32::NAN, 1.0, 6.0], [0.0, f32::INFINITY, 6.0],
            [0.0, 1.0, f32::INFINITY]] {
            params.knee = knee;
            assert!(OutputGamutCompress::build(&params, "sRGB").is_err());
        }
        params.knee = [0.0, 1.0, 6.0];
        for lightness in [[1.0, 1.0, 2.2], [0.7, 0.0, 2.2], [0.7, 1.0, -1.0],
            [0.7, f32::INFINITY, 2.2], [f32::NAN, 1.0, 2.2]] {
            params.lightness_compression = Some(lightness);
            assert!(OutputGamutCompress::build(&params, "sRGB").is_err());
        }
    }

    #[test]
    fn gpu_support_requires_cpu_for_active_unsupported_transforms() {
        for destination in ["sRGB", "DCI-P3", "Display P3", "Adobe RGB (1998)",
            "ITU-R BT.2020", "ProPhoto RGB", "ACES2065-1"] {
            for algorithm in ["off", "aces_rgc", "oklch", "oklrab", "jzazbz", "cam16ucs"] {
                let params = OutputGamutCompressParams { algorithm: algorithm.into(), ..Default::default() };
                let compressor = OutputGamutCompress::build(&params, destination).unwrap();
                assert_eq!(compressor.is_active(), algorithm != "off");
                assert_eq!(compressor.gpu_params().is_some(),
                    destination == "sRGB" && algorithm != "off" && algorithm != "jzazbz");
                if algorithm == "off" {
                    assert_eq!(compressor.compress([1.7, -0.2, 0.3]), [1.7, -0.2, 0.3]);
                }
            }
        }
    }
}
