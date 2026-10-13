use serde_json::{Value, json};
use spektrafilm_benchmark::{common, documents};
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
fn measurement(
    case: &str,
    region: &str,
    metric: &str,
    population: &[u64],
    available: bool,
) -> Value {
    let mut row = json!({"case_id":case,"region_id":region,"metric":metric,
        "domain":"linear_rgb","units":"linear RGB","estimator":"retained estimator",
        "sample_count":16,"availability":if available {"available"} else {"unavailable"},
        "seed_population":population,"artifact_refs":["evidence.json"]});
    if available {
        row["value"] = json!(0.125);
    } else {
        row["reason"] = json!("source_color_unavailable");
    }
    row
}
fn comparison(root: &Path, single: bool) {
    fs::create_dir(root).unwrap();
    common::dump(
        &root.join("evidence.json"),
        &json!({"retained":"measurement evidence"}),
    )
    .unwrap();
    let mut cases = Vec::new();
    for case in ["probe", "scene"] {
        for seed in [0, 1] {
            let mut measured = vec![
                measurement(case, "full_image", "numeric.rmse", &[seed], true),
                measurement(case, "patch", "color.unavailable", &[seed], false),
            ];
            // Large retained rows make compact-view regressions observable without rendering.
            for index in 0..24 {
                measured.push(measurement(
                    case,
                    "full_image",
                    &format!("numeric.retained-{index}"),
                    &[seed],
                    true,
                ));
            }
            let row = if single {
                for value in &mut measured {
                    value["pair_id"] = json!("input");
                }
                json!({"case_id":case,"seed":seed,"status":"completed","measurements":measured,"pairs":[{
                    "id":"input","left_role":"input","right_role":"simulation","pixel_aligned":true,
                    "status":"completed","measurements":measured,"review":{},"regions":[],"automatic_regions":[]}]})
            } else {
                json!({"case_id":case,"seed":seed,"status":"completed","measurements":measured,"review":{},"regions":[]})
            };
            cases.push(row);
        }
    }
    let failure = if single {
        json!({"case_id":"failed","seed":0,"status":"failed","pairs":[{
            "id":"input","left_role":"input","right_role":"simulation","pixel_aligned":false,
            "status":"failed","reason":"failed_render","measurements":[],"review":{},"regions":[],"automatic_regions":[]}]})
    } else {
        json!({"case_id":"failed","seed":0,"status":"failed","reason":"failed_render","error":{"code":"renderer_failed","message":"retained failure"},"measurements":[],"review":{}})
    };
    cases.push(failure);
    let mut finding = json!({"id":"failure","case_id":"failed","seed":0,"region_id":"full_image","metric":"execution","decision":"failed","evidence":["evidence.json"]});
    if single {
        finding["pair_id"] = json!("input");
    }
    let mut value = json!({"schema_version":1,"execution_status":"failed","evaluation_status":"not_evaluated",
        "protocol":{},"selection":{"case_ids":["probe","scene","failed"]},"condition_differences":[],
        "cases":cases,"findings":[finding],"summary":{"selected_cases":3,"completed_results":4,"failed_results":1},
        "artifacts":[{"path":"evidence.json","sha256":common::sha256_file(&root.join("evidence.json")).unwrap(),"role":"measurements","media_type":"application/json"}]});
    if single {
        value["mode"] = json!("single_run");
        value["default_pair"] = json!("input");
        value["run"] = json!({"id":"removed-run","manifest_sha256":"a".repeat(64)});
    } else {
        value["baseline"] = json!({"id":"removed-baseline"});
        value["candidate"] = json!({"id":"removed-candidate"});
        value["seed_measurements"] = json!([measurement(
            "probe",
            "full_image",
            "numeric.rmse",
            &[0, 1],
            true
        )]);
    }
    value["id"] = json!(common::digest(&value).unwrap());
    common::dump(&root.join("comparison.json"), &value).unwrap();
}

