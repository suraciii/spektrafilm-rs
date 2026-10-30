use serde_json::{Value, json};
use spektrafilm_core::presets::PRESET_SCHEMA_VERSION;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture {
    root: PathBuf,
    input: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "spektrafilm-preset-cli-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let input = root.join("input.png");
        image::save_buffer(
            &input,
            &[40_u8, 90, 170].repeat(16),
            4,
            4,
            image::ColorType::Rgb8,
        )
        .unwrap();
        Self {
            root,
            input,
            data: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"),
        }
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_spektrafilm"))
            .args(args)
            .env("RUST_LOG", "off")
            .env("SPEKTRAFILM_DATA_DIR", &self.data)
            .env("SPEKTRAFILM_CONFIG_DIR", self.root.join("config"))
            .env_remove("SPEKTRAFILM_BACKEND")
            .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
            .output()
            .unwrap()
    }
    #[cfg(feature = "precision-f64")]
    fn run_lab(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
            .args(args)
            .env("RUST_LOG", "off")
            .env("SPEKTRAFILM_DATA_DIR", &self.data)
            .env("SPEKTRAFILM_CONFIG_DIR", self.root.join("config"))
            .env_remove("SPEKTRAFILM_BACKEND")
            .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
            .output()
            .unwrap()
    }
    fn process(&self, extra: &[&str]) -> Output {
        let output = self.root.join("output.tif");
        let mut args = vec![
            "process",
            self.input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ];
        // Preserve this synthetic PNG fixture's historical linear interpretation.
        // Declaration-specific cases and RAW input must supply their own contract.
        if self.input.extension().and_then(|ext| ext.to_str()) == Some("png")
            && !extra.iter().any(|arg| {
                arg.contains("input_color_space")
                    || arg.contains("input_cctf_decoding")
                    || arg.starts_with("--input-color-space")
                    || arg.starts_with("--input-cctf-decoding")
            })
        {
            args.extend_from_slice(&[
                "--input-color-space",
                "ProPhoto RGB",
                "--input-cctf-decoding",
                "false",
            ]);
        }
        args.extend_from_slice(extra);
        self.run(&args)
    }
    fn report(&self, extra: &[&str]) -> Value {
        let output = self.process(extra);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn capture_illuminant_reports_inheritance_overrides_clearing_and_inactive_routes() {
    let f = Fixture::new();
    let inherited = f.report(&["--film", "kodak_portra_400", "--dry-run"]);
    assert_eq!(inherited["capture_illuminant"]["requested"], Value::Null);
    assert_eq!(inherited["capture_illuminant"]["effective"], "D55");
    assert_eq!(inherited["capture_illuminant"]["source"], "unset");
    assert_eq!(inherited["capture_illuminant"]["status"], "active");

    let explicit = f.report(&[
        "--film",
        "kodak_portra_400",
        "--set",
        "camera.capture_illuminant=BB4000",
        "--dry-run",
    ]);
    assert_eq!(explicit["capture_illuminant"]["requested"], "BB4000");
    assert_eq!(explicit["capture_illuminant"]["effective"], "BB4000");
    assert_eq!(explicit["capture_illuminant"]["source"], "set");
    assert!(
        explicit["edits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edit| edit["path"] == "camera.capture_illuminant")
    );

    let sparse = f.root.join("capture.toml");
    std::fs::write(&sparse, "[camera]\ncapture_illuminant = \"T\"\n").unwrap();
    let from_file = f.report(&[
        "--film",
        "kodak_portra_400",
        "--set",
        sparse.to_str().unwrap(),
        "--dry-run",
    ]);
    assert_eq!(from_file["capture_illuminant"]["requested"], "T");
    assert_eq!(from_file["capture_illuminant"]["effective"], "T");
    assert_eq!(from_file["capture_illuminant"]["source"], "set");

    let cleared = f.report(&[
        "--film",
        "kodak_portra_400",
        "--set",
        "camera.capture_illuminant=BB4000",
        "--set",
        "camera.capture_illuminant=null",
        "--dry-run",
    ]);
    assert_eq!(cleared["capture_illuminant"]["requested"], Value::Null);
    assert_eq!(cleared["capture_illuminant"]["effective"], "D55");
    assert_eq!(cleared["capture_illuminant"]["source"], "unset");

    let inactive = f.report(&[
        "--film",
        "kodak_portra_400",
        "--route",
        "input",
        "--set",
        "camera.capture_illuminant=BB4000,film_render.grain.active=false",
        "--dry-run",
    ]);
    assert_eq!(inactive["capture_illuminant"]["status"], "inactive");
    assert_eq!(inactive["capture_illuminant"]["effective"], Value::Null);
    assert_eq!(
        inactive["capture_illuminant"]["profile_reference_illuminant"],
        "D55"
    );
    assert!(inactive["capture_illuminant"]["reason"].is_string());
}

#[test]
fn output_mix_edits_are_discoverable_and_invalid_sources_preserve_outputs() {
    let f = Fixture::new();
    std::fs::write(f.root.join("output.tif"), b"retained-image").unwrap();
    let report = f.report(&[
        "--film",
        "kodak_gold_200",
        "--set",
        "output_mix.amount=0.25",
        "--dry-run",
    ]);
    assert_eq!(report["parameters"]["output_mix"]["amount"], 0.25);
    for edit in [
        "output_mix.amount=-0.01",
        "output_mix.amount=1.01",
        "io.output_gamut_compress.lightness_compression=null",
        "film_render.response.amount=1",
        "print_render.response.amount=1",
        "enlarger.c_filter_shift=0",
    ] {
        let result = f.process(&["--film", "kodak_gold_200", "--set", edit, "--dry-run"]);
        assert!(!result.status.success(), "{edit}");
        assert_eq!(
            std::fs::read(f.root.join("output.tif")).unwrap(),
            b"retained-image"
        );
    }
}

#[test]
fn scanner_operator_cli_discovery_rendering_and_rejection_share_the_contract() {
    let f = Fixture::new();
    let discovery = f.run(&[
        "describe",
        "--module",
        "scanner.operator",
        "--format",
        "json",
    ]);
    assert!(
        discovery.status.success(),
        "{}",
        String::from_utf8_lossy(&discovery.stderr)
    );
    let fields: Value = serde_json::from_slice(&discovery.stdout).unwrap();
    for (name, default) in [
        ("exposure_ev", 0.0),
        ("warmth_ev", 0.0),
        ("tint_ev", 0.0),
        ("contrast", 1.0),
    ] {
        let path = format!("scanner.operator.{name}");
        let field = fields["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["path"] == path)
            .unwrap();
        assert_eq!(field["default"], default);
    }
    let common = [
        "--film",
        "kodak_portra_400",
        "--backend",
        "cpu",
        "--scan-film",
        "--scan-output",
        "negative_to_positive",
        "--set",
        "camera.auto_exposure=false,debug.deactivate_spatial_effects=true,debug.deactivate_stochastic_effects=true",
    ];
    let baseline = f.process(&common);
    assert!(
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline.stderr)
    );
    let baseline = image::open(f.root.join("output.tif")).unwrap().to_rgb32f();
    let mut corrected = common.to_vec();
    corrected.extend_from_slice(&["--set", "scanner.operator.exposure_ev=1"]);
    let result = f.process(&corrected);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let positive = image::open(f.root.join("output.tif")).unwrap().to_rgb32f();
    assert_eq!(positive.dimensions(), baseline.dimensions());
    assert!(positive.as_raw().iter().sum::<f32>() > baseline.as_raw().iter().sum::<f32>());
    let sentinel = std::fs::read(f.root.join("output.tif")).unwrap();
    for edit in [
        "scanner.operator.contrast=0",
        "scanner.operator.contrast=-1",
        "scanner.operator.unknown=0",
        "scanner.auto_setup=full",
        "scanner.finish=lab_scan",
        "scanner.conversion_exposure_ev=0",
        "scanner.conversion_style=fixed,scanner.operator.warmth_ev=0.2",
    ] {
        let mut args = common.to_vec();
        args.extend_from_slice(&["--set", edit, "--dry-run"]);
        assert!(!f.process(&args).status.success(), "{edit}");
        assert_eq!(std::fs::read(f.root.join("output.tif")).unwrap(), sentinel);
    }
    assert!(
        !f.process(&[
            "--film",
            "kodak_portra_400",
            "--scan-film",
            "--set",
            "scanner.operator.exposure_ev=0.5",
            "--dry-run"
        ])
        .status
        .success()
    );
    let report = f.report(&[
        "--film", "kodak_portra_400", "--scan-film", "--scan-output", "negative_to_positive",
        "--set", "scanner.operator.exposure_ev=0.5000000000000001,scanner.operator.warmth_ev=-0.25,scanner.operator.tint_ev=0.125,scanner.operator.contrast=1.2", "--dry-run",
    ]);
    assert_eq!(
        report["parameters"]["scanner"]["operator"]["exposure_ev"],
        0.5000000000000001
    );
}

#[test]
fn sources_compose_in_order_and_dry_run_preserves_existing_outputs() {
    let f = Fixture::new();
    let sparse = f.root.join("edits.toml");
    std::fs::write(&sparse, "[camera]\nexposure_compensation_ev = 1.0\n[film_render.grain]\nrms_granularity = [1.0,2.0,3.0]\nprocedural_amount = 25.0\n[enlarger]\ny_filter_neutral = 12.0\n").unwrap();
    let raw = f.root.join("buffer.raw");
    std::fs::write(f.root.join("output.tif"), b"existing-image").unwrap();
    std::fs::write(&raw, b"existing-buffer").unwrap();
    let report = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--set",
        "camera.exposure_compensation_ev=-1",
        "--set",
        sparse.to_str().unwrap(),
        "--set",
        "camera.exposure_compensation_ev=0.5,film_render.grain.procedural_amount=null",
        "--dry-run",
        "--backend",
        "gpu",
        "--raw-out",
        raw.to_str().unwrap(),
    ]);
    assert_eq!(report["film_profile"], "kodak_portra_400");
    assert_eq!(report["print_profile"], "kodak_portra_endura");
    assert_eq!(
        report["parameters"]["camera"]["exposure_compensation_ev"],
        0.5
    );
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["rms_granularity"],
        json!([1.0, 2.0, 3.0])
    );
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["procedural_amount"],
        Value::Null
    );
    assert_eq!(report["parameters"]["enlarger"]["y_filter_neutral"], 12.0);
    assert_eq!(
        report["parameters"]["settings"]["neutral_print_filters_from_database"],
        true
    );
    assert_eq!(
        std::fs::read(f.root.join("output.tif")).unwrap(),
        b"existing-image"
    );
    assert_eq!(std::fs::read(raw).unwrap(), b"existing-buffer");
}

