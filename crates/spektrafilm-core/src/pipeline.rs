/// 3-stage simulation pipeline: Filming → Printing → Scanning.
///
/// Full calibration chain:
///   1. Load spectra LUT → compute TC LUT for film sensitivity
///   2. Process virtual gray card through filming to get midgray spectral density
///   3. Compute print exposure normalization factor from midgray spectral density
///   4. Pass all calibration data to the pipeline stages
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use rayon::prelude::*;
use spektrafilm_math::precision::{from_f64, to_f64};
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::spectral::TcLut;

/// Diagnostic helper: when env var `$var` is set, dump the image's f64
/// pixel data to that path as a raw little-endian f64 blob. Used by the
/// parity harness (`scripts/parity/run_parity.py`) to bisect the
/// Python ↔ Rust drift one stage at a time without modifying call
/// sites. Silent no-op when the env var is unset.
pub(crate) fn dump_if_env(var: &str, image: &ImageBuf) {
    let Ok(path) = std::env::var(var) else {
        return;
    };
    let bytes: Vec<u8> = image
        .data
        .iter()
        .flat_map(|&v| (v as f64).to_le_bytes())
        .collect();
    match std::fs::write(&path, &bytes) {
        Ok(()) => tracing::info!(
            path = %path,
            count = image.data.len(),
            "dumped f64 buffer for {}",
            var
        ),
        Err(e) => tracing::error!("dump {var} → {path}: {e}"),
    }
}

use crate::spectral_service::{select_illuminant, select_illuminant_f64};

fn stage_timings_enabled() -> bool {
    std::env::var_os("SPEKTRAFILM_STAGE_TIMINGS").is_some()
}

fn print_stage_timing(enabled: bool, stage: &str, start: Instant) {
    if enabled {
        eprintln!("stage {stage}: {} ms", start.elapsed().as_millis());
    }
}

fn record_stage_timing(
    timings: &mut Option<&mut BTreeMap<String, f64>>,
    stage: &str,
    start: Instant,
) {
    if let Some(timings) = timings.as_deref_mut() {
        *timings.entry(stage.to_string()).or_default() += start.elapsed().as_secs_f64();
    }
}

use crate::enlarger;
use crate::params::{RuntimeParams, Tap};
use crate::profile::Profile;
use crate::spectral_service;
use crate::stages;
fn interpolate(values: &[f64], wavelengths: &[f64], wavelength: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    if values.len() < 2 || wavelengths.len() < 2 {
        return values[0];
    }
    if wavelength <= wavelengths[0] {
        return values[0];
    }
    if wavelength >= wavelengths[wavelengths.len() - 1] {
        return values[values.len() - 1];
    }
    let upper = wavelengths.partition_point(|&x| x < wavelength).min(values.len() - 1);
    let lower = upper - 1;
    let fraction = (wavelength - wavelengths[lower]) / (wavelengths[upper] - wavelengths[lower]);
    values[lower] + (values[upper] - values[lower]) * fraction
}


fn gaussian_smooth(values: &[f64], sigma_points: f64) -> Vec<f64> {
    if values.is_empty() || sigma_points <= 0.0 {
        return values.to_vec();
    }
    let radius = (sigma_points * 4.0).ceil() as isize;
    let mut out = vec![0.0; values.len()];
    for (index, value) in out.iter_mut().enumerate() {
        let mut sum = 0.0;
        let mut weight_sum = 0.0;
        for offset in -radius..=radius {
            let sample = (index as isize + offset).clamp(0, values.len() as isize - 1) as usize;
            let weight = (-0.5 * (offset as f64 / sigma_points).powi(2)).exp();
            sum += values[sample] * weight;
            weight_sum += weight;
        }
        *value = sum / weight_sum;
    }
    out
}

pub(crate) fn apply_base_tuning(
    profile: &mut Profile,
    film_base: &crate::params::FilmBaseParams,
    print_base: Option<&crate::params::PrintBaseParams>,
) {
    let (active, scale, channels, tilt, sigma_points, peaks) = if let Some(base) = print_base {
        (
            base.active,
            base.scale,
            [base.yellow, base.magenta, base.cyan],
            0.0,
            20.0 / 5.0,
            [445.0, 530.0, 610.0],
        )
    } else {
        (
            film_base.active,
            film_base.scale,
            [film_base.yellow, film_base.magenta, film_base.cyan],
            film_base.tilt,
            35.0 / 5.0,
            [460.0, 555.0, 650.0],
        )
    };
    if !active || profile.data.base_density.is_empty() {
        return;
    }
    let neutral = channels == [1.0, 1.0, 1.0] && tilt == 0.0;
    if neutral {
        for value in &mut profile.data.base_density {
            if value.is_finite() {
                *value *= scale;
            }
        }
        return;
    }
    let base = &profile.data.base_density;
    let wavelengths = if profile.data.wavelengths.len() == base.len() {
        profile.data.wavelengths.as_slice()
    } else {
        &[]
    };
    let wavelength_at = |index: usize| {
        wavelengths
            .get(index)
            .copied()
            .unwrap_or(380.0 + 5.0 * index as f64)
    };
    let tuned = if print_base.is_some() {
        let channel_density = [
            interpolate(base, wavelengths, peaks[0]) * channels[0],
            interpolate(base, wavelengths, peaks[1]) * channels[1],
            interpolate(base, wavelengths, peaks[2]) * channels[2],
        ];
        let mut spectral = Vec::with_capacity(base.len());
        for i in 0..base.len() {
            let wavelength = wavelength_at(i);
            let value = if wavelength <= peaks[1] {
                let p = ((wavelength - peaks[0]) / (peaks[1] - peaks[0])).clamp(0.0, 1.0);
                channel_density[0] * (1.0 - p) + channel_density[1] * p
            } else {
                let p = ((wavelength - peaks[1]) / (peaks[2] - peaks[1])).clamp(0.0, 1.0);
                channel_density[1] * (1.0 - p) + channel_density[2] * p
            };
            spectral.push(value);
        }
        gaussian_smooth(&spectral, sigma_points)
    } else {
        let mut channel_scale = Vec::with_capacity(base.len());
        for i in 0..base.len() {
            let wavelength = wavelength_at(i);
            let value = if wavelength <= peaks[1] {
                let p = ((wavelength - peaks[0]) / (peaks[1] - peaks[0])).clamp(0.0, 1.0);
                channels[0] * (1.0 - p) + channels[1] * p
            } else {
                let p = ((wavelength - peaks[1]) / (peaks[2] - peaks[1])).clamp(0.0, 1.0);
                channels[1] * (1.0 - p) + channels[2] * p
            };
            channel_scale.push(value);
        }
        let density_scale = gaussian_smooth(&channel_scale, sigma_points);
        let tilt_scale: Vec<f64> = (0..base.len())
            .map(|i| {
                let wavelength = wavelength_at(i);
                (1.0 + tilt / 95.0 * (wavelength - 555.0)).max(0.0)
            })
            .collect();
        let density_tilt = gaussian_smooth(&tilt_scale, sigma_points);
        base.iter()
            .zip(density_scale.iter().zip(density_tilt.iter()))
            .map(|(&value, (&channel, &tilt_value))| value * channel * tilt_value)
            .collect()
    };
    for (value, replacement) in profile.data.base_density.iter_mut().zip(tuned) {
        if value.is_finite() {
            *value = replacement * scale;
        }
    }
}

fn apply_film_chemistry(profile: &mut Profile, chemistry: &crate::params::PrintCurvesMorphParams) -> Result<(), String> {
    let Some(model) = profile.data.density_curves_model.as_ref() else { return Ok(()); };
    if model.n_layers() == 0 { return Ok(()); }
    let (curves, layers) = crate::print_morph::morph_density_curves_with_layers(
        &profile.log_exposure_f64(), model, chemistry, profile.is_positive(),
    )?;
    let has_sublayers = model.n_layers() > 1;
    profile.data.density_curves = curves.into_iter().map(|row| row.to_vec()).collect();
    profile.data.density_curves_layers = if has_sublayers { layers } else { Vec::new() };
    Ok(())
}

#[derive(Clone)]
pub struct Pipeline {
    pub film: Profile,
    pub print: Profile,
    pub params: RuntimeParams,
    tc_lut: Option<TcLut>,
    /// Mallett2019 reflectance-basis core matrix (linear-sRGB → raw), when that
    /// upsampler is selected instead of the hanatos2025 tc LUT. Mutually
    /// exclusive with `tc_lut`.
    mallett_core: Option<[[f64; 3]; 3]>,
    /// Illuminant used by the RGB→tc projection before TC LUT lookup.
    /// Most methods use the film reference illuminant; arctic2026alpha02
    /// recovers reflectance in D65 coordinates and relights separately.
    front_illuminant: String,
    /// Print exposure normalization factor (1/geomean of midgray raw through enlarger).
    print_exposure_factor: f64,
    /// Filtered enlarger illuminant for printing stage (f64 for Python parity).
    print_illuminant: Vec<f64>,
    /// Constant preflash raw 3-vector added to the print exposure before the
    /// inner log10. `[0; 3]` unless the enlarger preflash is active.
    preflash_raw: [f64; 3],
    /// Output gamut compressor (built once; identity unless oklch is enabled).
    output_gamut: crate::gamut_compression::OutputGamutCompress,
    /// Data directory the spectral front-end was built from. Present on
    /// `new_with_spectral` pipelines; lets `with_params` rebuild the TC LUT
    /// when a sensitivity-affecting control changes instead of reusing a
    /// stale one.
    data_dir: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Copy)]
enum ResidentFallbackReason {
    WorkflowRoute,
    LangmuirChemistry,
    InputTransferDecoding,
    RequestedSpectralLut,
    ActiveOpticalDiffusion,
    FaithfulGrainDistribution,
    UnsupportedOutputGamut,
    BlurRadiusExceedsBackendSupport,
    MissingResidentFrontPass,
}

#[derive(Debug)]
enum ResidentDecision {
    UseResident,
    PerStage { reasons: Vec<ResidentFallbackReason> },
}

