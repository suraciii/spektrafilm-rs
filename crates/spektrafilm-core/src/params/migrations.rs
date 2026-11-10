//! Shared parameter transformations for versioned legacy documents.
//! Document readers own version gates, payload scope and publication.

use super::validation::retired_lightness_error_detailed;
use super::{ValidationError as Error, ValidationReason as Reason};
use serde_json::Value;

/// Normalize the runtime-parameter subtree of a versioned legacy complete document.
/// Rejection leaves the supplied document unchanged. Current complete documents
/// and sparse sources must use their strict boundary without normalization.
///
/// The migration is transactional: every conflict is detected before any
/// mutation, so a rejected document stays byte-identical. It is idempotent:
/// a migrated document carries only canonical `cmyk` fields and the canonical
/// base route, and re-running the migration is a no-op.
pub fn normalize_legacy_complete_params(value: &mut Value) -> Result<(), String> {
    normalize_legacy_complete_params_detailed(value).map_err(|error| error.to_string())
}

pub fn normalize_legacy_complete_params_detailed(value: &mut Value) -> Result<(), Error> {
    // Reject the retired leaf before renaming anything so a rejected
    // document stays byte-identical.
    let has_retired = match value.pointer("/io/output_gamut_compress/lightness_compression") {
        Some(retired) if !retired.is_null() => return Err(retired_lightness_error_detailed()),
        Some(_) => true,
        None => false,
    };
    let exposure = legacy_print_exposure(value, true)?;
    let legacy_route = legacy_cmyk_route(value)?;
    if let Some(exposure) = exposure {
        exposure.apply(value);
    }
    apply_legacy_cmyk(value, legacy_route);
    if has_retired {
        value
            .pointer_mut("/io/output_gamut_compress")
            .and_then(Value::as_object_mut)
            .expect("retired field belongs to an object")
            .remove("lightness_compression");
    }
    Ok(())
}

const LEGACY_GROUP: &str = "magazine_print_color";
const NEW_GROUP: &str = "cmyk";

/// Validate the retired CMYK group and route before any document is changed.
/// The returned flag records whether the route implies `cmyk.active = true`.
fn legacy_cmyk_route(value: &Value) -> Result<bool, Error> {
    const RETIRED_ROUTE: &str = crate::WorkflowRoute::REMOVED_MAGAZINE_ROUTE;

    let Some(object) = value.as_object() else {
        return Ok(false);
    };

    // Validate every conflict before mutating so a rejected document stays
    // byte-identical.
    if let Some(legacy) = object.get(LEGACY_GROUP) {
        if !legacy.is_object() {
            return Err(Error::new(
                Reason::LegacyGroupObject,
                LEGACY_GROUP,
                format!("legacy complete params carry a non-object {LEGACY_GROUP} group"),
            ));
        }
        if object.contains_key(NEW_GROUP) {
            return Err(Error::new(Reason::LegacyConflict, LEGACY_GROUP, format!(
                "legacy complete params carry both {LEGACY_GROUP} and {NEW_GROUP}; keep exactly one spelling"
            )).arg("other", NEW_GROUP));
        }
    }

    let has_legacy_route = object
        .get("workflow")
        .and_then(Value::as_object)
        .and_then(|w| w.get("route"))
        .and_then(Value::as_str)
        == Some(RETIRED_ROUTE);

    // A legacy route and a legacy group together are a conflict: the route
    // implies `cmyk.active = true`, which the group may contradict.
    if has_legacy_route
        && let Some(legacy) = object.get(LEGACY_GROUP).and_then(Value::as_object)
        && legacy.contains_key("active")
    {
        return Err(Error::new(Reason::LegacyRouteConflict, format!("{LEGACY_GROUP}.active"), format!(
            "legacy complete params carry both the retired route {RETIRED_ROUTE} and {LEGACY_GROUP}.active; keep exactly one spelling"
        )).arg("route", RETIRED_ROUTE));
    }
    Ok(has_legacy_route)
}