#[test]
fn preset_discovery_exports_both_carriers_and_reloads_same_parameters() {
    let f = Fixture::new();
    let list = f.run(&["preset", "list"]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let listed: Value = serde_json::from_slice(&list.stdout).unwrap();
    let portra = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "Kodak Portra 400")
        .unwrap();
    assert_eq!(portra["film_profile"]["stock"], "kodak_portra_400");
    assert_eq!(portra["print_profile"]["stock"], "kodak_portra_endura");
    let mut reports = Vec::new();
    for format in ["json", "toml"] {
        let shown = f.run(&["preset", "show", "Kodak Portra 400", "--format", format]);
        assert!(
            shown.status.success(),
            "{}",
            String::from_utf8_lossy(&shown.stderr)
        );
        let path = f.root.join(format!("portable.{format}"));
        std::fs::write(&path, &shown.stdout).unwrap();
        let before = std::fs::read(&path).unwrap();
        reports.push(f.report(&["--preset", path.to_str().unwrap(), "--dry-run"]));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    assert_eq!(reports[0], reports[1]);
    let description = f.run(&[
        "describe",
        "--module",
        "film_render.grain",
        "--format",
        "json",
    ]);
    assert!(
        description.status.success(),
        "{}",
        String::from_utf8_lossy(&description.stderr)
    );
    let fields: Value = serde_json::from_slice(&description.stdout).unwrap();
    let amount = fields["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["path"] == "film_render.grain.procedural_amount")
        .unwrap();
    assert_eq!(amount["nullable"], true);
    assert_eq!(amount["default"], Value::Null);
    assert_eq!(amount["minimum"], 0.0);
    assert_eq!(amount["maximum"], 100.0);
    assert!(!fields.to_string().contains("film_render.grain.monochrome"));
    assert!(!f.run(&["describe", "--module", "missing"]).status.success());
}

#[test]
fn dry_run_legacy_baseline_matches_original_digest() {
    use spektrafilm_core::{
        params::RuntimeParams,
        runtime::{self, DigestMode},
    };
    let f = Fixture::new();
    // Compare the complete parameter-file consumer with the runtime digest.
    let path = f.root.join("legacy.json");
    let wire = json!({"film_render":{"grain":{"rms_granularity":[9,8,7]}},"settings":{"preview_mode":true}});
    std::fs::write(&path, serde_json::to_vec(&wire).unwrap()).unwrap();
    let baseline: RuntimeParams = serde_json::from_value(wire).unwrap();
    let mut photo =
        runtime::init_params("kodak_portra_400", "kodak_portra_endura", &f.data).unwrap();
    photo.params = baseline;
    let expected = photo
        .digested_params(DigestMode::ApplyStockSpecifics)
        .unwrap();
    let report = f.report(&[
        "--film",
        "kodak_portra_400",
        "--paper",
        "kodak_portra_endura",
        "--params",
        path.to_str().unwrap(),
        "--dry-run",
    ]);
    let reported: RuntimeParams = serde_json::from_value(report["parameters"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(reported).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn inactive_chemistry_domains_agree_across_complete_and_sparse_sources() {
    let f = Fixture::new();
    let output = f.root.join("output.tif");
    std::fs::write(&output, b"accepted output").unwrap();
    for (group, feature) in [
        ("film_render", "chemistry"),
        ("print_render", "density_curves_morph"),
    ] {
        let path = format!("{group}.{feature}.developer_exhaustion");
        for value in [0.0, 0.5, 1.0, -0.1, 2.0] {
            let mut wire = json!({});
            wire[group] = json!({});
            wire[group][feature] = json!({"active": false, "developer_exhaustion": value});
            let json_path = f.root.join("chemistry.json");
            let toml_path = f.root.join("chemistry.toml");
            let json_text = serde_json::to_string(&wire).unwrap();
            std::fs::write(&json_path, &json_text).unwrap();
            std::fs::write(
                &toml_path,
                format!("[{group}.{feature}]\nactive = false\ndeveloper_exhaustion = {value:?}\n"),
            )
            .unwrap();
            let inline = format!("{group}.{feature}.active=false,{path}={value}");
            for (option, source) in [
                ("--params", json_path.to_str().unwrap()),
                ("--set", json_path.to_str().unwrap()),
                ("--set", toml_path.to_str().unwrap()),
                ("--set", inline.as_str()),
            ] {
                let mut args = vec![
                    "--film",
                    "kodak_portra_400",
                    "--paper",
                    "kodak_portra_endura",
                    "--route",
                    "input",
                    "--backend",
                    "cpu",
                    "--set",
                    "film_render.grain.active=false",
                    option,
                    source,
                ];
                let replacement = format!("{path}=0.5");
                if (0.0..=1.0).contains(&value) {
                    args.push("--dry-run");
                    let result = f.process(&args);
                    assert!(
                        result.status.success(),
                        "{}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
                    assert_eq!(report["parameters"][group][feature]["active"], false);
                    assert_eq!(
                        report["parameters"][group][feature]["developer_exhaustion"],
                        value
                    );
                } else {
                    // Neither a later valid source nor an absent operation hides invalid input.
                    args.extend(["--set", replacement.as_str()]);
                    let result = f.process(&args);
                    assert!(!result.status.success());
                    assert!(String::from_utf8_lossy(&result.stderr).contains(&path));
                }
                assert_eq!(std::fs::read(&output).unwrap(), b"accepted output");
                assert_eq!(std::fs::read_to_string(&json_path).unwrap(), json_text);
            }
        }
    }
}

#[test]
fn sparse_edits_protect_only_supplied_neutral_axes() {
    use spektrafilm_core::{neutral_filters::NeutralFilters, params::RuntimeParams, profile};
    let f = Fixture::new();
    let path = f.root.join("neutral.json");
    let wire =
        json!({"enlarger":{"y_filter_neutral":9.0,"m_filter_neutral":8.0,"c_filter_neutral":7.0}});
    std::fs::write(&path, serde_json::to_vec(&wire).unwrap()).unwrap();
    let baseline: RuntimeParams = serde_json::from_value(wire).unwrap();
    let report = f.report(&[
        "--film",
        "kodak_portra_400",
        "--paper",
        "kodak_portra_endura",
        "--params",
        path.to_str().unwrap(),
        "--set",
        "enlarger.y_filter_neutral=9,camera.exposure_compensation_ev=0.5",
        "--dry-run",
    ]);
    assert_eq!(report["parameters"]["enlarger"]["y_filter_neutral"], 9.0);
    let neutral = NeutralFilters::load(&f.data).unwrap();
    let film = profile::load_profile_by_name(&f.data, "kodak_portra_400").unwrap();
    let print = profile::load_profile_by_name(&f.data, "kodak_portra_endura").unwrap();
    let calibrated = spektrafilm_core::params_builder::apply_database_neutral_print_filters(
        baseline,
        &film,
        &print,
        Some(&neutral),
    );
    assert_eq!(
        report["parameters"]["enlarger"]["m_filter_neutral"],
        calibrated.enlarger.m_filter_neutral
    );
    assert_eq!(
        report["parameters"]["enlarger"]["c_filter_neutral"],
        calibrated.enlarger.c_filter_neutral
    );
    assert_eq!(
        report["parameters"]["settings"]["neutral_print_filters_from_database"],
        true
    );
}

#[test]
fn invalid_sources_fail_even_when_later_replaced_and_publish_nothing() {
    let f = Fixture::new();
    for source in [
        "film_render.grain.procedural_amount=101",
        "camera.exposure_compensation_ev=3.5e38",
        "settings.preview_max_size=1.5",
        "camera.color_filter=null",
        "workflow.route=\"input\"",
        "io.scan_film=true",
        "film_render.grain.monochrome=true",
        "film_render.grain.particle_area_um2=0.2",
        "film_render.grain.particle_scale=[1.6,1.6,3.2]",
        "film_render.grain.particle_scale_layers=[1,1,1]",
        "film_render.grain.sublayers_active=true",
        "film_render.grain.n_sub_layers=1",
        "export.bit_depth=16",
        "camera.auto_exposure=\"true\"",
        "camera={}",
        "camera.exposure_compensation_ev=1,",
        "camera.exposure_compensation_ev=1,,camera.auto_exposure=false",
    ] {
        let result = f.process(&[
            "--preset",
            "Kodak Portra 400",
            "--set",
            source,
            "--set",
            "film_render.grain.procedural_amount=25",
            "--dry-run",
        ]);
        assert!(
            !result.status.success(),
            "accepted invalid source: {source}"
        );
        assert!(!f.root.join("output.tif").exists());
    }
    for (name, contents) in [
        (
            "duplicate.json",
            "{\"camera\":{\"auto_exposure\":true,\"auto_exposure\":false}}",
        ),
        ("nonfinite.toml", "[scanner]\nlens_blur = nan"),
        ("date.toml", "[camera]\ncolor_filter = 2026-10-10"),
        ("group.json", "{\"scanner\":null}"),
        ("unknown-empty.json", "{\"unknown\":{}}"),
        (
            "wrapper.json",
            "{\"parameters\":{\"camera\":{\"auto_exposure\":true}}}",
        ),
        ("array-shape.json", "{\"camera\":{\"filter_uv\":[1,2]}}"),
    ] {
        let path = f.root.join(name);
        std::fs::write(&path, contents).unwrap();
        let result = f.process(&[
            "--preset",
            "Kodak Portra 400",
            "--set",
            path.to_str().unwrap(),
            "--dry-run",
        ]);
        assert!(!result.status.success(), "accepted invalid file: {name}");
    }
    assert!(
        !f.process(&["--preset", "missing.toml", "--dry-run"])
            .status
            .success()
    );
    assert!(
        !f.process(&["--preset", "classic-kodak-portra-400", "--dry-run"])
            .status
            .success()
    );
    assert!(
        !f.process(&[
            "--preset",
            "Kodak Portra 400",
            "--film",
            "kodak_portra_400",
            "--dry-run"
        ])
        .status
        .success()
    );
}

#[test]
fn route_and_scan_film_conflict_in_public_process() {
    let f = Fixture::new();
    for dry in [false, true] {
        let mut args = vec![
            "--preset",
            "Kodak Portra 400",
            "--scan-film",
            "--route",
            "input",
        ];
        if dry {
            args.push("--dry-run");
        }
        let output = f.process(&args);
        assert_eq!(output.status.code(), Some(2));
        let message = String::from_utf8_lossy(&output.stderr);
        assert!(
            message.contains("--route") && message.contains("--scan-film"),
            "{message}"
        );
        assert!(!f.root.join("output.tif").exists());
    }
}

#[test]
fn final_workflow_and_runtime_constraints_resolve_after_edits() {
    let f = Fixture::new();
    let report = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--route",
        "input",
        "--set",
        "settings.preview_mode=true,debug.lut_mode=true",
        "--set",
        "settings.preview_mode=false,debug.lut_mode=false,film_render.grain.active=true",
        "--dry-run",
    ]);
    assert_eq!(report["parameters"]["workflow"]["route"], "input");
    assert_eq!(report["parameters"]["io"]["scan_film"], false);
    assert_eq!(report["parameters"]["film_render"]["grain"]["active"], true);
    assert_eq!(report["print_profile"], "kodak_portra_endura");
    let report = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--set",
        "settings.preview_mode=true,film_render.grain.active=true",
        "--dry-run",
    ]);
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["active"],
        false
    );
}

#[test]
fn report_and_render_use_identical_resolved_controls() {
    use spektrafilm_backend::cpu_backend::CpuBackend;
    use spektrafilm_core::{params::RuntimeParams, profile, runtime::Runtime};
    let f = Fixture::new();
    let edits = "camera.auto_exposure=false,camera.exposure_compensation_ev=0.5,debug.deactivate_spatial_effects=true,debug.deactivate_stochastic_effects=true,enlarger.y_filter_neutral=12";
    let report = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--set",
        edits,
        "--backend",
        "cpu",
        "--dry-run",
    ]);
    let raw = f.root.join("render.raw");
    let output = f.process(&[
        "--preset",
        "Kodak Portra 400",
        "--set",
        edits,
        "--backend",
        "cpu",
        "--raw-out",
        raw.to_str().unwrap(),
        "--saving-color-space",
        "ProPhoto RGB",
        "--saving-cctf-encoding",
        "true",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut params: RuntimeParams = serde_json::from_value(report["parameters"].clone()).unwrap();
    // The report has resolved all neutral axes; disable a second database
    // lookup only in the independent expected render.
    params.settings.neutral_print_filters_from_database = false;
    let runtime = Runtime::new(
        profile::load_profile_by_name(&f.data, "kodak_portra_400").unwrap(),
        profile::load_profile_by_name(&f.data, "kodak_portra_endura").unwrap(),
        params,
        &f.data,
    )
    .unwrap();
    assert_eq!(runtime.params().enlarger.y_filter_neutral, 12.0);
    let expected = runtime
        .process(
            spektrafilm_core::image_io::load(&f.input).unwrap().image,
            &CpuBackend,
        )
        .unwrap();
    let rendered: Vec<f64> = std::fs::read(raw)
        .unwrap()
        .as_chunks::<8>()
        .0
        .iter()
        .map(|bytes| f64::from_ne_bytes(*bytes))
        .collect();
    // Replaying the published parameters through the real entry point must
    // reproduce the original render bit-identically: authored axes keep the
    // carried values and baseline axes re-derive the same built-in table.
    let params_file = f.root.join("params.json");
    std::fs::write(
        &params_file,
        serde_json::to_vec(&report["parameters"]).unwrap(),
    )
    .unwrap();
    let replay_raw = f.root.join("replay.raw");
    let replay = f.process(&[
        "--film",
        "kodak_portra_400",
        "--params",
        params_file.to_str().unwrap(),
        "--backend",
        "cpu",
        "--raw-out",
        replay_raw.to_str().unwrap(),
        "--saving-color-space",
        "ProPhoto RGB",
        "--saving-cctf-encoding",
        "true",
    ]);
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let replayed: Vec<f64> = std::fs::read(replay_raw)
        .unwrap()
        .as_chunks::<8>()
        .0
        .iter()
        .map(|bytes| f64::from_ne_bytes(*bytes))
        .collect();
    assert_eq!(
        replayed, rendered,
        "published parameters must replay the render bit-identically"
    );
    // The independent expected render consumes the published f32 views with
    // the second lookup disabled; the exposure normalization then misses the
    // f64 refinement of the built-in table values. That refinement is the
    // documented precision rule (authored axes exact, baseline axes refined
    // at f64 for Python parity), so the published-view render must agree
    // within the measured budget. Measured: 6.4e-10 absolute for this
    // fixture; 1e-8 keeps more than 10x headroom while still rejecting any
    // real control or resolution regression.
    let expected: Vec<f64> = expected.data.iter().map(|&v| v as f64).collect();
    assert_eq!(rendered.len(), expected.len());
    let max_diff = rendered
        .iter()
        .zip(expected.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f64, f64::max);
    assert!(
        max_diff <= 1e-8,
        "published-view render diverges from the render by {max_diff:e}"
    );
    assert!(f.root.join("output.tif").is_file());
}

#[test]
fn writer_options_validate_before_decoding_and_keep_scanner_color_independent() {
    let f = Fixture::new();
    std::fs::write(&f.input, b"invalid-pixels").unwrap();
    let report = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--set",
        "io.output_color_space=\"sRGB\"",
        "--saving-color-space",
        "ProPhoto RGB",
        "--saving-cctf-encoding",
        "false",
        "--bit-depth",
        "32",
        "--compression",
        "none",
        "--dry-run",
    ]);
    assert_eq!(report["parameters"]["io"]["output_color_space"], "sRGB");
    assert_eq!(report["output"]["color_space"], "ProPhoto RGB");
    assert_eq!(report["output"]["cctf_encoding"], false);
    assert_eq!(report["output"]["bit_depth"], 32);
    assert_eq!(report["output"]["compression"], "none");
    assert_eq!(report["output"]["jpeg_quality"], Value::Null);
    for extra in [
        vec!["--saving-color-space", "unknown-space"],
        vec!["--jpeg-quality", "95"],
        vec!["--format", "png"],
    ] {
        let mut args = vec!["--preset", "Kodak Portra 400", "--backend", "gpu"];
        args.extend(extra);
        let result = f.process(&args);
        assert!(!result.status.success());
        let message = String::from_utf8_lossy(&result.stderr);
        assert!(
            !message.contains("GPU backend unavailable"),
            "writer validation initialized GPU: {message}"
        );
        assert!(!f.root.join("output.tif").exists());
    }
}

