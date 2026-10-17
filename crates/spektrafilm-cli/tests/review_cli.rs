#![cfg(feature = "precision-f64")]

use serde_json::{Value, json};
use std::{fs, process::Command};
use tempfile::TempDir;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
}

fn result(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn regions_grid_writes_an_explicit_regions_document() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("regions.json");
    let value = result(
        cli()
            .args([
                "regions",
                "grid",
                "--id",
                "chart",
                "--rows",
                "2",
                "--columns",
                "3",
                "--origin",
                "10,20",
                "--patch-size",
                "16x12",
                "--gap",
                "2,4",
                "--sample",
                "8x6",
                "--role",
                "color",
                "--measurements",
                "color,tone",
                "--out",
            ])
            .arg(&out)
            .output()
            .unwrap(),
    );
    assert_eq!(value["command"], "regions");
    assert_eq!(value["operation"], "grid");
    assert_eq!(value["region_count"], 6);
    let regions: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(regions["id"], "chart");
    assert_eq!(regions["regions"][0]["id"], "A1");
    assert_eq!(regions["regions"][1]["id"], "B1");
    assert_eq!(regions["regions"][3]["id"], "A2");
    assert_eq!(
        regions["regions"][0]["geometry"],
        json!({"x": 14, "y": 23, "width": 8, "height": 6, "space": "source"})
    );
    let validated = result(
        cli()
            .args(["validate", "--kind", "regions", "--file"])
            .arg(&out)
            .output()
            .unwrap(),
    );
    assert_eq!(validated["kind"], "regions");
}

#[test]
fn regions_grid_refuses_invalid_geometry_and_existing_destinations() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("regions.json");
    let invalid = cli()
        .args([
            "regions",
            "grid",
            "--id",
            "chart",
            "--rows",
            "1",
            "--columns",
            "1",
            "--origin",
            "0,0",
            "--patch-size",
            "4x4",
            "--gap",
            "0,0",
            "--sample",
            "5x4",
            "--out",
        ])
        .arg(&out)
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(!out.exists());

    result(
        cli()
            .args([
                "regions",
                "grid",
                "--id",
                "chart",
                "--rows",
                "1",
                "--columns",
                "1",
                "--origin",
                "0,0",
                "--patch-size",
                "4x4",
                "--gap",
                "0,0",
                "--out",
            ])
            .arg(&out)
            .output()
            .unwrap(),
    );
    let duplicate = cli()
        .args([
            "regions",
            "grid",
            "--id",
            "chart",
            "--rows",
            "1",
            "--columns",
            "1",
            "--origin",
            "0,0",
            "--patch-size",
            "4x4",
            "--gap",
            "0,0",
            "--out",
        ])
        .arg(&out)
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stdout).contains("destination_exists"));
}

#[test]
fn review_kind_and_removed_manifest_inputs_are_rejected() {
    let review = TempDir::new().unwrap();
    let file = review.path().join("review.json");
    fs::write(&file, b"{}").unwrap();
    for args in [
        vec!["schema", "--kind", "review"],
        vec![
            "validate",
            "--kind",
            "review",
            "--file",
            file.to_str().unwrap(),
        ],
    ] {
        let output = cli().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["errors"][0]["code"], "unsupported_document_kind");
    }
    for args in [
        vec!["report", "--manifest", "review.json", "--out", "report"],
        vec![
            "regions",
            "--manifest",
            "review.json",
            "--out",
            "regions.json",
        ],
    ] {
        let output = cli().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn report_comparison_is_the_only_report_input_form() {
    let output = cli()
        .args([
            "report",
            "comparison",
            "--comparison",
            "missing",
            "--out",
            "report",
        ])
        .output()
        .unwrap();
    assert_ne!(output.status.code(), Some(2));
    assert!(!output.stdout.is_empty());
}
