use crate::params::RuntimeParams;
use crate::suggest::{MAX_SUGGESTIONS, closest, error_suffix};
use serde_json::Value;
use std::sync::LazyLock;

use super::metadata::metadata;

/// Describe one editable leaf using the module discovery metadata.
pub fn describe_field(path: &str) -> Result<Value, String> {
    if super::source_option_path(path) {
        return Ok(
            serde_json::json!({"path":path,"type":"source_option","default":null,"validation":"Original pinned Fotufilm preparation producer; use engine catalogue for stock-specific options"}),
        );
    }
    let default = super::lookup(schema_value(), path).ok_or_else(|| super::unknown_path(path))?;
    if default.is_object() {
        return Err(format!(
            "parameter field is a group: {path}; use spektrafilm describe --module {path}"
        ));
    }
    Ok(metadata(path, default))
}

fn collect_groups(prefix: &str, value: &Value, groups: &mut Vec<String>) -> bool {
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
        has_leaves |= collect_groups(&path, child, groups);
    }
    if has_leaves && !prefix.is_empty() {
        groups.push(prefix.to_owned());
    }
    has_leaves
}

/// Canonical group prefixes with editable leaves, including nested groups.
pub fn describe_groups() -> Vec<String> {
    let mut groups = Vec::new();
    collect_groups("", schema_value(), &mut groups);
    groups.sort();
    groups
}

/// Top-level groups that contain editable leaves.
pub fn describe_root_groups() -> Vec<String> {
    let mut roots = Vec::new();
    if let Some(map) = schema_value().as_object() {
        for (key, child) in map {
            if !child.is_object() {
                // Root leaves carry no sub-groups.
                continue;
            }
            let mut nested = Vec::new();
            if collect_groups(key, child, &mut nested) {
                roots.push(key.clone());
            }
        }
    }
    roots.sort();
    roots
}

/// Workflow routes accepted by parameter validation.
pub fn supported_routes() -> &'static [&'static str] {
    &crate::params::validation::SUPPORTED_ROUTES
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
    let group = super::lookup(schema, prefix).ok_or_else(|| {
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

pub(super) fn schema_value() -> &'static Value {
    static SCHEMA: LazyLock<Value> = LazyLock::new(|| {
        let mut value =
            serde_json::to_value(RuntimeParams::default()).expect("RuntimeParams serializes");
        value.as_object_mut().unwrap().remove("workflow");
        value["io"].as_object_mut().unwrap().remove("scan_film");
        value["film_render"]["grain"]
            .as_object_mut()
            .unwrap()
            .remove("monochrome");
        value
    });
    &SCHEMA
}