#[test]
fn raw_dry_run_enforces_linear_aces_without_decoding() {
    let mut f = Fixture::new();
    f.input = f.root.join("input.CR2");
    std::fs::write(&f.input, b"raw-placeholder").unwrap();
    let report = f.report(&["--preset", "Kodak Portra 400", "--dry-run"]);
    assert_eq!(
        report["parameters"]["io"]["input_color_space"],
        "ACES2065-1"
    );
    assert_eq!(report["parameters"]["io"]["input_cctf_decoding"], false);
    for edits in [
        "io.input_color_space=\"sRGB\"",
        "io.input_cctf_decoding=true",
    ] {
        assert!(
            !f.process(&["--preset", "Kodak Portra 400", "--set", edits, "--dry-run"])
                .status
                .success()
        );
    }
}

#[test]
fn dry_run_rejects_missing_inputs_unknown_presets_and_duplicate_single_selectors() {
    let mut f = Fixture::new();
    let unknown = f.process(&[
        "--preset",
        "Kodak Portra 400",
        "--preset",
        "Definitely nonexistent preset",
        "--dry-run",
    ]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("Definitely nonexistent preset"));
    for options in [
        vec!["--film", "kodak_portra_400", "--film", "kodak_gold_200"],
        vec![
            "--preset",
            "Kodak Portra 400",
            "--route",
            "input",
            "--route",
            "input > film > scan",
        ],
    ] {
        let mut args = options;
        args.push("--dry-run");
        assert!(!f.process(&args).status.success());
    }
    f.input = f.root.join("missing.png");
    assert!(
        !f.process(&["--preset", "Kodak Portra 400", "--dry-run"])
            .status
            .success()
    );
    f.input = f.root.join("unsupported.txt");
    std::fs::write(&f.input, b"text").unwrap();
    assert!(
        !f.process(&["--preset", "Kodak Portra 400", "--dry-run"])
            .status
            .success()
    );
    f.input = f.root.clone();
    assert!(
        !f.process(&["--preset", "Kodak Portra 400", "--dry-run"])
            .status
            .success()
    );
    assert!(!f.root.join("output.tif").exists());
}

