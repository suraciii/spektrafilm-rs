//! Process-only composition of legacy baselines, complete looks and sparse edits.
use crate::WorkflowRoute;
use crate::neutral_filters::NeutralFilters;
use crate::params::sources::lookup;
use crate::params::sources::{ParameterEdits, validate_finite};
use crate::params::validation::REMOVED_MAGAZINE_ROUTE;
use crate::params::{GrainEngine, RuntimeParams};
use crate::params_builder::{
    apply_database_neutral_print_filters_protected, apply_film_specifics,
    apply_runtime_constraints, digest_params, normalize_runtime_topology,
};
use crate::presets;
use crate::profile::{self, Profile};
use crate::runtime::DigestMode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

mod continuation;
pub use continuation::{ResolvedContinuation, resolve_continuation};

#[derive(Debug, Clone, Serialize)]
pub struct DeclarationSource {
    pub kind: String,
    pub option: Option<String>,
    pub index: Option<usize>,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InputDeclarationSources {
    pub color_space: DeclarationSource,
    pub cctf_decoding: DeclarationSource,
}

#[derive(Debug, Clone, Serialize)]
pub struct InputDeclaration {
    pub color_space: String,
    pub cctf_decoding: bool,
    pub sources: InputDeclarationSources,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EditStatus {
    Active,
    Inactive,
    Constrained,
    Overridden,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureProfileFeedback {
    pub family: String,
    pub name: String,
    pub source: DeclarationSource,
    pub scope: Vec<String>,
    pub effective: String,
    pub status: EditStatus,
    pub reason: Option<String>,
    pub resolved: Value,
    pub requested: Value,
}
#[derive(Debug, Clone, Serialize)]
pub struct EditFeedback {
    pub path: String,
    pub requested: Value,
    pub effective: Value,
    pub source: DeclarationSource,
    pub status: EditStatus,
    pub reason: Option<String>,
}

pub struct ProcessParamsRequest<'a> {
    pub film: Option<&'a str>,
    pub paper: Option<&'a str>,
    pub presets: &'a [presets::Preset],
    pub params_file: Option<&'a Path>,
    pub sources: &'a [String],
    pub feature_profiles: &'a [crate::feature_profiles::FeatureProfileSelection<'a>],
    pub route: Option<&'a str>,
    pub scan_film: bool,
    pub scan_output: Option<&'a str>,
    pub input_is_raw: bool,
    pub input_color_space: Option<&'a str>,
    pub input_cctf_decoding: Option<bool>,
    pub require_input_declaration: bool,
    pub digest_mode: DigestMode,
}

pub struct ResolvedProcessParams {
    pub film: Profile,
    pub print: Profile,
    pub film_name: String,
    pub print_name: String,
    pub params: RuntimeParams,
    pub input_declaration: InputDeclaration,
    pub edits: Vec<EditFeedback>,
    pub feature_profiles: Vec<FeatureProfileFeedback>,
}

fn equal_control_values(path: &str, requested: &Value, effective: &Value) -> bool {
    if requested == effective {
        return true;
    }
    let metadata = crate::params::sources::describe_field(path).expect("validated edit path");
    if metadata["type"]
        .as_str()
        .is_some_and(|ty| ty.contains("f32"))
    {
        fn equal_f32(a: &Value, b: &Value) -> bool {
            match (a, b) {
                (Value::Number(a), Value::Number(b)) => {
                    a.as_f64().map(|v| v as f32) == b.as_f64().map(|v| v as f32)
                }
                (Value::Array(a), Value::Array(b)) => {
                    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal_f32(a, b))
                }
                _ => a == b,
            }
        }
        equal_f32(requested, effective)
    } else {
        fn equal_numbers(a: &Value, b: &Value) -> bool {
            match (a, b) {
                (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
                (Value::Array(a), Value::Array(b)) => {
                    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal_numbers(a, b))
                }
                _ => a == b,
            }
        }
        equal_numbers(requested, effective)
    }
}

/// Return effective parameters ready for `Runtime::new`, without a second digest.
pub fn resolve(
    request: ProcessParamsRequest<'_>,
    data_dir: &Path,
) -> Result<ResolvedProcessParams, String> {
    resolve_with_preview_mode(request, data_dir, None)
}

