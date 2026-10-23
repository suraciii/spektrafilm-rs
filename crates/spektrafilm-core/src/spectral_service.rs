//! Descriptor-driven spectral upsampling and exact illuminant sampling.
//!
//! Illuminant data, LUT loading/cache, camera filters and spectral contraction
//! live in private submodules; this module preserves the public service API.

mod filters;
mod illuminant;
mod registry;
mod upsampling;

use crate::params::RuntimeParams;
use crate::profile::Profile;
use spektrafilm_math::spectral::{self, TcLut};
use std::path::Path;

pub use filters::{
    NO_COLOR_FILTER, available_color_filters, is_supported_color_filter,
    load_color_filter_transmittance,
};
pub use illuminant::{
    ILLUMINANT_K75P, ILLUMINANT_K75P_F64, ILLUMINANT_T, ILLUMINANT_T_F64, ILLUMINANT_TH_KG3,
    ILLUMINANT_TH_KG3_F64, ILLUMINANT_TH_KG3_L, ILLUMINANT_TH_KG3_L_F64, available_illuminants,
    is_supported_illuminant, select_illuminant, select_illuminant_f64,
    select_illuminant_on_grid_f64,
};
pub use registry::{
    LutDescriptor, SpectraCube, SpectraLut, available_lut_identifiers, available_spectral_methods,
    load_arctic2026alpha02_lut, load_lut, load_lut_on_grid, load_spectra_lut, lut_descriptor,
    parse_lut_descriptor,
};
pub use upsampling::{
    Hanatos2025Adaptation, apply_camera_uv_ir_band_pass, apply_log_exposure_surface,
    compute_band_pass_filter, compute_hanatos2025_tc_lut, compute_midgray_normalization,
    compute_reflectance_tc_lut, compute_registered_tc_lut, compute_tc_lut,
    eval_poly4_log_exposure_surface, gaussian_blur_wavelength, pairwise_sum_f64_pub,
};

#[cfg(test)]
use spektrafilm_math::spectral::N_WAVELENGTHS;

pub const fn default_spectral_shape() -> [f64; 3] {
    [380.0, 780.0, 5.0]
}

/// A regular wavelength grid. Samples include both endpoints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectralShape {
    pub bounds: [f64; 3],
    pub samples: usize,
}

impl SpectralShape {
    pub fn new(bounds: [f64; 3]) -> Result<Self, String> {
        Self::new_detailed(bounds).map_err(|error| error.to_string())
    }

    pub(crate) fn new_detailed(bounds: [f64; 3]) -> Result<Self, crate::params::ValidationError> {
        use crate::params::{ValidationError as Error, ValidationReason as Reason};
        let [start, end, step] = bounds;
        if !bounds.iter().all(|v| v.is_finite()) || start <= 0.0 || end <= start || step <= 0.0 {
            return Err(Error::new(
                Reason::SpectralBounds,
                "",
                "expected finite positive wavelengths, end > start and step > 0",
            )
            .arg("value", format!("{bounds:?}")));
        }
        let intervals = (end - start) / step;
        if intervals > 100_000.0 || (intervals - intervals.round()).abs() > 1e-8 {
            return Err(Error::new(
                Reason::SpectralIntervals,
                "",
                "grid must contain an integral number of intervals and at most 100001 samples",
            )
            .arg("value", format!("{bounds:?}")));
        }
        Ok(Self {
            bounds,
            samples: intervals.round() as usize + 1,
        })
    }

    pub fn wavelength(self, index: usize) -> f64 {
        self.bounds[0] + self.bounds[2] * index as f64
    }
}

/// The film front end of a resolved chain: the (optionally filtered) film
/// sensitivity, the RGB → film-raw upsampler built for it, and the illuminant
/// that upsampler projects through.
pub struct FilmingUpsampler {
    /// The resolved `settings.rgb_to_raw_method` identifier.
    pub method: String,
    /// The film sensitivity after the camera UV/IR and colour filters.
    pub sensitivity: Vec<[f64; 3]>,
    /// Hanatos2025 spectral TC LUT; `Some` whenever the method uses a LUT.
    pub tc_lut: Option<TcLut>,
    /// Mallett2019 reflectance-basis matrix; `Some` for `mallett2019`.
    pub mallett_core: Option<[[f64; 3]; 3]>,
    /// `camera.capture_illuminant` when set, otherwise the film profile's
    /// reference illuminant. It is not the sensitivity-calibration
    /// illuminant, which always stays the film reference.
    pub front_illuminant: String,
}

