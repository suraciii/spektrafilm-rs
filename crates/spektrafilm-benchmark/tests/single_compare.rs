use image::{GrayImage, Luma, Rgb, RgbImage};
use serde_json::{Value, json};
use spektrafilm_benchmark::{
    common::{self, Evidence},
    compare, documents, report,
};
use std::{fs, path::Path};

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/progress.rs"]
mod progress;

use fixtures::{record, save_manifest, write};

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
        json!({"parameters":{"io":{"output_color_space":"ACES2065-1","output_cctf_encoding":false}}}),
    );
    let diagnostics = record(
        &mut evidence,
        "diagnostics.json",
        "render_diagnostics",
        json!({"configuration":{"published_dimensions":[4,4]}}),
    );
    let color = json!({"color_space":"ACES2065-1","transfer":"linear"});
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
    let source_color = json!({"color_space":"ACES2065-1","transfer":"linear"});
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
        json!({"width":4,"height":4,"channels":3,"dtype":"f64","channel_order":"RGB","endianness":"little","color_space":"ACES2065-1","transfer":"linear"}),
    );
    let row = json!({"case_id":"probe","seed":0,"status":"completed","input":input,"raw":raw,"raw_metadata":metadata,"source_raw":source_raw,"source_raw_metadata":source_metadata,"source_resolved":source_resolved,"source_diagnostics":source_diagnostics,"source_master":master,"source_dimensions":[4,4],"source_geometry":{"published_dimensions":[4,4]},"simulation_geometry":{"published_dimensions":[4,4]},"source_pixel_aligned":true,"master":master,"diagnostics":diagnostics,"resolved":resolved,"stdout":stdout,"stderr":stderr,"dimensions":[4,4],"execution":{"backend_selected":"cpu","path":"cpu"},"precision":{"host_precision":"f64"},"command":["renderer","process","input.png","--output","master.tif","--raw-out","master.f64","--diagnostics","diagnostics.json","--seed","0"],"source_command":["renderer","process","input.png","--route","input","--output","source.tif","--raw-out","source.f64","--diagnostics","source-diagnostics.json","--seed","0"]});
    save_manifest(
        root,
        "run.json",
        json!({"schema_version":1,"execution_status":"completed","suite":suite_record,"target":target_record,"protocol":suite["protocol"],"selection":{"case_ids":["probe"],"full_suite":true},"dependencies":dependencies,"cases":[row],"artifacts":evidence.values(),"environment":{}}),
    );
}

fn add_reference(root: &Path, scope: &str, color: Value, histogram_matching: &str) {
    let mut run = documents::load_json_document(&root.join("run.json")).unwrap();
    let mut suite = documents::load_json_document(&root.join("suite.json")).unwrap();
    let mut evidence = Evidence::new(root);
    for artifact in run["artifacts"].as_array().unwrap() {
        evidence
            .records
            .insert(artifact["path"].as_str().unwrap().into(), artifact.clone());
    }
    let mut reference = json!({
        "id":"reference", "path":"input.png", "sha256":suite["cases"][0]["input"]["sha256"],
        "scope":scope, "color":color,
        "provenance":{"source":"controlled fixture","attribution":"test","redistribution":"permitted","capture":"unknown","development":"unknown","digitization":"unknown"},
        "processing":{"grading":"none","histogram_matching":histogram_matching,"compression":"none","sharpening":"none","noise_reduction":"none","resampling":"none"},
    });
    if scope == "aligned_scene" {
        GrayImage::from_pixel(4, 4, Luma([255]))
            .save(root.join("common.png"))
            .unwrap();
        let mask = evidence
            .add(
                &root.join("common.png"),
                "common_area_mask",
                "image/png",
                json!({}),
            )
            .unwrap();
        let review = record(
            &mut evidence,
            "correspondence.json",
            "correspondence_review",
            json!({"status":"approved","reviewer":"fixture","excluded_regions":[],"limitations":[]}),
        );
        for artifact in [&mask, &review] {
            let mut dependency = artifact.clone();
            dependency["source_path"] = json!(
                root.parent()
                    .unwrap()
                    .join("original")
                    .join(artifact["path"].as_str().unwrap())
            );
            run["dependencies"].as_array_mut().unwrap().push(dependency);
        }
        reference["aligned_derivative"] = json!({"path":"input.png","sha256":reference["sha256"]});
        reference["common_area_mask"] = json!({"path":"common.png","sha256":mask["sha256"]});
        reference["correspondence_review"] =
            json!({"path":"correspondence.json","sha256":review["sha256"]});
    }
    suite["cases"][0]["reference"] = reference;
    write(&root.join("suite.json"), &suite);
    evidence.records.remove("suite.json");
    let suite_record = evidence
        .add(
            &root.join("suite.json"),
            "suite",
            "application/json",
            json!({}),
        )
        .unwrap();
    run["suite"]["sha256"] = suite_record["sha256"].clone();
    run["suite"]["digest"] = json!(common::digest(&suite).unwrap());
    run["artifacts"] = json!(evidence.values());
    save_manifest(root, "run.json", run);
}

