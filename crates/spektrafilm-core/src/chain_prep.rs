//! Backend-neutral preparation shared by the per-stage CPU path and the
//! GPU-resident chain builder.
//!
//! Curve, spectral, DIR-control and scan-colour derivations stay f64 here.
//! Execution routes retain their own operation order and narrow only at
//! the GPU boundary.

use spektrafilm_math::colorspace;
use spektrafilm_math::spectral::{self, N_WAVELENGTHS};

use crate::params::RuntimeParams;
use crate::profile::Profile;

pub(crate) struct FilmCurves<'a> {
    pub log_exposure: &'a [f64],
    pub raw: Vec<[f64; 3]>,
    pub normalized: Vec<[f64; 3]>,
}

impl<'a> FilmCurves<'a> {
    pub fn prepare(film: &'a Profile) -> Self {
        let raw = film.density_curves_f64();
        let normalized = spektrafilm_model::density_curves::normalize_density_curves_f64(&raw);
        Self { log_exposure: &film.data.log_exposure, raw, normalized }
    }
}

pub(crate) struct PrintCurves<'a> {
    pub log_exposure: &'a [f64],
    pub density: Vec<[f64; 3]>,
    pub gamma: f64,
}

impl<'a> PrintCurves<'a> {
    pub fn prepare(print: &'a Profile, params: &RuntimeParams) -> Result<Self, String> {
        let log_exposure = print.data.log_exposure.as_slice();
        // Fitted models own their gamma; model-less profiles use raw stored curves.
        let (density, gamma) = match print.data.density_curves_model.as_ref() {
            Some(model) => (
                crate::print_morph::morph_density_curves(
                    log_exposure,
                    model,
                    &params.print_render.density_curves_morph,
                    print.is_positive(),
                ).map_err(|error| format!("invalid print density-curve model: {error}"))?,
                1.0,
            ),
            None => (print.density_curves_f64(), params.print_render.density_curve_gamma as f64),
        };
        Ok(Self { log_exposure, density, gamma })
    }
}

pub(crate) fn channel_density(profile: &Profile) -> Vec<[f64; 3]> {
    profile.data.channel_density.iter().map(|row| [
        row.first().copied().unwrap_or(0.0),
        row.get(1).copied().unwrap_or(0.0),
        row.get(2).copied().unwrap_or(0.0),
    ]).collect()
}

pub(crate) fn print_sensitivity(print: &Profile) -> Vec<[f64; 3]> {
    print.data.log_sensitivity.iter().map(|row| {
        let mut sensitivity = [0.0; 3];
        for c in 0..3 {
            let value = 10.0f64.powf(row.get(c).copied().unwrap_or(0.0));
            sensitivity[c] = if value.is_nan() { 0.0 } else { value };
        }
        sensitivity
    }).collect()
}

pub(crate) fn dir_matrix(dir: &crate::params::couplers::DirCouplersParams) -> [[f64; 3]; 3] {
    spektrafilm_model::couplers::compute_dir_couplers_matrix(
        dir.gamma_samelayer_rgb,
        dir.gamma_interlayer_r_to_gb,
        dir.gamma_interlayer_g_to_rb,
        dir.gamma_interlayer_b_to_rg,
        dir.inhibition_samelayer,
        dir.inhibition_interlayer,
    )
}

/// Colour context of one scan pass: viewing illuminant, its normalization,
/// the derived chromatic-adaptation matrix, and the output XYZ→RGB matrix.
#[derive(Debug, Clone)]
pub struct ScanColorContext {
    /// Resolved viewing (or scan-override) illuminant, f64 per wavelength.
    pub illuminant: Vec<f64>,
    /// Wavelength count shared by the illuminant, the profile's dye
    /// densities, and the historical 81-sample grid.
    pub n_wl: usize,
    /// `Σ illu·CMF_Y` — Python parity (81-element sum in f64).
    pub normalization: f64,
    /// Illuminant XYZ integrated against the CMFs and divided by
    /// `normalization` (Y = 1).
    pub illu_xyz: [f64; 3],
    /// Viewing white derived through the XYZ→xy→XYZ roundtrip that
    /// renormalizes Y to exactly 1 (matches Python's contract chain).
    pub viewing_white: [f64; 3],
    /// Chromatic adaptation from the viewing white to the output white.
    pub adapt: [[f64; 3]; 3],
    /// Output colour space XYZ→RGB matrix.
    pub base_xyz_to_rgb: [[f64; 3]; 3],
}

