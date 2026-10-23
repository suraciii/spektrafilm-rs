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
        Self {
            log_exposure: &film.data.log_exposure,
            raw,
            normalized,
        }
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
                )
                .map_err(|error| format!("invalid print density-curve model: {error}"))?,
                1.0,
            ),
            None => (
                print.density_curves_f64(),
                params.print_render.density_curve_gamma as f64,
            ),
        };
        Ok(Self {
            log_exposure,
            density,
            gamma,
        })
    }
}

pub(crate) fn channel_density(profile: &Profile) -> Vec<[f64; 3]> {
    profile
        .data
        .channel_density
        .iter()
        .map(|row| {
            [
                row.first().copied().unwrap_or(0.0),
                row.get(1).copied().unwrap_or(0.0),
                row.get(2).copied().unwrap_or(0.0),
            ]
        })
        .collect()
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

/// Colour context of one scan pass: the scanner lamp, the clear-film white
/// calibration derived from it, the chromatic adaptation to the output white,
/// and the output XYZ→RGB matrix.
///
/// Neutral scan model v2: the white reference is the clear film (base)
/// reading under the lamp, not the lamp itself. Dividing the capture by the
/// base reading keeps the lamp's spectral shape in play — exactly how a real
/// scanner's white calibration behaves — and adapting from the base white to
/// the output white removes the film-base tint.
#[derive(Debug, Clone)]
pub struct ScanColorContext {
    /// Effective readout illuminant, f64 per wavelength: the product constant
    /// selected by the material category, without a device filter.
    /// research `k64-neutral-scan-v2`) folded in. Every kernel multiplies
    /// this spectrum, so the cut needs no separate plumbing.
    pub illuminant: Vec<f64>,
    /// Wavelength count shared by the lamp, the profile's dye and base
    /// densities, and the CMF tables.
    pub n_wl: usize,
    /// Clear-film XYZ reading under the lamp (`∫ 10^-D_base·L·CMF`) — the
    /// scanner's white calibration reference.
    pub base_xyz: [f64; 3],
    /// `base_xyz[1]` — the scalar every scan kernel divides the capture by.
    pub normalization: f64,
    /// Lamp XYZ integrated against the CMFs (with the IR cut applied); the
    /// glare offset colour source.
    pub lamp_xyz: [f64; 3],
    /// Clear-film white (Y = 1) the chromatic adaptation starts from.
    pub base_white: [f64; 3],
    /// Chromatic adaptation from the clear-film white to the output white.
    pub adapt: [[f64; 3]; 3],
    /// Output colour space XYZ→RGB matrix.
    pub base_xyz_to_rgb: [[f64; 3]; 3],
}

impl ScanColorContext {
    /// Build the context for one scan pass. `lamp` is the resolved scanner
    /// lamp; `channel_density` and `base_density` are the scanned medium's
    /// spectral data (pass zeros for the base density of a base-excluded
    /// convert-film pass, which degenerates the white reference to the lamp
    /// itself).
    pub fn build(
        lamp: Vec<f64>,
        channel_density: &[[f64; 3]],
        base_density: &[f64],
        output_color_space: &str,
    ) -> Self {
        let n_wl = lamp
            .len()
            .min(base_density.len())
            .min(channel_density.len())
            .min(N_WAVELENGTHS);

        let mut base_xyz = [0.0f64; 3];
        let mut lamp_xyz = [0.0f64; 3];
        // The wavelength index aligns lamp, dye, base density, and
        // color-matching samples. A row the kernels would mask — a
        // non-finite lamp, dye, or base sample (NaN·0 is NaN, then masked
        // to zero) — is excluded from the white reference too, so the
        // clear-film reading the normalization divides by is exactly the
        // kernel's.
        #[allow(clippy::needless_range_loop)]
        for i in 0..n_wl {
            let dye = channel_density[i];
            if !lamp[i].is_finite()
                || !base_density[i].is_finite()
                || !dye[0].is_finite()
                || !dye[1].is_finite()
                || !dye[2].is_finite()
            {
                continue;
            }
            let lamp = lamp[i];
            let base_transmission = 10.0f64.powf(-base_density[i]);
            base_xyz[0] += base_transmission * lamp * spectral::CMF_X_F64[i];
            base_xyz[1] += base_transmission * lamp * spectral::CMF_Y_F64[i];
            base_xyz[2] += base_transmission * lamp * spectral::CMF_Z_F64[i];
            lamp_xyz[0] += lamp * spectral::CMF_X_F64[i];
            lamp_xyz[1] += lamp * spectral::CMF_Y_F64[i];
            lamp_xyz[2] += lamp * spectral::CMF_Z_F64[i];
        }
        let normalization = base_xyz[1];
        assert!(
            normalization.is_finite() && normalization > 0.0,
            "clear-film scan reference must be positive"
        );
        let base_white = [
            base_xyz[0] / normalization,
            1.0f64,
            base_xyz[2] / normalization,
        ];

        let output_space = colorspace::resolve(output_color_space)
            .expect("output color space must be validated before scanning");
        let adapt =
            colorspace::chromatic_adaptation_matrix_f64(base_white, output_space.whitepoint_xyz());

        Self {
            illuminant: lamp,
            n_wl,
            base_xyz,
            normalization,
            lamp_xyz,
            base_white,
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
        include_base: bool,
    ) -> Self {
        let lamp = crate::spectral_service::select_illuminant_f64(profile.readout_illuminant());
        let base_density = if include_base {
            std::borrow::Cow::Borrowed(profile.data.base_density.as_slice())
        } else {
            std::borrow::Cow::Owned(vec![0.0; profile.data.base_density.len()])
        };
        let scan = ScanColorContext::build(
            lamp.into_owned(),
            &channel_density(profile),
            &base_density,
            &params.io.output_color_space,
        );
        let mut scan_xyz_to_rgb = [[0.0f64; 3]; 3];
        // Matrix multiplication indexes both operands by row and column.
        #[allow(clippy::needless_range_loop)]
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

/// Per-channel RGB glare offset `M · CAT · (lamp_xyz / normalization)`,
/// applied as `rgb += g * offset / 100`. The two matrix applications follow
/// the per-stage scan order — CAT first, then the output XYZ→RGB matrix —
/// and the division places the stray lamp light in the same clear-film
/// normalized sensor domain as the capture.
pub fn glare_rgb_offset_f64(ctx: &ScanColorContext) -> [f64; 3] {
    let a = ctx.adapt;
    let m = ctx.base_xyz_to_rgb;
    let xyz = [
        ctx.lamp_xyz[0] / ctx.normalization,
        ctx.lamp_xyz[1] / ctx.normalization,
        ctx.lamp_xyz[2] / ctx.normalization,
    ];
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
