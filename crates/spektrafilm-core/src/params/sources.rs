mod discovery;
mod effects;
mod metadata;
pub use effects::{
    grain_control_inactive_reason, grain_control_inactive_reason_with_profile, inactive_reason,
};

use discovery::schema_value;
pub use discovery::{
    describe_field, describe_groups, describe_module, describe_root_groups, suggest_groups,
    suggest_parameter_paths, supported_routes,
};

use super::RuntimeParams;
use super::{ValidationError as Error, ValidationReason as Reason};
use crate::suggest::error_suffix;
use metadata::{nullable, rust_type};
use serde_json::{Map, Value};

/// Runtime leaves intentionally outside the user-settable parameter surface.
pub fn non_settable_leaves() -> &'static [(&'static str, &'static str)] {
    &[(
        "random_seed",
        "per-render stochastic seed is set by the recipe runner",
    )]
}

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
    normalize_legacy_conversion_paths_detailed(value, true)?;
    normalize_legacy_cmyk_detailed(value)?;
    if has_retired {
        value
            .pointer_mut("/io/output_gamut_compress")
            .and_then(Value::as_object_mut)
            .expect("retired field belongs to an object")
            .remove("lightness_compression");
    }
    Ok(())
}

/// Migrate the retired `magazine_print_color` group and the retired
/// `input > film > scan > magazine` route of a versioned legacy complete
/// document onto the canonical `cmyk` group and base route.
///
/// The mapping is transactional (all conflicts detected before mutation),
/// idempotent, and preserves unrelated fields. A document supplying both a
/// legacy field and its new counterpart is rejected. A document naming the
/// retired route migrates to `input > film > scan` with `cmyk.active = true`.
fn normalize_legacy_cmyk_detailed(value: &mut Value) -> Result<(), Error> {
    const LEGACY_GROUP: &str = "magazine_print_color";
    const NEW_GROUP: &str = "cmyk";
    const RETIRED_ROUTE: &str = "input > film > scan > magazine";
    const BASE_ROUTE: &str = "input > film > scan";

    let Some(object) = value.as_object_mut() else {
        return Ok(());
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
        for leaf in ["active", "strength"] {
            if legacy.as_object().unwrap().contains_key(leaf)
                && object
                    .get(NEW_GROUP)
                    .and_then(Value::as_object)
                    .is_some_and(|new| new.contains_key(leaf))
            {
                return Err(Error::new(Reason::LegacyConflict, format!("{LEGACY_GROUP}.{leaf}"), format!(
                    "legacy complete params carry both {LEGACY_GROUP}.{leaf} and {NEW_GROUP}.{leaf}; keep exactly one spelling"
                )).arg("other", format!("{NEW_GROUP}.{leaf}")));
            }
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

    Ok(())
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
    let Some(object) = value.as_object_mut() else {
        return Ok(());
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
    if let Some((mode, tracking)) = mixed_state {
        let enlarger = object
            .get_mut("enlarger")
            .and_then(Value::as_object_mut)
            .expect("legacy pair belongs to the enlarger object");
        enlarger.remove("normalize_print_exposure");
        enlarger.remove("print_exposure_compensation");
        enlarger.insert("print_exposure_mode".into(), mode.into());
        if let Some(tracking) = tracking {
            object
                .entry("settings")
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                .as_object_mut()
                .expect("settings group is an object")
                .insert("print_exposure_ev_tracking".into(), tracking.into());
        }
    }
    Ok(())
}

pub(crate) fn retired_lightness_error() -> String {
    "io.output_gamut_compress.lightness_compression was retired; no automatic replacement exists; explicitly remove this field from the document".into()
}

fn retired_lightness_error_detailed() -> Error {
    Error::new(
        Reason::RetiredLightness,
        "io.output_gamut_compress.lightness_compression",
        retired_lightness_error(),
    )
}

use super::grain_compat::canonical_grain_path;

/// Ordered sparse edits to runtime parameter leaves.
#[derive(Debug, Clone, Default)]
pub struct ParameterEdits {
    edits: Vec<(String, Value)>,
}

impl ParameterEdits {
    pub fn parse(source: &str) -> Result<Self, String> {
        if source.trim().is_empty() {
            return Err("parameter source is empty".into());
        }
        let assignment = source
            .split_once('=')
            .is_some_and(|(path, _)| path.trim().split('.').all(valid_ident));
        if !assignment && (source.ends_with(".toml") || source.ends_with(".json")) {
            let text = std::fs::read_to_string(source).map_err(|e| format!("{source}: {e}"))?;
            return Self::from_value(parse_carrier(&text, source.ends_with(".toml"))?)
                .map_err(|e| format!("{source}: {e}"));
        }
        let mut out = Self::default();
        for item in split_top_level(source)? {
            let item = item.trim();
            if item.is_empty() {
                return Err("empty parameter assignment".into());
            }
            let eq = find_equals(item).ok_or_else(|| {
                format!("expected PATH=VALUE or a .toml/.json file, got {item:?}; example: --set film_render.grain.engine=procedural")
            })?;
            let path = item[..eq].trim();
            let canonical = canonical_grain_path(path);
            let path = canonical.as_ref();
            let raw = item[eq + 1..].trim();
            validate_path(path)?;
            if raw.is_empty() {
                return Err(format!("empty value for {path}"));
            }
            let value = parse_inline_value(path, raw)?;
            out.push(path, value)?;
        }
        Ok(out)
    }

    pub(crate) fn from_value(value: Value) -> Result<Self, String> {
        Self::from_value_detailed(value).map_err(|error| error.to_string())
    }

    pub(crate) fn from_value_detailed(mut value: Value) -> Result<Self, Error> {
        if let Some(grain) = value.pointer_mut("/film_render/grain") {
            super::normalize_grain_control_names(grain).map_err(|detail| {
                Error::new(Reason::LegacyConflict, "film_render.grain", detail)
            })?;
        }
        let root = value.as_object().ok_or_else(|| {
            Error::new(
                Reason::DocumentObject,
                "",
                "parameter document root must be an object",
            )
        })?;
        let mut out = Self::default();
        flatten_object("", root, &mut out)?;
        Ok(out)
    }

    fn push(&mut self, path: &str, value: Value) -> Result<(), String> {
        self.push_detailed(path, value)
            .map_err(|error| error.to_string())
    }

    fn push_detailed(&mut self, path: &str, value: Value) -> Result<(), Error> {
        let canonical = canonical_grain_path(path);
        let path = canonical.as_ref();
        let expected = lookup(schema_value(), path).ok_or_else(|| unknown_path_detailed(path))?;
        validate_leaf(path, &value, expected)?;
        self.edits.push((path.to_owned(), value));
        Ok(())
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.edits.iter().map(|(p, _)| p.as_str())
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.edits
            .iter()
            .map(|(path, value)| (path.as_str(), value))
    }

    /// Validate a dedicated option with the canonical leaf validator.
    pub(crate) fn single(path: &str, value: Value) -> Result<Self, String> {
        let mut edits = Self::default();
        edits.push(path, value)?;
        Ok(edits)
    }

    pub(crate) fn file_path(source: &str) -> Option<&std::path::Path> {
        let assignment = source
            .split_once('=')
            .is_some_and(|(path, _)| path.trim().split('.').all(valid_ident));
        (!assignment && (source.ends_with(".toml") || source.ends_with(".json")))
            .then(|| std::path::Path::new(source))
    }

    pub fn apply(&self, params: &mut RuntimeParams) -> Result<(), String> {
        self.apply_detailed(params)
            .map_err(|error| error.to_string())
    }

    pub fn apply_detailed(&self, params: &mut RuntimeParams) -> Result<(), Error> {
        // serde_json turns non-finite floats into null; reject the typed
        // baseline before that conversion can erase an invalid Option value.
        validate_finite_detailed(params)?;
        let seed = params.random_seed;
        let explicit_section_paths = params.explicit_section_paths.clone();
        let mut root = serde_json::to_value(&*params).map_err(|error| {
            Error::new(
                Reason::SerializeParameters,
                "",
                format!("serialize parameters: {error}"),
            )
            .arg("detail", error)
        })?;
        for (path, value) in &self.edits {
            set_value_detailed(&mut root, path, value.clone())?;
        }
        *params = serde_json::from_value(root).map_err(|error| {
            Error::new(
                Reason::ApplyParameters,
                "",
                format!("apply parameter edits: {error}"),
            )
            .arg("detail", error)
        })?;
        params.random_seed = seed;
        params.explicit_section_paths = explicit_section_paths;
        validate_finite_detailed(params)
    }

    /// Reject RAW-incompatible explicit values before composition can overwrite them.
    pub(crate) fn validate_raw_input(&self) -> Result<(), String> {
        for (path, value) in &self.edits {
            if path == "io.input_color_space" && value.as_str() != Some("ACES2065-1") {
                return Err("io.input_color_space: RAW input requires ACES2065-1".into());
            }
            if path == "io.input_cctf_decoding" && value.as_bool() != Some(false) {
                return Err("io.input_cctf_decoding: RAW input requires false".into());
            }
        }
        Ok(())
    }
}

fn flatten_object(
    prefix: &str,
    object: &Map<String, Value>,
    out: &mut ParameterEdits,
) -> Result<(), Error> {
    for (key, value) in object {
        if !valid_ident(key) {
            return Err(Error::new(
                Reason::InvalidField,
                key,
                format!("invalid parameter field: {key}"),
            ));
        }
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            Value::Object(map) => {
                if lookup(schema_value(), &path)
                    .and_then(Value::as_object)
                    .is_none()
                {
                    return Err(Error::new(
                        Reason::UnknownGroup,
                        &path,
                        format!("unknown parameter group: {path}"),
                    ));
                }
                flatten_object(&path, map, out)?;
            }
            _ => out.push_detailed(&path, value.clone())?,
        }
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.split('.').any(|part| !valid_ident(part)) {
        return Err(format!("invalid parameter path: {path}"));
    }
    Ok(())
}
fn valid_ident(s: &str) -> bool {
    !s.is_empty()
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
pub(crate) fn lookup<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(root, |v, key| v.as_object()?.get(key))
}
fn set_value_detailed(root: &mut Value, path: &str, value: Value) -> Result<(), Error> {
    let mut cur = root;
    let mut parts = path.split('.').peekable();
    while let Some(key) = parts.next() {
        let map = cur.as_object_mut().ok_or_else(|| {
            Error::new(
                Reason::ObjectRequired,
                key,
                format!("parameter group is not an object: {key}"),
            )
        })?;
        if parts.peek().is_none() {
            map.insert(key.to_owned(), value);
            return Ok(());
        }
        cur = map.get_mut(key).ok_or_else(|| {
            Error::new(
                Reason::UnknownPath,
                path,
                format!("unknown parameter path: {path}"),
            )
        })?;
    }
    Err(Error::new(
        Reason::UnknownPath,
        path,
        format!("invalid parameter path: {path}"),
    ))
}

/// Insert `value` at `path`, creating intermediate objects as needed.
/// Unlike `set_value`, which requires the path to already exist.
pub(crate) fn insert(root: &mut Value, path: &str, value: Value) {
    let mut parts = path.split('.').peekable();
    let mut current = root;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            current[part] = value;
            return;
        }
        if current.get(part).is_none() {
            current[part] = serde_json::json!({});
        }
        current = &mut current[part];
    }
}

fn parse_inline_value(path: &str, raw: &str) -> Result<Value, String> {
    let expected = lookup(schema_value(), path).ok_or_else(|| unknown_path(path))?;
    if expected.is_object() {
        return Err(group_rejected(path));
    }
    let ty = rust_type(path);
    let base_ty = ty
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(&ty);
    let string_value = !matches!(
        base_ty,
        "bool" | "f32" | "f64" | "u32" | "u64" | "i32" | "i64"
    ) && !base_ty.starts_with('[');
    if string_value && !raw.starts_with('"') && !(raw == "null" && nullable(path)) {
        return Ok(Value::String(raw.to_owned()));
    }
    if let Ok(value) = serde_json::from_str(raw) {
        return Ok(value);
    }
    if matches!(base_ty, "f32" | "f64")
        && let Ok(number) = raw.parse::<f64>()
        && let Some(number) = serde_json::Number::from_f64(number)
    {
        return Ok(Value::Number(number));
    }
    let form = if string_value {
        "text or a double-quoted string"
    } else if base_ty == "bool" {
        "true or false"
    } else if base_ty.starts_with('[') {
        "a numeric array, such as [1,2,3]"
    } else {
        "a finite number"
    };
    let unset = if nullable(path) {
        "; use null to clear it"
    } else {
        ""
    };
    Err(format!("{path}: expected {form}{unset}, got {raw:?}"))
}

#[cfg(test)]
mod inline_value_tests {
    use super::*;

    #[test]
    fn strict_carrier_and_sparse_errors_retain_owned_causes() {
        let error = parse_carrier_detailed(
            r#"{"camera":{"auto_exposure":true,"auto_exposure":false}}"#,
            false,
        )
        .unwrap_err();
        assert_eq!(error.reason, Reason::DuplicateJsonKey);
        assert_eq!(
            error.arguments[0],
            ("value", "\"auto_exposure\"".to_owned())
        );
        assert_eq!(error.arguments[1], ("line", "1".to_owned()));
        assert_eq!(error.arguments[2].0, "column");
        assert!(error.arguments[2].1.parse::<usize>().unwrap() > 0);
        let error = parse_carrier_detailed("when = 1979-05-27T07:32:00Z", true).unwrap_err();
        assert_eq!(error.reason, Reason::TomlDatetime);
        assert_eq!(error.path, "when");
        let error = ParameterEdits::from_value_detailed(
            serde_json::json!({"camera": {"film_format_mm": 0}}),
        )
        .unwrap_err();
        assert_eq!(error.reason, Reason::FilmFormat);
        assert_eq!(error.path, "camera.film_format_mm");
        let mut value = serde_json::json!({"enlarger": {"normalize_print_exposure": "yes"}});
        let error = normalize_legacy_conversion_paths_detailed(&mut value, true).unwrap_err();
        assert_eq!(error.reason, Reason::LegacyExposureBoolean);
    }

    #[test]
    fn plain_values_follow_parameter_types_and_preserve_quoted_forms() {
        let source = "film_render.grain.engine=procedural,film_render.grain.procedural_profile=custom,io.output_color_space=ProPhoto RGB,camera.auto_exposure=false,camera.exposure_compensation_ev=+.5,camera.filter_uv=[1,2,3],film_render.grain.procedural_amount=null";
        let mut params = RuntimeParams::default();
        ParameterEdits::parse(source)
            .unwrap()
            .apply(&mut params)
            .unwrap();
        assert_eq!(
            params.film_render.grain.engine,
            super::super::grain::GrainEngine::Procedural
        );
        assert_eq!(params.film_render.grain.procedural_profile, "custom");
        assert_eq!(params.io.output_color_space, "ProPhoto RGB");
        assert!(!params.camera.auto_exposure);
        assert_eq!(params.camera.exposure_compensation_ev, 0.5);
        assert_eq!(params.camera.filter_uv, [1.0, 2.0, 3.0]);
        assert_eq!(params.film_render.grain.procedural_amount, None);
        ParameterEdits::parse(r#"io.output_color_space="sRGB",film_render.grain.engine="spektra""#)
            .unwrap()
            .apply(&mut params)
            .unwrap();
        assert_eq!(params.io.output_color_space, "sRGB");
        assert_eq!(
            params.film_render.grain.engine,
            super::super::grain::GrainEngine::Spektra
        );
    }

    #[test]
    fn unquoted_text_keeps_equals_and_file_extensions() {
        let mut params = RuntimeParams::default();
        ParameterEdits::parse("film_render.convert.calibration=example=value.json")
            .unwrap()
            .apply(&mut params)
            .unwrap();
        assert_eq!(params.film_render.convert.calibration, "example=value.json");
        ParameterEdits::parse(r#"film_render.convert.calibration="a,b=c""#)
            .unwrap()
            .apply(&mut params)
            .unwrap();
        assert_eq!(params.film_render.convert.calibration, "a,b=c");
    }

    #[test]
    fn invalid_typed_values_do_not_become_text_or_disappear_under_overrides() {
        for source in [
            "camera.auto_exposure=yes,camera.auto_exposure=false",
            "camera.exposure_compensation_ev=fast,camera.exposure_compensation_ev=0",
            "camera.exposure_compensation_ev=inf",
            "film_render.grain.engine=invalid,film_render.grain.engine=procedural",
            "settings.preview_max_size=1.5",
            "camera.filter_uv=1,2,3",
            r#"camera.auto_exposure="false""#,
            r#"camera.exposure_compensation_ev="0.5""#,
        ] {
            assert!(ParameterEdits::parse(source).is_err(), "accepted {source}");
        }
        let error = ParameterEdits::parse("camera.auto_exposure=yes").unwrap_err();
        assert!(error.contains("true or false"));
        let error = ParameterEdits::parse("film_render.grain.engine=unknown").unwrap_err();
        assert!(error.contains("spektra") && error.contains("procedural"));
    }
}

fn validate_leaf(path: &str, value: &Value, expected: &Value) -> Result<(), Error> {
    if value.is_null() {
        if nullable(path) {
            return Ok(());
        }
        return Err(Error::new(
            Reason::NullNotAllowed,
            path,
            format!("{path}: null is only valid for nullable parameters"),
        ));
    }
    if !finite_json(value) {
        return Err(Error::new(
            Reason::NonFinite,
            path,
            format!("{path}: non-finite numbers are not allowed"),
        ));
    }
    if expected.is_object() {
        return Err(group_rejected_detailed(path));
    }
    if value.is_object() {
        return Err(Error::new(
            Reason::LeafRequired,
            path,
            format!("{path}: assignments must address scalar or array leaves"),
        ));
    }
    let ty = rust_type(path);
    let base_ty = ty
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(&ty);
    let ok = if let Some(arr) = base_ty.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let (inner, len) = arr
            .split_once(';')
            .map(|(inner, n)| (inner.trim(), n.trim()))
            .unwrap_or((arr, "0"));
        let len = len.parse::<usize>().unwrap_or(0);
        let element_ok = |v: &Value| match inner {
            "bool" => v.is_boolean(),
            _ => v.is_number(),
        };
        value
            .as_array()
            .is_some_and(|a| a.len() == len && a.iter().all(element_ok))
    } else if base_ty == "bool" {
        value.is_boolean()
    } else if matches!(base_ty, "f32" | "f64" | "u32" | "u64" | "i32" | "i64") {
        value.is_number()
    } else {
        value.is_string()
    };
    if !ok {
        return Err(Error::new(
            Reason::TypeMismatch,
            path,
            format!("{path}: expected {ty}, supplied {value}"),
        )
        .arg("expected", &ty)
        .arg("value", value));
    }
    if base_ty.contains("f32") {
        let fits = |v: &Value| v.as_f64().is_some_and(|n| (n as f32).is_finite());
        if !value
            .as_array()
            .map(|a| a.iter().all(fits))
            .unwrap_or_else(|| fits(value))
        {
            return Err(Error::new(
                Reason::F32Range,
                path,
                format!("{path}: value exceeds finite f32 range"),
            )
            .arg("value", value));
        }
    }
    if path == "camera.film_format_mm" {
        let wire = value.as_f64().unwrap();
        super::validation::validate_film_format_mm_detailed(wire)?;
        if (wire as f32) <= 0.0 {
            return Err(Error::new(Reason::F32Underflow, path, format!(
                "{path}: {wire} is below the smallest positive value the stored f32 field can hold"
            )).arg("value", wire));
        }
    }
    enum_check(path, value)?;
    range_check(path, value)?;
    // Deserialize the sparse one-leaf overlay against serde defaults to catch
    // integer/f32 overflow and exact Rust types; no whole-runtime state is
    // validated here, so temporarily conflicting intermediate compositions
    // remain legal.
    let mut overlay = Map::new();
    let mut cur = &mut overlay;
    let parts: Vec<_> = path.split('.').collect();
    for p in &parts[..parts.len() - 1] {
        cur.insert((*p).into(), Value::Object(Map::new()));
        cur = cur.get_mut(*p).unwrap().as_object_mut().unwrap();
    }
    cur.insert(parts[parts.len() - 1].into(), value.clone());
    serde_json::from_value::<RuntimeParams>(Value::Object(overlay)).map_err(|error| {
        Error::new(
            Reason::Representation,
            path,
            format!("{path}: value cannot be represented by parameter type: {error}"),
        )
        .arg("detail", error)
    })?;
    Ok(())
}

fn finite_json(v: &Value) -> bool {
    match v {
        Value::Number(n) => n.as_f64().map(|x| x.is_finite()).unwrap_or(false),
        Value::Array(a) => a.iter().all(finite_json),
        _ => true,
    }
}

fn enum_check(path: &str, value: &Value) -> Result<(), Error> {
    if let Some(s) = value.as_str() {
        super::validation::validate_enum_value_detailed(path, s)?;
    }
    Ok(())
}
fn range_check(path: &str, value: &Value) -> Result<(), Error> {
    if path.starts_with("scanner.operator.") {
        return super::validation::validate_scan_operator_value_detailed(
            path,
            value.as_f64().expect("numeric operator leaf"),
        );
    }
    if super::validation::gamut_component_domains(path).is_some() {
        let values: [f32; 3] = serde_json::from_value(value.clone()).map_err(|error| {
            Error::new(
                Reason::GamutRepresentation,
                path,
                format!("{path}: invalid gamut triplet: {error}"),
            )
            .arg("detail", error)
        })?;
        super::validation::validate_gamut_triplet_detailed(path, values)?;
        return Ok(());
    }
    if path == "film_render.grain.procedural_timer" {
        if let Some(n) = value.as_f64() {
            super::validation::validate_procedural_timer_detailed(f64::from(n as f32))?;
        }
        return Ok(());
    }
    let Some((min, max)) = super::validation::leaf_numeric_bounds(path) else {
        return Ok(());
    };
    let nums: Vec<f64> = match value {
        Value::Number(n) => n.as_f64().into_iter().collect(),
        Value::Array(a) => a.iter().filter_map(Value::as_f64).collect(),
        _ => vec![],
    };
    if path.starts_with("film_render.grain.fotufilm.") {
        return super::validation::validate_fotufilm_values_detailed(path, &nums);
    }
    let too_low = nums
        .iter()
        .any(|x| *x < min || (path == "io.input_gamut_compress.hull_detail" && *x <= 0.0));
    let too_high = max.is_some_and(|hi| nums.iter().any(|x| *x > hi));
    if too_low || too_high {
        let maximum = max.map(|m| m.to_string()).unwrap_or_else(|| "inf".into());
        return Err(Error::new(
            Reason::OutsideRange,
            path,
            format!("{path}: value outside [{min}, {maximum}]"),
        )
        .arg("min", min)
        .arg("max", maximum)
        .arg("value", value));
    }
    Ok(())
}

pub(crate) fn parse_carrier(text: &str, toml: bool) -> Result<Value, String> {
    parse_carrier_detailed(text, toml).map_err(|error| error.to_string())
}

pub(crate) fn parse_carrier_detailed(text: &str, toml: bool) -> Result<Value, Error> {
    if toml {
        toml_to_json(text.parse::<toml::Value>().map_err(|error| {
            Error::new(Reason::NativeToml, "", format!("invalid TOML: {error}"))
                .arg("detail", error)
        })?)
    } else {
        // Strict JSON: serde_json's Value keeps the last duplicate key
        // silently, so decode through a visitor that rejects duplicates
        // after full escape decoding.
        struct StrictValue<'a>(&'a std::cell::RefCell<Option<(Reason, String)>>);
        // Nested objects also go through StrictValue; serde_json's own Value
        // deserialization would silently keep the last duplicate key.
        impl<'de> serde::de::DeserializeSeed<'de> for StrictValue<'_> {
            type Value = Value;
            fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
                d.deserialize_any(self)
            }
        }
        impl<'de> serde::de::Visitor<'de> for StrictValue<'_> {
            type Value = Value;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a JSON value without duplicate object keys")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<Value, A::Error> {
                let mut map = Map::new();
                while let Some(key) = access.next_key::<String>()? {
                    if map.contains_key(&key) {
                        *self.0.borrow_mut() = Some((Reason::DuplicateJsonKey, key.clone()));
                        return Err(serde::de::Error::custom(format!(
                            "duplicate JSON object key {key:?}"
                        )));
                    }
                    let value = access.next_value_seed(StrictValue(self.0))?;
                    map.insert(key, value);
                }
                Ok(Value::Object(map))
            }
            fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
                Ok(Value::Number(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
                Ok(Value::Number(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
                serde_json::Number::from_f64(v)
                    .map(Value::Number)
                    .ok_or_else(|| {
                        *self.0.borrow_mut() = Some((Reason::NonFinite, String::new()));
                        E::custom("non-finite numbers are not allowed")
                    })
            }
            fn visit_str<E>(self, v: &str) -> Result<Value, E> {
                Ok(Value::String(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> Result<Value, E> {
                Ok(Value::String(v))
            }
            fn visit_none<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_unit<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
                d.deserialize_any(self)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<Value, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = access.next_element_seed(StrictValue(self.0))? {
                    items.push(item);
                }
                Ok(Value::Array(items))
            }
        }
        let owned = std::cell::RefCell::new(None);
        let failure = |error: serde_json::Error| {
            let english = format!("invalid JSON: {error}");
            match owned.borrow_mut().take() {
                Some((reason, path)) => Error::new(reason, &path, english)
                    .arg("value", format!("{path:?}"))
                    .arg("line", error.line())
                    .arg("column", error.column()),
                None => Error::new(Reason::NativeJson, "", english).arg("detail", error),
            }
        };
        let mut deserializer = serde_json::Deserializer::from_str(text);
        let value = serde::de::DeserializeSeed::deserialize(StrictValue(&owned), &mut deserializer)
            .map_err(failure)?;
        deserializer.end().map_err(failure)?;
        Ok(value)
    }
}
fn toml_to_json(v: toml::Value) -> Result<Value, Error> {
    fn convert(v: toml::Value, path: &str) -> Result<Value, Error> {
        match v {
            toml::Value::String(s) => Ok(Value::String(s)),
            toml::Value::Integer(i) => Ok(Value::Number(i.into())),
            toml::Value::Float(f) => serde_json::Number::from_f64(f)
                .map(Value::Number)
                .ok_or_else(|| {
                    Error::new(
                        Reason::TomlNonFinite,
                        path,
                        format!("{path}: TOML non-finite float is not allowed"),
                    )
                }),
            toml::Value::Boolean(b) => Ok(Value::Bool(b)),
            toml::Value::Datetime(_) => Err(Error::new(
                Reason::TomlDatetime,
                path,
                format!("{path}: TOML datetime values are not allowed"),
            )),
            toml::Value::Array(a) => Ok(Value::Array(
                a.into_iter()
                    .enumerate()
                    .map(|(index, value)| convert(value, &format!("{path}[{index}]")))
                    .collect::<Result<_, _>>()?,
            )),
            toml::Value::Table(t) => Ok(Value::Object(
                t.into_iter()
                    .map(|(key, value)| {
                        let next = if path.is_empty() {
                            key.clone()
                        } else {
                            format!("{path}.{key}")
                        };
                        Ok((key, convert(value, &next)?))
                    })
                    .collect::<Result<_, Error>>()?,
            )),
        }
    }
    convert(v, "")
}
fn find_equals(s: &str) -> Option<usize> {
    let (mut depth, mut quote, mut esc) = (0, false, false);
    for (i, c) in s.char_indices() {
        if quote {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                quote = false;
            }
        } else if c == '"' {
            quote = true;
        } else if c == '[' {
            depth += 1;
        } else if c == ']' {
            depth -= 1;
        } else if c == '=' && depth == 0 {
            return Some(i);
        }
    }
    None
}
fn split_top_level(s: &str) -> Result<Vec<&str>, String> {
    let mut out = Vec::new();
    let mut start = 0;
    let (mut depth, mut quote, mut esc) = (0, false, false);
    for (i, c) in s.char_indices() {
        if quote {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                quote = false;
            }
        } else if c == '"' {
            quote = true;
        } else if c == '[' {
            depth += 1;
        } else if c == ']' {
            depth -= 1;
            if depth < 0 {
                return Err("unbalanced array".into());
            }
        } else if c == ',' && depth == 0 {
            out.push(&s[start..i]);
            start = i + 1;
        }
    }
    if quote || depth != 0 {
        return Err("unterminated string or array".into());
    }
    out.push(&s[start..]);
    Ok(out)
}
fn unknown_path(path: &str) -> String {
    unknown_path_detailed(path).to_string()
}

