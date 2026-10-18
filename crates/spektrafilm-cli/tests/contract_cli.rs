#![cfg(feature = "precision-f64")]
use serde_json::{Value, json};
use spektrafilm_benchmark::common;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn artifact(root: &Path, path: &str, role: &str, media_type: &str, value: &Value) -> Value {
    let path = root.join(path);
    write_json(&path, value);
    json!({
        "path": path.file_name().unwrap().to_string_lossy(),
        "sha256": common::sha256_file(&path).unwrap(),
        "role": role,
        "media_type": media_type
    })
}

fn identified(mut value: Value) -> Value {
    value.as_object_mut().unwrap().remove("id");
    value["id"] = json!(common::digest(&value).unwrap());
    value
}

fn failed_run(root: &Path) {
    fs::create_dir_all(root).unwrap();
    let input = root.join("input.png");
    fs::write(&input, b"retained only as suite declaration input").unwrap();
    let suite = json!({
        "schema_version": 1,
        "id": "contract-suite",
        "protocol": {
            "id": "appearance-v1",
            "pixels_per_degree": 60.0,
            "region_sampling_scales": [1.0],
            "grain": {
                "residual": {"method":"flat_mean", "high_pass_radius_px":4.0, "border_px":1},
                "spectrum": {"window":"hann"}
            },
            "hotspots": {"count": 0, "min_area_px": 1}
        },
        "cases": [{
            "id":"probe",
            "kind":"probe",
            "input": {
                "kind":"raster", "path":"input.png",
                "sha256":common::sha256_file(&input).unwrap(),
                "color_space":"sRGB", "cctf_decoding":true
            },
            "configuration":{}, "seeds":[0], "regions":[], "measurements":[]
        }]
    });
    let suite_file = artifact(root, "suite.json", "suite", "application/json", &suite);
    let target = json!({
        "schema_version":1,
        "id":"contract-target",
        "renderer":{"path":"historical-renderer","sha256":"a".repeat(64)},
        "data_dir":"/unavailable-retained-data",
        "backend":"cpu",
        "changes":{"set_files":[],"set":[]},
        "environment":{}
    });
    let target_file = artifact(root, "target.json", "target", "application/json", &target);
    let inventory = json!({"sha256":common::digest(&json!([])).unwrap(),"files":[]});
    let inventory_file = artifact(
        root,
        "data-inventory.json",
        "data_inventory",
        "application/json",
        &inventory,
    );
    let mut run = json!({
        "schema_version":1,
        "kind":"run",
        "execution_status":"failed",
        "suite": {"id":"contract-suite", "digest":common::digest(&suite).unwrap(), "path":"suite.json", "sha256":suite_file["sha256"]},
        "target": {"id":"contract-target", "digest":common::digest(&target).unwrap(), "path":"target.json", "sha256":target_file["sha256"]},
        "protocol":suite["protocol"],
        "selection":{"case_ids":["probe"],"full_suite":true},
        "dependencies":[inventory_file.clone()],
        "cases":[{"case_id":"probe","seed":0,"status":"failed","error":{"code":"renderer_failed","message":"fixture failure"}}],
        "artifacts":[suite_file,target_file,inventory_file],
        "environment":{}
    });
    run = identified(run);
    write_json(&root.join("run.json"), &run);
}

fn success_json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn path_traversal_case_id_is_rejected_before_run_output_creation() {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("input.png");
    fs::write(&input, b"synthetic input").unwrap();
    let renderer = temp.path().join("renderer");
    fs::write(&renderer, b"not invoked by invalid suite").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&renderer, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let data = temp.path().join("data");
    fs::create_dir(&data).unwrap();
    let suite = json!({
        "schema_version":1,
        "id":"path-suite",
        "protocol":{
            "id":"appearance-v1",
            "pixels_per_degree":60.0,
            "region_sampling_scales":[1.0],
            "grain":{"residual":{"method":"flat_mean","high_pass_radius_px":4.0,"border_px":1},"spectrum":{"window":"hann"}},
            "hotspots":{"count":0,"min_area_px":1}
        },
        "cases":[{"id":"../outside","kind":"probe","input":{"kind":"raster","path":"input.png","sha256":common::sha256_file(&input).unwrap(),"color_space":"sRGB","cctf_decoding":true},"configuration":{},"seeds":[0],"regions":[],"measurements":[]}]
    });
    let suite_path = temp.path().join("suite.json");
    write_json(&suite_path, &suite);
    let target = json!({"schema_version":1,"id":"path-target","renderer":{"path":"renderer","sha256":common::sha256_file(&renderer).unwrap()},"data_dir":"data","backend":"cpu","changes":{"set_files":[],"set":[]}});
    let target_path = temp.path().join("target.json");
    write_json(&target_path, &target);
    let output_dir = temp.path().join("run");

    let output = cli()
        .args(["run", "--suite"])
        .arg(&suite_path)
        .arg("--target")
        .arg(&target_path)
        .arg("--out")
        .arg(&output_dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["execution_status"], "failed");
    assert_eq!(result["errors"][0]["code"], "invalid_identifier");
    assert!(
        !output_dir.exists(),
        "invalid case ID must be rejected before output creation"
    );
    assert!(!temp.path().join("outside").exists());
}

#[test]
fn failed_render_remains_failed_through_compare_validation_and_report() {
    let temp = TempDir::new().unwrap();
    let baseline = temp.path().join("baseline");
    let candidate = temp.path().join("candidate");
    failed_run(&baseline);
    failed_run(&candidate);
    let comparison = temp.path().join("comparison");

    let output = cli()
        .args(["compare", "--baseline"])
        .arg(&baseline)
        .arg("--candidate")
        .arg(&candidate)
        .arg("--out")
        .arg(&comparison)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["execution_status"], "failed");
    assert_eq!(result["evaluation_status"], "not_evaluated");

    let manifest = comparison.join("comparison.json");
    let validated = cli()
        .args(["validate", "--kind", "comparison", "--file"])
        .arg(&manifest)
        .output()
        .unwrap();
    success_json(&validated);

    let report = temp.path().join("report");
    let reported = cli()
        .args(["report", "comparison", "--comparison"])
        .arg(&comparison)
        .arg("--out")
        .arg(&report)
        .output()
        .unwrap();
    success_json(&reported);
    assert!(report.join("index.html").is_file());

    let mut corrupted: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    corrupted["cases"][0]["measurements"] = json!([{"availability":"available"}]);
    let corrupted = identified(corrupted);
    write_json(&manifest, &corrupted);
    let rejected = cli()
        .args(["validate", "--kind", "comparison", "--file"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(result["errors"][0]["code"], "measurement_fields");
}

#[test]
fn altered_run_evidence_rejects_compare_before_output_publication() {
    let temp = TempDir::new().unwrap();
    let baseline = temp.path().join("baseline");
    let candidate = temp.path().join("candidate");
    failed_run(&baseline);
    failed_run(&candidate);
    fs::write(
        baseline.join("suite.json"),
        b"tampered after run publication",
    )
    .unwrap();
    let comparison = temp.path().join("comparison");

    let output = cli()
        .args(["compare", "--baseline"])
        .arg(&baseline)
        .arg("--candidate")
        .arg(&candidate)
        .arg("--out")
        .arg(&comparison)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["execution_status"], "failed");
    assert_eq!(result["errors"][0]["code"], "hash_mismatch");
    assert!(
        !comparison.exists(),
        "invalid run evidence must not publish a comparison directory"
    );
}
