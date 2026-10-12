use serde_json::{Value, json};
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
fn sources_compose_in_order_and_dry_run_preserves_existing_outputs() {
    let f = Fixture::new();
    let sparse = f.root.join("edits.toml");
    std::fs::write(&sparse, "[camera]\nexposure_compensation_ev = 1.0\n[film_render.grain]\nrms_granularity = [1.0,2.0,3.0]\nv2_amount = 25.0\n[enlarger]\ny_filter_neutral = 12.0\n").unwrap();
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
        "camera.exposure_compensation_ev=0.5,film_render.grain.v2_amount=null",
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
        report["parameters"]["film_render"]["grain"]["v2_amount"],
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
        .find(|field| field["path"] == "film_render.grain.v2_amount")
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
    // A present grain group has serde field defaults distinct from GrainParams::default().
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
fn sparse_edits_preserve_legacy_group_defaults_and_protect_only_supplied_axes() {
    use spektrafilm_core::{neutral_filters::NeutralFilters, params::RuntimeParams, profile};
    let f = Fixture::new();
    let path = f.root.join("legacy.json");
    let wire = json!({"film_render":{"grain":{}},"enlarger":{"y_filter_neutral":9.0,"m_filter_neutral":8.0,"c_filter_neutral":7.0}});
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
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["blur"],
        baseline.film_render.grain.blur
    );
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
        "film_render.grain.v2_amount=101",
        "camera.exposure_compensation_ev=3.5e38",
        "settings.preview_max_size=1.5",
        "camera.color_filter=null",
        "workflow.route=\"input\"",
        "io.scan_film=true",
        "film_render.grain.monochrome=true",
        "film_render.grain.particle_scale_layers=[1,1,1]",
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
            "film_render.grain.v2_amount=25",
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
    use spektrafilm_core::{params::RuntimeParams, profile, runtime::Runtime};
    use spektrafilm_gpu::cpu_backend::CpuBackend;
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
    let expected: Vec<f64> = expected.data.iter().map(|&v| v as f64).collect();
    assert_eq!(rendered, expected);
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
fn scanner_selectors_report_approximation_and_reject_unsupported_routes_before_pixels() {
    let f = Fixture::new();
    let report = f.report(&["--film", "kodak_portra_400", "--scan-film", "--scan-output", "positive_scan", "--set", "scanner.receiver_profile=lab_scan_approx,scanner.positive_interpretation=coupled,scanner.auto_setup=per_frame,scanner.finish=lab_scan,scanner.finish_strength=0.5", "--dry-run"]);
    assert_eq!(
        report["parameters"]["scanner"]["receiver_profile"],
        "lab_scan_approx"
    );
    assert_eq!(
        report["parameters"]["scanner"]["positive_interpretation"],
        "coupled"
    );
    assert_eq!(report["parameters"]["scanner"]["auto_setup"], "per_frame");
    assert_eq!(
        report["scanner_provenance"]["receiver"]["calibration_status"],
        "approximate_uncalibrated"
    );
    std::fs::write(&f.input, b"invalid-pixels").unwrap();
    for selection in [
        "scanner.positive_interpretation=coupled",
        "scanner.auto_setup=per_frame",
        "scanner.finish=lab_scan",
        "scanner.receiver_profile=not_a_scanner",
    ] {
        for dry in [false, true] {
            let mut args = vec!["--film", "kodak_portra_400", "--set", selection];
            if dry {
                args.push("--dry-run");
            }
            let result = f.process(&args);
            assert!(!result.status.success(), "accepted {selection}");
            assert!(String::from_utf8_lossy(&result.stderr).contains("scanner"));
            assert!(!f.root.join("output.tif").exists());
        }
    }
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
        "io.output_gamut_compress.lightness_compression",
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
fn legacy_json_decoder_rejects_optional_f32_overflow_before_finite_validation() {
    let f = Fixture::new();
    let path = f.root.join("legacy.json");
    std::fs::write(&path, r#"{"film_render":{"grain":{"v2_amount":1e100}}}"#).unwrap();
    let result = f.process(&[
        "--film",
        "kodak_portra_400",
        "--params",
        path.to_str().unwrap(),
        "--set",
        "camera.auto_exposure=false",
        "--dry-run",
    ]);
    assert!(!result.status.success());
    // The workspace's correctly rounded JSON decoder rejects overflow of the declared
    // f32 type before finite validation; this diagnostic belongs to the decoder.
    assert!(String::from_utf8_lossy(&result.stderr).contains("number out of range"));
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
fn magazine_enabled_preserves_preset_profiles_and_selects_direct_scan_without_preset() {
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
            "magazine_print_color.active=true,magazine_print_color.strength=0.75",
            "--dry-run",
        ]);
        assert_eq!(report["parameters"]["io"]["scan_film"], true);
        assert_eq!(
            report["parameters"]["magazine_print_color"]["strength"],
            0.75
        );
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
        "magazine_print_color.strength=1.1",
        "--set",
        "magazine_print_color.strength=0.75",
        "--dry-run",
    ]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("magazine_print_color.strength"));
    assert!(!f.root.join("output.tif").exists());
}

