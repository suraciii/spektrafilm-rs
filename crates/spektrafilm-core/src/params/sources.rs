mod metadata;

use super::RuntimeParams;
use crate::suggest::{MAX_SUGGESTIONS, closest, error_suffix};
use metadata::{metadata, nullable, rust_type};
use serde_json::{Map, Value};

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
                format!("expected PATH=VALUE or a .toml/.json file, got {item:?}; example: --set film_render.grain.engine=v2")
            })?;
            let path = item[..eq].trim();
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
        let root = value
            .as_object()
            .ok_or_else(|| "parameter document root must be an object".to_string())?;
        let mut out = Self::default();
        flatten_object("", root, &mut out)?;
        Ok(out)
    }

    fn push(&mut self, path: &str, value: Value) -> Result<(), String> {
        let expected = lookup(schema_value(), path).ok_or_else(|| unknown_path(path))?;
        validate_leaf(path, &value, expected)?;
        self.edits.push((path.to_owned(), value));
        Ok(())
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.edits.iter().map(|(p, _)| p.as_str())
    }

    pub fn apply(&self, params: &mut RuntimeParams) -> Result<(), String> {
        // serde_json turns non-finite floats into null; reject the typed
        // baseline before that conversion can erase an invalid Option value.
        validate_finite(params)?;
        let seed = params.random_seed;
        let particle_layers = params.film_render.grain.particle_scale_layers;
        let neutral_protected = params.neutral_print_filters_protected;
        let mut root =
            serde_json::to_value(&*params).map_err(|e| format!("serialize parameters: {e}"))?;
        for (path, value) in &self.edits {
            set_value(&mut root, path, value.clone())?;
        }
        *params =
            serde_json::from_value(root).map_err(|e| format!("apply parameter edits: {e}"))?;
        params.random_seed = seed;
        params.film_render.grain.particle_scale_layers = particle_layers;
        params.neutral_print_filters_protected = neutral_protected;
        validate_finite(params)
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
) -> Result<(), String> {
    for (key, value) in object {
        if !valid_ident(key) {
            return Err(format!("invalid parameter field: {key}"));
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
                    return Err(format!("unknown parameter group: {path}"));
                }
                flatten_object(&path, map, out)?;
            }
            _ => out.push(&path, value.clone())?,
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
fn lookup<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(root, |v, key| v.as_object()?.get(key))
}
fn set_value(root: &mut Value, path: &str, value: Value) -> Result<(), String> {
    let mut cur = root;
    let mut parts = path.split('.').peekable();
    while let Some(key) = parts.next() {
        let map = cur
            .as_object_mut()
            .ok_or_else(|| format!("parameter group is not an object: {key}"))?;
        if parts.peek().is_none() {
            map.insert(key.to_owned(), value);
            return Ok(());
        }
        cur = map
            .get_mut(key)
            .ok_or_else(|| format!("unknown parameter path: {path}"))?;
    }
    Err(format!("invalid parameter path: {path}"))
}

