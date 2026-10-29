use std::path::Path;

use crate::WorkflowRoute;
use crate::enlarger;
use crate::params::{PrintExposureEvTracking, PrintExposureMode, RuntimeParams};
use crate::profile::Profile;
use crate::spectral_service::{self, select_illuminant};

struct SourceIdentity<'a>(&'a Profile);

impl PartialEq for SourceIdentity<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum FilmChemistryInputs<'a> {
    NoModel,
    Unmorphed,
    Morphed(&'a crate::params::PrintCurvesMorphParams),
}

#[derive(PartialEq)]
pub(crate) struct FilmMaterialInputs<'a> {
    source: SourceIdentity<'a>,
    base: Option<&'a crate::params::FilmBaseParams>,
    chemistry: FilmChemistryInputs<'a>,
    development_time: Option<f64>,
}

impl<'a> FilmMaterialInputs<'a> {
    pub fn new(source: &'a Profile, params: &'a RuntimeParams) -> Self {
        let chemistry = &params.film_render.chemistry;
        Self {
            source: SourceIdentity(source),
            base: params
                .film_render
                .base
                .active
                .then_some(&params.film_render.base),
            chemistry: if source.data.density_curves_model.is_none() {
                FilmChemistryInputs::NoModel
            } else if chemistry.active {
                FilmChemistryInputs::Morphed(chemistry)
            } else {
                FilmChemistryInputs::Unmorphed
            },
            development_time: params.film_render.development_time,
        }
    }

    pub fn resolve(&self) -> Profile {
        crate::profile::resolve_for_render(self.source.0.clone(), self.development_time)
    }

    pub fn build(&self, mut film: Profile) -> Result<Profile, String> {
        if let Some(base) = self.base {
            apply_base_tuning_inner(&mut film, BaseTuning::Film(base));
        }
        let inactive = crate::params::PrintCurvesMorphParams {
            active: false,
            ..Default::default()
        };
        let chemistry = match self.chemistry {
            FilmChemistryInputs::NoModel => return Ok(film),
            FilmChemistryInputs::Unmorphed => &inactive,
            FilmChemistryInputs::Morphed(chemistry) => chemistry,
        };
        apply_film_chemistry(&mut film, chemistry)
            .map_err(|error| format!("invalid film density-curve model: {error}"))?;
        Ok(film)
    }
}

#[derive(PartialEq)]
pub(crate) struct PrintMaterialInputs<'a> {
    source: SourceIdentity<'a>,
    base: Option<&'a crate::params::PrintBaseParams>,
    development_time: Option<f64>,
}

impl<'a> PrintMaterialInputs<'a> {
    pub fn new(source: &'a Profile, params: &'a RuntimeParams) -> Option<Self> {
        (!matches!(
            params.workflow.route_enum(),
            WorkflowRoute::Input | WorkflowRoute::InputFilm
        ))
        .then_some(Self {
            source: SourceIdentity(source),
            base: params
                .print_render
                .base
                .active
                .then_some(&params.print_render.base),
            development_time: params.print_render.development_time,
        })
    }

    pub fn build(&self) -> Profile {
        let mut print =
            crate::profile::resolve_for_render(self.source.0.clone(), self.development_time);
        if let Some(base) = self.base {
            apply_base_tuning_inner(&mut print, BaseTuning::Print(base));
        }
        print
    }
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
    let tuning = match print_base {
        Some(base) => BaseTuning::Print(base),
        None => BaseTuning::Film(film_base),
    };
    apply_base_tuning_inner(profile, tuning);
}