fn resolve_with_preview_mode(
    request: ProcessParamsRequest<'_>,
    data_dir: &Path,
    preview_mode: Option<bool>,
) -> Result<ResolvedProcessParams, String> {
    let supplies_film = request
        .presets
        .iter()
        .any(|preset| preset.film_profile.is_some());
    let supplies_print = request
        .presets
        .iter()
        .any(|preset| preset.print_profile.is_some());
    if (supplies_film && request.film.is_some())
        || (supplies_print && request.paper.is_some())
        || ((supplies_film || supplies_print) && request.params_file.is_some())
    {
        return Err("--preset conflicts with --film, --paper and --params when it supplies their profile references".into());
    }
    let (mut params, baseline_granularity_authored) = load_baseline(request.params_file)?;
    let mut declarations = baseline_declarations(request.params_file)?;
    let mut winning = BTreeMap::new();
    // A saved removed route must be reported before any explicit route
    // override can mask it.
    if params.workflow.route == REMOVED_MAGAZINE_ROUTE {
        params.validate()?;
    }
    let parsed_sources: Vec<ParameterEdits> = request
        .sources
        .iter()
        .map(|source| ParameterEdits::parse(source).map_err(|e| format!("--set {source:?}: {e}")))
        .collect::<Result<_, _>>()?;
    // Stock defaults depend on the winning engine, not the engine carried by
    // an earlier baseline. Apply all other authored controls in normal order.
    let mut grain_engine = params.film_render.grain.engine;
    for preset in request.presets {
        preset.validate()?;
        match &preset.parameters {
            presets::PresetParameters::Look(look) => grain_engine = look.film_render.grain.engine,
            presets::PresetParameters::Section(payload) => {
                if let Some(value) = lookup(payload, "film_render.grain.engine") {
                    grain_engine = GrainEngine::deserialize(value).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    for edits in &parsed_sources {
        for (path, value) in edits.values() {
            if path == "film_render.grain.engine" {
                grain_engine = GrainEngine::deserialize(value).map_err(|e| e.to_string())?;
            }
        }
    }
    params.film_render.grain.engine = grain_engine;
    // Resolve omitted RMS by the winning engine before stock seeding. Presence,
    // never numeric equality, distinguishes a baseline's authored strength.
    if !baseline_granularity_authored {
        params.film_render.grain.rms_granularity = match grain_engine {
            GrainEngine::Spektra => [6.0, 8.0, 10.0],
            _ => [0.0; 3],
        };
    }
    let added_sources = !request.presets.is_empty() || !request.sources.is_empty();
    let profile_sources = !request.feature_profiles.is_empty();
    let mut resolved_profiles = None;
    let image_stage_route = request
        .route
        .map(WorkflowRoute::parse)
        .is_some_and(|route| route.artifact_output().is_some() && route.artifact_input().is_none());
    if image_stage_route {
        for (source, edits) in request.sources.iter().zip(&parsed_sources) {
            if edits
                .paths()
                .any(|p| matches!(p, "taps.inject" | "taps.collect"))
            {
                return Err(format!(
                    "--set {source:?}: taps.inject and taps.collect are inapplicable to stage artifacts"
                ));
            }
        }
        if let Some(path) = request.params_file {
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let value = crate::params::sources::parse_carrier(&text, false)?;
            if value.pointer("/taps/inject").is_some() || value.pointer("/taps/collect").is_some() {
                return Err(format!(
                    "--params {}: taps.inject and taps.collect are inapplicable to stage artifacts",
                    path.display()
                ));
            }
        }
    }
    if supplies_film && image_stage_route {
        for preset in request.presets {
            preset.validate()?;
            if let presets::PresetParameters::Look(look) = &preset.parameters {
                let film_ref = preset.film_profile.as_ref().expect("validated look");
                let film = load_referenced_profile(film_ref, data_dir)?;
                if !film.is_film() || !film.is_filming() {
                    return Err("film profile must have film/filming support/stage".into());
                }
                apply_film_specifics(&mut params, &film);
                look.apply_to(&mut params);
                let defaults = crate::params::EnlargerParams::default();
                params.neutral_print_filters_protected = Some([
                    look.enlarger.c_filter_neutral != defaults.c_filter_neutral,
                    look.enlarger.m_filter_neutral != defaults.m_filter_neutral,
                    look.enlarger.y_filter_neutral != defaults.y_filter_neutral,
                ]);
                let route = WorkflowRoute::parse(request.route.expect("stage route"));
                let (print, print_name) = if route.prints() {
                    let reference = preset.print_profile.as_ref().expect("validated look");
                    let print = load_referenced_profile(reference, data_dir)?;
                    if !print.is_print_stock() {
                        return Err("print profile must have printing support/stage".into());
                    }
                    (print, reference.stock.clone())
                } else {
                    (film.clone(), String::new())
                };
                resolved_profiles = Some((film, print, film_ref.stock.clone(), print_name));
            } else {
                params = preset.overlay_section(&params)?;
            }
        }
    } else if supplies_film {
        for preset in request.presets {
            preset.validate()?;
            // The requested route is part of the workflow context a look resolves
            // against: declaring a film-scan route here lets a document's scan
            // mode survive resolution instead of being normalized away.
            choose_workflow(&mut params, request.route, request.scan_film);
            let resolved = preset.resolve(data_dir, &params)?;
            params = resolved.params;
            resolved_profiles = Some((
                resolved.film,
                resolved.print,
                resolved.film_name,
                resolved.print_name,
            ));
        }
    }
    let (film, print, film_name, print_name) = if let Some(profiles) = resolved_profiles {
        profiles
    } else {
        choose_workflow(&mut params, request.route, request.scan_film);
        if params.workflow.route == REMOVED_MAGAZINE_ROUTE {
            params.validate()?;
        }
        let selected_route = params.workflow.route_enum();
        let film_only = selected_route == WorkflowRoute::InputFilm;
        let direct_scan = selected_route == WorkflowRoute::FilmScan
            || (!request.require_input_declaration && !added_sources && request.scan_film);
        let name = request
            .film
            .ok_or("--film is required when the preset set supplies no film reference")?;
        let film = profile::load_profile_by_name(data_dir, name)
            .map_err(|e| format!("loading film profile {name:?}: {e}"))?;
        let print_name = if film_only {
            String::new()
        } else if direct_scan {
            name.to_owned()
        } else if let Some(paper) = request.paper {
            paper.to_owned()
        } else {
            film.info.target_print.clone().ok_or(
                "no paper specified and film has no target_print — use --paper or --scan-film",
            )?
        };
        // A film terminal has no dependency on an unused future print selection.
        // The existing result shape retains a harmless in-memory profile carrier.
        let print = if film_only {
            film.clone()
        } else {
            profile::load_profile_by_name(data_dir, &print_name)
                .map_err(|e| format!("loading print profile {print_name:?}: {e}"))?
        };
        // A print material is not a film: it converts no encoded input to a
        // film exposure, so it carries no sensitivity-adaptation parameters
        // (specs/film-chain/spec.md). Rejecting it here names the user's
        // intent instead of failing later inside the spectral pipeline.
        // This check is unconditional: the legacy parameter-declaration path
        // reaches the same profile resolution and must not skip it.
        if !film.is_film() || !film.is_filming() {
            return Err(format!(
                "--film {name:?}: incompatible support/stage; expected film/filming"
            ));
        }
        if !direct_scan && !film_only && !print.is_print_stock() {
            return Err(format!(
                "print profile {print_name:?}: incompatible support/stage; expected paper/printing or film/printing"
            ));
        }
        if (added_sources || profile_sources)
            && request.digest_mode == DigestMode::ApplyStockSpecifics
        {
            apply_film_specifics(&mut params, &film);
        }
        (film, print, name.to_owned(), print_name)
    };
    if !supplies_film {
        for preset in request.presets {
            params = preset.overlay_section(&params)?;
        }
    }

    let mut feature_profile_feedback = Vec::new();
    if !request.feature_profiles.is_empty() {
        let baseline = params.clone();
        let (candidate, requested_values) =
            crate::feature_profiles::apply_with_requests(&baseline, request.feature_profiles)?;
        params = candidate;
        for (selection, requested) in request.feature_profiles.iter().zip(requested_values) {
            let origin = DeclarationSource {
                kind: "feature_profile".into(),
                option: Some("--feature-profile".into()),
                index: Some(selection.position),
                path: None,
            };
            for path in crate::feature_profiles::scope(selection.family) {
                if let Some(value) = requested.get(*path) {
                    winning.insert((*path).to_owned(), (value.clone(), origin.clone()));
                }
            }
            feature_profile_feedback.push((selection, requested, origin));
        }
    }
    if added_sources {
        for (index, (source, edits)) in request.sources.iter().zip(&parsed_sources).enumerate() {
            if request.input_is_raw {
                edits
                    .validate_raw_input()
                    .map_err(|e| format!("--set {source:?}: {e}"))?;
            }
            let origin = DeclarationSource {
                kind: "set".into(),
                option: Some("--set".into()),
                index: Some(index + 1),
                path: ParameterEdits::file_path(source)
                    .map(absolute_path)
                    .transpose()?,
            };
            for (path, value) in edits.values() {
                if matches!(path, "io.input_color_space" | "io.input_cctf_decoding") {
                    declarations.insert(path.to_owned(), origin.clone());
                }
                winning.insert(path.to_owned(), (value.clone(), origin.clone()));
            }
        }
    }
    for (path, value, option) in [
        (
            "io.input_color_space",
            request.input_color_space.map(|v| Value::String(v.into())),
            "--input-color-space",
        ),
        (
            "io.input_cctf_decoding",
            request.input_cctf_decoding.map(Value::Bool),
            "--input-cctf-decoding",
        ),
    ] {
        if let Some(value) = value {
            let edits =
                ParameterEdits::single(path, value).map_err(|e| format!("{option}: {e}"))?;
            if request.input_is_raw {
                edits
                    .validate_raw_input()
                    .map_err(|e| format!("{option}: {e}"))?;
            }
            edits.apply(&mut params)?;
            declarations.insert(
                path.into(),
                DeclarationSource {
                    kind: "argument".into(),
                    option: Some(option.into()),
                    index: None,
                    path: None,
                },
            );
        }
    }
    if request.require_input_declaration
        && !request.input_is_raw
        && (!declarations.contains_key("io.input_color_space")
            || !declarations.contains_key("io.input_cctf_decoding"))
    {
        return Err("raster input requires both io.input_color_space and io.input_cctf_decoding; declare both with --input-color-space and --input-cctf-decoding, supplied --params leaves, or --set".into());
    }
    if !added_sources && !profile_sources {
        // Keep the legacy decoder, route/scan relationship and digest ordering.
        choose_workflow(&mut params, request.route, request.scan_film);
        params.io.scan_film = params.workflow.route_enum() == WorkflowRoute::FilmScan
            || (!request.require_input_declaration && request.scan_film);
        if let Some(mode) = request.scan_output {
            params.scanner.scan_output = mode.to_owned();
        }
        params.validate()?;
        crate::pipeline_calibration::validate_scan_output(&params, &film)?;
        if request.input_is_raw {
            force_raw_input(&mut params);
        }
        params.validate_color()?;
        if let Some(mode) = preview_mode {
            params.settings.preview_mode = mode;
        }
        let neutral = NeutralFilters::load(data_dir)?;
        params = digest_params(
            params,
            &film,
            &print,
            Some(&neutral),
            request.digest_mode == DigestMode::ApplyStockSpecifics,
        );
    } else {
        let mut protected = params.neutral_print_filters_protected;
        for edits in &parsed_sources {
            edits.apply(&mut params)?;
            for path in edits.paths() {
                match path {
                    "enlarger.c_filter_neutral" => protected.get_or_insert([false; 3])[0] = true,
                    "enlarger.m_filter_neutral" => protected.get_or_insert([false; 3])[1] = true,
                    "enlarger.y_filter_neutral" => protected.get_or_insert([false; 3])[2] = true,
                    _ => {}
                }
            }
        }
        if let Some(value) = request.input_color_space {
            params.io.input_color_space = value.into();
        }
        if let Some(value) = request.input_cctf_decoding {
            params.io.input_cctf_decoding = value;
        }
        choose_workflow(&mut params, request.route, request.scan_film);
        params.io.scan_film = params.workflow.route_enum() == WorkflowRoute::FilmScan;
        if request.input_is_raw {
            force_raw_input(&mut params);
        }
        if let Some(mode) = request.scan_output {
            params.scanner.scan_output = mode.to_owned();
        }
        if let Some(mode) = preview_mode {
            params.settings.preview_mode = mode;
        }
        validate_finite(&params)?;
        params.validate()?;
        crate::pipeline_calibration::validate_scan_output(&params, &film)?;
        let neutral = NeutralFilters::load(data_dir)?;
        params = apply_database_neutral_print_filters_protected(
            params,
            &film,
            &print,
            Some(&neutral),
            protected,
        );
        params.neutral_print_filters_protected = protected;
        apply_runtime_constraints(&mut params);
        validate_finite(&params)?;
        params.validate()?;
    }

    normalize_runtime_topology(&film, &mut params);
    crate::pipeline_calibration::validate_scan_output(&params, &film)?;
    let raw_source = DeclarationSource {
        kind: "raw".into(),
        option: None,
        index: None,
        path: None,
    };
    // Internal protocol callers can retain implicit controls. Public callers cannot.
    let implicit_source = DeclarationSource {
        kind: "params".into(),
        option: None,
        index: None,
        path: None,
    };
    let declaration_source = |path: &str| {
        if request.input_is_raw {
            raw_source.clone()
        } else {
            declarations
                .get(path)
                .cloned()
                .unwrap_or_else(|| implicit_source.clone())
        }
    };
    let input_declaration = InputDeclaration {
        color_space: params.io.input_color_space.clone(),
        cctf_decoding: params.io.input_cctf_decoding,
        sources: InputDeclarationSources {
            color_space: declaration_source("io.input_color_space"),
            cctf_decoding: declaration_source("io.input_cctf_decoding"),
        },
    };
    let effective_params = serde_json::to_value(&params).map_err(|e| e.to_string())?;
    let feature_profiles = feature_profile_feedback
        .into_iter()
        .map(|(selection, requested, source)| {
            let scope = crate::feature_profiles::scope(selection.family);
            let effective = crate::feature_profiles::effective_values(selection.family, &params);
            let superseded = request.feature_profiles.iter().any(|other| {
                other.family == selection.family && other.position > selection.position
            });
            let overridden_path = scope.iter().find(|path| {
                winning.get(**path).is_some_and(|(_, winner)| {
                    winner.kind != "feature_profile" || winner.index != source.index
                })
            });
            let constrained_path = scope.iter().find(|path| {
                let requested_value = requested.get(*path).unwrap_or(&Value::Null);
                let effective_value = lookup(&effective_params, path).unwrap_or(&Value::Null);
                !equal_control_values(path, requested_value, effective_value)
            });
            let inactive_reason = scope.iter().find_map(|path| {
                crate::params::sources::inactive_reason(path, &params, &film, &print)
            });
            let (status, reason) = if superseded {
                (
                    EditStatus::Overridden,
                    Some("replaced by a later --feature-profile selection".into()),
                )
            } else if let Some(path) = overridden_path {
                (
                    EditStatus::Overridden,
                    Some(format!("scope leaf {path} was replaced by a later source")),
                )
            } else if let Some(path) = constrained_path {
                (
                    EditStatus::Constrained,
                    Some(format!("runtime preparation changed scope leaf {path}")),
                )
            } else if let Some(reason) = inactive_reason {
                (EditStatus::Inactive, Some(reason))
            } else {
                (EditStatus::Active, None)
            };
            FeatureProfileFeedback {
                family: selection.family.id().into(),
                name: selection.name.into(),
                source,
                scope: scope.iter().map(|path| (*path).into()).collect(),
                effective: crate::feature_profiles::effective_name(selection.family, &params)
                    .into(),
                status,
                reason,
                resolved: effective,
                requested,
            }
        })
        .collect();
    let edits = winning.into_iter().map(|(path, (requested, source))| {
        let effective = lookup(&effective_params, &path).cloned().unwrap_or(Value::Null);
        let override_option = match path.as_str() {
            "io.input_color_space" if request.input_color_space.is_some() => Some("--input-color-space"),
            "io.input_cctf_decoding" if request.input_cctf_decoding.is_some() => Some("--input-cctf-decoding"),
            "scanner.scan_output" if request.scan_output.is_some() => Some("--scan-output"),
            _ => None,
        };
        let (status, reason) = if let Some(option) = override_option {
            (EditStatus::Overridden, Some(format!("replaced by {option}")))
        } else if !equal_control_values(&path, &requested, &effective) {
            let mut causes = Vec::new();
            if params.settings.preview_mode { causes.push("settings.preview_mode"); }
            if params.debug.lut_mode { causes.push("debug.lut_mode"); }
            if params.debug.deactivate_spatial_effects { causes.push("debug.deactivate_spatial_effects"); }
            if params.debug.deactivate_stochastic_effects { causes.push("debug.deactivate_stochastic_effects"); }
            let reason = if causes.is_empty() { "runtime preparation changed the requested value for derived profile topology".into() } else { format!("runtime preparation changed the requested value with {} enabled; disable the applicable constraint to retain it", causes.join(", ")) };
            (EditStatus::Constrained, Some(reason))
        } else if let Some(reason) = crate::params::sources::inactive_reason(&path, &params, &film, &print) {
            (EditStatus::Inactive, Some(reason))
        } else { (EditStatus::Active, None) };
        EditFeedback { path, requested, effective, source, status, reason }
    }).collect();
    Ok(ResolvedProcessParams {
        film,
        print,
        film_name,
        print_name,
        params,
        input_declaration,
        edits,
        feature_profiles,
    })
}

fn load_baseline(path: Option<&Path>) -> Result<(RuntimeParams, bool), String> {
    match path {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("opening params file {}: {e}", path.display()))?;
            let value = crate::params::sources::parse_carrier(&text, false)
                .map_err(|e| format!("parsing params file {}: {e}", path.display()))?;
            if value
                .pointer("/io/output_gamut_compress/lightness_compression")
                .is_some()
            {
                return Err(format!(
                    "params file {}: {}",
                    path.display(),
                    crate::params::sources::retired_lightness_error()
                ));
            }
            let granularity_authored = value
                .pointer("/film_render/grain/rms_granularity")
                .is_some();
            let params: RuntimeParams = serde_json::from_value(value)
                .map_err(|e| format!("parsing params file {}: {e}", path.display()))?;
            params
                .output_mix
                .validate()
                .map_err(|e| format!("params file {}: {e}", path.display()))?;
            crate::params::validation::validate_enum_value(
                "scanner.scan_output",
                &params.scanner.scan_output,
            )
            .map_err(|e| format!("params file {}: {e}", path.display()))?;
            Ok((params, granularity_authored))
        }
        None => Ok((RuntimeParams::default(), false)),
    }
}

fn choose_workflow(params: &mut RuntimeParams, route: Option<&str>, scan_film: bool) {
    if let Some(route) = route {
        params.workflow.route = route.to_owned();
    } else if scan_film {
        params.workflow.route = "input > film > scan".into();
    }
}

fn force_raw_input(params: &mut RuntimeParams) {
    params.io.input_color_space = "ACES2065-1".into();
    params.io.input_cctf_decoding = false;
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    std::path::absolute(path).map_err(|e| format!("resolving {}: {e}", path.display()))
}

fn baseline_declarations(
    path: Option<&Path>,
) -> Result<BTreeMap<String, DeclarationSource>, String> {
    let mut declarations = BTreeMap::new();
    if let Some(path) = path {
        let value: Value = serde_json::from_reader(
            std::fs::File::open(path)
                .map_err(|e| format!("opening params file {}: {e}", path.display()))?,
        )
        .map_err(|e| format!("parsing params file {}: {e}", path.display()))?;
        for leaf in ["io.input_color_space", "io.input_cctf_decoding"] {
            if lookup(&value, leaf).is_some() {
                declarations.insert(
                    leaf.into(),
                    DeclarationSource {
                        kind: "params".into(),
                        option: Some("--params".into()),
                        index: None,
                        path: Some(absolute_path(path)?),
                    },
                );
            }
        }
    }
    Ok(declarations)
}

/// Read a retained image-input material route without exposing the carrier
/// parser or admitting implicit continuation routes for artifact input.
pub fn legacy_image_stage_route(path: &Path) -> Result<Option<WorkflowRoute>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("opening params file {}: {e}", path.display()))?;
    let baseline = crate::params::sources::parse_carrier(&text, false)
        .map_err(|e| format!("parsing params file {}: {e}", path.display()))?;
    let route = baseline
        .pointer("/workflow/route")
        .and_then(Value::as_str)
        .map(WorkflowRoute::parse);
    Ok(route.filter(|route| route.artifact_output().is_some() && route.artifact_input().is_none()))
}

#[cfg(test)]
mod tests;

fn load_referenced_profile(
    reference: &presets::ProfileRef,
    data_dir: &Path,
) -> Result<Profile, String> {
    let profile =
        profile::load_profile_by_name(data_dir, &reference.stock).map_err(|e| e.to_string())?;
    if profile.info.stock.as_deref() != Some(reference.stock.as_str()) {
        return Err(format!(
            "profile {:?} does not declare its stock identifier",
            reference.stock
        ));
    }
    if reference
        .version
        .as_ref()
        .is_some_and(|version| version != &profile.metadata.version)
    {
        return Err(format!("profile {:?} version mismatch", reference.stock));
    }
    if let Some(expected) = &reference.content_identity {
        use sha2::{Digest, Sha256};
        let path = data_dir
            .join("profiles")
            .join(format!("{}.json", reference.stock));
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let actual = format!("sha256-{:x}", Sha256::digest(bytes));
        if &actual != expected {
            return Err(format!(
                "profile {:?} content identity mismatch",
                reference.stock
            ));
        }
    }
    Ok(profile)
}