#[test]
fn finished_input_dry_run_preserves_profiles_with_independent_magazine() {
    let f = Fixture::new();
    let report = f.report(&[
        "--film",
        "kodak_portra_400",
        "--route",
        "input",
        "--set",
        "magazine_print_color.active=true,magazine_print_color.strength=0.75",
        "--dry-run",
    ]);
    assert_eq!(report["film_profile"], "kodak_portra_400");
    assert_eq!(report["print_profile"], "kodak_portra_endura");
    assert_eq!(report["parameters"]["workflow"]["route"], "input");
    assert_eq!(report["parameters"]["magazine_print_color"]["active"], true);
    assert_eq!(
        report["parameters"]["magazine_print_color"]["strength"],
        0.75
    );
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
    assert!(
        error.contains("magazine_print_color.active = true"),
        "{error}"
    );
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
    assert!(
        error.contains("magazine_print_color.active = true"),
        "{error}"
    );
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
        (
            "kodak_portra_400",
            "input > film > scan",
            "scanner.white_correction=true",
            "white/black correction",
        ),
        (
            "kodak_portra_400",
            "input > film > scan",
            "scanner.black_correction=true",
            "white/black correction",
        ),
        (
            "kodak_portra_400",
            "input > film > scan",
            "film_render.grain.engine=\"v2\",film_render.grain.active=true,settings.preview_mode=true",
            "active Grain V2",
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
                "positive_scan",
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
fn positive_scan_rejects_paper_without_sparse_parameter_edits() {
    let f = Fixture::new();
    for dry_run in [false, true] {
        let mut args = vec![
            "--film",
            "kodak_portra_endura",
            "--scan-film",
            "--scan-output",
            "positive_scan",
            "--backend",
            "cpu",
        ];
        if dry_run {
            args.push("--dry-run");
        }
        let result = f.process(&args);
        assert!(!result.status.success());
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(
            error.contains("requires a negative filming-stage film profile"),
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
        "scanner.scan_output=\"positive_scan\",scanner.white_correction=true",
        "--scan-output",
        "direct_scan",
        "--dry-run",
    ]);
    assert_eq!(
        report["parameters"]["scanner"]["scan_output"],
        "direct_scan"
    );
    assert_eq!(report["parameters"]["scanner"]["white_correction"], true);
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
fn inline_strings_and_enums_accept_normal_cli_values() {
    let f = Fixture::new();
    let report = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--set",
        "film_render.grain.engine=v2,film_render.grain.v2_profile=custom",
        "--set",
        "io.output_color_space=ProPhoto RGB",
        "--set",
        "camera.auto_exposure=false,camera.exposure_compensation_ev=+.5",
        "--set",
        "film_render.grain.v2_amount=25",
        "--set",
        "film_render.grain.v2_amount=null",
        "--dry-run",
    ]);
    assert_eq!(report["parameters"]["film_render"]["grain"]["engine"], "v2");
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["v2_profile"],
        "custom"
    );
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["v2_amount"],
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
        ("film_render.grain.engine=unknown", "v1"),
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

fn library_document(f: &Fixture, name: &str, parameters: Value) -> PathBuf {
    let library = f.root.join("config").join("presets");
    std::fs::create_dir_all(&library).unwrap();
    let path = library.join(format!("{name}.json"));
    let document = json!({"schema_version": 2, "name": name,
        "parameters": parameters,
        "provenance": {"implementation_version": "test", "model_version": "test"}});
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    path
}

#[test]
fn user_library_listing_keeps_valid_entries_and_reports_document_faults() {
    let f = Fixture::new();
    let valid = library_document(
        &f,
        "Soft grain",
        json!({"film_render":{"grain":{"v2_amount":12.0}}}),
    );
    let fault = library_document(
        &f,
        "Wrong filename",
        json!({"film_render":{"grain":{"v2_amount":5.0}}}),
    );
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
        assert_eq!(grain["kind"], "section");
        assert_eq!(grain["section"], "grain");
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
    library_document(
        &f,
        "Kodak Portra 400",
        json!({"film_render":{"grain":{"v2_amount":12.0}}}),
    );
    let selected = f.run(&["preset", "show", "Kodak Portra 400", "--format", "json"]);
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    let document: Value = serde_json::from_slice(&selected.stdout).unwrap();
    assert_eq!(
        document["parameters"]["film_render"]["grain"]["v2_amount"],
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
    library_document(
        &f,
        "Soft grain",
        json!({"film_render":{"grain":{"v2_amount":12.0}}}),
    );
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
fn repeated_presets_compose_look_then_section_and_keep_profile_workflow() {
    let f = Fixture::new();
    library_document(
        &f,
        "Soft grain",
        json!({"film_render":{"grain":{"v2_amount":12.0}}}),
    );
    let report = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--preset",
        "Soft grain",
        "--dry-run",
    ]);
    assert_eq!(report["film_profile"], "kodak_portra_400");
    assert_eq!(report["print_profile"], "kodak_portra_endura");
    assert_eq!(
        report["parameters"]["workflow"]["route"],
        "input > film > print > scan"
    );
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["v2_amount"],
        12.0
    );
    let section = f.report(&[
        "--film",
        "kodak_portra_400",
        "--preset",
        "Soft grain",
        "--dry-run",
    ]);
    let baseline = f.report(&["--film", "kodak_portra_400", "--dry-run"]);
    assert_eq!(
        section["parameters"]["workflow"],
        baseline["parameters"]["workflow"]
    );
    assert_eq!(section["film_profile"], baseline["film_profile"]);
    assert_eq!(section["print_profile"], baseline["print_profile"]);
    assert_eq!(
        section["parameters"]["film_render"]["grain"]["v2_amount"],
        12.0
    );
    let direct = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--preset",
        "Soft grain",
        "--scan-film",
        "--dry-run",
    ]);
    assert_eq!(direct["print_profile"], "kodak_portra_endura");
    assert_eq!(direct["parameters"]["io"]["scan_film"], true);
    assert!(
        !f.process(&["--preset", "Soft grain", "--dry-run"])
            .status
            .success()
    );
    for extra in [
        ["--film", "kodak_portra_400"],
        ["--paper", "kodak_portra_endura"],
    ] {
        let result = f.process(&[
            "--preset",
            "Kodak Portra 400",
            extra[0],
            extra[1],
            "--dry-run",
        ]);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("conflicts"));
    }
    let params = f.root.join("baseline.json");
    std::fs::write(&params, "{}").unwrap();
    assert!(
        !f.process(&[
            "--preset",
            "Kodak Portra 400",
            "--params",
            params.to_str().unwrap(),
            "--dry-run"
        ])
        .status
        .success()
    );
    assert!(
        f.process(&[
            "--film",
            "kodak_portra_400",
            "--paper",
            "kodak_portra_endura",
            "--params",
            params.to_str().unwrap(),
            "--preset",
            "Soft grain",
            "--dry-run"
        ])
        .status
        .success()
    );
}

