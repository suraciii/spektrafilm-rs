use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "spektrafilm-image-cli-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let pixels: Vec<u8> = (0..32 * 24)
            .flat_map(|i| [(i % 200 + 20) as u8, 90, 170])
            .collect();
        image::save_buffer(
            root.join("input.png"),
            &pixels,
            32,
            24,
            image::ColorType::Rgb8,
        )
        .unwrap();
        Self(root)
    }
    fn run(&self, output: &str, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_spektrafilm"))
            .arg("process")
            .arg(self.0.join("input.png"))
            .arg("--output")
            .arg(self.0.join(output))
            .args([
                "--film",
                "kodak_portra_400",
                "--backend",
                "cpu",
                "--data-dir",
            ])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"))
            .args(extra)
            .env("RUST_LOG", "off")
            .env("SPEKTRAFILM_CONFIG_DIR", self.0.join("config"))
            .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
            .env_remove("SPEKTRAFILM_DATA_DIR")
            .env_remove("SPEKTRAFILM_BACKEND")
            .output()
            .unwrap()
    }
    fn report(&self, extra: &[&str]) -> Value {
        let result = self.run("output.tif", extra);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn raster_declaration_requires_supplied_components_without_publication() {
    let f = Fixture::new();
    std::fs::write(f.0.join("output.tif"), b"prior image").unwrap();
    for args in [
        vec![],
        vec!["--input-color-space", "sRGB"],
        vec!["--input-cctf-decoding", "false"],
    ] {
        let result = f.run("output.tif", &args);
        assert_eq!(result.status.code(), Some(1));
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains("input"), "{error}");
        assert_eq!(
            std::fs::read(f.0.join("output.tif")).unwrap(),
            b"prior image"
        );
    }
    let report = f.report(&[
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "false",
        "--dry-run",
    ]);
    assert_eq!(report["input"]["declaration"]["cctf_decoding"], false);
    assert_eq!(
        report["input"]["declaration"]["sources"]["cctf_decoding"]["option"],
        "--input-cctf-decoding"
    );
    assert_eq!(
        std::fs::read(f.0.join("output.tif")).unwrap(),
        b"prior image"
    );
}

#[test]
fn fotufilm_dry_run_reports_authored_controls_and_cpu_provenance() {
    let f = Fixture::new();
    let report = f.report(&[
        "--input-color-space", "sRGB", "--input-cctf-decoding", "true",
        "--route", "input > film > scan",
        "--set", "film_render.grain.engine=fotufilm,film_render.grain.fotufilm.amount=1.5,film_render.grain.fotufilm.rms_density=0.012,film_render.grain.fotufilm.clump_radius_um=7.5,film_render.grain.fotufilm.material=dye_negative",
        "--dry-run",
    ]);
    let grain = &report["parameters"]["film_render"]["grain"];
    assert_eq!(grain["engine"], "fotufilm");
    assert_eq!(grain["fotufilm"]["amount"], 1.5);
    assert_eq!(grain["fotufilm"]["rms_density"], 0.012);
    assert_eq!(grain["fotufilm"]["clump_radius_um"], 7.5);
    assert_eq!(grain["fotufilm"]["material"], "dye_negative");
    let provenance = &report["film_grain_fotufilm"];
    assert_eq!(provenance["active"], true);
    assert_eq!(provenance["implementation"], "rust_cpu_film");
    assert_eq!(provenance["effective_backend"], "cpu");
    assert!(
        provenance["note"]
            .as_str()
            .is_some_and(|note| note.contains("CPU"))
    );
    assert_eq!(
        provenance["appearance_status"],
        "authored_effective_not_stock_calibration"
    );
    assert_eq!(provenance["readout"], "complete_periodic_tile");
    assert_eq!(provenance["prepared"], false);
    assert!(
        provenance["source_revision"]
            .as_str()
            .is_some_and(|revision| !revision.is_empty())
    );
    assert_eq!(report["film_grain_dye"]["active"], false);
    assert!(!f.0.join("output.tif").exists());
}

#[test]
fn fotufilm_zero_amount_is_not_reported_as_active() {
    let f = Fixture::new();
    let report = f.report(&[
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--set",
        "film_render.grain.engine=fotufilm,film_render.grain.fotufilm.amount=0",
        "--dry-run",
    ]);
    assert_eq!(report["film_grain_fotufilm"]["active"], false);
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["engine"],
        "fotufilm"
    );
}

