//! Validation of retained run manifests: completed source evidence is
//! unconditional, requested stage bundles are validated and retained, and the
//! manifest digest stays intact.
#![cfg(feature = "precision-f64")]
use serde_json::{Value, json};
use spektrafilm_benchmark::common::{self, Evidence};
use spektrafilm_benchmark::documents;
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

#[path = "support/fixtures.rs"]
mod fixtures;
use fixtures::{record, save_manifest};

fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn validate_run(root: &Path) -> (Value, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
        .args(["validate", "--kind", "run", "--file"])
        .arg(root.join("run.json"))
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    (value, output.status.code().unwrap())
}

/// Six 4x4x3 f64-LE stage buffers and the shared bundle manifest.
fn stage_bundle(bundle: &Path) -> Value {
    fs::create_dir_all(bundle).unwrap();
    let mut stages = Vec::new();
    for (id, order, domain, units, space, color) in [
        (
            "rgb_in",
            "RGB",
            "input_rgb",
            "relative",
            "source_pixels",
            Some(json!({"color_space":"sRGB","cctf_decoding":false})),
        ),
        (
            "log_e_film",
            "CMY",
            "record_exposure",
            "log10",
            "film_frame_pixels",
            None,
        ),
        (
            "cmy_film",
            "CMY",
            "record_density",
            "optical_density",
            "film_frame_pixels",
            None,
        ),
        (
            "log_e_print",
            "CMY",
            "record_exposure",
            "log10",
            "film_frame_pixels",
            None,
        ),
        (
            "cmy_print",
            "CMY",
            "record_density",
            "optical_density",
            "film_frame_pixels",
            None,
        ),
        (
            "rgb_out",
            "RGB",
            "output_rgb",
            "relative",
            "output_pixels",
            Some(json!({"color_space":"sRGB","cctf_decoding":true})),
        ),
    ] {
        let path = bundle.join(format!("{id}.f64"));
        let bytes: Vec<u8> = (0..48).flat_map(|_| 0.25f64.to_le_bytes()).collect();
        fs::write(&path, bytes).unwrap();
        let mut record = json!({
            "id": id,
            "availability": "available",
            "path": format!("{id}.f64"),
            "sha256": common::sha256_file(&path).unwrap(),
            "layout": {"width":4,"height":4,"channels":3,"dtype":"f64","endianness":"little","channel_order":order},
            "domain": domain,
            "units": units,
            "geometry": {"coordinate_space": space},
        });
        if let Some(color) = color {
            record["color"] = color;
        }
        stages.push(record);
    }
    let document = json!({
        "schema_version": 1,
        "execution_status": "completed",
        "host_precision": "f64",
        "workflow_route": "print",
        "scan_output": "negative_to_positive",
        "normalization_mode": "profile_prepared",
        "exposure": {"availability":"available","auto_enabled":true,"method":"center_weighted","source_dimensions":[4,4],"metered_ev":0.5,"metered_ev_source":"measured","compensation_ev":0.25,"applied_ev":0.75,"gain":1.681792830507429},
        "print_normalization": {"availability":"available","value":1.0,"mode":"profile_prepared"},
        "terminal_cmyk": {"availability":"not_applicable","reason":"cmyk_not_active"},
        "stages": stages,
        "errors": [],
    });
    fixtures::write(&bundle.join("stage-evidence.json"), &document);
    document
}

