//! Conditions shared by field discovery and winning-edit feedback.
use crate::params::{GrainEngine, RuntimeParams};
use crate::profile::Profile;

#[derive(Clone, Copy)]
pub(super) enum Condition {
    NoOp,
    MetadataNoOp,
    Grain,
    Spektra,
    Procedural,
    Dye,
    Retained,
    DensityMinimum(Option<usize>),
    MaterialRecord(Option<usize>),
    Analogue,
    Fotufilm,
    Custom,
    Spatial,
    Diffusion(bool),
    Halation,
    Boost,
    LutModeOff,
    Glare,
    Couplers,
    AutoExposure,
    Crop,
    CmykActive,
    CmykStrength,
    FilmDevelopment,
    PrintDevelopment,
    Preflash,
    CaptureIlluminant,
    ScanOperator,
    Finishing,
    Expand,
    Chemistry(bool),
    FilmExposure,
    PrintExposure,
    Scan,
    TerminalRgbFinishing,
    PrintReadout,
    ConvertFilm,
    FilmInterpretation,
    PrintInterpretation,
    GrainOperation,
    ExposureCompensation,
    FilmBase,
    PrintBase,
    Metering,
    NegativeScan,
    SpectralReference,
    FilmCurve,
}
impl Condition {
    pub(super) fn description(self) -> &'static str {
        match self {
            Self::Finishing => {
                "completed eligible positive scan; inactive routes retain finishing values"
            }
            Self::Expand => {
                "finishing.expand.enabled = true; disabled references remain stored and validated"
            }
            Self::ScanOperator => {
                "standard negative_to_positive on an eligible negative film scan route"
            }
            Self::NoOp => "preserved upstream no-op",
            Self::MetadataNoOp => {
                "preserved Procedural Film Type metadata; not used by rendering kernels"
            }
            Self::Grain => "film_render.grain.active = true; stochastic effects enabled",
            Self::Spektra => "film_render.grain.engine = spektra",
            Self::Procedural => "film_render.grain.engine = procedural",
            Self::Dye => "film_render.grain.engine = dye; supported stock, format and route",
            Self::Retained => "retained legacy Dye value; the delivered Dye law does not read it",
            Self::DensityMinimum(_) => {
                "Spektra grain on film development, or an executing film scanner or enlarger LUT; image input applies monochrome record and debug LUT-mode constraints"
            }
            Self::MaterialRecord(_) => {
                "Spektra and Dye use record 0 for black-and-white material; G/B values remain stored and inactive"
            }
            Self::Analogue => {
                "effective Procedural mode = analogue; Noise does not use Film Resolution or its method"
            }
            Self::Fotufilm => "film_render.grain.engine = fotufilm",
            Self::Custom => {
                "film_render.grain.procedural_profile = custom; select custom to use this control"
            }
            Self::Spatial => "spatial effects enabled",
            Self::Diffusion(false) => {
                "camera.diffusion_filter.active = true; strength and spatial_scale positive"
            }
            Self::Diffusion(true) => {
                "enlarger.diffusion_filter.active = true; strength and spatial_scale positive"
            }
            Self::Halation => "film_render.halation.active = true",
            Self::Boost => "film_render.halation.boost_ev != 0",
            Self::LutModeOff => "debug.lut_mode = false",
            Self::Glare => {
                "print_render.glare.active = true; percent > 0; stochastic effects enabled"
            }
            Self::Couplers => "film_render.dir_couplers.active = true",
            Self::AutoExposure => "camera.auto_exposure = true",
            Self::Crop => "io.crop = true",
            Self::CmykActive => "cmyk.active = true",
            Self::CmykStrength => "cmyk.strength > 0",
            Self::FilmDevelopment => {
                "actual Film development route; captured negatives and the input route are not redeveloped"
            }
            Self::PrintDevelopment => {
                "admitted print-development route, including recovered-negative printing"
            }
            Self::Preflash => {
                "enlarger.preflash_exposure > 0; zero disables flash and retains filter settings"
            }
            Self::Chemistry(false) => {
                "film_render.chemistry.active = true; supported fitted density_curves_model required"
            }
            Self::CaptureIlluminant => {
                "film-exposure route that converts scene RGB to film exposure"
            }
            Self::Chemistry(true) => {
                "print_render.density_curves_morph.active = true; supported fitted density_curves_model required"
            }
            Self::FilmExposure => "film-exposure route and executed span required",
            Self::PrintExposure => "print exposure operation required",
            Self::Scan => "scan operation required",
            Self::TerminalRgbFinishing => "terminal RGB finishing required",
            Self::PrintReadout => "print readout scan operation required",
            Self::ConvertFilm => "convert-film inversion operation required",
            Self::FilmInterpretation => "film material operation or interpretation required",
            Self::PrintInterpretation => "print material operation or interpretation required",
            Self::GrainOperation => {
                "selected grain engine requires film development or a scan for Procedural grain"
            }
            Self::ExposureCompensation => {
                "film exposure, film scan interpretation, or print exposure tracking required"
            }
            Self::FilmBase => "film_render.base.active = true",
            Self::PrintBase => "print_render.base.active = true",
            Self::Metering => "input metering operation required",
            Self::NegativeScan => "negative-to-positive film scan interpretation required",
            Self::SpectralReference => {
                "film exposure or a downstream calibrated spectral reference required"
            }
            Self::FilmCurve => {
                "film development or a downstream calibrated film-curve reference required"
            }
        }
    }
    fn holds_for(self, p: &RuntimeParams, film: Option<&Profile>) -> bool {
        let g = &p.film_render.grain;
        match self {
            Self::Finishing => film.is_some_and(|medium| {
                crate::finishing::is_applicable(p, medium.is_paper() || medium.is_positive())
            }),
            Self::Expand => p.finishing.expand.enabled,
            Self::ScanOperator => {
                p.scanner.operator_eligible()
                    && p.workflow.route_enum().executes_scan(&p.taps)
                    && film.is_some_and(|film| {
                        crate::pipeline_calibration::validate_scan_output(p, film).is_ok()
                    })
            }
            Self::NoOp | Self::MetadataNoOp => false,
            Self::Grain => g.active && !p.debug.deactivate_stochastic_effects,
            Self::Spektra => g.engine == GrainEngine::Spektra,
            Self::Procedural => g.engine == GrainEngine::Procedural,
            Self::Dye => g.engine == GrainEngine::Dye,
            Self::DensityMinimum(component) => {
                (Self::density_min_lut(p)
                    && (p.workflow.route_enum().artifact_input().is_some()
                        || Self::MaterialRecord(component).holds_for(p, film)))
                    || (!p.debug.lut_mode
                        && Self::GrainOperation.holds_for(p, film)
                        && Self::Grain.holds_for(p, film)
                        && Self::Spektra.holds_for(p, film)
                        && Self::MaterialRecord(component).holds_for(p, film))
            }
            Self::Retained => false,
            Self::MaterialRecord(component) => {
                component.is_none_or(|component| component == 0)
                    || film.map_or(!g.monochrome, Profile::is_color)
            }
            Self::Analogue => {
                g.procedural_profile != "custom"
                    || g.procedural_mode == crate::params::grain::ProceduralGrainMode::Analogue
            }
            Self::Fotufilm => g.engine == GrainEngine::Fotufilm,
            Self::Custom => g.procedural_profile == "custom",

            Self::Spatial => !p.debug.deactivate_spatial_effects,
            Self::Diffusion(print) => {
                let d = if print {
                    &p.enlarger.diffusion_filter
                } else {
                    &p.camera.diffusion_filter
                };
                d.active && d.strength > 0.0 && d.spatial_scale > 0.0
            }
            Self::Halation => p.film_render.halation.active,
            Self::Boost => p.film_render.halation.boost_ev != 0.0,
            Self::LutModeOff => !p.debug.lut_mode,
            Self::Glare => {
                p.print_render.glare.active
                    && p.print_render.glare.percent > 0.0
                    && !p.debug.deactivate_stochastic_effects
            }
            Self::Couplers => p.film_render.dir_couplers.active,
            Self::AutoExposure => p.camera.auto_exposure,
            Self::Crop => p.io.crop,
            Self::CmykActive => p.cmyk.active,
            Self::CmykStrength => p.cmyk.strength > 0.0,
            Self::FilmDevelopment => p.workflow.route_enum().executes_film_development(&p.taps),
            Self::PrintDevelopment => p.workflow.route_enum().executes_print_development(&p.taps),
            Self::Preflash => p.enlarger.preflash_exposure > 0.0,
            Self::CaptureIlluminant | Self::FilmExposure => {
                p.workflow.route_enum().executes_film_exposure(&p.taps)
            }
            Self::Chemistry(print) => {
                if print {
                    p.print_render.density_curves_morph.active
                } else {
                    p.film_render.chemistry.active
                }
            }
            Self::PrintExposure => p.workflow.route_enum().executes_print_exposure(&p.taps),
            Self::Scan => p.workflow.route_enum().executes_scan(&p.taps),
            Self::TerminalRgbFinishing => p
                .workflow
                .route_enum()
                .executes_terminal_rgb_finishing(&p.taps),
            Self::PrintReadout => p.workflow.route_enum().executes_print_readout(&p.taps),
            Self::ConvertFilm => p.workflow.route_enum().executes_conversion(&p.taps),
            Self::FilmCurve => {
                p.workflow.route_enum().executes_film_development(&p.taps)
                    || Self::SpectralReference.holds_for(p, film)
            }
            Self::SpectralReference => {
                let route = p.workflow.route_enum();
                route.executes_film_exposure(&p.taps)
                    || (route.artifact_input().is_none()
                        && route.executes_print_exposure(&p.taps)
                        && (p.enlarger.print_exposure_mode
                            == crate::params::PrintExposureMode::Auto
                            || p.settings.print_exposure_ev_tracking
                                == crate::params::PrintExposureEvTracking::Always))
            }
            Self::FilmInterpretation => {
                let route = p.workflow.route_enum();
                route.executes_film_exposure(&p.taps)
                    || route.executes_film_development(&p.taps)
                    || route.executes_conversion(&p.taps)
                    || route.executes_print_exposure(&p.taps)
                    || (route.scans_film() && route.executes_scan(&p.taps))
                    || (route.is_convert_film() && route.executes_scan(&p.taps))
            }
            Self::PrintInterpretation => {
                let route = p.workflow.route_enum();
                route.executes_print_exposure(&p.taps)
                    || route.executes_print_development(&p.taps)
                    || route.executes_print_readout(&p.taps)
            }
            Self::GrainOperation => {
                let route = p.workflow.route_enum();
                if g.engine == GrainEngine::Procedural {
                    route.executes_scan(&p.taps)
                } else {
                    route.executes_film_development(&p.taps)
                }
            }
            Self::ExposureCompensation => {
                let route = p.workflow.route_enum();
                route.executes_film_exposure(&p.taps)
                    || (route.scans_film()
                        && route.executes_scan(&p.taps)
                        && p.scanner.scan_output == "negative_to_positive")
                    || (route.executes_print_exposure(&p.taps)
                        && match p.settings.print_exposure_ev_tracking {
                            crate::params::PrintExposureEvTracking::Never => false,
                            crate::params::PrintExposureEvTracking::Always => true,
                            crate::params::PrintExposureEvTracking::FollowMode => {
                                p.enlarger.print_exposure_mode
                                    == crate::params::PrintExposureMode::Auto
                            }
                        })
            }
            Self::FilmBase => p.film_render.base.active,
            Self::PrintBase => p.print_render.base.active,
            Self::Metering => p.workflow.route_enum().executes_input_metering(&p.taps),
            Self::NegativeScan => {
                p.scanner.scan_output == "negative_to_positive"
                    && film.is_some_and(|film| {
                        crate::pipeline_calibration::validate_scan_output(p, film).is_ok()
                    })
            }
        }
    }

    fn density_min_lut(p: &RuntimeParams) -> bool {
        let route = p.workflow.route_enum();
        // Material continuations reuse the artifact baseline without replaying
        // image-input debug constraints; downstream LUT switches can be reset.
        (route.artifact_input().is_some() || !p.debug.lut_mode)
            && ((p.settings.use_enlarger_lut && route.executes_print_exposure(&p.taps))
                || (p.settings.use_scanner_lut
                    && p.scanner.scan_output != "negative_to_positive"
                    && route.executes_scan(&p.taps)
                    && !route.executes_print_readout(&p.taps)))
    }
}

