//! Retained-document fixtures for the lab suites.
//!
//! The canonical recorded run is committed by the crate that owns the run
//! format; a test copies it and mutates the copy.
use serde_json::{Value, json};
use spektrafilm_benchmark::common::{self, Evidence};
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

/// The canonical recorded run, hosted by the crate that owns the run format.
pub fn retained_run() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spektrafilm-benchmark/tests/fixtures/retained-run")
}

/// Copy the recorded run so a test can mutate it.
pub fn copy_fixture(destination: &Path) {
    fn copy(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    copy(&retained_run(), destination);
}
