//! Retained-document fixtures for the lab suites.
use serde_json::{Value, json};
use spektrafilm_lab::common::{self, Evidence};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn write(path: &Path, value: &Value) {
    common::dump(path, value).unwrap();
}

/// Re-derive the content-addressed `id` after a caller mutates a record.
pub fn identify(mut value: Value) -> Value {
    value.as_object_mut().unwrap().remove("id");
    value["id"] = json!(common::digest(&value).unwrap());
    value
}

pub fn save_manifest(root: &Path, name: &str, value: Value) {
    write(&root.join(name), &identify(value));
}

pub fn record(evidence: &mut Evidence, name: &str, role: &str, value: Value) -> Value {
    let path = evidence.root.join(name);
    write(&path, &value);
    evidence
        .add(&path, role, "application/json", json!({}))
        .unwrap()
}