#[test]
fn film_scan_without_preset_keeps_legacy_print_selection() {
    let f = Fixture::new();
    let report = f.report(&[
        "--film",
        "kodak_portra_400",
        "--scan-film",
        "--set",
        "camera.auto_exposure=false",
        "--dry-run",
    ]);
    assert_eq!(report["film_profile"], "kodak_portra_400");
    assert_eq!(report["print_profile"], "kodak_portra_400");
    assert_eq!(
        report["parameters"]["workflow"]["route"],
        "input > film > scan"
    );
    assert_eq!(report["parameters"]["io"]["scan_film"], true);
}

#[test]
fn exr_writer_constraints_fail_before_decoding_or_device_selection() {
    let f = Fixture::new();
    std::fs::write(&f.input, b"invalid-pixels").unwrap();
    let output = f.root.join("output.exr");
    std::fs::write(&output, b"existing-image").unwrap();
    for flags in [
        vec!["--saving-color-space", "ProPhoto RGB"],
        vec!["--compression", "none"],
    ] {
        for dry in [false, true] {
            let mut args = vec![
                "process",
                f.input.to_str().unwrap(),
                "-o",
                output.to_str().unwrap(),
                "--input-color-space",
                "ProPhoto RGB",
                "--input-cctf-decoding",
                "false",
                "--preset",
                "Kodak Portra 400",
                "--backend",
                "gpu",
            ];
            args.extend_from_slice(&flags);
            if dry {
                args.push("--dry-run");
            }
            let result = f.run(&args);
            assert!(!result.status.success());
            let message = String::from_utf8_lossy(&result.stderr);
            assert!(message.contains("EXR"), "{message}");
            assert!(!message.contains("GPU backend unavailable"), "{message}");
            assert_eq!(std::fs::read(&output).unwrap(), b"existing-image");
        }
    }
}

