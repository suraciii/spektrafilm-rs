//! The committed recorded run is a valid record of its format.
//!
//! The record is the canonical example later revisions read: a schema change
//! that stops accepting it is a compatibility break, not a fixture fix.

use serde_json::Value;
use spektrafilm_benchmark::{common, documents};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn retained_run() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/retained-run")
}

fn copy_bundle(destination: &Path) {
    let source = retained_run().join("cases/probe/seed-0/stage-evidence");
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(&source).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
    }
}

#[test]
fn committed_recorded_run_stays_valid() {
    let root = retained_run();
    let run: Value = serde_json::from_slice(&fs::read(root.join("run.json")).unwrap()).unwrap();
    documents::validate_generated_document("run", &run, &root).unwrap();
    documents::validate_stage_bundle(&root.join("cases/probe/seed-0/stage-evidence")).unwrap();
}

#[test]
fn stage_bundle_rejects_a_nonfinite_sample_and_a_hash_disagreement() {
    let temp = tempfile::TempDir::new().unwrap();
    let bundle = temp.path().join("stage-evidence");
    copy_bundle(&bundle);
    documents::validate_stage_bundle(&bundle).unwrap();
    // A buffer with a non-finite sample fails even when every hash matches.
    let buffer = bundle.join("rgb_in.f64");
    fs::write(
        &buffer,
        (0..48)
            .flat_map(|index| {
                if index == 0 {
                    f64::NAN.to_le_bytes()
                } else {
                    0.25f64.to_le_bytes()
                }
            })
            .collect::<Vec<u8>>(),
    )
    .unwrap();
    let mut document: Value =
        serde_json::from_slice(&fs::read(bundle.join("stage-evidence.json")).unwrap()).unwrap();
    document["stages"][0]["sha256"] = serde_json::json!(common::sha256_file(&buffer).unwrap());
    common::dump(&bundle.join("stage-evidence.json"), &document).unwrap();
    assert_eq!(
        documents::validate_stage_bundle(&bundle).unwrap_err().code,
        "stage_evidence"
    );
    // A hash disagreement fails without any sample scan.
    document["stages"][0]["sha256"] = serde_json::json!("b".repeat(64));
    common::dump(&bundle.join("stage-evidence.json"), &document).unwrap();
    assert_eq!(
        documents::validate_stage_bundle(&bundle).unwrap_err().code,
        "stage_evidence"
    );
}