fn unknown_path_detailed(path: &str) -> Error {
    if path == "io.output_gamut_compress.lightness_compression" {
        return retired_lightness_error_detailed();
    }
    if let Some(replacement) = retired_cmyk_replacement(path) {
        return Error::new(Reason::RetiredPath, path, format!(
            "{path} was retired; use {replacement} instead; a versioned legacy complete document may migrate it, but current-schema documents and sparse sources must use the canonical path"
        )).arg("replacement", replacement);
    }
    if let Some((_, reason)) = non_settable_leaves().iter().find(|(leaf, _)| *leaf == path) {
        return Error::new(
            Reason::NonSettable,
            path,
            format!("{path} is a runtime field outside the settable parameter surface; {reason}"),
        );
    }
    let suggestions = suggest_parameter_paths(path);
    Error::new(Reason::UnknownPath, path, format!(
        "unknown parameter path: {path}; {}", error_suffix(&suggestions,
            "use spektrafilm describe --module MODULE --format text to list fields, then --field PATH to describe one")
    )).arg("suggestions", suggestions.join(", "))
}

/// The canonical replacement for a retired `magazine_print_color` leaf, or
/// `None` when the path is not a retired CMYK path.
fn retired_cmyk_replacement(path: &str) -> Option<&'static str> {
    match path {
        "magazine_print_color.active" => Some("cmyk.active"),
        "magazine_print_color.strength" => Some("cmyk.strength"),
        "magazine_print_color" => Some("cmyk"),
        _ => None,
    }
}

