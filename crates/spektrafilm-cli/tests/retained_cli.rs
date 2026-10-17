#![cfg(feature = "precision-f64")]
use image::{GrayImage, Luma, Rgb, RgbImage};
use serde_json::{Value, json};
use spektrafilm_benchmark::{
    common::{self, Evidence},
    documents,
};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/progress.rs"]
mod progress;

use fixtures::{record, save_manifest, write};

fn invoke(args: &[&str], paths: &[(&str, &Path)], code: i32) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"));
    command.args(args);
    for (option, path) in paths {
        command.arg(option).arg(path);
    }
    let output = command.output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result = serde_json::from_slice(&output.stdout).unwrap();
    if args.first() == Some(&"compare") {
        let destination = paths
            .iter()
            .find(|(option, _)| *option == "--out")
            .unwrap()
            .1;
        let manifest = destination.join("comparison.json");
        if manifest.is_file() {
            progress::assert_case_seed_progress(&output.stderr, "compare", &read(&manifest));
        }
    }
    result
}
fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

// Retained masters exercise the public CLI without requiring a renderer or network in PR.
fn retained_run(root: &Path, value: f64, masked: bool) {
    retained_run_with_review_regions(root, value, masked, false);
}

fn retained_run_with_review_regions(root: &Path, value: f64, masked: bool, crops: bool) {
    fs::create_dir(root).unwrap();
    let mut evidence = Evidence::new(root);
    let source = root.join("input.png");
    RgbImage::from_pixel(4, 4, Rgb([100, 100, 100]))
        .save(&source)
        .unwrap();
    let mut input = evidence
        .add(&source, "source_input", "image/png", json!({}))
        .unwrap();
    input["source_path"] = json!(root.parent().unwrap().join("original/input.png"));
    let mut dependencies = vec![input.clone()];
    let mut regions = vec![];
    if masked {
        let mut mask = GrayImage::from_pixel(4, 4, Luma([0]));
        for y in 0..4 {
            for x in 0..2 {
                mask.put_pixel(x, y, Luma([255]));
            }
        }
        mask.save(root.join("mask.png")).unwrap();
        let mut retained = evidence
            .add(&root.join("mask.png"), "mask", "image/png", json!({}))
            .unwrap();
        retained["source_path"] = json!(root.parent().unwrap().join("original/mask.png"));
        regions.push(json!({"id":"patch","label":"Patch","role":"other","geometry":{"path":"mask.png","sha256":retained["sha256"],"width":4,"height":4,"space":"render"},"measurements":["numeric"]}));
        dependencies.push(retained);
    }
    if crops {
        regions.push(json!({"id":"detail","label":"Detail","role":"other","geometry":{"x":1,"y":1,"width":2,"height":2,"space":"render"},"measurements":["numeric"]}));
    }
    let suite = json!({"schema_version":1,"id":"retained","protocol":{"id":"appearance-v1","pixels_per_degree":60.0,"region_sampling_scales":[1.0],"grain":{"residual":{"method":"flat_mean","high_pass_radius_px":1.0,"border_px":0},"spectrum":{"window":"hann"}},"hotspots":{"count":if crops { 1 } else { 0 },"min_area_px":1}},"cases":[{"id":"probe","kind":"probe","input":{"kind":"raster","path":"input.png","sha256":input["sha256"],"color_space":"sRGB","cctf_decoding":true},"configuration":{},"seeds":[0],"regions":regions,"measurements":["numeric"]}]});
    let mut suite_record = record(&mut evidence, "suite.json", "suite", suite.clone());
    suite_record["digest"] = json!(common::digest(&suite).unwrap());
    suite_record["source_path"] = json!(root.parent().unwrap().join("original/suite.json"));
    let target = json!({"schema_version":1,"id":"retained-target","renderer":{"path":"missing-renderer","sha256":"a".repeat(64)},"data_dir":"missing-data","backend":"cpu","changes":{"set_files":[],"set":[]}});
    let target_record = record(&mut evidence, "target.json", "target", target);
    let inventory = record(
        &mut evidence,
        "inventory.json",
        "data_inventory",
        json!({"sha256":"b".repeat(64),"files":[]}),
    );
    dependencies.push(inventory);
    let resolved = record(
        &mut evidence,
        "resolved.json",
        "resolved_configuration",
        json!({"parameters":{"io":{"output_color_space":"sRGB","output_cctf_encoding":false}}}),
    );
    let diagnostics = record(
        &mut evidence,
        "diagnostics.json",
        "render_diagnostics",
        json!({"configuration":{"published_dimensions":[4,4]}}),
    );
    let color = json!({"color_space":"sRGB","cctf_decoding":false});
    let layout = json!({"width":4,"height":4,"channels":3,"dtype":"f64","channel_order":"RGB","endianness":"little"});
    let mut bytes = Vec::new();
    for _ in 0..48 {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(root.join("master.f64"), bytes).unwrap();
    let raw = evidence
        .add(
            &root.join("master.f64"),
            "native_master",
            "application/octet-stream",
            json!({"layout":layout,"color":color}),
        )
        .unwrap();
    let metadata = record(
        &mut evidence,
        "master.f64.json",
        "raw_metadata",
        json!({"layout":layout,"color":color}),
    );
    RgbImage::from_pixel(4, 4, Rgb([100, 100, 100]))
        .save(root.join("master.tif"))
        .unwrap();
    let master = evidence
        .add(
            &root.join("master.tif"),
            "floating_master",
            "image/tiff",
            json!({"color":color}),
        )
        .unwrap();
    fs::write(root.join("stdout.txt"), b"retained renderer output").unwrap();
    fs::write(root.join("stderr.txt"), b"").unwrap();
    let stdout = evidence
        .add(
            &root.join("stdout.txt"),
            "renderer_log",
            "text/plain",
            json!({}),
        )
        .unwrap();
    let stderr = evidence
        .add(
            &root.join("stderr.txt"),
            "renderer_log",
            "text/plain",
            json!({}),
        )
        .unwrap();
    let source_color = json!({"color_space":"ACES2065-1","cctf_decoding":false});
    let source_diagnostics = record(
        &mut evidence,
        "source-diagnostics.json",
        "source_diagnostics",
        json!({"configuration":{"input_dimensions":[4,4],"published_dimensions":[4,4]}}),
    );
    let source_resolved = record(
        &mut evidence,
        "source-resolved.json",
        "source_configuration",
        json!({"parameters":{"io":{"output_color_space":"ACES2065-1","output_cctf_encoding":false}}}),
    );
    let source_bytes: Vec<u8> = (0..48).flat_map(|_| 0.1f64.to_le_bytes()).collect();
    fs::write(root.join("source.f64"), source_bytes).unwrap();
    let source_raw = evidence
        .add(
            &root.join("source.f64"),
            "source_raw",
            "application/octet-stream",
            json!({"layout":layout,"color":source_color}),
        )
        .unwrap();
    let source_metadata = record(
        &mut evidence,
        "source.f64.json",
        "source_raw_metadata",
        json!({"width":4,"height":4,"channels":3,"dtype":"f64","channel_order":"RGB","endianness":"little","color_space":"ACES2065-1","cctf_decoding":false}),
    );
    let row = json!({"case_id":"probe","seed":0,"status":"completed","input":input,"raw":raw,"raw_metadata":metadata,"source_raw":source_raw,"source_raw_metadata":source_metadata,"source_resolved":source_resolved,"source_diagnostics":source_diagnostics,"source_master":master,"source_dimensions":[4,4],"source_geometry":{"published_dimensions":[4,4]},"simulation_geometry":{"published_dimensions":[4,4]},"source_pixel_aligned":true,"master":master,"diagnostics":diagnostics,"resolved":resolved,"stdout":stdout,"stderr":stderr,"dimensions":[4,4],"execution":{"backend_selected":"cpu","path":"cpu"},"precision":{"host_precision":"f64"},"command":["renderer","process","input.png","--output","master.tif","--raw-out","master.f64","--diagnostics","diagnostics.json","--seed","0"],"source_command":["renderer","process","input.png","--route","input","--output","source.tif","--raw-out","source.f64","--diagnostics","source-diagnostics.json","--seed","0"]});
    save_manifest(
        root,
        "run.json",
        json!({"schema_version":1,"kind":"run","execution_status":"completed","suite":suite_record,"target":target_record,"protocol":suite["protocol"],"selection":{"case_ids":["probe"],"full_suite":true},"dependencies":dependencies,"cases":[row],"artifacts":evidence.values(),"environment":{}}),
    );
}
fn compare(a: &Path, b: &Path, out: &Path, code: i32) -> Value {
    invoke(
        &["compare"],
        &[("--baseline", a), ("--candidate", b), ("--out", out)],
        code,
    )
}
fn validate(root: &Path, name: &str, code: i32) -> Value {
    invoke(
        &["validate", "--kind", "comparison"],
        &[("--file", &root.join(name))],
        code,
    )
}
fn measurement(document: &Value, region: &str, metric: &str) -> Value {
    document["cases"][0]["measurements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["region_id"] == region && v["metric"] == metric)
        .unwrap()
        .clone()
}
fn policy(path: &Path, suite: &Value) -> Value {
    let p = json!({"schema_version":1,"id":"exploratory","protocol_digest":documents::protocol_digest(suite).unwrap(),"calibration":{"status":"exploratory","evidence":[]},"rules":[{"id":"zero","purpose":"protect","selector":{"case_id":"probe","region_id":"full_image"},"metric":"numeric.rmse","required":true,"max":0.0}]});
    write(path, &p);
    p
}

#[test]
fn cli_report_opens_images_and_exposes_real_manual_and_automatic_crops() {
    let temp = TempDir::new().unwrap();
    let baseline = temp.path().join("baseline");
    let candidate = temp.path().join("candidate");
    retained_run_with_review_regions(&baseline, 0.2, false, true);
    retained_run_with_review_regions(&candidate, 0.3, false, true);
    let comparison = temp.path().join("comparison");
    compare(&baseline, &candidate, &comparison, 0);
    let report = temp.path().join("report");
    invoke(
        &["report"],
        &[("--comparison", &comparison), ("--out", &report)],
        0,
    );
    let html = fs::read_to_string(report.join("index.html")).unwrap();
    let full = html
        .split_once("<div data-review-full>")
        .expect("main image view must be present")
        .1
        .split_once("<div data-review-difference")
        .unwrap()
        .0;
    assert!(full.contains("class=\"viewer\" data-mode=\"split\" data-aligned=\"true\""));
    assert!(full.contains("data-split-handle"));
    assert!(full.contains("data-fullscreen"));
    assert!(full.contains("data-native=\""));
    assert!(html.contains("data-difference-toggle"));
    assert!(html.contains("data-crop-target=\"detail\" data-crop-kind=\"manual\""));
    assert!(html.contains("data-crop-target=\"automatic_001\" data-crop-kind=\"automatic\""));
    for region in ["detail", "automatic_001"] {
        let card = html
            .split_once(&format!("data-crop-target=\"{region}\""))
            .unwrap()
            .1
            .split_once("</button>")
            .unwrap()
            .0;
        for image in card.split("<img src=\"").skip(1) {
            let path = image.split_once('"').unwrap().0;
            let pixels = image::open(report.join(path)).unwrap();
            assert_eq!(pixels.color(), image::ColorType::Rgb16);
            if region == "detail" {
                assert_eq!((pixels.width(), pixels.height()), (2, 2));
            }
            assert!(pixels.width() > 0 && pixels.height() > 0);
        }
        assert_eq!(card.matches("<img src=\"").count(), 2);
        let detail = html
            .split_once(&format!(
                "data-region-id=\"{region}\" data-review-region hidden"
            ))
            .expect("thumbnail must target a retained detail view")
            .1
            .split("<details data-region-id=")
            .next()
            .unwrap();
        assert!(detail.contains("class=\"viewer\""));
        assert!(detail.contains("class=\"locator-image\""));
    }
    let difference = html
        .split_once("<div data-review-difference hidden>")
        .unwrap()
        .1
        .split_once("</pre></div>")
        .unwrap()
        .0;
    let metadata = difference.split_once("<pre>").unwrap().1;
    let metadata: Value = serde_json::from_str(&metadata.replace("&quot;", "\"")).unwrap();
    let manifest = read(&report.join("report.json"));
    let display = manifest["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["role"] == "difference_display")
        .unwrap();
    for key in ["units", "scale", "clipping", "amplification"] {
        assert_eq!(metadata[key], display[key]);
        assert!(!metadata[key].is_null());
    }
}

#[test]
fn cli_report_without_regions_has_an_explicit_crop_empty_state() {
    let temp = TempDir::new().unwrap();
    let baseline = temp.path().join("baseline");
    let candidate = temp.path().join("candidate");
    retained_run(&baseline, 0.2, false);
    retained_run(&candidate, 0.3, false);
    let comparison = temp.path().join("comparison");
    compare(&baseline, &candidate, &comparison, 0);
    let report = temp.path().join("report");
    invoke(
        &["report"],
        &[("--comparison", &comparison), ("--out", &report)],
        0,
    );
    let html = fs::read_to_string(report.join("index.html")).unwrap();
    assert!(html.contains("data-review-full"));
    assert!(html.contains("data-crop-empty"));
    assert!(!html.contains("data-crop-target="));
}

#[test]
fn offline_masked_comparison_preserves_native_linear_samples() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    retained_run(&a, 0.2, true);
    retained_run(&b, 0.3, true);
    let out = temp.path().join("comparison");
    compare(&a, &b, &out, 0);
    let d = read(&out.join("comparison.json"));
    let m = measurement(&d, "patch", "numeric.rmse");
    assert!((m["value"].as_f64().unwrap() - 0.1).abs() < 1e-14);
    assert_eq!(m["sample_count"], 24);
    validate(&out, "comparison.json", 0);
    let report = temp.path().join("report");
    invoke(
        &["report"],
        &[("--comparison", &out), ("--out", &report)],
        0,
    );
    assert_eq!(read(&report.join("comparison.json")), d);
    let snapshot_hash = common::sha256_file(&out.join("comparison.json")).unwrap();
    fs::remove_dir_all(&a).unwrap();
    fs::remove_dir_all(&b).unwrap();
    fs::remove_dir_all(&out).unwrap();
    assert_eq!(
        common::sha256_file(&report.join("comparison.json")).unwrap(),
        snapshot_hash
    );
    validate(&report, "comparison.json", 0);
    let manifest = read(&report.join("report.json"));
    for artifact in d["artifacts"].as_array().unwrap() {
        let path = artifact["path"].as_str().unwrap();
        assert_eq!(
            common::sha256_file(&report.join(path)).unwrap(),
            artifact["sha256"].as_str().unwrap()
        );
        assert!(
            manifest["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| {
                    entry["path"] == artifact["path"] && entry["sha256"] == artifact["sha256"]
                })
        );
    }
    let html = fs::read_to_string(report.join("index.html")).unwrap();
    assert!(html.contains("data-case-id=\"probe\" data-seed=\"0\""));
    assert!(html.contains("data-pair-id=\"baseline/candidate\" data-default-pair=\"true\""));
    assert!(html.contains(
        "href=\"#case=probe&amp;seed=0&amp;pair=baseline%2Fcandidate&amp;region=patch\""
    ));
    assert!(html.contains("data-region-id=\"patch\""));
    assert!(html.contains("data-navigation-error role=\"alert\""));
}

#[test]
fn nonfinite_master_fails_compare_without_gate() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    retained_run(&a, 0.2, false);
    retained_run(&b, f64::NAN, false);
    let out = temp.path().join("comparison");
    let result = compare(&a, &b, &out, 1);
    assert_eq!(result["execution_status"], "failed");
    assert_eq!(result["errors"][0]["code"], "measurement_failure");
    let d = read(&out.join("comparison.json"));
    assert_eq!(d["cases"][0]["status"], "failed");
    assert_ne!(d["evaluation_status"], "passed");
    validate(&out, "comparison.json", 0);
}