fn parse_inline_value(path: &str, raw: &str) -> Result<Value, String> {
    let expected = lookup(schema_value(), path).ok_or_else(|| unknown_path(path))?;
    if expected.is_object() {
        return Err(format!(
            "{path}: assign a parameter within this group, not the whole group"
        ));
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
    if matches!(base_ty, "f32" | "f64") {
        if let Ok(number) = raw.parse::<f64>() {
            if let Some(number) = serde_json::Number::from_f64(number) {
                return Ok(Value::Number(number));
            }
        }
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
    fn plain_values_follow_parameter_types_and_preserve_quoted_forms() {
        let source = "film_render.grain.engine=v2,film_render.grain.v2_profile=custom,io.output_color_space=ProPhoto RGB,camera.auto_exposure=false,camera.exposure_compensation_ev=+.5,camera.filter_uv=[1,2,3],film_render.grain.v2_amount=null";
        let mut params = RuntimeParams::default();
        ParameterEdits::parse(source)
            .unwrap()
            .apply(&mut params)
            .unwrap();
        assert_eq!(
            params.film_render.grain.engine,
            super::super::grain::GrainEngine::V2
        );
        assert_eq!(params.film_render.grain.v2_profile, "custom");
        assert_eq!(params.io.output_color_space, "ProPhoto RGB");
        assert!(!params.camera.auto_exposure);
        assert_eq!(params.camera.exposure_compensation_ev, 0.5);
        assert_eq!(params.camera.filter_uv, [1.0, 2.0, 3.0]);
        assert_eq!(params.film_render.grain.v2_amount, None);
        ParameterEdits::parse(r#"io.output_color_space="sRGB",film_render.grain.engine="v1""#)
            .unwrap()
            .apply(&mut params)
            .unwrap();
        assert_eq!(params.io.output_color_space, "sRGB");
        assert_eq!(
            params.film_render.grain.engine,
            super::super::grain::GrainEngine::V1
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
            "film_render.grain.engine=invalid,film_render.grain.engine=v2",
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
        assert!(error.contains("v1") && error.contains("v2"));
    }
}

fn validate_leaf(path: &str, value: &Value, expected: &Value) -> Result<(), String> {
    if value.is_null() {
        if nullable(path) {
            return Ok(());
        }
        return Err(format!(
            "{path}: null is only valid for nullable parameters"
        ));
    }
    if !finite_json(value) {
        return Err(format!("{path}: non-finite numbers are not allowed"));
    }
    if expected.is_object() || value.is_object() {
        return Err(format!(
            "{path}: assignments must address scalar or array leaves"
        ));
    }
    let ty = rust_type(path);
    let base_ty = ty
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(&ty);
    let ok = if let Some(arr) = base_ty.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let len = arr
            .split_once(';')
            .and_then(|(_, n)| n.trim().parse::<usize>().ok())
            .unwrap_or(0);
        value
            .as_array()
            .is_some_and(|a| a.len() == len && a.iter().all(Value::is_number))
    } else if base_ty == "bool" {
        value.is_boolean()
    } else if matches!(base_ty, "f32" | "f64" | "u32" | "u64" | "i32" | "i64") {
        value.is_number()
    } else {
        value.is_string()
    };
    if !ok {
        return Err(format!("{path}: expected {ty}, supplied {value}"));
    }
    if base_ty.contains("f32") {
        let fits = |v: &Value| v.as_f64().is_some_and(|n| (n as f32).is_finite());
        if !value
            .as_array()
            .map(|a| a.iter().all(fits))
            .unwrap_or_else(|| fits(value))
        {
            return Err(format!("{path}: value exceeds finite f32 range"));
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
    serde_json::from_value::<RuntimeParams>(Value::Object(overlay))
        .map_err(|e| format!("{path}: value cannot be represented by parameter type: {e}"))?;
    Ok(())
}

fn finite_json(v: &Value) -> bool {
    match v {
        Value::Number(n) => n.as_f64().map(|x| x.is_finite()).unwrap_or(false),
        Value::Array(a) => a.iter().all(finite_json),
        _ => true,
    }
}

fn enum_check(path: &str, value: &Value) -> Result<(), String> {
    if let Some(s) = value.as_str() {
        super::validation::validate_enum_value(path, s)?;
    }
    Ok(())
}
fn range_check(path: &str, value: &Value) -> Result<(), String> {
    if super::validation::gamut_component_domains(path).is_some() {
        let values: [f32; 3] = serde_json::from_value(value.clone())
            .map_err(|e| format!("{path}: invalid gamut triplet: {e}"))?;
        super::validation::validate_gamut_triplet(path, values)?;
        return Ok(());
    }
    if path == "film_render.grain.v2_timer" {
        if let Some(n) = value.as_f64() {
            super::validation::validate_v2_timer(f64::from(n as f32))?;
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
    let too_low = nums
        .iter()
        .any(|x| *x < min || (path == "io.input_gamut_compress.hull_detail" && *x <= 0.0));
    let too_high = max.is_some_and(|hi| nums.iter().any(|x| *x > hi));
    if too_low || too_high {
        return Err(format!(
            "{path}: value outside [{min}, {}]",
            max.map(|m| m.to_string()).unwrap_or_else(|| "inf".into())
        ));
    }
    Ok(())
}

pub(crate) fn parse_carrier(text: &str, toml: bool) -> Result<Value, String> {
    if toml {
        toml_to_json(
            text.parse::<toml::Value>()
                .map_err(|e| format!("invalid TOML: {e}"))?,
        )
    } else {
        use serde::Deserialize;
        // Strict JSON: serde_json's Value keeps the last duplicate key
        // silently, so decode through a visitor that rejects duplicates
        // after full escape decoding.
        struct StrictValue;
        // Nested objects also go through StrictValue; serde_json's own Value
        // deserialization would silently keep the last duplicate key.
        struct StrictWrapper(Value);
        impl<'de> Deserialize<'de> for StrictWrapper {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                d.deserialize_any(StrictValue).map(StrictWrapper)
            }
        }
        impl<'de> serde::de::Visitor<'de> for StrictValue {
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
                        return Err(serde::de::Error::custom(format!(
                            "duplicate JSON object key {key:?}"
                        )));
                    }
                    let StrictWrapper(value) = access.next_value::<StrictWrapper>()?;
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
                    .ok_or_else(|| E::custom("non-finite numbers are not allowed"))
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
                d.deserialize_any(StrictValue)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<Value, A::Error> {
                let mut items = Vec::new();
                while let Some(StrictWrapper(item)) = access.next_element::<StrictWrapper>()? {
                    items.push(item);
                }
                Ok(Value::Array(items))
            }
        }
        let mut deserializer = serde_json::Deserializer::from_str(text);
        let value = serde::de::Deserializer::deserialize_any(&mut deserializer, StrictValue)
            .map_err(|e| format!("invalid JSON: {e}"))?;
        deserializer
            .end()
            .map_err(|e| format!("invalid JSON: {e}"))?;
        Ok(value)
    }
}
fn toml_to_json(v: toml::Value) -> Result<Value, String> {
    match v {
        toml::Value::String(s) => Ok(Value::String(s)),
        toml::Value::Integer(i) => Ok(Value::Number(i.into())),
        toml::Value::Float(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .ok_or_else(|| "TOML non-finite float is not allowed".into()),
        toml::Value::Boolean(b) => Ok(Value::Bool(b)),
        toml::Value::Datetime(_) => Err("TOML datetime values are not allowed".into()),
        toml::Value::Array(a) => Ok(Value::Array(
            a.into_iter().map(toml_to_json).collect::<Result<_, _>>()?,
        )),
        toml::Value::Table(t) => Ok(Value::Object(
            t.into_iter()
                .map(|(k, v)| Ok((k, toml_to_json(v)?)))
                .collect::<Result<_, String>>()?,
        )),
    }
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
    format!(
        "unknown parameter path: {path}; {}",
        error_suffix(
            &suggest_parameter_paths(path),
            "use spektrafilm describe --module MODULE --format text to list fields, then --field PATH to describe one"
        )
    )
}

/// Describe one editable leaf using the module discovery metadata.
pub fn describe_field(path: &str) -> Result<Value, String> {
    let default = lookup(schema_value(), path).ok_or_else(|| unknown_path(path))?;
    if default.is_object() {
        return Err(format!(
            "parameter field is a group: {path}; use spektrafilm describe --module {path}"
        ));
    }
    Ok(metadata(path, default))
}

/// Canonical group prefixes with editable leaves, including nested groups.
pub fn describe_groups() -> Vec<String> {
    fn collect(prefix: &str, value: &Value, groups: &mut Vec<String>) -> bool {
        let Some(map) = value.as_object() else {
            return true;
        };
        let mut has_leaves = false;
        for (key, child) in map {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            has_leaves |= collect(&path, child, groups);
        }
        if has_leaves && !prefix.is_empty() {
            groups.push(prefix.to_owned());
        }
        has_leaves
    }
    let mut groups = Vec::new();
    collect("", schema_value(), &mut groups);
    groups.sort();
    groups
}

pub fn suggest_parameter_paths(input: &str) -> Vec<String> {
    fn collect(prefix: &str, value: &Value, paths: &mut Vec<String>) {
        if let Some(map) = value.as_object() {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                collect(&path, child, paths);
            }
        } else {
            paths.push(prefix.to_owned());
        }
    }
    let mut paths = Vec::new();
    collect("", schema_value(), &mut paths);
    closest(input, paths.into_iter(), MAX_SUGGESTIONS)
}

pub fn suggest_groups(input: &str) -> Vec<String> {
    closest(input, describe_groups().into_iter(), MAX_SUGGESTIONS)
}

pub fn describe_module(prefix: &str) -> Result<Value, String> {
    let schema = schema_value();
    let group = lookup(schema, prefix).ok_or_else(|| {
        format!(
            "unknown parameter module: {prefix}; {}",
            error_suffix(
                &suggest_groups(prefix),
                "use spektrafilm describe --format text to list modules"
            )
        )
    })?;
    if !group.is_object() {
        return Err(format!(
            "parameter module is not a group: {prefix}; use spektrafilm describe --field {prefix}"
        ));
    }
    let mut fields = Vec::new();
    collect_metadata(prefix, group, &mut fields);
    Ok(serde_json::json!({"module": prefix, "fields": fields}))
}
fn collect_metadata(prefix: &str, value: &Value, out: &mut Vec<Value>) {
    if let Some(map) = value.as_object() {
        for (key, default) in map {
            let path = format!("{prefix}.{key}");
            if default.is_object() {
                collect_metadata(&path, default, out);
            } else {
                out.push(metadata(&path, default));
            }
        }
    }
}
fn schema_value() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        let mut value =
            serde_json::to_value(RuntimeParams::default()).expect("RuntimeParams serializes");
        value.as_object_mut().unwrap().remove("workflow");
        value["io"].as_object_mut().unwrap().remove("scan_film");
        value["film_render"]["grain"]
            .as_object_mut()
            .unwrap()
            .remove("monochrome");
        value
    })
}

pub fn validate_finite(params: &RuntimeParams) -> Result<(), String> {
    let value = serde_json::to_value(params).map_err(|e| e.to_string())?;
    fn check(path: &str, value: &Value) -> Result<(), String> {
        match value {
            Value::Null if !nullable(path) => Err(format!("{path}: numeric value must be finite")),
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
                    Err(format!("{path}: array values must be finite"))
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
        ("v2_size", grain.v2_size),
        ("v2_amount", grain.v2_amount),
        ("v2_shadows", grain.v2_shadows),
        ("v2_midtones", grain.v2_midtones),
        ("v2_highlights", grain.v2_highlights),
        ("v2_chroma", grain.v2_chroma),
        ("v2_resolution_factor", grain.v2_resolution_factor),
    ] {
        if value.is_some_and(|v| !v.is_finite()) {
            return Err(format!(
                "film_render.grain.{path}: numeric value must be finite"
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
            return Err(format!("{path}: numeric value must be finite"));
        }
    }
    if params
        .io
        .output_gamut_compress
        .lightness_compression
        .is_some_and(|v| v.iter().any(|x| !x.is_finite()))
        || params
            .film_render
            .grain
            .particle_scale_layers
            .iter()
            .any(|x| !x.is_finite())
    {
        return Err("parameter array values must be finite".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn unknown_leaves_suggest_canonical_paths_or_discovery() {
        let typo = "film_render.grain.engin";
        assert_eq!(suggest_parameter_paths(typo), ["film_render.grain.engine"]);
        for error in [
            describe_field(typo).unwrap_err(),
            ParameterEdits::parse("film_render.grain.engin=v2").unwrap_err(),
            ParameterEdits::from_value(
                serde_json::json!({"film_render": {"grain": {"engin": "v2"}}}),
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
        let mut params = RuntimeParams::default();
        params.random_seed = 7;
        params.film_render.grain.particle_scale_layers = [9., 8., 7.];
        params.neutral_print_filters_protected = [true, false, true];
        edits.apply(&mut params).unwrap();
        assert!(params.camera.auto_exposure);
        assert_eq!(params.camera.filter_uv, [1.0, 2.0, 3.0]);
        assert_eq!(params.random_seed, 7);
        assert_eq!(params.film_render.grain.particle_scale_layers, [9., 8., 7.]);
        assert_eq!(params.neutral_print_filters_protected, [true, false, true]);
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
        assert!(ParameterEdits::parse("film_render.grain.v2_timer=1.0").is_err());
        assert!(
            ParameterEdits::parse(
                "film_render.grain.v2_timer=0.999999999,film_render.grain.v2_timer=0.5"
            )
            .is_err()
        );
        assert!(ParameterEdits::parse("film_render.grain.v2_size=200").is_err());
    }

    #[test]
    fn nullable_leaves_accept_null_and_finite_values() {
        let edits = ParameterEdits::parse("film_render.grain.v2_amount=null,film_render.grain.v2_amount=25,film_render.grain.v2_timer=0.5").unwrap();
        let mut params = RuntimeParams::default();
        edits.apply(&mut params).unwrap();
        assert_eq!(params.film_render.grain.v2_amount, Some(25.0));
        assert_eq!(params.film_render.grain.v2_timer, 0.5);
        assert!(ParameterEdits::parse("film_render.grain.v2_amount=nan").is_err());
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
                .any(|f| f["path"] == "film_render.grain.v2_amount"
                    && f["nullable"] == true
                    && f["minimum"] == 0.0
                    && f["maximum"] == 100.0)
        );
        assert!(fields.iter().any(|f| {
            f["path"] == "film_render.grain.engine"
                && f["enum_values"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|e| e == "v2"))
        }));
        assert!(describe_module("workflow").is_err());
        assert!(describe_module("unknown").is_err());
    }

    #[test]
    fn gamut_domains_reject_invalid_leaves_even_when_overwritten() {
        for path in [
            "io.input_gamut_compress.knee",
            "io.output_gamut_compress.knee",
            "io.output_gamut_compress.lightness_compression",
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
                set_value(&mut root, path, serde_json::from_str(invalid).unwrap()).unwrap();
                assert!(
                    ParameterEdits::from_value(root.clone()).is_err(),
                    "{path}={invalid}"
                );
                set_value(&mut root, path, serde_json::json!([0.9, 1, 1])).unwrap();
                ParameterEdits::from_value(root).unwrap();
                // Valid boundary values remain accepted in the typed domain.
                ParameterEdits::parse(&format!("{path}=[0,1,1]"))
                    .unwrap()
                    .apply(&mut document)
                    .unwrap();
            }
        }
        ParameterEdits::parse("io.output_gamut_compress.lightness_compression=null").unwrap();
    }

    #[test]
    fn apply_rejects_optional_overflow_in_legacy_baseline_before_serialization() {
        let mut params: RuntimeParams =
            serde_json::from_str(r#"{"film_render":{"grain":{"v2_amount":1e100}}}"#).unwrap();
        assert!(params.film_render.grain.v2_amount.unwrap().is_infinite());
        let edits = ParameterEdits::parse("camera.auto_exposure=false").unwrap();
        let error = edits.apply(&mut params).unwrap_err();
        assert!(error.contains("film_render.grain.v2_amount"), "{error}");
        assert!(params.film_render.grain.v2_amount.unwrap().is_infinite());
        assert!(params.camera.auto_exposure);
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
            "io.output_gamut_compress.lightness_compression",
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
    fn discovery_matches_stock_initialization_and_consumer_conditions() {
        for path in [
            "film_render.grain.rms_granularity",
            "film_render.grain.density_min",
            "film_render.grain.uniformity",
            "film_render.grain.particle_scale_sublayers",
            "film_render.dir_couplers.gamma_samelayer_rgb",
            "film_render.dir_couplers.gamma_interlayer_r_to_gb",
            "film_render.dir_couplers.gamma_interlayer_g_to_rb",
            "film_render.dir_couplers.gamma_interlayer_b_to_rg",
            "film_render.halation.halation_first_sigma_um",
            "film_render.halation.halation_strength",
        ] {
            assert!(
                described_field(path)["default_source"]
                    .as_str()
                    .unwrap()
                    .contains("derived from stock profile"),
                "{path}"
            );
        }
        assert_eq!(
            described_field("film_render.dir_couplers.diffusion_size_um")["default_source"],
            "static"
        );
        for module in ["film_render.glare", "film_render.grain"] {
            let fields = describe_module(module).unwrap();
            for field in fields["fields"].as_array().unwrap() {
                let path = field["path"].as_str().unwrap();
                if module == "film_render.glare"
                    || path.ends_with("rms_granularity")
                    || path.ends_with("micro_sublayers")
                {
                    assert_eq!(
                        field["conditions"],
                        serde_json::json!(["preserved upstream no-op"])
                    );
                }
            }
        }
        for field in describe_module("print_render.glare").unwrap()["fields"]
            .as_array()
            .unwrap()
        {
            let condition = field["conditions"][0].as_str().unwrap();
            for required in [
                "io.scan_film = false",
                "print_render.glare.active",
                "print_render.glare.percent > 0",
                "stochastic effects enabled",
            ] {
                assert!(condition.contains(required), "{condition}");
            }
        }
        for name in ["boost_ev", "boost_range", "protect_ev"] {
            assert_eq!(
                described_field(&format!("film_render.halation.{name}"))["conditions"],
                serde_json::json!(["film_render.halation.boost_ev != 0; LUT mode disabled"])
            );
        }
        let composite = described_field("film_render.grain.n_sub_layers");
        assert!(
            composite["conditions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c == "film_render.grain.sublayers_active = false")
        );
        for name in [
            "particle_scale_sublayers",
            "blur_dye_clouds_um",
            "micro_structure",
            "mult_usm_sigma",
            "mult_usm_amount",
        ] {
            let field = described_field(&format!("film_render.grain.{name}"));
            assert!(
                field["conditions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c.as_str().unwrap().contains("sublayers_active = true")),
                "{name}"
            );
        }
        for (path, unit) in [
            ("settings.spectral_gaussian_blur", "nm"),
            ("scanner.lens_blur", "px (sigma)"),
            (
                "scanner.unsharp_mask",
                "[px (sigma), dimensionless (amount)]",
            ),
        ] {
            assert_eq!(described_field(path)["unit"], unit);
        }
    }
}
