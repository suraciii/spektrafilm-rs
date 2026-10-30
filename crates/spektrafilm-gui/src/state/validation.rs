//! State normalization and typed runtime validation.
use anyhow::{Result, bail};
use serde_json::{Value, json};
use spektrafilm_core::params::RuntimeParams;

use super::{overlay_runtime, translate_legacy_print_exposure};

/// Stored operator values may be inactive; rendering validates the effective controls.
pub(crate) fn validate_stored_runtime(params: &mut RuntimeParams) -> Result<()> {
    params
        .scanner
        .operator
        .validate_detailed()
        .map_err(anyhow::Error::new)?;
    let operator = std::mem::take(&mut params.scanner.operator);
    let result = params.validate_detailed();
    params.scanner.operator = operator;
    result.map_err(anyhow::Error::new)
}

/// Validate a normalized runtime carrier before another stored mirror can replace it.
fn validate_runtime_values(value: Value, path: &str) -> Result<()> {
    let params: RuntimeParams = serde_json::from_value(value).map_err(|error| {
        crate::app::Message::new("operations.state.invalid_parameters")
            .arg("path", path)
            .context(error)
    })?;
    params
        .validate_values_detailed()
        .map_err(anyhow::Error::new)
}

fn normalize_legacy_runtime_cmyk(runtime: &mut Value) -> Result<()> {
    let Some(object) = runtime.as_object_mut() else {
        return Ok(());
    };
    // Limit this migration to CMYK: other legacy conversions have different
    // version boundaries. The core migration owns the route and conflict rules.
    let mut carrier = json!({});
    for key in ["workflow", "cmyk", "magazine_print_color"] {
        if let Some(fields) = object.get(key) {
            carrier[key] = fields.clone();
        }
    }
    if carrier.pointer("/workflow/route").and_then(Value::as_str)
        == Some("input > film > scan > magazine")
        && carrier.get("cmyk").is_some_and(|cmyk| !cmyk.is_object())
    {
        validate_runtime_values(json!({"cmyk": carrier["cmyk"]}), "rust.runtime.cmyk")?;
    }
    spektrafilm_core::params::sources::normalize_legacy_complete_params_detailed(&mut carrier)
        .map_err(anyhow::Error::new)?;
    for key in ["workflow", "cmyk", "magazine_print_color"] {
        if let Some(fields) = carrier
            .as_object_mut()
            .and_then(|fields| fields.remove(key))
        {
            object.insert(key.to_owned(), fields);
        } else {
            object.remove(key);
        }
    }
    Ok(())
}

pub(super) fn normalize_grain_section(section: &mut Value) {
    let Some(object) = section.as_object_mut() else {
        return;
    };
    object.remove("monochrome");
}