#[test]
fn failed_render_comparison_keeps_execution_finding_and_progress() {
    let temp = TempDir::new().unwrap();
    let baseline = temp.path().join("baseline");
    let candidate = temp.path().join("candidate");
    retained_run(&baseline, 0.2, false);
    retained_run(&candidate, 0.3, false);
    let mut failed = read(&candidate.join("run.json"));
    failed["execution_status"] = json!("failed");
    failed["cases"][0]["status"] = json!("failed");
    failed["cases"][0]["error"] =
        json!({"code":"render_failed","message":"renderer rejected the input"});
    save_manifest(&candidate, "run.json", failed);
    let out = temp.path().join("comparison");
    let result = compare(&baseline, &candidate, &out, 1);
    assert_eq!(result["execution_status"], "failed");
    let retained = read(&out.join("comparison.json"));
    let finding = retained["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["metric"] == "execution")
        .unwrap();
    assert_eq!(finding["case_id"], "probe");
    assert_eq!(finding["seed"], 0);
    assert_eq!(finding["decision"], "failed");
    let evidence_path = finding["evidence"][0].as_str().unwrap();
    let failure = read(&out.join(evidence_path));
    assert_eq!(failure["candidate"]["error"]["code"], "render_failed");
    validate(&out, "comparison.json", 0);
}

#[test]
fn undecodable_master_reports_failed_progress_and_retains_incomplete_destination() {
    let temp = TempDir::new().unwrap();
    let run = temp.path().join("run");
    retained_run(&run, 0.2, false);
    let mut manifest = read(&run.join("run.json"));
    fs::rename(run.join("master.f64"), run.join("master.npy")).unwrap();
    manifest["cases"][0]["raw"]["path"] = json!("master.npy");
    for artifact in manifest["artifacts"].as_array_mut().unwrap() {
        if artifact["path"] == "master.f64" {
            artifact["path"] = json!("master.npy");
        }
    }
    save_manifest(&run, "run.json", manifest);
    let out = temp.path().join("comparison");
    let output = Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
        .arg("compare")
        .arg("--baseline")
        .arg(&run)
        .arg("--candidate")
        .arg(&run)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["execution_status"], "failed");
    assert!(
        result["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("incomplete")
    );
    assert!(out.is_dir());
    assert!(!out.join("comparison.json").exists());
    progress::assert_case_seed_progress(
        &output.stderr,
        "compare",
        &json!({"cases":[{"case_id":"probe","seed":0,"status":"failed"}]}),
    );
}

#[test]
fn exploratory_policy_cannot_qualify_and_wrong_protocol_creates_no_output() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    retained_run(&a, 0.2, false);
    let p = temp.path().join("policy.json");
    let mut value = policy(&p, &read(&a.join("suite.json")));
    let out = temp.path().join("comparison");
    let result = invoke(
        &["compare", "--gate"],
        &[
            ("--baseline", &a),
            ("--candidate", &a),
            ("--policy", &p),
            ("--out", &out),
        ],
        1,
    );
    assert_eq!(result["execution_status"], "completed");
    assert_eq!(result["evaluation_status"], "insufficient_evidence");
    assert_eq!(
        read(&out.join("comparison.json"))["rules"][0]["status"],
        "passed"
    );
    value["protocol_digest"] = json!("0".repeat(64));
    write(&p, &value);
    let wrong = temp.path().join("wrong");
    let result = invoke(
        &["compare"],
        &[
            ("--baseline", &a),
            ("--candidate", &a),
            ("--policy", &p),
            ("--out", &wrong),
        ],
        1,
    );
    assert_eq!(result["errors"][0]["code"], "protocol_mismatch");
    assert!(!wrong.exists());
    value["protocol_digest"] =
        json!(documents::protocol_digest(&read(&a.join("suite.json"))).unwrap());
    value["rules"][0]["selector"]["region_id"] = json!("unknown-region");
    write(&p, &value);
    let unresolved = temp.path().join("unresolved");
    let result = invoke(
        &["compare"],
        &[
            ("--baseline", &a),
            ("--candidate", &a),
            ("--policy", &p),
            ("--out", &unresolved),
        ],
        1,
    );
    assert_eq!(result["errors"][0]["code"], "unresolved_identifier");
    assert!(!unresolved.exists());
}

