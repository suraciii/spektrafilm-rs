use serde_json::{Value, json};
use spektrafilm_benchmark::image_data;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-bench"))
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
/// A grid set over two 32×8 images plus an explicit-region set, so every
/// derivation failure has a declared alternative to report.
fn manifest(dir: &Path) -> std::path::PathBuf {
    png(&dir.join("a.png"), 32, 8);
    png(&dir.join("b.png"), 32, 8);
    let body = json!({
        "schema_version": 1,
        "title": "区域派生",
        "sets": [
            {"id": "grid", "label": "网格集合", "grid": {
                "rows": 1, "columns": 2, "origin": [0, 0], "patch_size": [16, 8], "gap": [0, 0]
            }},
            {"id": "explicit", "label": "区域集合", "regions": [
                {"id": "r1", "label": "区域一", "geometry": {"c": [0, 0, 8, 8]}}
            ]}
        ],
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

#[test]
fn regions_derive_review_patch_windows_and_validate_as_the_regions_kind() {
    let dir = TempDir::new().unwrap();
    let review = manifest(dir.path());
    let out = dir.path().join("regions.json");
    let result = success(
        cli()
            .args(["regions", "--review"])
            .arg(&review)
            .args([
                "--set",
                "grid",
                "--sample",
                "8x8",
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
    assert_eq!(result["command"], "regions");
    assert_eq!(result["execution_status"], "completed");
    assert_eq!(result["set"], "grid");
    assert_eq!(result["region_count"], 2);
    let regions: Value = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(
        regions,
        json!([
            {"id": "A1", "label": "A1", "role": "color",
             "geometry": [4, 0, 8, 8], "measurements": ["color", "tone"]},
            {"id": "B1", "label": "B1", "role": "color",
             "geometry": [20, 0, 8, 8], "measurements": ["color", "tone"]}
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

    // Without a sample each region keeps the full patch window, and the
    // defaults stay role `other` with no measurements.
    let full = dir.path().join("full.json");
    success(
        cli()
            .args(["regions", "--review"])
            .arg(&review)
            .args(["--set", "grid", "--out"])
            .arg(&full)
            .output()
            .unwrap(),
    );
    let regions: Value = serde_json::from_slice(&fs::read(&full).unwrap()).unwrap();
    assert_eq!(regions[0]["geometry"], json!([0, 0, 16, 8]));
    assert_eq!(regions[1]["geometry"], json!([16, 0, 16, 8]));
    assert_eq!(regions[0]["role"], "other");
    assert_eq!(regions[0]["measurements"], json!([]));
}

#[test]
fn regions_reject_unknown_sets_regions_only_sets_samples_and_destinations() {
    let dir = TempDir::new().unwrap();
    let review = manifest(dir.path());
    let out = dir.path().join("regions.json");
    let derive = |set: &str, extra: &[&str], out: &Path| {
        cli()
            .args(["regions", "--review"])
            .arg(&review)
            .args(["--set", set])
            .args(extra)
            .args(["--out"])
            .arg(out)
            .output()
            .unwrap()
    };

    let error = failure(derive("missing", &[], &out));
    assert_eq!(code(&error), "unknown_set");
    assert!(
        error["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("available: grid, explicit")
    );
    let error = failure(derive("explicit", &[], &out));
    assert_eq!(code(&error), "missing_grid");
    let error = failure(derive("grid", &["--sample", "32x8"], &out));
    assert_eq!(code(&error), "invalid_sample");
    let error = failure(derive("grid", &["--sample", "8x0"], &out));
    assert_eq!(code(&error), "invalid_sample");
    let error = failure(derive("grid", &["--sample", "8"], &out));
    assert_eq!(code(&error), "invalid_sample");
    let error = failure(derive("grid", &["--role", "bogus"], &out));
    assert_eq!(code(&error), "invalid_role");
    let error = failure(derive("grid", &["--measurements", "color,bogus"], &out));
    assert_eq!(code(&error), "invalid_measurements");
    let error = failure(derive("grid", &["--measurements", "color,,tone"], &out));
    assert_eq!(code(&error), "invalid_measurements");
    assert!(!out.exists());

    success(derive("grid", &[], &out));
    let error = failure(derive("grid", &[], &out));
    assert_eq!(code(&error), "destination_exists");
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
