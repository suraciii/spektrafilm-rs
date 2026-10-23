#![cfg(unix)]
#![cfg(feature = "precision-f64")]

use serde_json::{Value, json};
use spektrafilm_benchmark::common;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use tempfile::TempDir;

#[path = "support/progress.rs"]
mod progress;
fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn fixture() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    // Invalid pixels demonstrate that preflight does not decode the input.
    fs::write(root.join("input.png"), b"not an image").unwrap();
    fs::create_dir_all(root.join("data/profiles")).unwrap();
    fs::write(root.join("data/profiles/film.json"), b"{}").unwrap();
    fs::write(root.join("data/profiles/paper.json"), b"{}").unwrap();
    let renderer = root.join("renderer");
    fs::write(&renderer, r#"#!/bin/sh
seed=0
dry=false
bad=false
seed_sensitive=false
crop=false
preview=false
route=normal
while [ "$#" -gt 0 ]; do
    case "$1" in
        --dry-run) dry=true ;;
        --seed) shift; seed="$1" ;;
        --route) shift; route="$1" ;;
        --set)
            shift
            if [ -f "$1" ]; then crop=true; fi
            case "$1" in
                *grain.engine=stale*) bad=true ;;
                *seed_sensitive=true*) seed_sensitive=true ;;
                *io.crop=true*) crop=true ;;
                *preview_mode=true*) preview=true ;;
            esac ;;
    esac
    shift
done
if [ "$dry" != true ]; then
    echo 'render deliberately unavailable in configuration fixture' >&2
    exit 2
fi
if [ "$bad" = true ]; then
    echo 'film_render.grain.engine: unknown engine stale; choose dye or fotufilm' >&2
    exit 2
fi
if [ "$seed_sensitive" = true ] && [ "$seed" = 1 ]; then
    echo 'random_seed: seed 1 rejected by fixture' >&2
    exit 2
fi
printf '{"film_profile":"film","print_profile":"paper","parameters":{"random_seed":%s,"io":{"crop":%s,"output_color_space":"ACES2065-1","output_cctf_encoding":false},"workflow":{"route":"%s"},"settings":{"preview_mode":%s},"cmyk":{"active":false},"taps":{"inject":"rgb_in","collect":"rgb_out"}}}\n' "$seed" "$crop" "$route" "$preview"
"#).unwrap();
    fs::set_permissions(&renderer, fs::Permissions::from_mode(0o755)).unwrap();
    let input = json!({"kind":"raster","path":"input.png","sha256":common::sha256_file(&root.join("input.png")).unwrap(),"color_space":"sRGB","cctf_decoding":true});
    let suite = json!({"schema_version":1,"id":"preflight-suite","protocol":{"id":"appearance-v1","pixels_per_degree":60.0,"region_sampling_scales":[1.0],"grain":{"residual":{"method":"flat_mean","high_pass_radius_px":4.0,"border_px":1},"spectrum":{"window":"hann"}},"hotspots":{"count":0,"min_area_px":1}},"cases":[
        {"id":"first","kind":"probe","input":input,"configuration":{},"seeds":[0,1,2],"regions":[],"measurements":[]},
        {"id":"second","kind":"probe","input":input,"configuration":{},"seeds":[0],"regions":[],"measurements":[]}
    ]});
    common::dump(&root.join("suite.json"), &suite).unwrap();
    common::dump(&root.join("target.json"), &json!({"schema_version":1,"id":"preflight-target","renderer":{"path":"renderer","sha256":common::sha256_file(&renderer).unwrap()},"data_dir":"data","backend":"cpu","changes":{"set_files":[],"set":[]}})).unwrap();
    temp
}

fn invoke(root: &Path, out: &str, dry: bool, selection: &[&str], code: i32) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"));
    command
        .arg("run")
        .arg("suite")
        .arg("--suite")
        .arg(root.join("suite.json"))
        .arg("--target")
        .arg(root.join("target.json"))
        .arg("--out")
        .arg(root.join(out));
    if dry {
        command.arg("--dry-run");
    }
    for id in selection {
        command.arg("--case").arg(id);
    }
    let output = command.output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Parsing the entire stream also enforces one JSON result object.
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let manifest = root
        .join(out)
        .join(if dry { "preflight.json" } else { "run.json" });
    if manifest.is_file() {
        progress::assert_case_seed_progress(&output.stderr, "run", &read(&manifest));
    } else {
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Starting run "));
    }
    result
}

fn change_target(root: &Path, changes: Value) {
    let path = root.join("target.json");
    let mut target = read(&path);
    target["case_changes"] = changes;
    for change in target["case_changes"].as_object_mut().unwrap().values_mut() {
        if change.get("set_files").is_none() {
            change["set_files"] = json!([]);
        }
    }
    common::dump(&path, &target).unwrap();
}

#[test]
fn selectors_report_unknown_duplicate_and_available_ids_before_output_creation() {
    let temp = fixture();
    for dry in [false, true] {
        let result = invoke(
            temp.path(),
            "invalid",
            dry,
            &["missing", "first", "first"],
            1,
        );
        let error = &result["errors"][0];
        assert_eq!(error["code"], "case_selection");
        let message = error["message"].as_str().unwrap();
        assert!(
            message.contains("missing")
                && message.contains("duplicate")
                && message.contains("second")
        );
        assert!(!temp.path().join("invalid").exists());
    }
}