fn complete_run(root: &Path, capture: bool) {
    fs::create_dir_all(root).unwrap();
    let mut evidence = Evidence::new(root);
    let input = root.join("input.png");
    fs::write(&input, b"retained declaration input").unwrap();
    let input = evidence
        .add(&input, "source_input", "image/png", json!({}))
        .unwrap();
    let suite = json!({"schema_version":1,"id":"validation-suite","protocol":{"id":"appearance-v1","pixels_per_degree":60.0,"region_sampling_scales":[1.0],"grain":{"residual":{"method":"flat_mean","high_pass_radius_px":1.0,"border_px":0},"spectrum":{"window":"hann"}},"hotspots":{"count":0,"min_area_px":1}},"cases":[{"id":"probe","kind":"probe","input":{"kind":"raster","path":"input.png","sha256":input["sha256"],"color_space":"sRGB","cctf_decoding":true},"configuration":{},"seeds":[0],"regions":[],"measurements":[]}]});
    let mut suite_record = record(&mut evidence, "suite.json", "suite", suite.clone());
    suite_record["digest"] = json!(common::digest(&suite).unwrap());
    let target_record = record(
        &mut evidence,
        "target.json",
        "target",
        json!({"schema_version":1,"id":"validation-target","renderer":{"path":"renderer","sha256":"a".repeat(64)},"data_dir":"data","backend":"cpu","changes":{"set_files":[],"set":[]}}),
    );
    let resolved = record(
        &mut evidence,
        "resolved.json",
        "resolved_configuration",
        json!({"parameters":{"camera":{"auto_exposure":false,"auto_exposure_method":"center_weighted","exposure_compensation_ev":0.0},"enlarger":{"print_exposure":1.0},"settings":{"print_exposure_ev_tracking":"never"},"io":{"input_color_space":"sRGB","input_cctf_decoding":true,"output_color_space":"sRGB","output_cctf_encoding":false}}}),
    );
    let diagnostics = record(
        &mut evidence,
        "diagnostics.json",
        "render_diagnostics",
        json!({"configuration":{"published_dimensions":[4,4]},"execution":{"backend_selected":"cpu","path":"cpu"}}),
    );
    let color = json!({"color_space":"sRGB","cctf_decoding":false});
    let layout = json!({"width":4,"height":4,"channels":3,"dtype":"f64","channel_order":"RGB","endianness":"little"});
    fs::write(
        root.join("master.f64"),
        (0..48)
            .flat_map(|_| 0.5f64.to_le_bytes())
            .collect::<Vec<u8>>(),
    )
    .unwrap();
    let raw = evidence
        .add(
            &root.join("master.f64"),
            "native_master",
            "application/octet-stream",
            json!({"layout":layout,"color":color}),
        )
        .unwrap();
    let raw_metadata = record(
        &mut evidence,
        "master.f64.json",
        "raw_metadata",
        json!({"layout":layout,"color":color}),
    );
    fs::write(root.join("master.tif"), b"floating master").unwrap();
    let master = evidence
        .add(
            &root.join("master.tif"),
            "floating_master",
            "image/tiff",
            json!({"dimensions":[4,4],"color":color}),
        )
        .unwrap();
    fs::write(root.join("stdout.txt"), b"output").unwrap();
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
    fs::write(
        root.join("source.f64"),
        (0..48)
            .flat_map(|_| 0.25f64.to_le_bytes())
            .collect::<Vec<u8>>(),
    )
    .unwrap();
    let source_raw = evidence
        .add(
            &root.join("source.f64"),
            "source_native_master",
            "application/octet-stream",
            json!({"layout":layout,"color":source_color}),
        )
        .unwrap();
    let source_raw_metadata = record(
        &mut evidence,
        "source.f64.json",
        "source_raw_metadata",
        json!({"width":4,"height":4,"channels":3,"dtype":"f64","channel_order":"RGB","endianness":"little","color_space":"ACES2065-1","cctf_decoding":false}),
    );
    let source_resolved = record(
        &mut evidence,
        "source-resolved.json",
        "source_configuration",
        json!({"parameters":{"io":{"output_color_space":"ACES2065-1","output_cctf_encoding":false}}}),
    );
    let source_diagnostics = record(
        &mut evidence,
        "source-diagnostics.json",
        "source_diagnostics",
        json!({"configuration":{"input_dimensions":[4,4],"published_dimensions":[4,4]}}),
    );
    fs::write(root.join("source.tif"), b"source master").unwrap();
    let source_master = evidence
        .add(
            &root.join("source.tif"),
            "source_floating_master",
            "image/tiff",
            json!({"dimensions":[4,4],"color":source_color}),
        )
        .unwrap();
    let mut row = json!({"case_id":"probe","seed":0,"status":"completed","environment":{},"command":["renderer","process","input.png","--output","master.tif","--raw-out","master.f64","--diagnostics","diagnostics.json","--seed","0"],"source_command":["renderer","process","input.png","--route","input","--output","source.tif","--raw-out","source.f64","--diagnostics","source-diagnostics.json","--seed","0"],"input":input,"resolved":resolved,"stdout":stdout,"stderr":stderr,"raw":raw,"raw_metadata":raw_metadata,"master":master,"diagnostics":diagnostics,"dimensions":[4,4],"source_raw":source_raw,"source_raw_metadata":source_raw_metadata,"source_resolved":source_resolved,"source_diagnostics":source_diagnostics,"source_master":source_master,"source_dimensions":[4,4],"source_geometry":{"published_dimensions":[4,4]},"simulation_geometry":{"published_dimensions":[4,4]},"source_pixel_aligned":true,"execution":{"backend_selected":"cpu","path":"cpu"},"precision":{"host_precision":"f64"}});
    if capture {
        let document = stage_bundle(&root.join("cases/probe/seed-0/stage-evidence"));
        let mut context = json!({});
        for key in [
            "host_precision",
            "workflow_route",
            "scan_output",
            "normalization_mode",
            "exposure",
            "print_normalization",
            "terminal_cmyk",
            "stages",
            "errors",
        ] {
            context[key] = document[key].clone();
        }
        let manifest = evidence
            .add(
                &root.join("cases/probe/seed-0/stage-evidence/stage-evidence.json"),
                "stage_evidence_manifest",
                "application/json",
                json!({"context":context}),
            )
            .unwrap();
        let mut buffers = Vec::new();
        for stage in document["stages"].as_array().unwrap() {
            let mut metadata = json!({"stage_id":stage["id"],"layout":stage["layout"],"bundle_path":stage["path"],"domain":stage["domain"],"units":stage["units"],"geometry":stage["geometry"]});
            if let Some(color) = stage.get("color") {
                metadata["color"] = color.clone();
            }
            buffers.push(
                evidence
                    .add(
                        &root
                            .join("cases/probe/seed-0/stage-evidence")
                            .join(stage["path"].as_str().unwrap()),
                        "stage_evidence_buffer",
                        "application/octet-stream",
                        metadata,
                    )
                    .unwrap(),
            );
        }
        row["stage_evidence"] = json!({"requested":true,"availability":"available","manifest":manifest,"buffers":buffers,"context":context});
    }
    save_manifest(
        root,
        "run.json",
        json!({"schema_version":1,"kind":"run","id":"validation-run","execution_status":"completed","suite":suite_record,"target":target_record,"protocol":suite["protocol"],"selection":{"case_ids":["probe"],"full_suite":true},"dependencies":[],"cases":[row],"artifacts":evidence.values(),"environment":{}}),
    )
}