#[derive(Clone, Copy)]
enum BaseTuning<'a> {
    Film(&'a crate::params::FilmBaseParams),
    Print(&'a crate::params::PrintBaseParams),
}

fn apply_base_tuning_inner(profile: &mut Profile, tuning: BaseTuning<'_>) {
    let (active, scale, channels, tilt, sigma_points, peaks) = match tuning {
        BaseTuning::Print(base) => (
            base.active,
            base.scale,
            [base.yellow, base.magenta, base.cyan],
            0.0,
            20.0 / 5.0,
            [445.0, 530.0, 610.0],
        ),
        BaseTuning::Film(film_base) => (
            film_base.active,
            film_base.scale,
            [film_base.yellow, film_base.magenta, film_base.cyan],
            film_base.tilt,
            35.0 / 5.0,
            [460.0, 555.0, 650.0],
        ),
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
    let tuned = if matches!(tuning, BaseTuning::Print(_)) {
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
    // BW family columns index development times before render-time replication.
    for row in &film.data.density_curves {
        for (channel, value) in row.iter().enumerate() {
            if !value.is_finite() {
                return Err(Error::new(Reason::ScanFiniteDensity, "scanner.scan_output", format!(
                    "scanner.scan_output=negative_to_positive requires finite density curves (channel {channel})"
                )).arg("channel", channel).arg("value", value));
            }
        }
    }
    Ok(())
}

pub(crate) struct GrayReferences {
    pub midgray_density: Vec<f64>,
    pub compensated_midgray_density: Vec<f64>,
}

pub(crate) struct GrayInputs<'a> {
    film: &'a Profile,
    spectral: &'a spectral_service::SpectralPreparation,
    gamma: f32,
    exposure_ev: f32,
    use_cat16: bool,
}

impl<'a> GrayInputs<'a> {
    pub fn new(
        film: &'a Profile,
        spectral: &'a spectral_service::SpectralPreparation,
        params: &RuntimeParams,
    ) -> Self {
        Self {
            film,
            spectral,
            gamma: params.film_render.density_curve_gamma,
            exposure_ev: params.camera.exposure_compensation_ev,
            use_cat16: spectral.tc_lut().is_some() && params.settings.use_cat16,
        }
    }

    pub fn build(&self) -> GrayReferences {
        let raw = |scale: f64| {
            if let Some(prepared) = self.spectral.prepared_tc_lut() {
                let xyz = spectral_service::calibration_neutral_xyz(
                    &self.spectral.front_illuminant,
                    self.use_cat16,
                    scale,
                );
                let brightness = xyz[0] + xyz[1] + xyz[2];
                prepared.neutral.map(|value| {
                    spektrafilm_math::precision::to_f64(spektrafilm_math::precision::from_f64(
                        value * brightness,
                    ))
                })
            } else if let Some(lut) = self.spectral.tc_lut() {
                enlarger::midgray_raw_hanatos(
                    lut,
                    &select_illuminant(&self.spectral.front_illuminant),
                    self.use_cat16,
                    scale,
                )
            } else {
                let core = self
                    .spectral
                    .mallett_core()
                    .expect("spectral transform is present");
                let matrix = crate::mallett::film_matrix(core, "sRGB");
                let g = 0.184 * scale;
                crate::mallett::apply(&matrix, [g, g, g])
            }
        };
        let midgray_density =
            enlarger::midgray_density_spectral_from_raw(raw(1.0), self.film, self.gamma);
        let compensated_midgray_density = if self.exposure_ev == 0.0 {
            midgray_density.clone()
        } else {
            enlarger::midgray_density_spectral_from_raw(
                raw(2.0f64.powf(self.exposure_ev as f64)),
                self.film,
                self.gamma,
            )
        };
        GrayReferences {
            midgray_density,
            compensated_midgray_density,
        }
    }
}

impl PartialEq for GrayInputs<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.film, other.film)
            && std::ptr::eq(self.spectral, other.spectral)
            && self.gamma == other.gamma
            && self.exposure_ev == other.exposure_ev
            && self.use_cat16 == other.use_cat16
    }
}

pub(crate) struct PrintCalibration {
    pub exposure_factor: f64,
    pub illuminant: Vec<f64>,
    pub preflash_raw: [f64; 3],
    pub neutral_cmy: [f64; 3],
}

pub(crate) struct PrintInputs<'a> {
    film: &'a Profile,
    print: &'a Profile,
    gray: &'a GrayReferences,
    illuminant: &'a str,
    neutral_cmy: [f64; 3],
    m_shift: f32,
    y_shift: f32,
    preflash_exposure: f32,
    preflash_m_shift: f32,
    preflash_y_shift: f32,
    mode: PrintExposureMode,
    tracking: PrintExposureEvTracking,
    exposure_ev: f32,
}