/// One shared rejection for a value assigned to a parameter group.
fn group_rejected(path: &str) -> String {
    group_rejected_detailed(path).to_string()
}

fn group_rejected_detailed(path: &str) -> Error {
    Error::new(
        Reason::GroupAssignment,
        path,
        format!(
            "{path}: assign a parameter within this group, not the whole group; spektrafilm describe --module {path} lists its fields"
        ),
    )
}

pub fn validate_finite(params: &RuntimeParams) -> Result<(), String> {
    validate_finite_detailed(params).map_err(|error| error.to_string())
}

pub fn validate_finite_detailed(params: &RuntimeParams) -> Result<(), Error> {
    let value = serde_json::to_value(params).map_err(|error| {
        Error::new(Reason::SerializeParameters, "", error.to_string()).arg("detail", error)
    })?;
    fn check(path: &str, value: &Value) -> Result<(), Error> {
        match value {
            Value::Null if !nullable(path) => Err(Error::new(
                Reason::NumericFinite,
                path,
                format!("{path}: numeric value must be finite"),
            )),
            Value::Object(m) => {
                for (key, v) in m {
                    check(
                        &if path.is_empty() {
                            key.clone()
                        } else {
                            format!("{path}.{key}")
                        },
                        v,
                    )?;
                }
                Ok(())
            }
            Value::Array(a) => {
                if a.iter().any(Value::is_null) {
                    Err(Error::new(
                        Reason::ArrayFinite,
                        path,
                        format!("{path}: array values must be finite"),
                    ))
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }
    check("", &value)?;
    let grain = &params.film_render.grain;
    for (path, value) in [
        ("procedural_size", grain.procedural_size),
        ("procedural_amount", grain.procedural_amount),
        ("procedural_shadows", grain.procedural_shadows),
        ("procedural_midtones", grain.procedural_midtones),
        ("procedural_highlights", grain.procedural_highlights),
        ("procedural_chroma", grain.procedural_chroma),
        (
            "procedural_resolution_factor",
            grain.procedural_resolution_factor,
        ),
    ] {
        if value.is_some_and(|v| !v.is_finite()) {
            return Err(Error::new(
                Reason::NumericFinite,
                format!("film_render.grain.{path}"),
                format!("film_render.grain.{path}: numeric value must be finite"),
            ));
        }
    }
    for (path, value) in [
        (
            "film_render.development_time",
            params.film_render.development_time,
        ),
        (
            "print_render.development_time",
            params.print_render.development_time,
        ),
    ] {
        if value.is_some_and(|v| !v.is_finite()) {
            return Err(Error::new(
                Reason::NumericFinite,
                path,
                format!("{path}: numeric value must be finite"),
            ));
        }
    }
    if params
        .film_render
        .grain
        .fotufilm
        .layers
        .iter()
        .chain(params.film_render.grain.fotufilm.layer_weights.iter())
        .chain(params.film_render.grain.fotufilm.layer_size_ratio.iter())
        .chain(params.film_render.grain.fotufilm.sublayer_shares.iter())
        .any(|x| !x.is_finite())
        || [
            params.film_render.grain.fotufilm.amount,
            params.film_render.grain.fotufilm.size,
            params.film_render.grain.fotufilm.softness,
            params.film_render.grain.fotufilm.color,
            params.film_render.grain.fotufilm.rms_density,
            params.film_render.grain.fotufilm.clump_radius_um,
            params.film_render.grain.fotufilm.radius_span,
            params.film_render.grain.fotufilm.coating_density_scale,
            params.film_render.grain.fotufilm.fog_density,
        ]
        .iter()
        .any(|x| !x.is_finite())
    {
        return Err(Error::new(
            Reason::ArrayFinite,
            "",
            "parameter array values must be finite",
        ));
    }
    params.film_render.grain.fotufilm.validate_detailed()?;
    Ok(())
}

#[cfg(test)]
mod fotufilm_tests {
    use super::*;

    #[test]
    fn fotufilm_metadata_has_exact_types_bounds_units_materials() {
        let fields = [
            ("amount", "f64", 0.0, 4.0, "dimensionless"),
            ("size", "f64", 0.5, 4.0, "dimensionless"),
            ("softness", "f64", 0.5, 4.0, "dimensionless"),
            ("color", "f64", 0.0, 1.0, "dimensionless"),
            ("layers", "[f64; 3]", 0.0, 4.0, "dimensionless"),
            ("rms_density", "f64", 0.0, 0.1, "optical density"),
            ("clump_radius_um", "f64", 0.0, 32.0, "µm"),
            ("layer_weights", "[f64; 3]", 0.0, 4.0, "dimensionless"),
            ("layer_size_ratio", "[f64; 3]", 0.25, 4.0, "dimensionless"),
            ("radius_span", "f64", 1.0, 20.0, "dimensionless"),
            ("sublayer_shares", "[f64; 4]", 0.0, 1.0, "dimensionless"),
            ("coating_density_scale", "f64", 0.25, 4.0, "dimensionless"),
            ("fog_density", "f64", 0.0, 0.3, "optical density"),
        ];
        for (name, ty, min, max, unit) in fields {
            let path = format!("film_render.grain.fotufilm.{name}");
            let field = describe_field(&path).unwrap();
            assert_eq!(field["type"], ty);
            assert_eq!(field["minimum"], min);
            assert_eq!(field["maximum"], max);
            assert_eq!(field["unit"], unit);
            assert!(
                field["conditions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == "film_render.grain.engine = fotufilm")
            );
        }
        let material = describe_field("film_render.grain.fotufilm.material").unwrap();
        assert_eq!(material["type"], "FotufilmMaterial");
        assert_eq!(
            material["enum_values"],
            serde_json::json!(["auto", "silver", "dye_negative", "dye_reversal"])
        );
        assert_eq!(
            describe_module("film_render.grain.fotufilm").unwrap()["fields"]
                .as_array()
                .unwrap()
                .len(),
            14
        );
    }

    #[test]
    fn operator_sparse_sources_keep_f64_and_do_not_clamp_editor_ranges() {
        let mut params = RuntimeParams::default();
        ParameterEdits::parse("scanner.operator.exposure_ev=4.000000000001,scanner.operator.warmth_ev=-2,scanner.operator.tint_ev=2,scanner.operator.contrast=3")
            .unwrap().apply(&mut params).unwrap();
        assert_eq!(params.scanner.operator.exposure_ev, 4.000000000001);
        assert_eq!(params.scanner.operator.contrast, 3.0);
        for path in ["exposure_ev", "warmth_ev", "tint_ev", "contrast"] {
            let record = describe_field(&format!("scanner.operator.{path}")).unwrap();
            assert_eq!(record["type"], "f64");
            assert_eq!(
                record["unit"],
                if path == "contrast" {
                    "dimensionless"
                } else {
                    "EV"
                }
            );
            assert!(record["maximum"].is_null());
            assert!(ParameterEdits::parse(&format!("scanner.operator.{path}=inf")).is_err());
        }
        for value in ["0", "-0.5"] {
            assert!(ParameterEdits::parse(&format!("scanner.operator.contrast={value}")).is_err());
        }
    }

    #[test]
    fn fotufilm_sparse_domains_and_share_sum_reject_invalid_input() {
        for (name, values) in [
            ("amount", vec!["-1", "4.1"]),
            ("size", vec!["0.49", "4.1"]),
            ("softness", vec!["0.49", "4.1"]),
            ("color", vec!["-0.1", "1.1"]),
            ("layers", vec!["[1,-1,1]", "[1,4.1,1]"]),
            ("rms_density", vec!["0", "0.11"]),
            ("clump_radius_um", vec!["0", "33"]),
            ("layer_weights", vec!["[1,0,1]", "[1,4.1,1]"]),
            ("layer_size_ratio", vec!["[1,0.24,1]", "[1,4.1,1]"]),
            ("radius_span", vec!["0.9", "21"]),
            (
                "sublayer_shares",
                vec!["[0,0.5,0.25,0.25]", "[0.25,0.25,0.25,0.2]"],
            ),
            ("coating_density_scale", vec!["0.24", "4.1"]),
            ("fog_density", vec!["-0.1", "0.31"]),
            ("material", vec!["dye", "v4"]),
        ] {
            let path = format!("film_render.grain.fotufilm.{name}");
            for value in values {
                let error = ParameterEdits::parse(&format!("{path}={value}")).unwrap_err();
                assert!(error.contains(&path), "{error}");
            }
        }
        ParameterEdits::parse("film_render.grain.fotufilm.sublayer_shares=[0.1,0.2,0.3,0.4]")
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::PrintExposureMode;

    #[test]
    fn format_sources_reject_invalid_and_preserve_fractional_values() {
        for text in [
            "camera.film_format_mm=0",
            "camera.film_format_mm=-1",
            "camera.film_format_mm=nan",
            "camera.film_format_mm=inf",
            "camera.film_format_mm=1e-100",
        ] {
            assert!(ParameterEdits::parse(text).is_err(), "{text}");
        }
        let error = ParameterEdits::parse("camera.film_format_mm=1e-46").unwrap_err();
        assert!(error.contains("smallest positive value"), "{error}");
        for mm in [35.0, 35.5, 120.0, 150.25] {
            let source = ParameterEdits::parse(&format!("camera.film_format_mm={mm}")).unwrap();
            let mut params = RuntimeParams::default();
            source.apply(&mut params).unwrap();
            params.validate().unwrap();
            assert_eq!(params.camera.film_format_mm, mm);
        }
        let record = describe_field("camera.film_format_mm").unwrap();
        let conditions = record["conditions"].as_array().unwrap();
        assert!(
            conditions
                .iter()
                .any(|c| c.as_str().is_some_and(|t| t.contains("strictly positive"))),
            "discovery must state the strict-positivity refinement: {record}"
        );
    }

    #[test]
    fn single_field_discovery_reuses_module_records() {
        for group in describe_groups() {
            for record in describe_module(&group).unwrap()["fields"]
                .as_array()
                .unwrap()
            {
                let path = record["path"].as_str().unwrap();
                assert_eq!(describe_field(path).unwrap(), *record, "{path}");
            }
        }
        assert!(
            describe_field("film_render.grain")
                .unwrap_err()
                .contains("--module film_render.grain")
        );
        assert!(
            describe_module("film_render.grain.engine")
                .unwrap_err()
                .contains("--field film_render.grain.engine")
        );
    }

    #[test]
    fn discoverable_groups_are_sorted_and_editable() {
        let groups = describe_groups();
        assert!(groups.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(groups.iter().any(|group| group == "film_render.grain"));
        assert!(!groups.iter().any(|group| group == "workflow"));
        for group in groups {
            assert!(
                !describe_module(&group).unwrap()["fields"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn root_groups_are_the_first_segment_of_editable_groups() {
        let roots = describe_root_groups();
        let mut expected: Vec<String> = describe_groups()
            .into_iter()
            .map(|group| group.split('.').next().unwrap().to_owned())
            .collect();
        expected.dedup();
        assert_eq!(roots, expected);
        assert!(roots.iter().any(|group| group == "film_render"));
        assert!(!roots.iter().any(|group| group.contains('.')));
        for root in &roots {
            assert!(
                !describe_module(root).unwrap()["fields"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn out_of_band_runtime_leaf_names_field_and_reason() {
        let error = ParameterEdits::parse("random_seed=17").unwrap_err();
        assert!(error.contains("random_seed"), "{error}");
        assert!(
            error.contains("runtime field outside the settable parameter surface"),
            "{error}"
        );
        assert!(error.contains("recipe runner"), "{error}");
    }

    #[test]
    fn group_values_share_one_rejection_across_sources() {
        let inline = ParameterEdits::parse("film_render.convert=nope").unwrap_err();
        let file =
            ParameterEdits::from_value(serde_json::json!({"film_render": {"convert": "nope"}}))
                .unwrap_err();
        assert_eq!(inline, file);
        assert!(inline.contains("describe --module film_render.convert"));
    }

    #[test]
    fn unknown_leaves_suggest_canonical_paths_or_discovery() {
        let typo = "film_render.grain.engin";
        assert_eq!(suggest_parameter_paths(typo), ["film_render.grain.engine"]);
        for error in [
            describe_field(typo).unwrap_err(),
            ParameterEdits::parse("film_render.grain.engin=v2").unwrap_err(),
            ParameterEdits::from_value(
                serde_json::json!({"film_render": {"grain": {"engin": "procedural"}}}),
            )
            .unwrap_err(),
        ] {
            assert!(
                error.contains("did you mean film_render.grain.engine?"),
                "{error}"
            );
        }
        assert!(suggest_parameter_paths("completely_unrelated.field").is_empty());
        let error = describe_field("completely_unrelated.field").unwrap_err();
        assert!(error.contains("spektrafilm describe --module MODULE --format text"));
        assert!(error.contains("--field"));
        assert!(
            ParameterEdits::parse("camera.unknown_leaf=1")
                .unwrap_err()
                .contains("--module MODULE")
        );
        assert!(
            ParameterEdits::from_value(serde_json::json!({"camera": {"unknown_leaf": 1}}))
                .unwrap_err()
                .contains("--module MODULE")
        );
        assert!(suggest_parameter_paths("workflow.route").is_empty());
    }

    #[test]
    fn unknown_groups_suggest_canonical_prefixes_or_discovery() {
        assert_eq!(suggest_groups("film_render.grai"), ["film_render.grain"]);
        assert!(
            describe_module("film_render.grai")
                .unwrap_err()
                .contains("did you mean film_render.grain?")
        );
        assert!(suggest_groups("completely_unrelated").is_empty());
        assert!(
            describe_module("completely_unrelated")
                .unwrap_err()
                .contains("spektrafilm describe --format text")
        );
    }

    #[test]
    fn inline_apply_is_ordered_and_preserves_runtime_state() {
        let edits = ParameterEdits::parse(
            "camera.auto_exposure=false,camera.auto_exposure=true,camera.filter_uv=[1,2,3]",
        )
        .unwrap();
        assert_eq!(edits.paths().count(), 3);
        let mut params = RuntimeParams {
            random_seed: 7,
            neutral_print_filters_protected: Some([true, false, true]),
            ..Default::default()
        };
        edits.apply(&mut params).unwrap();
        assert!(params.camera.auto_exposure);
        assert_eq!(params.camera.filter_uv, [1.0, 2.0, 3.0]);
        assert_eq!(params.random_seed, 7);
        assert_eq!(
            params.neutral_print_filters_protected,
            Some([true, false, true])
        );
        // The persisted flags are themselves settable and round-trip through
        // the serialize-apply-deserialize path like any other leaf.
        let flags =
            ParameterEdits::parse("neutral_print_filters_protected=[false,true,false]").unwrap();
        flags.apply(&mut params).unwrap();
        assert_eq!(
            params.neutral_print_filters_protected,
            Some([false, true, false])
        );
    }

    #[test]
    fn rejects_unknown_forbidden_and_invalid_leaves() {
        assert!(ParameterEdits::parse("no_such_module.field=1").is_err());
        assert!(ParameterEdits::parse("workflow.route=\"input\"").is_err());
        assert!(ParameterEdits::parse("film_render.grain.monochrome=true").is_err());
        assert!(ParameterEdits::parse("camera.auto_exposure=null").is_err());
        assert!(ParameterEdits::parse("camera.exposure_compensation_ev=3.5e38").is_err());
        assert!(
            ParameterEdits::parse("camera.diffusion_filter.filter_family=\"unknown\"").is_err()
        );
        assert!(ParameterEdits::parse("film_render.grain.procedural_timer=1.0").is_err());
        assert!(
            ParameterEdits::parse(
                "film_render.grain.procedural_timer=0.999999999,film_render.grain.procedural_timer=0.5"
            )
            .is_err()
        );
        assert!(ParameterEdits::parse("film_render.grain.procedural_size=200").is_err());
    }

    #[test]
    fn nullable_leaves_accept_null_and_finite_values() {
        let edits = ParameterEdits::parse("film_render.grain.procedural_amount=null,film_render.grain.procedural_amount=25,film_render.grain.procedural_timer=0.5").unwrap();
        let mut params = RuntimeParams::default();
        edits.apply(&mut params).unwrap();
        assert_eq!(params.film_render.grain.procedural_amount, Some(25.0));
        assert_eq!(params.film_render.grain.procedural_timer, 0.5);
        assert!(ParameterEdits::parse("film_render.grain.procedural_amount=nan").is_err());
    }

    #[test]
    fn capture_illuminant_accepts_explicit_names_blackbodies_and_null() {
        let mut params = RuntimeParams::default();
        assert_eq!(params.camera.capture_illuminant, None);
        for source in [
            "camera.capture_illuminant=BB4000",
            r#"camera.capture_illuminant="BB4000""#,
            "camera.capture_illuminant=D75",
            "camera.capture_illuminant=TH-KG3",
        ] {
            let mut params = RuntimeParams::default();
            ParameterEdits::parse(source)
                .unwrap()
                .apply(&mut params)
                .unwrap();
            assert!(
                params.camera.capture_illuminant.is_some(),
                "{source} left the leaf unset"
            );
            params.validate().unwrap();
        }
        let mut explicit = RuntimeParams::default();
        explicit.camera.capture_illuminant = Some("BB4000".into());
        ParameterEdits::parse("camera.capture_illuminant=null")
            .unwrap()
            .apply(&mut explicit)
            .unwrap();
        assert_eq!(explicit.camera.capture_illuminant, None);
        // A null on an unset leaf stays unset.
        ParameterEdits::parse("camera.capture_illuminant=null")
            .unwrap()
            .apply(&mut params)
            .unwrap();
        assert_eq!(params.camera.capture_illuminant, None);
    }

    #[test]
    fn capture_illuminant_rejects_unsupported_values_with_the_canonical_path() {
        for source in [
            "camera.capture_illuminant=not-an-illuminant",
            "camera.capture_illuminant=BB1000",
            "camera.capture_illuminant=BB25001",
            "camera.capture_illuminant=",
        ] {
            assert!(ParameterEdits::parse(source).is_err(), "accepted {source}");
        }
        let error =
            ParameterEdits::parse("camera.capture_illuminant=not-an-illuminant").unwrap_err();
        assert!(error.starts_with("camera.capture_illuminant:"), "{error}");
        assert!(error.contains("BB<kelvin>"), "{error}");
    }

    #[test]
    fn json_rejects_escape_decoded_duplicate_keys() {
        assert!(
            parse_carrier(
                r#"{"camera":{"auto_exposure":false},"camer\u0061":{}}"#,
                false
            )
            .is_err()
        );
        assert!(
            parse_carrier(
                r#"{"camera":{"auto_exposure":false,"auto_exposure":true}}"#,
                false
            )
            .is_err()
        );
    }

    #[test]
    fn describe_reports_types_nullable_bounds_and_vocabulary() {
        let module = describe_module("camera").unwrap();
        assert_eq!(module["module"], "camera");
        assert!(
            module["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["path"] == "camera.exposure_compensation_ev"
                    && f["type"] == "f32"
                    && f["unit"] == "EV")
        );
        let grain = describe_module("film_render.grain").unwrap();
        let fields = grain["fields"].as_array().unwrap();
        assert!(
            fields
                .iter()
                .any(|f| f["path"] == "film_render.grain.procedural_amount"
                    && f["nullable"] == true
                    && f["minimum"] == 0.0
                    && f["maximum"] == 100.0)
        );
        assert!(fields.iter().any(|f| {
            f["path"] == "film_render.grain.engine"
                && f["enum_values"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|e| e == "procedural"))
        }));
        let camera = describe_module("camera").unwrap();
        assert!(camera["fields"].as_array().unwrap().iter().any(|f| {
            f["path"] == "camera.capture_illuminant"
                && f["type"] == "Option<String>"
                && f["nullable"] == true
                && f["default"] == Value::Null
                && f["enum_values"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|e| e == "BB<kelvin> (1667..=25000)"))
        }));
        assert!(describe_module("workflow").is_err());
        assert!(describe_module("unknown").is_err());
    }

    #[test]
    fn gamut_domains_reject_invalid_leaves_even_when_overwritten() {
        for path in [
            "io.input_gamut_compress.knee",
            "io.output_gamut_compress.knee",
        ] {
            for invalid in [
                "[-1,1,1]",
                "[1,1,1]",
                "[0.999999999,1,1]",
                "[0.9,0,1]",
                "[0.9,-1,1]",
                "[0.9,1,0]",
                "[0.9,1,-1]",
                "[0.9,1e100,1]",
                "[0.9,1,1e-100]",
            ] {
                let source = format!("{path}={invalid},{path}=[0.9,1,1]");
                assert!(ParameterEdits::parse(&source).is_err(), "{source}");
                let mut document = RuntimeParams::default();
                let mut root = serde_json::json!({"io": {
                    "input_gamut_compress": {}, "output_gamut_compress": {}
                }});
                set_value_detailed(&mut root, path, serde_json::from_str(invalid).unwrap())
                    .unwrap();
                assert!(
                    ParameterEdits::from_value(root.clone()).is_err(),
                    "{path}={invalid}"
                );
                set_value_detailed(&mut root, path, serde_json::json!([0.9, 1, 1])).unwrap();
                ParameterEdits::from_value(root).unwrap();
                // Valid boundary values remain accepted in the typed domain.
                ParameterEdits::parse(&format!("{path}=[0,1,1]"))
                    .unwrap()
                    .apply(&mut document)
                    .unwrap();
            }
        }
    }

    #[test]
    fn apply_rejects_optional_overflow_in_legacy_baseline_before_serialization() {
        let mut params = RuntimeParams::default();
        params.film_render.grain.procedural_amount = Some(f32::INFINITY);
        assert!(
            params
                .film_render
                .grain
                .procedural_amount
                .unwrap()
                .is_infinite()
        );
        let edits = ParameterEdits::parse("camera.auto_exposure=false").unwrap();
        let error = edits.apply(&mut params).unwrap_err();
        assert!(
            error.contains("film_render.grain.procedural_amount"),
            "{error}"
        );
        assert!(
            params
                .film_render
                .grain
                .procedural_amount
                .unwrap()
                .is_infinite()
        );
        assert!(params.camera.auto_exposure);
    }

    #[test]
    fn json_numeric_decoding_rejects_overflow_and_preserves_legacy_finite_range() {
        // Preset File formats: reject numbers overflowing their declared f32/f64
        // type. Correctly rounded JSON decoding rejects overflow before merging.
        assert!(
            serde_json::from_str::<RuntimeParams>(
                r#"{"film_render":{"grain":{"procedural_amount":1e100}}}"#
            )
            .is_err()
        );
        let params: RuntimeParams = serde_json::from_str(r#"{"film_render":{"grain":{"procedural_amount":42.5}},"camera":{"exposure_compensation_ev":0.5}}"#).unwrap();
        assert_eq!(params.film_render.grain.procedural_amount, Some(42.5));
        assert_eq!(params.camera.exposure_compensation_ev, 0.5);
    }

    fn described_field(path: &str) -> Value {
        let module = path.rsplit_once('.').unwrap().0;
        describe_module(module).unwrap()["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["path"] == path)
            .unwrap()
            .clone()
    }

    #[test]
    fn discovery_reports_gamut_component_domains() {
        for path in [
            "io.input_gamut_compress.knee",
            "io.output_gamut_compress.knee",
        ] {
            let field = described_field(path);
            assert!(field["minimum"].is_null());
            let domains = field["component_domains"].as_array().unwrap();
            assert_eq!(domains.len(), 3);
            assert_eq!(domains[0]["component"], "threshold");
            assert_eq!(domains[0]["minimum"], 0.0);
            assert_eq!(domains[0]["minimum_exclusive"], false);
            assert_eq!(domains[0]["maximum"], 1.0);
            assert_eq!(domains[0]["maximum_exclusive"], true);
            for (domain, name) in domains[1..].iter().zip(["limit", "power"]) {
                assert_eq!(domain["component"], name);
                assert_eq!(domain["minimum"], 0.0);
                assert_eq!(domain["minimum_exclusive"], true);
                assert!(domain["maximum"].is_null());
            }
        }
    }

    #[test]
    fn legacy_conversion_paths_rename_and_preserve_the_four_states() {
        for (pair, mode, tracking) in [
            (
                serde_json::json!({"normalize_print_exposure": true, "print_exposure_compensation": true}),
                "auto",
                "follow_mode",
            ),
            (
                serde_json::json!({"normalize_print_exposure": false, "print_exposure_compensation": false}),
                "manual",
                "follow_mode",
            ),
            (
                serde_json::json!({"normalize_print_exposure": true, "print_exposure_compensation": false}),
                "auto",
                "never",
            ),
            (
                serde_json::json!({"normalize_print_exposure": false, "print_exposure_compensation": true}),
                "manual",
                "always",
            ),
            // Sparse legacy documents default each absent leaf to true.
            (
                serde_json::json!({"normalize_print_exposure": false}),
                "manual",
                "always",
            ),
            (
                serde_json::json!({"print_exposure_compensation": false}),
                "auto",
                "never",
            ),
        ] {
            let mut value = serde_json::json!({ "enlarger": pair });
            normalize_legacy_complete_params(&mut value).unwrap();
            let params: RuntimeParams = serde_json::from_value(value).unwrap();
            assert_eq!(params.enlarger.print_exposure_mode.as_str(), mode);
            assert_eq!(
                params.settings.print_exposure_ev_tracking.as_str(),
                tracking
            );
        }
        // Idempotent on already-canonical documents.
        let mut canonical = serde_json::to_value(RuntimeParams::default()).unwrap();
        normalize_legacy_complete_params(&mut canonical).unwrap();
        let round: RuntimeParams = serde_json::from_value(canonical).unwrap();
        assert_eq!(round.enlarger.print_exposure_mode, PrintExposureMode::Auto);
        // Both spellings together are a hard error, not a silent overwrite.
        let mut both = serde_json::json!({
            "enlarger": {
                "normalize_print_exposure": true,
                "print_exposure_compensation": false,
                "print_exposure_mode": "auto",
            }
        });
        assert!(normalize_legacy_complete_params(&mut both).is_err());
        // Legacy scanner conversion leaves are retired with the scan
        // surface: they stay untouched and fail unknown-field validation.
        let mut retired = serde_json::json!({
            "scanner": { "positive_interpretation": "coupled" }
        });
        assert!(normalize_legacy_complete_params(&mut retired).is_ok());
        assert!(serde_json::from_value::<RuntimeParams>(retired).is_err());
    }

    #[test]
    fn retired_lightness_migration_is_explicit_and_atomic() {
        for algorithm in ["off", "aces_rgc", "oklch"] {
            for retired in [serde_json::json!([0.95, 1.0, 1.6]), Value::Null] {
                let mut value = serde_json::json!({"io":{"output_color_space":"ProPhoto RGB","output_gamut_compress":{"algorithm":algorithm,"lightness_compression":retired}}});
                assert!(serde_json::from_value::<RuntimeParams>(value.clone()).is_err());
                assert!(ParameterEdits::from_value(value.clone()).is_err());
                let original = value.clone();
                if retired.is_null() {
                    normalize_legacy_complete_params(&mut value).unwrap();
                    let params: RuntimeParams = serde_json::from_value(value).unwrap();
                    assert_eq!(params.io.output_color_space, "ProPhoto RGB");
                    assert_eq!(params.output_mix.amount, 1.0);
                } else {
                    let error = normalize_legacy_complete_params(&mut value).unwrap_err();
                    assert!(error.contains("io.output_gamut_compress.lightness_compression"));
                    assert!(error.contains("no automatic replacement"));
                    assert!(error.contains("explicitly remove"));
                    assert_eq!(value, original);
                }
            }
        }
        assert!(
            ParameterEdits::parse("io.output_gamut_compress.lightness_compression=null").is_err()
        );
    }

    #[test]
    fn legacy_cmyk_migration_is_transactional_and_idempotent() {
        // Legacy group migrates onto the canonical name.
        let mut value = serde_json::json!({
            "magazine_print_color": {"active": true, "strength": 0.75}
        });
        normalize_legacy_complete_params(&mut value).unwrap();
        let params: RuntimeParams = serde_json::from_value(value.clone()).unwrap();
        assert!(params.cmyk.active);
        assert_eq!(params.cmyk.strength, 0.75);
        assert!(value.get("magazine_print_color").is_none());
        assert_eq!(value["cmyk"]["active"], true);
        assert_eq!(value["cmyk"]["strength"], 0.75);

        // Idempotent: re-running on the migrated document is a no-op.
        let migrated = value.clone();
        normalize_legacy_complete_params(&mut value).unwrap();
        assert_eq!(value, migrated);

        // Legacy route migrates onto the base route with cmyk active.
        let mut value = serde_json::json!({
            "workflow": {"route": "input > film > scan > magazine"}
        });
        normalize_legacy_complete_params(&mut value).unwrap();
        let params: RuntimeParams = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(params.workflow.route, "input > film > scan");
        assert!(params.cmyk.active);
        assert_eq!(value["workflow"]["route"], "input > film > scan");
        assert_eq!(value["cmyk"]["active"], true);

        // Idempotent on the migrated route document.
        let migrated = value.clone();
        normalize_legacy_complete_params(&mut value).unwrap();
        assert_eq!(value, migrated);

        // Unrelated fields are preserved.
        let mut value = serde_json::json!({
            "magazine_print_color": {"active": true, "strength": 0.5},
            "io": {"output_color_space": "ProPhoto RGB"},
            "camera": {"film_format_mm": 120.0}
        });
        normalize_legacy_complete_params(&mut value).unwrap();
        let params: RuntimeParams = serde_json::from_value(value).unwrap();
        assert_eq!(params.io.output_color_space, "ProPhoto RGB");
        assert_eq!(params.camera.film_format_mm, 120.0);
        assert!(params.cmyk.active);
        assert_eq!(params.cmyk.strength, 0.5);
    }

    #[test]
    fn legacy_cmyk_conflicts_reject_atomically() {
        // Both spellings together are a hard error.
        let mut both = serde_json::json!({
            "magazine_print_color": {"active": true, "strength": 0.75},
            "cmyk": {"active": false, "strength": 1.0}
        });
        let original = both.clone();
        let error = normalize_legacy_complete_params(&mut both).unwrap_err();
        assert!(error.contains("magazine_print_color"), "{error}");
        assert!(error.contains("cmyk"), "{error}");
        assert_eq!(both, original);

        // A legacy route and a legacy group with an explicit active leaf
        // conflict: the route implies cmyk.active = true.
        let mut conflict = serde_json::json!({
            "workflow": {"route": "input > film > scan > magazine"},
            "magazine_print_color": {"active": false}
        });
        let original = conflict.clone();
        let error = normalize_legacy_complete_params(&mut conflict).unwrap_err();
        assert!(error.contains("retired route"), "{error}");
        assert_eq!(conflict, original);

        // A non-object legacy group is rejected before mutation.
        let mut bad = serde_json::json!({"magazine_print_color": 42});
        let original = bad.clone();
        assert!(normalize_legacy_complete_params(&mut bad).is_err());
        assert_eq!(bad, original);
    }

    #[test]
    fn current_schema_rejects_retired_cmyk_paths() {
        // Sparse sources reject the retired spelling with an actionable
        // replacement.
        for (path, replacement) in [
            ("magazine_print_color.active", "cmyk.active"),
            ("magazine_print_color.strength", "cmyk.strength"),
        ] {
            let error = ParameterEdits::parse(&format!("{path}=true")).unwrap_err();
            assert!(error.contains(path), "{error}");
            assert!(error.contains(replacement), "{error}");
            assert!(error.contains("was retired"), "{error}");
        }
        // Discovery rejects the retired group and leaves.
        assert!(describe_field("magazine_print_color.active").is_err());
        assert!(describe_field("magazine_print_color.strength").is_err());
        assert!(describe_module("magazine_print_color").is_err());
        // Current-schema documents reject the retired group.
        assert!(
            serde_json::from_value::<RuntimeParams>(serde_json::json!({
                "magazine_print_color": {"active": true}
            }))
            .is_err()
        );
        // The retired route is not selectable.
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan > magazine".into();
        let error = params.validate().unwrap_err();
        assert!(error.contains("input > film > scan"), "{error}");
        assert!(error.contains("cmyk.active = true"), "{error}");
    }

    #[test]
    fn cmyk_discovery_exposes_canonical_fields() {
        let active = describe_field("cmyk.active").unwrap();
        assert_eq!(active["type"], "bool");
        assert_eq!(active["default"], false);
        let strength = describe_field("cmyk.strength").unwrap();
        assert_eq!(strength["type"], "f64");
        assert_eq!(strength["default"], 1.0);
        assert_eq!(strength["minimum"], 0.0);
        assert_eq!(strength["maximum"], 1.0);
        let module = describe_module("cmyk").unwrap();
        assert_eq!(module["module"], "cmyk");
        let paths: Vec<_> = module["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert!(paths.contains(&"cmyk.active"));
        assert!(paths.contains(&"cmyk.strength"));
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn cmyk_inactive_reason_reports_activation_condition() {
        use crate::params::RuntimeParams;
        use crate::profile::Profile;
        let params = RuntimeParams::default();
        let film: Profile = serde_json::from_str(r#"{"metadata":{},"info":{},"data":{}}"#).unwrap();
        let print = film.clone();
        // Default is inactive: cmyk.active is false.
        let reason = inactive_reason("cmyk.strength", &params, &film, &print);
        assert_eq!(reason.as_deref(), Some("cmyk.active = true"));
        // Active with strength > 0 has no inactive reason.
        let mut active = params.clone();
        active.cmyk.active = true;
        active.cmyk.strength = 0.5;
        assert_eq!(
            inactive_reason("cmyk.strength", &active, &film, &print),
            None
        );
        // Active with strength 0 reports the strength condition.
        let mut zero = params.clone();
        zero.cmyk.active = true;
        zero.cmyk.strength = 0.0;
        assert_eq!(
            inactive_reason("cmyk.strength", &zero, &film, &print).as_deref(),
            Some("cmyk.strength > 0")
        );
    }

    #[test]
    fn dye_look_controls_are_effective_and_legacy_values_are_retained() {
        use super::effects::conditions;
        for path in [
            "film_render.grain.dye_strength",
            "film_render.grain.dye_scale",
        ] {
            let described: Vec<_> = conditions(path).iter().map(|c| c.description()).collect();
            assert!(
                described
                    .iter()
                    .any(|d| d.starts_with("film_render.grain.engine = dye;")),
                "missing the Dye engine condition for {path}: {described:?}"
            );
            assert!(
                !described
                    .iter()
                    .any(|d| d.starts_with("retained legacy Dye")),
                "an effective Dye control must not be marked retained: {path}: {described:?}"
            );
        }
        for path in [
            "film_render.grain.dye_support_um",
            "film_render.grain.dye_event_area_um2",
            "film_render.grain.channel_correlation",
        ] {
            let described: Vec<_> = conditions(path).iter().map(|c| c.description()).collect();
            assert!(
                described
                    .iter()
                    .any(|d| d.starts_with("retained legacy Dye")),
                "missing the retained condition for {path}: {described:?}"
            );
            assert!(
                !described
                    .iter()
                    .any(|d| d.starts_with("film_render.grain.engine = spektra")),
                "a Dye-only control must not claim the Spektra engine: {path}: {described:?}"
            );
        }
    }

    #[test]
    fn rms_granularity_is_effective_only_for_spektra_without_layer_controls() {
        let film: crate::profile::Profile =
            serde_json::from_str(r#"{"metadata":{},"info":{},"data":{}}"#).unwrap();
        let mut params = RuntimeParams::default();
        ParameterEdits::parse("film_render.grain.rms_granularity=[4,5,6]")
            .unwrap()
            .apply(&mut params)
            .unwrap();
        let path = "film_render.grain.rms_granularity";
        params.film_render.grain.engine = crate::params::GrainEngine::Spektra;
        assert_eq!(inactive_reason(path, &params, &film, &film), None);
        for engine in [
            crate::params::GrainEngine::Dye,
            crate::params::GrainEngine::Procedural,
        ] {
            params.film_render.grain.engine = engine;
            assert_eq!(
                inactive_reason(path, &params, &film, &film).as_deref(),
                Some("film_render.grain.engine = spektra")
            );
        }
    }

    #[test]
    fn legacy_dye_values_are_retained_but_inactive() {
        use crate::profile::Profile;
        let color: Profile = serde_json::from_value(serde_json::json!({
            "metadata": {}, "info": {"channel_model": "color"}, "data": {}
        }))
        .unwrap();
        let mut bw = color.clone();
        bw.info.channel_model = "bw".into();
        let mut params = RuntimeParams::default();
        params.film_render.grain.active = true;
        params.film_render.grain.engine = crate::params::GrainEngine::Dye;
        for (path, value) in [
            ("film_render.grain.channel_correlation", "0.35"),
            ("film_render.grain.dye_event_area_um2", "2.25"),
            ("film_render.grain.dye_support_um", "4.5"),
        ] {
            ParameterEdits::parse(&format!("{path}={value}"))
                .unwrap()
                .apply(&mut params)
                .unwrap();
            let expected =
                Some("retained legacy Dye value; the delivered Dye law does not read it");
            for film in [&bw, &color] {
                assert_eq!(
                    inactive_reason(path, &params, film, film).as_deref(),
                    expected,
                    "{path}"
                );
            }
        }
        assert_eq!(params.film_render.grain.channel_correlation, 0.35);
        assert_eq!(params.film_render.grain.dye_event_area_um2, 2.25);
        assert_eq!(params.film_render.grain.dye_support_um, 4.5);
    }

    #[test]
    fn legacy_conversion_rejections_leave_documents_unchanged() {
        // A mixed print-exposure pair needs the settings group; a non-object
        // there must fail before any mutation instead of panicking.
        let mut value = serde_json::json!({
            "enlarger": {"normalize_print_exposure": true, "print_exposure_compensation": false},
            "settings": 42
        });
        let original = value.clone();
        let error = normalize_legacy_conversion_paths(&mut value, true).unwrap_err();
        assert!(error.contains("settings"), "{error}");
        assert_eq!(value, original);

        // Combining legacy conversion paths with a rejected retired lightness
        // leaf must fail atomically, before any rename.
        let mut both = serde_json::json!({
            "scanner": {"positive_density_ev": 0.4},
            "enlarger": {"normalize_print_exposure": true, "print_exposure_compensation": true},
            "io": {"output_gamut_compress": {"lightness_compression": [0.95, 1.0, 1.6]}}
        });
        let original = both.clone();
        let error = normalize_legacy_complete_params(&mut both).unwrap_err();
        assert!(error.contains("lightness_compression"), "{error}");
        assert_eq!(both, original);
    }

    #[test]
    fn mix_sources_share_f64_domain_and_reject_withdrawn_paths() {
        for value in [0.0, 0.1234567890123456, 1.0] {
            let mut inline = RuntimeParams::default();
            ParameterEdits::parse(&format!("output_mix.amount={value}"))
                .unwrap()
                .apply(&mut inline)
                .unwrap();
            for toml in [false, true] {
                let text = if toml {
                    format!("[output_mix]\namount = {value}")
                } else {
                    serde_json::json!({"output_mix":{"amount":value}}).to_string()
                };
                let mut params = RuntimeParams::default();
                ParameterEdits::from_value(parse_carrier(&text, toml).unwrap())
                    .unwrap()
                    .apply(&mut params)
                    .unwrap();
                assert_eq!(params.output_mix.amount, inline.output_mix.amount);
            }
        }
        for invalid in ["-0.01", "1.01", "null", "nan", "inf"] {
            assert!(
                ParameterEdits::parse(&format!("output_mix.amount={invalid},output_mix.amount=1"))
                    .is_err()
            );
        }
        for path in [
            "film_render.response",
            "print_render.response",
            "enlarger.c_filter_shift",
        ] {
            assert!(ParameterEdits::parse(&format!("{path}=0")).is_err());
        }
        assert!(describe_field("io.output_gamut_compress.lightness_compression").is_err());
        let field = describe_field("output_mix.amount").unwrap();
        assert_eq!(field["type"], "f64");
        assert_eq!(field["default"], 1.0);
        assert_eq!(field["minimum"], 0.0);
        assert_eq!(field["maximum"], 1.0);
    }

    #[test]
    fn material_discovery_reports_actual_units_and_capabilities() {
        let flash = describe_field("enlarger.preflash_exposure").unwrap();
        assert_eq!(flash["default"], 0.0);
        assert_eq!(flash["unit"], "linear exposure multiplier");
        assert!(
            flash["conditions"]
                .to_string()
                .contains("print-development")
        );
        for path in [
            "enlarger.preflash_m_filter_shift",
            "enlarger.preflash_y_filter_shift",
        ] {
            let field = describe_field(path).unwrap();
            assert_eq!(field["unit"], "Kodak CC");
            assert_eq!(field["default"], 0.0);
            assert!(
                field["conditions"]
                    .to_string()
                    .contains("preflash_exposure > 0")
            );
        }
        for path in [
            "film_render.chemistry.developer_exhaustion",
            "print_render.density_curves_morph.developer_exhaustion",
        ] {
            let field = describe_field(path).unwrap();
            assert_eq!(field["unit"], "dimensionless");
            assert_eq!(field["default"], 0.0);
            assert_eq!(field["minimum"], 0.0);
            assert_eq!(field["maximum"], 1.0);
            let conditions = field["conditions"].to_string();
            assert!(conditions.contains("development"));
            assert!(conditions.contains("active = true"));
            assert!(conditions.contains("fitted density_curves_model"));
        }
    }

    #[test]
    fn typed_mix_validates_finite_endpoints_and_bounds() {
        for amount in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01, 1.01] {
            let mut params = RuntimeParams::default();
            params.output_mix.amount = amount;
            assert!(params.validate().unwrap_err().contains("output_mix.amount"));
        }
        for amount in [0.0, 1.0] {
            let mut params = RuntimeParams::default();
            params.output_mix.amount = amount;
            params.validate().unwrap();
        }
        let params: RuntimeParams =
            serde_json::from_value(serde_json::json!({"output_mix":{}})).unwrap();
        assert_eq!(params.output_mix.amount, 1.0);
        assert!(
            serde_json::from_value::<RuntimeParams>(
                serde_json::json!({"output_mix":{"amount":null}})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<RuntimeParams>(
                serde_json::json!({"output_mix":{"amount":1,"extra":0}})
            )
            .is_err()
        );
    }
}