#[test]
fn measurement_availability_and_unindexed_references_are_rejected_by_validate_and_report() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    retained_run(&a, 0.2, false);
    let out = temp.path().join("comparison");
    compare(&a, &a, &out, 0);
    let original = read(&out.join("comparison.json"));
    let mut missing_value = original.clone();
    missing_value["cases"][0]["measurements"][0]
        .as_object_mut()
        .unwrap()
        .remove("value");
    let mut unexpected_value = original.clone();
    unexpected_value["cases"][0]["measurements"][0]["availability"] = json!("unavailable");
    unexpected_value["cases"][0]["measurements"][0]["reason"] = json!("missing_seed");
    let mut unindexed = original.clone();
    unindexed["cases"][0]["measurements"][0]["artifact_refs"] = json!(["absent.npy"]);
    for (index, (d, expected)) in [
        (missing_value, "measurement_value"),
        (unexpected_value, "measurement_availability"),
        (unindexed, "unindexed_artifact"),
    ]
    .into_iter()
    .enumerate()
    {
        save_manifest(&out, "comparison.json", d);
        assert_eq!(
            validate(&out, "comparison.json", 1)["errors"][0]["code"],
            expected
        );
        let report = temp.path().join(format!("report-{index}"));
        let result = invoke(
            &["report"],
            &[("--comparison", &out), ("--out", &report)],
            1,
        );
        assert_eq!(result["errors"][0]["code"], expected);
        assert!(!report.exists());
    }
}