#[test]
fn cli_report_opens_unaligned_reference_as_side_views() {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("run");
    retained_run(&run, 0.4, false);
    add_reference(
        &run,
        "visual",
        json!({"color_space":"sRGB","cctf_decoding":true}),
        "none",
    );
    let comparison = temp.path().join("comparison");
    let report = temp.path().join("report");
    for (command, source_option, source, destination) in [
        ("compare", "--run", &run, &comparison),
        ("generate-report", "--comparison", &comparison, &report),
    ] {
        let mut cli = std::process::Command::new(env!("CARGO_BIN_EXE_spektrafilm-bench"));
        cli.arg(command)
            .arg(source_option)
            .arg(source)
            .arg("--out")
            .arg(destination);
        if command == "compare" {
            cli.args(["--against", "reference"]);
        }
        let output = cli.output().unwrap();
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let _: Value = serde_json::from_slice(&output.stdout).unwrap();
        if command == "compare" {
            let manifest: Value =
                serde_json::from_slice(&fs::read(comparison.join("comparison.json")).unwrap())
                    .unwrap();
            progress::assert_case_seed_progress(&output.stderr, "compare", &manifest);
        }
    }
    let snapshot = documents::load_json_document(&report.join("comparison.json")).unwrap();
    assert_eq!(snapshot["default_pair"], "reference");
    assert_eq!(snapshot["cases"][0]["pairs"][0]["pixel_aligned"], false);
    let html = fs::read_to_string(report.join("index.html")).unwrap();
    let main = html
        .split_once("<div data-review-full>")
        .expect("unaligned reference still needs an image-first main view")
        .1
        .split_once("<div data-review-difference")
        .unwrap()
        .0;
    assert!(main.contains("class=\"viewer\" data-mode=\"side\" data-aligned=\"false\""));
    assert!(!main.contains("<option value=\"split\">"));
    assert!(main.contains("class=\"side-pane\" data-side=\"baseline\""));
    assert!(main.contains("class=\"side-pane\" data-side=\"candidate\""));
    let mut images = 0;
    for image in main.split("data-native=\"").skip(1) {
        let path = image.split_once('"').unwrap().0;
        let pixels = image::open(report.join(path)).unwrap();
        assert_eq!((pixels.width(), pixels.height()), (4, 4));
        images += 1;
    }
    assert!(images >= 2);
}

