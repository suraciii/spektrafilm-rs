use image::{GrayImage, Luma, Rgb, RgbImage};
use serde_json::{Value, json};
use spektrafilm_benchmark::{
    common::{self, Evidence},
    documents,
};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn invoke(args: &[&str], paths: &[(&str, &Path)], code: i32) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spektrafilm-bench"));
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
    serde_json::from_slice(&output.stdout).unwrap()
}
fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn write(path: &Path, value: &Value) {
    common::dump(path, value).unwrap();
}
fn identify(mut value: Value) -> Value {
    value.as_object_mut().unwrap().remove("id");
    value["id"] = json!(common::digest(&value).unwrap());
    value
}
fn save_manifest(root: &Path, name: &str, value: Value) {
    write(&root.join(name), &identify(value));
}
fn record(evidence: &mut Evidence, name: &str, role: &str, value: Value) -> Value {
    let path = evidence.root.join(name);
    write(&path, &value);
    evidence
        .add(&path, role, "application/json", json!({}))
        .unwrap()
}

// Retained masters exercise the public CLI without requiring a renderer or network in PR.
fn retained_run(root: &Path, value: f64, masked: bool) {
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
        regions.push(json!({"id":"patch","label":"Patch","role":"other","geometry":{"path":"mask.png","sha256":retained["sha256"],"width":4,"height":4},"measurements":["numeric"]}));
        dependencies.push(retained);
    }
    let suite = json!({"schema_version":1,"id":"retained","protocol":{"id":"appearance-v1","pixels_per_degree":60.0,"region_sampling_scales":[1.0],"grain":{"residual":{"method":"flat_mean","high_pass_radius_px":1.0,"border_px":0},"spectrum":{"window":"hann"}},"hotspots":{"count":0,"min_area_px":1}},"cases":[{"id":"probe","kind":"probe","input":{"kind":"raster","path":"input.png","sha256":input["sha256"],"color_space":"sRGB","cctf_decoding":true},"configuration":{},"seeds":[0],"regions":regions,"measurements":["numeric"]}]});
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
    let color = json!({"color_space":"sRGB","transfer":"linear"});
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
    let row = json!({"case_id":"probe","seed":0,"status":"completed","input":input,"raw":raw,"raw_metadata":metadata,"master":master,"diagnostics":diagnostics,"resolved":resolved,"stdout":stdout,"stderr":stderr,"dimensions":[4,4],"execution":{"backend_selected":"cpu","path":"cpu"},"precision":{"host_precision":"f64"}});
    save_manifest(
        root,
        "run.json",
        json!({"schema_version":1,"execution_status":"completed","suite":suite_record,"target":target_record,"protocol":suite["protocol"],"selection":{"case_ids":["probe"],"full_suite":true},"dependencies":dependencies,"cases":[row],"artifacts":evidence.values(),"environment":{}}),
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
