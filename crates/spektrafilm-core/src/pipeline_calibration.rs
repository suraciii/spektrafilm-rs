use std::path::Path;

use spektrafilm_math::spectral::TcLut;

use crate::WorkflowRoute;
use crate::enlarger;
use crate::params::{PrintExposureEvTracking, PrintExposureMode, RuntimeParams};
use crate::profile::Profile;
use crate::spectral_service::{self, select_illuminant};

/// Runtime controls that invalidate spectral calibration artifacts.
#[derive(Debug, Clone, PartialEq)]
struct SpectralControlsKey {
    capture_illuminant: Option<String>,
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
    input_gamut_boundary: String,
    input_gamut_hull_detail: f64,
}

/// Runtime controls that invalidate print/front-end calibration data.
#[derive(Debug, Clone, PartialEq)]
struct CalibrationControlsKey {
    exposure_compensation_ev: f32,
    print_exposure_mode: PrintExposureMode,
    print_exposure_ev_tracking: PrintExposureEvTracking,
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
    neutral_print_filters_protected: [bool; 3],
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
        print_exposure_mode: params.enlarger.print_exposure_mode,
        print_exposure_ev_tracking: params.settings.print_exposure_ev_tracking,
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
        neutral_print_filters_protected: params
            .neutral_print_filters_protected
            .unwrap_or([false; 3]),
    }
}