#[test]
fn complete_retained_run_with_and_without_capture_validates() {
    for capture in [false, true] {
        let temp = TempDir::new().unwrap();
        complete_run(temp.path(), capture);
        let (result, code) = validate_run(temp.path());
        assert_eq!(code, 0, "{result}");
        assert_eq!(result["execution_status"], "completed");
    }
}

#[test]
fn sample_reads_the_exact_stage_from_a_retained_run_directory_or_manifest() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    complete_run(root, true);
    let regions = root.join("regions.json");
    fs::write(
        &regions,
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "id": "one-pixel",
            "regions": [{
                "id": "p00",
                "role": "other",
                "measurements": [],
                "geometry": {"x": 0, "y": 0, "width": 1, "height": 1, "space": "source"}
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    for (run_argument, output_name) in [
        (root.to_path_buf(), "sample-directory"),
        (root.join("run.json"), "sample-manifest"),
    ] {
        let output = root.join(output_name);
        let result = Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
            .args(["sample", "--from-run"])
            .arg(&run_argument)
            .args([
                "--case",
                "probe",
                "--seed",
                "0",
                "--stage",
                "rgb_in",
                "--regions",
            ])
            .arg(&regions)
            .args(["--out"])
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let sample: Value =
            serde_json::from_slice(&fs::read(output.join("run.json")).unwrap()).unwrap();
        assert_eq!(sample["source"]["kind"], "run");
        assert_eq!(sample["source"]["stage"], "rgb_in");
        assert_eq!(sample["source"]["coordinate_space"], "source_pixels");
        assert_eq!(sample["stages"][0]["id"], "rgb_in");
        assert_eq!(sample["stages"][0]["domain"], "input_rgb");
        assert_eq!(sample["stages"][0]["units"], "relative");
        assert_eq!(sample["stages"][0]["channel_order"], "RGB");
        assert_eq!(sample["summaries"][0]["count"], 1);
    }
}

#[test]
fn validating_a_retained_run_lists_the_checks_it_did_not_perform() {
    let temp = TempDir::new().unwrap();
    complete_run(temp.path(), false);
    let (result, code) = validate_run(temp.path());
    assert_eq!(code, 0, "{result}");
    // The record was produced by execution. Validating its shape decides
    // nothing about whether its images decode, its buffers match the device
    // that produced them, or its measurements hold on re-execution, so every
    // such check must be named rather than left to implication.
    let unchecked = result["unchecked"].as_array().unwrap();
    for check in [
        "image_decoding",
        "rendering",
        "compute_device",
        "measurement",
    ] {
        assert!(
            unchecked.iter().any(|v| v == check),
            "{check} must be listed as unchecked: {result}"
        );
    }
}

#[test]
fn missing_source_evidence_fails_semantically_with_recomputed_identity() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    complete_run(root, false);
    let mut run = read(&root.join("run.json"));
    run["cases"][0]
        .as_object_mut()
        .unwrap()
        .remove("source_raw");
    run["cases"][0]
        .as_object_mut()
        .unwrap()
        .remove("source_raw_metadata");
    // The reproduction recomputes the manifest digest, so the failure below is
    // semantic completeness and not identity protection.
    save_manifest(root, "run.json", run);
    let (result, code) = validate_run(root);
    assert_eq!(code, 1, "{result}");
    assert_eq!(result["execution_status"], "failed");
    let error = result["errors"][0]["code"].as_str().unwrap();
    assert!(
        ["source_evidence", "missing_field"].contains(&error),
        "{result}"
    );
}

#[test]
fn manifest_digest_still_rejects_changed_contents() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    complete_run(root, false);
    let mut run = read(&root.join("run.json"));
    run["cases"][0]["dimensions"] = json!([5, 5]);
    fixtures::write(&root.join("run.json"), &run);
    let (result, code) = validate_run(root);
    assert_eq!(code, 1, "{result}");
    assert_eq!(result["errors"][0]["code"], "manifest_identity");
}

