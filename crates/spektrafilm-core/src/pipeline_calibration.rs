use std::path::Path;

use spektrafilm_math::spectral::TcLut;

use crate::enlarger;
use crate::params::RuntimeParams;
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
    let upper = wavelengths
        .partition_point(|&x| x < wavelength)
        .min(values.len() - 1);
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

pub(crate) fn apply_film_chemistry(
    profile: &mut Profile,
    chemistry: &crate::params::PrintCurvesMorphParams,
) -> Result<(), String> {
    let Some(model) = profile.data.density_curves_model.as_ref() else {
        return Ok(());
    };
    let (curves, layers) = crate::print_morph::morph_density_curves_with_layers(
        &profile.log_exposure_f64(),
        model,
        chemistry,
        profile.is_positive(),
    )?;
    profile.data.density_curves = curves.iter().map(|row| row.to_vec()).collect();
    profile.data.density_curves_layers = layers;
    Ok(())
}
pub(crate) fn validate_scan_output_route(params: &RuntimeParams) -> Result<(), String> {
    if params.scanner.scan_output != "positive_scan" {
        return Ok(());
    }
    let direct_film_scan = params.workflow.route == "input > film > scan";
    let convert_scan = params.workflow.route == "input > convert-film > scan";
    if !direct_film_scan && !convert_scan {
        return Err(
            "scanner.scan_output=positive_scan requires a direct film scan route (input > film > scan or input > convert-film > scan)".into(),
        );
    }
    Ok(())
}

pub(crate) fn validate_scan_output(params: &RuntimeParams, film: &Profile) -> Result<(), String> {
    validate_scan_output_route(params)?;
    if params.scanner.scan_output != "positive_scan" {
        return Ok(());
    }
    if !film.is_negative() || !film.is_film() || !film.is_filming() {
        return Err(
            "scanner.scan_output=positive_scan requires a negative filming-stage film profile; positive film and paper scans use direct_scan".into(),
        );
    }
    for channel in 0..3 {
        for row in &film.data.density_curves {
            if !row[channel].is_finite() {
                return Err(format!(
                    "scanner.scan_output=positive_scan requires finite density curves (channel {channel})"
                ));
            }
        }
    }
    if params.scanner.white_correction || params.scanner.black_correction {
        return Err(
            "scanner.scan_output=positive_scan cannot be combined with scanner white/black correction"
                .into(),
        );
    }
    if params.film_render.grain.active
        && matches!(
            params.film_render.grain.engine,
            crate::params::grain::GrainEngine::V2
        )
    {
        return Err(
            "scanner.scan_output=positive_scan cannot be combined with active Grain V2".into(),
        );
    }
    Ok(())
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
    validate_scan_output(&params, &film)?;
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

    // Film sensitivity: Python `sensitivity = np.nan_to_num(10 ** log_sensitivity)`.
    let mut sensitivity: Vec<[f64; 3]> = film.sensitivity_f64();

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

    // Print sensitivity: Python `sensitivity = np.nan_to_num(10 ** log_sensitivity)`.
    let print_sensitivity: Vec<[f64; 3]> = print.sensitivity_f64();

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

#[cfg(test)]
mod tests {
    use super::{validate_scan_output, validate_scan_output_route};
    use crate::params::{RuntimeParams, grain::GrainEngine};
    use crate::profile;
    use std::path::Path;

    fn data_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data")
    }

    #[test]
    fn positive_scan_route_rejects_print_and_accepts_direct_film_scan() {
        let mut params = RuntimeParams::default();
        params.scanner.scan_output = "positive_scan".into();
        params.io.scan_film = true;
        params.workflow.route = "input > film > print > scan".into();
        let error = validate_scan_output_route(&params).unwrap_err();
        assert!(error.contains("scanner.scan_output=positive_scan"));
        assert!(error.contains("direct film scan route"));
        params.workflow.route = "input > film > scan".into();
        assert!(validate_scan_output_route(&params).is_ok());
    }

    #[test]
    fn positive_scan_accepts_only_a_direct_negative_film_route() {
        let film = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "positive_scan".into();
        assert!(validate_scan_output(&params, &film).is_ok());
    }

    #[test]
    fn positive_scan_rejects_positive_profiles_and_unsupported_effects() {
        let film = profile::load_profile_by_name(&data_dir(), "fujifilm_provia_100f").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "positive_scan".into();
        assert!(validate_scan_output(&params, &film).is_err());

        let paper = profile::load_profile_by_name(&data_dir(), "kodak_portra_endura").unwrap();
        assert!(validate_scan_output(&params, &paper).is_err());

        let film = profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
        params.scanner.white_correction = true;
        assert!(validate_scan_output(&params, &film).is_err());
        params.scanner.white_correction = false;
        params.film_render.grain.active = true;
        params.film_render.grain.engine = GrainEngine::V2;
        assert!(validate_scan_output(&params, &film).is_err());
    }
}