#[test]
fn section_show_preserves_null_and_refuses_lossy_toml_export() {
    let f = Fixture::new();
    let path = library_document(
        &f,
        "Clear amount",
        json!({"film_render":{"grain":{"v2_amount":null}}}),
    );
    let before = std::fs::read(&path).unwrap();
    let json = f.run(&["preset", "show", "Clear amount", "--format", "json"]);
    assert!(json.status.success());
    let document: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(document["name"], "Clear amount");
    assert!(
        document["parameters"]["film_render"]["grain"]
            .as_object()
            .unwrap()
            .contains_key("v2_amount")
    );
    assert_eq!(
        document["parameters"]["film_render"]["grain"]["v2_amount"],
        Value::Null
    );
    let toml = f.run(&["preset", "show", "Clear amount"]);
    assert!(!toml.status.success());
    assert!(toml.stdout.is_empty());
    assert!(String::from_utf8_lossy(&toml.stderr).contains("film_render.grain.v2_amount"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn later_look_replaces_earlier_section_and_section_only_keeps_static_route() {
    let f = Fixture::new();
    library_document(
        &f,
        "Soft grain",
        json!({"film_render":{"grain":{"v2_amount":12.0}}}),
    );
    let baseline = f.report(&["--preset", "Kodak Portra 400", "--dry-run"]);
    let replaced = f.report(&[
        "--preset",
        "Soft grain",
        "--preset",
        "Kodak Portra 400",
        "--dry-run",
    ]);
    assert_eq!(replaced, baseline);
    let sections = f.report(&[
        "--film",
        "fujifilm_provia_100f",
        "--paper",
        "fujifilm_crystal_archive_typeii",
        "--preset",
        "Soft grain",
        "--dry-run",
    ]);
    let baseline = f.report(&[
        "--film",
        "fujifilm_provia_100f",
        "--paper",
        "fujifilm_crystal_archive_typeii",
        "--dry-run",
    ]);
    assert_eq!(
        sections["parameters"]["workflow"],
        baseline["parameters"]["workflow"]
    );
    let explicit = f.process(&[
        "--preset",
        "Kodak Portra 400",
        "--scan-film",
        "--route",
        "input > film > print > scan",
        "--dry-run",
    ]);
    assert!(!explicit.status.success());
    let message = String::from_utf8_lossy(&explicit.stderr);
    assert!(
        message.contains("--route") && message.contains("--scan-film"),
        "{message}"
    );
    assert!(!f.root.join("output.tif").exists());
    let section = f.process(&[
        "--film",
        "kodak_portra_400",
        "--preset",
        "Soft grain",
        "--scan-film",
        "--route",
        "input > film > print > scan",
        "--dry-run",
    ]);
    assert!(!section.status.success());
    let message = String::from_utf8_lossy(&section.stderr);
    assert!(
        message.contains("--route") && message.contains("--scan-film"),
        "{message}"
    );
}

#[test]
fn look_preset_normalizes_inapplicable_positive_scan_to_direct_scan() {
    let f = Fixture::new();
    let shown = f.run(&["preset", "show", "Kodak Portra 400", "--format", "json"]);
    assert!(shown.status.success());
    let mut document: Value = serde_json::from_slice(&shown.stdout).unwrap();
    document["parameters"]["scanner"]["scan_output"] = json!("positive_scan");
    let path = f.root.join("positive-output-look.json");
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
fn invalid_scanner_section_rejects_without_printing_effective_configuration() {
    let f = Fixture::new();
    library_document(
        &f,
        "Positive scanner",
        json!({"scanner":{"scan_output":"positive_scan"}}),
    );
    let baseline = f.report(&["--preset", "Kodak Portra 400", "--dry-run"]);
    let rejected = f.process(&[
        "--preset",
        "Kodak Portra 400",
        "--preset",
        "Positive scanner",
        "--dry-run",
    ]);
    assert!(!rejected.status.success());
    let error = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        error.contains("scanner.scan_output=positive_scan")
            && error.contains("requires a direct film scan route"),
        "{error}"
    );
    assert!(rejected.stdout.is_empty());
    assert_eq!(
        f.report(&["--preset", "Kodak Portra 400", "--dry-run"]),
        baseline
    );
}

#[test]
fn json_section_export_reload_preserves_optional_grain_clearing_edit() {
    let f = Fixture::new();
    library_document(
        &f,
        "Clear amount",
        json!({"film_render":{"grain":{"v2_amount":null}}}),
    );
    library_document(
        &f,
        "Amount twelve",
        json!({"film_render":{"grain":{"v2_amount":12.0}}}),
    );
    // `preset show` exports documents; selecting the exported path reloads them.
    let exported = f.run(&["preset", "show", "Clear amount", "--format", "json"]);
    assert!(exported.status.success());
    let document: Value = serde_json::from_slice(&exported.stdout).unwrap();
    assert_eq!(
        document["parameters"]["film_render"]["grain"]
            .as_object()
            .unwrap()
            .get("v2_amount"),
        Some(&Value::Null)
    );
    let path = f.root.join("exported-clear-amount.json");
    std::fs::write(&path, exported.stdout).unwrap();
    let baseline = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--preset",
        "Amount twelve",
        "--dry-run",
    ]);
    assert_eq!(
        baseline["parameters"]["film_render"]["grain"]["v2_amount"],
        12.0
    );
    let cleared = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--preset",
        "Amount twelve",
        "--preset",
        path.to_str().unwrap(),
        "--dry-run",
    ]);
    assert_eq!(
        cleared["parameters"]["film_render"]["grain"]["v2_amount"],
        Value::Null
    );
}