// Stack-resident so per-widget queries and field discovery share one rule owner.
fn grain_conditions(path: &str, component: Option<usize>) -> [Option<Condition>; 4] {
    use Condition::*;
    let mut out = [None; 4];
    if !path.starts_with("film_render.grain.")
        || path.ends_with(".active")
        || path.ends_with(".engine")
    {
        return out;
    }
    if path == "film_render.grain.micro_sublayers" {
        out[0] = Some(NoOp);
        return out;
    }
    out[0] = Some(Grain);
    if path.starts_with("film_render.grain.fotufilm.") {
        out[1] = Some(Fotufilm);
    } else if path.contains(".procedural_") {
        out[1] = Some(Procedural);
        if path.ends_with(".procedural_film_type") {
            out[2] = Some(MetadataNoOp);
        } else {
            if ![
                "procedural_profile",
                "procedural_amount",
                "procedural_timer",
                "procedural_resolution_type",
            ]
            .iter()
            .any(|field| path.ends_with(field))
            {
                out[2] = Some(Custom);
            }
            if path.ends_with(".procedural_resolution_factor")
                || path.ends_with(".procedural_resolution_type")
            {
                out[3] = Some(Analogue);
            }
        }
    } else if path.ends_with(".dye_strength") || path.ends_with(".dye_scale") {
        out[1] = Some(Dye);
    } else if path.ends_with(".dye_support_um")
        || path.ends_with(".dye_event_area_um2")
        || path.ends_with(".channel_correlation")
    {
        out[1] = Some(Dye);
        out[2] = Some(Retained);
    } else if [
        "rms_granularity",
        "density_min",
        "uniformity",
        "particle_scale_sublayers",
    ]
    .iter()
    .any(|field| path.ends_with(field))
    {
        out[1] = Some(Spektra);
        if !path.ends_with(".particle_scale_sublayers") {
            out[2] = Some(MaterialRecord(component));
        }
    } else {
        out[1] = Some(Spektra);
    }
    out
}