#[test]
fn artifact_traversal_missing_and_tampered_bytes_never_publish_reports() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    retained_run(&a, 0.2, false);
    let out = temp.path().join("comparison");
    compare(&a, &a, &out, 0);
    let original = read(&out.join("comparison.json"));
    for (index, (name, expected)) in [
        ("../outside", "unsafe_artifact_path"),
        ("missing.bin", "missing_artifact"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut d = original.clone();
        d["artifacts"][0]["path"] = json!(name);
        save_manifest(&out, "comparison.json", d);
        let report = temp.path().join(format!("report-{index}"));
        let result = invoke(
            &["report"],
            &[("--comparison", &out), ("--out", &report)],
            1,
        );
        assert_eq!(result["errors"][0]["code"], expected);
        assert!(!report.exists());
    }
    save_manifest(&out, "comparison.json", original.clone());
    fs::write(
        out.join(original["artifacts"][0]["path"].as_str().unwrap()),
        b"tampered",
    )
    .unwrap();
    assert_eq!(
        validate(&out, "comparison.json", 1)["errors"][0]["code"],
        "hash_mismatch"
    );
    let report = temp.path().join("tampered-report");
    assert_eq!(
        invoke(
            &["report"],
            &[("--comparison", &out), ("--out", &report)],
            1
        )["errors"][0]["code"],
        "hash_mismatch"
    );
    assert!(!report.exists());
}

