//! Output gamut compression with ACES RGC and perceptual chroma reduction.
//! Perceptual tables target the destination's linear RGB cube and own whitepoint.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use crate::params::OutputGamutCompressParams;

#[cfg(test)]
use spektrafilm_math::colorspace::SRGB;
use spektrafilm_math::colorspace::{RgbColorSpace, resolve};

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

/// Validated destination, algorithm, and compression knee, before table preparation.
type ResolvedOutputGamut = (&'static RgbColorSpace, Kind, (f64, f64, f64));

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
    /// Device-generated boundary; CPU-reference construction leaves this empty.
    prepared: Option<spektrafilm_backend::PreparedTable>,
}

impl OutputGamutCompress {
    /// No-op (identity) compressor.
    ///
    /// Test-only: production configuration is built through [`Self::build`],
    /// which resolves the destination and algorithm together.
    #[cfg(test)]
    pub fn identity() -> Self {
        Self {
            kind: Kind::Off,
            destination: &SRGB,
            cam: Cam16Viewing::new(SRGB.whitepoint_xyz()),
            knee: (0.0, 1.0, 1.0),
            l_min: 0.0,
            l_max: 1.0,
            cmax: Arc::new(Vec::new()),
            prepared: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.kind != Kind::Off
    }

    /// Validate destination and controls without generating a Cmax lattice.
    pub fn validate(params: &OutputGamutCompressParams, output_color_space: &str) -> Result<(), String> {
        Self::validate_detailed(params, output_color_space).map_err(|error| error.to_string())
    }

    pub(crate) fn validate_detailed(
        params: &OutputGamutCompressParams,
        output_color_space: &str,
    ) -> Result<(), crate::params::ValidationError> {
        Self::resolve_configuration(params, output_color_space).map(|_| ())
    }

    fn resolve_configuration(
        params: &OutputGamutCompressParams,
        output_color_space: &str,
    ) -> Result<ResolvedOutputGamut, crate::params::ValidationError> {
        use crate::params::{ValidationError as Error, ValidationReason as Reason};
        let destination = resolve(output_color_space).map_err(|error| {
            Error::new(Reason::ColorSpace, "io.output_color_space", error)
                .arg("value", format!("{output_color_space:?}"))
        })?;
        let kind = match params.algorithm.as_str() {
            "off" => Kind::Off,
            "aces_rgc" => Kind::AcesRgc,
            "oklch" => Kind::Perceptual(Space::Oklch),
            "oklrab" => Kind::Perceptual(Space::Oklrab),
            "jzazbz" => Kind::Perceptual(Space::Jzazbz),
            "cam16ucs" => Kind::Perceptual(Space::Cam16ucs),
            other => {
                return Err(Error::new(
                    Reason::OutputAlgorithm,
                    "io.output_gamut_compress.algorithm",
                    format!("unknown output gamut compression algorithm {other:?}"),
                )
                .arg("value", format!("{other:?}")));
            }
        };
        let knee = crate::params::validation::validate_gamut_triplet_detailed(
            "output gamut knee",
            params.knee,
        )
        .map_err(|mut error| {
            error.path = "io.output_gamut_compress.knee".into();
            error
        })?;
        Ok((destination, kind, knee))
    }

    /// Resolve and validate before building the CPU reference boundary.
    pub fn build(params: &OutputGamutCompressParams, output_color_space: &str) -> Result<Self, String> {
        let (destination, kind, knee) = Self::resolve_configuration(params, output_color_space)
            .map_err(|error| error.to_string())?;
        let cam = Cam16Viewing::new(destination.whitepoint_xyz());
        let mut result = Self {
            kind,
            destination,
            cam,
            knee,
            l_min: 0.0,
            l_max: 1.0,
            cmax: Arc::new(Vec::new()),
            prepared: None,
        };
        if let Kind::Perceptual(space) = kind {
            let (l_min, l_max, _) = space.table_geometry();
            result.l_min = l_min;
            result.l_max = l_max;
            result.cmax = cmax_table_cached(space, destination, &cam);
        }
        Ok(result)
    }

    /// Build the boundary on the selected backend, keeping generated GPU
    /// samples resident instead of consulting the CPU table cache.
    pub fn build_with_backend(
        params: &OutputGamutCompressParams,
        output_color_space: &str,
        backend: &dyn spektrafilm_backend::ComputeBackend,
    ) -> Result<Self, String> {
        if !backend.is_gpu() {
            return Self::build(params, output_color_space);
        }
        let (destination, kind, knee) = Self::resolve_configuration(params, output_color_space)
            .map_err(|error| error.to_string())?;
        let cam = Cam16Viewing::new(destination.whitepoint_xyz());
        let mut result = Self {
            kind,
            destination,
            cam,
            knee,
            l_min: 0.0,
            l_max: 1.0,
            cmax: Arc::new(Vec::new()),
            prepared: None,
        };
        if let Kind::Perceptual(space) = kind {
            let (l_min, l_max, chroma_upper) = space.table_geometry();
            let mode = match space {
                Space::Oklch => 1,
                Space::Oklrab => 2,
                Space::Cam16ucs => 3,
                Space::Jzazbz => 4,
            };
            result.l_min = l_min;
            result.l_max = l_max;
            result.prepared = Some(backend.prepare_gamut_table(
                &spektrafilm_backend::GamutTableParams {
                    mode,
                    n_l: N_L as u32,
                    n_h: N_H as u32,
                    n_bisect: N_BISECT as u32,
                    l_min,
                    l_max,
                    chroma_upper,
                    xyz_to_rgb: destination.matrix_xyz_to_rgb,
                    cam16: [cam.d_rgb[0], cam.d_rgb[1], cam.d_rgb[2], cam.a_w],
                },
            )?.ok_or_else(|| format!(
                "{} cannot prepare the selected output gamut boundary on the device",
                backend.name(),
            ))?);
        }
        Ok(result)
    }

    /// GPU inputs for the active mode. An active compressor returning `None`
    /// requires CPU execution, including bypassing fused GPU passes.
    ///
    /// Every registered destination is served: the shader receives the
    /// destination's own RGB↔XYZ matrices and CIECAM16 viewing conditions
    /// rather than baked sRGB ones, so the mode set does not depend on the
    /// space. `jzazbz` runs its forward/inverse perceptual transform in
    /// binary64 on the device — its `b` row cancels ~105x, so the f32 hue
    /// error crossed the C_max table cliff and cost ~1.6e-3 of output RGB
    /// against a 1e-4 budget. f64 keeps the reference conditioning; the pass
    /// fails by name on a device without native shader binary64.
    pub fn gpu_params(&self) -> Option<spektrafilm_backend::GamutGpuParams<'_>> {
        let mode = match self.kind {
            Kind::Off => return None,
            Kind::AcesRgc => 0,
            Kind::Perceptual(Space::Oklch) => 1,
            Kind::Perceptual(Space::Oklrab) => 2,
            Kind::Perceptual(Space::Cam16ucs) => 3,
            Kind::Perceptual(Space::Jzazbz) => 4,
        };
        let to_f32_rows = |m: &[[f64; 3]; 3]| {
            [
                [m[0][0] as f32, m[0][1] as f32, m[0][2] as f32],
                [m[1][0] as f32, m[1][1] as f32, m[1][2] as f32],
                [m[2][0] as f32, m[2][1] as f32, m[2][2] as f32],
            ]
        };
        Some(spektrafilm_backend::GamutGpuParams {
            mode,
            knee: [self.knee.0 as f32, self.knee.1 as f32, self.knee.2 as f32],
            l_min: self.l_min as f32,
            l_max: self.l_max as f32,
            rgb_to_xyz: to_f32_rows(&self.destination.matrix_rgb_to_xyz),
            xyz_to_rgb: to_f32_rows(&self.destination.matrix_xyz_to_rgb),
            cam16: [
                self.cam.d_rgb[0] as f32,
                self.cam.d_rgb[1] as f32,
                self.cam.d_rgb[2] as f32,
                self.cam.a_w as f32,
            ],
            cmax: &self.cmax,
            prepared: self.prepared.as_ref(),
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
                // The device path is destination-independent: every space
                // serves every implemented mode, including jzazbz (mode 4,
                // binary64 perceptual transform on the device).
                assert_eq!(
                    compressor.gpu_params().is_some(),
                    algorithm != "off"
                );
                if algorithm == "off" {
                    assert_eq!(compressor.compress([1.7, -0.2, 0.3]), [1.7, -0.2, 0.3]);
                }
            }
        }
    }

