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
    ColorRecords,
    MaterialRecord(Option<usize>),
    Analogue,
    Fotufilm,
    SpektraOrDye,
    Custom,
    Spatial,
    Diffusion(bool),
    Halation,
    Boost,
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
    Chemistry(bool),
}
impl Condition {
    pub(super) fn description(self) -> &'static str {
        match self {
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
            Self::ColorRecords => {
                "selected film has color records; channel_correlation is retained but inactive for single-record black-and-white material"
            }
            Self::MaterialRecord(_) => {
                "Spektra and Dye use record 0 for black-and-white material; G/B values remain stored and inactive"
            }
            Self::Analogue => {
                "effective Procedural mode = analogue; Noise does not use Film Resolution or its method"
            }
            Self::Fotufilm => "film_render.grain.engine = fotufilm",
            Self::SpektraOrDye => "film_render.grain.engine = spektra or dye",
            Self::Custom => {
                "film_render.grain.v2_profile = custom; select custom to use this control"
            }
            Self::Spatial => "spatial effects enabled",
            Self::Diffusion(false) => {
                "camera.diffusion_filter.active = true; strength and spatial_scale positive"
            }
            Self::Diffusion(true) => {
                "enlarger.diffusion_filter.active = true; strength and spatial_scale positive; io.scan_film = false"
            }
            Self::Halation => "film_render.halation.active = true",
            Self::Boost => "film_render.halation.boost_ev != 0; LUT mode disabled",
            Self::Glare => {
                "io.scan_film = false; print_render.glare.active = true; percent > 0; stochastic effects enabled"
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
        }
    }
    fn holds(self, p: &RuntimeParams, film: &Profile) -> bool {
        self.holds_for(p, Some(film))
    }
    fn holds_for(self, p: &RuntimeParams, film: Option<&Profile>) -> bool {
        let g = &p.film_render.grain;
        match self {
            Self::ScanOperator => {
                p.scanner.requires_full_frame()
                    && p.workflow.route_enum().artifact_output().is_none()
                    && film.is_some_and(|film| {
                        crate::pipeline_calibration::validate_scan_output(p, film).is_ok()
                    })
            }
            Self::NoOp | Self::MetadataNoOp => false,
            Self::Grain => g.active && !p.debug.deactivate_stochastic_effects,
            Self::Spektra => g.engine == GrainEngine::Spektra,
            Self::Procedural => g.engine == GrainEngine::Procedural,
            Self::Dye => g.engine == GrainEngine::Dye,
            Self::ColorRecords => film.map_or(!g.monochrome, Profile::is_color),
            Self::MaterialRecord(component) => {
                component.is_none_or(|component| component == 0)
                    || film.map_or(!g.monochrome, Profile::is_color)
            }
            Self::Analogue => {
                g.v2_profile != "custom" || g.v2_mode == crate::params::grain::GrainV2Mode::Analogue
            }
            Self::Fotufilm => g.engine == GrainEngine::Fotufilm,
            Self::SpektraOrDye => matches!(g.engine, GrainEngine::Spektra | GrainEngine::Dye),
            Self::Custom => g.v2_profile == "custom",

            Self::Spatial => !p.debug.deactivate_spatial_effects,
            Self::Diffusion(print) => {
                let d = if print {
                    &p.enlarger.diffusion_filter
                } else {
                    &p.camera.diffusion_filter
                };
                d.active && d.strength > 0.0 && d.spatial_scale > 0.0 && (!print || !p.io.scan_film)
            }
            Self::Halation => p.film_render.halation.active,
            Self::Boost => p.film_render.halation.boost_ev != 0.0 && !p.debug.lut_mode,
            Self::Glare => {
                !p.io.scan_film
                    && p.print_render.glare.active
                    && p.print_render.glare.percent > 0.0
                    && !p.debug.deactivate_stochastic_effects
            }
            Self::Couplers => p.film_render.dir_couplers.active,
            Self::AutoExposure => p.camera.auto_exposure,
            Self::Crop => p.io.crop,
            Self::CmykActive => p.cmyk.active,
            Self::CmykStrength => p.cmyk.strength > 0.0,
            Self::FilmDevelopment => p.workflow.route_enum().exposes_film(),
            Self::PrintDevelopment => p.workflow.route_enum().prints(),
            Self::Preflash => p.enlarger.preflash_exposure > 0.0,
            Self::CaptureIlluminant => p.workflow.route_enum().exposes_film(),
            Self::Chemistry(print) => {
                if print {
                    p.print_render.density_curves_morph.active
                } else {
                    p.film_render.chemistry.active
                }
            }
        }
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
    } else if path.contains(".v2_") {
        out[1] = Some(Procedural);
        if path.ends_with(".v2_film_type") {
            out[2] = Some(MetadataNoOp);
        } else {
            if !["v2_profile", "v2_amount", "v2_timer", "v2_resolution_type"]
                .iter()
                .any(|field| path.ends_with(field))
            {
                out[2] = Some(Custom);
            }
            if path.ends_with(".v2_resolution_factor") || path.ends_with(".v2_resolution_type") {
                out[3] = Some(Analogue);
            }
        }
    } else if path.contains(".v3_") || path.ends_with(".channel_correlation") {
        out[1] = Some(Dye);
        if path.ends_with(".channel_correlation") {
            out[2] = Some(ColorRecords);
        }
    } else if [
        "rms_granularity",
        "density_min",
        "uniformity",
        "particle_scale_sublayers",
    ]
    .iter()
    .any(|field| path.ends_with(field))
    {
        out[1] = Some(SpektraOrDye);
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
    grain_conditions(path, component)
        .into_iter()
        .flatten()
        .find(|condition| !condition.holds_for(params, film))
        .map(Condition::description)
}
pub(super) fn conditions(path: &str) -> Vec<Condition> {
    use Condition::*;
    let mut out = Vec::new();
    if path.starts_with("scanner.operator.") {
        return vec![ScanOperator];
    }
    if path.starts_with("enlarger.preflash_") {
        out.push(PrintDevelopment);
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
    if path.starts_with("film_render.grain.") {
        return grain_conditions(path, None).into_iter().flatten().collect();
    }
    if path == "debug.print_timings" {
        return vec![NoOp];
    }
    if path.starts_with("print_render.glare.") {
        return vec![Glare];
    }
    if path.contains("diffusion_filter.") {
        out.extend([Spatial, Diffusion(path.starts_with("enlarger."))]);
    } else if matches!(
        path,
        "film_render.halation.boost_ev"
            | "film_render.halation.boost_range"
            | "film_render.halation.protect_ev"
    ) {
        out.push(Boost);
    } else if path.contains(".halation.") {
        out.extend([Halation, Spatial]);
    } else if path.starts_with("print_render.glare.") {
        out.push(Glare);
    } else if path.contains(".dir_couplers.") {
        out.push(Couplers);
    } else if path == "camera.auto_exposure_method" {
        out.push(AutoExposure);
    } else if path.starts_with("io.crop_") {
        out.push(Crop);
    } else if path == "camera.capture_illuminant" {
        out.push(CaptureIlluminant);
    } else if path.starts_with("cmyk.") {
        if path.ends_with(".strength") {
            out.push(CmykActive);
            out.push(CmykStrength);
        } else {
            out.push(CmykActive);
        }
    }
    out
}
pub fn inactive_reason(
    path: &str,
    params: &RuntimeParams,
    film: &Profile,
    print: &Profile,
) -> Option<String> {
    if path.starts_with("film_render.grain.") {
        return grain_control_inactive_reason_with_profile(path, params, film, None)
            .map(str::to_owned);
    }
    if let Some(reason) = conditions(path)
        .into_iter()
        .find(|condition| !condition.holds(params, film))
        .map(|condition| condition.description().into())
    {
        return Some(reason);
    }
    let material = if path.starts_with("film_render.chemistry.") {
        Some(film)
    } else if path.starts_with("print_render.density_curves_morph.") {
        Some(print)
    } else {
        None
    };
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
    use crate::params::grain::GrainV2Mode;

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
    fn procedural_resolution_follows_effective_mode_and_profile_ownership() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Procedural;
        params.film_render.grain.v2_profile = "custom".into();
        let factor = "film_render.grain.v2_resolution_factor";
        let method = "film_render.grain.v2_resolution_type";
        for path in [factor, method] {
            assert_eq!(grain_control_inactive_reason(path, &params, None), None);
        }
        params.film_render.grain.v2_mode = GrainV2Mode::Noise;
        for path in [factor, method] {
            assert_eq!(
                grain_control_inactive_reason(path, &params, None),
                Some(Condition::Analogue.description())
            );
        }
        params.film_render.grain.v2_profile = "35mm250".into();
        // Named profiles are Analogue even when a stored Custom request is Noise.
        assert_eq!(grain_control_inactive_reason(method, &params, None), None);
        assert_eq!(
            grain_control_inactive_reason(factor, &params, None),
            Some(Condition::Custom.description())
        );
        for path in ["film_render.grain.v2_amount", "film_render.grain.v2_timer"] {
            assert_eq!(grain_control_inactive_reason(path, &params, None), None);
        }
        for path in ["film_render.grain.v2_mode", "film_render.grain.v2_size"] {
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
        let path = "film_render.grain.v2_film_type";
        for profile in ["35mm250", "custom"] {
            params.film_render.grain.v2_profile = profile.into();
            for mode in [GrainV2Mode::Analogue, GrainV2Mode::Noise] {
                params.film_render.grain.v2_mode = mode;
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
    fn monochrome_dye_hides_record_lanes_but_not_numerical_components() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Dye;
        params.film_render.grain.monochrome = true;
        let before = params.clone();
        let bw = film(false, true);
        let color = film(true, true);
        for path in [
            "film_render.grain.rms_granularity",
            "film_render.grain.density_min",
            "film_render.grain.uniformity",
        ] {
            for component in [None, Some(0)] {
                assert_eq!(
                    grain_control_inactive_reason(path, &params, component),
                    None
                );
            }
            for component in [Some(1), Some(2)] {
                assert_eq!(
                    grain_control_inactive_reason(path, &params, component),
                    Some(Condition::MaterialRecord(component).description())
                );
                // Loaded material wins over a stale runtime monochrome flag.
                assert_eq!(
                    grain_control_inactive_reason_with_profile(path, &params, &color, component),
                    None
                );
            }
        }
        let coupling = "film_render.grain.channel_correlation";
        assert_eq!(
            grain_control_inactive_reason(coupling, &params, None),
            Some(Condition::ColorRecords.description())
        );
        assert_eq!(
            grain_control_inactive_reason_with_profile(coupling, &params, &color, None),
            None
        );
        for component in [Some(0), Some(1), Some(2)] {
            assert_eq!(
                grain_control_inactive_reason_with_profile(
                    "film_render.grain.particle_scale_sublayers",
                    &params,
                    &bw,
                    component
                ),
                None
            );
        }
        assert_eq!(
            params, before,
            "applicability must not mutate stored requests"
        );
        params.film_render.grain.monochrome = false;
        assert_eq!(grain_control_inactive_reason(coupling, &params, None), None);
        assert_eq!(
            grain_control_inactive_reason_with_profile(coupling, &params, &bw, None),
            Some(Condition::ColorRecords.description())
        );
        assert!(
            grain_control_inactive_reason_with_profile(
                "film_render.grain.uniformity",
                &params,
                &bw,
                Some(1)
            )
            .is_some()
        );
        params.film_render.grain.engine = GrainEngine::Spektra;
        for path in [
            "film_render.grain.rms_granularity",
            "film_render.grain.density_min",
            "film_render.grain.uniformity",
        ] {
            assert_eq!(
                grain_control_inactive_reason_with_profile(path, &params, &bw, Some(0)),
                None
            );
            for component in [Some(1), Some(2)] {
                assert_eq!(
                    grain_control_inactive_reason_with_profile(path, &params, &bw, component),
                    Some(Condition::MaterialRecord(component).description())
                );
                assert_eq!(
                    grain_control_inactive_reason_with_profile(path, &params, &color, component),
                    None
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
                Some(Condition::SpektraOrDye.description())
            );
        }
    }

    #[test]
    fn configured_applicability_ignores_preview_bypass_but_respects_enable() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.engine = GrainEngine::Dye;
        params.settings.preview_mode = true;
        let path = "film_render.grain.v3_event_area_um2";
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
}