#[test]
fn motion_picture_stock_default_print_remains_available_with_sources() {
    let f = Fixture::new();
    for film in ["kodak_vision3_250d", "kodak_vision3_500t"] {
        let report = f.report(&[
            "--film",
            film,
            "--set",
            "camera.auto_exposure=false",
            "--dry-run",
        ]);
        assert_eq!(report["print_profile"], "kodak_2383");
        assert_eq!(report["parameters"]["camera"]["auto_exposure"], false);
    }
}

#[test]
fn every_gamut_array_source_validates_before_later_overrides() {
    let f = Fixture::new();
    for path in [
        "io.input_gamut_compress.knee",
        "io.output_gamut_compress.knee",
    ] {
        let good = format!("{path}=[0.9,1,1]");
        for bad in ["[-1,1,1]", "[1,1,1]", "[0.9,0,1]", "[0.9,1,0]"] {
            let invalid = format!("{path}={bad}");
            let result = f.process(&[
                "--film",
                "kodak_portra_400",
                "--set",
                &invalid,
                "--set",
                &good,
                "--dry-run",
            ]);
            assert!(
                !result.status.success(),
                "accepted earlier invalid source {invalid}"
            );
            assert!(String::from_utf8_lossy(&result.stderr).contains(path));
        }
    }
    assert!(!f.root.join("output.tif").exists());
}

#[test]
fn dry_run_reports_runtime_route_and_monochrome_normalization() {
    let f = Fixture::new();
    let legacy = f.report(&[
        "--film",
        "kodak_portra_400",
        "--route",
        "input > film > scan",
        "--dry-run",
    ]);
    assert_eq!(legacy["parameters"]["io"]["scan_film"], true);
    let report = f.report(&["--film", "kodak_doublex", "--scan-film", "--set", "film_render.grain.density_min=[0.1,0.2,0.3],film_render.dir_couplers.gamma_samelayer_rgb=[0.4,0.5,0.6]", "--dry-run"]);
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["monochrome"],
        true
    );
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["density_min"][0],
        report["parameters"]["film_render"]["grain"]["density_min"][1]
    );
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["density_min"][1],
        report["parameters"]["film_render"]["grain"]["density_min"][2]
    );
    assert_eq!(
        report["parameters"]["film_render"]["dir_couplers"]["gamma_interlayer_r_to_gb"],
        json!([0.0, 0.0])
    );
    assert_eq!(report["print_profile"], "kodak_doublex");
}