#[test]
fn scanner_sections_validate_the_final_cli_workflow() {
    let f = Fixture::new();
    library_document(
        &f,
        "Positive scanner",
        json!({"scanner":{"scan_output":"positive_scan"}}),
    );
    for base in [
        vec!["--preset", "Kodak Portra 400"],
        vec!["--film", "kodak_portra_400"],
    ] {
        for workflow in [vec!["--scan-film"], vec!["--route", "input > film > scan"]] {
            let mut args = base.clone();
            args.extend_from_slice(&["--preset", "Positive scanner"]);
            args.extend_from_slice(&workflow);
            args.push("--dry-run");
            let report = f.report(&args);
            assert_eq!(
                report["parameters"]["workflow"]["route"],
                "input > film > scan"
            );
            assert_eq!(report["parameters"]["io"]["scan_film"], true);
            assert_eq!(
                report["parameters"]["scanner"]["scan_output"],
                "positive_scan"
            );
        }
    }
    let replaced = f.report(&[
        "--preset",
        "Kodak Portra 400",
        "--preset",
        "Positive scanner",
        "--set",
        "scanner.scan_output=direct_scan",
        "--dry-run",
    ]);
    assert_eq!(
        replaced["parameters"]["scanner"]["scan_output"],
        "direct_scan"
    );
    assert_eq!(
        replaced["parameters"]["workflow"]["route"],
        "input > film > print > scan"
    );
    let rejected = f.process(&[
        "--preset",
        "Fujifilm Provia 100F",
        "--preset",
        "Positive scanner",
        "--scan-film",
        "--dry-run",
    ]);
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
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
        let inactive =
            format!("{group}.active=false,{group}.gamma_factor=0,{group}.developer_exhaustion=2");
        let report = f.report(&[
            "--preset",
            "Kodak Portra 400",
            "--set",
            &inactive,
            "--dry-run",
        ]);
        let pointer = format!("/parameters/{}/active", group.replace('.', "/"));
        assert_eq!(report.pointer(&pointer), Some(&json!(false)));
    }
}

