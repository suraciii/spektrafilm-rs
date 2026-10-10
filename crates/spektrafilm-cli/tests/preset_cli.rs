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
        "classic-kodak-portra-400",
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
        .find(|entry| entry["id"] == "classic-kodak-portra-400")
        .unwrap();
    assert_eq!(portra["film_profile"]["stock"], "kodak_portra_400");
    assert_eq!(portra["print_profile"]["stock"], "kodak_portra_endura");
    let mut reports = Vec::new();
    for format in ["json", "toml"] {
        let shown = f.run(&[
            "preset",
            "show",
            "classic-kodak-portra-400",
            "--format",
            format,
        ]);
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
            "classic-kodak-portra-400",
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
            "classic-kodak-portra-400",
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
        !f.process(&["--preset", "Kodak Portra 400", "--dry-run"])
            .status
            .success()
    );
    assert!(
        !f.process(&[
            "--preset",
            "classic-kodak-portra-400",
            "--film",
            "kodak_portra_400",
            "--dry-run"
        ])
        .status
        .success()
    );
}

#[test]
fn final_workflow_and_runtime_constraints_resolve_after_edits() {
    let f = Fixture::new();
    let report = f.report(&[
        "--preset",
        "classic-kodak-portra-400",
        "--scan-film",
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
        "classic-kodak-portra-400",
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
        "classic-kodak-portra-400",
        "--set",
        edits,
        "--backend",
        "cpu",
        "--dry-run",
    ]);
    let raw = f.root.join("render.raw");
    let output = f.process(&[
        "--preset",
        "classic-kodak-portra-400",
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
        .chunks_exact(8)
        .map(|bytes| f64::from_ne_bytes(bytes.try_into().unwrap()))
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
        "classic-kodak-portra-400",
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
        let mut args = vec!["--preset", "classic-kodak-portra-400", "--backend", "gpu"];
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
    let report = f.report(&["--preset", "classic-kodak-portra-400", "--dry-run"]);
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
            !f.process(&[
                "--preset",
                "classic-kodak-portra-400",
                "--set",
                edits,
                "--dry-run"
            ])
            .status
            .success()
        );
    }
}

#[test]
fn dry_run_rejects_missing_inputs_and_duplicate_selectors() {
    let mut f = Fixture::new();
    for options in [
        vec![
            "--preset",
            "classic-kodak-portra-400",
            "--preset",
            "classic-kodak-gold-200",
        ],
        vec!["--film", "kodak_portra_400", "--film", "kodak_gold_200"],
        vec![
            "--preset",
            "classic-kodak-portra-400",
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
        !f.process(&["--preset", "classic-kodak-portra-400", "--dry-run"])
            .status
            .success()
    );
    f.input = f.root.join("unsupported.txt");
    std::fs::write(&f.input, b"text").unwrap();
    assert!(
        !f.process(&["--preset", "classic-kodak-portra-400", "--dry-run"])
            .status
            .success()
    );
    f.input = f.root.clone();
    assert!(
        !f.process(&["--preset", "classic-kodak-portra-400", "--dry-run"])
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
                "--preset",
                "classic-kodak-portra-400",
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
fn edits_reject_optional_overflow_in_legacy_baseline() {
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
    assert!(String::from_utf8_lossy(&result.stderr).contains("v2_amount"));
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
        ["--preset", "classic-kodak-portra-400"],
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
            "direct input",
        ),
        (
            "kodak_portra_400",
            "input > convert-film > scan",
            "camera.auto_exposure=false",
            "direct input",
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
            assert!(error.contains(message), "{error}");
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
            error.contains("requires a negative film profile"),
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
        "classic-kodak-portra-400",
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