impl ResidentDecision {
    fn reasons(reasons: Vec<ResidentFallbackReason>) -> Self {
        if reasons.is_empty() {
            Self::UseResident
        } else {
            Self::PerStage { reasons }
        }
    }
}

impl Pipeline {
    /// Accessor for the pre-computed TC LUT (used by parity tests).
    pub fn tc_lut(&self) -> Option<&TcLut> {
        self.tc_lut.as_ref()
    }
    /// Accessor for the print exposure factor (parity tests).
    pub fn print_exposure_factor(&self) -> f64 {
        self.print_exposure_factor
    }
    /// Accessor for the filtered print illuminant (parity tests).
    pub fn print_illuminant_slice(&self) -> &[f64] {
        &self.print_illuminant
    }
    /// Accessor for the constant preflash raw vector (parity tests / debug).
    pub fn preflash_raw(&self) -> [f64; 3] {
        self.preflash_raw
    }

    /// Return a copy of this calibrated pipeline with updated runtime params.
    ///
    /// When a sensitivity-affecting (spectral) control changes, the whole
    /// spectral front-end is rebuilt from `data_dir` — mirroring the cache
    /// invalidation of Python's `SpectralLUTService`, where
    /// `set_hanatos2025_adaptation` clears the filming tc_lut whenever the
    /// adaptation state changes and `get_filming_tc_lut` re-keys on the
    /// (camera-filtered) sensitivity. Reusing the stale LUT would silently
    /// ignore the new UV/IR filters, blur and surface settings.
    ///
    /// Calibration-affecting runtime controls are detected and rebuilt from
    /// the spectral source data; rebuild and validation failures are returned
    /// instead of retaining stale calibration.
    pub fn with_params(mut self, params: RuntimeParams) -> Result<Self, String> {
        let mut params = params;
        params.validate_color()?;
        crate::params_builder::broadcast_monochrome_layout(&self.film, &mut params);
        if let Some(model) = self.print.data.density_curves_model.as_ref() {
            crate::print_morph::morph_density_curves(
                &self.print.log_exposure_f64(),
                model,
                &params.print_render.density_curves_morph,
                self.print.is_positive(),
            )
            .map_err(|error| format!("invalid print density-curve model: {error}"))?;
        }
        let spectral_changed = spectral_controls_key(&self.params) != spectral_controls_key(&params);
        let calibration_changed =
            calibration_controls_key(&self.params) != calibration_controls_key(&params);
        if spectral_changed || calibration_changed {
            let data_dir = self
                .data_dir
                .clone()
                .ok_or_else(|| "pipeline lacks data_dir for calibration rebuild".to_owned())?;
            return Self::new_with_spectral(
                self.film.clone(),
                self.print.clone(),
                params,
                &data_dir,
            );
        }
        self.output_gamut = crate::gamut_compression::OutputGamutCompress::build(
            &params.io.output_gamut_compress,
            &params.io.output_color_space,
        )?;
        self.params = params;
        Ok(self)
    }
}

/// Parameters read while constructing derived print calibration data.
#[derive(Debug, Clone, PartialEq)]
struct CalibrationControlsKey {
    exposure_compensation_ev: f32,
    print_exposure_compensation: bool,
    normalize_print_exposure: bool,
    illuminant: String,
    y_filter_shift: f32,
    m_filter_shift: f32,
    y_filter_neutral: f32,
    m_filter_neutral: f32,
    c_filter_neutral: f32,
    preflash_exposure: f32,
    preflash_y_filter_shift: f32,
    preflash_m_filter_shift: f32,
    neutral_print_filters_from_database: bool,
}

fn calibration_controls_key(params: &RuntimeParams) -> CalibrationControlsKey {
    CalibrationControlsKey {
        exposure_compensation_ev: params.camera.exposure_compensation_ev,
        print_exposure_compensation: params.enlarger.print_exposure_compensation,
        normalize_print_exposure: params.enlarger.normalize_print_exposure,
        illuminant: params.enlarger.illuminant.clone(),
        y_filter_shift: params.enlarger.y_filter_shift,
        m_filter_shift: params.enlarger.m_filter_shift,
        y_filter_neutral: params.enlarger.y_filter_neutral,
        m_filter_neutral: params.enlarger.m_filter_neutral,
        c_filter_neutral: params.enlarger.c_filter_neutral,
        preflash_exposure: params.enlarger.preflash_exposure,
        preflash_y_filter_shift: params.enlarger.preflash_y_filter_shift,
        preflash_m_filter_shift: params.enlarger.preflash_m_filter_shift,
        neutral_print_filters_from_database: params.settings.neutral_print_filters_from_database,
    }
}

