//! `sample --from-run` reads the exact stage a retained run kept.

#![cfg(feature = "precision-f64")]
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

/// The recorded run, committed by the crate that owns the run format. The
/// reader does not mutate it, so the test reads it where it lives.
fn retained_run() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spektrafilm-benchmark/tests/fixtures/retained-run")
}

#[test]
fn sample_reads_the_exact_stage_from_a_retained_run_directory_or_manifest() {
    let temp = TempDir::new().unwrap();
    let root = retained_run();
    let regions = temp.path().join("regions.json");
    fs::write(
        &regions,
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "id": "one-pixel",
            "regions": [{
                "id": "p00",
                "role": "other",
                "measurements": [],
                "geometry": {"x": 0, "y": 0, "width": 1, "height": 1, "space": "source"}
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    for (run_argument, output_name) in [
        (root.to_path_buf(), "sample-directory"),
        (root.join("run.json"), "sample-manifest"),
    ] {
        let output = temp.path().join(output_name);
        let result = Command::new(env!("CARGO_BIN_EXE_spektrafilm-experiment"))
            .args(["sample", "--from-run"])
            .arg(&run_argument)
            .args([
                "--case",
                "probe",
                "--seed",
                "0",
                "--stage",
                "rgb_in",
                "--regions",
            ])
            .arg(&regions)
            .args(["--out"])
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let sample: Value =
            serde_json::from_slice(&fs::read(output.join("run.json")).unwrap()).unwrap();
        assert_eq!(sample["source"]["kind"], "run");
        assert_eq!(sample["source"]["stage"], "rgb_in");
        assert_eq!(sample["source"]["coordinate_space"], "source_pixels");
        assert_eq!(sample["stages"][0]["id"], "rgb_in");
        assert_eq!(sample["stages"][0]["domain"], "input_rgb");
        assert_eq!(sample["stages"][0]["units"], "relative");
        assert_eq!(sample["stages"][0]["channel_order"], "RGB");
        assert_eq!(sample["summaries"][0]["count"], 1);
    }
}