impl<'a> PrintInputs<'a> {
    pub fn new(
        film: &'a Profile,
        print: &'a Profile,
        gray: &'a GrayReferences,
        params: &'a RuntimeParams,
        neutral_cmy: [f64; 3],
    ) -> Self {
        Self {
            film,
            print,
            gray,
            neutral_cmy,
            illuminant: &params.enlarger.illuminant,
            m_shift: params.enlarger.m_filter_shift,
            y_shift: params.enlarger.y_filter_shift,
            preflash_exposure: params.enlarger.preflash_exposure,
            preflash_m_shift: params.enlarger.preflash_m_filter_shift,
            preflash_y_shift: params.enlarger.preflash_y_filter_shift,
            mode: params.enlarger.print_exposure_mode,
            tracking: params.settings.print_exposure_ev_tracking,
            exposure_ev: params.camera.exposure_compensation_ev,
        }
    }

    pub fn build(&self) -> PrintCalibration {
        let [c, m, y] = self.neutral_cmy;
        let illuminant = enlarger::enlarger_filtered_illuminant_f64(
            self.illuminant,
            c,
            m + self.m_shift as f64,
            y + self.y_shift as f64,
        );
        let sensitivity = self.print.sensitivity_f64();
        let preflash_raw = enlarger::compute_preflash_raw(
            self.illuminant,
            self.preflash_exposure,
            self.preflash_m_shift,
            self.preflash_y_shift,
            self.neutral_cmy,
            &self.film.data.base_density,
            &sensitivity,
        );
        let midgray = enlarger::compute_exposure_factor(
            &self.gray.midgray_density,
            &illuminant,
            &sensitivity,
        );
        let compensated = if self.tracking == PrintExposureEvTracking::Never {
            1.0
        } else if self.exposure_ev == 0.0 {
            midgray
        } else {
            enlarger::compute_exposure_factor(
                &self.gray.compensated_midgray_density,
                &illuminant,
                &sensitivity,
            )
        };
        PrintCalibration {
            exposure_factor: select_print_factor(self.mode, self.tracking, midgray, compensated),
            illuminant,
            preflash_raw,
            neutral_cmy: self.neutral_cmy,
        }
    }
}

impl PartialEq for PrintInputs<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.film, other.film)
            && std::ptr::eq(self.print, other.print)
            && std::ptr::eq(self.gray, other.gray)
            && self.illuminant == other.illuminant
            && self.neutral_cmy == other.neutral_cmy
            && self.m_shift == other.m_shift
            && self.y_shift == other.y_shift
            && self.preflash_exposure == other.preflash_exposure
            && self.preflash_m_shift == other.preflash_m_shift
            && self.preflash_y_shift == other.preflash_y_shift
            && self.mode == other.mode
            && self.tracking == other.tracking
            && self.exposure_ev == other.exposure_ev
    }
}

#[derive(PartialEq)]
pub(crate) struct NeutralInputs<'a> {
    film_stock: &'a str,
    print_stock: &'a str,
    illuminant: &'a str,
    controls: [f32; 3],
    protected: [bool; 3],
    database: bool,
}

impl<'a> NeutralInputs<'a> {
    pub fn new(
        film: &'a Profile,
        print: &'a Profile,
        params: &'a RuntimeParams,
        prepare_print: bool,
    ) -> Self {
        Self {
            film_stock: film.info.stock.as_deref().unwrap_or(""),
            print_stock: print.info.stock.as_deref().unwrap_or(""),
            illuminant: &params.enlarger.illuminant,
            controls: [
                params.enlarger.c_filter_neutral,
                params.enlarger.m_filter_neutral,
                params.enlarger.y_filter_neutral,
            ],
            protected: crate::params_builder::authored_filter_axes(
                params.neutral_print_filters_protected,
            ),
            database: prepare_print && params.settings.neutral_print_filters_from_database,
        }
    }

    pub fn build(&self, data_dir: Option<&Path>) -> Result<[f64; 3], String> {
        let mut cmy = self.controls.map(f64::from);
        if self.database
            && let Some(data_dir) = data_dir
        {
            let db = crate::neutral_filters::NeutralFilters::load(data_dir)?;
            if let Some(database) = db.lookup(self.print_stock, self.illuminant, self.film_stock) {
                for axis in 0..3 {
                    if !self.protected[axis] {
                        cmy[axis] = database[axis];
                    }
                }
            }
        }
        Ok(cmy)
    }
}