#[test]
fn missing_stage_bundle_manifest_cannot_pass_silently() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    complete_run(root, true);
    fs::remove_file(root.join("cases/probe/seed-0/stage-evidence/stage-evidence.json")).unwrap();
    let (result, code) = validate_run(root);
    assert_eq!(code, 1, "{result}");
    assert_eq!(result["errors"][0]["code"], "missing_artifact");
}

#[test]
fn duplicate_retained_stage_cannot_replace_missing_stage() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    complete_run(root, true);
    let mut run = read(&root.join("run.json"));
    let buffers = run["cases"][0]["stage_evidence"]["buffers"]
        .as_array_mut()
        .unwrap();
    buffers[1] = buffers[0].clone();
    save_manifest(root, "run.json", run);
    let (result, code) = validate_run(root);
    assert_eq!(code, 1, "{result}");
    assert_eq!(result["errors"][0]["code"], "stage_evidence");
}

#[test]
fn malformed_stage_bundles_are_rejected_by_the_bundle_validator() {
    let temp = TempDir::new().unwrap();
    let bundle = temp.path().join("stage-evidence");
    stage_bundle(&bundle);
    documents::validate_stage_bundle(&bundle).unwrap();
    // A buffer with a non-finite sample fails even when every hash matches.
    let corrupt = bundle.join("rgb_in.f64");
    fs::write(
        &corrupt,
        (0..48)
            .flat_map(|index| {
                if index == 0 {
                    f64::NAN.to_le_bytes()
                } else {
                    0.25f64.to_le_bytes()
                }
            })
            .collect::<Vec<u8>>(),
    )
    .unwrap();
    let mut document = read(&bundle.join("stage-evidence.json"));
    document["stages"][0]["sha256"] = json!(common::sha256_file(&corrupt).unwrap());
    fixtures::write(&bundle.join("stage-evidence.json"), &document);
    let error = documents::validate_stage_bundle(&bundle).unwrap_err();
    assert_eq!(error.code, "stage_evidence");
    // A hash disagreement fails without any sample scan.
    document["stages"][0]["sha256"] = json!("b".repeat(64));
    fixtures::write(&bundle.join("stage-evidence.json"), &document);
    let error = documents::validate_stage_bundle(&bundle).unwrap_err();
    assert_eq!(error.code, "stage_evidence");
}

#[test]
fn failed_required_capture_is_not_complete_evidence() {
    let temp = TempDir::new().unwrap();
    let bundle = temp.path().join("stage-evidence");
    let mut document = stage_bundle(&bundle);
    document["stages"][1]["availability"] = json!("unavailable");
    document["stages"][1]["reason"] = json!("buffer_write_failed");
    for key in ["path", "sha256", "layout", "domain", "units", "geometry"] {
        document["stages"][1].as_object_mut().unwrap().remove(key);
    }
    fixtures::write(&bundle.join("stage-evidence.json"), &document);
    let validated = documents::validate_stage_bundle(&bundle).unwrap();
    assert_eq!(validated.availability, "unavailable");
    // A completed run row may never retain that as complete evidence.
    let root = temp.path().join("run");
    complete_run(&root, true);
    let mut run = read(&root.join("run.json"));
    run["cases"][0]["stage_evidence"]["availability"] = json!("unavailable");
    run["cases"][0]["stage_evidence"]["reason"] = json!("incomplete stage capture: log_e_film");
    save_manifest(&root, "run.json", run);
    let (result, code) = validate_run(&root);
    assert_eq!(code, 1, "{result}");
    assert_eq!(result["errors"][0]["code"], "stage_evidence");
}
