//! Suite identity is validated before the renderer runs or output is created.

#![cfg(feature = "precision-f64")]
use serde_json::{Value, json};
use spektrafilm_benchmark::common;
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-f64"))
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
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
        .args(["run", "suite", "--suite"])
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
