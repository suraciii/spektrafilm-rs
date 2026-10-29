//! Read-only migration of historical grain control names. Runtime fields and
//! emitted documents use engine names; numbered spellings stay at this boundary.

use serde_json::Value;

pub(super) const CONTROL_NAMES: &[(&str, &str)] = &[
    ("v2_profile", "procedural_profile"),
    ("v2_film_type", "procedural_film_type"),
    ("v2_mode", "procedural_mode"),
    ("v2_size", "procedural_size"),
    ("v2_amount", "procedural_amount"),
    ("v2_shadows", "procedural_shadows"),
    ("v2_midtones", "procedural_midtones"),
    ("v2_highlights", "procedural_highlights"),
    ("v2_chroma", "procedural_chroma"),
    ("v2_resolution_type", "procedural_resolution_type"),
    ("v2_timer", "procedural_timer"),
    ("v2_resolution_factor", "procedural_resolution_factor"),
    ("v3_dye_support_um", "dye_support_um"),
    ("v3_event_area_um2", "dye_event_area_um2"),
];

pub(super) fn canonical_control(name: &str) -> &str {
    CONTROL_NAMES
        .iter()
        .find_map(|&(old, new)| (old == name).then_some(new))
        .unwrap_or(name)
}

/// Canonicalize a persisted parameter ownership path without accepting unknown leaves.
pub(super) fn canonical_grain_path(path: &str) -> std::borrow::Cow<'_, str> {
    let Some(control) = path.strip_prefix("film_render.grain.") else {
        return std::borrow::Cow::Borrowed(path);
    };
    let canonical = canonical_control(control);
    if canonical == control {
        return std::borrow::Cow::Borrowed(path);
    }
    std::borrow::Cow::Owned(format!("film_render.grain.{canonical}"))
}

pub(super) fn deserialize_preparation_paths<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    use serde::Deserialize;
    Ok(Vec::<String>::deserialize(deserializer)?
        .into_iter()
        .map(|path| canonical_grain_path(&path).into_owned())
        .collect())
}

/// Normalize only a grain object. Check conflicts before changing any key.
/// Engine selector values and unknown fields are deliberately not migrated.
pub fn normalize_grain_control_names(value: &mut Value) -> Result<(), String> {
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    for &(old, new) in CONTROL_NAMES {
        if object.contains_key(old) && object.contains_key(new) {
            return Err(format!(
                "grain carries both {old} and {new}; keep exactly one spelling"
            ));
        }
    }
    for &(old, new) in CONTROL_NAMES {
        if let Some(value) = object.remove(old) {
            object.insert(new.into(), value);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{GrainParams, RuntimeParams, sources::ParameterEdits};
    use serde_json::json;

    #[test]
    fn numbered_control_inputs_emit_only_engine_names() {
        let canonical = json!({
            "engine":"procedural", "procedural_profile":"custom",
            "procedural_film_type":"positive", "procedural_mode":"noise",
            "procedural_size":12.0,"procedural_amount":27.0,
            "procedural_shadows":31.0,"procedural_midtones":41.0,
            "procedural_highlights":51.0,"procedural_chroma":61.0,
            "procedural_resolution_type":0,"procedural_timer":0.25,
            "procedural_resolution_factor":73.0,"dye_support_um":4.5,"dye_event_area_um2":0.6
        });
        let expected: GrainParams = serde_json::from_value(canonical.clone()).unwrap();
        let mut historical = canonical.clone();
        for &(old, new) in CONTROL_NAMES {
            let value = historical.as_object_mut().unwrap().remove(new).unwrap();
            historical[old] = value;
        }
        assert_eq!(
            serde_json::from_value::<GrainParams>(historical.clone()).unwrap(),
            expected
        );
        let toml_source = toml::to_string(historical.as_object().unwrap()).unwrap();
        assert_eq!(
            toml::from_str::<GrainParams>(&toml_source).unwrap(),
            expected
        );
        let saved = serde_json::to_value(&expected).unwrap();
        for &(old, new) in CONTROL_NAMES {
            assert!(saved.get(old).is_none());
            assert_eq!(saved[new], canonical[new]);
            let source = format!("film_render.grain.{old}={}", historical[old]);
            let edits = ParameterEdits::parse(&source).unwrap();
            assert_eq!(
                edits.paths().collect::<Vec<_>>(),
                [format!("film_render.grain.{new}")]
            );
        }
        let edits =
            ParameterEdits::from_value(json!({"film_render":{"grain":historical}})).unwrap();
        let mut params = RuntimeParams::default();
        edits.apply(&mut params).unwrap();
        assert_eq!(params.film_render.grain, expected);
    }

    #[test]
    fn numbered_control_conflicts_and_unknown_keys_stay_errors() {
        for &(old, new) in CONTROL_NAMES {
            // Null still counts as supplied, even where it means inheritance.
            let mut both = json!({});
            both[old] = Value::Null;
            both[new] = Value::Null;
            assert!(serde_json::from_value::<GrainParams>(both.clone()).is_err());
            assert!(
                ParameterEdits::from_value(json!({"film_render":{"grain":both.clone()}})).is_err()
            );
            let original = both.clone();
            assert!(normalize_grain_control_names(&mut both).is_err());
            assert_eq!(both, original);
        }
        assert!(serde_json::from_value::<GrainParams>(json!({"procedural_amunt":20})).is_err());
        for engine in ["v1", "v2", "v3"] {
            assert!(serde_json::from_value::<GrainParams>(json!({"engine":engine})).is_err());
        }
        let edits = ParameterEdits::parse(
            "film_render.grain.v2_amount=20,film_render.grain.procedural_amount=30",
        )
        .unwrap();
        let mut params = RuntimeParams::default();
        edits.apply(&mut params).unwrap();
        assert_eq!(params.film_render.grain.procedural_amount, Some(30.0));
    }
}