#[test]
fn fixed_input_precedence_and_winning_edit_feedback_are_observable() {
    let f = Fixture::new();
    let report = f.report(&[
        "--input-color-space", "sRGB", "--input-cctf-decoding", "true",
        "--set", "io.input_color_space=ProPhoto RGB,io.input_cctf_decoding=false",
        "--set", "film_render.grain.engine=procedural,film_render.grain.v2_profile=35mm250,film_render.grain.v2_size=12",
        "--set", "film_render.grain.v2_size=15,film_render.grain.v2_amount=25", "--dry-run",
    ]);
    assert_eq!(report["parameters"]["io"]["input_color_space"], "sRGB");
    let edits = report["edits"].as_array().unwrap();
    let field = |path: &str| edits.iter().find(|entry| entry["path"] == path).unwrap();
    assert_eq!(field("io.input_color_space")["status"], "overridden");
    assert_eq!(field("film_render.grain.v2_size")["requested"], 15.0);
    assert_eq!(field("film_render.grain.v2_size")["source"]["index"], 3);
    assert_eq!(field("film_render.grain.v2_size")["status"], "inactive");
    assert_eq!(field("film_render.grain.v2_amount")["status"], "active");
    assert_eq!(
        edits
            .iter()
            .filter(|entry| entry["path"] == "film_render.grain.v2_size")
            .count(),
        1
    );
    assert!(!f.0.join("output.tif").exists());
}