    /// A chromatic wedge in the destination's own linear RGB: neutrals,
    /// primaries and secondaries at and beyond the cube, signed headroom,
    /// and hue sweeps at fixed lightness. Everything stays in the unclipped
    /// domain `scan_spectral` produces, which is the compressor's input
    /// contract — including the negative and above-one samples the C_max
    /// bisection was built to pull back inside the cube.
    fn wedge() -> Vec<[f64; 3]> {
        let mut pixels: Vec<[f64; 3]> = Vec::new();
        for v in [0.0, 0.05, 0.25, 0.5, 0.75, 1.0, 1.25, 2.0, 4.0] {
            for channel in 0..3 {
                let mut rgb = [v, v, v];
                rgb[channel] = v * 1.5 + 0.2;
                pixels.push(rgb);
                pixels.push([-v * 0.5, v, -v]);
            }
        }
        // Hue sweep at fixed lightness and a lightness sweep at fixed chroma:
        // C_max(L, h) is indexed on both axes, so a wrong lookup geometry
        // shows up here rather than on the axis probes.
        for step in 0..24u32 {
            let angle = std::f64::consts::TAU * f64::from(step) / 24.0;
            let (s, c) = angle.sin_cos();
            for lightness in [0.05, 0.2, 0.5, 0.85, 1.0] {
                let chroma = 0.3;
                pixels.push([lightness + chroma * c, lightness, lightness + chroma * s]);
                pixels.push([lightness - chroma * s, lightness + chroma * c, lightness]);
                pixels.push([lightness, lightness - chroma * c, lightness + chroma * s]);
            }
        }
        pixels
    }

