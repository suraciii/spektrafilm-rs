//! `validate --kind run` reads a retained run record: it accepts a complete
//! record, names the checks it did not perform, and rejects an incomplete or
//! altered one.
#![cfg(feature = "precision-f64")]
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

mod support;
use support::fixtures::{self, save_manifest};

fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn validate_run(root: &Path) -> (Value, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
        .args(["validate", "--kind", "run", "--file"])
        .arg(root.join("run.json"))
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    (value, output.status.code().unwrap())
}

/// A recorded run, copied so the test can mutate it. `capture` selects whether
/// the retained stage bundle stays in the record.
fn complete_run(root: &Path, capture: bool) {
    fixtures::copy_fixture(root);
    if capture {
        return;
    }
    let mut run = read(&root.join("run.json"));
    run["cases"][0]
        .as_object_mut()
        .unwrap()
        .remove("stage_evidence");
    let captured = |artifact: &Value| {
        !matches!(
            artifact["role"].as_str(),
            Some("stage_evidence_buffer" | "stage_evidence_manifest")
        )
    };
    run["artifacts"] = Value::Array(
        run["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|artifact| captured(artifact))
            .cloned()
            .collect(),
    );
    save_manifest(root, "run.json", run);
}

#[test]
fn complete_retained_run_with_and_without_capture_validates() {
    for capture in [false, true] {
        let temp = tempfile::TempDir::new().unwrap();
        complete_run(temp.path(), capture);
        let (result, code) = validate_run(temp.path());
        assert_eq!(code, 0, "{result}");
        assert_eq!(result["execution_status"], "completed");
    }
}

#[test]
fn validating_a_retained_run_lists_the_checks_it_did_not_perform() {
    let temp = tempfile::TempDir::new().unwrap();
    complete_run(temp.path(), false);
    let (result, code) = validate_run(temp.path());
    assert_eq!(code, 0, "{result}");
    // The record was produced by execution. Validating its shape decides
    // nothing about whether its images decode, its buffers match the device
    // that produced them, or its measurements hold on re-execution, so every
    // such check must be named rather than left to implication.
    let unchecked = result["unchecked"].as_array().unwrap();
    for check in [
        "image_decoding",
        "rendering",
        "compute_device",
        "measurement",
    ] {
        assert!(
            unchecked.iter().any(|v| v == check),
            "{check} must be listed as unchecked: {result}"
        );
    }
}

#[test]
fn missing_source_evidence_fails_semantically_with_recomputed_identity() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path();
    complete_run(root, false);
    let mut run = read(&root.join("run.json"));
    run["cases"][0]
        .as_object_mut()
        .unwrap()
        .remove("source_raw");
    run["cases"][0]
        .as_object_mut()
        .unwrap()
        .remove("source_raw_metadata");
    // The reproduction recomputes the manifest digest, so the failure below is
    // semantic completeness and not identity protection.
    save_manifest(root, "run.json", run);
    let (result, code) = validate_run(root);
    assert_eq!(code, 1, "{result}");
    assert_eq!(result["execution_status"], "failed");
    let error = result["errors"][0]["code"].as_str().unwrap();
    assert!(
        ["source_evidence", "missing_field"].contains(&error),
        "{result}"
    );
}

#[test]
fn manifest_digest_still_rejects_changed_contents() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path();
    complete_run(root, false);
    let mut run = read(&root.join("run.json"));
    run["cases"][0]["dimensions"] = json!([5, 5]);
    fixtures::write(&root.join("run.json"), &run);
    let (result, code) = validate_run(root);
    assert_eq!(code, 1, "{result}");
    assert_eq!(result["errors"][0]["code"], "manifest_identity");
}

#[test]
fn missing_stage_bundle_manifest_cannot_pass_silently() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path();
    complete_run(root, true);
    fs::remove_file(root.join("cases/probe/seed-0/stage-evidence/stage-evidence.json")).unwrap();
    let (result, code) = validate_run(root);
    assert_eq!(code, 1, "{result}");
    assert_eq!(result["errors"][0]["code"], "missing_artifact");
}