#[test]
fn chemistry_sections_validate_after_later_sources_and_looks() {
    let f = Fixture::new();
    let shown = f.run(&["preset", "show", "Kodak Portra 400", "--format", "json"]);
    assert!(shown.status.success());
    let mut look: Value = serde_json::from_slice(&shown.stdout).unwrap();
    look["name"] = json!("Inactive chemistry");
    look["parameters"]["print_render"]["density_curves_morph"]["active"] = json!(false);
    look["parameters"]["print_render"]["density_curves_morph"]["gamma_factor"] = json!(0.0);
    let path = f.root.join("inactive.json");
    std::fs::write(&path, serde_json::to_vec(&look).unwrap()).unwrap();
    library_document(
        &f,
        "Enable chemistry",
        json!({"print_render":{"density_curves_morph":{"active":true}}}),
    );
    let base = [
        "--preset",
        path.to_str().unwrap(),
        "--preset",
        "Enable chemistry",
    ];
    let mut repaired = base.to_vec();
    repaired.extend_from_slice(&[
        "--set",
        "print_render.density_curves_morph.gamma_factor=1",
        "--dry-run",
    ]);
    let report = f.report(&repaired);
    assert_eq!(
        report["parameters"]["print_render"]["density_curves_morph"]["active"],
        true
    );
    assert_eq!(
        report["parameters"]["print_render"]["density_curves_morph"]["gamma_factor"],
        1.0
    );
    let mut replaced = base.to_vec();
    replaced.extend_from_slice(&["--preset", "Kodak Portra 400", "--dry-run"]);
    assert_eq!(
        f.report(&replaced),
        f.report(&["--preset", "Kodak Portra 400", "--dry-run"])
    );
    let mut invalid = base.to_vec();
    invalid.push("--dry-run");
    let rejected = f.process(&invalid);
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("print_render.density_curves_morph.gamma_factor")
    );
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