fn assert_local_artifact(root: &Path, manifest: &Value, artifact: &Value) {
    let path = artifact["path"].as_str().unwrap();
    assert!(
        manifest["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|indexed| indexed["path"] == path && indexed["sha256"] == artifact["sha256"])
    );
    let resolved = documents::safe_artifact_path(root, path).unwrap();
    assert_eq!(
        common::sha256_file(&resolved).unwrap(),
        artifact["sha256"].as_str().unwrap()
    );
}

#[test]
fn single_input_comparison_uses_real_retained_source_offline() {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("run");
    retained_run(&run, 0.4, true);
    let out = temp.path().join("comparison");
    let (_, code) = compare::compare_single(&run, "input", &out).unwrap();
    assert_eq!(code, 0);
    let manifest = documents::load_json_document(&out.join("comparison.json")).unwrap();
    assert_eq!(manifest["mode"], "single_run");
    assert_eq!(manifest["default_pair"], "input");
    let pair = &manifest["cases"][0]["pairs"][0];
    assert_eq!(pair["pixel_aligned"], true);
    assert!(
        pair["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["metric"] == "numeric.rmse" && m["value"].as_f64().unwrap_or(0.) > 0.)
    );
    assert!(
        pair["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["pair_id"] == "input")
    );
    assert_ne!(
        pair["left_master"]["sha256"],
        pair["right_master"]["sha256"]
    );
    fs::remove_dir_all(&run).unwrap();
    for key in [
        "source_raw_metadata",
        "source_resolved",
        "source_diagnostics",
        "left_resolved",
    ] {
        assert_local_artifact(&out, &manifest, &pair[key]);
        documents::load_json_document(&out.join(pair[key]["path"].as_str().unwrap())).unwrap();
    }
    assert_eq!(pair["left_resolved"], pair["source_resolved"]);
    assert_eq!(
        pair["left_execution"]["raw_metadata"],
        pair["source_raw_metadata"]
    );
    assert_eq!(
        pair["left_execution"]["diagnostics"],
        pair["source_diagnostics"]
    );
    let report_root = temp.path().join("report");
    report::build_report(&out, &report_root).unwrap();
    fs::remove_dir_all(&out).unwrap();
    let snapshot = documents::load_json_document(&report_root.join("comparison.json")).unwrap();
    let report_manifest = documents::load_json_document(&report_root.join("report.json")).unwrap();
    for key in [
        "source_raw_metadata",
        "source_resolved",
        "source_diagnostics",
    ] {
        let artifact = &snapshot["cases"][0]["pairs"][0][key];
        assert_local_artifact(&report_root, &report_manifest, artifact);
    }
}

#[test]
fn regional_readouts_keep_direction_and_reject_corrupt_evidence_offline() {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("run");
    retained_run(&run, 0.4, true);
    let mut suite = documents::load_json_document(&run.join("suite.json")).unwrap();
    suite["cases"][0]["regions"][0]["measurements"] = json!(["color"]);
    write(&run.join("suite.json"), &suite);
    let mut manifest = documents::load_json_document(&run.join("run.json")).unwrap();
    let hash = json!(common::sha256_file(&run.join("suite.json")).unwrap());
    manifest["suite"]["sha256"] = hash.clone();
    manifest["suite"]["digest"] = json!(common::digest(&suite).unwrap());
    for artifact in manifest["artifacts"].as_array_mut().unwrap() {
        if artifact["path"] == "suite.json" {
            artifact["sha256"] = hash.clone();
        }
    }
    save_manifest(&run, "run.json", manifest);
    let out = temp.path().join("comparison");
    assert_eq!(compare::compare_single(&run, "input", &out).unwrap().1, 0);
    fs::remove_dir_all(&run).unwrap();
    let inspected =
        spektrafilm_benchmark::inspect::inspect(&out, &[], &["patch".into()], &[]).unwrap();
    for row in inspected["region_readouts"].as_array().unwrap() {
        assert_eq!(row["sides"]["input"]["sample_count"], 8);
        let displacement = &row["displacements"][0];
        assert_eq!(displacement["from_role"], "input");
        assert_eq!(displacement["to_role"], "simulation");
        // Neutral CIE lightness is 116 * cbrt(Y / Yn) - 16 above the toe.
        let expected = 116.0 * (0.4_f64.cbrt() - 0.1_f64.cbrt());
        assert!((displacement["delta_lab"]["value"][0].as_f64().unwrap() - expected).abs() < 0.01);
        assert!(
            displacement["delta_lab"]["value"][1]
                .as_f64()
                .unwrap()
                .abs()
                < 0.01
        );
        assert!(
            displacement["delta_lab"]["value"][2]
                .as_f64()
                .unwrap()
                .abs()
                < 0.01
        );
        assert_eq!(
            displacement["delta_h_degrees"]["availability"],
            "unavailable"
        );
    }
    assert_eq!(inspected["region_readouts"].as_array().unwrap().len(), 2);
    let original = documents::load_json_document(&out.join("comparison.json")).unwrap();
    let mut unindexed = original.clone();
    unindexed["cases"][0]["pairs"][0]["region_readouts"][0]["artifact_refs"] =
        json!(["absent.bin"]);
    let mut wrong_direction = original.clone();
    wrong_direction["cases"][0]["pairs"][0]["region_readouts"][0]["displacements"][0]["from_role"] =
        json!("simulation");
    let mut unavailable_value = original.clone();
    unavailable_value["cases"][0]["pairs"][0]["region_readouts"][0]["displacements"][0]["delta_h_degrees"]
        ["value"] = json!(0.0);
    for (invalid, expected) in [
        (unindexed, "unindexed_artifact"),
        (wrong_direction, "readout_identity"),
        (unavailable_value, "schema_validation"),
    ] {
        save_manifest(&out, "comparison.json", invalid);
        let error = spektrafilm_benchmark::inspect::inspect(&out, &[], &[], &[]).unwrap_err();
        assert_eq!(error.code, expected);
    }
    let mut legacy = original;
    legacy["cases"][0]
        .as_object_mut()
        .unwrap()
        .remove("stage_readouts");
    legacy["cases"][0]["pairs"][0]
        .as_object_mut()
        .unwrap()
        .remove("region_readouts");
    save_manifest(&out, "comparison.json", legacy);
    let inspected =
        spektrafilm_benchmark::inspect::inspect(&out, &[], &["patch".into()], &[]).unwrap();
    assert_eq!(
        inspected["readout_status"][0]["region_readouts"],
        "not_retained"
    );
    assert_eq!(inspected["region_readouts"], json!([]));
}
#[test]
fn missing_reference_fails_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("run");
    retained_run(&run, 0.4, false);
    let out = temp.path().join("comparison");
    let error = compare::compare_single(&run, "both", &out).unwrap_err();
    assert_eq!(error.code, "missing_reference");
    assert!(!out.exists());
}
#[test]
fn equal_dimensions_with_unproven_geometry_retains_visual_pair() {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("run");
    retained_run(&run, 0.4, false);
    let mut manifest = documents::load_json_document(&run.join("run.json")).unwrap();
    manifest["cases"][0]["source_pixel_aligned"] = json!(false);
    save_manifest(&run, "run.json", manifest);
    let out = temp.path().join("comparison");
    let (_, code) = compare::compare_single(&run, "input", &out).unwrap();
    assert_eq!(code, 0);
    let manifest = documents::load_json_document(&out.join("comparison.json")).unwrap();
    let pair = &manifest["cases"][0]["pairs"][0];
    assert_eq!(pair["status"], "completed");
    assert_eq!(pair["pixel_aligned"], false);
    assert_eq!(pair["measurements"][0]["reason"], "input_geometry_mismatch");
    assert!(pair["measurements"][0].get("value").is_none());
    assert!(pair["review"]["left"]["sha256"].is_string());
    assert!(pair["review"]["right"]["sha256"].is_string());
}

#[test]
fn visual_and_unavailable_reference_findings_publish_inspectable_evidence() {
    for (scope, color) in [
        ("visual", json!({"color_space":"sRGB","cctf_decoding":true})),
        ("aligned_scene", json!({})),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let run = temp.path().join("run");
        retained_run(&run, 0.4, false);
        add_reference(&run, scope, color, "none");
        let out = temp.path().join("comparison");
        let (_, code) = compare::compare_single(&run, "both", &out).unwrap();
        assert_eq!(code, 0);
        let manifest = documents::load_json_document(&out.join("comparison.json")).unwrap();
        let pair = &manifest["cases"][0]["pairs"][1];
        assert_eq!(manifest["default_pair"], "reference");
        assert_eq!(pair["status"], "completed");
        assert!(pair["review"]["left"]["path"].is_string());
        assert!(
            pair["measurements"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m["availability"] == "unavailable" && m.get("value").is_none())
        );
        let findings: Vec<_> = manifest["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|finding| finding["pair_id"] == "reference")
            .collect();
        assert!(!findings.is_empty());
        for finding in findings {
            let refs = finding["evidence"].as_array().unwrap();
            assert!(!refs.is_empty());
            for reference in refs {
                let path = reference.as_str().unwrap();
                assert!(
                    manifest["artifacts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|artifact| artifact["path"] == path)
                );
                assert!(out.join(path).is_file());
            }
        }
        fs::remove_dir_all(&run).unwrap();
        report::build_report(&out, &temp.path().join("report")).unwrap();
    }
}

#[test]
fn tone_contaminated_reference_map_remains_unavailable_in_offline_report() {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("run");
    retained_run(&run, 0.4, false);
    add_reference(
        &run,
        "aligned_scene",
        json!({"color_space":"sRGB","cctf_decoding":true}),
        "brightness_only",
    );
    let out = temp.path().join("comparison");
    let (_, code) = compare::compare_single(&run, "reference", &out).unwrap();
    assert_eq!(code, 0);
    let manifest = documents::load_json_document(&out.join("comparison.json")).unwrap();
    let difference = &manifest["cases"][0]["pairs"][0]["review"]["difference"];
    assert_eq!(difference["availability"], "unavailable");
    assert_eq!(difference["reason"], "tone_contaminated_reference");
    assert_local_artifact(&out, &manifest, difference);
    assert_eq!(
        manifest["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|artifact| artifact["path"] == difference["path"])
            .unwrap()["availability"],
        "unavailable"
    );
    fs::remove_dir_all(&run).unwrap();
    let report_root = temp.path().join("report");
    report::build_report(&out, &report_root).unwrap();
    fs::remove_dir_all(&out).unwrap();
    let report_manifest = documents::load_json_document(&report_root.join("report.json")).unwrap();
    let retained = report_manifest["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["path"] == difference["path"])
        .unwrap();
    assert_eq!(retained["availability"], "unavailable");
    assert_eq!(retained["reason"], "tone_contaminated_reference");
    assert_local_artifact(&report_root, &report_manifest, retained);
}

#[test]
fn unavailable_findings_across_seeds_keep_distinct_identifiers() {
    let temp = tempfile::tempdir().unwrap();
    let run_root = temp.path().join("run");
    retained_run(&run_root, 0.4, false);
    let mut suite = documents::load_json_document(&run_root.join("suite.json")).unwrap();
    suite["cases"][0]["seeds"] = json!([0, 1]);
    suite["cases"][0]["measurements"] = json!(["numeric", "edge"]);
    write(&run_root.join("suite.json"), &suite);
    let mut run = documents::load_json_document(&run_root.join("run.json")).unwrap();
    let hash = json!(common::sha256_file(&run_root.join("suite.json")).unwrap());
    run["suite"]["sha256"] = hash.clone();
    run["suite"]["digest"] = json!(common::digest(&suite).unwrap());
    for artifact in run["artifacts"].as_array_mut().unwrap() {
        if artifact["path"] == "suite.json" {
            artifact["sha256"] = hash.clone();
        }
    }
    let mut second = run["cases"][0].clone();
    second["seed"] = json!(1);
    run["cases"].as_array_mut().unwrap().push(second);
    save_manifest(&run_root, "run.json", run);
    let out = temp.path().join("comparison");
    let (_, code) = compare::compare_single(&run_root, "input", &out).unwrap();
    assert_eq!(code, 0);
    let manifest = documents::load_json_document(&out.join("comparison.json")).unwrap();
    let findings = manifest["findings"].as_array().unwrap();
    assert!(findings.len() >= 2);
    let ids: std::collections::BTreeSet<_> = findings
        .iter()
        .map(|finding| finding["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), findings.len());
    for finding in findings {
        assert!(!finding["evidence"].as_array().unwrap().is_empty());
    }
}

#[test]
fn unknown_retained_case_returns_structured_error_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let run_root = temp.path().join("run");
    retained_run(&run_root, 0.4, false);
    let mut run = documents::load_json_document(&run_root.join("run.json")).unwrap();
    run["cases"][0]["case_id"] = json!("unknown");
    save_manifest(&run_root, "run.json", run);
    let out = temp.path().join("comparison");
    let error = compare::compare_single(&run_root, "input", &out).unwrap_err();
    assert_eq!(error.code, "run_selection");
    assert!(!out.exists());
}

#[test]
fn report_rejects_reference_scope_and_alignment_contradictions() {
    let temp = tempfile::tempdir().unwrap();
    let run_root = temp.path().join("run");
    retained_run(&run_root, 0.4, false);
    add_reference(
        &run_root,
        "visual",
        json!({"color_space":"sRGB","cctf_decoding":true}),
        "none",
    );
    let out = temp.path().join("comparison");
    compare::compare_single(&run_root, "reference", &out).unwrap();
    let original = documents::load_json_document(&out.join("comparison.json")).unwrap();
    for (index, expected) in ["comparison_correspondence", "measurement_applicability"]
        .into_iter()
        .enumerate()
    {
        let mut manifest = original.clone();
        let pair = &mut manifest["cases"][0]["pairs"][0];
        if index == 0 {
            pair["pixel_aligned"] = json!(true);
        } else {
            let measurement = &mut pair["measurements"][0];
            measurement["availability"] = json!("available");
            measurement["value"] = json!(1.0);
            measurement.as_object_mut().unwrap().remove("reason");
        }
        save_manifest(&out, "comparison.json", manifest);
        let report_root = temp.path().join(format!("rejected-report-{index}"));
        let error = report::build_report(&out, &report_root).unwrap_err();
        assert_eq!(error.code, expected);
    }
}
