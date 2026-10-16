#![cfg(feature = "precision-f64")]
use serde_json::{Value, json};
use spektrafilm_benchmark::image_data;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
}
fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn failure(output: Output) -> Value {
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["execution_status"], "failed");
    value
}
fn code(value: &Value) -> &str {
    value["errors"][0]["code"].as_str().unwrap()
}
fn png(path: &Path, width: usize, height: usize) {
    image_data::write_png16_codes(path, &vec![[0x8000; 3]; width * height], width, height).unwrap();
}

fn grid_set(measurement: Option<Value>) -> Value {
    let mut set = json!({
        "id": "grid", "label": "网格集合", "grid": {
            "rows": 1, "columns": 2, "origin": [0, 0], "patch_size": [16, 8], "gap": [0, 0]
        }
    });
    if let Some(measurement) = measurement {
        set["measurement"] = measurement;
    }
    set
}

fn explicit_set(measurement: Option<Value>) -> Value {
    let mut set = json!({
        "id": "explicit", "label": "区域集合", "regions": [
            {"id": "r1", "label": "区域一", "geometry": {"c": [0, 0, 8, 8]}}
        ]
    });
    if let Some(measurement) = measurement {
        set["measurement"] = measurement;
    }
    set
}

/// One manifest over two 32×8 images plus an explicit-region set, so every
/// derivation failure has a declared alternative to report.
fn write_manifest(dir: &Path, sets: Value) -> std::path::PathBuf {
    png(&dir.join("a.png"), 32, 8);
    png(&dir.join("b.png"), 32, 8);
    let body = json!({
        "schema_version": 1,
        "title": "区域派生",
        "sets": sets,
        "images": [
            {"id": "a", "set": "grid", "path": "a.png", "label": "输入 a",
             "color_space": "sRGB", "cctf_decoding": false},
            {"id": "b", "set": "grid", "path": "b.png", "label": "输入 b",
             "color_space": "sRGB", "cctf_decoding": false},
            {"id": "c", "set": "explicit", "path": "a.png", "label": "输入 c",
             "color_space": "sRGB", "cctf_decoding": false}
        ],
        "default_pair": {"left": "a", "right": "b"}
    });
    let path = dir.join("review.json");
    fs::write(&path, serde_json::to_vec_pretty(&body).unwrap()).unwrap();
    path
}

fn manifest(dir: &Path, measurement: Option<Value>) -> std::path::PathBuf {
    write_manifest(dir, json!([grid_set(measurement), explicit_set(None)]))
}

