//! Conditions shared by field discovery and winning-edit feedback.
use crate::params::{GrainEngine, RuntimeParams};
use crate::profile::Profile;

#[derive(Clone, Copy)]
pub(super) enum Condition {
    NoOp,
    Grain,
    V1,
    V2,
    V3,
    V1OrV3,
    Custom,
    Sublayers,
    V1Sublayers,
    NoSublayers,
    Spatial,
    Diffusion(bool),
    Halation,
    Boost,
    Glare,
    Couplers,
    AutoExposure,
    Crop,
    ScannerPoints,
    PositiveScan,
    LabScanFinish,
    FilmScan,
    MagazineActive,
    MagazineStrength,
}
impl Condition {
    pub(super) fn description(self) -> &'static str {
        match self {
            Self::NoOp => "preserved upstream no-op",
            Self::Grain => "film_render.grain.active = true; stochastic effects enabled",
            Self::V1 => "film_render.grain.engine = v1",
            Self::V2 => "film_render.grain.engine = v2",
            Self::V3 => "film_render.grain.engine = v3; supported stock, format and route",
            Self::V1OrV3 => "film_render.grain.engine = v1 or v3",
            Self::Custom => {
                "film_render.grain.v2_profile = custom; select custom to use this control"
            }
            Self::Sublayers => {
                "film_render.grain.sublayers_active = true; film profile supplies sublayer density curves"
            }
            Self::V1Sublayers => {
                "when engine = v1: film_render.grain.sublayers_active = true; film profile supplies sublayer density curves"
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
            Self::ScannerPoints => {
                "slide film scan or negative film scan with scanner.scan_output = positive_scan; ignored for paper and direct negative scans"
            }
            Self::PositiveScan => "scanner.scan_output = positive_scan",
            Self::LabScanFinish => "scanner.finish = lab_scan",
            Self::FilmScan => "film-scan route",
            Self::MagazineActive => "magazine_print_color.active = true",
            Self::MagazineStrength => "magazine_print_color.strength > 0",
        }
    }
    fn holds(self, p: &RuntimeParams, film: &Profile) -> bool {
        let g = &p.film_render.grain;
        match self {
            Self::NoOp => false,
            Self::Grain => g.active && !p.debug.deactivate_stochastic_effects,
            Self::V1 => g.engine == GrainEngine::V1,
            Self::V2 => g.engine == GrainEngine::V2,
            Self::V3 => g.engine == GrainEngine::V3,
            Self::V1OrV3 => matches!(g.engine, GrainEngine::V1 | GrainEngine::V3),
            Self::Custom => g.v2_profile == "custom",
            Self::Sublayers => g.sublayers_active && !film.data.density_curves_layers.is_empty(),
            Self::V1Sublayers => {
                g.engine != GrainEngine::V1
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
            Self::ScannerPoints => {
                (p.io.scan_film || p.workflow.route == "input > convert-film > scan")
                    && (film.is_positive() || p.scanner.scan_output == "positive_scan")
            }
            Self::PositiveScan => p.scanner.scan_output == "positive_scan",
            Self::LabScanFinish => p.scanner.finish == crate::params::ScannerFinish::LabScan,
            Self::FilmScan => p.io.scan_film || p.workflow.route == "input > convert-film > scan",
            Self::MagazineActive => p.magazine_print_color.active,
            Self::MagazineStrength => p.magazine_print_color.strength > 0.0,
        }
    }
}
pub(super) fn conditions(path: &str) -> Vec<Condition> {
    use Condition::*;
    let mut out = Vec::new();
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
        if path.contains(".v2_") {
            out.push(V2);
            if !["v2_profile", "v2_amount", "v2_timer", "v2_resolution_type"]
                .iter()
                .any(|f| path.ends_with(f))
            {
                out.push(Custom);
            }
        } else if path.contains(".v3_") || path.ends_with(".rms_granularity") {
            out.push(V3);
        } else if ["density_min", "uniformity", "particle_scale_sublayers"]
            .iter()
            .any(|f| path.ends_with(f))
        {
            out.push(V1OrV3);
            if path.ends_with(".particle_scale_sublayers") {
                out.push(V1Sublayers);
            }
        } else {
            out.push(V1);
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
    } else if matches!(path, "scanner.black_point" | "scanner.white_point") {
        out.push(ScannerPoints);
    } else if path == "scanner.finish_strength" {
        out.extend([PositiveScan, LabScanFinish]);
    } else if path == "scanner.receiver_profile" {
        out.push(FilmScan);
    } else if path.starts_with("scanner.positive_")
        || matches!(path, "scanner.auto_setup" | "scanner.finish")
    {
        out.push(PositiveScan);
    } else if path.starts_with("magazine_print_color.") {
        out.push(if path.ends_with(".strength") {
            MagazineActive
        } else {
            MagazineStrength
        });
    }
    out
}
pub(crate) fn inactive_reason(
    path: &str,
    params: &RuntimeParams,
    film: &Profile,
) -> Option<String> {
    conditions(path)
        .into_iter()
        .find(|condition| !condition.holds(params, film))
        .map(|condition| condition.description().into())
}
