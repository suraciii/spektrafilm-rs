//! Composition on a restored material baseline, without replaying input preparation.
use super::*;
use crate::stage_artifact::{EmbeddedProfile, MaterialKind, StageHeader};

pub struct ResolvedContinuation {
    pub params: RuntimeParams,
    pub print: Option<Profile>,
    pub edits: Vec<EditFeedback>,
}

/// Validate each source before composition so a later override cannot hide a
/// forbidden edit to already completed material or interpretation context.
pub fn resolve_continuation(
    request: ProcessParamsRequest<'_>,
    header: &StageHeader,
    data_dir: &Path,
) -> Result<ResolvedContinuation, String> {
    if request.scan_film {
        return Err(
            "artifact input requires an explicit --route; --scan-film is for image input".into(),
        );
    }
    if request.input_color_space.is_some() || request.input_cctf_decoding.is_some() {
        return Err("raster input declarations are inapplicable to a stage artifact".into());
    }
    let route = request
        .route
        .ok_or("stage artifact input requires an explicit --route")?;
    let route = WorkflowRoute::parse(route);
    if route.artifact_input() != Some(header.state) {
        return Err("--route input node must match the artifact state".into());
    }
    let mut params = header.baseline.clone();
    params.workflow.route = route.as_str().into();
    params.io.scan_film = route.scans_film();
    let mut winning = BTreeMap::new();
    let baseline = serde_json::to_value(&params).map_err(|e| e.to_string())?;
    let mut apply = |edits: ParameterEdits,
                     origin: DeclarationSource,
                     params: &mut RuntimeParams|
     -> Result<(), String> {
        for (path, value) in edits.values() {
            if matches!(
                path,
                "io.input_color_space" | "io.input_cctf_decoding" | "taps.inject" | "taps.collect"
            ) {
                return Err(format!(
                    "{path} from {} is inapplicable to stage artifact input",
                    origin.option.as_deref().unwrap_or(&origin.kind)
                ));
            }
            if crate::stage_execution::is_frozen_parameter(header.state, path, &header.baseline)
                && !equal_control_values(
                    path,
                    value,
                    lookup(&baseline, path).unwrap_or(&Value::Null),
                )
            {
                return Err(format!(
                    "{path} from {}{} changes frozen state; regenerate from the earlier input",
                    origin.option.as_deref().unwrap_or(&origin.kind),
                    origin.index.map(|i| format!(" #{i}")).unwrap_or_default()
                ));
            }
            winning.insert(path.to_owned(), (value.clone(), origin.clone()));
        }
        let mut candidate = params.clone();
        edits.apply(&mut candidate)?;
        crate::stage_execution::validate_frozen_controls(header, &candidate).map_err(|e| {
            format!(
                "{}{}: {e}",
                origin.option.as_deref().unwrap_or(&origin.kind),
                origin.index.map(|i| format!(" #{i}")).unwrap_or_default()
            )
        })?;
        *params = candidate;
        Ok(())
    };
    if let Some(path) = request.params_file {
        // Keep the legacy decoder and validate all carried leaves, not just the
        // differences remaining after subsequent sources have been composed.
        let _ = load_baseline(Some(path))?;
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let value = crate::params::sources::parse_carrier(&text, false)?;
        apply(
            ParameterEdits::from_value(value)?,
            DeclarationSource {
                kind: "params".into(),
                option: Some("--params".into()),
                index: None,
                path: Some(absolute_path(path)?),
            },
            &mut params,
        )?;
    }
    let retained_print = header.original_print.as_ref().or(header.print.as_ref());
    let mut print = None;
    for (index, preset) in request.presets.iter().enumerate() {
        preset.validate()?;
        if let Some(reference) = &preset.film_profile {
            let _ = load_referenced_profile(reference, data_dir)?;
            validate_frozen_profile(&reference.stock, &header.film, data_dir, "film")?;
        }
        if let Some(reference) = &preset.print_profile {
            let selected = load_referenced_profile(reference, data_dir)?;
            if header.state == MaterialKind::Print {
                validate_frozen_profile(
                    &reference.stock,
                    header
                        .print
                        .as_ref()
                        .ok_or("print state missing material")?,
                    data_dir,
                    "print",
                )?;
            } else {
                print = Some(selected);
            }
        }
        let mut candidate = params.clone();
        match &preset.parameters {
            presets::PresetParameters::Look(look) => look.apply_to(&mut candidate),
            presets::PresetParameters::Section(_) => candidate = preset.overlay_section(&params)?,
        }
        // Presets own only their documented leaves. Use the shared overlay to
        // map preset-specific names and preserve derived runtime-only fields.
        let candidate = serde_json::to_value(candidate).map_err(|e| e.to_string())?;
        let current = serde_json::to_value(&params).map_err(|e| e.to_string())?;
        let origin = DeclarationSource {
            kind: "preset".into(),
            option: Some("--preset".into()),
            index: Some(index + 1),
            path: None,
        };
        let mut leaves = Vec::new();
        changed_leaves("", &current, &candidate, &mut leaves);
        for (path, value) in leaves {
            apply(
                ParameterEdits::single(&path, value)?,
                origin.clone(),
                &mut params,
            )?;
        }
    }
    for selection in request.feature_profiles {
        let (_, requested) =
            crate::feature_profiles::apply_with_requests(&params, std::slice::from_ref(selection))?;
        for path in crate::feature_profiles::scope(selection.family) {
            if let Some(value) = requested[0].get(*path) {
                apply(
                    ParameterEdits::single(path, value.clone())?,
                    DeclarationSource {
                        kind: "feature_profile".into(),
                        option: Some("--feature-profile".into()),
                        index: Some(selection.position),
                        path: None,
                    },
                    &mut params,
                )?;
            }
        }
    }
    for (index, source) in request.sources.iter().enumerate() {
        let edits = ParameterEdits::parse(source).map_err(|e| format!("--set {source:?}: {e}"))?;
        apply(
            edits,
            DeclarationSource {
                kind: "set".into(),
                option: Some("--set".into()),
                index: Some(index + 1),
                path: ParameterEdits::file_path(source)
                    .map(absolute_path)
                    .transpose()?,
            },
            &mut params,
        )?;
    }
    if let Some(name) = request.film {
        validate_frozen_profile(name, &header.film, data_dir, "film")?;
    }
    if let Some(name) = request.paper {
        if header.state == MaterialKind::Print {
            validate_frozen_profile(
                name,
                header
                    .print
                    .as_ref()
                    .ok_or("print state missing material")?,
                data_dir,
                "print",
            )?;
        } else {
            print = Some(profile::load_profile_by_name(data_dir, name).map_err(|e| e.to_string())?);
        }
    }
    if route.prints() && print.is_none() && retained_print.is_none() {
        let name = header
            .film
            .profile
            .info
            .target_print
            .as_deref()
            .ok_or("new print operation requires --paper or a retained target_print")?;
        print = Some(
            profile::load_profile_by_name(data_dir, name)
                .map_err(|e| format!("loading downstream print profile {name:?}: {e}"))?,
        );
    }
    if route.prints() && print.as_ref().is_some_and(|p| !p.is_print_stock()) {
        return Err("downstream print profile must have printing support/stage".into());
    }
    if let (Some(selected), Some(retained)) = (&print, retained_print) {
        if retained
            .matches_source(selected)
            .map_err(|e| e.to_string())?
        {
            print = None;
        }
    }
    params.workflow.route = route.as_str().into();
    params.io.scan_film = route.scans_film();
    if let Some(value) = request.scan_output {
        ParameterEdits::single("scanner.scan_output", Value::String(value.into()))?
            .apply(&mut params)?;
    }
    validate_finite(&params)?;
    params.validate()?;
    crate::stage_execution::validate_continuation(header, &params)?;
    let effective = serde_json::to_value(&params).map_err(|e| e.to_string())?;
    let edits = winning
        .into_iter()
        .map(|(path, (requested, source))| EditFeedback {
            effective: lookup(&effective, &path).cloned().unwrap_or(Value::Null),
            path,
            requested,
            source,
            status: EditStatus::Active,
            reason: None,
        })
        .collect();
    Ok(ResolvedContinuation {
        params,
        print,
        edits,
    })
}

fn validate_frozen_profile(
    name: &str,
    frozen: &EmbeddedProfile,
    data_dir: &Path,
    kind: &str,
) -> Result<(), String> {
    let selected = profile::load_profile_by_name(data_dir, name).map_err(|e| e.to_string())?;
    if !frozen
        .matches_source(&selected)
        .map_err(|e| e.to_string())?
    {
        return Err(format!(
            "{kind} profile {name:?} changes frozen material content; regenerate from the earlier input"
        ));
    }
    Ok(())
}

fn changed_leaves(
    prefix: &str,
    baseline: &Value,
    candidate: &Value,
    out: &mut Vec<(String, Value)>,
) {
    if let Some(map) = candidate.as_object() {
        for (key, value) in map {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            changed_leaves(&path, &baseline[key], value, out);
        }
    } else if baseline != candidate {
        out.push((prefix.to_owned(), candidate.clone()));
    }
}