#[test]
fn cmyk_enabled_preserves_preset_profiles_and_selects_direct_scan_without_preset() {
    let f = Fixture::new();
    for selector in [
        ["--film", "kodak_portra_400"],
        ["--preset", "Kodak Portra 400"],
    ] {
        let report = f.report(&[
            selector[0],
            selector[1],
            "--route",
            "input > film > scan",
            "--set",
            "cmyk.active=true,cmyk.strength=0.75",
            "--dry-run",
        ]);
        assert_eq!(report["parameters"]["io"]["scan_film"], true);
        assert_eq!(report["parameters"]["cmyk"]["strength"], 0.75);
        assert_eq!(
            report["print_profile"],
            if selector[0] == "--preset" {
                "kodak_portra_endura"
            } else {
                "kodak_portra_400"
            }
        );
    }
    let result = f.process(&[
        "--film",
        "kodak_portra_400",
        "--route",
        "input > film > scan",
        "--set",
        "cmyk.strength=1.1",
        "--set",
        "cmyk.strength=0.75",
        "--dry-run",
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("cmyk.strength"));
    assert!(!f.root.join("output.tif").exists());
}

#[test]
fn finished_input_dry_run_preserves_profiles_with_independent_cmyk() {
    let f = Fixture::new();
    let report = f.report(&[
        "--film",
        "kodak_portra_400",
        "--route",
        "input",
        "--set",
        "cmyk.active=true,cmyk.strength=0.75,film_render.grain.active=false",
        "--dry-run",
    ]);
    assert_eq!(report["film_profile"], "kodak_portra_400");
    assert_eq!(report["print_profile"], "kodak_portra_endura");
    assert_eq!(report["parameters"]["workflow"]["route"], "input");
    assert_eq!(report["parameters"]["cmyk"]["active"], true);
    assert_eq!(report["parameters"]["cmyk"]["strength"], 0.75);
}

#[test]
fn removed_magazine_route_reports_migration_before_profiles_load() {
    let f = Fixture::new();
    let result = f.process(&[
        "--film",
        "kodak_portra_400",
        "--route",
        "input > film > scan > magazine",
        "--dry-run",
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("input > film > scan"), "{error}");
    assert!(error.contains("cmyk.active = true"), "{error}");
}

#[test]
fn saved_magazine_route_requires_migration_even_with_route_override() {
    let f = Fixture::new();
    let params = f.root.join("obsolete-route.json");
    std::fs::write(
        &params,
        serde_json::to_vec(&json!({
            "workflow": {"route": "input > film > scan > magazine"}
        }))
        .unwrap(),
    )
    .unwrap();
    let result = f.process(&[
        "--film",
        "kodak_portra_400",
        "--params",
        params.to_str().unwrap(),
        "--route",
        "input > film > scan",
        "--dry-run",
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("input > film > scan"), "{error}");
    assert!(error.contains("cmyk.active = true"), "{error}");
}

#[test]
fn scan_output_rejects_invalid_combinations_before_dry_run_or_render() {
    let f = Fixture::new();
    let cases: &[(&str, &str, &str, &str)] = &[
        (
            "fujifilm_provia_100f",
            "input > film > scan",
            "camera.auto_exposure=false",
            "negative film",
        ),
        (
            "kodak_portra_400",
            "input > film > print > scan",
            "camera.auto_exposure=false",
            "direct film scan route",
        ),
    ];
    std::fs::write(f.root.join("output.tif"), b"retained output").unwrap();
    for (film, route, edits, message) in cases {
        for dry_run in [false, true] {
            let mut args = vec![
                "--film",
                film,
                "--route",
                route,
                "--scan-output",
                "negative_to_positive",
                "--set",
                edits,
                "--backend",
                "cpu",
            ];
            if dry_run {
                args.push("--dry-run");
            }
            let result = f.process(&args);
            assert!(
                !result.status.success(),
                "accepted {film}, {route}, {edits}"
            );
            let error = String::from_utf8_lossy(&result.stderr);
            assert!(error.contains(message), "{film} {route} {edits}: {error}");
            assert_eq!(
                std::fs::read(f.root.join("output.tif")).unwrap(),
                b"retained output"
            );
        }
    }
}

#[test]
fn negative_to_positive_rejects_paper_without_sparse_parameter_edits() {
    let f = Fixture::new();
    for dry_run in [false, true] {
        let mut args = vec![
            "--film",
            "kodak_portra_endura",
            "--scan-film",
            "--scan-output",
            "negative_to_positive",
            "--backend",
            "cpu",
        ];
        if dry_run {
            args.push("--dry-run");
        }
        let result = f.process(&args);
        assert!(!result.status.success());
        let error = String::from_utf8_lossy(&result.stderr);
        // The support/stage check rejects a print material as a film before
        // the negative-conversion check runs, so without sparse parameter
        // edits the material is refused for what it is rather than for the
        // route it was given.
        assert!(
            error.contains("incompatible support/stage; expected film/filming"),
            "{error}"
        );
        assert!(!f.root.join("output.tif").exists());
    }
}

#[test]
fn scan_output_flag_overrides_sources_but_invalid_source_is_rejected() {
    let f = Fixture::new();
    let report = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--scan-film",
        "--set",
        "scanner.scan_output=\"negative_to_positive\"",
        "--scan-output",
        "direct_scan",
        "--dry-run",
    ]);
    assert_eq!(
        report["parameters"]["scanner"]["scan_output"],
        "direct_scan"
    );
    let result = f.process(&[
        "--film",
        "kodak_portra_400",
        "--scan-film",
        "--set",
        "scanner.scan_output=\"unknown\"",
        "--scan-output",
        "direct_scan",
        "--dry-run",
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("scanner.scan_output"));
    assert!(!f.root.join("output.tif").exists());
}

#[test]
fn removed_scan_output_rejects_all_carriers_before_publication() {
    let f = Fixture::new();
    let json_path = f.root.join("removed-scan-output.json");
    let toml_path = f.root.join("removed-scan-output.toml");
    let parameters = json!({"scanner":{"scan_output":"positive_scan"}});
    std::fs::write(&json_path, serde_json::to_vec(&parameters).unwrap()).unwrap();
    std::fs::write(&toml_path, "[scanner]\nscan_output = \"positive_scan\"\n").unwrap();
    let shown = f.run(&["preset", "show", "Kodak Portra 400", "--format", "json"]);
    assert!(shown.status.success());
    let mut preset: Value = serde_json::from_slice(&shown.stdout).unwrap();
    preset["parameters"]["scanner"]["scan_output"] = json!("positive_scan");
    let preset_path = f.root.join("removed-scan-output-preset.json");
    std::fs::write(&preset_path, serde_json::to_vec(&preset).unwrap()).unwrap();
    let raw_path = f.root.join("buffer.raw");
    let output_path = f.root.join("output.tif");
    let input_before = std::fs::read(&f.input).unwrap();
    std::fs::write(&output_path, b"retained image").unwrap();
    std::fs::write(&raw_path, b"retained buffer").unwrap();
    let cases: &[&[&str]] = &[
        &[
            "--film",
            "kodak_portra_400",
            "--scan-film",
            "--scan-output",
            "positive_scan",
        ],
        &[
            "--film",
            "kodak_portra_400",
            "--scan-film",
            "--set",
            "scanner.scan_output=positive_scan",
            "--scan-output",
            "direct_scan",
        ],
        &[
            "--film",
            "kodak_portra_400",
            "--scan-film",
            "--set",
            json_path.to_str().unwrap(),
            "--scan-output",
            "direct_scan",
        ],
        &[
            "--film",
            "kodak_portra_400",
            "--scan-film",
            "--set",
            toml_path.to_str().unwrap(),
            "--scan-output",
            "direct_scan",
        ],
        &[
            "--film",
            "kodak_portra_400",
            "--scan-film",
            "--params",
            json_path.to_str().unwrap(),
            "--scan-output",
            "direct_scan",
        ],
        &[
            "--preset",
            preset_path.to_str().unwrap(),
            "--scan-film",
            "--scan-output",
            "direct_scan",
        ],
    ];
    for case in cases {
        for dry_run in [false, true] {
            let mut args = case.to_vec();
            args.extend_from_slice(&["--backend", "cpu", "--raw-out", raw_path.to_str().unwrap()]);
            if dry_run {
                args.push("--dry-run");
            }
            let result = f.process(&args);
            assert!(!result.status.success(), "accepted {case:?}");
            let error = String::from_utf8_lossy(&result.stderr);
            assert!(error.contains("positive_scan"), "{case:?}: {error}");
            assert!(error.contains("negative_to_positive"), "{case:?}: {error}");
            assert_eq!(std::fs::read(&output_path).unwrap(), b"retained image");
            assert_eq!(std::fs::read(&raw_path).unwrap(), b"retained buffer");
            assert_eq!(std::fs::read(&f.input).unwrap(), input_before);
        }
    }

    let input_path = f.root.join("recipe-input.tiff");
    let file = std::fs::File::create(&input_path).unwrap();
    let mut encoder = tiff::encoder::TiffEncoder::new(file).unwrap();
    encoder
        .new_image::<tiff::encoder::colortype::RGB32Float>(4, 4)
        .unwrap()
        .write_data(&[0.1_f32, 0.2, 0.3].repeat(16))
        .unwrap();
    drop(encoder);
    let recipe_path = f.root.join("removed-scan-output-recipe.json");
    let recipe = json!({
        "schemaVersion":"spektrafilm-rs-params-1",
        "filmProfile":"kodak_portra_400",
        "printProfile":"kodak_portra_endura",
        "parameters":parameters,
        "output":{"format":"png","precisionBits":8,"colorSpace":"sRGB","transferFunction":"srgb","geometry":"bounded","encoding":"bounded-preview","maxEdge":4}
    });
    std::fs::write(&recipe_path, serde_json::to_vec(&recipe).unwrap()).unwrap();
    let recipe_output = f.root.join("recipe-output.png");
    let input_before = std::fs::read(&input_path).unwrap();
    let result = f.run(&[
        "render",
        "--input",
        input_path.to_str().unwrap(),
        "--recipe",
        recipe_path.to_str().unwrap(),
        "--output",
        recipe_output.to_str().unwrap(),
        "--data-dir",
        f.data.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("positive_scan"), "{error}");
    assert!(error.contains("negative_to_positive"), "{error}");
    assert!(!recipe_output.exists());
    assert_eq!(std::fs::read(&input_path).unwrap(), input_before);

    #[cfg(feature = "precision-f64")]
    {
        let fit_report = f.root.join("fit-report.json");
        std::fs::write(&fit_report, b"retained fit report").unwrap();
        let result = f.run_lab(&[
            "fit",
            "--layout",
            "missing-layout.json",
            "--scene",
            "missing-scene.json",
            "--reference",
            "missing-reference.json",
            "--report",
            fit_report.to_str().unwrap(),
            "--scan-output",
            "positive_scan",
        ]);
        assert!(!result.status.success());
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains("positive_scan"), "{error}");
        assert!(error.contains("negative_to_positive"), "{error}");
        assert_eq!(std::fs::read(&fit_report).unwrap(), b"retained fit report");
    }
}

#[test]
fn inline_strings_and_enums_accept_normal_cli_values() {
    let f = Fixture::new();
    let report = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--set",
        "film_render.grain.engine=procedural,film_render.grain.procedural_profile=custom",
        "--set",
        "io.output_color_space=ProPhoto RGB",
        "--set",
        "camera.auto_exposure=false,camera.exposure_compensation_ev=+.5",
        "--set",
        "film_render.grain.procedural_amount=25",
        "--set",
        "film_render.grain.procedural_amount=null",
        "--dry-run",
    ]);
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["engine"],
        "procedural"
    );
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["procedural_profile"],
        "custom"
    );
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["procedural_amount"],
        Value::Null
    );
    assert_eq!(
        report["parameters"]["io"]["output_color_space"],
        "ProPhoto RGB"
    );
    assert_eq!(report["parameters"]["camera"]["auto_exposure"], false);
    assert_eq!(
        report["parameters"]["camera"]["exposure_compensation_ev"],
        0.5
    );
    assert!(!f.root.join("output.tif").exists());
}

#[test]
fn invalid_plain_values_report_the_field_requirement() {
    let f = Fixture::new();
    for (source, requirement) in [
        ("camera.auto_exposure=yes", "true or false"),
        ("camera.exposure_compensation_ev=bright", "finite number"),
        ("film_render.grain.engine=unknown", "spektra"),
    ] {
        let output = f.process(&["--preset", "Kodak Portra 400", "--set", source, "--dry-run"]);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(source.split_once('=').unwrap().0), "{error}");
        assert!(error.contains(requirement), "{error}");
    }
    assert!(!f.root.join("output.tif").exists());
}