#[test]
fn generate_report_renders_one_source_and_requires_exactly_one() {
    let dir = TempDir::new().unwrap();
    let review = manifest(dir.path(), None);
    let out = dir.path().join("report");
    let result = success(
        cli()
            .args(["report", "--manifest"])
            .arg(&review)
            .args(["--out"])
            .arg(&out)
            .output()
            .unwrap(),
    );
    assert_eq!(result["command"], "report");
    assert_eq!(result["source"], "manifest");
    assert_eq!(result["image_count"], 3);
    assert!(out.join("index.html").is_file());
    assert!(out.join("report.json").is_file());

    for args in [
        vec!["report", "--out", "report"],
        vec![
            "report",
            "--comparison",
            "comparison",
            "--manifest",
            "review.json",
            "--out",
            "report",
        ],
    ] {
        let output = cli().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn generate_regions_use_the_declared_measurement_intent() {
    let dir = TempDir::new().unwrap();
    let review = manifest(
        dir.path(),
        Some(json!({"sample": [8, 8], "role": "color", "measurements": ["color", "tone"]})),
    );
    let out = dir.path().join("regions.json");
    let result = success(
        cli()
            .args(["regions", "--manifest"])
            .arg(&review)
            .args(["--set", "grid", "--out"])
            .arg(&out)
            .output()
            .unwrap(),
    );
    assert_eq!(result["command"], "regions");
    assert_eq!(result["set"], "grid");
    assert_eq!(result["region_count"], 2);
    let regions: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(regions["schema_version"], json!(1));
    assert_eq!(
        regions["regions"],
        json!([
            {"id": "A1", "label": "A1", "role": "color",
             "geometry": {"x": 4, "y": 0, "width": 8, "height": 8, "space": "render"},
             "measurements": ["color", "tone"]},
            {"id": "B1", "label": "B1", "role": "color",
             "geometry": {"x": 20, "y": 0, "width": 8, "height": 8, "space": "render"},
             "measurements": ["color", "tone"]}
        ])
    );
    let validated = success(
        cli()
            .args(["validate", "--kind", "regions", "--file"])
            .arg(&out)
            .output()
            .unwrap(),
    );
    assert_eq!(validated["kind"], "regions");

    // Without a declared measurement the derivation keeps each full patch
    // window and defaults to role `other` with no families.
    let defaults = manifest(dir.path(), None);
    let full = dir.path().join("full.json");
    success(
        cli()
            .args(["regions", "--manifest"])
            .arg(&defaults)
            .args(["--set", "grid", "--out"])
            .arg(&full)
            .output()
            .unwrap(),
    );
    let regions: Value = serde_json::from_slice(&fs::read(&full).unwrap()).unwrap();
    assert_eq!(
        regions["regions"][0]["geometry"],
        json!({"x": 0, "y": 0, "width": 16, "height": 8, "space": "render"})
    );
    assert_eq!(
        regions["regions"][1]["geometry"],
        json!({"x": 16, "y": 0, "width": 16, "height": 8, "space": "render"})
    );
    assert_eq!(regions["regions"][0]["role"], "other");
    assert_eq!(regions["regions"][0]["measurements"], json!([]));
}

#[test]
fn generate_regions_reject_unknown_sets_regions_only_sets_and_destinations() {
    let dir = TempDir::new().unwrap();
    let review = manifest(dir.path(), None);
    let out = dir.path().join("regions.json");
    let derive = |set: &str, out: &Path| {
        cli()
            .args(["regions", "--manifest"])
            .arg(&review)
            .args(["--set", set, "--out"])
            .arg(out)
            .output()
            .unwrap()
    };

    let error = failure(derive("missing", &out));
    assert_eq!(code(&error), "unknown_set");
    assert!(
        error["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("available: grid, explicit")
    );
    let error = failure(derive("explicit", &out));
    assert_eq!(code(&error), "missing_grid");
    assert!(!out.exists());

    success(derive("grid", &out));
    let error = failure(derive("grid", &out));
    assert_eq!(code(&error), "destination_exists");
}

#[test]
fn review_validation_checks_declared_measurement_intent() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("regions.json");
    for (measurement, expected) in [
        (json!({"sample": [32, 8]}), "invalid_sample"),
        (json!({"sample": [8, 0]}), "invalid_bounds"),
        (json!({"role": "bogus"}), "invalid_role"),
        (
            json!({"measurements": ["color", "bogus"]}),
            "invalid_measurements",
        ),
        (
            json!({"measurements": ["color", "color"]}),
            "duplicate_value",
        ),
    ] {
        let review = manifest(dir.path(), Some(measurement.clone()));
        let validated = failure(
            cli()
                .args(["validate", "--kind", "review", "--file"])
                .arg(&review)
                .output()
                .unwrap(),
        );
        assert_eq!(code(&validated), expected, "{measurement}");
        let derived = failure(
            cli()
                .args(["regions", "--manifest"])
                .arg(&review)
                .args(["--set", "grid", "--out"])
                .arg(&out)
                .output()
                .unwrap(),
        );
        assert_eq!(code(&derived), expected, "{measurement}");
        assert!(!out.exists());
    }

    let explicit_measurement = write_manifest(
        dir.path(),
        json!([grid_set(None), explicit_set(Some(json!({"role": "other"})))]),
    );
    let validated = failure(
        cli()
            .args(["validate", "--kind", "review", "--file"])
            .arg(&explicit_measurement)
            .output()
            .unwrap(),
    );
    assert_eq!(code(&validated), "measurement_requires_grid");
}

#[test]
fn review_kind_rejects_the_v1_chart_manifest() {
    let dir = TempDir::new().unwrap();
    png(&dir.path().join("a.png"), 8, 8);
    let v1 = json!({
        "schema_version": 1,
        "title": "旧清单",
        "charts": [{"id": "c", "label": "色卡", "grid": {
            "rows": 1, "columns": 1, "origin": [0, 0], "patch_size": [8, 8], "gap": [0, 0]
        }}],
        "images": [{"id": "a", "chart": "c", "path": "a.png", "label": "输入 a",
                    "color_space": "sRGB", "cctf_decoding": false}],
        "default_pair": {"left": "a", "right": "a"}
    });
    let path = dir.path().join("v1.json");
    fs::write(&path, serde_json::to_vec_pretty(&v1).unwrap()).unwrap();
    let error = failure(
        cli()
            .args(["validate", "--kind", "review", "--file"])
            .arg(&path)
            .output()
            .unwrap(),
    );
    let code = code(&error);
    assert!(
        code == "missing_field" || code == "unknown_field",
        "unexpected code: {code}"
    );
}