#[test]
fn missing_seed_and_modified_manifest_are_rejected_before_comparison() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    retained_run(&a, 0.2, false);
    let original = read(&a.join("run.json"));
    let mut missing = original.clone();
    missing["cases"] = json!([]);
    save_manifest(&a, "run.json", missing);
    let out = temp.path().join("missing-seed");
    assert_eq!(
        compare(&a, &a, &out, 1)["errors"][0]["code"],
        "run_selection"
    );
    assert!(!out.exists());
    let mut changed = original;
    changed["environment"] = json!({"RAYON_NUM_THREADS":"9"});
    write(&a.join("run.json"), &changed);
    let out = temp.path().join("stale-identity");
    assert_eq!(
        compare(&a, &a, &out, 1)["errors"][0]["code"],
        "manifest_identity"
    );
    assert!(!out.exists());
}

#[cfg(unix)]
#[test]
fn artifact_symlink_is_rejected_even_when_bytes_match() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    retained_run(&a, 0.2, false);
    let out = temp.path().join("comparison");
    compare(&a, &a, &out, 0);
    let d = read(&out.join("comparison.json"));
    let artifact = out.join(d["artifacts"][0]["path"].as_str().unwrap());
    let target = temp.path().join("original-bytes");
    fs::rename(&artifact, &target).unwrap();
    std::os::unix::fs::symlink(&target, &artifact).unwrap();
    assert_eq!(
        validate(&out, "comparison.json", 1)["errors"][0]["code"],
        "unsafe_artifact_path"
    );
    let report = temp.path().join("report");
    assert_eq!(
        invoke(
            &["report"],
            &[("--comparison", &out), ("--out", &report)],
            1
        )["errors"][0]["code"],
        "unsafe_artifact_path"
    );
    assert!(!report.exists());
}

