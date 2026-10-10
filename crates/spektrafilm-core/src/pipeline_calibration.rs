use std::path::Path;

use spektrafilm_math::spectral::TcLut;

use crate::enlarger;
use crate::params::RuntimeParams;
use crate::pipeline::{apply_base_tuning, apply_film_chemistry};
use crate::profile::Profile;
use crate::spectral_service::{self, select_illuminant, select_illuminant_f64};

/// Runtime controls that invalidate spectral calibration artifacts.
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
    input_gamut_knee: [f32; 3],
}

/// Runtime controls that invalidate print/front-end calibration data.
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

pub(crate) fn spectral_changed(old: &RuntimeParams, new: &RuntimeParams) -> bool {
    spectral_controls_key(old) != spectral_controls_key(new)
}

pub(crate) fn calibration_changed(old: &RuntimeParams, new: &RuntimeParams) -> bool {
    calibration_controls_key(old) != calibration_controls_key(new)
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
        input_gamut_knee: params.io.input_gamut_compress.knee,
    }
}
pub(crate) struct CalibrationResult {
    pub film: Profile,
    pub print: Profile,
    pub params: RuntimeParams,
    pub tc_lut: Option<TcLut>,
    pub mallett_core: Option<[[f64; 3]; 3]>,
    pub front_illuminant: String,
    pub print_exposure_factor: f64,
    pub print_illuminant: Vec<f64>,
    pub preflash_raw: [f64; 3],
    pub output_gamut: crate::gamut_compression::OutputGamutCompress,
    pub data_dir: Option<std::path::PathBuf>,
}

pub(crate) fn build(
    film: Profile,
    print: Profile,
    mut params: RuntimeParams,
    data_dir: &Path,
) -> Result<CalibrationResult, String> {
    // Reject malformed params (e.g. an unknown diffusion-filter family)
    // up front — Python raises ValueError mid-run, which also aborts
    // before any artifact; failing at construction keeps the error
    // actionable and artifact-free on every entry point.
    params.validate()?;
    if params.workflow.route == "input > film > scan" {
        params.io.scan_film = true;
    }
    if params.workflow.route == "input > film > scan > magazine" {
        params.io.scan_film = true;
    }
    // time and broadcast the single channel onto the 3-channel engine
    // layout. Must happen before anything reads the profile data.
    let mut film = crate::profile::resolve_for_render(film, params.film_render.development_time);
    let mut print = crate::profile::resolve_for_render(print, params.print_render.development_time);
    apply_base_tuning(&mut film, &params.film_render.base, None);
    apply_film_chemistry(&mut film, &params.film_render.chemistry)
        .map_err(|error| format!("invalid film density-curve model: {error}"))?;
    apply_base_tuning(
        &mut print,
        &params.film_render.base,
        Some(&params.print_render.base),
    );

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
    let method_name = params.settings.rgb_to_raw_method.as_str();
    let method = crate::params::validation::RgbToRawMethod::parse(method_name)?;
    let (tc_lut, mallett_core): (Option<TcLut>, Option<[[f64; 3]; 3]>) = match method {
        crate::params::validation::RgbToRawMethod::Hanatos2025 => {
            let window_params: Vec<f64> = film.data.hanatos2025_adaptation_window_params.clone();
            let adaptation = spectral_service::Hanatos2025Adaptation {
                window_params: &window_params,
                surface_params: &film.data.hanatos2025_adaptation_surface_params,
                spectral_gaussian_blur: params.settings.spectral_gaussian_blur as f64,
                reference_illuminant: &ref_illuminant_f64,
                reference_illuminant_xy: spektrafilm_math::spectral::illuminant_to_xy(
                    &ref_illuminant,
                ),
                apply_window: params.settings.apply_hanatos2025_adaptation_window,
                apply_surface: params.settings.apply_hanatos2025_adaptation_surface,
            };
            let tc_lut = spectral_service::compute_registered_tc_lut(
                data_dir,
                "hanatos2025",
                &sensitivity,
                &ref_illuminant_f64,
                Some(&adaptation),
            )?;

            // Upstream Hanatos compresses around fixed D65, independently of
            // the film reference white used for spectral adaptation.
            let input_gamut =
                crate::input_gamut::InputGamutCompress::build(&params.io.input_gamut_compress)?;
            let tc_lut = if input_gamut.is_active() {
                let (rx, ry) =
                    spektrafilm_math::spectral::illuminant_to_xy(&select_illuminant("D65"));
                input_gamut.remap(&tc_lut, [rx, ry])
            } else {
                tc_lut
            };
            (Some(tc_lut), None)
        }
        crate::params::validation::RgbToRawMethod::Arctic2026Alpha02
        | crate::params::validation::RgbToRawMethod::Arctic2026Beta04
        | crate::params::validation::RgbToRawMethod::GaussLasers
        | crate::params::validation::RgbToRawMethod::Jakob2019
        | crate::params::validation::RgbToRawMethod::Otsu2018 => {
            front_illuminant = "D65".into();
            let tc_lut = spectral_service::compute_registered_tc_lut(
                data_dir,
                method_name,
                &sensitivity,
                &ref_illuminant_f64,
                None,
            )?;
            let input_gamut =
                crate::input_gamut::InputGamutCompress::build(&params.io.input_gamut_compress)?;
            let tc_lut = if input_gamut.is_active() {
                let (rx, ry) = spektrafilm_math::spectral::illuminant_to_xy(&select_illuminant(
                    &front_illuminant,
                ));
                input_gamut.remap(&tc_lut, [rx, ry])
            } else {
                tc_lut
            };
            (Some(tc_lut), None)
        }
        crate::params::validation::RgbToRawMethod::Mallett2019 => (
            None,
            Some(crate::mallett::compute_core_matrix(
                &sensitivity,
                &ref_illuminant_f64,
            )),
        ),
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
    Ok(CalibrationResult {
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