/// Build the filming front end for a resolved chain.
///
/// The film sensitivity is expanded and filtered (`sensitivity_f64` +
/// camera UV/IR and colour filters), then the `settings.rgb_to_raw_method`
/// upsampler is built: a windowed Hanatos2025 TC LUT, another registered
/// LUT, or the Mallett2019 matrix. The pipeline calibration and the lab
/// spectral evaluation share this one builder so both see the same chain.
///
/// `front_illuminant` reports the effective filming illuminant: an explicit
/// `camera.capture_illuminant` when set, otherwise the film profile's
/// `reference_illuminant`. Sensitivity calibration — UV/IR filter
/// normalization, the Hanatos2025 adaptation and the TC LUT — continues to
/// use the film reference regardless of the selected capture light.
pub fn build_filming_upsampler(
    film: &Profile,
    params: &RuntimeParams,
    data_dir: &Path,
) -> Result<FilmingUpsampler, String> {
    // Film sensitivity: Python `sensitivity = np.nan_to_num(10 ** log_sensitivity)`.
    let mut sensitivity: Vec<[f64; 3]> = film.sensitivity_f64();

    let ref_illuminant = select_illuminant(&film.info.reference_illuminant);
    let ref_illuminant_f64 = select_illuminant_f64(&film.info.reference_illuminant);
    // The explicit value wins; omission inherits the selected profile's
    // reference illuminant. The reference remains the calibration illuminant
    // below, independently of this front projection.
    let front_illuminant = params
        .camera
        .capture_illuminant
        .clone()
        .unwrap_or_else(|| film.info.reference_illuminant.clone());

    // Camera UV/IR filter — Python `FilmingStage._rgb_to_film_raw` filters
    // the film sensitivity through the UV/IR band-pass (with
    // reference-illuminant normalization preserving white balance) before
    // BOTH upsampler branches and before the midgray calibration below.
    // Off at default amplitudes (0, 0), matching Python's guard
    // `filter_uv[0] > 0 or filter_ir[0] > 0`.
    if params.camera.filter_uv[0] > 0.0 || params.camera.filter_ir[0] > 0.0 {
        apply_camera_uv_ir_band_pass(
            &mut sensitivity,
            [
                params.camera.filter_uv[0] as f64,
                params.camera.filter_uv[1] as f64,
                params.camera.filter_uv[2] as f64,
            ],
            [
                params.camera.filter_ir[0] as f64,
                params.camera.filter_ir[1] as f64,
                params.camera.filter_ir[2] as f64,
            ],
            &ref_illuminant_f64,
        );
    }
    if params.camera.color_filter != NO_COLOR_FILTER {
        let transmittance = load_color_filter_transmittance(data_dir, &params.camera.color_filter)?
            .expect("named camera color filters always have transmission");
        for (row, transmission) in sensitivity.iter_mut().zip(transmittance) {
            for channel in row {
                *channel *= transmission;
            }
        }
    }
    // RGB → film raw upsampler. Default `hanatos2025` builds the spectral tc
    // LUT; `mallett2019` builds a 3×3 reflectance-basis matrix instead (no
    // LUT). Dispatch mirrors Python `_rgb_to_film_raw`.
    let method_name = params.settings.rgb_to_raw_method.as_str();
    let method = crate::params::validation::RgbToRawMethod::parse(method_name)?;
    let (tc_lut, mallett_core): (Option<TcLut>, Option<[[f64; 3]; 3]>) = match method {
        crate::params::validation::RgbToRawMethod::Hanatos2025 => {
            let window_params: Vec<f64> = film.data.hanatos2025_adaptation_window_params.clone();
            // A filming-stage material without a fitted window runs the
            // RGB-to-exposure conversion without the sensitivity-adaptation
            // window: white balance is preserved but no ultraviolet or
            // infrared cutoff is modeled. The downgrade is reported, never
            // silent (specs/film-chain/spec.md).
            let apply_window =
                params.settings.apply_hanatos2025_adaptation_window && window_params.len() == 4;
            let apply_surface = params.settings.apply_hanatos2025_adaptation_surface
                && film
                    .data
                    .hanatos2025_adaptation_surface_params
                    .iter()
                    .all(|row| row.len() == 15)
                && film.data.hanatos2025_adaptation_surface_params.len() == 3;
            if params.settings.apply_hanatos2025_adaptation_window && !apply_window {
                tracing::warn!(
                    film = %film.info.stock.as_deref().unwrap_or("<unnamed>"),
                    "hanatos2025 adaptation window requested but the profile carries no fitted \
                     window parameters; running without the window, which models no ultraviolet \
                     or infrared cutoff"
                );
            }
            if params.settings.apply_hanatos2025_adaptation_surface && !apply_surface {
                tracing::warn!(
                    film = %film.info.stock.as_deref().unwrap_or("<unnamed>"),
                    "hanatos2025 adaptation surface requested but the profile carries no fitted \
                     surface parameters; running without the surface"
                );
            }
            let adaptation = Hanatos2025Adaptation {
                window_params: &window_params,
                surface_params: &film.data.hanatos2025_adaptation_surface_params,
                spectral_gaussian_blur: params.settings.spectral_gaussian_blur as f64,
                reference_illuminant: &ref_illuminant_f64,
                reference_illuminant_xy: spectral::illuminant_to_xy(&ref_illuminant),
                apply_window,
                apply_surface,
            };
            let tc_lut = compute_registered_tc_lut(
                data_dir,
                "hanatos2025",
                &sensitivity,
                &ref_illuminant_f64,
                Some(&adaptation),
            )?;

            (Some(tc_lut), None)
        }
        crate::params::validation::RgbToRawMethod::Arctic2026Alpha02
        | crate::params::validation::RgbToRawMethod::Arctic2026Beta04
        | crate::params::validation::RgbToRawMethod::GaussLasers
        | crate::params::validation::RgbToRawMethod::Jakob2019
        | crate::params::validation::RgbToRawMethod::Otsu2018 => {
            // Registered reflectance LUTs were recovered under D65; their
            // chromaticity projection must remain D65 when capture is unset.
            // An explicit capture illuminant is a deliberate scene-light
            // override and is applied to the registered front pass.
            let projection_illuminant = if params.camera.capture_illuminant.is_none() {
                "D65"
            } else {
                &front_illuminant
            };
            let tc_lut = compute_registered_tc_lut(
                data_dir,
                method_name,
                &sensitivity,
                &select_illuminant_f64(projection_illuminant),
                None,
            )?;
            (Some(tc_lut), None)
        }
        crate::params::validation::RgbToRawMethod::Mallett2019 => {
            let illuminant = if params.camera.capture_illuminant.is_some() {
                select_illuminant_f64(&front_illuminant)
            } else {
                ref_illuminant_f64.clone()
            };
            (
                None,
                Some(crate::mallett::compute_core_matrix(
                    &sensitivity,
                    &illuminant,
                )),
            )
        }
    };
    let tc_lut = if let Some(tc_lut) = tc_lut {
        let input_gamut =
            crate::input_gamut::InputGamutCompress::build(&params.io.input_gamut_compress)?;
        Some(if input_gamut.is_active() {
            // Hanatos remaps around fixed D65, independently of film adaptation.
            // Other LUT methods set their front illuminant before this boundary.
            let remap_illuminant = if matches!(
                method,
                crate::params::validation::RgbToRawMethod::Hanatos2025
            ) {
                "D65"
            } else {
                &front_illuminant
            };
            let (rx, ry) = spectral::illuminant_to_xy(&select_illuminant(remap_illuminant));
            input_gamut.remap(&tc_lut, [rx, ry])
        } else {
            tc_lut
        })
    } else {
        None
    };

    Ok(FilmingUpsampler {
        method: method_name.to_owned(),
        sensitivity,
        tc_lut,
        mallett_core,
        front_illuminant,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::data_dir;

    fn portra_sensitivity() -> Vec<[f64; 3]> {
        let film = crate::profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
        film.log_sensitivity_f64()
            .iter()
            .map(|row| {
                let mut out = [0.0f64; 3];
                for c in 0..3 {
                    let v = 10.0f64.powf(row[c]);
                    out[c] = if v.is_nan() { 0.0 } else { v };
                }
                out
            })
            .collect()
    }

    fn assert_close(got: f64, want: f64, ctx: &str) {
        let tol = 1e-12_f64.max(want.abs() * 1e-12);
        assert!(
            (got - want).abs() <= tol,
            "{ctx}: got {got:.17}, want {want:.17} (diff {:.3e})",
            (got - want).abs()
        );
    }

    /// The front-end light follows the profile-relative capture contract:
    /// omission inherits the film reference and an explicit value overrides
    /// it for every registered method.
    #[test]
    fn filming_upsampler_front_illuminant_follows_capture_illuminant() {
        let data = data_dir();
        let film = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let reference = film.info.reference_illuminant.clone();
        assert_eq!(reference, "D55");

        for method in [
            "hanatos2025",
            "jakob2019",
            "mallett2019",
            "arctic2026alpha02",
        ] {
            let base = |capture: Option<String>| {
                let mut params = RuntimeParams::default();
                params.settings.rgb_to_raw_method = method.into();
                params.camera.capture_illuminant = capture;
                params
            };

            let fallback = build_filming_upsampler(&film, &base(None), &data).unwrap();
            assert_eq!(
                fallback.front_illuminant, reference,
                "{method}: profile fallback"
            );

            let explicit =
                build_filming_upsampler(&film, &base(Some("D75".into())), &data).unwrap();
            assert_eq!(
                explicit.front_illuminant, "D75",
                "{method}: explicit override"
            );

            let blackbody =
                build_filming_upsampler(&film, &base(Some("BB4000".into())), &data).unwrap();
            assert_eq!(
                blackbody.front_illuminant, "BB4000",
                "{method}: blackbody override"
            );
        }
    }

    /// Sensitivity calibration stays on the film reference illuminant even
    /// when the front pass projects through an explicit capture illuminant.
    #[test]
    fn capture_illuminant_leaves_sensitivity_calibration_on_the_film_reference() {
        let data = data_dir();
        let film = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let reference = film.info.reference_illuminant.clone();
        let sensitivity = portra_sensitivity();

        let build = |capture: Option<String>| {
            let mut params = RuntimeParams::default();
            params.settings.rgb_to_raw_method = "hanatos2025".into();
            params.camera.capture_illuminant = capture;
            build_filming_upsampler(&film, &params, &data).unwrap()
        };

        let fallback = build(None);
        let explicit = build(Some("BB4000".into()));
        assert_eq!(fallback.front_illuminant, reference);
        assert_eq!(explicit.front_illuminant, "BB4000");
        // The UV/IR band-pass normalization and the Hanatos2025 adaptation
        // both read the film reference, so the filtered sensitivity and the
        // LUT are identical to the historical build.
        assert_eq!(fallback.sensitivity, sensitivity);
        assert_eq!(explicit.sensitivity, sensitivity);
        let reference_lut = fallback.tc_lut.expect("hanatos2025 builds a LUT");
        let explicit_lut = explicit.tc_lut.expect("hanatos2025 builds a LUT");
        assert_eq!(reference_lut.data, explicit_lut.data);
    }

    #[test]
    fn explicit_capture_illuminant_changes_registered_projection_and_mallett_matrix() {
        let data = data_dir();
        let film = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();

        let build = |method: &str, capture: Option<&str>| {
            let mut params = RuntimeParams::default();
            params.settings.rgb_to_raw_method = method.into();
            params.camera.capture_illuminant = capture.map(str::to_owned);
            build_filming_upsampler(&film, &params, &data).unwrap()
        };

        let registered_unset = build("jakob2019", None).tc_lut.expect("registered LUT");
        let registered_explicit = build("jakob2019", Some("BB4000"))
            .tc_lut
            .expect("registered LUT");
        assert_ne!(registered_unset.data, registered_explicit.data);

        let mallett_unset = build("mallett2019", None)
            .mallett_core
            .expect("Mallett matrix");
        let mallett_explicit = build("mallett2019", Some("BB4000"))
            .mallett_core
            .expect("Mallett matrix");
        assert_ne!(mallett_unset, mallett_explicit);
    }

    /// Python `compute_band_pass_filter` parity (model/color_filters.py) —
    /// reference values generated by replicating the pinned 0.3.4 code with
    /// numpy + math.erf for filter_uv=(0.6, 415, 9), filter_ir=(0.8, 670, 18)
    /// against the kodak_portra_400 wavelengths (380–780 nm, 5 nm step).
    #[test]
    fn camera_band_pass_filter_parity() {
        let bpf = compute_band_pass_filter([0.6, 415.0, 9.0], [0.8, 670.0, 18.0]);
        for (i, want) in &[
            (0usize, 0.40000001141188285_f64),
            (20, 1.0),
            (40, 0.99999999999938505),
            (60, 0.37282335245675713),
            (80, 0.19999999999999996),
        ] {
            assert_close(bpf[*i], *want, &format!("bandpass[{i}]"));
        }

        // Amplitudes clip to [0, 1] before the edge mix (Python np.clip).
        let clipped = compute_band_pass_filter([1.7, 415.0, 9.0], [0.0, 670.0, 18.0]);
        assert_close(
            clipped[40],
            1.0,
            "clamped uv amplitude passes full strength",
        );
        let zero_amp = compute_band_pass_filter([0.0, 415.0, 9.0], [0.0, 670.0, 18.0]);
        assert!(
            zero_amp.iter().all(|&v| v == 1.0),
            "zero amplitudes = identity"
        );
    }

    /// Python `FilmingStage._rgb_to_film_raw` camera-filter parity: the
    /// band-pass must multiply the sensitivity with reference-illuminant
    /// normalization (numerator `(sens * bpf) * illuminant`, denominator
    /// `sens * illuminant`, pairwise sums), preserving white balance.
    #[test]
    fn camera_uv_ir_filter_parity() {
        let mut sensitivity = portra_sensitivity();
        // The film profile's reference illuminant is D55; this is the same
        // illuminant used by the filming runtime for filter normalization.
        let illuminant = spectral::ILLUMINANT_D55_F64.to_vec();
        let before = sensitivity.clone();
        apply_camera_uv_ir_band_pass(
            &mut sensitivity,
            [0.6, 415.0, 9.0],
            [0.8, 670.0, 18.0],
            &illuminant,
        );

        // Filtered sensitivity reference values from the pinned upstream
        // filming path (D55 normalization).
        let cases = [
            (0usize, 0usize, 8.9233947803746231e-05_f64),
            (10, 1, 0.02811757780387461),
            (40, 2, 4.9000721126212758e-05),
            (60, 0, 0.0019786738339406741),
            (80, 2, 1.2709441518568956e-07),
        ];
        for (wl, c, want) in cases {
            assert_close(
                sensitivity[wl][c],
                want,
                &format!("filtered sensitivity[{wl}][{c}]"),
            );
        }

        // White balance is preserved: the filtered sensitivity integrates to
        // the same per-channel response under the reference illuminant.
        let mut resp_before = [0.0f64; 3];
        let mut resp_after = [0.0f64; 3];
        for wl in 0..N_WAVELENGTHS {
            for c in 0..3 {
                resp_before[c] += before[wl][c] * illuminant[wl];
                resp_after[c] += sensitivity[wl][c] * illuminant[wl];
            }
        }
        let ratio_before = resp_before[0] / resp_before[1];
        let ratio_after = resp_after[0] / resp_after[1];
        assert!(
            (ratio_before - ratio_after).abs() < 1e-12,
            "white balance shifted: {ratio_before} → {ratio_after}"
        );
    }

    /// `gaussian_blur_wavelength` parity with the `spectral_gaussian_blur`
    /// step of Python `compute_hanatos2025_tc_lut`
    /// (`scipy.ndimage.gaussian_filter(spectra_lut, (0, 0, sigma))`):
    /// truncate-4.0 radius, full-support kernel normalization, and the
    /// reflect boundary that repeats the edge sample. Reference values from
    /// an independent numpy replication of the scipy semantics.
    #[test]
    fn gaussian_blur_wavelength_parity() {
        let row: Vec<f64> = (0..81)
            .map(|i| 0.3 + 0.2 * (0.15 * i as f64 + 0.7).cos())
            .collect();
        let mut row = row;
        row[37] = 5.0;
        let cube = SpectraCube {
            size: 1,
            n_wavelengths: 81,
            data: row.clone(),
        };
        let blurred = gaussian_blur_wavelength(&cube, 7.5);
        for (i, want) in &[
            (0usize, 0.31560871760374259_f64),
            (1, 0.31340073670466867),
            (37, 0.64554715171155164),
            (40, 0.61810654447618485),
            (80, 0.42851042941668716),
        ] {
            assert_close(blurred.data[*i], *want, &format!("blur[{i}]"));
        }

        // scipy 'reflect' keeps a constant signal constant, and a zero sigma
        // is a no-op (Python guards `spectral_gaussian_blur > 0`).
        let flat = SpectraCube {
            size: 1,
            n_wavelengths: 81,
            data: vec![0.37; 81],
        };
        let flat_blurred = gaussian_blur_wavelength(&flat, 12.0);
        assert!(
            flat_blurred.data.iter().all(|&v| (v - 0.37).abs() < 1e-15),
            "constant spectrum must be preserved by the reflect boundary"
        );
        let unblurred = gaussian_blur_wavelength(&cube, 0.0);
        assert_eq!(unblurred.data, cube.data, "sigma 0 must be a no-op");

        // Full-width kernel normalization: radius int(4*7.5+0.5) = 30, and
        // the interior of a delta impulse reproduces the kernel weights.
        let mut impulse = vec![0.0f64; 81];
        impulse[40] = 1.0;
        let impulse_cube = SpectraCube {
            size: 1,
            n_wavelengths: 81,
            data: impulse,
        };
        let response = gaussian_blur_wavelength(&impulse_cube, 7.5).data;
        // Interior samples reproduce the (normalized) kernel weights:
        // out[40 + d] / out[40] == exp(-0.5 d²/σ²) for |d| small enough to
        // stay clear of both reflect boundaries.
        let sigma = 7.5f64;
        let peak = response[40];
        for d in -10i64..=10 {
            let want = (-0.5 / (sigma * sigma) * (d * d) as f64).exp();
            let got = response[(40 + d) as usize] / peak;
            assert!(
                (got - want).abs() < 1e-12,
                "kernel shape mismatch at offset {d}: {got} vs {want}"
            );
        }
    }

    /// `eval_poly4_log_exposure_surface` parity with Python
    /// `eval_poly4_log_exposure_surface` on the kodak_portra_400 surface
    /// parameters and D55 reference illuminant.
    #[test]
    fn poly4_surface_parity() {
        let film = crate::profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
        let surface_params = &film.data.hanatos2025_adaptation_surface_params;
        assert_eq!(surface_params.len(), 3);
        assert!(surface_params.iter().all(|r| r.len() == 15));

        // D55 xy from the f64 constants (matches Python `_illuminant_to_xy`).
        let xy = spektrafilm_math::spectral::illuminant_to_xy(&spectral::ILLUMINANT_D55);
        assert_close(xy.0, 0.33243163892154942, "D55 x");
        assert_close(xy.1, 0.34744439893064599, "D55 y");
        let (cx, cy) = spektrafilm_math::spectral::xy_to_tc(xy.0, xy.1);
        assert_close(cx, 0.44564751671296865, "surface center tx");
        assert_close(cy, 0.52046265100004552, "surface center ty");

        let size = 192;
        let surface = eval_poly4_log_exposure_surface(surface_params, xy, size);
        let at = |i: usize, j: usize, ch: usize| surface[(i * size + j) * 3 + ch];
        // (log2-correction, 2**correction) reference pairs.
        let cases = [
            (
                0usize,
                0usize,
                0usize,
                1.910822742667304_f64,
                3.7602347788263835_f64,
            ),
            (96, 96, 1, -0.01168258409209211, 0.9919349482334215),
            (191, 191, 2, 0.3418168367578539, 1.2673516100193958),
            (50, 130, 0, 0.08447324267349479, 1.0603005333758568),
        ];
        for (i, j, ch, want_s, want_pow) in cases {
            let got = at(i, j, ch);
            assert_close(got, want_s, &format!("surface[{i}][{j}][{ch}]"));
            // `raw_lut *= 2**surface` step.
            let mut lut = TcLut {
                size,
                channels: 3,
                data: vec![1.0; size * size * 3],
            };
            apply_log_exposure_surface(&mut lut, surface_params, xy);
            assert_close(
                lut.data[(i * size + j) * 3 + ch],
                want_pow,
                &format!("2**surface[{i}][{j}][{ch}]"),
            );
        }

        // The correction is bounded to ±2 stops (hanatos fit bound).
        assert!(
            surface.iter().all(|&v| v.abs() <= 2.0 + 1e-12),
            "surface exceeds the ±2-stop bound"
        );
    }

    /// `compute_hanatos2025_tc_lut` adaptation semantics: blur, window and
    /// surface each change the LUT, and malformed parameters fail with
    /// actionable errors instead of silently producing artifacts.
    #[test]
    fn compute_hanatos2025_tc_lut_adaptation() {
        let dir = data_dir();
        let spectra_lut = load_spectra_lut(&dir).unwrap();
        let sensitivity = portra_sensitivity();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let window_params = film.data.hanatos2025_adaptation_window_params.clone();
        let surface_params = &film.data.hanatos2025_adaptation_surface_params;
        let xy = spektrafilm_math::spectral::illuminant_to_xy(&spectral::ILLUMINANT_D55);
        let illu = spectral::ILLUMINANT_D55_F64;

        let adaptation = |blur: f64, win: bool, surf: bool| Hanatos2025Adaptation {
            window_params: &window_params,
            surface_params,
            spectral_gaussian_blur: blur,
            reference_illuminant: &illu,
            reference_illuminant_xy: xy,
            apply_window: win,
            apply_surface: surf,
        };

        let base =
            compute_hanatos2025_tc_lut(&spectra_lut, &sensitivity, &adaptation(0.0, true, false))
                .unwrap();
        let blurred =
            compute_hanatos2025_tc_lut(&spectra_lut, &sensitivity, &adaptation(10.0, true, false))
                .unwrap();
        let surfaced =
            compute_hanatos2025_tc_lut(&spectra_lut, &sensitivity, &adaptation(0.0, true, true))
                .unwrap();
        let windowless =
            compute_hanatos2025_tc_lut(&spectra_lut, &sensitivity, &adaptation(0.0, false, false))
                .unwrap();

        assert_eq!(base.size, spectra_lut.size);
        assert!(base.data.iter().zip(&blurred.data).any(|(a, b)| a != b));
        assert!(base.data.iter().zip(&surfaced.data).any(|(a, b)| a != b));
        assert!(base.data.iter().zip(&windowless.data).any(|(a, b)| a != b));
        // Windowing must preserve the reconstructed neutral in every channel.
        // The reference SPD has the same chromaticity but a different spectrum.
        let (tx, ty) = spectral::xy_to_tc(xy.0, xy.1);
        let sample = |lut: &TcLut| {
            spektrafilm_math::lut::bicubic_2d_f64(
                &lut.data,
                lut.size,
                lut.size,
                lut.channels,
                tx * (lut.size - 1) as f64,
                ty * (lut.size - 1) as f64,
            )
        };
        let neutral = sample(&windowless);
        let windowed_neutral = sample(&base);
        for channel in 0..3 {
            assert!((neutral[channel] - windowed_neutral[channel]).abs() < 1e-12);
        }

        // Malformed window (Python unpacks exactly 4 erf4 parameters).
        let short_window = [1.0, 2.0, 3.0];
        let bad = Hanatos2025Adaptation {
            window_params: &short_window,
            surface_params,
            spectral_gaussian_blur: 0.0,
            reference_illuminant: &illu,
            reference_illuminant_xy: xy,
            apply_window: true,
            apply_surface: false,
        };
        let err = compute_hanatos2025_tc_lut(&spectra_lut, &sensitivity, &bad)
            .err()
            .expect("invalid window must fail");
        assert!(err.contains("4 erf4"), "window error not actionable: {err}");

        // Malformed surface (Python unpacks exactly 3 × 15 coefficients).
        let bad_surface = vec![vec![0.0; 15], vec![0.0; 15]];
        let bad = Hanatos2025Adaptation {
            window_params: &window_params,
            surface_params: &bad_surface,
            spectral_gaussian_blur: 0.0,
            reference_illuminant: &illu,
            reference_illuminant_xy: xy,
            apply_window: true,
            apply_surface: true,
        };
        let err = compute_hanatos2025_tc_lut(&spectra_lut, &sensitivity, &bad)
            .err()
            .expect("invalid surface must fail");
        assert!(
            err.contains("3 channels"),
            "surface error not actionable: {err}"
        );
    }
    #[test]
    fn descriptor_parser_and_registry_shape_contract() {
        let text = r#"
            identifier = "example"
            file = "example.npy"
            kind = "reflectance"
            title = "Example"
            [array]
            lut_size = 2
            bands = 81
            spectral_shape = [380.0, 780.0, 5.0]
            [reflectance]
            scene_illuminant = "D65"
            midgray = 0.184
        "#;
        let d = parse_lut_descriptor(text).unwrap();
        assert_eq!(d.identifier, "example");
        assert_eq!(d.lut_size * d.lut_size * d.bands, 324);
        assert_eq!(d.scene_illuminant.as_deref(), Some("D65"));
    }

    #[test]
    fn reflectance_contract_has_three_channels() {
        let spectra = SpectraLut {
            size: 2,
            n_wavelengths: N_WAVELENGTHS,
            data: vec![1.0; 4 * N_WAVELENGTHS],
        };
        let sensitivity = vec![[1.0; 3]; N_WAVELENGTHS];
        let lut = compute_reflectance_tc_lut(&spectra, &sensitivity, &spectral::ILLUMINANT_D65_F64);
        assert_eq!(lut.channels, 3);
        assert_eq!(lut.data.len(), 2 * 2 * 3);
        assert!(lut.data.iter().all(|v| v.is_finite() && *v > 0.0));
    }

    #[test]
    fn bundled_reflectance_registry_methods_produce_finite_luts() {
        let data = data_dir();
        let sensitivity = portra_sensitivity();
        let ids = available_lut_identifiers(&data).unwrap();
        assert!(
            ids.len() >= 6,
            "expected bundled spectral registry entries: {ids:?}"
        );
        for id in ids {
            let descriptor = lut_descriptor(&data, &id).unwrap();
            if descriptor.kind != "reflectance" {
                continue;
            }
            let lut = compute_registered_tc_lut(
                &data,
                &id,
                &sensitivity,
                &spectral::ILLUMINANT_D55_F64,
                None,
            )
            .unwrap_or_else(|error| panic!("{id} failed: {error}"));
            assert_eq!(lut.channels, 3);
            assert!(
                lut.data.iter().all(|value| value.is_finite()),
                "{id} contains non-finite values"
            );
        }
    }

    #[test]
    fn generated_illuminants_are_supported_and_normalized() {
        for name in ["A", "D60", "D75", "E", "BB3400", "BB5000", "TH-KG3-L"] {
            assert!(is_supported_illuminant(name), "{name} should be supported");
            let values = select_illuminant_f64(name);
            assert_eq!(values.len(), N_WAVELENGTHS);
            assert!(values.iter().all(|value| value.is_finite() && *value > 0.0));
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            assert!((mean - 1.0).abs() < 1e-12, "{name} mean={mean}");
        }
        assert!(!is_supported_illuminant("BB1000"));
        assert!(!is_supported_illuminant("not-an-illuminant"));
    }
}