#[test]
fn retained_input_configuration_and_inventory_tampering_are_rejected() {
    for name in [
        "input.png",
        "resolved.json",
        "target.json",
        "inventory.json",
    ] {
        let temp = TempDir::new().unwrap();
        let a = temp.path().join("a");
        retained_run(&a, 0.2, false);
        fs::write(a.join(name), b"changed after publication").unwrap();
        let out = temp.path().join("comparison");
        assert_eq!(
            compare(&a, &a, &out, 1)["errors"][0]["code"],
            "hash_mismatch",
            "{name}"
        );
        assert!(!out.exists());
    }
}

#[test]
fn raw_declaration_rejects_raster_fields_and_raster_rejects_raw_controls() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    retained_run(&a, 0.2, false);
    let path = a.join("suite.json");
    let mut suite = read(&path);
    suite["cases"][0]["input"]["kind"] = json!("raw");
    suite["cases"][0]["input"]["adapter"] = json!("spektrafilm-raw");
    write(&path, &suite);
    assert_eq!(
        invoke(&["validate", "--kind", "suite"], &[("--file", &path)], 1)["errors"][0]["code"],
        "source_declaration"
    );
    suite["cases"][0]["input"]
        .as_object_mut()
        .unwrap()
        .remove("color_space");
    suite["cases"][0]["input"]
        .as_object_mut()
        .unwrap()
        .remove("cctf_decoding");
    write(&path, &suite);
    invoke(&["validate", "--kind", "suite"], &[("--file", &path)], 0);
    suite["cases"][0]["input"] = json!({"kind":"raster", "path":"input.png", "sha256": common::sha256_file(&a.join("input.png")).unwrap(), "color_space":"sRGB", "cctf_decoding":true});
    suite["cases"][0]["configuration"]["raw"] =
        json!({"white_balance":"custom", "temperature":4000});
    write(&path, &suite);
    assert_eq!(
        invoke(&["validate", "--kind", "suite"], &[("--file", &path)], 1)["errors"][0]["code"],
        "inapplicable_field"
    );
}

#[test]
fn malformed_run_file_record_returns_json_instead_of_panicking() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    retained_run(&a, 0.2, false);
    let mut run = read(&a.join("run.json"));
    run["cases"][0]["diagnostics"] = json!("diagnostics.json");
    save_manifest(&a, "run.json", run);
    let result = invoke(
        &["validate", "--kind", "run"],
        &[("--file", &a.join("run.json"))],
        1,
    );
    assert_eq!(result["execution_status"], "failed");
    let out = temp.path().join("comparison");
    assert_eq!(compare(&a, &a, &out, 1)["execution_status"], "failed");
    assert!(!out.exists());
}

#[test]
fn protocol_tone_policy_remains_selected_with_empty_case_probe() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    retained_run(&a, 0.2, false);
    let mut suite = read(&a.join("suite.json"));
    suite["cases"][0]["measurements"] = json!(["tone"]);
    suite["cases"][0]["probe"] = json!({});
    suite["protocol"]["probe"] = json!({"tone_response":[{"exposure_stops":0.0,"geometry":{"x":0,"y":0,"width":2,"height":4,"space":"render"}},{"exposure_stops":1.0,"geometry":{"x":2,"y":0,"width":2,"height":4,"space":"render"}}]});
    write(&a.join("suite.json"), &suite);
    let mut run = read(&a.join("run.json"));
    let hash = json!(common::sha256_file(&a.join("suite.json")).unwrap());
    run["suite"]["sha256"] = hash.clone();
    run["suite"]["digest"] = json!(common::digest(&suite).unwrap());
    run["protocol"] = suite["protocol"].clone();
    for artifact in run["artifacts"].as_array_mut().unwrap() {
        if artifact["path"] == "suite.json" {
            artifact["sha256"] = hash.clone();
        }
    }
    save_manifest(&a, "run.json", run);
    let p = temp.path().join("policy.json");
    let mut value = policy(&p, &suite);
    value["rules"][0]["metric"] = json!("tone.response.interval_0.linear_luminance.delta.slope");
    write(&p, &value);
    let out = temp.path().join("comparison");
    let result = invoke(
        &["compare", "--gate"],
        &[
            ("--baseline", &a),
            ("--candidate", &a),
            ("--policy", &p),
            ("--out", &out),
        ],
        1,
    );
    if result["execution_status"] != "completed" {
        panic!(
            "COMPARE ERRORS: {}",
            serde_json::to_string_pretty(&result).unwrap()
        );
    }
    assert_eq!(result["execution_status"], "completed");
    assert_eq!(result["evaluation_status"], "insufficient_evidence");
    let d = read(&out.join("comparison.json"));
    assert_eq!(d["rules"][0]["status"], "passed");
    assert_eq!(
        measurement(
            &d,
            "full_image",
            "tone.response.interval_0.linear_luminance.delta.slope"
        )["value"],
        0.0
    );
}