/// Configured final-render applicability, without loading a film profile.
///
/// `component` selects a zero-based vector lane; `None` queries the whole leaf.
/// Color follows `grain.monochrome` unless an actual film profile is supplied.
/// Preview bypass and profile/route support validation are deliberately excluded.
/// Use the profile-aware query when actual material capabilities are available.
pub fn grain_control_inactive_reason(
    path: &str,
    params: &RuntimeParams,
    component: Option<usize>,
) -> Option<&'static str> {
    grain_inactive_reason(path, params, None, component)
}

/// Grain applicability using the loaded film's color-record capabilities.
/// This does not replace render-time stock/route validation or model preparation.
pub fn grain_control_inactive_reason_with_profile(
    path: &str,
    params: &RuntimeParams,
    film: &Profile,
    component: Option<usize>,
) -> Option<&'static str> {
    grain_inactive_reason(path, params, Some(film), component)
}

fn grain_inactive_reason(
    path: &str,
    params: &RuntimeParams,
    film: Option<&Profile>,
    component: Option<usize>,
) -> Option<&'static str> {
    if path == "film_render.grain.density_min" {
        if Condition::density_min_lut(params) {
            // Only image input broadcasts monochrome records before LUT use.
            if params.workflow.route_enum().artifact_input().is_some() {
                return None;
            }
            return (!Condition::MaterialRecord(component).holds_for(params, film))
                .then(|| Condition::MaterialRecord(component).description());
        }
        if params.debug.lut_mode && params.workflow.route_enum().artifact_input().is_none() {
            return Some(Condition::LutModeOff.description());
        }
    }
    if path.starts_with("film_render.grain.")
        && !path.ends_with(".engine")
        && !Condition::GrainOperation.holds_for(params, film)
    {
        return Some(Condition::GrainOperation.description());
    }
    grain_conditions(path, component)
        .into_iter()
        .flatten()
        .find(|condition| !condition.holds_for(params, film))
        .map(Condition::description)
}
pub(super) fn conditions(path: &str) -> Vec<Condition> {
    use Condition::*;
    let mut out = Vec::new();
    if path.starts_with("finishing.") {
        out.push(Finishing);
        if matches!(path, "finishing.expand.black" | "finishing.expand.white") {
            out.push(Expand);
        }
        return out;
    }
    if path.starts_with("scanner.operator.") {
        return vec![Scan, ScanOperator];
    }
    if path.starts_with("enlarger.preflash_") {
        out.push(PrintExposure);
        if path != "enlarger.preflash_exposure" {
            out.push(Preflash);
        }
        return out;
    }
    if path.starts_with("film_render.chemistry.")
        || path.starts_with("print_render.density_curves_morph.")
    {
        let print = path.starts_with("print_render.");
        out.push(if print {
            PrintDevelopment
        } else {
            FilmDevelopment
        });
        if !path.ends_with(".active") {
            out.push(Chemistry(print));
        }
        return out;
    }
    if path == "film_render.grain.density_min" {
        return vec![DensityMinimum(None)];
    }
    if path.starts_with("film_render.grain.") {
        if !path.ends_with(".engine") {
            out.push(GrainOperation);
        }
        out.extend(grain_conditions(path, None).into_iter().flatten());
        return out;
    }
    if path == "debug.print_timings" || path.starts_with("film_render.glare.") {
        return vec![NoOp];
    }
    if path.starts_with("print_render.glare.") {
        out.push(PrintReadout);
        if !path.ends_with(".active") {
            out.push(Glare);
        }
    } else if path.contains("diffusion_filter.") {
        let print = path.starts_with("enlarger.");
        out.push(if print { PrintExposure } else { FilmExposure });
        if !path.ends_with(".active") {
            out.extend([Spatial, Diffusion(print)]);
        }
    } else if path.starts_with("film_render.halation.") {
        out.push(FilmExposure);
        if path.ends_with(".boost_ev") {
            out.extend([Halation, LutModeOff]);
        } else if path.ends_with(".boost_range") || path.ends_with(".protect_ev") {
            out.extend([Halation, Boost, LutModeOff]);
        } else if !path.ends_with(".active") {
            out.extend([Halation, Spatial]);
        }
    } else if path.starts_with("film_render.dir_couplers.") {
        out.push(FilmDevelopment);
        if !path.ends_with(".active") {
            out.push(Couplers);
        }
    } else if path.starts_with("film_render.convert.") {
        out.push(ConvertFilm);
    } else if path.starts_with("film_render.base.") {
        out.push(FilmInterpretation);
        if !path.ends_with(".active") {
            out.push(FilmBase);
        }
    } else if path.starts_with("print_render.base.") {
        out.push(PrintInterpretation);
        if !path.ends_with(".active") {
            out.push(PrintBase);
        }
    } else if path == "film_render.density_curve_gamma" {
        out.push(FilmCurve);
    } else if path == "film_render.development_time" {
        out.push(FilmDevelopment);
    } else if matches!(
        path,
        "print_render.development_time" | "print_render.density_curve_gamma"
    ) {
        out.push(PrintDevelopment);
    } else if path.starts_with("enlarger.") || path == "settings.print_exposure_ev_tracking" {
        out.push(PrintExposure);
    } else if path == "camera.exposure_compensation_ev" {
        out.push(ExposureCompensation);
    } else if matches!(path, "camera.auto_exposure" | "camera.auto_exposure_method") {
        out.push(Metering);
        if path == "camera.auto_exposure_method" {
            out.push(AutoExposure);
        }
    } else if path == "camera.lens_blur_um" {
        out.push(FilmExposure);
    } else if matches!(
        path,
        "camera.filter_uv" | "camera.filter_ir" | "camera.color_filter"
    ) || path.starts_with("io.input_gamut_compress.")
        || matches!(
            path,
            "settings.rgb_to_raw_method"
                | "settings.apply_hanatos2025_adaptation_window"
                | "settings.apply_hanatos2025_adaptation_surface"
                | "settings.spectral_gaussian_blur"
                | "settings.use_cat16"
        )
    {
        out.push(SpectralReference);
    } else if path.starts_with("scanner.") {
        out.push(Scan);
        if matches!(
            path,
            "scanner.conversion_style" | "scanner.experimental_setup"
        ) {
            out.push(NegativeScan);
        }
    } else if path.starts_with("io.crop_") {
        out.push(Crop);
    } else if path == "camera.capture_illuminant" {
        out.push(CaptureIlluminant);
    } else if path.starts_with("cmyk.") {
        out.push(TerminalRgbFinishing);
        if !path.ends_with(".active") {
            out.push(CmykActive);
            if path.ends_with(".strength") {
                out.push(CmykStrength);
            }
        }
    }
    out
}