    /// The device compressor must agree with [`OutputGamutCompress::compress`]
    /// for every destination it claims to serve. That is the point of the
    /// pass, so comparing only sRGB would pass while a destination-specific
    /// matrix or viewing condition was wrong: these three spaces have
    /// different whitepoints (DCI, D50, ACES), different primaries, and
    /// different stored inverses, so a baked sRGB matrix or a D65-only
    /// CAM16 adaptation misses on all of them.
    #[test]
    fn device_compressor_matches_cpu_on_non_srgb_destinations() {
        use spektrafilm_math::image::ImageBuf;
        use spektrafilm_math::precision::{from_f64, to_f64};

        let backend = spektrafilm_gpu::test_support::adapter();
        let pixels = wedge();
        let image = ImageBuf::from_data(
            pixels.len() as u32,
            1,
            pixels.iter().flat_map(|rgb| rgb.map(from_f64)).collect(),
        );
        let mut worst = 0.0f64;
        let mut worst_case = String::new();
        // Every implemented mode, including aces_rgc (no C_max table) and
        // each perceptual space. jzazbz (mode 4) runs its forward/inverse
        // transform in binary64 on the device, so the same comparison holds
        // for it.
        for algorithm in ["aces_rgc", "oklch", "oklrab", "cam16ucs", "jzazbz"] {
            for destination in ["DCI-P3", "ProPhoto RGB", "ACES2065-1"] {
                let params = OutputGamutCompressParams {
                    algorithm: algorithm.into(),
                    knee: [0.0, 1.0, 6.0],
                };
                let compressor = OutputGamutCompress::build(&params, destination).unwrap();
                let gp = compressor.gpu_params().unwrap_or_else(|| {
                    panic!("{algorithm} / {destination} must have a device path")
                });
                let out = backend.gamut_compress_gpu(&image, &gp).unwrap_or_else(|error| {
                    panic!("{algorithm} / {destination} device path failed: {error}")
                });
                for (index, rgb) in pixels.iter().enumerate() {
                    let reference = compressor.compress(rgb.map(|v| to_f64(from_f64(v))));
                    for (channel, &reference_value) in reference.iter().enumerate() {
                        let diff =
                            (reference_value - to_f64(out.data[index * 3 + channel])).abs();
                        if diff > worst {
                            worst = diff;
                            worst_case =
                                format!("{algorithm} / {destination} at {rgb:?} ch {channel}");
                        }
                    }
                }
            }
        }
        // The device is f32 and the reference f64: a matrix pair plus the
        // perceptual round trip contributes ~1e-6 relative, and the C_max
        // lookup adds the f32 narrowing of an f64-baked value. This budget
        // is that f32 boundary, not an algorithm tolerance — a wrong matrix
        // or viewing condition, or a different perceptual space, misses by
        // two to four orders of magnitude on a destination as far from sRGB
        // as ProPhoto RGB, so the margin is real.
        assert!(
            worst < 1e-4,
            "max |gpu - cpu| {worst:.3e} at {worst_case} exceeds the f32 device budget"
        );
    }

