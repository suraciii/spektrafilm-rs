//! Conditions shared by field discovery and winning-edit feedback.
use crate::params::{GrainEngine, RuntimeParams};
use crate::profile::Profile;

#[derive(Clone, Copy)]
pub(super) enum Condition {
    NoOp,
    Grain,
    Spektra,
    Procedural,
    Dye,
    Fotufilm,
    SpektraOrDye,
    Custom,
    Sublayers,
    SpektraSublayers,
    NoSublayers,
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
    Chemistry(bool),
}
impl Condition {
    pub(super) fn description(self) -> &'static str {
        match self {
            Self::NoOp => "preserved upstream no-op",
            Self::Grain => "film_render.grain.active = true; stochastic effects enabled",
            Self::Spektra => "film_render.grain.engine = spektra",
            Self::Procedural => "film_render.grain.engine = procedural",
            Self::Dye => "film_render.grain.engine = dye; supported stock, format and route",
            Self::Fotufilm => "film_render.grain.engine = fotufilm",
            Self::SpektraOrDye => "film_render.grain.engine = spektra or dye",
            Self::Custom => {
                "film_render.grain.v2_profile = custom; select custom to use this control"
            }
            Self::Sublayers => {
                "film_render.grain.sublayers_active = true; film profile supplies sublayer density curves"
            }
            Self::SpektraSublayers => {
                "when engine = spektra: film_render.grain.sublayers_active = true; film profile supplies sublayer density curves"
            }
            Self::NoSublayers => "film_render.grain.sublayers_active = false",
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
        let g = &p.film_render.grain;
        match self {
            Self::NoOp => false,
            Self::Grain => g.active && !p.debug.deactivate_stochastic_effects,
            Self::Spektra => g.engine == GrainEngine::Spektra,
            Self::Procedural => g.engine == GrainEngine::Procedural,
            Self::Dye => g.engine == GrainEngine::Dye,
            Self::Fotufilm => g.engine == GrainEngine::Fotufilm,
            Self::SpektraOrDye => matches!(g.engine, GrainEngine::Spektra | GrainEngine::Dye),
            Self::Custom => g.v2_profile == "custom",
            Self::Sublayers => g.sublayers_active && !film.data.density_curves_layers.is_empty(),
            Self::SpektraSublayers => {
                g.engine != GrainEngine::Spektra
                    || (g.sublayers_active && !film.data.density_curves_layers.is_empty())
            }
            Self::NoSublayers => !g.sublayers_active,
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
pub(super) fn conditions(path: &str) -> Vec<Condition> {
    use Condition::*;
    let mut out = Vec::new();
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
    if path == "film_render.grain.micro_sublayers" || path == "debug.print_timings" {
        return vec![NoOp];
    }
    if path.starts_with("print_render.glare.") {
        return vec![Glare];
    }
    if path.starts_with("film_render.grain.") {
        if path.ends_with(".active") || path.ends_with(".engine") {
            return out;
        }
        out.push(Grain);
        if path.starts_with("film_render.grain.fotufilm.") {
            out.push(Fotufilm);
        } else if path.contains(".v2_") {
            out.push(Procedural);
            if !["v2_profile", "v2_amount", "v2_timer", "v2_resolution_type"]
                .iter()
                .any(|f| path.ends_with(f))
            {
                out.push(Custom);
            }
        } else if path.contains(".v3_")
            || path.ends_with(".rms_granularity")
            || path.ends_with(".channel_correlation")
        {
            out.push(Dye);
        } else if ["density_min", "uniformity", "particle_scale_sublayers"]
            .iter()
            .any(|f| path.ends_with(f))
        {
            out.push(SpektraOrDye);
            if path.ends_with(".particle_scale_sublayers") {
                out.push(SpektraSublayers);
            }
        } else {
            out.push(Spektra);
            if path.ends_with(".n_sub_layers") {
                out.push(NoSublayers);
            }
            if [
                "blur_dye_clouds_um",
                "micro_structure",
                "mult_usm_sigma",
                "mult_usm_amount",
            ]
            .iter()
            .any(|f| path.ends_with(f))
            {
                out.push(Sublayers);
            }
        }
    } else if path.contains("diffusion_filter.") {
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