/// Configured applicability when material data is not available. A missing
/// profile never establishes support for a material-dependent condition.
pub fn inactive_reason_without_profiles(
    path: &str,
    params: &RuntimeParams,
) -> Option<&'static str> {
    if path.starts_with("film_render.grain.") {
        return grain_control_inactive_reason(path, params, None);
    }
    if let Some(reason) = conditions(path)
        .into_iter()
        .find(|condition| !condition.holds_for(params, None))
    {
        return Some(reason.description());
    }
    if !path.ends_with(".active")
        && (path.starts_with("film_render.chemistry.")
            || path.starts_with("print_render.density_curves_morph."))
    {
        return Some("selected material capability is unavailable");
    }
    None
}
pub fn inactive_reason(
    path: &str,
    params: &RuntimeParams,
    film: &Profile,
    print: &Profile,
) -> Option<String> {
    inactive_reason_with_materials(path, params, Some(film), Some(print))
}

/// Evaluates against available cached materials without loading or cloning them.
pub fn inactive_reason_with_materials(
    path: &str,
    params: &RuntimeParams,
    film: Option<&Profile>,
    print: Option<&Profile>,
) -> Option<String> {
    if path.starts_with("film_render.grain.") {
        let reason = grain_inactive_reason(path, params, film, None);
        if reason.is_some() {
            return reason.map(str::to_owned);
        }
        if path == "film_render.grain.density_min" && Condition::density_min_lut(params) {
            return None;
        }
        if film.is_none()
            && params.film_render.grain.engine != GrainEngine::Procedural
            && !path.ends_with(".active")
            && !path.ends_with(".engine")
        {
            return Some("film material capability is unavailable".into());
        }
        return None;
    }
    let medium = if params.workflow.route_enum().prints()
        || params.workflow.route_enum().artifact_input()
            == Some(crate::stage_artifact::MaterialKind::Print)
    {
        print
    } else {
        film
    };
    if let Some(reason) = conditions(path)
        .into_iter()
        .find(|condition| {
            !condition.holds_for(
                params,
                if path.starts_with("finishing.") {
                    medium
                } else {
                    film
                },
            )
        })
        .map(|condition| condition.description().into())
    {
        return Some(reason);
    }
    let needs_film = !path.ends_with(".active") && path.starts_with("film_render.chemistry.");
    let needs_print =
        !path.ends_with(".active") && path.starts_with("print_render.density_curves_morph.");
    let material = if needs_film {
        film
    } else if needs_print {
        print
    } else {
        None
    };
    if (needs_film || needs_print) && material.is_none() {
        return Some("selected material capability is unavailable".into());
    }
    if let Some(material) = material
        && !material
            .data
            .density_curves_model
            .as_ref()
            .is_some_and(|model| {
                matches!(
                    model.model_type.as_str(),
                    "cdfs" | "norm_cdfs" | "sept_norm_cdfs"
                ) && model.n_layers() > 0
            })
    {
        return Some("selected material has no supported fitted density_curves_model; chemistry values remain stored and inactive".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::grain::ProceduralGrainMode;

    #[test]
    fn finishing_feedback_uses_the_scanned_medium_and_preserves_inactive_references() {
        let data = crate::test_support::data_dir();
        let negative = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let paper = crate::profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        assert!(inactive_reason("finishing.brightness", &params, &negative, &paper).is_some());
        params.scanner.scan_output = "negative_to_positive".into();
        assert!(inactive_reason("finishing.brightness", &params, &negative, &paper).is_none());
        assert!(inactive_reason("finishing.expand.black", &params, &negative, &paper).is_some());
        params.finishing.expand.enabled = true;
        assert!(inactive_reason("finishing.expand.black", &params, &negative, &paper).is_none());
        params.workflow.route = "print > scan".into();
        params.scanner.scan_output = "direct_scan".into();
        assert!(inactive_reason("finishing.brightness", &params, &negative, &paper).is_none());
        params.workflow.route = "input".into();
        assert!(inactive_reason("finishing.brightness", &params, &negative, &paper).is_some());
    }

    fn film(color: bool, sublayers: bool) -> Profile {
        serde_json::from_value(serde_json::json!({
            "metadata": {},
            "info": {"channel_model": if color { "color" } else { "bw" }},
            "data": {"density_curves_layers": if sublayers {
                vec![vec![vec![0.1; 3]; 3]]
            } else {
                Vec::<Vec<Vec<f64>>>::new()
            }}
        }))
        .unwrap()
    }

    #[test]
    fn shared_spectral_controls_follow_calibration_consumers_without_new_exposure() {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > convert-film > print > scan".into();
        params.enlarger.print_exposure_mode = crate::params::PrintExposureMode::Auto;
        params.settings.print_exposure_ev_tracking = crate::params::PrintExposureEvTracking::Never;
        let paths = [
            "settings.rgb_to_raw_method",
            "camera.color_filter",
            "film_render.density_curve_gamma",
        ];
        for path in paths {
            assert!(
                inactive_reason_without_profiles(path, &params).is_none(),
                "{path}"
            );
        }
        assert!(inactive_reason_without_profiles("film_render.halation.active", &params).is_some());
        params.enlarger.print_exposure_mode = crate::params::PrintExposureMode::Manual;
        for path in paths {
            assert!(
                inactive_reason_without_profiles(path, &params).is_some(),
                "{path}"
            );
        }
        params.settings.print_exposure_ev_tracking = crate::params::PrintExposureEvTracking::Always;
        for path in paths {
            assert!(
                inactive_reason_without_profiles(path, &params).is_none(),
                "{path}"
            );
        }
        params.workflow.route = "film > print > scan".into();
        // Continuation uses the artifact's frozen calibration, not a new spectral front end.
        for path in paths {
            assert!(
                inactive_reason_without_profiles(path, &params).is_some(),
                "{path}"
            );
        }
    }

    #[test]
    fn cmyk_follows_terminal_rgb_finishing_not_scan_alone() {
        let mut params = RuntimeParams::default();
        params.cmyk.active = true;
        params.cmyk.strength = 0.75;
        for (route, inject, collect, expected) in [
            ("input", None, None, true),
            ("input", Some("rgb_in"), Some("rgb_out"), true),
            ("input", Some("rgb_in"), Some("rgb_pre"), false),
            ("input", Some("rgb_out"), Some("rgb_out"), false),
            ("input > film", None, None, false),
            ("input > film > print", None, None, false),
            ("film > print", None, None, false),
            ("input > film > scan", None, None, true),
            ("film > scan", None, None, true),
            ("print > scan", None, None, true),
            ("input > convert-film > scan", None, None, true),
            ("input > convert-film > print > scan", None, None, true),
            (
                "input > film > print > scan",
                Some("cmy_print"),
                Some("rgb_out"),
                true,
            ),
            (
                "input > film > scan",
                Some("cmy_film"),
                Some("rgb_out"),
                true,
            ),
            (
                "input > film > scan",
                Some("log_e_film"),
                Some("cmy_film"),
                false,
            ),
        ] {
            params.workflow.route = route.into();
            params.taps.inject = inject.map(str::to_owned);
            params.taps.collect = collect.map(str::to_owned);
            for path in ["cmyk.active", "cmyk.strength"] {
                assert_eq!(
                    inactive_reason_without_profiles(path, &params).is_none(),
                    expected,
                    "{route} {inject:?}..{collect:?}: {path}"
                );
            }
        }
        params.workflow.route = "input".into();
        params.taps.inject = None;
        params.taps.collect = None;
        params.cmyk.active = false;
        assert!(inactive_reason_without_profiles("cmyk.active", &params).is_none());
        assert!(inactive_reason_without_profiles("cmyk.strength", &params).is_some());
    }

    #[test]
    fn negative_readout_without_exposure_does_not_consume_spectral_reference() {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.scanner.scan_output = "negative_to_positive".into();
        params.taps.inject = Some("cmy_film".into());
        params.taps.collect = Some("rgb_out".into());
        for path in [
            "settings.rgb_to_raw_method",
            "camera.color_filter",
            "film_render.density_curve_gamma",
        ] {
            assert!(
                inactive_reason_without_profiles(path, &params).is_some(),
                "{path}"
            );
        }
        assert!(inactive_reason_without_profiles("scanner.scan_output", &params).is_none());
    }

    #[test]
    fn route_operations_distinguish_exposure_development_and_readout() {
        let material = film(true, true);
        let mut params = RuntimeParams::default();
        params.film_render.halation.active = true;
        params.print_render.glare.active = true;
        params.print_render.glare.percent = 0.2;
        params.debug.deactivate_stochastic_effects = false;
        let paths = [
            "camera.capture_illuminant",
            "film_render.halation.boost_ev",
            "film_render.development_time",
            "enlarger.preflash_exposure",
            "print_render.development_time",
            "print_render.glare.active",
            "scanner.scan_output",
            "film_render.convert.scan_illuminant",
        ];
        for (route, expected) in [
            (
                "input",
                [false, false, false, false, false, false, false, false],
            ),
            (
                "input > film",
                [true, true, true, false, false, false, false, false],
            ),
            (
                "input > film > scan",
                [true, true, true, false, false, false, true, false],
            ),
            (
                "input > film > print",
                [true, true, true, true, true, false, false, false],
            ),
            (
                "input > film > print > scan",
                [true, true, true, true, true, true, true, false],
            ),
            (
                "film > scan",
                [false, false, false, false, false, false, true, false],
            ),
            (
                "film > print",
                [false, false, false, true, true, false, false, false],
            ),
            (
                "film > print > scan",
                [false, false, false, true, true, true, true, false],
            ),
            (
                "print > scan",
                [false, false, false, false, false, true, true, false],
            ),
            (
                "input > convert-film > scan",
                [false, false, false, false, false, false, true, true],
            ),
            (
                "input > convert-film > scan-minus-base",
                [false, false, false, false, false, false, true, true],
            ),
            (
                "input > convert-film > print > scan",
                [false, false, false, true, true, true, true, true],
            ),
        ] {
            params.workflow.route = route.into();
            // Route, not this derived legacy flag, owns applicability.
            params.io.scan_film = false;
            for (path, expected) in paths.iter().zip(expected) {
                assert_eq!(
                    inactive_reason(path, &params, &material, &material).is_none(),
                    expected,
                    "{route}: {path}"
                );
            }
        }
    }

    #[test]
    fn tap_spans_and_grain_engines_follow_the_executed_operations() {
        let mut p = RuntimeParams::default();
        p.workflow.route = "input > film > print > scan".into();
        p.film_render.grain.active = true;
        p.debug.deactivate_stochastic_effects = false;
        let material = film(true, true);
        let active = |path: &str, p: &RuntimeParams| {
            inactive_reason(path, p, &material, &material).is_none()
        };
        p.taps.inject = Some("log_e_film".into());
        p.taps.collect = Some("cmy_film".into());
        assert!(!active("camera.capture_illuminant", &p));
        assert!(active("film_render.development_time", &p));
        assert!(!active("scanner.scan_output", &p));
        p.film_render.grain.engine = GrainEngine::Spektra;
        assert!(active("film_render.grain.rms_granularity", &p));
        p.film_render.grain.engine = GrainEngine::Procedural;
        assert!(!active("film_render.grain.procedural_amount", &p));
        assert!(active("film_render.grain.engine", &p));
        p.taps.inject = Some("cmy_print".into());
        p.taps.collect = Some("rgb_out".into());
        assert!(active("film_render.grain.procedural_amount", &p));
        assert!(active("print_render.glare.active", &p));
        assert!(!active("enlarger.print_exposure", &p));
        p.film_render.grain.engine = GrainEngine::Spektra;
        assert!(!active("film_render.grain.rms_granularity", &p));
        assert!(active("film_render.grain.engine", &p));
        p.workflow.route = "input".into();
        p.taps.inject = Some("log_e_film".into());
        p.taps.collect = Some("cmy_film".into());
        assert!(!active("camera.capture_illuminant", &p));
        assert!(active("film_render.development_time", &p));
        assert!(active("film_render.grain.rms_granularity", &p));
        assert!(!active("scanner.scan_output", &p));
    }

    #[test]
    fn density_min_follows_executing_lut_consumers_across_routes_and_spans() {
        let path = "film_render.grain.density_min";
        let bw = film(false, true);
        let color = film(true, true);
        let mut p = RuntimeParams::default();
        p.film_render.grain.active = false;
        p.film_render.grain.engine = GrainEngine::Dye;
        p.film_render.grain.monochrome = true;
        p.debug.deactivate_stochastic_effects = true;
        // Each row is route, inject, collect, scanner LUT, enlarger LUT, output, active.
        for (route, inject, collect, scanner, enlarger, output, active) in [
            (
                "input > film > scan",
                Some("cmy_film"),
                Some("rgb_out"),
                true,
                false,
                "direct_scan",
                true,
            ),
            (
                "input > film > scan",
                Some("cmy_film"),
                Some("rgb_out"),
                true,
                false,
                "negative_to_positive",
                false,
            ),
            (
                "input > film > scan",
                Some("cmy_film"),
                Some("rgb_out"),
                false,
                true,
                "direct_scan",
                false,
            ),
            (
                "input > film > scan",
                Some("log_e_film"),
                Some("cmy_film"),
                true,
                true,
                "direct_scan",
                false,
            ),
            ("film > scan", None, None, true, false, "direct_scan", true),
            (
                "film > scan",
                None,
                None,
                false,
                false,
                "direct_scan",
                false,
            ),
            (
                "input > convert-film > scan",
                None,
                None,
                true,
                false,
                "direct_scan",
                true,
            ),
            (
                "input > convert-film > scan-minus-base",
                None,
                None,
                true,
                false,
                "direct_scan",
                true,
            ),
            (
                "input > convert-film > scan",
                Some("cmy_print"),
                Some("rgb_out"),
                true,
                false,
                "direct_scan",
                false,
            ),
            (
                "input > film > print > scan",
                Some("cmy_film"),
                Some("log_e_print"),
                false,
                true,
                "direct_scan",
                true,
            ),
            (
                "input > film > print > scan",
                Some("cmy_print"),
                Some("rgb_out"),
                true,
                true,
                "direct_scan",
                false,
            ),
            (
                "input > film > print > scan",
                Some("cmy_film"),
                Some("log_e_print"),
                true,
                false,
                "direct_scan",
                false,
            ),
            ("film > print", None, None, false, true, "direct_scan", true),
            (
                "film > print > scan",
                None,
                None,
                false,
                true,
                "direct_scan",
                true,
            ),
            (
                "input > convert-film > print > scan",
                None,
                None,
                false,
                true,
                "direct_scan",
                true,
            ),
            ("print > scan", None, None, true, true, "direct_scan", false),
            ("input", None, None, true, true, "direct_scan", false),
            (
                "input",
                Some("cmy_film"),
                Some("log_e_print"),
                false,
                true,
                "direct_scan",
                true,
            ),
        ] {
            p.workflow.route = route.into();
            p.taps.inject = inject.map(str::to_owned);
            p.taps.collect = collect.map(str::to_owned);
            p.settings.use_scanner_lut = scanner;
            p.settings.use_enlarger_lut = enlarger;
            p.scanner.scan_output = output.into();
            let case = format!(
                "{route} {inject:?}..{collect:?}, scanner={scanner}, enlarger={enlarger}, {output}"
            );
            assert_eq!(
                inactive_reason_without_profiles(path, &p).is_none(),
                active,
                "{case}"
            );
            assert_eq!(
                inactive_reason_with_materials(path, &p, None, None).is_none(),
                active,
                "{case}"
            );
            assert_eq!(
                inactive_reason(path, &p, &bw, &bw).is_none(),
                active,
                "{case}"
            );
            assert_eq!(
                conditions(path)
                    .into_iter()
                    .all(|c| c.holds_for(&p, Some(&bw))),
                active,
                "{case}"
            );
            for component in [None, Some(0), Some(1), Some(2)] {
                let component_active = active
                    && (p.workflow.route_enum().artifact_input().is_some()
                        || component.is_none_or(|index| index == 0));
                assert_eq!(
                    grain_control_inactive_reason(path, &p, component).is_none(),
                    component_active,
                    "{case} {component:?}"
                );
                assert_eq!(
                    grain_control_inactive_reason_with_profile(path, &p, &bw, component).is_none(),
                    component_active,
                    "{case} {component:?}"
                );
                assert_eq!(
                    grain_control_inactive_reason_with_profile(path, &p, &color, component)
                        .is_none(),
                    active,
                    "{case} {component:?}"
                );
            }
        }
        // The reported case also remains active with Spektra enabled.
        p.workflow.route = "input > film > scan".into();
        p.taps.inject = Some("cmy_film".into());
        p.taps.collect = Some("rgb_out".into());
        p.settings.use_scanner_lut = true;
        p.settings.use_enlarger_lut = false;
        p.film_render.grain.active = true;
        p.film_render.grain.engine = GrainEngine::Spektra;
        p.debug.deactivate_stochastic_effects = false;
        assert!(inactive_reason(path, &p, &bw, &bw).is_none());
        assert!(grain_control_inactive_reason_with_profile(path, &p, &bw, Some(0)).is_none());
        for component in [Some(1), Some(2)] {
            assert_eq!(
                grain_control_inactive_reason_with_profile(path, &p, &bw, component),
                Some(Condition::MaterialRecord(component).description())
            );
        }
        p.settings.use_scanner_lut = false;
        assert!(inactive_reason(path, &p, &bw, &bw).is_some());
    }

    #[test]
    fn density_min_retains_grain_only_record_and_switch_rules() {
        let path = "film_render.grain.density_min";
        let bw = film(false, true);
        let color = film(true, true);
        let mut p = RuntimeParams::default();
        p.workflow.route = "input > film".into();
        p.settings.use_scanner_lut = false;
        p.settings.use_enlarger_lut = false;
        p.film_render.grain.engine = GrainEngine::Spektra;
        p.film_render.grain.active = true;
        p.film_render.grain.monochrome = true;
        p.debug.deactivate_stochastic_effects = false;
        for component in [None, Some(0), Some(1), Some(2)] {
            let bw_active = component.is_none_or(|index| index == 0);
            assert_eq!(
                grain_control_inactive_reason(path, &p, component).is_none(),
                bw_active
            );
            assert_eq!(
                grain_control_inactive_reason_with_profile(path, &p, &bw, component).is_none(),
                bw_active
            );
            assert!(
                grain_control_inactive_reason_with_profile(path, &p, &color, component).is_none()
            );
            assert_eq!(
                Condition::DensityMinimum(component).holds_for(&p, Some(&bw)),
                bw_active
            );
        }
        p.film_render.grain.active = false;
        assert!(grain_control_inactive_reason(path, &p, None).is_some());
        p.film_render.grain.active = true;
        p.debug.deactivate_stochastic_effects = true;
        assert!(grain_control_inactive_reason(path, &p, None).is_some());
        p.debug.deactivate_stochastic_effects = false;
        p.film_render.grain.engine = GrainEngine::Procedural;
        assert!(grain_control_inactive_reason(path, &p, None).is_some());
    }

    #[test]
    fn density_min_is_inactive_in_debug_lut_mode() {
        let path = "film_render.grain.density_min";
        let color = film(true, true);
        let mut p = RuntimeParams::default();
        p.film_render.grain.engine = GrainEngine::Spektra;
        p.debug.deactivate_stochastic_effects = false;
        for (route, scanner, enlarger) in [
            ("input > film > scan", true, false),
            ("input > film > print > scan", false, true),
            ("input > film", false, false),
        ] {
            p.workflow.route = route.into();
            p.settings.use_scanner_lut = scanner;
            p.settings.use_enlarger_lut = enlarger;
            for grain in [false, true] {
                p.film_render.grain.active = grain;
                p.debug.lut_mode = false;
                let active_without_lut_mode = scanner || enlarger || grain;
                assert_eq!(
                    inactive_reason(path, &p, &color, &color).is_none(),
                    active_without_lut_mode,
                    "{route} grain={grain}"
                );
                p.debug.lut_mode = true;
                assert!(inactive_reason_without_profiles(path, &p).is_some());
                assert!(inactive_reason_with_materials(path, &p, None, None).is_some());
                assert!(inactive_reason(path, &p, &color, &color).is_some());
                assert!(
                    conditions(path)
                        .into_iter()
                        .any(|c| !c.holds_for(&p, Some(&color)))
                );
                for component in [None, Some(0), Some(1), Some(2)] {
                    assert_eq!(
                        grain_control_inactive_reason(path, &p, component),
                        Some(Condition::LutModeOff.description())
                    );
                    assert_eq!(
                        grain_control_inactive_reason_with_profile(path, &p, &color, component),
                        Some(Condition::LutModeOff.description())
                    );
                }
            }
        }
    }

    #[test]
    fn density_min_continuation_uses_restored_lut_values_without_image_preparation() {
        let path = "film_render.grain.density_min";
        let bw = film(false, true);
        let mut p = RuntimeParams::default();
        p.film_render.grain.engine = GrainEngine::Procedural;
        p.film_render.grain.monochrome = true;
        p.film_render.grain.active = false;
        p.debug.lut_mode = true;
        p.debug.deactivate_stochastic_effects = true;
        // Continuations neither reapply the debug constraints nor broadcast
        // monochrome density_min before a downstream LUT consumes its bounds.
        for (route, scanner, enlarger) in [
            ("film > scan", true, false),
            ("film > print", false, true),
            ("film > print > scan", false, true),
        ] {
            p.workflow.route = route.into();
            p.settings.use_scanner_lut = scanner;
            p.settings.use_enlarger_lut = enlarger;
            assert!(inactive_reason_without_profiles(path, &p).is_none());
            assert!(inactive_reason_with_materials(path, &p, None, None).is_none());
            assert!(inactive_reason(path, &p, &bw, &bw).is_none());
            assert!(
                conditions(path)
                    .into_iter()
                    .all(|c| c.holds_for(&p, Some(&bw)))
            );
            for component in [None, Some(0), Some(1), Some(2)] {
                assert!(grain_control_inactive_reason(path, &p, component).is_none());
                assert!(
                    grain_control_inactive_reason_with_profile(path, &p, &bw, component).is_none()
                );
                assert!(Condition::DensityMinimum(component).holds_for(&p, Some(&bw)));
            }
        }
        p.workflow.route = "film > scan".into();
        p.settings.use_scanner_lut = true;
        p.settings.use_enlarger_lut = false;
        p.scanner.scan_output = "negative_to_positive".into();
        assert!(inactive_reason(path, &p, &bw, &bw).is_some());
        p.scanner.scan_output = "direct_scan".into();
        p.settings.use_scanner_lut = false;
        assert!(inactive_reason(path, &p, &bw, &bw).is_some());
        p.workflow.route = "print > scan".into();
        p.settings.use_scanner_lut = true;
        assert!(inactive_reason(path, &p, &bw, &bw).is_some());
    }

    #[test]
    fn metering_follows_input_preprocessing_not_film_exposure() {
        let material = film(true, true);
        let mut p = RuntimeParams::default();
        let active = |path: &str, p: &RuntimeParams| {
            inactive_reason(path, p, &material, &material).is_none()
        };
        p.camera.auto_exposure = true;
        p.workflow.route = "input".into();
        assert!(!active("camera.auto_exposure", &p));
        p.taps.collect = Some("rgb_pre".into());
        assert!(active("camera.auto_exposure", &p));
        assert!(active("camera.auto_exposure_method", &p));
        assert!(!active("camera.capture_illuminant", &p));
        p.workflow.route = "input > film > scan".into();
        assert!(active("camera.auto_exposure_method", &p));
        assert!(!active("camera.capture_illuminant", &p));
        p.workflow.route = "input > convert-film > scan".into();
        p.taps.collect = Some("log_e_film".into());
        assert!(!active("camera.auto_exposure", &p));
        assert!(!active("camera.auto_exposure_method", &p));
        assert!(active("camera.capture_illuminant", &p));
        p.workflow.route = "input > film > scan".into();
        p.taps.inject = Some("log_e_film".into());
        p.taps.collect = Some("cmy_film".into());
        assert!(!active("camera.auto_exposure", &p));
        p.taps.inject = None;
        p.taps.collect = None;
        p.camera.auto_exposure = false;
        assert!(active("camera.auto_exposure", &p));
        assert!(!active("camera.auto_exposure_method", &p));
    }

    #[test]
    fn missing_material_context_does_not_claim_chemistry_support() {
        let mut p = RuntimeParams::default();
        p.print_render.density_curves_morph.active = true;
        assert!(
            inactive_reason_with_materials(
                "print_render.density_curves_morph.gamma_factor",
                &p,
                None,
                None
            )
            .is_some()
        );
        assert!(
            inactive_reason_with_materials(
                "print_render.density_curves_morph.active",
                &p,
                None,
                None
            )
            .is_none()
        );
        p.workflow.route = "input".into();
        let reason = inactive_reason_with_materials(
            "print_render.density_curves_morph.gamma_factor",
            &p,
            None,
            None,
        )
        .unwrap();
        assert_eq!(reason, Condition::PrintDevelopment.description());
    }

    #[test]
    fn material_interpretation_and_corrective_selectors_survive_roundtrip() {
        let material = film(true, true);
        let mut p = RuntimeParams::default();
        p.film_render.halation.active = false;
        p.camera.diffusion_filter.active = false;
        p.film_render.base.scale = 1.4;
        p.print_render.base.scale = 1.3;
        p.enlarger.preflash_exposure = 0.25;
        let stored = serde_json::to_value(&p).unwrap();
        let mut p: RuntimeParams = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(serde_json::to_value(&p).unwrap(), stored);
        for path in [
            "film_render.halation.active",
            "camera.diffusion_filter.active",
            "film_render.grain.engine",
            "print_render.glare.active",
        ] {
            assert!(
                inactive_reason(path, &p, &material, &material).is_none(),
                "{path}"
            );
        }
        p.workflow.route = "film > scan".into();
        assert!(inactive_reason("film_render.base.scale", &p, &material, &material).is_none());
        assert!(inactive_reason("camera.capture_illuminant", &p, &material, &material).is_some());
        assert!(inactive_reason("enlarger.preflash_exposure", &p, &material, &material).is_some());
        p.workflow.route = "print > scan".into();
        assert!(inactive_reason("print_render.base.scale", &p, &material, &material).is_none());
        assert!(inactive_reason("film_render.base.scale", &p, &material, &material).is_some());
        p.workflow.route = "input > film > print > scan".into();
        assert_eq!(serde_json::to_value(&p).unwrap(), stored);
    }

    #[test]
    fn procedural_resolution_follows_effective_mode_and_profile_ownership() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Procedural;
        params.film_render.grain.procedural_profile = "custom".into();
        let factor = "film_render.grain.procedural_resolution_factor";
        let method = "film_render.grain.procedural_resolution_type";
        for path in [factor, method] {
            assert_eq!(grain_control_inactive_reason(path, &params, None), None);
        }
        params.film_render.grain.procedural_mode = ProceduralGrainMode::Noise;
        for path in [factor, method] {
            assert_eq!(
                grain_control_inactive_reason(path, &params, None),
                Some(Condition::Analogue.description())
            );
        }
        params.film_render.grain.procedural_profile = "35mm250".into();
        // Named profiles are Analogue even when a stored Custom request is Noise.
        assert_eq!(grain_control_inactive_reason(method, &params, None), None);
        assert_eq!(
            grain_control_inactive_reason(factor, &params, None),
            Some(Condition::Custom.description())
        );
        for path in [
            "film_render.grain.procedural_amount",
            "film_render.grain.procedural_timer",
        ] {
            assert_eq!(grain_control_inactive_reason(path, &params, None), None);
        }
        for path in [
            "film_render.grain.procedural_mode",
            "film_render.grain.procedural_size",
        ] {
            assert_eq!(
                grain_control_inactive_reason(path, &params, None),
                Some(Condition::Custom.description())
            );
        }
        params.film_render.grain.engine = GrainEngine::Spektra;
        assert_eq!(
            grain_control_inactive_reason(method, &params, None),
            Some(Condition::Procedural.description())
        );
    }

    #[test]
    fn procedural_film_type_is_metadata_in_every_processing_mode() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Procedural;
        let path = "film_render.grain.procedural_film_type";
        for profile in ["35mm250", "custom"] {
            params.film_render.grain.procedural_profile = profile.into();
            for mode in [ProceduralGrainMode::Analogue, ProceduralGrainMode::Noise] {
                params.film_render.grain.procedural_mode = mode;
                assert_eq!(
                    grain_control_inactive_reason(path, &params, None),
                    Some(Condition::MetadataNoOp.description())
                );
                let material = film(true, true);
                assert_eq!(
                    inactive_reason(path, &params, &material, &material).as_deref(),
                    Some(Condition::MetadataNoOp.description())
                );
            }
        }
        let metadata = crate::params::sources::describe_field(path).unwrap();
        assert!(
            metadata
                .to_string()
                .contains(Condition::MetadataNoOp.description())
        );
    }

    #[test]
    fn monochrome_dye_marks_shared_material_controls_inactive() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Dye;
        params.film_render.grain.monochrome = true;
        let bw = film(false, true);
        let color = film(true, true);
        for path in [
            "film_render.grain.rms_granularity",
            "film_render.grain.density_min",
            "film_render.grain.uniformity",
        ] {
            for component in [None, Some(0), Some(1), Some(2)] {
                assert_eq!(
                    grain_control_inactive_reason(path, &params, component),
                    Some(Condition::Spektra.description())
                );
                assert_eq!(
                    grain_control_inactive_reason_with_profile(path, &params, &bw, component),
                    Some(Condition::Spektra.description())
                );
                assert_eq!(
                    grain_control_inactive_reason_with_profile(path, &params, &color, component),
                    Some(Condition::Spektra.description())
                );
            }
        }
        // Spektra keeps its record-0 behavior for the same shared fields.
        params.film_render.grain.engine = GrainEngine::Spektra;
        for path in [
            "film_render.grain.rms_granularity",
            "film_render.grain.density_min",
            "film_render.grain.uniformity",
        ] {
            assert_eq!(grain_control_inactive_reason(path, &params, None), None);
            for component in [Some(1), Some(2)] {
                assert_eq!(
                    grain_control_inactive_reason_with_profile(path, &params, &color, component),
                    None
                );
                assert_eq!(
                    grain_control_inactive_reason_with_profile(path, &params, &bw, component),
                    Some(Condition::MaterialRecord(component).description())
                );
            }
        }
    }

    #[test]
    fn spektra_curve_controls_do_not_require_layered_profile_data() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Spektra;
        for material in [film(true, true), film(true, false)] {
            for path in [
                "film_render.grain.rms_granularity",
                "film_render.grain.particle_scale_sublayers",
                "film_render.grain.blur_dye_clouds_um",
                "film_render.grain.micro_structure",
                "film_render.grain.mult_usm_sigma",
                "film_render.grain.mult_usm_amount",
            ] {
                assert_eq!(
                    grain_control_inactive_reason_with_profile(path, &params, &material, None),
                    None,
                    "{path}"
                );
                assert_eq!(inactive_reason(path, &params, &material, &material), None);
            }
        }
        for engine in [GrainEngine::Procedural, GrainEngine::Fotufilm] {
            params.film_render.grain.engine = engine;
            assert_eq!(
                grain_control_inactive_reason("film_render.grain.rms_granularity", &params, None),
                Some(Condition::Spektra.description())
            );
        }
    }

    #[test]
    fn configured_applicability_ignores_preview_bypass_but_respects_enable() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Dye;
        params.settings.preview_mode = true;
        let path = "film_render.grain.dye_strength";
        assert_eq!(grain_control_inactive_reason(path, &params, None), None);
        params.debug.deactivate_stochastic_effects = true;
        assert_eq!(
            grain_control_inactive_reason(path, &params, None),
            Some(Condition::Grain.description())
        );
        params.debug.deactivate_stochastic_effects = false;
        params.film_render.grain.active = false;
        assert_eq!(
            grain_control_inactive_reason(path, &params, None),
            Some(Condition::Grain.description())
        );
        for path in ["film_render.grain.active", "film_render.grain.engine"] {
            assert_eq!(grain_control_inactive_reason(path, &params, None), None);
        }
    }

    #[test]
    fn halation_boost_depends_on_halation_and_its_own_switch() {
        let material = film(true, true);
        let reason = |path: &str, params: &RuntimeParams| {
            inactive_reason(path, params, &material, &material)
        };
        let mut params = RuntimeParams::default();

        // The switch owns the module and is never inactive.
        assert_eq!(reason("film_render.halation.active", &params), None);
        // A zero boost is a normal off state for the switch itself.
        assert_eq!(reason("film_render.halation.boost_ev", &params), None);
        // The shape parameters need a nonzero boost.
        for path in [
            "film_render.halation.boost_range",
            "film_render.halation.protect_ev",
        ] {
            assert_eq!(
                reason(path, &params).as_deref(),
                Some(Condition::Boost.description()),
                "{path}"
            );
        }

        params.film_render.halation.boost_ev = 1.0;
        for path in [
            "film_render.halation.boost_ev",
            "film_render.halation.boost_range",
            "film_render.halation.protect_ev",
        ] {
            assert_eq!(reason(path, &params), None, "{path}");
        }

        params.debug.lut_mode = true;
        for path in [
            "film_render.halation.boost_ev",
            "film_render.halation.boost_range",
            "film_render.halation.protect_ev",
        ] {
            assert_eq!(
                reason(path, &params).as_deref(),
                Some(Condition::LutModeOff.description()),
                "{path}"
            );
        }

        params.debug.lut_mode = false;
        params.film_render.halation.active = false;
        for path in [
            "film_render.halation.boost_ev",
            "film_render.halation.boost_range",
            "film_render.halation.protect_ev",
        ] {
            assert_eq!(
                reason(path, &params).as_deref(),
                Some(Condition::Halation.description()),
                "{path}"
            );
        }
    }
}