pub(super) fn validate_state_parameters(value: &mut Value, version: Option<u64>) -> Result<()> {
    // Normalize both stored representations before defaults or overlays can
    // introduce the canonical spelling beside a historical control.
    for pointer in ["/grain", "/rust/runtime/film_render/grain"] {
        if let Some(grain) = value.pointer_mut(pointer) {
            spektrafilm_core::params::normalize_grain_control_names(grain)
                .map_err(anyhow::Error::msg)?;
        }
    }
    if let Some(preparation) = value.pointer_mut("/rust/render_preparation") {
        let canonical: spektrafilm_core::params::RenderPreparationState =
            serde_json::from_value(preparation.clone())?;
        *preparation = serde_json::to_value(canonical)?;
    }
    // Version 1 states may still carry the retired lightness leaf; versions
    // 1 and 2 carry the pre-v4 conversion-control paths. Current documents
    // must fail unknown-field validation instead of being migrated.
    let legacy = version == Some(1);
    let legacy_conversion = matches!(version, Some(1 | 2));
    // Versions 1..=3 are legacy complete documents for the CMYK cutover: they
    // may carry the retired `magazine_print_color` group and the retired
    // `input > film > scan > magazine` route, which migrate onto the canonical
    // `cmyk` group and base route. Version 4 documents are current-schema and
    // reject both spellings.
    let legacy_cmyk = matches!(version, Some(1..=3));
    const LEGACY_GROUP: &str = "magazine_print_color";
    const RETIRED_ROUTE: &str = "input > film > scan > magazine";
    if value.get("output_mix").is_some() {
        bail!(crate::app::Message::new(
            "operations.state.output_mix_owner"
        ));
    }
    if !legacy_cmyk {
        if value.get(LEGACY_GROUP).is_some()
            || value
                .pointer("/rust/runtime/magazine_print_color")
                .is_some()
        {
            bail!(
                crate::app::Message::new("operations.state.retired_group")
                    .arg("group", LEGACY_GROUP)
            );
        }
        if value.pointer("/simulation/route").and_then(Value::as_str) == Some(RETIRED_ROUTE)
            || value
                .pointer("/rust/runtime/workflow/route")
                .and_then(Value::as_str)
                == Some(RETIRED_ROUTE)
        {
            bail!(
                crate::app::Message::new("operations.state.retired_route")
                    .arg("route", RETIRED_ROUTE)
            );
        }
    }
    let legacy_route =
        value.pointer("/simulation/route").and_then(Value::as_str) == Some(RETIRED_ROUTE);
    let legacy_group_has_active = value
        .get(LEGACY_GROUP)
        .and_then(Value::as_object)
        .is_some_and(|group| group.contains_key("active"));
    if legacy_cmyk && legacy_route && legacy_group_has_active {
        bail!(crate::app::Message::new(
            "operations.state.legacy_route_conflict"
        ));
    }
    // Migrate the retired CMYK group and route of a legacy complete document
    // before any typed validation reads it. The mapping is transactional and
    // idempotent, and a rejected document stays unchanged.
    if legacy_cmyk {
        if let Some(magazine) = value.get("magazine_print_color") {
            if !magazine.is_object() {
                bail!(crate::app::Message::new(
                    "operations.state.legacy_group_object"
                ));
            }
            if value.get("cmyk").is_some() {
                bail!(crate::app::Message::new(
                    "operations.state.legacy_groups_conflict"
                ));
            }
            let migrated = magazine.clone();
            value
                .as_object_mut()
                .expect("state object")
                .insert("cmyk".to_owned(), migrated);
            value
                .as_object_mut()
                .expect("state object")
                .remove("magazine_print_color");
        }
        if value.pointer("/simulation/route").and_then(Value::as_str) == Some(RETIRED_ROUTE) {
            value["simulation"]["route"] = json!("input > film > scan");
            if !value["cmyk"].is_object() {
                value["cmyk"] = json!({});
            }
            value["cmyk"]["active"] = json!(true);
        }
    }
    // Validate the mirror before canonicalization drops extension-only leaves.
    // In particular, an explicit null or unknown engine is never omission.
    if let Some(grain) = value.get("grain") {
        let mut grain = grain.clone();
        normalize_grain_section(&mut grain);
        let _: spektrafilm_core::params::GrainParams =
            serde_json::from_value(grain).map_err(|error| {
                crate::app::Message::new("operations.state.invalid_parameters")
                    .arg("path", "grain")
                    .context(error)
            })?;
    }
    // Validate the supplied camera mirror before the dedicated diffusion
    // panel replaces it. Apply the same known-leaf projection as the loader.
    if let Some(camera) = value.get("camera") {
        let mut normalized = super::GuiState::factory().sections["camera"].clone();
        super::merge(&mut normalized, camera);
        validate_runtime_values(json!({"camera": normalized}), "camera")?;
    }
    if matches!(version, Some(2 | 3))
        && let Some(runtime) = value.pointer_mut("/rust/runtime")
    {
        normalize_legacy_runtime_cmyk(runtime)?;
    }
    let mut explicit = value["rust"]["runtime"].clone();
    if explicit.is_null() {
        explicit = json!({});
    }
    if legacy_conversion {
        spektrafilm_core::params::sources::normalize_legacy_conversion_paths_detailed(value, true)
            .map_err(anyhow::Error::new)?;
        spektrafilm_core::params::sources::normalize_legacy_conversion_paths_detailed(
            &mut explicit,
            true,
        )
        .map_err(anyhow::Error::new)?;
        if let Some(runtime) = value["rust"].get_mut("runtime") {
            spektrafilm_core::params::sources::normalize_legacy_conversion_paths_detailed(
                runtime, true,
            )
            .map_err(anyhow::Error::new)?;
        }
    }
    if legacy {
        spektrafilm_core::params::sources::normalize_legacy_complete_params_detailed(value)
            .map_err(anyhow::Error::new)?;
    } else if value["io"]["output_gamut_compress"]
        .get("lightness_compression")
        .is_some()
    {
        bail!(
            crate::app::Message::new("operations.state.retired_lightness")
                .arg("path", "io.output_gamut_compress.lightness_compression")
        );
    }
    if !legacy
        && (explicit["io"]["output_gamut_compress"]
            .get("lightness_compression")
            .is_some()
            || value["output_gamut_compress"]
                .get("lightness_compression")
                .is_some())
    {
        bail!(
            crate::app::Message::new("operations.state.retired_lightness")
                .arg("path", "io.output_gamut_compress.lightness_compression")
        );
    }
    if legacy {
        spektrafilm_core::params::sources::normalize_legacy_complete_params_detailed(&mut explicit)
            .map_err(anyhow::Error::new)?;
    }
    validate_runtime_values(explicit.clone(), "rust.runtime")?;
    if let Some(fields) = value.get("grain") {
        let mut grain = fields.clone();
        normalize_grain_section(&mut grain);
        let projected = json!({"film_render": {"grain": grain}});
        validate_runtime_values(projected, "grain")?;
    }
    for (section, group, leaf) in [
        ("output_gamut_compress", "io", "output_gamut_compress"),
        ("preflashing", "enlarger", ""),
    ] {
        if let Some(fields) = value.get(section) {
            if !explicit[group].is_object() {
                explicit[group] = json!({});
            }
            if leaf.is_empty() {
                if section == "preflashing" {
                    let (translated, tracking) = translate_legacy_print_exposure(fields);
                    if let Some(tracking) = tracking {
                        if !explicit["settings"].is_object() {
                            explicit["settings"] = json!({});
                        }
                        overlay_runtime(&mut explicit["settings"], &tracking);
                    }
                    overlay_runtime(&mut explicit[group], &translated);
                } else {
                    overlay_runtime(&mut explicit[group], fields);
                }
            } else {
                explicit[group][leaf] = fields.clone();
            }
        }
    }
    for group in [
        "film_render",
        "print_render",
        "enlarger",
        "reproduction",
        "io",
    ] {
        if let Some(fields) = value.get(group) {
            explicit[group] = fields.clone();
        }
    }
    if let Some(enlarger) = value["simulation"].get("enlarger") {
        if !explicit["enlarger"].is_object() {
            explicit["enlarger"] = json!({});
        }
        // The upstream boolean mirror is derived: it normalizes (auto) and
        // only toggles camera-EV tracking. It may only refine the default
        // (auto without tracking), never overwrite the mapping an
        // authoritative carrier (`rust.runtime`, the translated `preflashing`
        // pair) already produced — the same guard the projection applies.
        let mut enlarger = enlarger.clone();
        if let Some(Value::Bool(comp)) = enlarger
            .as_object_mut()
            .and_then(|fields| fields.remove("print_exposure_compensation"))
        {
            let tracking_default = explicit["settings"]["print_exposure_ev_tracking"]
                .as_str()
                .is_none_or(|v| v == "follow_mode");
            let auto = explicit["enlarger"]["print_exposure_mode"]
                .as_str()
                .is_none_or(|v| v == "auto");
            if tracking_default && auto && !comp {
                explicit["settings"]["print_exposure_ev_tracking"] = json!("never");
            }
        }
        overlay_runtime(&mut explicit["enlarger"], &enlarger);
    }
    if let Some(io) = value["simulation"].get("io") {
        let mut projected = json!({"io": io});
        if legacy {
            spektrafilm_core::params::sources::normalize_legacy_complete_params_detailed(
                &mut projected,
            )
            .map_err(anyhow::Error::new)?;
        } else if io["output_gamut_compress"]
            .get("lightness_compression")
            .is_some()
        {
            bail!(
                crate::app::Message::new("operations.state.retired_lightness").arg(
                    "path",
                    "simulation.io.output_gamut_compress.lightness_compression"
                )
            );
        }
        validate_runtime_values(projected.clone(), "simulation.io")?;
        if legacy {
            value["simulation"]["io"] = projected["io"].clone();
        }
    }
    // Inspect both carriers independently so an ignored duplicate cannot hide a retired value.
    // The scanner section projects through the typed runtime so retired
    // scanner keys fail the document instead of being silently dropped.
    if let Some(fields) = value.get("scanner") {
        let projected = json!({"scanner": fields});
        validate_runtime_values(projected, "scanner")?;
    }
    if let Some(fields) = value["rust"]["runtime"].get("scanner") {
        let projected = json!({"scanner": fields});
        validate_runtime_values(projected, "rust.runtime.scanner")?;
    }
    if let Some(output) = value.get_mut("output_gamut_compress") {
        let mut projected = json!({"io": {"output_gamut_compress": output.clone()}});
        if legacy {
            spektrafilm_core::params::sources::normalize_legacy_complete_params_detailed(
                &mut projected,
            )
            .map_err(anyhow::Error::new)?;
        }
        validate_runtime_values(projected, "io.output_gamut_compress")?;
        if legacy {
            output
                .as_object_mut()
                .map(|fields| fields.remove("lightness_compression"));
        }
    }
    if legacy {
        spektrafilm_core::params::sources::normalize_legacy_complete_params_detailed(&mut explicit)
            .map_err(anyhow::Error::new)?;
        if let Some(runtime) = value["rust"].get_mut("runtime") {
            spektrafilm_core::params::sources::normalize_legacy_complete_params_detailed(runtime)
                .map_err(anyhow::Error::new)?;
        }
    }
    validate_runtime_values(explicit, "rust.runtime")?;
    Ok(())
}