#[test]
fn stale_configuration_groups_all_checked_seeds_and_continues_other_cases() {
    let temp = fixture();
    let root = temp.path();
    change_target(
        root,
        json!({"first":{"set":["film_render.grain.engine=stale"]}}),
    );
    let result = invoke(root, "preflight", true, &[], 1);
    assert_eq!(result["mode"], "dry_run");
    assert_eq!(result["errors"][0]["affected_seeds"], json!([0, 1, 2]));
    assert!(
        result["errors"][0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("film_render.grain.engine")
    );
    let manifest = read(&root.join("preflight/preflight.json"));
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 4);
    assert_eq!(manifest["cases"][3]["status"], "validated");
    assert_eq!(manifest["cases"][3]["stage_evidence"]["requested"], false);
    assert_eq!(
        manifest["cases"][3]["stage_evidence"]["availability"],
        "not_applicable"
    );
    assert_eq!(
        manifest["cases"][3]["stage_evidence"]["context"]["exposure"]["availability"],
        "unavailable"
    );
    assert!(!root.join("preflight/run.json").exists());
    for row in manifest["cases"].as_array().unwrap() {
        let folder = root
            .join("preflight/cases")
            .join(row["case_id"].as_str().unwrap())
            .join(format!("seed-{}", row["seed"]));
        assert!(!folder.join("master.tif").exists());
        assert!(!folder.join("source.tif").exists());
    }
    for artifact in manifest["artifacts"].as_array().unwrap() {
        assert_eq!(
            artifact["sha256"],
            common::sha256_file(
                &root
                    .join("preflight")
                    .join(artifact["path"].as_str().unwrap())
            )
            .unwrap()
        );
    }
    for name in [
        "pixel_decode",
        "gpu_execution",
        "output_write",
        "observed_output_geometry",
        "numerical_output",
    ] {
        assert!(
            manifest["unchecked"]
                .as_array()
                .unwrap()
                .iter()
                .any(|check| check["check"] == name && check["status"] == "unchecked")
        );
    }
}

#[test]
fn seed_dependent_failure_does_not_claim_other_seeds_failed_or_skip_them() {
    let temp = fixture();
    change_target(
        temp.path(),
        json!({"first":{"set":["seed_sensitive=true"]}}),
    );
    let result = invoke(temp.path(), "preflight", true, &["first"], 1);
    assert_eq!(result["errors"][0]["affected_seeds"], json!([1]));
    let manifest = read(&temp.path().join("preflight/preflight.json"));
    assert_eq!(manifest["cases"][0]["status"], "validated");
    assert_eq!(manifest["cases"][1]["status"], "failed");
    assert_eq!(manifest["cases"][2]["status"], "validated");
}

#[test]
fn preflight_and_run_retain_identical_configuration_for_every_seed() {
    let temp = fixture();
    let root = temp.path();
    let result = invoke(root, "preflight", true, &["first"], 0);
    assert_eq!(result["execution_status"], "validated");
    assert_eq!(result["mode"], "dry_run");
    assert!(!root.join("preflight/run.json").exists());
    let manifest = read(&root.join("preflight/preflight.json"));
    assert_eq!(manifest["execution_status"], "validated");
    let mut content = manifest.clone();
    content.as_object_mut().unwrap().remove("id");
    assert_eq!(manifest["id"], common::digest(&content).unwrap());
    let compare = Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
        .arg("compare")
        .arg("runs")
        .arg("--baseline")
        .arg(root.join("preflight"))
        .arg("--candidate")
        .arg(root.join("preflight"))
        .arg("--out")
        .arg(root.join("comparison"))
        .output()
        .unwrap();
    assert_eq!(compare.status.code(), Some(1));
    let rejected: Value = serde_json::from_slice(&compare.stdout).unwrap();
    assert_eq!(rejected["execution_status"], "failed");
    assert!(!root.join("comparison").exists());
    // Rendering is deliberately unavailable, but real run retains the same
    // checked configuration for all seeds before its render boundary fails.
    invoke(root, "actual", false, &["first"], 1);
    for seed in 0..3 {
        let relative = format!("cases/first/seed-{seed}/resolved.json");
        let preflight = read(&root.join("preflight").join(&relative));
        assert_eq!(preflight, read(&root.join("actual").join(&relative)));
        assert_eq!(preflight["parameters"]["random_seed"], seed);
    }
}

#[test]
fn target_owned_geometry_changes_are_rejected_in_both_modes() {
    let temp = fixture();
    let root = temp.path();
    fs::write(root.join("crop.set"), b"io.crop=true").unwrap();
    change_target(root, json!({"first":{"set_files":["crop.set"],"set":[]}}));
    invoke(root, "preflight", true, &["first"], 1);
    invoke(root, "actual", false, &["first"], 1);
    for (out, manifest) in [("preflight", "preflight.json"), ("actual", "run.json")] {
        let document = read(&root.join(out).join(manifest));
        for row in document["cases"].as_array().unwrap() {
            assert_eq!(row["error"]["code"], "target_geometry");
            assert!(
                row["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("io.crop")
            );
        }
    }
}

#[test]
fn ordinary_preflight_rejects_retired_capture_without_creating_outputs() {
    let temp = fixture();
    let root = temp.path();
    let mut suite = read(&root.join("suite.json"));
    suite["cases"][0]["configuration"] = json!({"set":["settings.preview_mode=true"]});
    common::dump(&root.join("suite.json"), &suite).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
        .arg("run")
        .arg("suite")
        .arg("--suite")
        .arg(root.join("suite.json"))
        .arg("--target")
        .arg(root.join("target.json"))
        .arg("--out")
        .arg(root.join("capture"))
        .arg("--dry-run")
        .arg("--stage-evidence")
        .arg("--case")
        .arg("first")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--stage-evidence"));
    assert!(!root.join("capture").exists());
    // Preview remains valid when no dedicated numerical observer is requested.
    invoke(root, "plain", true, &["first"], 0);
}