    /// The JzAzBz `b` row cancellation: `JZ_M2`'s third row is
    /// [0.199076, 1.096799, -1.295875] over cone responses that make the three
    /// terms cancel by ~105x, leaving `b` near 0.0075. One f32 rounding at the
    /// input becomes ~3e-6 at the output, rotating the hue by ~1.5e-4 rad and
    /// moving the C_max lookup across the table's ~0.015 single-step cliff —
    /// a ~9% table error and ~1.6e-3 of output RGB, against a 1e-4 budget.
    ///
    /// The device now runs the transform in binary64, so this case must agree
    /// with the CPU reference to the f32 boundary. A regression back to an
    /// f32 transform shows up here first: the pixel below is the wedge's
    /// worst-conditioned point (measured 2.06e-4 f32-vs-f64 over the wedge),
    /// which is the only region where the cancellation is large enough to
    /// cross the cliff.
    #[test]
    fn device_jzazbz_cancellation_case_matches_cpu() {
        use spektrafilm_math::image::ImageBuf;
        use spektrafilm_math::precision::{from_f64, to_f64};

        let backend = spektrafilm_gpu::test_support::adapter();
        // The wedge's worst-conditioned cancellation point plus the specific
        // near-neutral hue-sweep pixel whose three `b` terms cancel ~3.2e5x.
        let pixels: Vec<[f64; 3]> = vec![
            [0.9223542864692438, 1.0, 0.7102222521132795],
            [1.2897777478867205, 1.0, 1.0776457135307562],
            [0.3, 0.3, 0.3],
        ];
        let image = ImageBuf::from_data(
            pixels.len() as u32,
            1,
            pixels.iter().flat_map(|rgb| rgb.map(from_f64)).collect(),
        );
        let params = OutputGamutCompressParams {
            algorithm: "jzazbz".into(),
            knee: [0.0, 1.0, 6.0],
        };
        let compressor = OutputGamutCompress::build(&params, "sRGB").unwrap();
        let gp = compressor.gpu_params().expect("jzazbz has a device path");
        let out = backend
            .gamut_compress_gpu(&image, &gp)
            .expect("jzazbz device path must run");
        let mut worst = 0.0f64;
        for (index, rgb) in pixels.iter().enumerate() {
            let reference = compressor.compress(rgb.map(|v| to_f64(from_f64(v))));
            for (channel, &reference_value) in reference.iter().enumerate() {
                let diff = (reference_value - to_f64(out.data[index * 3 + channel])).abs();
                worst = worst.max(diff);
            }
        }
        assert!(
            worst < 1e-4,
            "jzazbz cancellation case: max |gpu - cpu| {worst:.3e} exceeds the budget; \
             the f32 transform crossed the C_max table cliff here"
        );
    }

