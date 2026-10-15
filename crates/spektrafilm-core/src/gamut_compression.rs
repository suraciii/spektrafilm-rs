//! Output gamut compression with ACES RGC and perceptual chroma reduction.
//! Perceptual tables target the destination's linear RGB cube and own whitepoint.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use crate::params::OutputGamutCompressParams;

use spektrafilm_math::colorspace::{RgbColorSpace, SRGB, resolve};

mod cam16;
mod jzazbz;

use cam16::{Cam16Viewing, cam16ucs_to_xyz, xyz_to_cam16ucs};
use jzazbz::{jzazbz_to_xyz, xyz_to_jzazbz};

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

use crate::math::mat_vec;

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
    fn rgb_to_perceptual(self, rgb: [f64; 3], cs: &RgbColorSpace, cam: &Cam16Viewing) -> [f64; 3] {
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
            l_min: 0.0,
            l_max: 1.0,
            cmax: Arc::new(Vec::new()),
        }
    }

    pub fn is_active(&self) -> bool {
        self.kind != Kind::Off
    }

    /// Resolve and validate before building a destination-specific compressor.
    pub fn build(
        params: &OutputGamutCompressParams,
        output_color_space: &str,
    ) -> Result<Self, String> {
        let destination = resolve(output_color_space)?;
        let kind = match params.algorithm.as_str() {
            "off" => Kind::Off,
            "aces_rgc" => Kind::AcesRgc,
            "oklch" => Kind::Perceptual(Space::Oklch),
            "oklrab" => Kind::Perceptual(Space::Oklrab),
            "jzazbz" => Kind::Perceptual(Space::Jzazbz),
            "cam16ucs" => Kind::Perceptual(Space::Cam16ucs),
            other => {
                return Err(format!(
                    "unknown output gamut compression algorithm {other:?}"
                ));
            }
        };
        let knee =
            crate::params::validation::validate_gamut_triplet("output gamut knee", params.knee)?;
        let cam = Cam16Viewing::new(destination.whitepoint_xyz());
        let mut result = Self {
            kind,
            destination,
            cam,
            knee,
            l_min: 0.0,
            l_max: 1.0,
            cmax: Arc::new(Vec::new()),
        };
        if let Kind::Perceptual(space) = kind {
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
        let lab = space.rgb_to_perceptual(rgb, self.destination, &self.cam);
        let l = lab[0];
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
        space.to_rgb(
            [l, c_new * h.cos(), c_new * h.sin()],
            self.destination,
            &self.cam,
        )
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

type CmaxTableCache = Mutex<HashMap<(Space, &'static str), Arc<Vec<f64>>>>;

/// Process-wide tables keyed by both perceptual algorithm and destination.
/// Builds occur outside the lock; concurrent duplicate builds are harmless.
fn cmax_table_cached(
    space: Space,
    destination: &'static RgbColorSpace,
    cam: &Cam16Viewing,
) -> Arc<Vec<f64>> {
    static CACHE: LazyLock<CmaxTableCache> = LazyLock::new(|| Mutex::new(HashMap::new()));
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
                    let in_gamut = rgb.iter().all(|&v| (-1e-6..=1.0 + 1e-6).contains(&v));
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
    fn perceptual_chroma_matches_pinned_python() {
        // Pinned upstream references with lightness compression disabled.
        // These values continue to guard the unchanged chroma algorithms.
        let inputs = [[2.0, 0.0, 0.0], [0.01, 0.005, 0.02], [1.2, -0.1, 0.4]];
        let references: &[(&str, &str, [[f64; 3]; 3])] = &[
            (
                "oklch",
                "sRGB",
                [
                    [0.9999502607939913, 0.3336117822943832, 0.2677783559617964],
                    [
                        0.00999680553911392,
                        0.005004552721989224,
                        0.01998010779258867,
                    ],
                    [0.921102605384875, 0.00779662847232664, 0.36339051119064136],
                ],
            ),
            (
                "oklch",
                "DCI-P3",
                [
                    [0.9999939574793686, 0.3342470371354583, 0.31902542499990877],
                    [
                        0.009998618213796934,
                        0.0050019438310469,
                        0.01999011341157336,
                    ],
                    [0.9203567007330993, 0.007295735299738715, 0.3753692561192554],
                ],
            ),
            (
                "oklch",
                "ProPhoto RGB",
                [
                    [0.999999951502701, 0.5725019013395553, 0.6331637913412947],
                    [
                        0.009998508276040994,
                        0.0050009746229878116,
                        0.01999501623114913,
                    ],
                    [0.969545666767737, 0.028419517885580396, 0.4264534798155361],
                ],
            ),
            (
                "oklch",
                "ACES2065-1",
                [
                    [0.9999479628775944, 0.7791713540010654, 0.7705831044351357],
                    [
                        0.009999301926185694,
                        0.005000279466504453,
                        0.01999728030655998,
                    ],
                    [0.9778225259652394, 0.05055329579142336, 0.42122114356819285],
                ],
            ),
            (
                "oklrab",
                "sRGB",
                [
                    [0.9998968385593685, 0.3336292099779188, 0.26779824591096096],
                    [
                        0.009996801426300956,
                        0.005004557660767545,
                        0.019980084755050587,
                    ],
                    [0.921101983884097, 0.007796870837573754, 0.3633904273823209],
                ],
            ),
            (
                "oklrab",
                "DCI-P3",
                [
                    [0.999976691082727, 0.3342526483134597, 0.31903340372976663],
                    [
                        0.009998615563955811,
                        0.005001947558566613,
                        0.019990094453170053,
                    ],
                    [
                        0.9203548906467645,
                        0.007296438303749417,
                        0.37536908934854984,
                    ],
                ],
            ),
            (
                "oklrab",
                "ProPhoto RGB",
                [
                    [0.9999952078629938, 0.5725047634891345, 0.6331685460073101],
                    [
                        0.009998501230392174,
                        0.005000979404664618,
                        0.01999499087077371,
                    ],
                    [0.96862667546514, 0.02893989144612738, 0.42656214158310957],
                ],
            ),
            (
                "oklrab",
                "ACES2065-1",
                [
                    [0.9999914396462434, 0.7791321804137881, 0.7705235347806088],
                    [
                        0.009999298779052495,
                        0.005000280726432953,
                        0.01999726804544119,
                    ],
                    [0.9778057584263041, 0.05056487239336675, 0.4212227775645808],
                ],
            ),
            (
                "jzazbz",
                "sRGB",
                [
                    [1.0001755660963343, 0.4238363873947827, 0.33677520485712037],
                    [
                        0.009995979181245155,
                        0.00501027961292606,
                        0.019967933094343273,
                    ],
                    [0.9768078151721247, 0.01524507950435017, 0.3871939758264947],
                ],
            ),
            (
                "jzazbz",
                "DCI-P3",
                [
                    [1.0001462569943491, 0.4453081899264903, 0.4293914859029522],
                    [
                        0.00999857643525042,
                        0.005004881322520727,
                        0.019983234450790765,
                    ],
                    [0.9768638152802378, 0.020214232860892346, 0.4044213482794233],
                ],
            ),
            (
                "jzazbz",
                "ProPhoto RGB",
                [
                    [0.9355488943551639, 0.9776190449279439, 1.2735237196610298],
                    [
                        0.009997436251877066,
                        0.005003415926877663,
                        0.019989283990374637,
                    ],
                    [0.9902184624441568, 0.09466866888116787, 0.48608410676205355],
                ],
            ),
            (
                "jzazbz",
                "ACES2065-1",
                [
                    [1.150579472480684, 1.1623284547938184, 1.2429518912630804],
                    [
                        0.009997299434950733,
                        0.005003100010569582,
                        0.01998609308135035,
                    ],
                    [0.9952673151418769, 0.1573517463603509, 0.49242217142741124],
                ],
            ),
            (
                "cam16ucs",
                "sRGB",
                [
                    [0.998862466762302, 0.3246094028643281, 0.25578338753138874],
                    [
                        0.009980054497212823,
                        0.005026279839882753,
                        0.019855856127262914,
                    ],
                    [0.878749761195602, 0.02028000337135383, 0.3481821566595246],
                ],
            ),
            (
                "cam16ucs",
                "DCI-P3",
                [
                    [0.9988542207740082, 0.32794786237029644, 0.24207446887168632],
                    [
                        0.00997982591403333,
                        0.005026482013543773,
                        0.01984969490169732,
                    ],
                    [0.8732029749544876, 0.02107884135292239, 0.3484700548165006],
                ],
            ),
            (
                "cam16ucs",
                "ProPhoto RGB",
                [
                    [0.9994396202316868, 0.5176060199136153, 0.4404041262255034],
                    [
                        0.009974476756004977,
                        0.005015475071365567,
                        0.01987672582051979,
                    ],
                    [
                        0.9108547216735219,
                        0.037396026757958375,
                        0.38645957060697356,
                    ],
                ],
            ),
            (
                "cam16ucs",
                "ACES2065-1",
                [
                    [0.9999091662068789, 0.696453911904479, 0.6277447414374682],
                    [
                        0.009995947300312528,
                        0.005001038628580664,
                        0.019982950634259998,
                    ],
                    [0.9259541075641037, 0.050305732704408274, 0.4033039335268],
                ],
            ),
        ];
        for &(algorithm, destination, expected) in references {
            let params = OutputGamutCompressParams {
                algorithm: algorithm.into(),
                knee: [0.0, 1.0, 6.0],
            };
            let comp = OutputGamutCompress::build(&params, destination).unwrap();
            for (rgb, want) in inputs.iter().zip(expected) {
                let got = comp.compress(*rgb);
                for i in 0..3 {
                    assert!(
                        (got[i] - want[i]).abs() < 1e-6,
                        "{algorithm}/{destination} {rgb:?}: {got:?} expected {want:?}"
                    );
                }
            }
        }
    }

    // Fresh Python probes from commit 3bb2c2d2801ff68b92019cf1dbcbb133d60832bc,
    // with knee values cast through np.float32 to match the params API.
    // Cases cover in-gamut, above-white red/cyan, black, darks and negative channels.
    #[test]
    fn off_and_aces_rgc_match_pinned_python() {
        let inputs = [
            [0.5, 0.4, 0.3],
            [2.0, 0.0, 0.0],
            [0.0, 2.0, 2.0],
            [0.0, 0.0, 0.0],
            [0.01, 0.005, 0.02],
            [1.2, -0.1, 0.4],
        ];
        let references: &[(&str, &str, [[f64; 3]; 6])] = &[
            (
                "off",
                "sRGB",
                [
                    [0.5, 0.4, 0.3],
                    [2.0, 0.0, 0.0],
                    [0.0, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.01, 0.005, 0.02],
                    [1.2, -0.1, 0.4],
                ],
            ),
            (
                "off",
                "DCI-P3",
                [
                    [0.5, 0.4, 0.3],
                    [2.0, 0.0, 0.0],
                    [0.0, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.01, 0.005, 0.02],
                    [1.2, -0.1, 0.4],
                ],
            ),
            (
                "off",
                "Display P3",
                [
                    [0.5, 0.4, 0.3],
                    [2.0, 0.0, 0.0],
                    [0.0, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.01, 0.005, 0.02],
                    [1.2, -0.1, 0.4],
                ],
            ),
            (
                "off",
                "Adobe RGB (1998)",
                [
                    [0.5, 0.4, 0.3],
                    [2.0, 0.0, 0.0],
                    [0.0, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.01, 0.005, 0.02],
                    [1.2, -0.1, 0.4],
                ],
            ),
            (
                "off",
                "ITU-R BT.2020",
                [
                    [0.5, 0.4, 0.3],
                    [2.0, 0.0, 0.0],
                    [0.0, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.01, 0.005, 0.02],
                    [1.2, -0.1, 0.4],
                ],
            ),
            (
                "off",
                "ProPhoto RGB",
                [
                    [0.5, 0.4, 0.3],
                    [2.0, 0.0, 0.0],
                    [0.0, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.01, 0.005, 0.02],
                    [1.2, -0.1, 0.4],
                ],
            ),
            (
                "off",
                "ACES2065-1",
                [
                    [0.5, 0.4, 0.3],
                    [2.0, 0.0, 0.0],
                    [0.0, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.01, 0.005, 0.02],
                    [1.2, -0.1, 0.4],
                ],
            ),
            (
                "aces_rgc",
                "sRGB",
                [
                    [0.5, 0.4000010666268463, 0.30013620807161223],
                    [2.0, 0.21820256371932145, 0.21820256371932145],
                    [0.21820256371932145, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.010025806953548249, 0.005403960473700402, 0.02],
                    [1.2, 0.09255148433961402, 0.4111416191919668],
                ],
            ),
            (
                "aces_rgc",
                "DCI-P3",
                [
                    [0.5, 0.4000010666268463, 0.30013620807161223],
                    [2.0, 0.21820256371932145, 0.21820256371932145],
                    [0.21820256371932145, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.010025806953548249, 0.005403960473700402, 0.02],
                    [1.2, 0.09255148433961402, 0.4111416191919668],
                ],
            ),
            (
                "aces_rgc",
                "Display P3",
                [
                    [0.5, 0.4000010666268463, 0.30013620807161223],
                    [2.0, 0.21820256371932145, 0.21820256371932145],
                    [0.21820256371932145, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.010025806953548249, 0.005403960473700402, 0.02],
                    [1.2, 0.09255148433961402, 0.4111416191919668],
                ],
            ),
            (
                "aces_rgc",
                "Adobe RGB (1998)",
                [
                    [0.5, 0.4000010666268463, 0.30013620807161223],
                    [2.0, 0.21820256371932145, 0.21820256371932145],
                    [0.21820256371932145, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.010025806953548249, 0.005403960473700402, 0.02],
                    [1.2, 0.09255148433961402, 0.4111416191919668],
                ],
            ),
            (
                "aces_rgc",
                "ITU-R BT.2020",
                [
                    [0.5, 0.4000010666268463, 0.30013620807161223],
                    [2.0, 0.21820256371932145, 0.21820256371932145],
                    [0.21820256371932145, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.010025806953548249, 0.005403960473700402, 0.02],
                    [1.2, 0.09255148433961402, 0.4111416191919668],
                ],
            ),
            (
                "aces_rgc",
                "ProPhoto RGB",
                [
                    [0.5, 0.4000010666268463, 0.30013620807161223],
                    [2.0, 0.21820256371932145, 0.21820256371932145],
                    [0.21820256371932145, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.010025806953548249, 0.005403960473700402, 0.02],
                    [1.2, 0.09255148433961402, 0.4111416191919668],
                ],
            ),
            (
                "aces_rgc",
                "ACES2065-1",
                [
                    [0.5, 0.4000010666268463, 0.30013620807161223],
                    [2.0, 0.21820256371932145, 0.21820256371932145],
                    [0.21820256371932145, 2.0, 2.0],
                    [0.0, 0.0, 0.0],
                    [0.010025806953548249, 0.005403960473700402, 0.02],
                    [1.2, 0.09255148433961402, 0.4111416191919668],
                ],
            ),
        ];
        for &(algorithm, destination, expected) in references {
            let params = OutputGamutCompressParams {
                algorithm: algorithm.into(),
                knee: [0.0, 1.0, 6.0],
            };
            let compressor = OutputGamutCompress::build(&params, destination).unwrap();
            for (rgb, want) in inputs.iter().zip(expected) {
                let got = compressor.compress(*rgb);
                for channel in 0..3 {
                    assert!(
                        (got[channel] - want[channel]).abs() < 1e-6,
                        "{algorithm} / {destination} at {rgb:?}: {got:?}, expected {want:?}"
                    );
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
        for knee in [
            [-0.1, 1.0, 6.0],
            [1.0, 1.0, 6.0],
            [0.0, 0.0, 6.0],
            [0.0, 1.0, 0.0],
            [f32::NAN, 1.0, 6.0],
            [0.0, f32::INFINITY, 6.0],
            [0.0, 1.0, f32::INFINITY],
        ] {
            params.knee = knee;
            assert!(OutputGamutCompress::build(&params, "sRGB").is_err());
        }
    }

    #[test]
    fn gpu_support_requires_cpu_for_active_unsupported_transforms() {
        for destination in [
            "sRGB",
            "DCI-P3",
            "Display P3",
            "Adobe RGB (1998)",
            "ITU-R BT.2020",
            "ProPhoto RGB",
            "ACES2065-1",
        ] {
            for algorithm in ["off", "aces_rgc", "oklch", "oklrab", "jzazbz", "cam16ucs"] {
                let params = OutputGamutCompressParams {
                    algorithm: algorithm.into(),
                    ..Default::default()
                };
                let compressor = OutputGamutCompress::build(&params, destination).unwrap();
                assert_eq!(compressor.is_active(), algorithm != "off");
                assert_eq!(
                    compressor.gpu_params().is_some(),
                    destination == "sRGB" && algorithm != "off" && algorithm != "jzazbz"
                );
                if algorithm == "off" {
                    assert_eq!(compressor.compress([1.7, -0.2, 0.3]), [1.7, -0.2, 0.3]);
                }
            }
        }
    }
}