fn apply_legacy_cmyk(value: &mut Value, has_legacy_route: bool) {
    const BASE_ROUTE: &str = "input > film > scan";

    let Some(object) = value.as_object_mut() else {
        return;
    };

    // Migrate the legacy group onto the canonical name.
    if let Some(legacy) = object.remove(LEGACY_GROUP) {
        object.insert(NEW_GROUP.to_owned(), legacy);
    }

    // Migrate the retired route onto the base route with cmyk active.
    if has_legacy_route {
        let workflow = object
            .entry("workflow")
            .or_insert_with(|| Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .expect("workflow group is an object");
        workflow.insert("route".to_owned(), BASE_ROUTE.into());
        let cmyk = object
            .entry(NEW_GROUP)
            .or_insert_with(|| Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .expect("cmyk group is an object");
        cmyk.insert("active".to_owned(), Value::Bool(true));
    }
}

/// Remove the old readout control only at a versioned legacy boundary.
/// Complete captures retain its value in the independent convert acquisition
/// light unless that document already declares one. Scanner-only presets
/// discard the retired leaf instead of gaining a different section.
pub fn normalize_legacy_scan_readout(
    value: &mut Value,
    preserve_capture: bool,
) -> Result<(), String> {
    normalize_legacy_scan_readout_detailed(value, preserve_capture)
        .map_err(|error| error.to_string())
}

pub fn normalize_legacy_scan_readout_detailed(
    value: &mut Value,
    preserve_capture: bool,
) -> Result<(), Error> {
    let Some(lamp) = value.pointer("/scanner/lamp").cloned() else {
        return Ok(());
    };
    let name = lamp
        .as_str()
        .filter(|name| crate::spectral_service::is_supported_illuminant(name))
        .ok_or_else(|| {
            Error::new(
                Reason::LegacyLamp,
                "scanner.lamp",
                "legacy scanner.lamp must name a supported illuminant",
            )
        })?;
    let mut candidate = value.clone();
    candidate["scanner"]
        .as_object_mut()
        .expect("lamp belongs to scanner object")
        .remove("lamp");
    if preserve_capture
        && candidate
            .pointer("/film_render/convert/scan_illuminant")
            .is_none()
    {
        let film = candidate
            .as_object_mut()
            .ok_or_else(|| {
                Error::new(
                    Reason::ObjectRequired,
                    "",
                    "legacy parameters must be an object",
                )
            })?
            .entry("film_render")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(|| {
                Error::new(
                    Reason::ObjectRequired,
                    "film_render",
                    "film_render must be an object",
                )
            })?;
        let convert = film
            .entry("convert")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(|| {
                Error::new(
                    Reason::ObjectRequired,
                    "film_render.convert",
                    "film_render.convert must be an object",
                )
            })?;
        convert.insert("scan_illuminant".into(), name.into());
    }
    *value = candidate;
    Ok(())
}

/// Map the pre-v4 legacy `enlarger` print-exposure pair of a complete
/// runtime document onto `enlarger.print_exposure_mode` with
/// `settings.print_exposure_ev_tracking` for the two mixed states.
/// `settings_allowed` must be false for single-section payloads, which cannot
/// carry a `settings` group. Only versioned legacy documents may run this
/// mapping; current documents must fail unknown-field validation instead.
/// The mapping is idempotent and preserves rendered pixels.
pub fn normalize_legacy_conversion_paths(
    value: &mut Value,
    settings_allowed: bool,
) -> Result<(), String> {
    normalize_legacy_conversion_paths_detailed(value, settings_allowed)
        .map_err(|error| error.to_string())
}

pub fn normalize_legacy_conversion_paths_detailed(
    value: &mut Value,
    settings_allowed: bool,
) -> Result<(), Error> {
    if let Some(exposure) = legacy_print_exposure(value, settings_allowed)? {
        exposure.apply(value);
    }
    Ok(())
}

struct LegacyPrintExposure {
    mode: &'static str,
    tracking: Option<&'static str>,
}

fn legacy_print_exposure(
    value: &Value,
    settings_allowed: bool,
) -> Result<Option<LegacyPrintExposure>, Error> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let mut mixed_state = None;
    if let Some(enlarger) = object.get("enlarger").and_then(Value::as_object) {
        let has_legacy = enlarger.contains_key("normalize_print_exposure")
            || enlarger.contains_key("print_exposure_compensation");
        if has_legacy && enlarger.contains_key("print_exposure_mode") {
            return Err(Error::new(
                Reason::LegacyExposureConflict,
                "enlarger.print_exposure_mode",
                "legacy complete params carry both the print exposure pair and enlarger.print_exposure_mode; keep exactly one spelling",
            ));
        }
        if has_legacy {
            let leaf = |value: Option<&Value>| -> Result<bool, Error> {
                match value {
                    None => Ok(true),
                    Some(Value::Bool(b)) => Ok(*b),
                    Some(_) => Err(Error::new(
                        Reason::LegacyExposureBoolean,
                        "enlarger",
                        "legacy enlarger print exposure leaves must be booleans",
                    )),
                }
            };
            let normalize = leaf(enlarger.get("normalize_print_exposure"))?;
            let compensation = leaf(enlarger.get("print_exposure_compensation"))?;
            mixed_state = Some(match (normalize, compensation) {
                (true, true) => ("auto", None),
                (false, false) => ("manual", None),
                (true, false) => ("auto", Some("never")),
                (false, true) => ("manual", Some("always")),
            });
        }
    }
    if let Some((_, tracking)) = mixed_state
        && tracking.is_some()
        && !settings_allowed
    {
        return Err(Error::new(
            Reason::LegacyExposureMixed,
            "enlarger.print_exposure_mode",
            "the legacy enlarger print exposure pair mixes normalization and compensation; this document cannot carry settings.print_exposure_ev_tracking, so set enlarger.print_exposure_mode explicitly",
        ));
    }
    // A mixed pair writes into `settings`; an existing non-object there
    // cannot carry the leaf, and this must fail before any mutation.
    if let Some((_, tracking)) = mixed_state
        && tracking.is_some()
        && matches!(object.get("settings"), Some(value) if !value.is_object())
    {
        return Err(Error::new(
            Reason::LegacySettingsObject,
            "settings",
            "the legacy enlarger print exposure pair needs the settings.print_exposure_ev_tracking leaf, but settings is not an object",
        ));
    }
    Ok(mixed_state.map(|(mode, tracking)| LegacyPrintExposure { mode, tracking }))
}

impl LegacyPrintExposure {
    fn apply(self, value: &mut Value) {
        let object = value
            .as_object_mut()
            .expect("legacy parameters are an object");
        let enlarger = object
            .get_mut("enlarger")
            .and_then(Value::as_object_mut)
            .expect("legacy pair belongs to the enlarger object");
        enlarger.remove("normalize_print_exposure");
        enlarger.remove("print_exposure_compensation");
        enlarger.insert("print_exposure_mode".into(), self.mode.into());
        if let Some(tracking) = self.tracking {
            object
                .entry("settings")
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                .as_object_mut()
                .expect("settings group is an object")
                .insert("print_exposure_ev_tracking".into(), tracking.into());
        }
    }
}

#[cfg(test)]
mod tests;