fn spectral_controls_key(params: &RuntimeParams) -> SpectralControlsKey {
    SpectralControlsKey {
        capture_illuminant: params.camera.capture_illuminant.clone(),
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
        input_gamut_boundary: params.io.input_gamut_compress.boundary.clone(),
        input_gamut_hull_detail: params.io.input_gamut_compress.hull_detail,
    }
}
pub(crate) struct CalibrationResult {
    pub film: Profile,
    pub print: Profile,
    pub params: RuntimeParams,
    pub tc_lut: Option<TcLut>,
    pub prepared_tc_lut: Option<spektrafilm_backend::PreparedTable>,
    pub mallett_core: Option<[[f64; 3]; 3]>,
    pub front_illuminant: String,
    /// Resolved inverse-scan converter for a convert-film route. `None` on
    /// every other route, which resolves an rgb→raw front pass instead; the
    /// three are alternative chain heads.
    pub convert_converter: Option<crate::stages::converting::Converter>,
    pub print_exposure_factor: f64,
    pub print_illuminant: Vec<f64>,
    pub preflash_raw: [f64; 3],
    pub midgray_density: Vec<f64>,
    pub compensated_midgray_density: Vec<f64>,
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
pub(crate) fn validate_scan_output_route_detailed(
    params: &RuntimeParams,
) -> Result<(), crate::params::ValidationError> {
    crate::params::validation::validate_enum_value_detailed(
        "scanner.scan_output",
        &params.scanner.scan_output,
    )?;
    if params.scanner.scan_output != "negative_to_positive"
        || params.workflow.route_enum().artifact_output().is_some()
    {
        return Ok(());
    }
    let direct_film_scan = params.workflow.route_enum().scans_film();
    let convert_scan = params.workflow.route_enum() == WorkflowRoute::ConvertFilmScan;
    if !direct_film_scan && !convert_scan {
        return Err(crate::params::ValidationError::new(
            crate::params::ValidationReason::ScanRoute,
            "scanner.scan_output",
            "scanner.scan_output=negative_to_positive requires a direct film scan route (input > film > scan, film > scan, or input > convert-film > scan)",
        ));
    }
    Ok(())
}

pub fn validate_scan_output(params: &RuntimeParams, film: &Profile) -> Result<(), String> {
    validate_scan_output_detailed(params, film).map_err(|error| error.to_string())
}

pub fn validate_scan_output_detailed(
    params: &RuntimeParams,
    film: &Profile,
) -> Result<(), crate::params::ValidationError> {
    use crate::params::{ValidationError as Error, ValidationReason as Reason};
    validate_scan_output_route_detailed(params)?;
    if params.scanner.scan_output != "negative_to_positive"
        || params.workflow.route_enum().artifact_output().is_some()
    {
        return Ok(());
    }
    if !film.is_negative() || !film.is_film() || !film.is_filming() {
        return Err(Error::new(
            Reason::ScanNegativeProfile,
            "scanner.scan_output",
            "scanner.scan_output=negative_to_positive requires a negative filming-stage film profile; positive film and paper scans use direct_scan",
        ));
    }
    for channel in 0..3 {
        for row in &film.data.density_curves {
            if !row[channel].is_finite() {
                return Err(Error::new(Reason::ScanFiniteDensity, "scanner.scan_output", format!(
                    "scanner.scan_output=negative_to_positive requires finite density curves (channel {channel})"
                )).arg("channel", channel).arg("value", row[channel]));
            }
        }
    }
    if params.film_render.grain.active
        && matches!(
            params.film_render.grain.engine,
            crate::params::grain::GrainEngine::Procedural
        )
    {
        return Err(Error::new(
            Reason::ScanProceduralGrain,
            "scanner.scan_output",
            "scanner.scan_output=negative_to_positive cannot be combined with active Grain Procedural",
        ));
    }
    Ok(())
}

pub(crate) fn build_with_backend(
    film: Profile,
    print: Profile,
    mut params: RuntimeParams,
    data_dir: &Path,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<CalibrationResult, String> {
    // Reject malformed params (e.g. an unknown diffusion-filter family)
    // up front — Python raises ValueError mid-run, which also aborts
    // before any artifact; failing at construction keeps the error
    // actionable and artifact-free on every entry point.
    params.validate()?;
    crate::params_builder::normalize_runtime_topology(&film, &mut params);
    let prepare_print = params.workflow.route_enum() != WorkflowRoute::InputFilm;
    // Resolve development time and broadcast the profile's single channel
    // onto the 3-channel engine layout before reading the profile data.
    let mut film = crate::profile::resolve_for_render(film, params.film_render.development_time);
    validate_scan_output(&params, &film)?;
    let mut print = if prepare_print {
        crate::profile::resolve_for_render(print, params.print_render.development_time)
    } else {
        print
    };
    apply_base_tuning(&mut film, &params.film_render.base, None);
    apply_film_chemistry(&mut film, &params.film_render.chemistry)
        .map_err(|error| format!("invalid film density-curve model: {error}"))?;
    if prepare_print {
        apply_base_tuning(
            &mut print,
            &params.film_render.base,
            Some(&params.print_render.base),
        );

        if let Some(model) = print.data.density_curves_model.as_ref() {
            crate::print_morph::morph_density_curves(
                &print.log_exposure_f64(),
                model,
                &params.print_render.density_curves_morph,
                print.is_positive(),
            )
            .map_err(|error| format!("invalid print density-curve model: {error}"))?;
        }
    }

    // Python parity: look up per-(print, illuminant, film) neutral filter values from
    // the JSON database — matches `apply_database_neutral_print_filters`. Defaults to
    // params.enlarger.{c,m,y}_filter_neutral when the combo isn't in the database.
    // Keep the f64 lookup values around (params is f32) — narrowing to
    // f32 here costs ~4e-8 precision through the `10^(-cc/100)` step.
    // Authored-versus-baseline resolution is shared with preparation
    // (`authored_filter_axes`): authored axes keep the carried value so
    // replaying a resolved document reproduces its render; baseline axes
    // follow the per-paper database.
    let mut neutral_cmy_f64: Option<[f64; 3]> = None;
    if prepare_print && params.settings.neutral_print_filters_from_database {
        let db = crate::neutral_filters::NeutralFilters::load(data_dir)?;
        let print_stock = print.info.stock.as_deref().unwrap_or("");
        let film_stock = film.info.stock.as_deref().unwrap_or("");
        if let Some(mut cmy) = db.lookup(print_stock, &params.enlarger.illuminant, film_stock) {
            let protected =
                crate::params_builder::authored_filter_axes(params.neutral_print_filters_protected);
            let controls = [
                &mut params.enlarger.c_filter_neutral,
                &mut params.enlarger.m_filter_neutral,
                &mut params.enlarger.y_filter_neutral,
            ];
            for (axis, control) in controls.into_iter().enumerate() {
                if protected[axis] {
                    cmy[axis] = *control as f64;
                } else {
                    *control = cmy[axis] as f32;
                }
            }
            neutral_cmy_f64 = Some(cmy);
        }
    }
    let cmy_f64 = neutral_cmy_f64.unwrap_or([
        params.enlarger.c_filter_neutral as f64,
        params.enlarger.m_filter_neutral as f64,
        params.enlarger.y_filter_neutral as f64,
    ]);
    let (c_neutral_f64, m_neutral_f64, y_neutral_f64) = (cmy_f64[0], cmy_f64[1], cmy_f64[2]);

    // Capture illuminant is a filming-context input only for direct film-exposure
    // routes. Keep the user value in the runtime params for reporting/persistence,
    // but remove it from calibration on input and convert-film routes so inactive
    // controls cannot change midgray or auto print-exposure artifacts.
    let spectral_params = if params.workflow.route_enum().exposes_film() {
        params.clone()
    } else {
        let mut inactive_params = params.clone();
        inactive_params.camera.capture_illuminant = None;
        inactive_params
    };
    let spectral_service::FilmingUpsampler {
        tc_lut,
        prepared_tc_lut,
        mallett_core,
        front_illuminant,
        ..
    } = spectral_service::build_filming_upsampler_with_backend(&film, &spectral_params, data_dir, backend)?;

    // Python parity: sensitivities are pre-balanced in the profile so midgray ≈ 1.0.
    // The TC LUT is used unnormalized — Python's `rgb_to_raw_hanatos2025` does NOT scale it.
    // See `spektrafilm/utils/spectral_upsampling.py:rgb_to_raw_hanatos2025` (comment line 373).

    // Compute enlarger illuminant with dichroic filters — f64 for Python parity.
    // Unprotected neutral axes retain f64 database values; explicit controls and shifts are f32.
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
        if let Some(prepared) = &prepared_tc_lut {
            let xyz = spectral_service::calibration_neutral_xyz(
                &front_illuminant, params.settings.use_cat16, scale,
            );
            let brightness = xyz[0] + xyz[1] + xyz[2];
            prepared.neutral.map(|value| spektrafilm_math::precision::to_f64(
                spektrafilm_math::precision::from_f64(value * brightness),
            ))
        } else if let Some(lut) = &tc_lut {
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

    let compensated_midgray_density = if params.camera.exposure_compensation_ev == 0.0 {
        density_spectral_midgray.clone()
    } else {
        enlarger::midgray_density_spectral_from_raw(
            midgray_raw(2.0f64.powf(params.camera.exposure_compensation_ev as f64)),
            &film,
            &params,
        )
    };
    // Print sensitivity: Python `sensitivity = np.nan_to_num(10 ** log_sensitivity)`.
    let print_sensitivity: Vec<[f64; 3]> = if prepare_print {
        print.sensitivity_f64()
    } else {
        Vec::new()
    };

    // Constant enlarger preflash exposure (Python `_compute_raw_preflash`):
    // film base density lit by the preflash-filtered illuminant, integrated
    // against the print sensitivity. Off (→ [0; 3]) unless preflash_exposure > 0.
    let preflash_raw = if prepare_print {
        enlarger::compute_preflash_raw(
            &params.enlarger,
            c_neutral_f64,
            m_neutral_f64,
            y_neutral_f64,
            &film.data.base_density,
            &print_sensitivity,
        )
    } else {
        [0.0; 3]
    };

    // Print exposure normalization — mirror Python's
    // `_compute_exposure_factor_midgray` in
    // `spektrafilm/runtime/stages/printing.py`. There are two
    // candidate factors:
    //   * `factor_midgray`     = 1 / geomean(raw_midgray)
    //   * `factor_midgray_comp`= 1 / geomean(raw_midgray_with_neg_EV)
    // selected by `enlarger.print_exposure_mode` and
    // `settings.print_exposure_ev_tracking`. The legacy boolean pair
    // `normalize_print_exposure` × `print_exposure_compensation` maps
    // one-to-one onto the two enums: (true, true) → auto/follow_mode,
    // (true, false) → auto/never, (false, false) → manual/follow_mode,
    // (false, true) → manual/always.
    //
    // With the defaults (auto/follow_mode) and no EV compensation,
    // Python returns `factor_midgray_comp == 1.0`. Rust used to
    // unconditionally apply `factor_midgray` here, which biased
    // the print exposure by ~3% on every render and was the root
    // cause of the residual Python-parity drift in the print stage.
    let factor_midgray = if prepare_print {
        enlarger::compute_exposure_factor(
            &density_spectral_midgray,
            &print_illuminant,
            &print_sensitivity,
        )
    } else {
        1.0
    };
    // Python builds `density_spectral_midgray_comp` whenever EV
    // tracking is on — even when EV == 0, in which case
    // `rgb_midgray_comp = rgb_midgray * 2^0` and so
    // `factor_midgray_comp == factor_midgray`. We have to mirror
    // that (NOT short-circuit to 1.0) because the
    // `factor_midgray_comp` branch is what gets returned by default.
    let factor_midgray_comp = if !prepare_print
        || params.settings.print_exposure_ev_tracking == PrintExposureEvTracking::Never
    {
        1.0
    } else if params.camera.exposure_compensation_ev == 0.0 {
        factor_midgray
    } else {
        // The same frozen compensated reference is consumed by continuation.
        enlarger::compute_exposure_factor(
            &compensated_midgray_density,
            &print_illuminant,
            &print_sensitivity,
        )
    };
    let print_exposure_factor = select_print_factor(&params, factor_midgray, factor_midgray_comp);

    tracing::info!(
        method = params.settings.rgb_to_raw_method,
        lut_size = tc_lut.as_ref().map_or(0, |l| l.size),
        film = film.info.stock.as_deref().unwrap_or("unknown"),
        print_exposure_factor = print_exposure_factor,
        "pipeline calibrated"
    );

    let output_gamut = crate::gamut_compression::OutputGamutCompress::build_with_backend(
        &params.io.output_gamut_compress,
        &params.io.output_color_space,
        backend,
    )?;
    // The inverse-scan converter reads the resolved film's dye tables, the
    // route, and the input colour space — all of which must be final before
    // the record is built, so it resolves after every calibration mutation
    // and before the result consumes the film and the params.
    let convert_converter = crate::stages::converting::resolve_converter(&film, &params)?;
    Ok(CalibrationResult {
        film,
        print,
        params,
        convert_converter,
        tc_lut,
        prepared_tc_lut,
        mallett_core,
        front_illuminant,
        print_exposure_factor,
        print_illuminant,
        preflash_raw,
        midgray_density: density_spectral_midgray,
        compensated_midgray_density,
        data_dir: Some(data_dir.to_path_buf()),
        output_gamut,
    })
}

fn select_print_factor(params: &RuntimeParams, midgray: f64, compensated: f64) -> f64 {
    match (
        params.enlarger.print_exposure_mode,
        params.settings.print_exposure_ev_tracking,
    ) {
        (PrintExposureMode::Auto, PrintExposureEvTracking::Never) => midgray,
        (PrintExposureMode::Auto, _) => compensated,
        (PrintExposureMode::Manual, PrintExposureEvTracking::Always) => compensated / midgray,
        (PrintExposureMode::Manual, _) => 1.0,
    }
}

/// Initialize only printing, consuming the frozen filming calibration.
pub(crate) fn prepare_continued_print(
    film: &Profile,
    source: Profile,
    params: &mut RuntimeParams,
    context: &crate::stage_artifact::InterpretationContext,
    data_dir: &Path,
) -> Result<(Profile, f64, Vec<f64>, [f64; 3]), String> {
    if !source.is_print_stock() {
        return Err("new printing material must be paper/printing or film/printing".into());
    }
    let mut print =
        crate::profile::resolve_for_render(source, params.print_render.development_time);
    apply_base_tuning(
        &mut print,
        &params.film_render.base,
        Some(&params.print_render.base),
    );
    if let Some(model) = print.data.density_curves_model.as_ref() {
        crate::print_morph::morph_density_curves(
            &print.log_exposure_f64(),
            model,
            &params.print_render.density_curves_morph,
            print.is_positive(),
        )?;
    }
    let mut cmy = [
        params.enlarger.c_filter_neutral as f64,
        params.enlarger.m_filter_neutral as f64,
        params.enlarger.y_filter_neutral as f64,
    ];
    if params.settings.neutral_print_filters_from_database {
        let db = crate::neutral_filters::NeutralFilters::load(data_dir)?;
        if let Some(database) = db.lookup(
            print.info.stock.as_deref().unwrap_or(""),
            &params.enlarger.illuminant,
            film.info.stock.as_deref().unwrap_or(""),
        ) {
            let protected =
                crate::params_builder::authored_filter_axes(params.neutral_print_filters_protected);
            for axis in 0..3 {
                if !protected[axis] {
                    cmy[axis] = database[axis];
                }
            }
            params.enlarger.c_filter_neutral = cmy[0] as f32;
            params.enlarger.m_filter_neutral = cmy[1] as f32;
            params.enlarger.y_filter_neutral = cmy[2] as f32;
        }
    }
    let illuminant = enlarger::enlarger_filtered_illuminant_f64(
        &params.enlarger.illuminant,
        cmy[0],
        cmy[1] + params.enlarger.m_filter_shift as f64,
        cmy[2] + params.enlarger.y_filter_shift as f64,
    );
    let sensitivity = print.sensitivity_f64();
    let preflash = enlarger::compute_preflash_raw(
        &params.enlarger,
        cmy[0],
        cmy[1],
        cmy[2],
        &film.data.base_density,
        &sensitivity,
    );
    let midgray =
        enlarger::compute_exposure_factor(&context.midgray_density, &illuminant, &sensitivity);
    let compensated =
        if params.settings.print_exposure_ev_tracking == PrintExposureEvTracking::Never {
            1.0
        } else if context.camera_compensation_ev == 0.0 {
            midgray
        } else {
            enlarger::compute_exposure_factor(
                &context.compensated_midgray_density,
                &illuminant,
                &sensitivity,
            )
        };
    Ok((
        print,
        select_print_factor(params, midgray, compensated),
        illuminant,
        preflash,
    ))
}

#[cfg(test)]
mod tests {
    use super::{validate_scan_output, validate_scan_output_route_detailed};
    use crate::params::{RuntimeParams, grain::GrainEngine};
    use crate::profile::Profile;

    fn profile(film_type: &str, support: &str, stage: &str) -> Profile {
        serde_json::from_value(serde_json::json!({
            "metadata": {},
            "info": {
                "stock": "test_stock",
                "name": "Test profile",
                "type": film_type,
                "support": support,
                "stage": stage
            },
            "data": {
                "log_exposure": [-2.0, 0.0, 2.0],
                "density_curves": [
                    [0.1, 0.1, 0.1],
                    [0.5, 0.5, 0.5],
                    [1.0, 1.0, 1.0]
                ]
            }
        }))
        .unwrap()
    }

    #[test]
    fn print_exposure_factor_folds_degenerate_tracking_onto_the_mode_default() {
        use crate::params::{PrintExposureEvTracking, PrintExposureMode};
        use crate::test_support::data_dir;
        use PrintExposureEvTracking::{Always, FollowMode, Never};
        use PrintExposureMode::{Auto, Manual};
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let factor = |mode: PrintExposureMode, tracking: PrintExposureEvTracking| {
            let mut params = RuntimeParams::default();
            params.enlarger.print_exposure_mode = mode;
            params.settings.print_exposure_ev_tracking = tracking;
            params.camera.exposure_compensation_ev = 0.5;
            super::build_with_backend(film.clone(), print.clone(), params, &dir, &spektrafilm_backend::cpu_backend::CpuBackend)
                .unwrap()
                .print_exposure_factor
        };
        let auto_follow = factor(Auto, FollowMode);
        let auto_never = factor(Auto, Never);
        let auto_always = factor(Auto, Always);
        let manual_follow = factor(Manual, FollowMode);
        let manual_never = factor(Manual, Never);
        let manual_always = factor(Manual, Always);
        // The degenerate tracking choices fold exactly onto the mode
        // default; they only carry the legacy mixed states.
        assert_eq!(auto_always, auto_follow);
        assert_eq!(manual_never, manual_follow);
        // The manual default applies the print exposure alone.
        assert_eq!(manual_follow, 1.0);
        // The tracked branches stay distinct at nonzero camera EV ...
        assert_ne!(auto_follow, auto_never);
        // ... and manual+always carries exactly the tracking ratio.
        assert!((manual_always * auto_never - auto_follow).abs() < 1e-12);
    }

    #[test]
    fn negative_to_positive_route_rejects_print_and_accepts_direct_film_scan() {
        let mut params = RuntimeParams::default();
        params.scanner.scan_output = "negative_to_positive".into();
        params.io.scan_film = true;
        params.workflow.route = "input > film > print > scan".into();
        let error = validate_scan_output_route_detailed(&params)
            .unwrap_err()
            .to_string();
        assert!(error.contains("scanner.scan_output=negative_to_positive"));
        assert!(error.contains("direct film scan route"));
        params.workflow.route = "input > film > scan".into();
        assert!(validate_scan_output_route_detailed(&params).is_ok());
    }

    #[test]
    fn negative_to_positive_accepts_only_a_direct_negative_film_route() {
        let film = profile("negative", "film", "filming");
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
        assert!(validate_scan_output(&params, &film).is_ok());
    }

    #[test]
    fn negative_to_positive_rejects_profiles_outside_the_filming_negative_case() {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();

        assert!(validate_scan_output(&params, &profile("positive", "film", "filming")).is_err());
        assert!(validate_scan_output(&params, &profile("negative", "paper", "printing")).is_err());
        assert!(validate_scan_output(&params, &profile("negative", "film", "printing")).is_err());

        let film = profile("negative", "film", "filming");
        params.film_render.grain.active = true;
        params.film_render.grain.engine = GrainEngine::Procedural;
        assert!(validate_scan_output(&params, &film).is_err());
    }

    #[test]
    fn negative_to_positive_rejects_nonfinite_density_curves() {
        let mut film = profile("negative", "film", "filming");
        film.data.density_curves[1][2] = f64::NAN;
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
        let error = validate_scan_output(&params, &film).unwrap_err();
        assert!(error.contains("finite density curves"));
        assert!(error.contains("channel 2"));
    }
}