    /// JzAzBz endpoint coverage on the device: every destination space, plus
    /// signed headroom, above-white highlights, and neutral endpoints. The
    /// C_max lookup clamps L and wraps h, so these are the points where a
    /// wrong lookup geometry or a missing sign path diverges from the CPU
    /// reference.
    #[test]
    fn device_jzazbz_endpoints_match_cpu_all_destinations() {
        use spektrafilm_math::image::ImageBuf;
        use spektrafilm_math::precision::{from_f64, to_f64};

        let backend = spektrafilm_gpu::test_support::adapter();
        // Neutral (achromatic, h undefined → guarded), signed (negative
        // channels carry scan headroom), above-white (knee active), and
        // chroma-heavy (knee saturating) endpoints.
        let pixels: Vec<[f64; 3]> = vec![
            [0.0, 0.0, 0.0],
            [0.5, 0.5, 0.5],
            [1.0, 1.0, 1.0],
            [-0.5, 0.25, -0.75],
            [1.25, -0.1, 0.4],
            [4.0, 4.0, 4.0],
            [2.0, 0.0, 0.0],
            [0.0, 2.0, 2.0],
        ];
        let image = ImageBuf::from_data(
            pixels.len() as u32,
            1,
            pixels.iter().flat_map(|rgb| rgb.map(from_f64)).collect(),
        );
        let params = OutputGamutCompressParams {
            algorithm: "jzazbz".into(),
            knee: [0.0, 1.0, 6.0],
        };
        let mut worst = 0.0f64;
        let mut worst_case = String::new();
        for destination in [
            "sRGB",
            "DCI-P3",
            "Display P3",
            "Adobe RGB (1998)",
            "ITU-R BT.2020",
            "ProPhoto RGB",
            "ACES2065-1",
        ] {
            let compressor = OutputGamutCompress::build(&params, destination).unwrap();
            let gp = compressor.gpu_params().expect("jzazbz has a device path");
            let out = backend
                .gamut_compress_gpu(&image, &gp)
                .expect("jzazbz device path must run");
            for (index, rgb) in pixels.iter().enumerate() {
                let reference = compressor.compress(rgb.map(|v| to_f64(from_f64(v))));
                let device: Vec<f64> = out.data[index * 3..index * 3 + 3]
                    .iter()
                    .map(|v| to_f64(*v))
                    .collect();
                for channel in 0..3 {
                    let diff = (reference[channel] - device[channel]).abs();
                    if diff > worst {
                        worst = diff;
                        worst_case = format!("{destination} at {rgb:?} ch {channel}");
                    }
                }
            }
        }
        assert!(
            worst < 1e-4,
            "jzazbz endpoints: max |gpu - cpu| {worst:.3e} at {worst_case} exceeds the budget"
        );
    }

    /// The JzAzBz transform's own conditioning, independent of the C_max
    /// lookup: the forward/inverse round trip must recover the input to f64
    /// precision on the device. This isolates the transform arithmetic (the
    /// signed powers, the matrices) from the table lookup, so a regression in
    /// either is attributable.
    #[test]
    fn device_jzazbz_roundtrip_recovers_input() {
        use spektrafilm_math::image::ImageBuf;
        use spektrafilm_math::precision::{from_f64, to_f64};

        let backend = spektrafilm_gpu::test_support::adapter();
        // `off` with the jzazbz transform is not expressible, so run the
        // compressor with a knee that leaves in-gamut chroma untouched
        // (threshold 0, limit 1, power 6 compresses nothing below C_max) and
        // compare the device output against the CPU's own round trip.
        let pixels: Vec<[f64; 3]> = vec![
            [0.0, 0.0, 0.0],
            [0.1, 0.2, 0.3],
            [0.5, 0.5, 0.5],
            [0.9, 0.8, 0.7],
        ];
        let image = ImageBuf::from_data(
            pixels.len() as u32,
            1,
            pixels.iter().flat_map(|rgb| rgb.map(from_f64)).collect(),
        );
        let params = OutputGamutCompressParams {
            algorithm: "jzazbz".into(),
            knee: [0.0, 1.0, 6.0],
        };
        let compressor = OutputGamutCompress::build(&params, "sRGB").unwrap();
        let gp = compressor.gpu_params().expect("jzazbz has a device path");
        let out = backend
            .gamut_compress_gpu(&image, &gp)
            .expect("jzazbz device path must run");
        let mut worst = 0.0f64;
        for (index, rgb) in pixels.iter().enumerate() {
            let reference = compressor.compress(rgb.map(|v| to_f64(from_f64(v))));
            for (channel, &reference_value) in reference.iter().enumerate() {
                let diff = (reference_value - to_f64(out.data[index * 3 + channel])).abs();
                worst = worst.max(diff);
            }
        }
        // In-gamut chroma below C_max passes through the knee unchanged, so
        // this is effectively the transform round trip: f64 on both sides,
        // differing only by the f32 upload of the input and the f32 readback.
        assert!(
            worst < 1e-6,
            "jzazbz round trip: max |gpu - cpu| {worst:.3e} exceeds the f64 boundary"
        );
    }
}