#[test]
fn discovery_formats_preserve_metadata_and_reject_invalid_syntax() {
    let f = Fixture::new();
    let module = f.run(&["describe", "--module", "film_render.grain"]);
    assert!(module.status.success());
    let module: Value = serde_json::from_slice(&module.stdout).unwrap();
    for record in module["fields"].as_array().unwrap() {
        let path = record["path"].as_str().unwrap();
        let field = f.run(&["describe", "--field", path]);
        assert!(
            field.status.success(),
            "{}",
            String::from_utf8_lossy(&field.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&field.stdout).unwrap(),
            *record
        );
        let text = f.run(&["describe", "--field", path, "--format", "text"]);
        assert!(text.status.success());
        let text = String::from_utf8(text.stdout).unwrap();
        for (key, value) in record.as_object().unwrap() {
            assert!(text.contains(&format!("{key}: {value}")), "{text}");
        }
    }
    let json = f.run(&["preset", "list"]);
    let entries: Value = serde_json::from_slice(&json.stdout).unwrap();
    let text = f.run(&["preset", "list", "--format", "text"]);
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    for entry in entries.as_array().unwrap() {
        for (key, value) in entry.as_object().unwrap() {
            assert!(text.contains(&format!("{key}: {value}")), "{text}");
        }
    }
    for args in [
        vec!["describe", "--format", "toml"],
        vec![
            "describe",
            "--field",
            "camera.auto_exposure",
            "--module",
            "camera",
        ],
        vec!["preset", "list", "--format", "toml"],
        vec!["preset", "show", "Kodak Portra 400", "--format", "text"],
    ] {
        assert_eq!(f.run(&args).status.code(), Some(2));
    }
}

#[test]
fn data_selection_retains_source_and_never_falls_back() {
    let f = Fixture::new();
    let report = f.report(&["--preset", "Kodak Portra 400", "--dry-run"]);
    assert_eq!(
        report["data"]["path"],
        std::fs::canonicalize(&f.data).unwrap().to_str().unwrap()
    );
    assert_eq!(report["data"]["source"], "environment");
    assert_eq!(report["input"]["kind"], "raster");
    assert_eq!(report["input"]["raw_loading"], Value::Null);
    let selected = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--dry-run",
        "--data-dir",
        f.data.to_str().unwrap(),
    ]);
    assert_eq!(selected["data"]["source"], "argument");
    let missing = f.root.join("missing");
    let error = f.run(&["list-profiles", "--data-dir", missing.to_str().unwrap()]);
    assert_eq!(error.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&error.stderr).contains(missing.to_str().unwrap()));
    let error = f.run(&["list-profiles", "--data-dir", f.root.to_str().unwrap()]);
    assert_eq!(error.status.code(), Some(1));
    std::fs::create_dir(f.root.join("profiles")).unwrap();
    let empty = f.run(&["list-profiles", "--data-dir", f.root.to_str().unwrap()]);
    assert!(empty.status.success());
    assert!(empty.stdout.is_empty());
    std::fs::write(f.root.join("profiles/broken.json"), b"{}").unwrap();
    assert_eq!(
        f.run(&["list-profiles", "--data-dir", f.root.to_str().unwrap()])
            .status
            .code(),
        Some(1)
    );
}