pub(crate) fn validate_print_morph(
    print: &Profile,
    params: &crate::params::PrintCurvesMorphParams,
) -> Result<(), String> {
    if let Some(model) = print.data.density_curves_model.as_ref() {
        crate::print_morph::morph_density_curves(
            &print.log_exposure_f64(),
            model,
            params,
            print.is_positive(),
        )
        .map_err(|error| format!("invalid print density-curve model: {error}"))?;
    }
    Ok(())
}

fn select_print_factor(
    mode: PrintExposureMode,
    tracking: PrintExposureEvTracking,
    midgray: f64,
    compensated: f64,
) -> f64 {
    match (mode, tracking) {
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
    let cmy = NeutralInputs::new(film, &print, params, true).build(Some(data_dir))?;
    params.enlarger.c_filter_neutral = cmy[0] as f32;
    params.enlarger.m_filter_neutral = cmy[1] as f32;
    params.enlarger.y_filter_neutral = cmy[2] as f32;
    let illuminant = enlarger::enlarger_filtered_illuminant_f64(
        &params.enlarger.illuminant,
        cmy[0],
        cmy[1] + params.enlarger.m_filter_shift as f64,
        cmy[2] + params.enlarger.y_filter_shift as f64,
    );
    let sensitivity = print.sensitivity_f64();
    let preflash = enlarger::compute_preflash_raw(
        &params.enlarger.illuminant,
        params.enlarger.preflash_exposure,
        params.enlarger.preflash_m_filter_shift,
        params.enlarger.preflash_y_filter_shift,
        cmy,
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
        select_print_factor(
            params.enlarger.print_exposure_mode,
            params.settings.print_exposure_ev_tracking,
            midgray,
            compensated,
        ),
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
    fn neutral_projection_retains_database_f64_and_authored_axes() {
        let mut film = profile("negative", "film", "filming");
        film.info.stock = Some("cinestill_800t_cs41_approx".into());
        let mut print = profile("positive", "paper", "printing");
        print.info.stock = Some("fujifilm_crystal_archive_typeii".into());
        let mut params = RuntimeParams {
            neutral_print_filters_protected: Some([true, false, false]),
            ..Default::default()
        };
        params.enlarger.c_filter_neutral = 12.0;
        let cmy = super::NeutralInputs::new(&film, &print, &params, true)
            .build(Some(&crate::test_support::data_dir()))
            .unwrap();
        assert_eq!(cmy, [12.0, 37.6398, 50.9664]);
        assert_ne!(cmy[1], f64::from(cmy[1] as f32));
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
            crate::pipeline::Pipeline::new_with_spectral_backend(
                film.clone(),
                print.clone(),
                params,
                &dir,
                &spektrafilm_backend::cpu_backend::CpuBackend,
            )
            .unwrap()
            .print_exposure_factor()
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
    fn negative_to_positive_validates_raw_bw_development_families() {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
        params.film_render.grain.engine = GrainEngine::Dye;
        params.film_render.grain.active = true;
        for columns in [1, 2, 5] {
            let mut film = profile("negative", "film", "filming");
            film.info.channel_model = "bw".into();
            film.data.development_time = (0..columns).map(|i| i as f64 + 1.0).collect();
            film.data.density_curves =
                vec![vec![0.1; columns], vec![0.5; columns], vec![1.0; columns]];
            assert!(super::validate_scan_output_detailed(&params, &film).is_ok());
            for column in 0..columns {
                let mut invalid = film.clone();
                invalid.data.density_curves[1][column] = f64::NAN;
                let error = super::validate_scan_output_detailed(&params, &invalid).unwrap_err();
                assert_eq!(
                    error.reason,
                    crate::params::ValidationReason::ScanFiniteDensity
                );
            }
        }
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
    }

    #[test]
    fn negative_to_positive_accepts_every_grain_engine() {
        let film = profile("negative", "film", "filming");
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
        params.film_render.grain.active = true;
        for engine in [
            GrainEngine::Spektra,
            GrainEngine::Procedural,
            GrainEngine::Dye,
            GrainEngine::Fotufilm,
        ] {
            params.film_render.grain.engine = engine;
            let outcome = super::validate_scan_output_detailed(&params, &film);
            assert!(
                outcome.is_ok(),
                "negative_to_positive must accept {engine:?}: {outcome:?}"
            );
        }
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