/// The runtime-settable controls that change the spectral front-end (TC LUT
/// or Mallett core) and the calibration derived from it.
///
/// Python equivalents: the filming tc-lut cache of `SpectralLUTService`
/// re-keys on the adaptation state (`_same_hanatos2025_adaptation`: window
/// and surface params, apply flags, `spectral_gaussian_blur`, reference
/// illuminant) and on the sensitivity array — which the camera UV/IR
/// filters modify — plus the input-gamut spec baked into the LUT. The
/// adaptation parameters and reference illuminant are fixed per film
/// profile here, so only the runtime parts are listed. `use_cat16` feeds
/// the midgray calibration through the RGB→tc projection.
fn spectral_controls_key(params: &RuntimeParams) -> SpectralControlsKey {
    SpectralControlsKey {
        rgb_to_raw_method: params.settings.rgb_to_raw_method.clone(),
        apply_hanatos2025_adaptation_window: params.settings.apply_hanatos2025_adaptation_window,
        apply_hanatos2025_adaptation_surface: params.settings.apply_hanatos2025_adaptation_surface,
        spectral_gaussian_blur: params.settings.spectral_gaussian_blur,
        use_cat16: params.settings.use_cat16,
        filter_uv: params.camera.filter_uv,
        filter_ir: params.camera.filter_ir,
        color_filter: params.camera.color_filter.clone(),
        input_gamut_active: params.io.input_gamut_compress.active,
        input_gamut_algorithm: params.io.input_gamut_compress.algorithm.clone(),
        input_gamut_boundary: params.io.input_gamut_compress.boundary.clone(),
        input_gamut_hull_detail: params.io.input_gamut_compress.hull_detail,
        input_gamut_knee: params.io.input_gamut_compress.knee,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SpectralControlsKey {
    rgb_to_raw_method: String,
    apply_hanatos2025_adaptation_window: bool,
    apply_hanatos2025_adaptation_surface: bool,
    spectral_gaussian_blur: f32,
    use_cat16: bool,
    filter_uv: [f32; 3],
    filter_ir: [f32; 3],
    color_filter: String,
    input_gamut_active: bool,
    input_gamut_algorithm: String,
    input_gamut_boundary: String,
    input_gamut_hull_detail: f64,
    input_gamut_knee: [f32; 3],
}

impl Pipeline {
    /// Create pipeline without spectral LUT — **test-only**.
    ///
    /// Production callers must use [`Pipeline::new_with_spectral`]: the
    /// spectral LUT is the calibrated front end of the 0.3.4 pipeline, and
    /// silently degrading to the identity front end changes consumer-visible
    /// output. Kept for the parity/unit tests that exercise the stage chain
    /// without paying LUT construction.
    #[cfg(test)]
    pub fn new(film: Profile, print: Profile, params: RuntimeParams) -> Self {
        params.validate_color().expect("invalid colour configuration");
        // B&W profiles must be resolved on every construction path —
        // an unresolved family would read development-time columns as
        // R/G/B channels.
        let mut film = crate::profile::resolve_for_render(film, params.film_render.development_time);
        let mut params = params;
        if params.workflow.route == "input > film > scan" {
            params.io.scan_film = true;
        }
        crate::params_builder::broadcast_monochrome_layout(&film, &mut params);
        let mut print = crate::profile::resolve_for_render(print, params.print_render.development_time);
        apply_base_tuning(&mut film, &params.film_render.base, None);
        apply_film_chemistry(&mut film, &params.film_render.chemistry).expect("invalid film chemistry");
        apply_base_tuning(&mut print, &params.film_render.base, Some(&params.print_render.base));
        let print_illuminant = enlarger::enlarger_filtered_illuminant_f64(
            &params.enlarger.illuminant,
            params.enlarger.c_filter_neutral as f64,
            (params.enlarger.m_filter_neutral + params.enlarger.m_filter_shift) as f64,
            (params.enlarger.y_filter_neutral + params.enlarger.y_filter_shift) as f64,
        );
        let output_gamut = crate::gamut_compression::OutputGamutCompress::build(
            &params.io.output_gamut_compress,
            &params.io.output_color_space,
        ).expect("validated output gamut configuration");
        let front_illuminant = film.info.reference_illuminant.clone();
        Self {
            film,
            print,
            params,
            tc_lut: None,
            mallett_core: None,
            front_illuminant,
            print_exposure_factor: 1.0,
            print_illuminant,
            preflash_raw: [0.0; 3],
            output_gamut,
            data_dir: None,
        }
    }

    /// Create pipeline with full Hanatos2025 spectral upsampling and calibration.
    pub fn new_with_spectral(
        film: Profile,
        print: Profile,
        mut params: RuntimeParams,
        data_dir: &Path,
    ) -> Result<Self, String> {
        // Reject malformed params (e.g. an unknown diffusion-filter family)
        // up front — Python raises ValueError mid-run, which also aborts
        // before any artifact; failing at construction keeps the error
        // actionable and artifact-free on every entry point.
        params.validate()?;
        if params.workflow.route == "input > film > scan" {
            params.io.scan_film = true;
        }
        crate::profile::validate_profile(&film).map_err(|error| error.to_string())?;
        crate::profile::validate_profile(&print).map_err(|error| error.to_string())?;
        // B&W profiles: collapse the development-time family to the selected
        // time and broadcast the single channel onto the 3-channel engine
        // layout. Must happen before anything reads the profile data.
        let mut film = crate::profile::resolve_for_render(film, params.film_render.development_time);
        let mut print = crate::profile::resolve_for_render(print, params.print_render.development_time);
        apply_base_tuning(&mut film, &params.film_render.base, None);
        apply_film_chemistry(&mut film, &params.film_render.chemistry)?;
        apply_base_tuning(&mut print, &params.film_render.base, Some(&params.print_render.base));

        // Stock defaults belong to profile selection / digest_params, so
        // construction preserves later user edits and debug deactivation.
        crate::params_builder::broadcast_monochrome_layout(&film, &mut params);
        if let Some(model) = print.data.density_curves_model.as_ref() {
            crate::print_morph::morph_density_curves(
                &print.log_exposure_f64(),
                model,
                &params.print_render.density_curves_morph,
                print.is_positive(),
            )
            .map_err(|error| format!("invalid print density-curve model: {error}"))?;
        }

        // Python parity: look up per-(print, illuminant, film) neutral filter values from
        // the JSON database — matches `apply_database_neutral_print_filters`. Defaults to
        // params.enlarger.{c,m,y}_filter_neutral when the combo isn't in the database.
        // Keep the f64 lookup values around (params is f32) — narrowing to
        // f32 here costs ~4e-8 precision through the `10^(-cc/100)` step.
        let mut neutral_cmy_f64: Option<[f64; 3]> = None;
        if params.settings.neutral_print_filters_from_database {
            let db = crate::neutral_filters::NeutralFilters::load(data_dir)?;
            let print_stock = print.info.stock.as_deref().unwrap_or("");
            let film_stock = film.info.stock.as_deref().unwrap_or("");
            if let Some([c, m, y]) = db.lookup(print_stock, &params.enlarger.illuminant, film_stock) {
                params.enlarger.c_filter_neutral = c as f32;
                params.enlarger.m_filter_neutral = m as f32;
                params.enlarger.y_filter_neutral = y as f32;
                neutral_cmy_f64 = Some([c, m, y]);
            }
        }
        let cmy_f64 = neutral_cmy_f64.unwrap_or([
            params.enlarger.c_filter_neutral as f64,
            params.enlarger.m_filter_neutral as f64,
            params.enlarger.y_filter_neutral as f64,
        ]);
        let (c_neutral_f64, m_neutral_f64, y_neutral_f64) = (cmy_f64[0], cmy_f64[1], cmy_f64[2]);

        // Film sensitivity: Python `sensitivity = np.nan_to_num(10 ** log_sensitivity)` — f64.
        let log_sens = film.log_sensitivity_f64();
        let mut sensitivity: Vec<[f64; 3]> = log_sens
            .iter()
            .map(|row| {
                let mut out = [0.0f64; 3];
                for c in 0..3 {
                    let v = 10.0f64.powf(row[c]);
                    out[c] = if v.is_nan() { 0.0 } else { v };
                }
                out
            })
            .collect();

        let ref_illuminant = select_illuminant(&film.info.reference_illuminant);
        let ref_illuminant_f64 = select_illuminant_f64(&film.info.reference_illuminant);
        let mut front_illuminant = film.info.reference_illuminant.clone();

        // Camera UV/IR filter — Python `FilmingStage._rgb_to_film_raw` filters
        // the film sensitivity through the UV/IR band-pass (with
        // reference-illuminant normalization preserving white balance) before
        // BOTH upsampler branches and before the midgray calibration below.
        // Off at default amplitudes (0, 0), matching Python's guard
        // `filter_uv[0] > 0 or filter_ir[0] > 0`.
        if params.camera.filter_uv[0] > 0.0 || params.camera.filter_ir[0] > 0.0 {
            spectral_service::apply_camera_uv_ir_band_pass(
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
        if params.camera.color_filter != spectral_service::NO_COLOR_FILTER {
            let transmittance = spectral_service::load_color_filter_transmittance(
                data_dir,
                &params.camera.color_filter,
            )?
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
        let (tc_lut, mallett_core): (Option<TcLut>, Option<[[f64; 3]; 3]>) =
            match params.settings.rgb_to_raw_method.as_str() {
                "hanatos2025" => {
                    let window_params: Vec<f64> =
                        film.data.hanatos2025_adaptation_window_params.clone();
                    let adaptation = spectral_service::Hanatos2025Adaptation {
                        window_params: &window_params,
                        surface_params: &film.data.hanatos2025_adaptation_surface_params,
                        spectral_gaussian_blur: params.settings.spectral_gaussian_blur as f64,
                        reference_illuminant: &ref_illuminant_f64,
                        reference_illuminant_xy: spektrafilm_math::spectral::illuminant_to_xy(&ref_illuminant),
                        apply_window: params.settings.apply_hanatos2025_adaptation_window,
                        apply_surface: params.settings.apply_hanatos2025_adaptation_surface,
                    };
                    let tc_lut = spectral_service::compute_registered_tc_lut(
                        data_dir, "hanatos2025", &sensitivity, &ref_illuminant_f64, Some(&adaptation),
                    )?;

                    // Bake input gamut compression into the LUT at build time, around
                    // the film reference illuminant (the runtime's achromatic axis).
                    // The per-pixel path then stays compression-agnostic.
                    let input_gamut = crate::input_gamut::InputGamutCompress::build(
                        &params.io.input_gamut_compress,
                    )?;
                    let tc_lut = if input_gamut.is_active() {
                        let (rx, ry) = spektrafilm_math::spectral::illuminant_to_xy(&ref_illuminant);
                        input_gamut.remap(&tc_lut, [rx, ry])
                    } else {
                        tc_lut
                    };
                    (Some(tc_lut), None)
                }
                "arctic2026alpha02" | "arctic2026beta04" | "gauss-lasers" | "jakob2019" | "otsu2018" => {
                    front_illuminant = "D65".into();
                    let tc_lut = spectral_service::compute_registered_tc_lut(
                        data_dir, params.settings.rgb_to_raw_method.as_str(),
                        &sensitivity, &ref_illuminant_f64, None,
                    )?;
                    let input_gamut = crate::input_gamut::InputGamutCompress::build(
                        &params.io.input_gamut_compress,
                    )?;
                    let tc_lut = if input_gamut.is_active() {
                        let (rx, ry) = spektrafilm_math::spectral::illuminant_to_xy(
                            &select_illuminant(&front_illuminant),
                        );
                        input_gamut.remap(&tc_lut, [rx, ry])
                    } else {
                        tc_lut
                    };
                    (Some(tc_lut), None)
                }
                "mallett2019" => (
                    None,
                    Some(crate::mallett::compute_core_matrix(
                        &sensitivity,
                        &ref_illuminant_f64,
                    )),
                ),
                other => {
                    return Err(format!("unsupported rgb_to_raw_method: {other:?}"));
                }
            };

        // Python parity: sensitivities are pre-balanced in the profile so midgray ≈ 1.0.
        // The TC LUT is used unnormalized — Python's `rgb_to_raw_hanatos2025` does NOT scale it.
        // See `spektrafilm/utils/spectral_upsampling.py:rgb_to_raw_hanatos2025` (comment line 373).

        // Compute enlarger illuminant with dichroic filters — f64 for Python parity.
        // c/m/y come from the f64 lookup, shift values are f32 in params.
        let print_illuminant = enlarger::enlarger_filtered_illuminant_f64(
            &params.enlarger.illuminant,
            c_neutral_f64,
            m_neutral_f64 + params.enlarger.m_filter_shift as f64,
            y_neutral_f64 + params.enlarger.y_filter_shift as f64,
        );

        // Midgray sensor raw for the print-exposure normalization — always in
        // sRGB (Python `_rgb_to_film_raw` default), through whichever upsampler
        // is active. `scale` folds the EV compensation for the `_comp` branch.
        let midgray_raw = |scale: f64| -> [f64; 3] {
            if let Some(lut) = &tc_lut {
                enlarger::midgray_raw_hanatos(
                    lut,
                    &select_illuminant(&front_illuminant),
                    params.settings.use_cat16,
                    scale,
                )
            } else {
                let core = mallett_core
                    .as_ref()
                    .expect("one upsampler is always built");
                let m = crate::mallett::film_matrix(core, "sRGB");
                let g = 0.184 * scale;
                crate::mallett::apply(&m, [g, g, g])
            }
        };

        // Compute midgray spectral density (gray card through full filming path)
        let density_spectral_midgray =
            enlarger::midgray_density_spectral_from_raw(midgray_raw(1.0), &film, &params);

        // Print sensitivity: Python `sensitivity = np.nan_to_num(10 ** log_sensitivity)` — f64 with NaN→0.
        let print_log_sens = print.log_sensitivity_f64();
        let print_sensitivity: Vec<[f64; 3]> = print_log_sens
            .iter()
            .map(|row| {
                let mut out = [0.0f64; 3];
                for c in 0..3 {
                    let v = 10.0f64.powf(row[c]);
                    out[c] = if v.is_nan() { 0.0 } else { v };
                }
                out
            })
            .collect();

        // Constant enlarger preflash exposure (Python `_compute_raw_preflash`):
        // film base density lit by the preflash-filtered illuminant, integrated
        // against the print sensitivity. Off (→ [0; 3]) unless preflash_exposure > 0.
        let preflash_raw = enlarger::compute_preflash_raw(
            &params.enlarger,
            c_neutral_f64,
            m_neutral_f64,
            y_neutral_f64,
            &film.data.base_density,
            &print_sensitivity,
        );

        // Print exposure normalization — mirror Python's
        // `_compute_exposure_factor_midgray` in
        // `spektrafilm/runtime/stages/printing.py`. There are two
        // candidate factors:
        //   * `factor_midgray`     = 1 / geomean(raw_midgray)
        //   * `factor_midgray_comp`= 1 / geomean(raw_midgray_with_neg_EV)
        // and four flag combinations of
        // `enlarger.normalize_print_exposure` × `enlarger.print_exposure_compensation`.
        //
        // With both flags ON (defaults) and no EV compensation,
        // Python returns `factor_midgray_comp == 1.0`. Rust used to
        // unconditionally apply `factor_midgray` here, which biased
        // the print exposure by ~3% on every render and was the root
        // cause of the residual Python-parity drift in the print stage.
        let factor_midgray = enlarger::compute_exposure_factor(
            &density_spectral_midgray,
            &print_illuminant,
            &print_sensitivity,
        );
        // Python builds `density_spectral_midgray_comp` whenever
        // `print_exposure_compensation` is on — even when EV == 0, in
        // which case `rgb_midgray_comp = rgb_midgray * 2^0` and so
        // `factor_midgray_comp == factor_midgray`. We have to mirror
        // that (NOT short-circuit to 1.0) because the
        // `factor_midgray_comp` branch is what gets returned by default.
        let factor_midgray_comp = if !params.enlarger.print_exposure_compensation {
            1.0
        } else if params.camera.exposure_compensation_ev == 0.0 {
            factor_midgray
        } else {
            // Python: `rgb_midgray_comp = rgb_midgray * 2 ** exposure_compensation_ev`.
            let scale = 2.0f64.powf(params.camera.exposure_compensation_ev as f64);
            let density_spectral_midgray_comp =
                enlarger::midgray_density_spectral_from_raw(midgray_raw(scale), &film, &params);
            enlarger::compute_exposure_factor(
                &density_spectral_midgray_comp,
                &print_illuminant,
                &print_sensitivity,
            )
        };
        let print_exposure_factor = match (
            params.enlarger.normalize_print_exposure,
            params.enlarger.print_exposure_compensation,
        ) {
            (true, true) => factor_midgray_comp,
            (true, false) => factor_midgray,
            (false, true) => factor_midgray_comp / factor_midgray,
            (false, false) => 1.0,
        };

        tracing::info!(
            method = params.settings.rgb_to_raw_method,
            lut_size = tc_lut.as_ref().map_or(0, |l| l.size),
            film = film.info.stock.as_deref().unwrap_or("unknown"),
            print_exposure_factor = print_exposure_factor,
            "pipeline calibrated"
        );

        let output_gamut = crate::gamut_compression::OutputGamutCompress::build(
            &params.io.output_gamut_compress,
            &params.io.output_color_space,
        )?;
        Ok(Self {
            film,
            print,
            params,
            tc_lut,
            mallett_core,
            front_illuminant,
            print_exposure_factor,
            print_illuminant,
            preflash_raw,
            data_dir: Some(data_dir.to_path_buf()),
            output_gamut,
        })
    }

    /// Topological tap order of the active chain — the Rust spelling of
    /// upstream `_build_topology`. `io.scan_film` short-circuits the print
    /// stages exactly like the Python node list.
    fn tap_order(&self) -> &'static [Tap] {
        if self.params.io.scan_film {
            &[
                Tap::RgbIn,
                Tap::RgbPre,
                Tap::LogEFilm,
                Tap::CmyFilm,
                Tap::RgbOut,
            ]
        } else {
            &[
                Tap::RgbIn,
                Tap::RgbPre,
                Tap::LogEFilm,
                Tap::CmyFilm,
                Tap::LogEPrint,
                Tap::CmyPrint,
                Tap::RgbOut,
            ]
        }
    }

    /// One topology node: transform the image at tap `at` into its value at
    /// the next tap in [`Pipeline::tap_order`]. `RgbOut` is terminal.
    fn fire_node(
        &self,
        at: Tap,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        color_ref: &crate::color_reference::ColorReference,
        pixel_size_um: f64,
        ae_ev: f64,
        stage_timings: &mut Option<&mut BTreeMap<String, f64>>,
    ) -> Result<ImageBuf, String> {
        let print_timings = stage_timings_enabled();
        match at {
            Tap::RgbIn => unreachable!("rgb_in preprocessing is handled by run_from"),
            Tap::RgbPre => {
                let t = Instant::now();
                let out = stages::filming::expose(
                    &image,
                    &self.film,
                    &self.params,
                    backend,
                    self.tc_lut.as_ref(),
                    self.mallett_core.as_ref(),
                    &select_illuminant(&self.front_illuminant),
                    color_ref.filming_exposure_correction,
                    pixel_size_um,
                    ae_ev,
                );
                print_stage_timing(print_timings, "filming_expose", t);
                record_stage_timing(stage_timings, "filming_expose", t);
                dump_if_env("SPEKTRAFILM_DUMP_FILM_LOG_RAW", &out);
                Ok(out)
            }
            Tap::LogEFilm => {
                let t = Instant::now();
                let out = stages::filming::develop(&image, &self.film, &self.params, backend, pixel_size_um);
                print_stage_timing(print_timings, "filming_develop", t);
                record_stage_timing(stage_timings, "filming_develop", t);
                tracing::info!("pipeline: filming complete");
                dump_if_env("SPEKTRAFILM_DUMP_FILM_DENSITY", &out);
                Ok(out)
            }
            Tap::CmyFilm => {
                if self.params.io.scan_film {
                    let t = Instant::now();
                    let out = stages::scanning::process(
                        &image,
                        &self.film,
                        &self.params,
                        backend,
                        color_ref,
                        &self.output_gamut,
                    );
                    print_stage_timing(print_timings, "scanning", t);
                    record_stage_timing(stage_timings, "scanning", t);
                    tracing::info!("pipeline: scanning complete (film scan)");
                    Ok(out)
                } else {
                    let t = Instant::now();
                    let out = stages::printing::expose_calibrated(
                        &image,
                        &self.film,
                        &self.print,
                        &self.params,
                        backend,
                        &self.print_illuminant,
                        self.print_exposure_factor,
                        self.preflash_raw,
                        color_ref.printing_exposure_correction,
                        pixel_size_um,
                    );
                    print_stage_timing(print_timings, "printing", t);
                    record_stage_timing(stage_timings, "printing", t);
                    dump_if_env("SPEKTRAFILM_DUMP_PRINT_LOG_RAW", &out);
                    Ok(out)
                }
            }
            Tap::LogEPrint => {
                let t = Instant::now();
                let out = stages::printing::develop(&image, &self.print, &self.params, backend)?;
                print_stage_timing(print_timings, "printing_develop", t);
                record_stage_timing(stage_timings, "printing_develop", t);
                dump_if_env("SPEKTRAFILM_DUMP_PRINT_DENSITY", &out);
                Ok(out)
            }
            Tap::CmyPrint => {
                let t = Instant::now();
                let out = stages::scanning::process(
                    &image,
                    &self.print,
                    &self.params,
                    backend,
                    color_ref,
                    &self.output_gamut,
                );
                print_stage_timing(print_timings, "scanning", t);
                record_stage_timing(stage_timings, "scanning", t);
                tracing::info!("pipeline: printing complete");
                Ok(out)
            }
            Tap::RgbOut => unreachable!("RgbOut is terminal — run_from stops before it"),
        }
    }

    /// Run the topology from the value at `inject` to the value at
    /// `collect` (both inclusive bounds in [`Pipeline::tap_order`]).
    /// Mirrors upstream `run_topology`: unreachable pairs error with the
    /// upstream message, `inject == collect` returns the input unchanged.
    fn run_from(
        &self,
        image: ImageBuf,
        inject: Tap,
        collect: Tap,
        backend: &dyn ComputeBackend,
        color_ref: &crate::color_reference::ColorReference,
        physical_context: Option<(f64, f64)>,
        stage_timings: Option<&mut BTreeMap<String, f64>>,
    ) -> Result<ImageBuf, String> {
        let order = self.tap_order();
        let Some(ip) = order.iter().position(|t| *t == inject) else {
            return Err(format!(
                "tap {inject:?} is not part of this topology (scan_film = {})",
                self.params.io.scan_film
            ));
        };
        let Some(cp) = order.iter().position(|t| *t == collect) else {
            return Err(format!(
                "tap {collect:?} is not part of this topology (scan_film = {})",
                self.params.io.scan_film
            ));
        };
        if cp < ip {
            return Err(format!(
                "no node path reaches tap {:?} from {:?}",
                collect, inject
            ));
        }
        let (mut cur, pixel_size_um, ae_ev) = if inject == Tap::RgbIn && cp > ip {
            let ae_ev = self.meter_autoexposure(&image);
            let image = self.apply_autoexposure(image, ae_ev);
            let (working, pitch) = crate::resizing::crop_and_rescale(
                &image, &self.params.io, self.params.camera.film_format_mm,
            )?;
            let working = match working {
                std::borrow::Cow::Owned(working) => working,
                std::borrow::Cow::Borrowed(_) => image,
            };
            (working, pitch, 0.0)
        } else {
            let (pitch, ae_ev) = physical_context.unwrap_or_else(|| (
                self.params.camera.film_format_mm as f64 * 1000.0
                    / image.width.max(image.height).max(1) as f64, 0.0,
            ));
            (image, pitch, ae_ev)
        };
        let mut stage_timings = stage_timings;
        let mut i = ip;
        if inject == Tap::RgbIn && cp > ip { i += 1; }
        while i < cp {
            cur = self.fire_node(
                order[i],
                cur,
                backend,
                color_ref,
                pixel_size_um,
                ae_ev,
                &mut stage_timings,
            )?;
            i += 1;
        }
        Ok(cur)
    }

    /// Process an image with explicit pipeline taps — the Rust spelling of
    /// upstream `SimulationPipeline.process(image, inject, collect)`.
    ///
    /// Omitted endpoints use persistent `params.taps`, then `rgb_in` /
    /// `rgb_out`. Injecting at `log_e_film` or later bypasses the camera
    /// (and film) stages upstream of the injection point; collecting at
    /// `cmy_film` returns film densities instead of final RGB. Invalid or
    /// unreachable pairs fail with the upstream error text.
    ///
    /// Nondefault endpoints use per-stage boundaries. The normal end-to-end
    /// pair retains the fused GPU-resident path and full-input preparation.
    pub fn process_with_taps(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
    ) -> Result<ImageBuf, String> {
        self.process_with_taps_timed(image, backend, inject, collect, None)
    }

    /// Process while collecting per-node wall-clock timings.
    pub fn process_with_timings(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        timings: &mut BTreeMap<String, f64>,
    ) -> Result<ImageBuf, String> {
        self.process_with_taps_timed(image, backend, None, None, Some(timings))
    }

    pub fn process_with_taps_timed(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
        mut timings: Option<&mut BTreeMap<String, f64>>,
    ) -> Result<ImageBuf, String> {
        let inject = match inject {
            Some(tap) => tap,
            None => self.params.taps.inject.as_deref().map(Tap::parse)
                .transpose()?.unwrap_or(Tap::RgbIn),
        };
        let collect = match collect {
            Some(tap) => tap,
            None => self.params.taps.collect.as_deref().map(Tap::parse)
                .transpose()?.unwrap_or(Tap::RgbOut),
        };
        if inject == Tap::RgbIn && collect == Tap::RgbOut {
            return self.process_full(image, backend, timings);
        }
        tracing::info!(inject = inject.name(), collect = collect.name(), "pipeline: start");
        let t = Instant::now();
        let color_ref = crate::color_reference::ColorReference::compute(
            &self.film,
            &self.print,
            &self.params,
            &self.print_illuminant,
            self.print_exposure_factor,
            self.preflash_raw,
        );
        record_stage_timing(&mut timings, "color_reference", t);
        self.run_from(image, inject, collect, backend, &color_ref, None, timings)
    }

    pub fn process(&self, image: ImageBuf, backend: &dyn ComputeBackend) -> Result<ImageBuf, String> {
        self.process_with_taps(image, backend, None, None)
    }

    fn process_full(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        mut timings: Option<&mut BTreeMap<String, f64>>,
    ) -> Result<ImageBuf, String> {
        if self.params.workflow.route == "input" {
            use spektrafilm_math::colorspace::{resolve, conversion_matrix, convert_rgb};
            let source = resolve(&self.params.io.input_color_space)?;
            let destination = resolve(&self.params.io.output_color_space)?;
            let matrix = conversion_matrix(source, destination);
            let mut output = image;
            output.data.par_chunks_exact_mut(3).for_each(|pixel| {
                let rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
                let rgb = convert_rgb(rgb, source, self.params.io.input_cctf_decoding, destination, self.params.io.output_cctf_encoding, &matrix);
                for channel in 0..3 { pixel[channel] = from_f64(rgb[channel]); }
            });
            return Ok(output);
        }
        let ae_ev = self.meter_autoexposure(&image);
        let image = self.apply_autoexposure(image, ae_ev);
        let t = Instant::now();
        let (working, pixel_size_um) = crate::resizing::crop_and_rescale(
            &image,
            &self.params.io,
            self.params.camera.film_format_mm,
        )?;
        print_stage_timing(stage_timings_enabled(), "resize", t);
        record_stage_timing(&mut timings, "resize", t);
        let working = match working {
            std::borrow::Cow::Owned(working) => working,
            std::borrow::Cow::Borrowed(_) => image,
        };
        if self.params.workflow.route.starts_with("input > convert-film") {
            return self.process_convert(working, pixel_size_um, backend);
        }
        self.run(working, pixel_size_um, 0.0, backend, timings)
    }

    fn process_convert(
        &self,
        image: ImageBuf,
        pixel_size_um: f64,
        backend: &dyn ComputeBackend,
    ) -> Result<ImageBuf, String> {
        let route = self.params.workflow.route.as_str();
        let print_after_convert = route == "input > convert-film > print > scan";
        let scan_minus_base = route == "input > convert-film > scan-minus-base";
        let film_density = stages::converting::process(&image, &self.film, &self.params)?;
        let color_ref = crate::color_reference::ColorReference::compute(
            &self.film,
            &self.print,
            &self.params,
            &self.print_illuminant,
            self.print_exposure_factor,
            self.preflash_raw,
        );
        if print_after_convert {
            let print_density = stages::printing::expose_calibrated(
                &film_density,
                &self.film,
                &self.print,
                &self.params,
                backend,
                &self.print_illuminant,
                self.print_exposure_factor,
                self.preflash_raw,
                color_ref.printing_exposure_correction,
                pixel_size_um,
            );
            let print_density = stages::printing::develop(
                &print_density,
                &self.print,
                &self.params,
                backend,
            )?;
            Ok(stages::scanning::process(
                &print_density,
                &self.print,
                &self.params,
                backend,
                &color_ref,
                &self.output_gamut,
            ))
        } else {
            let mut scan_params = self.params.clone();
            scan_params.io.scan_film = true;
            Ok(stages::scanning::scan_with_options(
                &film_density,
                &self.film,
                &scan_params,
                backend,
                &color_ref,
                &self.output_gamut,
                Some(&self.params.film_render.convert.scan_illuminant),
                !scan_minus_base,
            ))
        }
    }

    /// Shared CPU/GPU chain from the prepared working image (crop +
    /// upscale already applied). `pixel_size_um` is the pitch derived
    /// from the full input; `ae_ev` the EV metered on the full input.
    fn run(
        &self,
        image: ImageBuf,
        pixel_size_um: f64,
        ae_ev: f64,
        backend: &dyn ComputeBackend,
        mut timings: Option<&mut BTreeMap<String, f64>>,
    ) -> Result<ImageBuf, String> {
        let stage_timings = stage_timings_enabled();
        // Scanner B&W/slide exposure correction (no-op unless scanner
        // white/black correction is on for a slide or print scan). Computed
        // up front so both the GPU-resident and per-stage paths share it.
        let t = Instant::now();
        let color_ref = crate::color_reference::ColorReference::compute(
            &self.film,
            &self.print,
            &self.params,
            &self.print_illuminant,
            self.print_exposure_factor,
            self.preflash_raw,
        );
        print_stage_timing(stage_timings, "color_reference", t);
        record_stage_timing(&mut timings, "color_reference", t);

        // GPU fast path: dispatch the whole filming→printing→scanning chain
        // (or filming→scanning when scan_film) as a single GPU command
        // buffer — one upload + one readback total. WGSL runs the
        // extended resident chain, including camera/scanner lens blur and
        // highlight boost. Unsupported effects select the faithful per-stage
        // path; backends without resident support return `None`.
        if self.tc_lut.is_some() || self.mallett_core.is_some() {
            if let Some(out) =
                self.try_gpu_resident(&image, backend, &color_ref, pixel_size_um, ae_ev)
            {
                tracing::info!("pipeline: gpu-resident fast path complete");
                return Ok(self.apply_post_scan(out));
            }
        }

        // Per-stage path — the same topology `process_with_taps` walks, from
        // the rescaled working image (rgb_pre) to rgb_out.
        self.run_from(
            image,
            Tap::RgbPre,
            Tap::RgbOut,
            backend,
            &color_ref,
            Some((pixel_size_um, ae_ev)),
            timings,
        )
    }

    /// Run only the GPU-resident path from a borrowed image. This is used by
    /// the live GUI, so a full-resolution preview does not need to
    /// deep-clone the loaded image just to satisfy `process(ImageBuf)`.
    ///
    /// Working geometry (crop + upscale, `crop_and_rescale`) is applied
    /// here from the borrow; with neither active the image stays
    /// zero-copy. Auto-exposure is metered on the full input exactly as
    /// [`Self::process`] does.
    ///
    /// `Ok(None)` means the backend cannot run the resident chain and the
    /// caller should fall back to [`Self::process`]; `Err` carries an
    /// invalid-geometry error that must fail the render.
    pub fn process_resident_borrowed(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<Option<ImageBuf>, String> {
        let inject = self.params.taps.inject.as_deref().map(Tap::parse)
            .transpose()?.unwrap_or(Tap::RgbIn);
        let collect = self.params.taps.collect.as_deref().map(Tap::parse)
            .transpose()?.unwrap_or(Tap::RgbOut);
        if inject != Tap::RgbIn || collect != Tap::RgbOut {
            return Ok(None);
        }
        if self.tc_lut.is_none() && self.mallett_core.is_none() {
            return Ok(None);
        }
        tracing::info!(backend = backend.name(), "pipeline: borrowed resident start");
        let ae_ev = self.meter_autoexposure(image);
        let exposed;
        let image = if self.params.camera.auto_exposure {
            exposed = self.apply_autoexposure(image.clone(), ae_ev);
            &exposed
        } else { image };
        let (working, pixel_size_um) = crate::resizing::crop_and_rescale(
            image,
            &self.params.io,
            self.params.camera.film_format_mm,
        )?;
        let color_ref = crate::color_reference::ColorReference::compute(
            &self.film,
            &self.print,
            &self.params,
            &self.print_illuminant,
            self.print_exposure_factor,
            self.preflash_raw,
        );
        let out = match self.try_gpu_resident(&working, backend, &color_ref, pixel_size_um, 0.0) {
            Some(out) => out,
            None => return Ok(None),
        };
        Ok(Some(self.apply_post_scan(out)))
    }
    fn apply_autoexposure(&self, mut image: ImageBuf, ev: f64) -> ImageBuf {
        if self.params.camera.auto_exposure {
            let scale = from_f64(2.0f64.powf(ev));
            image.data.par_iter_mut().for_each(|value| *value *= scale);
        }
        image
    }

    /// Meter the auto-exposure EV on the given (full, pre-geometry)
    /// image when auto-exposure is enabled.
    fn meter_autoexposure(&self, image: &ImageBuf) -> f64 {
        if !self.params.camera.auto_exposure {
            return 0.0;
        }
        stages::filming::meter_autoexposure_ev(image, &self.params)
    }

    fn resident_decision(&self) -> ResidentDecision {
        let mut reasons = Vec::new();
        if !matches!(self.params.workflow.route.as_str(), "input > film > scan" | "input > film > print > scan") {
            reasons.push(ResidentFallbackReason::WorkflowRoute);
        }
        let dir = &self.params.film_render.dir_couplers;
        let coefficients = if self.film.is_positive() { &dir.langmuir_receiver_k_rgb } else { &dir.langmuir_donor_k_rgb };
        if dir.active && coefficients.iter().any(|k| k.is_finite()) {
            reasons.push(ResidentFallbackReason::LangmuirChemistry);
        }
        if self.params.io.input_cctf_decoding {
            reasons.push(ResidentFallbackReason::InputTransferDecoding);
        }
        if self.params.settings.use_scanner_lut
            || (!self.params.io.scan_film && self.params.settings.use_enlarger_lut)
        {
            reasons.push(ResidentFallbackReason::RequestedSpectralLut);
        }
        let diffusion_effective = |df: &crate::params::DiffusionFilterParams| {
            df.active && df.strength > 0.0 && df.spatial_scale > 0.0
        };
        if diffusion_effective(&self.params.camera.diffusion_filter)
            || (!self.params.io.scan_film
                && diffusion_effective(&self.params.enlarger.diffusion_filter))
        {
            reasons.push(ResidentFallbackReason::ActiveOpticalDiffusion);
        }
        if self.params.film_render.grain.active {
            reasons.push(ResidentFallbackReason::FaithfulGrainDistribution);
        }
        if self.output_gamut.is_active() && self.output_gamut.gpu_params().is_none() {
            reasons.push(ResidentFallbackReason::UnsupportedOutputGamut);
        }
        if self.tc_lut.is_none() && self.mallett_core.is_none() {
            reasons.push(ResidentFallbackReason::MissingResidentFrontPass);
        }
        ResidentDecision::reasons(reasons)
    }

    /// Try the GPU-resident fast path. Builds all the per-stage data and
    /// hands it to the backend's `try_run_film_chain`. The output is unclipped
    /// linear destination RGB; `apply_post_scan` performs optional encoding.
    /// Pitch and metered EV come from the complete input before crop/rescale.
    fn try_gpu_resident(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
        color_ref: &crate::color_reference::ColorReference,
        pixel_size_um: f64,
        ae_ev: f64,
    ) -> Option<ImageBuf> {
        match self.resident_decision() {
            ResidentDecision::UseResident => {}
            ResidentDecision::PerStage { reasons } => {
                tracing::info!(
                    backend = backend.name(),
                    execution = "per_stage_cpu",
                    fallback_reasons = ?reasons,
                    "using per-stage path because resident GPU execution is unavailable"
                );
                return None;
            }
        }
        // Bake the exposure scale (auto-exposure × manual EV compensation)
        // into the front-pass matrix. Both upsamplers (hanatos and mallett)
        // are homogeneous in the input RGB, so scaling the matrix is
        // equivalent to scaling the input — saves a separate "scale" compute
        // pass at the head of the chain. Auto-exposure metering itself stays
        // on CPU (~30 ms at 6 MP after the per-row rayon parallelization);
        // the result is a single float that's cheap to roll into the matrix.
        let mut exposure_scale_f64 = 1.0f64;
        if self.params.camera.auto_exposure {
            exposure_scale_f64 *= 2.0f64.powf(ae_ev);
        }
        if self.params.camera.exposure_compensation_ev != 0.0 {
            exposure_scale_f64 *= 2.0f64.powf(self.params.camera.exposure_compensation_ev as f64);
        }
        // B&W/slide filming exposure correction is a linear scale on the raw
        // film exposure (CPU: `raw *= factor` before log10). Since both
        // upsamplers are homogeneous in the input RGB, folding it into the
        // exposure scale is equivalent. 1.0 (no-op) on every path except a
        // corrected slide scan.
        exposure_scale_f64 *= color_ref.filming_exposure_correction;
        let fold_exposure = |m: &mut [[f64; 3]; 3]| {
            if (exposure_scale_f64 - 1.0).abs() > 1e-9 {
                for row in m.iter_mut() {
                    for v in row.iter_mut() {
                        *v *= exposure_scale_f64;
                    }
                }
            }
        };

        // Front pass: hanatos TC LUT lookup, or the mallett 3×3 matmul
        // (`core · M_cs`, same fold as the CPU `expose` dispatch).
        let front = if let Some(tc_lut) = self.tc_lut.as_ref() {
            let ref_illuminant = select_illuminant(&self.front_illuminant);
            let mut rgb_to_adapted = spektrafilm_math::spectral::build_rgb_to_adapted_xyz(
                &self.params.io.input_color_space,
                &ref_illuminant,
                self.params.settings.use_cat16,
            );
            fold_exposure(&mut rgb_to_adapted);
            spektrafilm_gpu::FrontPass::Hanatos2025 {
                tc_lut,
                rgb_to_adapted_xyz: rgb_to_adapted,
            }
        } else {
            let core = self.mallett_core.as_ref()?;
            let mut matrix = crate::mallett::film_matrix(core, &self.params.io.input_color_space);
            fold_exposure(&mut matrix);
            spektrafilm_gpu::FrontPass::Mallett2019 { matrix }
        };

        // Film density curves: normalized (filming.develop subtracts nanmin).
        let film_log_exp = self.film.log_exposure_f64();
        let film_curves = self.film.density_curves_f64();
        let film_curves_norm =
            spektrafilm_model::density_curves::normalize_density_curves_f64(&film_curves);
        let film_channel_density: Vec<[f64; 3]> = self
            .film
            .data
            .channel_density
            .iter()
            .map(|r| {
                [
                    r.first().copied().unwrap_or(0.0),
                    r.get(1).copied().unwrap_or(0.0),
                    r.get(2).copied().unwrap_or(0.0),
                ]
            })
            .collect();
        let film_base_density = self.film.data.base_density.clone();

        // Print sensitivity (10**log_sensitivity, NaN→0).
        let print_sens: Vec<[f64; 3]> = self
            .print
            .log_sensitivity_f64()
            .iter()
            .map(|row| {
                let mut o = [0.0; 3];
                for c in 0..3 {
                    let v = 10f64.powf(row[c]);
                    o[c] = if v.is_nan() { 0.0 } else { v };
                }
                o
            })
            .collect();
        let print_log_exp = self.print.log_exposure_f64();
        // Print density curves: model-evaluated at gamma 1 whenever the
        // profile carries a fitted `density_curves_model` (identity when the
        // morph is inactive, morphed when active — matching the CPU
        // `develop_print_morph` path); stored RAW curves at the configured
        // gamma only for model-less profiles. Computed once on CPU here so
        // the resident print density pass needs no shader change.
        let morph = &self.params.print_render.density_curves_morph;
        let (print_curves, print_gamma_eff) = match self.print.data.density_curves_model.as_ref() {
            Some(model) => (
                crate::print_morph::morph_density_curves(
                    &print_log_exp,
                    model,
                    morph,
                    self.print.is_positive(),
                )
                .expect("print density-curve model was validated at pipeline construction"),
                1.0,
            ),
            None => (
                self.print.density_curves_f64(),
                self.params.print_render.density_curve_gamma as f64,
            ),
        };
        let print_channel_density: Vec<[f64; 3]> = self
            .print
            .data
            .channel_density
            .iter()
            .map(|r| {
                [
                    r.first().copied().unwrap_or(0.0),
                    r.get(1).copied().unwrap_or(0.0),
                    r.get(2).copied().unwrap_or(0.0),
                ]
            })
            .collect();
        let print_base_density = self.print.data.base_density.clone();

        // Scanning: viewing illuminant + normalization + combined XYZ→RGB
        // matrix. For scan_film we scan the developed film directly, so the
        // viewing illuminant and dye-density wavelength count come from the
        // film, not the print (mirrors the CPU scanning stage, which is
        // handed `self.film` as its profile when scan_film).
        let scan_profile = if self.params.io.scan_film {
            &self.film
        } else {
            &self.print
        };
        let viewing_illu = crate::spectral_service::select_illuminant(
            &scan_profile.info.viewing_illuminant,
        );
        let viewing_illu: Vec<f64> = viewing_illu.iter().map(|&v| v as f64).collect();
        let scan_channel_density_len = if self.params.io.scan_film {
            film_channel_density.len()
        } else {
            print_channel_density.len()
        };
        let scan_context = crate::chain_prep::ScanColorContext::build(
            viewing_illu,
            scan_channel_density_len,
            &self.params.io.output_color_space,
        );
        let viewing_illu: &[f64] = &scan_context.illuminant;
        let scan_norm = scan_context.normalization;
        let adapt = scan_context.adapt;
        let base_xyz_to_rgb = scan_context.base_xyz_to_rgb;
        let mut scan_xyz_to_rgb = [[0.0f64; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                scan_xyz_to_rgb[i][j] = base_xyz_to_rgb[i][0] * adapt[0][j]
                    + base_xyz_to_rgb[i][1] * adapt[1][j]
                    + base_xyz_to_rgb[i][2] * adapt[2][j];
            }
        }

        // GPU shaders take f32 — narrow the working-geometry pitch once.
        let pix_um = pixel_size_um as f32;

        // print_exposure_factor × print_exposure, with the B&W printing
        // exposure correction folded in (CPU applies it as a further raw
        // multiply; print_spectral applies the whole product as one
        // normalization). 1.0 except on a corrected print scan.
        let print_exposure_scale =
            self.params.enlarger.print_exposure as f64 * color_ref.printing_exposure_correction;
        let print_norm_factor = self.print_exposure_factor * print_exposure_scale;

        // Halation in the resident chain — only built when the halation
        // stage is active. Mirrors `apply_halation_um`: averages the
        // per-channel µm sigmas, converts to pixel space, and passes the
        // resulting scalars to the shaders.
        let halation = if self.params.film_render.halation.active {
            let h = &self.params.film_render.halation;
            // GPU shaders take f32 — narrow at the boundary.
            let avg_f64 = |a: [f64; 3]| (a[0] + a[1] + a[2]) / 3.0;
            let strength_avg = (avg_f64(h.halation_strength) * h.halation_amount) as f32;
            let a_tot = [
                (h.halation_strength[0] * h.halation_amount) as f32,
                (h.halation_strength[1] * h.halation_amount) as f32,
                (h.halation_strength[2] * h.halation_amount) as f32,
            ];
            Some(spektrafilm_gpu::HalationGpuParams {
                scatter_amount: h.scatter_amount as f32,
                scatter_core_px: (avg_f64(h.scatter_core_um) * h.scatter_spatial_scale
                    / pix_um as f64) as f32,
                scatter_tail_px: (avg_f64(h.scatter_tail_um) * h.scatter_spatial_scale
                    / pix_um as f64) as f32,
                scatter_tail_weight: [
                    h.scatter_tail_weight[0] as f32,
                    h.scatter_tail_weight[1] as f32,
                    h.scatter_tail_weight[2] as f32,
                ],
                halation_amount: h.halation_amount as f32,
                halation_strength_avg: strength_avg,
                halation_a_tot: a_tot,
                halation_first_sigma_px: (avg_f64(h.halation_first_sigma_um)
                    * h.halation_spatial_scale
                    / pix_um as f64) as f32,
                halation_n_bounces: h.halation_n_bounces,
                halation_bounce_decay: h.halation_bounce_decay as f32,
                halation_renormalize: h.halation_renormalize,
            })
        } else {
            None
        };

        // DIR couplers in the resident chain. Mirrors CPU
        // `apply_density_correction`: build the scaled couplers matrix and
        // pre-compute the "density curves before DIR" once. The shader
        // re-interpolates these against `log_raw - correction`.
        // Held in this binding so `&density_curves_0_f64` outlives the
        // backend call.
        let dir_inputs = if self.params.film_render.dir_couplers.active {
            let dir = &self.params.film_render.dir_couplers;
            let matrix = spektrafilm_model::couplers::compute_dir_couplers_matrix(
                dir.gamma_samelayer_rgb,
                dir.gamma_interlayer_r_to_gb,
                dir.gamma_interlayer_g_to_rb,
                dir.gamma_interlayer_b_to_rg,
                dir.inhibition_samelayer,
                dir.inhibition_interlayer,
            );
            let prepared = spektrafilm_model::couplers::prepare_dir(
                &self.film.density_curves_f64(),
                &self.film.log_exposure_f64(),
                &matrix,
                dir.amount,
                self.film.is_positive(),
            );
            Some((
                prepared,
                pixel_size_um,
                dir.diffusion_size_um,
                dir.diffusion_tail_um,
                dir.diffusion_tail_weight,
                self.film.is_positive(),
                self.params.film_render.density_curve_gamma as f64,
            ))
        } else {
            None
        };
        let dir_couplers = dir_inputs.as_ref().map(|d| {
            // GPU shader path is f32 — narrow at the boundary.
            let m = d.0.matrix_scaled;
            let matrix_f32: [[f32; 3]; 3] = [
                [m[0][0] as f32, m[0][1] as f32, m[0][2] as f32],
                [m[1][0] as f32, m[1][1] as f32, m[1][2] as f32],
                [m[2][0] as f32, m[2][1] as f32, m[2][2] as f32],
            ];
            let dm = d.0.density_max;
            spektrafilm_gpu::DirCouplersGpuParams {
                couplers_matrix_scaled: matrix_f32,
                density_max: [dm[0] as f32, dm[1] as f32, dm[2] as f32],
                is_positive: d.5,
                diffusion_size_px: (d.2 / d.1) as f32,
                diffusion_tail_px: (d.3 / d.1) as f32,
                diffusion_tail_weight: d.4 as f32,
                density_curves_0: &d.0.curves_0,
                log_exposure: &film_log_exp,
                gamma_factor: d.6,
            }
        });


        // Glare in the resident chain — applied after scan_spectral on the
        // final RGB buffer. Mirrors the CPU lognormal + blur + add. Python
        // 0.3.4 disables viewing glare entirely on the `io.scan_film` path
        // (`glare = None`; `film_render.glare` is never read upstream) —
        // only `print_render.glare` reaches the print scan.
        let glare = (!self.params.io.scan_film)
            .then(|| &self.params.print_render.glare)
            .filter(|g| g.active && g.percent > 0.0)
            .map(|g| {
                // LogNormal parameters shared with `compute_random_glare_amount`.
                let (mu, sigma) = spektrafilm_model::glare::lognormal_params(g.percent, g.roughness);
                // glare_rgb_offset = (XYZ→RGB) · illuminant_xyz / 100.
                let glare_rgb_offset =
                    crate::chain_prep::glare_rgb_offset_f64(&scan_context);
                let offset_rgb = [
                    (glare_rgb_offset[0] / 100.0) as f32,
                    (glare_rgb_offset[1] / 100.0) as f32,
                    (glare_rgb_offset[2] / 100.0) as f32,
                ];
                spektrafilm_gpu::GlareGpuParams {
                    mu: mu as f32,
                    sigma: sigma as f32,
                    blur_px: g.blur,
                    base_seed: self.params.random_seed.wrapping_add(42) as u32,
                    rgb_offset: offset_rgb,
                }
            });

        // Unsharp mask: scanner.unsharp_mask = [sigma, amount].
        let [usm_sigma, usm_amount] = self.params.scanner.unsharp_mask;
        let unsharp = if usm_sigma > 0.0 && usm_amount > 0.0 {
            Some(spektrafilm_gpu::UnsharpGpuParams {
                sigma_px: usm_sigma as f32,
                amount: usm_amount as f32,
            })
        } else {
            None
        };

        let camera_lens_blur_px = if self.params.camera.lens_blur_um > 0.0 {
            Some(self.params.camera.lens_blur_um / pix_um)
        } else {
            None
        };

        let scanner_lens_blur_px = if self.params.scanner.lens_blur > 0.0 {
            Some(self.params.scanner.lens_blur)
        } else {
            None
        };

        let hboost = &self.params.film_render.halation;
        let highlight_boost = if hboost.boost_ev != 0.0 {
            Some(spektrafilm_gpu::HighlightBoostGpuParams {
                boost_ev: hboost.boost_ev as f32,
                boost_range: hboost.boost_range as f32,
                protect_ev: hboost.protect_ev as f32,
            })
        } else {
            None
        };

        let params = spektrafilm_gpu::FilmChainParams {
            image,
            front,
            film_log_exposure: &film_log_exp,
            film_density_curves_normalized: &film_curves_norm,
            film_gamma: self.params.film_render.density_curve_gamma as f64,
            film_channel_density: &film_channel_density,
            film_base_density: &film_base_density,
            print_illuminant: &self.print_illuminant,
            print_sensitivity: &print_sens,
            print_normalization_factor: print_norm_factor,
            print_log_exposure: &print_log_exp,
            print_density_curves: &print_curves,
            print_gamma: print_gamma_eff,
            print_channel_density: &print_channel_density,
            print_base_density: &print_base_density,
            preflash: self.preflash_raw,
            viewing_illuminant: &viewing_illu,
            scan_normalization: scan_norm,
            scan_xyz_to_rgb: &scan_xyz_to_rgb,
            bw_xyz_remap: color_ref.xyz_remap(),
            scan_film: self.params.io.scan_film,
            halation,
            dir_couplers,
            glare,
            gamut: self.output_gamut.gpu_params(),
            unsharp,
            camera_lens_blur_px,
            scanner_lens_blur_px,
            highlight_boost,
        };
        if !params.gpu_blurs_supported() {
            tracing::info!(
                backend = backend.name(),
                execution = "per_stage_cpu",
                fallback_reasons = ?[ResidentFallbackReason::BlurRadiusExceedsBackendSupport],
                "using per-stage path because a resident FIR blur exceeds backend support"
            );
            return None;
        }
        backend.try_run_film_chain(&params)
    }

    /// Apply the shared destination encoding without clipping floating output.
    fn apply_post_scan(&self, mut rgb: ImageBuf) -> ImageBuf {
        use rayon::prelude::*;
        use spektrafilm_math::precision::from_f64;
        if self.params.io.output_cctf_encoding {
            let space = spektrafilm_math::colorspace::resolve(&self.params.io.output_color_space)
                .expect("validated output colour space");
            rgb.data.par_chunks_exact_mut(3).for_each(|px| {
                let out = spektrafilm_math::colorspace::encode_rgb(
                    [px[0] as f64, px[1] as f64, px[2] as f64], space,
                );
                for c in 0..3 { px[c] = from_f64(out[c]); }
            });
        }
        rgb
    }
}

#[cfg(test)]
mod spectral_invalidation_tests {
    use super::*;

    fn data_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data")
    }

    #[test]
    fn film_chemistry_rebuilds_total_and_grain_layers_from_the_fitted_model() {
        let mut film = crate::profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
        let fitted = film.data.density_curves.clone();
        let fitted_layers = film.data.density_curves_layers.clone();
        for active in [false, true] {
            film.data.density_curves.iter_mut().flatten().for_each(|v| *v = 0.5);
            film.data.density_curves_layers.iter_mut().flatten().flatten().for_each(|v| *v = 0.1);
            apply_film_chemistry(&mut film, &crate::params::PrintCurvesMorphParams {
                active, ..Default::default()
            }).unwrap();
            for (got, want) in film.data.density_curves.iter().flatten().zip(fitted.iter().flatten()) {
                assert!((got - want).abs() < 1e-12);
            }
            for (got, want) in film.data.density_curves_layers.iter().flatten().flatten()
                .zip(fitted_layers.iter().flatten().flatten()) {
                assert!((got - want).abs() < 1e-12);
            }
        }
        apply_film_chemistry(&mut film, &crate::params::PrintCurvesMorphParams {
            gamma_factor: 1.1, gamma_factor_fast: 0.9, gamma_factor_slow: 1.2,
            developer_exhaustion: 0.3, ..Default::default()
        }).unwrap();
        // Pinned 28bf883e apply_print_curves_morph_with_layers, Portra 400 sample 128.
        let expected = [
            [0.5911804126249434, 0.5581332496586162, 0.6521190325040326],
            [0.4509958575813556, 0.4580461526112701, 0.5776553146563388],
            [0.017750303385221305, 0.026894456204068887, 0.034051265456119285],
        ];
        for channel in 0..3 {
            for layer in 0..3 {
                assert!((film.data.density_curves_layers[128][layer][channel] - expected[layer][channel]).abs() < 1e-9);
            }
            let sum: f64 = expected.iter().map(|layer| layer[channel]).sum();
            assert!((film.data.density_curves[128][channel] - sum).abs() < 1e-9);
        }
    }

    fn build() -> Pipeline {
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        Pipeline::new_with_spectral(film, print, RuntimeParams::default(), &dir).unwrap()
    }

    fn lut_data(pipeline: &Pipeline) -> Vec<f64> {
        pipeline
            .tc_lut()
            .expect("spectral pipeline has a LUT")
            .data
            .clone()
    }

    /// Changing any sensitivity-affecting spectral control must rebuild the
    /// TC LUT (and its calibration) instead of reusing the cached one — the
    /// Rust counterpart of the `SpectralLUTService` invalidation keyed on
    /// `Hanatos2025SensitivityAdaptation` + filtered sensitivity.
    #[test]
    fn with_params_rebuilds_lut_on_spectral_change() {
        let base = build();
        let base_lut = lut_data(&base);

        let change = |mutate: fn(&mut RuntimeParams)| {
            let mut params = RuntimeParams::default();
            mutate(&mut params);
            params
        };

        let cases: Vec<(&str, RuntimeParams)> = vec![
            (
                "spectral_gaussian_blur",
                change(|p| p.settings.spectral_gaussian_blur = 8.0),
            ),
            (
                "apply_hanatos2025_adaptation_window",
                change(|p| p.settings.apply_hanatos2025_adaptation_window = false),
            ),
            (
                "apply_hanatos2025_adaptation_surface",
                change(|p| p.settings.apply_hanatos2025_adaptation_surface = true),
            ),
            (
                "camera.filter_uv",
                change(|p| p.camera.filter_uv = [0.5, 415.0, 9.0]),
            ),
            (
                "camera.filter_ir",
                change(|p| p.camera.filter_ir = [0.7, 670.0, 18.0]),
            ),
        ];

        for (name, params) in cases {
            let rebuilt = base.clone().with_params(params).unwrap();
            let lut = lut_data(&rebuilt);
            assert!(
                base_lut.iter().zip(&lut).any(|(a, b)| a != b),
                "{name}: TC LUT was reused after the spectral control changed"
            );
        }
    }

    /// Non-spectral changes keep the calibrated LUT — `with_params` must not
    /// rebuild (and must not corrupt) the front-end for ordinary tweaks.
    #[test]
    fn with_params_reuses_lut_on_non_spectral_change() {
        let base = build();
        let base_lut = lut_data(&base);

        let mut params = RuntimeParams::default();
        params.scanner.lens_blur = 0.7;
        params.film_render.halation.boost_ev = 0.5;
        let tweaked = base.clone().with_params(params).unwrap();
        assert_eq!(
            lut_data(&tweaked),
            base_lut,
            "non-spectral change must not rebuild the TC LUT"
        );
    }
    #[test]
    fn with_params_rejects_failed_calibration_rebuild() {
        let base = build();
        let mut params = base.params.clone();
        params.settings.rgb_to_raw_method = "unsupported-test-method".into();
        let error = match base.with_params(params) {
            Ok(_) => panic!("invalid rebuild must not keep stale calibration"),
            Err(error) => error,
        };
        assert!(error.contains("unsupported method"), "{error}");
    }
    #[test]
    fn with_params_rejects_invalid_print_morph_update() {
        let base = build();
        let mut params = base.params.clone();
        params.print_render.density_curves_morph.active = true;
        params.print_render.density_curves_morph.gamma_factor = 0.0;
        let error = match base.with_params(params) {
            Ok(_) => panic!("invalid print morph update must be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("gamma_factor must be strictly positive"), "{error}");
    }



    #[test]
    fn stock_edits_survive_construction_updates_and_spectral_rebuild() {
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "fujifilm_velvia_100").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let seeded = crate::params_builder::digest_params(RuntimeParams::default(), &film, &print, None, true);
        let mut edited = seeded;
        edited.film_render.halation.halation_strength = [0.3, 0.2, 0.1];
        edited.film_render.halation.halation_first_sigma_um = [13.0, 17.0, 23.0];
        edited.film_render.dir_couplers.gamma_samelayer_rgb = [0.9, 0.8, 0.7];
        edited.film_render.dir_couplers.gamma_interlayer_r_to_gb = [0.4, 0.3];
        let params = crate::params_builder::digest_params(edited, &film, &print, None, false);
        let mut pipeline = Pipeline::new_with_spectral(film, print, params.clone(), &dir).unwrap();
        assert_eq!(pipeline.params.film_render.halation.halation_strength, [0.3, 0.2, 0.1]);
        assert_eq!(pipeline.params.film_render.halation.halation_first_sigma_um, [13.0, 17.0, 23.0]);
        assert_eq!(pipeline.params.film_render.dir_couplers.gamma_samelayer_rgb, [0.9, 0.8, 0.7]);
        for rebuild in [false, true] {
            let mut updated = params.clone();
            if rebuild { updated.settings.spectral_gaussian_blur = 8.0; }
            pipeline = pipeline.with_params(updated).unwrap();
            assert_eq!(pipeline.params.film_render.halation.halation_strength, [0.3, 0.2, 0.1]);
            assert_eq!(pipeline.params.film_render.halation.halation_first_sigma_um, [13.0, 17.0, 23.0]);
            assert_eq!(pipeline.params.film_render.dir_couplers.gamma_samelayer_rgb, [0.9, 0.8, 0.7]);
            assert_eq!(pipeline.params.film_render.dir_couplers.gamma_interlayer_r_to_gb, [0.4, 0.3]);
        }
        let mut deactivated = params;
        deactivated.debug.deactivate_spatial_effects = true;
        let deactivated = crate::params_builder::digest_params(deactivated, &pipeline.film, &pipeline.print, None, false);
        let pipeline = pipeline.with_params(deactivated).unwrap();
        assert_eq!(pipeline.params.film_render.halation.halation_first_sigma_um, [0.0; 3]);
    }

    #[test]
    fn with_params_recalibrates_print_exposure_for_camera_ev() {
        let dir = data_dir();
        let base = build();
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let image = ImageBuf::from_data(8, 8, vec![from_f64(0.184); 8 * 8 * 3]);

        for ev in [-2.0f32, -1.0, 0.0, 1.0, 2.0] {
            let mut params = base.params.clone();
            params.camera.exposure_compensation_ev = ev;
            let updated = base.clone().with_params(params.clone()).unwrap();
            let fresh =
                Pipeline::new_with_spectral(base.film.clone(), base.print.clone(), params, &dir)
                    .unwrap();

            assert!(
                (updated.print_exposure_factor() - fresh.print_exposure_factor()).abs() < 1e-12,
                "EV {ev}: stale print exposure factor"
            );
            let updated_output = updated.process(image.clone(), &backend).unwrap();
            let fresh_output = fresh.process(image.clone(), &backend).unwrap();
            assert_eq!(updated_output.data.len(), fresh_output.data.len());
            for (actual, expected) in updated_output.data.iter().zip(&fresh_output.data) {
                assert!(
                    (*actual as f64 - *expected as f64).abs() < 1e-5,
                    "EV {ev}: output mismatch ({actual} vs {expected})"
                );
            }
        }
    }

    #[test]
    fn input_gamut_active_toggle_matches_fresh_lut() {
        let base = build();
        let mut updated = base.clone();
        for active in [false, true] {
            let mut params = base.params.clone();
            params.io.input_gamut_compress.active = active;
            let rebuilt = updated.with_params(params.clone()).unwrap();
            let fresh = Pipeline::new_with_spectral(base.film.clone(), base.print.clone(), params, &data_dir()).unwrap();
            assert_eq!(lut_data(&rebuilt), lut_data(&fresh));
            assert_eq!(rebuilt.print_exposure_factor(), fresh.print_exposure_factor());
            if !active { assert_ne!(lut_data(&rebuilt), lut_data(&base)); }
            updated = rebuilt;
        }
    }

    #[test]
    fn persistent_taps_and_explicit_precedence_follow_topology() {
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.debug.lut_mode = true;
        params.io.crop = true;
        params.io.crop_size = [0.5, 0.5];
        params.taps.inject = Some("log_e_film".into());
        params.taps.collect = Some("cmy_film".into());
        let params = crate::params_builder::digest_params(params, &film, &print, None, true);
        let pipeline = Pipeline::new(film, print, params);
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let image = ImageBuf::from_data(8, 8, vec![from_f64(-1.0); 8 * 8 * 3]);
        let expected = pipeline.process_with_taps(image.clone(), &backend, Some(Tap::LogEFilm), Some(Tap::CmyFilm)).unwrap();
        assert_eq!(pipeline.process(image.clone(), &backend).unwrap().data, expected.data);
        assert_eq!(pipeline.process_with_taps(image.clone(), &backend, None, None).unwrap().data, expected.data);
        let unchanged = pipeline.process_with_taps(image.clone(), &backend, None, Some(Tap::LogEFilm)).unwrap();
        assert_eq!(unchanged.data, image.data);
        let unchanged = pipeline.process_with_taps(image.clone(), &backend, Some(Tap::CmyFilm), None).unwrap();
        assert_eq!(unchanged.data, image.data);
        assert_eq!((expected.width, expected.height), (8, 8));
        assert!(pipeline.process_resident_borrowed(&image, &backend).unwrap().is_none());
        let image = ImageBuf::from_data(8, 8, vec![from_f64(0.184); 8 * 8 * 3]);
        let normal = pipeline.process_with_taps(image.clone(), &backend, Some(Tap::RgbIn), Some(Tap::RgbOut)).unwrap();
        // `digest_params(..., lut_mode = true)` disables crop so a LUT bake
        // remains a static point transform.
        assert_eq!((normal.width, normal.height), (8, 8));
        let mut params = pipeline.params.clone();
        params.taps = Default::default();
        let default = pipeline.with_params(params).unwrap().process(image, &backend).unwrap();
        assert_eq!(normal.data, default.data);
    }
}