#[test]
fn raw_dry_run_reports_validated_signed_loading_options() {
    let f = Fixture::new();
    let input = f.root.join("input.dng");
    // Dry run checks file kind and readability without decoding RAW pixels.
    std::fs::write(&input, b"readable-raw-placeholder").unwrap();
    let output = f.root.join("raw-output.tif");
    let base = [
        "process",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
        "--preset",
        "Kodak Portra 400",
        "--dry-run",
    ];
    let run = |extra: &[&str]| {
        let mut args = base.to_vec();
        args.extend_from_slice(extra);
        let output = f.run(&args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let shot = run(&[]);
    let custom = run(&[
        "--raw-white-balance",
        "custom",
        "--raw-temperature",
        "6500",
        "--raw-tint",
        "-5",
    ]);
    assert_eq!(
        custom,
        run(&[
            "--raw-white-balance",
            "custom",
            "--raw-temperature=6500",
            "--raw-tint=-5"
        ])
    );
    assert_eq!(
        custom["input"]["raw_loading"],
        json!({"white_balance":"custom", "temperature":6500.0, "tint":-5.0, "lens_correction":false})
    );
    assert_ne!(shot["input"], custom["input"]);
    assert_eq!(shot["input"]["raw_loading"]["temperature"], Value::Null);
    assert!(!custom["parameters"].to_string().contains("raw_loading"));
    assert!(!output.exists());
}

#[test]
fn text_discovery_lists_root_groups_and_keeps_nested_modules() {
    let f = Fixture::new();
    let text = f.run(&["describe", "--format", "text"]);
    assert!(
        text.status.success(),
        "{}",
        String::from_utf8_lossy(&text.stderr)
    );
    let text = String::from_utf8(text.stdout).unwrap();
    let groups: Vec<&str> = text.lines().collect();
    assert!(groups.contains(&"film_render") && groups.contains(&"camera"));
    assert!(groups.iter().all(|group| !group.contains('.')));
    let nested = f.run(&[
        "describe",
        "--module",
        "film_render.grain",
        "--format",
        "text",
    ]);
    assert!(
        nested.status.success(),
        "{}",
        String::from_utf8_lossy(&nested.stderr)
    );
    assert!(
        String::from_utf8(nested.stdout)
            .unwrap()
            .contains("film_render.grain.engine")
    );
}

#[test]
fn route_help_lists_every_validated_route() {
    let f = Fixture::new();
    for args in [["process", "--help"], ["process", "-h"]] {
        let help = f.run(&args);
        assert!(help.status.success());
        let help = String::from_utf8(help.stdout).unwrap();
        for route in spektrafilm_core::params::sources::supported_routes() {
            assert!(help.contains(route), "missing route {route}");
        }
        assert!(!help.contains("describe --module workflow"), "{help}");
    }
}

#[test]
fn signed_input_exposure_stops_accept_separate_arguments() {
    let f = Fixture::new();
    let build = |suffix: &str, exposure: &[&str]| {
        let out = f.root.join(format!("lut-{suffix}"));
        let mut args = vec![
            "lut",
            "build",
            out.to_str().unwrap(),
            "--name",
            "signed-stops",
            "--film",
            "kodak_portra_400",
            "--print",
            "kodak_portra_endura",
            "--input",
            "srgb",
            "--output",
            "srgb",
            "--resolution",
            "2",
        ];
        args.extend_from_slice(exposure);
        let output = f.run(&args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let metadata: Value =
            serde_json::from_slice(&std::fs::read(out.join("signed-stops/bundle.json")).unwrap())
                .unwrap();
        metadata["input_exposure"].clone()
    };
    let equal = build("equal", &["--stops-above-midgray=-1"]);
    let separate = build("separate", &["--stops-above-midgray", "-1"]);
    assert_eq!(equal, separate);
    assert_eq!(separate["stops_above_midgray"], -1.0);
    for invalid in ["-1e309", "inf", "nan"] {
        let out = f.root.join("lut-invalid");
        let output = f.run(&[
            "lut",
            "build",
            out.to_str().unwrap(),
            "--name",
            "signed-stops",
            "--film",
            "kodak_portra_400",
            "--print",
            "kodak_portra_endura",
            "--input",
            "srgb",
            "--output",
            "srgb",
            "--resolution",
            "2",
            &format!("--stops-above-midgray={invalid}"),
        ]);
        assert_eq!(output.status.code(), Some(1), "{invalid}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("finite number"),
            "{invalid}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!out.exists());
    }
}

fn library_document(f: &Fixture, name: &str, grain_amount: f64) -> PathBuf {
    let library = f.root.join("config").join("presets");
    std::fs::create_dir_all(&library).unwrap();
    let path = library.join(format!("{name}.json"));
    // A look document: both profiles plus every look-owned group, so the
    // loader accepts it. `LookParameters` has no `#[serde(default)]` on these
    // groups — a look is a whole look — so a document that carries only the
    // group under test is rejected as `missing field`.
    let document = json!({"schema_version": PRESET_SCHEMA_VERSION, "name": name,
        "film_profile": {"stock": "kodak_gold_200"},
        "print_profile": {"stock": "kodak_portra_endura"},
        "parameters": {
            "enlarger": {},
            "scanner": {},
            "film_render": {"grain": {"procedural_amount": grain_amount}},
            "print_render": {},
            "camera_diffusion_filter": {"active": false},
            "output_mix": {"amount": 1.0},
            "finishing": spektrafilm_core::params::FinishingParams::default(),
        },
        "provenance": {"implementation_version": "test", "model_version": "test"}});
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    path
}

#[test]
fn user_library_listing_keeps_valid_entries_and_reports_document_faults() {
    let f = Fixture::new();
    let valid = library_document(&f, "Soft grain", 12.0);
    let fault = library_document(&f, "Wrong filename", 5.0);
    let renamed = fault.with_file_name("Misnamed.json");
    std::fs::rename(&fault, &renamed).unwrap();
    for source in ["user", "all"] {
        let result = f.run(&["preset", "list", "--source", source]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let rows: Value = serde_json::from_slice(&result.stdout).unwrap();
        let rows = rows.as_array().unwrap();
        let grain = rows.iter().find(|row| row["name"] == "Soft grain").unwrap();
        assert_eq!(grain["group"], "My Presets");
        assert_eq!(grain["kind"], "look");
        assert_eq!(grain["path"], valid.to_str().unwrap());
        assert!(String::from_utf8_lossy(&result.stderr).contains(renamed.to_str().unwrap()));
        assert_eq!(
            rows.iter().any(|row| row["group"] == "Built-in"),
            source == "all"
        );
    }
    let result = Command::new(env!("CARGO_BIN_EXE_spektrafilm"))
        .args(["preset", "list", "--source", "all"])
        .current_dir(&f.root)
        .env("SPEKTRAFILM_CONFIG_DIR", f.root.join("config"))
        .env_remove("SPEKTRAFILM_DATA_DIR")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn names_resolve_user_first_and_explicit_groups_reach_shadowed_builtins() {
    let f = Fixture::new();
    library_document(&f, "Kodak Portra 400", 12.0);
    let selected = f.run(&["preset", "show", "Kodak Portra 400", "--format", "json"]);
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    let document: Value = serde_json::from_slice(&selected.stdout).unwrap();
    assert_eq!(
        document["parameters"]["film_render"]["grain"]["procedural_amount"],
        12.0
    );
    let stderr = String::from_utf8_lossy(&selected.stderr);
    assert!(
        stderr.contains("My Presets")
            && stderr.contains("shadows Built-in")
            && stderr.contains("Kodak Portra 400")
    );
    let builtin = f.run(&[
        "preset",
        "show",
        "built-in:Kodak Portra 400",
        "--format",
        "json",
    ]);
    assert!(builtin.status.success());
    let document: Value = serde_json::from_slice(&builtin.stdout).unwrap();
    assert_eq!(document["film_profile"]["stock"], "kodak_portra_400");
    assert!(!String::from_utf8_lossy(&builtin.stderr).contains("shadows"));
    library_document(&f, "Soft grain", 12.0);
    let unknown = f.run(&["preset", "show", "Soft grani"]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("Soft grain"));
    let missing = f.run(&["preset", "show", "Soft grain.toml"]);
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("Soft grain.toml"));
    assert!(!stderr.contains("did you mean"));
}

#[test]
fn look_preset_normalizes_inapplicable_negative_to_positive_to_direct_scan() {
    let f = Fixture::new();
    let shown = f.run(&["preset", "show", "Kodak Portra 400", "--format", "json"]);
    assert!(shown.status.success());
    let mut document: Value = serde_json::from_slice(&shown.stdout).unwrap();
    document["parameters"]["scanner"]["scan_output"] = json!("negative_to_positive");
    let path = f.root.join("negative-conversion-look.json");
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();

    let report = f.report(&["--preset", path.to_str().unwrap(), "--dry-run"]);
    assert_eq!(
        report["parameters"]["workflow"]["route"],
        "input > film > print > scan"
    );
    assert_eq!(
        report["parameters"]["scanner"]["scan_output"],
        "direct_scan"
    );
}

#[test]
fn active_chemistry_domains_fail_before_image_decode_in_dry_run_and_render() {
    let f = Fixture::new();
    std::fs::write(&f.input, b"invalid image data").unwrap();
    for group in ["film_render.chemistry", "print_render.density_curves_morph"] {
        for field in [
            "gamma_factor",
            "gamma_factor_fast",
            "gamma_factor_slow",
            "gamma_factor_red",
            "gamma_factor_green",
            "gamma_factor_blue",
            "developer_exhaustion",
        ] {
            let value = if field == "developer_exhaustion" {
                "2"
            } else {
                "0"
            };
            let edit = format!("{group}.active=true,{group}.{field}={value}");
            for dry_run in [false, true] {
                let mut args = vec![
                    "--preset",
                    "Kodak Portra 400",
                    "--backend",
                    "cpu",
                    "--set",
                    &edit,
                ];
                if dry_run {
                    args.push("--dry-run");
                }
                let rejected = f.process(&args);
                assert!(!rejected.status.success());
                let error = String::from_utf8_lossy(&rejected.stderr);
                assert!(error.contains(&format!("{group}.{field}")), "{error}");
                assert!(!f.root.join("output.tif").exists());
            }
        }
        let invalid_inactive = format!("{group}.active=false,{group}.developer_exhaustion=2");
        let rejected = f.process(&[
            "--preset",
            "Kodak Portra 400",
            "--set",
            &invalid_inactive,
            "--dry-run",
        ]);
        assert!(!rejected.status.success());
        assert!(
            String::from_utf8_lossy(&rejected.stderr)
                .contains(&format!("{group}.developer_exhaustion"))
        );
        assert!(!f.root.join("output.tif").exists());
    }
}

#[test]
fn invalid_inactive_preset_cannot_be_hidden_by_a_later_look_or_source() {
    let f = Fixture::new();
    let shown = f.run(&["preset", "show", "Kodak Portra 400", "--format", "json"]);
    assert!(shown.status.success());
    let mut look: Value = serde_json::from_slice(&shown.stdout).unwrap();
    look["parameters"]["print_render"]["density_curves_morph"]["active"] = json!(false);
    look["parameters"]["print_render"]["density_curves_morph"]["developer_exhaustion"] = json!(2.0);
    let source = serde_json::to_vec(&look).unwrap();
    let path = f.root.join("invalid-inactive.json");
    std::fs::write(&path, &source).unwrap();
    let output = f.root.join("output.tif");
    std::fs::write(&output, b"accepted output").unwrap();
    let result = f.process(&[
        "--preset",
        path.to_str().unwrap(),
        "--preset",
        "Kodak Portra 400",
        "--set",
        "print_render.density_curves_morph.developer_exhaustion=0.5",
    ]);
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("print_render.density_curves_morph.developer_exhaustion")
    );
    assert_eq!(std::fs::read(&path).unwrap(), source);
    assert_eq!(std::fs::read(&output).unwrap(), b"accepted output");
}

#[test]
fn describe_text_lists_non_settable_runtime_leaves_and_set_rejects_them() {
    let f = Fixture::new();
    let description = f.run(&["describe", "--format", "text"]);
    assert!(
        description.status.success(),
        "{}",
        String::from_utf8_lossy(&description.stderr)
    );
    let text = String::from_utf8_lossy(&description.stdout);
    assert!(
        text.contains(
            "random_seed (runtime field outside settable parameter surface: per-render stochastic seed is set by the recipe runner)"
        ),
        "{text}"
    );

    let rejected = f.process(&[
        "--film",
        "kodak_portra_400",
        "--paper",
        "kodak_portra_endura",
        "--set",
        "random_seed=17",
        "--dry-run",
    ]);
    assert!(!rejected.status.success());
    let error = String::from_utf8_lossy(&rejected.stderr);
    assert!(error.contains("random_seed"), "{error}");
    assert!(
        error.contains("runtime field outside the settable parameter surface"),
        "{error}"
    );
    assert!(error.contains("recipe runner"), "{error}");
}