#[test]
fn reference_policy_requires_distance_suffix_and_corresponding_region() {
    let temp = TempDir::new().unwrap();
    let a = temp.path().join("a");
    retained_run(&a, 0.2, true);
    let mut suite = read(&a.join("suite.json"));
    suite["cases"][0]["reference"] = json!({"id":"ref", "path":"input.png", "sha256":common::sha256_file(&a.join("input.png")).unwrap(), "scope":"regional", "color":{"color_space":"sRGB","cctf_decoding":true}, "provenance":{"source":"controlled fixture","attribution":"test","redistribution":"permitted","capture":"unknown","development":"unknown","digitization":"unknown"}, "processing":{"grading":"none","histogram_matching":"none","compression":"none","sharpening":"none","noise_reduction":"none","resampling":"none"}, "regions":suite["cases"][0]["regions"]});
    write(&a.join("suite.json"), &suite);
    let mut run = read(&a.join("run.json"));
    let hash = json!(common::sha256_file(&a.join("suite.json")).unwrap());
    run["suite"]["sha256"] = hash.clone();
    run["suite"]["digest"] = json!(common::digest(&suite).unwrap());
    for artifact in run["artifacts"].as_array_mut().unwrap() {
        if artifact["path"] == "suite.json" {
            artifact["sha256"] = hash.clone();
        }
    }
    save_manifest(&a, "run.json", run);
    let p = temp.path().join("policy.json");
    for (i, (metric, region)) in [
        ("reference.patch_delta_e00", "patch"),
        ("reference.patch_delta_e00.seed_mean", "patch"),
        ("reference.patch_delta_e00.candidate_distance", "full_image"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut value = policy(&p, &suite);
        value["rules"][0]["metric"] = json!(metric);
        value["rules"][0]["selector"]["region_id"] = json!(region);
        write(&p, &value);
        let out = temp.path().join(format!("invalid-{i}"));
        let result = invoke(
            &["compare"],
            &[
                ("--baseline", &a),
                ("--candidate", &a),
                ("--policy", &p),
                ("--out", &out),
            ],
            1,
        );
        assert_eq!(result["errors"][0]["code"], "unresolved_metric");
        assert!(!out.exists());
    }
    let mut value = policy(&p, &suite);
    value["rules"][0]["metric"] = json!("reference.patch_delta_e00.distance_delta");
    value["rules"][0]["selector"]["region_id"] = json!("patch");
    write(&p, &value);
    let out = temp.path().join("valid-reference");
    let result = invoke(
        &["compare", "--gate"],
        &[
            ("--baseline", &a),
            ("--candidate", &a),
            ("--policy", &p),
            ("--out", &out),
        ],
        1,
    );
    assert_eq!(result["execution_status"], "completed");
    assert_eq!(
        read(&out.join("comparison.json"))["rules"][0]["status"],
        "passed"
    );
}

#[test]
fn historical_parent_relative_and_current_directory_inputs_compare_offline() {
    for input_path in ["../input.png", "./input.png"] {
        let temp = TempDir::new().unwrap();
        let a = temp.path().join("a");
        retained_run(&a, 0.2, false);
        let mut suite = read(&a.join("suite.json"));
        suite["cases"][0]["input"]["path"] = json!(input_path);
        write(&a.join("suite.json"), &suite);
        let mut run = read(&a.join("run.json"));
        if input_path.starts_with("..") {
            run["suite"]["source_path"] = json!(temp.path().join("original/suite-dir/suite.json"));
        }
        let hash = json!(common::sha256_file(&a.join("suite.json")).unwrap());
        run["suite"]["sha256"] = hash.clone();
        run["suite"]["digest"] = json!(common::digest(&suite).unwrap());
        for artifact in run["artifacts"].as_array_mut().unwrap() {
            if artifact["path"] == "suite.json" {
                artifact["sha256"] = hash.clone();
            }
        }
        save_manifest(&a, "run.json", run);
        let out = temp.path().join("comparison");
        assert_eq!(compare(&a, &a, &out, 0)["execution_status"], "completed");
        assert_eq!(
            measurement(
                &read(&out.join("comparison.json")),
                "full_image",
                "numeric.rmse"
            )["value"],
            0.0
        );
    }
}

#[test]
fn offline_pair_report_preserves_unavailable_maps_and_redistribution_omissions() {
    let temp = TempDir::new().unwrap();
    let comparison = temp.path().join("comparison");
    fs::create_dir_all(comparison.join("evidence")).unwrap();
    let mut evidence = Evidence::new(&comparison);
    let mut images = vec![];
    for (name, role, color) in [
        ("input", "baseline_review", [12, 34, 56]),
        ("simulation", "simulation_review", [65, 43, 21]),
        ("private", "reference_review", [90, 80, 70]),
    ] {
        let path = comparison.join(format!("evidence/{name}.png"));
        RgbImage::from_pixel(4, 4, Rgb(color)).save(&path).unwrap();
        images.push(
            evidence
                .add(
                    &path,
                    role,
                    "image/png",
                    json!({"case_id":"image","seed":0}),
                )
                .unwrap(),
        );
    }
    let map_path = comparison.join("evidence/unavailable.npy");
    fs::write(&map_path, b"Unavailable map: no numeric samples").unwrap();
    let mut map = evidence.add(&map_path, "difference_map", "application/octet-stream", json!({"metric":"delta_e00","availability":"unavailable","reason":"visual_reference_only"})).unwrap();
    map["pair_id"] = json!("reference");
    let mut restricted = images[2].clone();
    restricted["redistribution"] = json!("unknown");
    restricted["attribution"] = json!("Private local reference");
    let mut artifacts = evidence.values();
    for artifact in &mut artifacts {
        if artifact["path"] == restricted["path"] {
            *artifact = restricted.clone();
        } else if artifact["path"] == map["path"] {
            *artifact = map.clone();
        }
    }
    let unavailable = json!({"case_id":"image","pair_id":"reference","seed_population":[0],"region_id":"full_image","metric":"reference.visual","domain":"visual","units":"not applicable","estimator":"reference scope","sample_count":0,"availability":"unavailable","reason":"visual_reference_only","artifact_refs":[map["path"]]});
    let input = json!({"id":"input","left_role":"input","right_role":"simulation","pixel_aligned":true,"status":"completed","measurements":[],"review":{"left":images[0],"right":images[1]},"regions":[],"automatic_regions":[]});
    let reference = json!({"id":"reference","left_role":"reference","right_role":"simulation","pixel_aligned":false,"status":"completed","reference":{"scope":"visual","geometry":"side_by_side"},"measurements":[unavailable],"review":{"left":restricted,"right":images[1],"difference":map},"artifacts":[map],"regions":[],"automatic_regions":[]});
    save_manifest(
        &comparison,
        "comparison.json",
        json!({"schema_version":1,"kind":"comparison","mode":"single_run","execution_status":"completed","evaluation_status":"not_evaluated","run":{"id":"retained","manifest_sha256":"a".repeat(64)},"default_pair":"reference","protocol":{},"selection":{"case_ids":["image"],"full_suite":true},"condition_differences":[],"cases":[{"case_id":"image","seed":0,"status":"completed","pairs":[input,reference]}],"findings":[],"summary":{"selected_cases":1,"completed_results":1,"failed_results":0},"artifacts":artifacts}),
    );
    let snapshot = fs::read(comparison.join("comparison.json")).unwrap();
    let report = temp.path().join("report");
    invoke(
        &["report"],
        &[("--comparison", &comparison), ("--out", &report)],
        0,
    );
    fs::remove_dir_all(&comparison).unwrap();
    assert_eq!(fs::read(report.join("comparison.json")).unwrap(), snapshot);
    let manifest = read(&report.join("report.json"));
    let retained = manifest["artifacts"].as_array().unwrap();
    let omitted = retained
        .iter()
        .find(|entry| entry["source_path"] == restricted["path"])
        .unwrap();
    assert!(omitted["path"].is_null());
    assert_eq!(omitted["availability"], "unavailable");
    assert_eq!(omitted["reason"], "redistribution_not_permitted");
    assert_eq!(omitted["redistribution"], "unknown");
    assert_eq!(omitted["attribution"], restricted["attribution"]);
    assert!(!report.join(restricted["path"].as_str().unwrap()).exists());
    let retained_map = retained
        .iter()
        .find(|entry| entry["source_path"] == map["path"])
        .unwrap();
    assert_eq!(retained_map["path"], map["path"]);
    assert_eq!(retained_map["availability"], "unavailable");
    assert_eq!(retained_map["reason"], "visual_reference_only");
    for entry in retained.iter().filter(|entry| !entry["path"].is_null()) {
        assert_eq!(
            common::sha256_file(&report.join(entry["path"].as_str().unwrap())).unwrap(),
            entry["sha256"].as_str().unwrap()
        );
    }
    assert!(
        !retained
            .iter()
            .any(|entry| entry["role"] == "difference_display")
    );
    let html = fs::read_to_string(report.join("index.html")).unwrap();
    assert!(html.contains("href=\"#case=image&amp;seed=0&amp;pair=input\""));
    assert!(html.contains("href=\"#case=image&amp;seed=0&amp;pair=reference\""));
    assert!(html.contains("data-pair-id=\"reference\" data-default-pair=\"true\""));
    let reference_panel = html.split_once("data-pair-panel=\"pair-0-1\"").unwrap().1;
    assert!(!reference_panel.contains("src=\"evidence/input.png\""));
    assert!(!reference_panel.contains("src=\"evidence/private.png\""));
    assert!(!reference_panel.contains("src=\"evidence/unavailable.npy\""));
    assert!(reference_panel.contains("src=\"evidence/simulation.png\""));
    assert!(html.contains("redistribution_not_permitted"));
}
