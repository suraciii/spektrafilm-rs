/// 3-stage simulation pipeline: Filming → Printing → Scanning.
///
/// Full calibration chain:
///   1. Load spectra LUT → compute TC LUT for film sensitivity
///   2. Process virtual gray card through filming to get midgray spectral density
///   3. Compute print exposure normalization factor from midgray spectral density
///   4. Pass all calibration data to the pipeline stages
use std::path::Path;
use std::time::Instant;

use rayon::prelude::*;
use spektrafilm_math::precision::from_f64;
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

use crate::enlarger;
use crate::params::{RuntimeParams, Tap};
use crate::profile::Profile;
use crate::spectral_service;
use crate::stages;

fn apply_film_specific_params(film: &Profile, params: &mut RuntimeParams) {
    // Stock-specific overrides (DIR couplers, halation preset, slide-film
    // retunes) + the B&W engine-layout broadcast. Kept on the construction
    // and rebuild paths so a caller that hands over undigested user params
    // still gets the stock behaviour; `params_builder::digest_params`
    // applies the same constants, so the double application is idempotent.
    crate::params_builder::apply_film_specifics(params, film);
    crate::params_builder::broadcast_monochrome_layout(film, params);
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
    /// For all other params the calibration is reused, so callers must keep
    /// the remaining calibration-affecting inputs (profiles, enlarger
    /// settings, exposure compensation) unchanged, as before.
    pub fn with_params(mut self, params: RuntimeParams) -> Self {
        let mut params = params;
        params.validate_color().expect("invalid colour configuration");
        apply_film_specific_params(&self.film, &mut params);
        if spectral_controls_key(&self.params) != spectral_controls_key(&params) {
            if let Some(data_dir) = self.data_dir.clone() {
                match Self::new_with_spectral(
                    self.film.clone(),
                    self.print.clone(),
                    params.clone(),
                    &data_dir,
                ) {
                    Ok(rebuilt) => return rebuilt,
                    // The same data_dir/profiles built successfully before;
                    // only a corrupted data directory can get here. Keep the
                    // old front-end and surface the failure rather than panic
                    // inside a GUI render path.
                    Err(e) => tracing::error!(
                        error = %e,
                        "spectral rebuild on settings change failed; keeping stale LUT"
                    ),
                }
            } else {
                // Pipeline without a spectral front-end (`Pipeline::new`):
                // nothing to invalidate.
            }
        }
        self.output_gamut = crate::gamut_compression::OutputGamutCompress::build(
            &params.io.output_gamut_compress,
            &params.io.output_color_space,
        ).expect("validated output gamut configuration");
        self.params = params;
        self
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
        input_gamut_algorithm: params.io.input_gamut_compress.algorithm.clone(),
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
    input_gamut_algorithm: String,
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
        let film = crate::profile::resolve_for_render(film, params.film_render.development_time);
        let print = crate::profile::resolve_for_render(print, params.print_render.development_time);
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
        // B&W profiles: collapse the development-time family to the selected
        // time and broadcast the single channel onto the 3-channel engine
        // layout. Must happen before anything reads the profile data.
        let film = crate::profile::resolve_for_render(film, params.film_render.development_time);
        let print = crate::profile::resolve_for_render(print, params.print_render.development_time);

        // Python parity: mirror `_apply_film_specifics` in
        // `spektrafilm/runtime/params_builder.py`. Python applies these
        // overrides inside `digest_params()` before the pipeline runs,
        // so a fresh `RuntimeParams::default()` does NOT match what
        // Python uses. The DIR-coupler gammas in particular differ
        // between positive and negative films.
        apply_film_specific_params(&film, &mut params);

        // Python parity: look up per-(print, illuminant, film) neutral filter values from
        // the JSON database — matches `apply_database_neutral_print_filters`. Defaults to
        // params.enlarger.{c,m,y}_filter_neutral when the combo isn't in the database.
        // Keep the f64 lookup values around (params is f32) — narrowing to
        // f32 here costs ~4e-8 precision through the `10^(-cc/100)` step.
        let mut neutral_cmy_f64: Option<[f64; 3]> = None;
        if params.settings.neutral_print_filters_from_database {
            let db = crate::neutral_filters::NeutralFilters::load(data_dir);
            let print_stock = print.info.stock.as_deref().unwrap_or("");
            let film_stock = film.info.stock.as_deref().unwrap_or("");
            if let Some([c, m, y]) = db.lookup(print_stock, &params.enlarger.illuminant, film_stock)
            {
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
                ref_illuminant_f64,
            );
        }
        // RGB → film raw upsampler. Default `hanatos2025` builds the spectral tc
        // LUT; `mallett2019` builds a 3×3 reflectance-basis matrix instead (no
        // LUT). Dispatch mirrors Python `_rgb_to_film_raw`.
        let (tc_lut, mallett_core): (Option<TcLut>, Option<[[f64; 3]; 3]>) =
            match params.settings.rgb_to_raw_method.as_str() {
                "hanatos2025" => {
                    let spectra_lut = spectral_service::load_spectra_lut(data_dir)?;
                    // Hanatos2025 sensitivity adaptation — Python
                    // `compute_hanatos2025_tc_lut`: spectral Gaussian blur of
                    // the spectra cube, erf4 UV/IR bandpass window with
                    // reference-illuminant normalization, and the poly4
                    // log-exposure-correction surface. The window/surface
                    // apply-flags and blur sigma come from the runtime
                    // settings; the parameters come from the film profile.
                    let window_params: Vec<f64> =
                        film.data.hanatos2025_adaptation_window_params.clone();
                    let adaptation = spectral_service::Hanatos2025Adaptation {
                        window_params: &window_params,
                        surface_params: &film.data.hanatos2025_adaptation_surface_params,
                        spectral_gaussian_blur: params.settings.spectral_gaussian_blur as f64,
                        reference_illuminant: ref_illuminant_f64,
                        reference_illuminant_xy: spektrafilm_math::spectral::illuminant_to_xy(
                            ref_illuminant,
                        ),
                        apply_window: params.settings.apply_hanatos2025_adaptation_window,
                        apply_surface: params.settings.apply_hanatos2025_adaptation_surface,
                    };
                    let tc_lut = spectral_service::compute_hanatos2025_tc_lut(
                        &spectra_lut,
                        &sensitivity,
                        &adaptation,
                    )?;

                    // Bake input gamut compression into the LUT at build time, around
                    // the film reference illuminant (the runtime's achromatic axis).
                    // The per-pixel path then stays compression-agnostic.
                    let input_gamut = crate::input_gamut::InputGamutCompress::build(
                        &params.io.input_gamut_compress,
                    )?;
                    let tc_lut = if input_gamut.is_active() {
                        let (rx, ry) = spektrafilm_math::spectral::illuminant_to_xy(ref_illuminant);
                        input_gamut.remap(&tc_lut, [rx, ry])
                    } else {
                        tc_lut
                    };
                    (Some(tc_lut), None)
                }
                "arctic2026alpha02" => {
                    front_illuminant = "D65".into();
                    let reflectance_lut = spectral_service::load_arctic2026alpha02_lut(data_dir)?;
                    let tc_lut = spectral_service::compute_reflectance_tc_lut(
                        &reflectance_lut,
                        &sensitivity,
                        ref_illuminant_f64,
                    );
                    let input_gamut = crate::input_gamut::InputGamutCompress::build(
                        &params.io.input_gamut_compress,
                    )?;
                    let tc_lut = if input_gamut.is_active() {
                        let (rx, ry) = spektrafilm_math::spectral::illuminant_to_xy(
                            select_illuminant(&front_illuminant),
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
                        ref_illuminant_f64,
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
                    select_illuminant(&front_illuminant),
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
    ) -> ImageBuf {
        let stage_timings = stage_timings_enabled();
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
                    select_illuminant(&self.front_illuminant),
                    color_ref.filming_exposure_correction,
                    pixel_size_um,
                    ae_ev,
                );
                print_stage_timing(stage_timings, "filming_expose", t);
                dump_if_env("SPEKTRAFILM_DUMP_FILM_LOG_RAW", &out);
                out
            }
            Tap::LogEFilm => {
                let t = Instant::now();
                let out = stages::filming::develop(&image, &self.film, &self.params, backend, pixel_size_um);
                print_stage_timing(stage_timings, "filming_develop", t);
                tracing::info!("pipeline: filming complete");
                dump_if_env("SPEKTRAFILM_DUMP_FILM_DENSITY", &out);
                out
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
                    print_stage_timing(stage_timings, "scanning", t);
                    tracing::info!("pipeline: scanning complete (film scan)");
                    out
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
                    print_stage_timing(stage_timings, "printing", t);
                    dump_if_env("SPEKTRAFILM_DUMP_PRINT_LOG_RAW", &out);
                    out
                }
            }
            Tap::LogEPrint => {
                let t = Instant::now();
                let out = stages::printing::develop(&image, &self.print, &self.params, backend);
                print_stage_timing(stage_timings, "printing_develop", t);
                dump_if_env("SPEKTRAFILM_DUMP_PRINT_DENSITY", &out);
                out
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
                print_stage_timing(stage_timings, "scanning", t);
                tracing::info!("pipeline: printing complete");
                out
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
        let mut i = ip;
        if inject == Tap::RgbIn && cp > ip { i += 1; }
        while i < cp {
            cur = self.fire_node(order[i], cur, backend, color_ref, pixel_size_um, ae_ev);
            i += 1;
        }
        Ok(cur)
    }

    /// Process an image with explicit pipeline taps — the Rust spelling of
    /// upstream `SimulationPipeline.process(image, inject, collect)`.
    ///
    /// `inject`/`collect` of `None` mean the defaults (`rgb_in` /
    /// `rgb_out`). Injecting at `log_e_film` or later bypasses the camera
    /// (and film) stages upstream of the injection point; collecting at
    /// `cmy_film` returns film densities instead of final RGB. Invalid or
    /// unreachable pairs fail with the upstream error text.
    ///
    /// The fused GPU-resident fast path is bypassed whenever a tap is set:
    /// taps need the per-stage boundaries, and the resident chain fuses
    /// them. Default taps keep [`Pipeline::process`] on the fast path.
    pub fn process_with_taps(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
    ) -> Result<ImageBuf, String> {
        let inject = inject.unwrap_or(Tap::RgbIn);
        let collect = collect.unwrap_or(Tap::RgbOut);
        tracing::info!(inject = inject.name(), collect = collect.name(), "pipeline: start");
        let color_ref = crate::color_reference::ColorReference::compute(
            &self.film,
            &self.print,
            &self.params,
            &self.print_illuminant,
            self.print_exposure_factor,
            self.preflash_raw,
        );
        self.run_from(image, inject, collect, backend, &color_ref, None)
    }

    pub fn process(&self, image: ImageBuf, backend: &dyn ComputeBackend) -> Result<ImageBuf, String> {
        let stage_timings = stage_timings_enabled();
        let ae_ev = self.meter_autoexposure(&image);
        let image = self.apply_autoexposure(image, ae_ev);
        let t = Instant::now();
        let (working, pixel_size_um) = crate::resizing::crop_and_rescale(
            &image,
            &self.params.io,
            self.params.camera.film_format_mm,
        )?;
        print_stage_timing(stage_timings, "resize", t);
        let working = match working {
            std::borrow::Cow::Owned(working) => working,
            std::borrow::Cow::Borrowed(_) => image,
        };
        self.run(working, pixel_size_um, 0.0, backend)
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

        // GPU fast path: dispatch the whole filming→printing→scanning chain
        // (or filming→scanning when scan_film) as a single GPU command
        // buffer — one upload + one readback total. CUDA and WGSL run the
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
        self.run_from(image, Tap::RgbPre, Tap::RgbOut, backend, &color_ref, Some((pixel_size_um, ae_ev)))
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
        if self.params.io.input_cctf_decoding
            || (self.output_gamut.is_active() && self.output_gamut.gpu_params().is_none())
        {
            return None;
        }
        if self.params.settings.use_scanner_lut
            || (!self.params.io.scan_film && self.params.settings.use_enlarger_lut)
        {
            tracing::info!(backend = backend.name(), stage = "spectral_lut", execution = "per_stage",
                "using per-stage path for requested PCHIP spectral LUT evaluation");
            return None;
        }
        // The Gaussian preview approximation does not reproduce the sampled,
        // finite, normalized PSF. Keep optical diffusion on the CPU FFT path.
        let diffusion_effective = |df: &crate::params::DiffusionFilterParams| {
            df.active && df.strength > 0.0 && df.spatial_scale > 0.0
        };
        if diffusion_effective(&self.params.camera.diffusion_filter)
            || (!self.params.io.scan_film
                && diffusion_effective(&self.params.enlarger.diffusion_filter))
        {
            tracing::info!(backend = backend.name(), stage = "diffusion", execution = "cpu_fft",
                "using per-stage path for faithful optical diffusion");
            return None;
        }
        // GPU composite grain always uses normal approximations. The binomial
        // low-variance regime depends on each pixel's developed density, so a
        // configuration-only gate cannot guarantee the exact distribution.
        // Keep both layered and composite grain on the faithful CPU sampler.
        if self.params.film_render.grain.active {
            tracing::info!(backend = backend.name(), stage = "grain", execution = "cpu",
                "using per-stage path for faithful grain distributions");
            return None;
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
                ref_illuminant,
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
            Some(model) => match crate::print_morph::morph_density_curves(
                &print_log_exp,
                model,
                morph,
                self.print.is_positive(),
            ) {
                Ok(curves) => (curves, 1.0),
                Err(e) => {
                    tracing::error!(error = %e, "print-curve model eval failed; using stored curves");
                    (
                        self.print.density_curves_f64(),
                        self.params.print_render.density_curve_gamma as f64,
                    )
                }
            },
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
        let viewing_illu_f32 = match scan_profile.info.viewing_illuminant.as_str() {
            "D50" => &spektrafilm_math::spectral::ILLUMINANT_D50[..],
            "D55" => &spektrafilm_math::spectral::ILLUMINANT_D55[..],
            "D65" => &spektrafilm_math::spectral::ILLUMINANT_D65[..],
            _ => &spektrafilm_math::spectral::ILLUMINANT_D50[..],
        };
        let viewing_illu: Vec<f64> = viewing_illu_f32.iter().map(|&v| v as f64).collect();
        let n_wl = if self.params.io.scan_film {
            film_channel_density.len()
        } else {
            print_channel_density.len()
        };
        let scan_norm: f64 = (0..n_wl)
            .map(|i| viewing_illu[i] * spektrafilm_math::spectral::CMF_Y_F64[i])
            .sum();
        let mut viewing_xyz = [0.0; 3];
        for i in 0..n_wl {
            viewing_xyz[0] += viewing_illu[i] * spektrafilm_math::spectral::CMF_X_F64[i];
            viewing_xyz[1] += viewing_illu[i] * spektrafilm_math::spectral::CMF_Y_F64[i];
            viewing_xyz[2] += viewing_illu[i] * spektrafilm_math::spectral::CMF_Z_F64[i];
        }
        let sum = viewing_xyz.iter().sum::<f64>();
        let x = viewing_xyz[0] / sum;
        let y = viewing_xyz[1] / sum;
        let viewing_white = [x / y, 1.0, (1.0 - x - y) / y];
        let output_white = spektrafilm_math::spectral::colorspace_white_xyz_f64(
            &self.params.io.output_color_space,
        );
        let adapt = spektrafilm_math::colorspace::chromatic_adaptation_matrix_f64(
            viewing_white,
            output_white,
        );
        let base_xyz_to_rgb = spektrafilm_math::colorspace::resolve(&self.params.io.output_color_space)
            .expect("validated output colour space").matrix_xyz_to_rgb;
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
            let mut matrix_scaled = matrix;
            for row in &mut matrix_scaled {
                for v in row.iter_mut() {
                    *v *= dir.amount;
                }
            }
            let norm_curves_f64 = spektrafilm_model::density_curves::normalize_density_curves_f64(
                &self.film.density_curves_f64(),
            );
            let density_curves_0_f64 = spektrafilm_model::couplers::compute_curves_before_dir(
                &norm_curves_f64,
                &self.film.log_exposure_f64(),
                &matrix_scaled,
                self.film.is_positive(),
            );
            let density_max_f64 = spektrafilm_model::density_curves::max_density_f64(&norm_curves_f64);
            Some((
                density_curves_0_f64,
                matrix_scaled,
                density_max_f64,
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
            let matrix_f32: [[f32; 3]; 3] = [
                [d.1[0][0] as f32, d.1[0][1] as f32, d.1[0][2] as f32],
                [d.1[1][0] as f32, d.1[1][1] as f32, d.1[1][2] as f32],
                [d.1[2][0] as f32, d.1[2][1] as f32, d.1[2][2] as f32],
            ];
            spektrafilm_gpu::DirCouplersGpuParams {
                couplers_matrix_scaled: matrix_f32,
                density_max: [d.2[0] as f32, d.2[1] as f32, d.2[2] as f32],
                is_positive: d.7,
                diffusion_size_px: (d.4 / d.3) as f32,
                diffusion_tail_px: (d.5 / d.3) as f32,
                diffusion_tail_weight: d.6 as f32,
                density_curves_0: &d.0,
                log_exposure: &film_log_exp,
                gamma_factor: d.8,
            }
        });

        // Grain in the resident chain — the composite sampler only
        // (`try_gpu_resident` bails out first when the layered model is
        // active). Same per-channel particle math as
        // `apply_grain_to_density`. The GPU uses normal-approximation
        // sampling; CPU does the same whenever λ > 30 / var > 9, which is
        // the typical regime for ≥ 1 MP images.
        let grain = if self.params.film_render.grain.active {
            let g = &self.params.film_render.grain;
            let pixel_area = pixel_size_um * pixel_size_um;
            let n_sub = g.n_sub_layers.max(1);
            // GPU shaders are f32 — narrow the f64 grain params at the boundary.
            let mut npp = [0.0f32; 3];
            for c in 0..3 {
                let particle_area = g.particle_area_um2 * g.particle_scale[c];
                npp[c] = ((pixel_area as f64 / particle_area) / n_sub as f64) as f32;
            }
            let film_curves_f32 = self.film.density_curves_f32();
            let norm_curves_f32 =
                spektrafilm_model::density_curves::normalize_density_curves(&film_curves_f32);
            let dmax_curves = spektrafilm_model::density_curves::max_density(&norm_curves_f32);
            let mut density_max = [0.0f32; 3];
            for c in 0..3 {
                density_max[c] = dmax_curves[c] + g.density_min[c] as f32;
            }
            Some(spektrafilm_gpu::GrainGpuParams {
                density_min: [
                    g.density_min[0] as f32,
                    g.density_min[1] as f32,
                    g.density_min[2] as f32,
                ],
                density_max,
                n_particles_per_pixel: npp,
                grain_uniformity: [
                    g.uniformity[0] as f32,
                    g.uniformity[1] as f32,
                    g.uniformity[2] as f32,
                ],
                n_sub_layers: n_sub,
                base_seed: 0,
                grain_blur: g.blur,
                monochrome: g.monochrome,
            })
        } else {
            None
        };

        // Glare in the resident chain — applied after scan_spectral on the
        // final RGB buffer. Mirrors the CPU lognormal + blur + add. Python
        // 0.3.4 disables viewing glare entirely on the `io.scan_film` path
        // (`glare = None`; `film_render.glare` is never read upstream) —
        // only `print_render.glare` reaches the print scan.
        let glare = (!self.params.io.scan_film)
            .then(|| &self.params.print_render.glare)
            .filter(|g| g.active && g.percent > 0.0)
            .map(|g| {
                // LogNormal parameters (same derivation as `compute_random_glare_amount`).
                let m = g.percent as f64;
                let s = (g.roughness * g.percent) as f64;
                let sigma2 = (1.0 + (s * s) / (m * m)).ln();
                let sigma = sigma2.sqrt();
                let mu = m.ln() - sigma2 / 2.0;
                // glare_rgb_offset = (XYZ→RGB) · illuminant_xyz / 100.
                let mut illu_xyz = [0.0f64; 3];
                for i in 0..n_wl {
                    illu_xyz[0] += viewing_illu[i] * spektrafilm_math::spectral::CMF_X_F64[i];
                    illu_xyz[1] += viewing_illu[i] * spektrafilm_math::spectral::CMF_Y_F64[i];
                    illu_xyz[2] += viewing_illu[i] * spektrafilm_math::spectral::CMF_Z_F64[i];
                }
                for c in 0..3 {
                    illu_xyz[c] /= scan_norm;
                }
                let mut offset_rgb = [0.0f32; 3];
                for i in 0..3 {
                    let v = scan_xyz_to_rgb[i][0] * illu_xyz[0]
                        + scan_xyz_to_rgb[i][1] * illu_xyz[1]
                        + scan_xyz_to_rgb[i][2] * illu_xyz[2];
                    offset_rgb[i] = (v / 100.0) as f32;
                }
                spektrafilm_gpu::GlareGpuParams {
                    mu: mu as f32,
                    sigma: sigma as f32,
                    blur_px: g.blur,
                    base_seed: 42,
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
            grain,
            glare,
            gamut: self.output_gamut.gpu_params(),
            unsharp,
            camera_lens_blur_px,
            scanner_lens_blur_px,
            highlight_boost,
        };
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
            let rebuilt = base.clone().with_params(params);
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
        let tweaked = base.clone().with_params(params);
        assert_eq!(
            lut_data(&tweaked),
            base_lut,
            "non-spectral change must not rebuild the TC LUT"
        );
    }
}