impl ScanColorContext {
    /// Build the context for one scan pass. `illuminant` is already resolved
    /// (the per-stage path may pass a scan override; the resident chain
    /// passes the profile's viewing illuminant). `channel_density_len` is
    /// the scanned profile's per-wavelength dye-density row count.
    pub fn build(
        illuminant: Vec<f64>,
        channel_density_len: usize,
        output_color_space: &str,
    ) -> Self {
        let n_wl = illuminant.len().min(channel_density_len).min(N_WAVELENGTHS);

        // Python parity — the normalization sums exactly n_wl terms in f64.
        let normalization: f64 = (0..n_wl)
            .map(|i| illuminant[i] * spectral::CMF_Y_F64[i])
            .sum();

        // Illuminant XYZ (Y normalized to 1) — same runtime derivation as
        // Python's integrate/contract chain, including the division by
        // `normalization` before the xy roundtrip below.
        let mut illu_xyz = [0.0f64; 3];
        for i in 0..n_wl {
            illu_xyz[0] += illuminant[i] * spectral::CMF_X_F64[i];
            illu_xyz[1] += illuminant[i] * spectral::CMF_Y_F64[i];
            illu_xyz[2] += illuminant[i] * spectral::CMF_Z_F64[i];
        }
        for c in 0..3 {
            illu_xyz[c] /= normalization;
        }

        // XYZ_to_xy then xy_to_xyz roundtrip (renormalizes Y to exactly 1).
        let sum_xyz = illu_xyz[0] + illu_xyz[1] + illu_xyz[2];
        let vx = illu_xyz[0] / sum_xyz;
        let vy = illu_xyz[1] / sum_xyz;
        let viewing_white = [vx / vy, 1.0f64, (1.0 - vx - vy) / vy];

        let output_space = colorspace::resolve(output_color_space)
            .expect("output color space must be validated before scanning");
        let adapt = colorspace::chromatic_adaptation_matrix_f64(
            viewing_white,
            output_space.whitepoint_xyz(),
        );

        Self {
            illuminant,
            n_wl,
            normalization,
            illu_xyz,
            viewing_white,
            adapt,
            base_xyz_to_rgb: output_space.matrix_xyz_to_rgb,
        }
    }
}

/// Backend-neutral scan preparation shared by the per-stage and resident paths.
///
/// All spectral values stay f64 here. GPU callers narrow only when constructing
/// their backend parameter structs.
#[derive(Debug, Clone)]
pub struct PreparedChain {
    pub scan: ScanColorContext,
    pub scan_xyz_to_rgb: [[f64; 3]; 3],
}

impl PreparedChain {
    pub fn for_scan(
        profile: &crate::profile::Profile,
        params: &crate::params::RuntimeParams,
        scan_illuminant: Option<&str>,
    ) -> Self {
        let illuminant = scan_illuminant
            .map(crate::spectral_service::select_illuminant_f64)
            .unwrap_or_else(|| {
                crate::spectral_service::select_illuminant_f64(&profile.info.viewing_illuminant)
            });
        let channel_density_len = profile.data.channel_density.len();
        let scan = ScanColorContext::build(
            illuminant.into_owned(),
            channel_density_len,
            &params.io.output_color_space,
        );
        let mut scan_xyz_to_rgb = [[0.0f64; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                scan_xyz_to_rgb[i][j] = scan.base_xyz_to_rgb[i][0] * scan.adapt[0][j]
                    + scan.base_xyz_to_rgb[i][1] * scan.adapt[1][j]
                    + scan.base_xyz_to_rgb[i][2] * scan.adapt[2][j];
            }
        }
        Self {
            scan,
            scan_xyz_to_rgb,
        }
    }
}

/// Per-channel RGB glare offset `M · illuminant_xyz`, applied as
/// `rgb += g * offset / 100`. The two matrix applications follow the
/// per-stage scan order — CAT first, then the output XYZ→RGB matrix —
/// keeping the reference rounding sequence authoritative.
pub fn glare_rgb_offset_f64(ctx: &ScanColorContext) -> [f64; 3] {
    let a = ctx.adapt;
    let m = ctx.base_xyz_to_rgb;
    let xyz = ctx.illu_xyz;
    let xyz_adapt = [
        a[0][0] * xyz[0] + a[0][1] * xyz[1] + a[0][2] * xyz[2],
        a[1][0] * xyz[0] + a[1][1] * xyz[1] + a[1][2] * xyz[2],
        a[2][0] * xyz[0] + a[2][1] * xyz[1] + a[2][2] * xyz[2],
    ];
    [
        m[0][0] * xyz_adapt[0] + m[0][1] * xyz_adapt[1] + m[0][2] * xyz_adapt[2],
        m[1][0] * xyz_adapt[0] + m[1][1] * xyz_adapt[1] + m[1][2] * xyz_adapt[2],
        m[2][0] * xyz_adapt[0] + m[2][1] * xyz_adapt[1] + m[2][2] * xyz_adapt[2],
    ]
}