#[test]
fn inspect_filters_retained_evidence_offline_without_mutation() {
    let temp = TempDir::new().unwrap();
    let original = temp.path().join("original-run");
    fs::create_dir(&original).unwrap();
    fs::write(original.join("obsolete"), b"original input").unwrap();
    let root = temp.path().join("comparison");
    comparison(&root, false);
    fs::remove_dir_all(original).unwrap();
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
    assert_eq!(all["execution_status"], "completed");
    assert_eq!(all["comparison_execution_status"], "failed");
    assert_eq!(all["failed_results"].as_array().unwrap().len(), 1);
    assert_eq!(all["failed_results"][0]["error"]["code"], "renderer_failed");
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
    assert_eq!(filtered["measurements"].as_array().unwrap().len(), 4);
    assert_eq!(filtered["seed_measurements"].as_array().unwrap().len(), 1);
    assert_eq!(
        filtered["seed_measurements"][0]["seed_population"],
        json!([0, 1])
    );
    assert!(filtered["seed_measurements"][0].get("seed").is_none());
    for row in filtered["measurements"].as_array().unwrap() {
        assert_eq!(row["seed_population"], json!([row["seed"]]));
        assert_eq!(row["metric"], "numeric.rmse");
        let path = Path::new(filtered["artifact_base"].as_str().unwrap())
            .join(row["artifact_refs"][0].as_str().unwrap());
        assert!(path.is_file());
    }
    assert!(
        serde_json::to_vec(&filtered).unwrap().len() * 3 < serde_json::to_vec(&all).unwrap().len()
    );
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
            .args([
                "--case",
                "probe",
                "--region",
                "patch",
                "--metric",
                "color.unavailable",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(unavailable["measurements"].as_array().unwrap().len(), 2);
    assert_eq!(
        unavailable["measurements"][0]["reason"],
        "source_color_unavailable"
    );
    assert!(unavailable["measurements"][0].get("value").is_none());
    let repeated = success(
        cli()
            .arg("inspect")
            .arg("--comparison")
            .arg(&root)
            .args([
                "--case",
                "probe",
                "--region",
                "patch",
                "--region",
                "full_image",
                "--metric",
                "numeric.rmse",
                "--metric",
                "color.unavailable",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(repeated["measurements"].as_array().unwrap().len(), 4);
}

#[test]
fn inspect_rejects_unknown_and_empty_intersections_with_choices() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("comparison");
    comparison(&root, false);
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

#[test]
fn inspect_preserves_single_run_pair_identity_and_failures() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("comparison");
    comparison(&root, true);
    let view = success(
        cli()
            .arg("inspect")
            .arg("--comparison")
            .arg(&root)
            .args(["--metric", "numeric.rmse"])
            .output()
            .unwrap(),
    );
    assert!(view["seed_measurements"].as_array().unwrap().is_empty());
    assert_eq!(view["measurements"].as_array().unwrap().len(), 4);
    assert!(
        view["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["pair_id"] == "input")
    );
    assert_eq!(view["failed_results"][0]["pair_id"], "input");
    assert_eq!(
        view["failed_results"][0]["artifact_refs"],
        json!(["evidence.json"])
    );
}

#[test]
fn target_pins_dependencies_and_refuses_existing_destinations() {
    let temp = TempDir::new().unwrap();
    let renderer = temp.path().join("renderer");
    fs::write(&renderer, b"pinned executable bytes").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&renderer, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let data = temp.path().join("data");
    fs::create_dir(&data).unwrap();
    let out = temp.path().join("my-renderer.json");
    let result = success(
        cli()
            .arg("target")
            .arg("--renderer")
            .arg(&renderer)
            .arg("--data-dir")
            .arg(&data)
            .args(["--backend", "gpu"])
            .arg("--out")
            .arg(&out)
            .output()
            .unwrap(),
    );
    assert_eq!(result["target_id"], "my-renderer");
    let target = documents::load_target(&out).unwrap();
    assert_eq!(
        target["renderer"]["path"],
        json!(renderer.canonicalize().unwrap())
    );
    assert_eq!(
        target["renderer"]["sha256"],
        common::sha256_file(&renderer).unwrap()
    );
    assert_eq!(target["data_dir"], json!(data.canonicalize().unwrap()));
    assert_eq!(target["changes"], json!({"set_files":[],"set":[]}));
    let before = fs::read(&out).unwrap();
    let repeat = cli()
        .arg("target")
        .arg("--renderer")
        .arg(&renderer)
        .arg("--data-dir")
        .arg(&data)
        .args(["--backend", "cpu"])
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(repeat.status.code(), Some(1));
    let refused: Value = serde_json::from_slice(&repeat.stdout).unwrap();
    assert_eq!(refused["errors"][0]["code"], "destination_exists");
    assert_eq!(fs::read(&out).unwrap(), before);
    #[cfg(unix)]
    {
        let link = temp.path().join("dangling.json");
        std::os::unix::fs::symlink(temp.path().join("missing"), &link).unwrap();
        let output = cli()
            .arg("target")
            .arg("--renderer")
            .arg(&renderer)
            .arg("--data-dir")
            .arg(&data)
            .args(["--backend", "cpu"])
            .arg("--out")
            .arg(&link)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let refused: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(refused["errors"][0]["code"], "destination_exists");
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    }
    fs::write(&renderer, b"changed executable bytes").unwrap();
    let invalid = temp.path().join("bad target.json");
    let rejected = cli()
        .arg("target")
        .arg("--renderer")
        .arg(&renderer)
        .arg("--data-dir")
        .arg(&data)
        .args(["--backend", "cpu"])
        .arg("--out")
        .arg(&invalid)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(!invalid.exists());
    assert_eq!(
        documents::load_target(&out).unwrap_err().code,
        "hash_mismatch"
    );
}

#[test]
fn inspect_legacy_failure_evidence_belongs_to_exact_seed() {
    for single in [false, true] {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("comparison");
        comparison(&root, single);
        let mut value = documents::load_json_document(&root.join("comparison.json")).unwrap();
        value["cases"] = json!([]);
        value["findings"] = json!([]);
        for seed in [0, 1] {
            let path = format!("cases/failed/seed-{seed}/render-failure.json");
            let destination = root.join(&path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            common::dump(
                &destination,
                &json!({"case_id":"failed","seed":seed,"status":"failed"}),
            )
            .unwrap();
            // Version comparisons historically retained no owner metadata on this artifact.
            value["artifacts"].as_array_mut().unwrap().push(json!({"path":path,
                "sha256":common::sha256_file(&destination).unwrap(),"role":"render_failure","media_type":"application/json"}));
            let case = if single {
                json!({"case_id":"failed","seed":seed,"status":"failed","pairs":[{
                    "id":"input","left_role":"input","right_role":"simulation","pixel_aligned":false,
                    "status":"failed","reason":"failed_render","measurements":[],"review":{},"regions":[],"automatic_regions":[]}]})
            } else {
                json!({"case_id":"failed","seed":seed,"status":"failed","reason":"failed_render","measurements":[]})
            };
            value["cases"].as_array_mut().unwrap().push(case);
            let mut finding = json!({"id":format!("failure-{seed}"),"case_id":"failed","region_id":"full_image",
                "metric":"execution","decision":"failed","evidence":[path,"evidence.json"]});
            if single {
                finding["pair_id"] = json!("input");
            }
            value["findings"].as_array_mut().unwrap().push(finding);
        }
        value.as_object_mut().unwrap().remove("id");
        value["id"] = json!(common::digest(&value).unwrap());
        common::dump(&root.join("comparison.json"), &value).unwrap();
        let view = success(
            cli()
                .arg("inspect")
                .arg("--comparison")
                .arg(&root)
                .output()
                .unwrap(),
        );
        assert_eq!(view["failed_results"].as_array().unwrap().len(), 2);
        for result in view["failed_results"].as_array().unwrap() {
            let path = format!("cases/failed/seed-{}/render-failure.json", result["seed"]);
            assert_eq!(result["artifact_refs"], json!([path]));
        }
    }
}
