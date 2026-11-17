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
                "--role",
                "color",
                "--out",
            ])
            .arg(&out)
            .output()
            .unwrap(),
    );
    assert_eq!(value["command"], "regions");
    assert_eq!(value["region_count"], 6);
    let regions: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(regions["id"], "chart");
    assert_eq!(regions["regions"][0]["id"], "A1");
    assert_eq!(regions["regions"][1]["id"], "B1");
    assert_eq!(regions["regions"][3]["id"], "A2");
    assert_eq!(
        regions["regions"][0]["geometry"],
        json!({"x": 10, "y": 20, "width": 16, "height": 12, "space": "render"})
    );
    let validated = result(
        cli()
            .args(["document", "--kind", "regions", "--check"])
            .arg(&out)
            .output()
            .unwrap(),
    );
    assert_eq!(validated["kind"], "regions");
}

#[test]
fn regions_grid_defaults_identity_and_origin_to_the_declaration_it_writes() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("chart.json");
    // The common grid is contiguous from the pixel origin and named after the
    // file that holds it. Ceremony that is always the same is not a decision.
    result(
        cli()
            .args([
                "regions",
                "--rows",
                "1",
                "--columns",
                "2",
                "--patch-size",
                "8x8",
            ])
            .args(["--role", "color", "--out"])
            .arg(&out)
            .output()
            .unwrap(),
    );
    let regions: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(regions["id"], "chart");
    assert_eq!(
        regions["regions"][1]["geometry"],
        json!({"x": 8, "y": 0, "width": 8, "height": 8, "space": "render"}),
        "one contiguous grid, written in the space this binary measures"
    );
}

/// A delivered raster has one coordinate space. A flag whose other six values
/// produce a document this binary refuses is a trap, not a choice.
#[test]
fn regions_grid_offers_no_coordinate_space_it_cannot_measure() {
    let dir = TempDir::new().unwrap();
    let output = cli()
        .args([
            "regions",
            "--rows",
            "1",
            "--columns",
            "1",
            "--patch-size",
            "8x8",
        ])
        .args(["--role", "color", "--space", "source_pixels", "--out"])
        .arg(dir.path().join("chart.json"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!dir.path().join("chart.json").exists());
}

#[test]
fn regions_grid_refuses_invalid_geometry_and_existing_destinations() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("regions.json");
    // Zero rows is a declaration the generator cannot expand, and a refused
    // expansion claims no destination.
    let invalid = cli()
        .args([
            "regions",
            "--id",
            "chart",
            "--rows",
            "0",
            "--columns",
            "1",
            "--origin",
            "0,0",
            "--patch-size",
            "4x4",
            "--gap",
            "0,0",
            "--role",
            "color",
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
                "--role",
                "color",
                "--out",
            ])
            .arg(&out)
            .output()
            .unwrap(),
    );
    let duplicate = cli()
        .args([
            "regions",
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
            "--role",
            "color",
            "--out",
        ])
        .arg(&out)
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    let duplicate_value: Value = serde_json::from_slice(&duplicate.stdout).unwrap();
    assert_eq!(duplicate_value["errors"][0]["code"], "destination_exists");
}

#[test]
fn review_kind_and_removed_manifest_inputs_are_rejected() {
    let review = TempDir::new().unwrap();
    let file = review.path().join("review.json");
    fs::write(&file, b"{}").unwrap();
    // Both endpoints read one registry, so an unknown kind reaches the caller
    // the same way from either: one code, one exit status, one record shape.
    // A kind outside the vocabulary is refused before it is read, from the one
    // endpoint that owns the vocabulary.
    {
        let arguments = ["document", "--kind", "review"];
        let mut command = cli();
        command.args(arguments);
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["errors"][0]["code"], "unknown_document_kind");
        assert_eq!(value["endpoint"], arguments[0]);
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
        let output = cli().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        // A usage error reaches the caller in the record shape too, so a
        // pipeline handles it like every other refusal.
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["errors"][0]["code"], "argument_syntax", "{args:?}");
    }
}