#[test]
fn route_conflicts_and_dry_run_format_fail_before_rendering() {
    let f = Fixture::new();
    for args in [
        vec!["--route", "input > film > scan", "--scan-film"],
        vec!["--dry-run-format", "text"],
        vec!["--seed", "4294967296"],
        vec!["--seed", "-1"],
        vec!["--seed", "0", "--seed", "42"],
    ] {
        let result = f.run("output.tif", &args);
        assert_eq!(
            result.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!f.0.join("output.tif").exists());
    }
    let result = f.run(
        "output.tif",
        &[
            "--input-color-space",
            "sRGB",
            "--input-cctf-decoding",
            "true",
            "--dry-run",
            "--dry-run-format",
            "text",
            "--seed",
            "42",
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(
        text.contains("sRGB") && text.contains("seed: 42") && text.contains("NOT CHECKED"),
        "{text}"
    );
    assert!(!f.0.join("output.tif").exists());
}

#[test]
fn seed_repeats_stochastic_pixels_and_benchmark_attempts() {
    let f = Fixture::new();
    let mut buffers = Vec::new();
    for (index, seed, iters) in [(0, "42", "1"), (1, "42", "2"), (2, "43", "1")] {
        let raw = f.0.join(format!("{index}.raw"));
        let result = f.run(&format!("{index}.tif"), &[
            "--input-color-space", "sRGB", "--input-cctf-decoding", "true",
            "--set", "camera.auto_exposure=false,film_render.grain.engine=procedural,film_render.grain.v2_profile=custom,film_render.grain.v2_amount=75",
            "--seed", seed, "--iters", iters, "--raw-out", raw.to_str().unwrap(),
        ]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
        let loaded = image::open(f.0.join(format!("{index}.tif"))).unwrap();
        assert_eq!((loaded.width(), loaded.height()), (32, 24));
        buffers.push(std::fs::read(raw).unwrap());
    }
    assert_eq!(buffers[0], buffers[1]);
    assert_ne!(buffers[0], buffers[2]);
}

#[test]
fn scanner_finish_feedback_and_text_report_follow_effective_mode() {
    let f = Fixture::new();
    for (finish, expected) in [("neutral", "inactive"), ("lab_scan", "active")] {
        let settings = format!(
            "scanner.receiver_profile=lab_scan_approx,scanner.finish={finish},scanner.finish_strength=0.5"
        );
        let result = f.run(
            "output.tif",
            &[
                "--input-color-space",
                "sRGB",
                "--input-cctf-decoding",
                "true",
                "--route",
                "input > film > scan",
                "--scan-output",
                "negative_to_positive",
                "--set",
                &settings,
                "--dry-run",
            ],
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        let edit = report["edits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|edit| edit["path"] == "scanner.finish_strength")
            .unwrap();
        assert_eq!(edit["status"], expected);
        assert_eq!(
            String::from_utf8_lossy(&result.stderr).contains("scanner.finish_strength"),
            finish == "neutral"
        );
    }
    let result = f.run(
        "output.tif",
        &[
            "--input-color-space",
            "sRGB",
            "--input-cctf-decoding",
            "true",
            "--route",
            "input > film > scan",
            "--scan-output",
            "negative_to_positive",
            "--set",
            "scanner.receiver_profile=lab_scan_approx,scanner.finish=lab_scan",
            "--dry-run",
            "--dry-run-format",
            "text",
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(
        text.contains("approximate_uncalibrated") && text.contains("fotufilm_lab_scan_approx_v1"),
        "{text}"
    );
}

#[test]
fn scanner_setup_uses_selected_seed_for_film_grain_measurement() {
    use spektrafilm_core::{image_io, params::RuntimeParams, runtime::Runtime};
    use spektrafilm_gpu::cpu_backend::CpuBackend;
    use spektrafilm_math::precision::to_f64;

    let f = Fixture::new();
    let settings = "camera.auto_exposure=false,camera.film_format_mm=0.1,film_render.halation.active=false,film_render.dir_couplers.active=false,print_render.glare.active=false,film_render.grain.engine=spektra,film_render.grain.sublayers_active=false,film_render.grain.particle_area_um2=2,scanner.auto_setup=per_frame,io.output_gamut_compress.algorithm=off,scanner.unsharp_mask=[0,0],settings.use_enlarger_lut=false,settings.use_scanner_lut=false";
    let args = [
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--route",
        "input > film > scan",
        "--scan-output",
        "negative_to_positive",
        "--set",
        settings,
        "--seed",
        "42",
    ];
    let mut dry = args.to_vec();
    dry.push("--dry-run");
    let report = f.report(&dry);
    let mut params: RuntimeParams = serde_json::from_value(report["parameters"].clone()).unwrap();
    params.random_seed = report["seed"].as_u64().unwrap();
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let runtime = Runtime::from_stocks(
        "kodak_portra_400",
        "kodak_portra_endura",
        params.clone(),
        &data,
    )
    .unwrap();
    let input = image_io::load(&f.0.join("input.png")).unwrap().image;
    let setup = runtime.prepare_scanner_setup(&input).unwrap();
    let mut wrong_seed = params;
    wrong_seed.random_seed = 0;
    let wrong =
        Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", wrong_seed, &data).unwrap();
    assert_ne!(setup, wrong.prepare_scanner_setup(&input).unwrap());
    let expected = runtime
        .with_scanner_setup(setup)
        .process(input, &CpuBackend)
        .unwrap();
    let raw = f.0.join("seed.raw");
    let mut render = args.to_vec();
    render.extend(["--raw-out", raw.to_str().unwrap()]);
    let result = f.run("seed.tif", &render);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let actual = std::fs::read(raw).unwrap();
    assert_eq!(actual.len(), expected.data.len() * 8);
    for (bytes, expected) in actual.as_chunks::<8>().0.iter().zip(expected.data) {
        assert_eq!(f64::from_le_bytes(*bytes), to_f64(expected));
    }
}

/// `--backend gpu` is an explicit request: a configuration that still needs a
/// host-executed stage (the default Spektra grain distribution is a fence) must
/// fail at assembly without publishing an output file. Omitting the flag keeps
/// automatic selection, which absorbs that stage as a CPU fence. A machine
/// without an adapter fails earlier with the backend-unavailable message.
#[test]
fn explicit_gpu_request_fails_before_publication_and_automatic_still_renders() {
    let f = Fixture::new();
    let input_flags = [
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "false",
    ];

    let explicit = Command::new(env!("CARGO_BIN_EXE_spektrafilm"))
        .arg("process")
        .arg(f.0.join("input.png"))
        .arg("--output")
        .arg(f.0.join("gpu.tif"))
        .args([
            "--film",
            "kodak_portra_400",
            "--backend",
            "gpu",
            "--data-dir",
        ])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"))
        .args(input_flags)
        .env("RUST_LOG", "off")
        .env("SPEKTRAFILM_CONFIG_DIR", f.0.join("config"))
        .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
        .env_remove("SPEKTRAFILM_DATA_DIR")
        .env_remove("SPEKTRAFILM_BACKEND")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&explicit.stderr);
    assert!(!explicit.status.success(), "{stderr}");
    assert!(
        stderr.contains("explicit GPU request") || stderr.contains("GPU backend unavailable"),
        "unexpected failure: {stderr}"
    );
    assert!(
        !f.0.join("gpu.tif").exists(),
        "a failed explicit GPU request wrote an output file"
    );

    let automatic = Command::new(env!("CARGO_BIN_EXE_spektrafilm"))
        .arg("process")
        .arg(f.0.join("input.png"))
        .arg("--output")
        .arg(f.0.join("auto.tif"))
        .args(["--film", "kodak_portra_400", "--data-dir"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"))
        .args(input_flags)
        .env("RUST_LOG", "off")
        .env("SPEKTRAFILM_CONFIG_DIR", f.0.join("config"))
        .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
        .env_remove("SPEKTRAFILM_DATA_DIR")
        .env_remove("SPEKTRAFILM_BACKEND")
        .output()
        .unwrap();
    assert!(
        automatic.status.success(),
        "{}",
        String::from_utf8_lossy(&automatic.stderr)
    );
    assert!(
        f.0.join("auto.tif").exists(),
        "automatic render published no output"
    );
}
