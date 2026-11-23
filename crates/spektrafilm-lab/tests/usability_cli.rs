#![cfg(feature = "precision-f64")]

use serde_json::{Value, json};
use spektrafilm_lab::common;
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

fn measurement(case: &str, region: &str, metric: &str, available: bool) -> Value {
    if available {
        json!({"case_id":case,"region_id":region,"metric":metric,"domain":"scalar","units":"1",
            "metric_version":"1","estimator":"mean","sample_count":4,"availability":"available","value":0.125,
            "artifact_refs":["evidence.json"]})
    } else {
        json!({"case_id":case,"region_id":region,"metric":metric,"domain":"scalar","units":"1",
            "metric_version":"1","estimator":"median","availability":"unavailable","reason":"source_color_unavailable",
            "artifact_refs":["evidence.json"]})
    }
}

/// An image comparison: two measured cases and one failed case.
fn comparison(root: &Path) {
    fs::create_dir(root).unwrap();
    common::dump(
        &root.join("evidence.json"),
        &json!({"retained": "measurement evidence"}),
    )
    .unwrap();
    let mut cases = Vec::new();
    for case in ["probe", "scene"] {
        let mut measured = vec![
            measurement(case, "full_image", "numeric.rmse", true),
            measurement(case, "patch", "color.unavailable", false),
        ];
        for index in 0..24 {
            measured.push(measurement(
                case,
                "full_image",
                &format!("numeric.retained-{index}"),
                true,
            ));
        }
        cases.push(
            json!({"case_id":case,"status":"completed","measurements":measured,
            "review":{},"regions":[{"id":"patch"}],"region_readouts":[]}),
        );
    }
    cases.push(
        json!({"case_id":"failed","status":"failed","reason":"failed_render",
        "error":{"code":"renderer_failed","message":"retained failure"},
        "measurements":[],"review":{}}),
    );
    let finding = json!({"id":"failure","case_id":"failed","region_id":"full_image",
        "metric":"execution","decision":"failed","evidence":["evidence.json"]});
    let mut value = json!({
        "schema_version":1,"kind":"comparison","execution_status":"failed",
        "evaluation_status":"not_evaluated","protocol":{},"protocol_digest":"0".repeat(64),
        "map_scale":1,
        "selection":{"case_ids":["probe","scene","failed"]},"condition_differences":[],
        "baseline":{"kind":"images","id":"baseline"},"candidate":{"kind":"images","id":"candidate"},
        "cases":cases,"findings":[finding],
        "summary":{"selected_cases":3,"completed_cases":2,"failed_cases":1},
        "artifacts":[{"path":"evidence.json",
            "sha256":common::sha256_file(&root.join("evidence.json")).unwrap(),
            "role":"measurements","media_type":"application/json"}]});
    value["id"] = json!(common::digest(&value).unwrap());
    common::dump(&root.join("comparison.json"), &value).unwrap();
}

#[test]
fn inspect_exposes_measurements_and_failures() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("comparison");
    comparison(&root);
    let view = success(
        cli()
            .arg("inspect")
            .arg("--comparison")
            .arg(&root)
            .args(["--metric", "numeric.rmse", "--metric", "color.unavailable"])
            .output()
            .unwrap(),
    );
    let rows = view["measurements"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["value"], 0.125);
    assert_eq!(rows[1]["reason"], "source_color_unavailable");
    assert!(rows[1].get("value").is_none());
    assert_eq!(
        view["failed_results"][0]["error"]["code"],
        "renderer_failed"
    );
    assert_eq!(
        view["failed_results"][0]["artifact_refs"],
        json!(["evidence.json"])
    );
    assert_eq!(view["comparison_id"], view["comparison_id"]);
}

#[test]
fn inspect_filters_retained_evidence_offline_without_mutation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("comparison");
    comparison(&root);
    let before = fs::read(root.join("comparison.json")).unwrap();
    let evidence_before = fs::read(root.join("evidence.json")).unwrap();
    let all = success(
        cli()
            .arg("inspect")
            .arg("--comparison")
            .arg(&root)
            .output()
            .unwrap(),
    );
    let filtered = success(
        cli()
            .arg("inspect")
            .arg("--comparison")
            .arg(&root)
            .args([
                "--case",
                "probe",
                "--case",
                "scene",
                "--region",
                "full_image",
                "--metric",
                "numeric.rmse",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(filtered["measurements"].as_array().unwrap().len(), 2);
    for row in filtered["measurements"].as_array().unwrap() {
        assert_eq!(row["metric"], "numeric.rmse");
        let path = Path::new(filtered["artifact_base"].as_str().unwrap())
            .join(row["artifact_refs"][0].as_str().unwrap());
        assert!(path.is_file());
    }
    assert!(serde_json::to_vec(&filtered).unwrap().len() < serde_json::to_vec(&all).unwrap().len());
    assert_eq!(fs::read(root.join("comparison.json")).unwrap(), before);
    assert_eq!(
        fs::read(root.join("evidence.json")).unwrap(),
        evidence_before
    );
    let unavailable = success(
        cli()
            .arg("inspect")
            .arg("--comparison")
            .arg(&root)
            .args(["--case", "probe", "--region", "patch"])
            .output()
            .unwrap(),
    );
    assert_eq!(unavailable["measurements"].as_array().unwrap().len(), 1);
    assert_eq!(
        unavailable["measurements"][0]["reason"],
        "source_color_unavailable"
    );
    assert!(unavailable["measurements"][0].get("value").is_none());
}

#[test]
fn inspect_rejects_unknown_and_empty_intersections_with_choices() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("comparison");
    comparison(&root);
    for (flag, expected) in [
        ("--case", "probe"),
        ("--region", "patch"),
        ("--metric", "numeric.rmse"),
    ] {
        let output = cli()
            .arg("inspect")
            .arg("--comparison")
            .arg(&root)
            .args([flag, "unknown"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(result.to_string().contains("unknown_selection"));
        assert!(result.to_string().contains(expected));
    }
    let output = cli()
        .arg("inspect")
        .arg("--comparison")
        .arg(&root)
        .args([
            "--case",
            "probe",
            "--region",
            "patch",
            "--metric",
            "numeric.rmse",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("empty_selection"));
}
