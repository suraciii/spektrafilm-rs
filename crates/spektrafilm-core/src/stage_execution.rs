//! Finite material execution and completed-state ownership rules.
use crate::params::{RuntimeParams, grain::GrainEngine};
use crate::profile::Profile;
use crate::stage_artifact::{MaterialKind, MaterialState, StageHeader};
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use std::path::Path;

pub enum StageResult {
    Material(MaterialState),
    Image(ImageBuf),
}

/// Validate without allocating image data or initializing a compute device.
pub fn validate_stage_params(film: &Profile, params: &RuntimeParams) -> Result<(), String> {
    params.validate()?;
    params.validate_color()?;
    let kind = params
        .workflow
        .route_enum()
        .artifact_output()
        .ok_or("route does not produce a material state")?;
    validate_stage_model(params, kind)?;
    crate::stages::grain_v3::validate_supported(film, params)?;
    crate::stages::grain_fotufilm::validate_supported(film, params)?;
    Ok(())
}

pub(crate) fn validate_stage_model(
    params: &RuntimeParams,
    kind: MaterialKind,
) -> Result<(), String> {
    if params.taps.inject.is_some() || params.taps.collect.is_some() {
        return Err("stage artifacts cannot use diagnostic taps.inject or taps.collect".into());
    }
    if crate::stages::grain_v3::active(params) {
        if crate::stages::grain_v3::cell_spectral_enabled()? && kind == MaterialKind::Film {
            return Err("Dye exact-cell optical readout fuses film development with acquisition: cmy_film is not a complete material state; choose the representable print boundary".into());
        }
        if crate::stages::grain_v3::aperture_factor()?.is_some() {
            return Err(
                "Dye aperture optical integration cannot be persisted at a material boundary"
                    .into(),
            );
        }
    }
    Ok(())
}

/// Ownership follows executed operations, not the GUI section containing a leaf.
pub fn is_frozen_parameter(kind: MaterialKind, path: &str, baseline: &RuntimeParams) -> bool {
    if path == "workflow.route" || path == "io.scan_film" {
        return false;
    }
    if path.starts_with("scanner.")
        || path.starts_with("cmyk.")
        || path.starts_with("output_mix.")
        || path == "io.output_color_space"
        || path == "io.output_cctf_encoding"
        || path.starts_with("io.output_gamut_compress.")
    {
        return false;
    }
    if path.starts_with("film_render.grain.") {
        // Procedural is a post-scan appearance; a formed material realization is immutable.
        return !(path == "film_render.grain.engine"
            || path == "film_render.grain.active"
            || path.starts_with("film_render.grain.v2_")
            || baseline.film_render.grain.engine == GrainEngine::Procedural);
    }
    if kind == MaterialKind::Film
        && (path.starts_with("enlarger.")
            || path.starts_with("print_render.")
            || path == "settings.print_exposure_ev_tracking"
            || path == "settings.neutral_print_filters_from_database"
            || path == "settings.use_enlarger_lut")
    {
        return false;
    }
    if kind == MaterialKind::Print && path.starts_with("print_render.glare.") {
        return false;
    }
    !matches!(path, "settings.use_scanner_lut" | "settings.preview_mode")
}

/// Validate each explicit source independently, before later overrides mask an edit.
pub fn validate_frozen_controls(
    header: &StageHeader,
    params: &RuntimeParams,
) -> Result<(), String> {
    let baseline = serde_json::to_value(&header.baseline).map_err(|e| e.to_string())?;
    let requested = serde_json::to_value(params).map_err(|e| e.to_string())?;
    fn check(
        path: &str,
        old: &serde_json::Value,
        new: &serde_json::Value,
        header: &StageHeader,
    ) -> Result<(), String> {
        if old == new {
            return Ok(());
        }
        if let (Some(a), Some(b)) = (old.as_object(), new.as_object()) {
            for (key, value) in b {
                let path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                check(
                    &path,
                    a.get(key).unwrap_or(&serde_json::Value::Null),
                    value,
                    header,
                )?;
            }
            return Ok(());
        }
        if is_frozen_parameter(header.state, path, &header.baseline) {
            return Err(format!(
                "{path}: completed material state is immutable; regenerate from the earlier input"
            ));
        }
        Ok(())
    }
    check("", &baseline, &requested, header)?;
    if params.film_render.grain.engine != header.baseline.film_render.grain.engine
        && params.film_render.grain.engine != GrainEngine::Procedural
    {
        return Err("film_render.grain.engine: new material grain belongs before the completed boundary; procedural is post-scan, other engines require regeneration from earlier input".into());
    }
    if params.film_render.grain.engine != GrainEngine::Procedural
        && params.film_render.grain.active != header.baseline.film_render.grain.active
    {
        return Err("film_render.grain.active: formed material grain cannot be added or removed after the completed boundary; regenerate from earlier input".into());
    }
    Ok(())
}

pub fn validate_continuation(header: &StageHeader, params: &RuntimeParams) -> Result<(), String> {
    params.validate()?;
    params.validate_color()?;
    let route = params.workflow.route_enum();
    if route.artifact_input() != Some(header.state) {
        return Err(format!(
            "route {:?} does not start with the artifact's {:?} material state",
            params.workflow.route, header.state
        ));
    }
    if params.taps.inject.is_some() || params.taps.collect.is_some() {
        return Err("stage artifacts cannot use diagnostic taps.inject or taps.collect".into());
    }
    validate_frozen_controls(header, params)?;
    if params.output_mix.amount < 1.0 && header.context.prepared_source_color_space.is_none() {
        return Err("output_mix.amount below 1 requires a retained prepared-source branch; regenerate the artifact with source mixing enabled".into());
    }
    crate::pipeline_calibration::validate_scan_output(params, &header.film.profile)?;
    Ok(())
}

pub fn continue_stage(
    state: MaterialState,
    params: RuntimeParams,
    print: Option<Profile>,
    data_dir: &Path,
    backend: &dyn ComputeBackend,
    seed: u64,
) -> Result<StageResult, String> {
    crate::pipeline::Pipeline::continue_material(state, params, print, data_dir, backend, seed)
}
