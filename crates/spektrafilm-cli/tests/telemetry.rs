use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture {
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "spektrafilm-cli-telemetry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        image::save_buffer(
            directory.join("input.png"),
            &[40, 90, 170].repeat(16),
            4,
            4,
            image::ColorType::Rgb8,
        )
        .unwrap();
        fs::write(directory.join("params.json"), serde_json::to_vec(&json!({"camera":{"auto_exposure":false},"debug":{"deactivate_spatial_effects":true,"deactivate_stochastic_effects":true}})).unwrap()).unwrap();
        Self { directory }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
    fn process(&self, name: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_spektrafilm"));
        command
            .args(["process"])
            .arg(self.path("input.png"))
            .arg("--output")
            .arg(self.path(name))
            .args([
                "--backend",
                "cpu",
                "--film",
                "kodak_portra_400",
                "--paper",
                "kodak_portra_endura",
                "--params",
            ])
            .arg(self.path("params.json"))
            .arg("--data-dir")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"))
            .env("RUST_LOG", "off");
        command
    }
    fn report(&self, name: &str) -> Value {
        let report: spektrafilm_core::telemetry::Report =
            serde_json::from_slice(&fs::read(self.path(name)).unwrap()).unwrap();
        report.validate().unwrap();
        serde_json::to_value(report).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn diagnostics_preserve_pixels_stdout_and_retain_indexed_attempts() {
    let fixture = Fixture::new();
    let off = fixture.process("off.png").output().unwrap();
    success(&off);
    let on = fixture
        .process("on.png")
        .arg("--diagnostics")
        .arg(fixture.path("report.json"))
        .args(["--iters", "3", "--timings"])
        .output()
        .unwrap();
    success(&on);
    assert_eq!(off.stdout, on.stdout);
    assert!(on.stdout.is_empty());
    let off_image = image::open(fixture.path("off.png")).unwrap();
    let on_image = image::open(fixture.path("on.png")).unwrap();
    assert_eq!(
        (off_image.width(), off_image.height(), off_image.color()),
        (on_image.width(), on_image.height(), on_image.color())
    );
    assert_eq!(off_image.as_bytes(), on_image.as_bytes());
    let report = fixture.report("report.json");
    assert_eq!(report["operation"]["outcome"], "succeeded");
    assert_eq!(report["attempts"].as_array().unwrap().len(), 3);
    for (index, attempt) in report["attempts"].as_array().unwrap().iter().enumerate() {
        assert_eq!(attempt["attempt_index"], (index + 1) as u64);
        assert_eq!(attempt["outcome"], "succeeded");
    }
    assert!(
        report["phases"]
            .as_array()
            .unwrap()
            .iter()
            .any(|phase| phase["name"] == "file_write")
    );
    assert!(String::from_utf8_lossy(&on.stderr).contains("Diagnostics report saved:"));
}

#[test]
fn existing_report_does_not_change_success_or_clobber() {
    let fixture = Fixture::new();
    fs::write(fixture.path("report.json"), b"retain me").unwrap();
    let output = fixture
        .process("output.png")
        .arg("--diagnostics")
        .arg(fixture.path("report.json"))
        .output()
        .unwrap();
    success(&output);
    assert!(fixture.path("output.png").is_file());
    assert_eq!(fs::read(fixture.path("report.json")).unwrap(), b"retain me");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Diagnostics report could not be saved:")
    );
}

#[test]
fn failed_validation_and_missing_input_produce_failed_reports() {
    let fixture = Fixture::new();
    let invalid = fixture
        .process("invalid.unknown")
        .arg("--diagnostics")
        .arg(fixture.path("invalid.json"))
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    let report = fixture.report("invalid.json");
    assert_eq!(report["operation"]["outcome"], "failed");
    assert!(report["configuration"]["input_dimensions"].is_null());
    assert!(report["attempts"].as_array().unwrap().is_empty());
    let missing = Command::new(env!("CARGO_BIN_EXE_spektrafilm"))
        .arg("render")
        .arg("--input")
        .arg(fixture.path("missing.tiff"))
        .arg("--recipe")
        .arg(fixture.path("missing-recipe.json"))
        .arg("--output")
        .arg(fixture.path("output.png"))
        .arg("--diagnostics")
        .arg(fixture.path("missing.json"))
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert_eq!(
        fixture.report("missing.json")["operation"]["outcome"],
        "failed"
    );
}

#[test]
fn invalid_configuration_retains_failure_without_image_publication() {
    let fixture = Fixture::new();
    fs::write(
        fixture.path("params.json"),
        b"{\"unknown_parameter\": true}",
    )
    .unwrap();
    let output = fixture
        .process("output.png")
        .arg("--diagnostics")
        .arg(fixture.path("failed.json"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!fixture.path("output.png").exists());
    assert_eq!(
        fixture.report("failed.json")["operation"]["outcome"],
        "failed"
    );
}

#[test]
fn protected_destinations_and_invalid_options_are_rejected_before_image_work() {
    let fixture = Fixture::new();
    for name in ["input.png", "params.json", "output.png", "raw.bin"] {
        let original = fs::read(fixture.path(name)).ok();
        let output = fixture
            .process("output.png")
            .arg("--raw-out")
            .arg(fixture.path("raw.bin"))
            .arg("--diagnostics")
            .arg(fixture.path(name))
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(fs::read(fixture.path(name)).ok(), original);
        assert!(!fixture.path("output.png").exists());
    }
    let output = fixture
        .process("output.png")
        .arg("--gpu-timings")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!fixture.path("output.png").exists());
    let output = Command::new(env!("CARGO_BIN_EXE_spektrafilm"))
        .args(["describe", "--diagnostics", "report.json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn alias_and_nonregular_report_paths_are_rejected() {
    let fixture = Fixture::new();
    fs::hard_link(fixture.path("input.png"), fixture.path("hard.json")).unwrap();
    let mut paths = vec![
        fixture.path("hard.json"),
        fixture.directory.clone(),
        PathBuf::from("-"),
    ];
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(fixture.path("params.json"), fixture.path("sym.json")).unwrap();
        paths.push(fixture.path("sym.json"));
        paths.push(PathBuf::from("/dev/null"));
    }
    for path in paths {
        let output = fixture
            .process("output.png")
            .arg("--diagnostics")
            .arg(path)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!fixture.path("output.png").exists());
    }
}

#[test]
fn cpu_gpu_timing_request_retains_summary_and_valid_image() {
    let fixture = Fixture::new();
    let output = fixture
        .process("output.png")
        .arg("--diagnostics")
        .arg(fixture.path("timing.json"))
        .arg("--gpu-timings")
        .output()
        .unwrap();
    success(&output);
    let report = fixture.report("timing.json");
    assert_eq!(
        report["configuration"]["collection_mode_requested"],
        "gpu_timing"
    );
    assert_eq!(report["execution"]["backend_selected"], "cpu");
}

#[cfg(target_os = "linux")]
#[test]
fn unwritable_report_parent_preserves_image_success() {
    let fixture = Fixture::new();
    let output = fixture
        .process("output.png")
        .arg("--diagnostics")
        .arg(format!(
            "/proc/self/spektrafilm-{}.json",
            std::process::id()
        ))
        .output()
        .unwrap();
    success(&output);
    assert!(fixture.path("output.png").is_file());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Diagnostics report could not be saved:")
    );
}

#[test]
fn render_keeps_machine_result_and_hash_inside_terminal_boundary() {
    let fixture = Fixture::new();
    let input = fixture.path("input.tiff");
    let mut encoder = tiff::encoder::TiffEncoder::new(fs::File::create(&input).unwrap()).unwrap();
    encoder
        .write_image::<tiff::encoder::colortype::RGB32Float>(4, 4, &[0.1_f32, 0.2, 0.3].repeat(16))
        .unwrap();
    drop(encoder);
    fs::write(fixture.path("recipe.json"), serde_json::to_vec(&json!({
        "schemaVersion":"spektrafilm-rs-params-1", "filmProfile":"kodak_portra_400", "printProfile":"kodak_portra_endura",
        "parameters":{"camera":{"auto_exposure":false},"debug":{"deactivate_spatial_effects":true,"deactivate_stochastic_effects":true}},
        "output":{"format":"png","precisionBits":8,"colorSpace":"sRGB","transferFunction":"srgb","geometry":"bounded","encoding":"bounded-preview","maxEdge":4},"seed":42
    })).unwrap()).unwrap();
    let run = |name: &str, diagnostics: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_spektrafilm"));
        command
            .arg("render")
            .arg("--input")
            .arg(&input)
            .arg("--recipe")
            .arg(fixture.path("recipe.json"))
            .arg("--output")
            .arg(fixture.path(name))
            .arg("--data-dir")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"))
            .env("SPEKTRAFILM_BACKEND", "cpu")
            .env("RUST_LOG", "off");
        if diagnostics {
            command
                .arg("--diagnostics")
                .arg(fixture.path("render.json"));
        }
        let output = command.output().unwrap();
        success(&output);
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let off = run("off.png", false);
    let on = run("on.png", true);
    let off_pixels = image::open(fixture.path("off.png")).unwrap().into_rgb8();
    let on_pixels = image::open(fixture.path("on.png")).unwrap().into_rgb8();
    assert_eq!(off_pixels, on_pixels);
    for (name, result) in [("off.png", &off), ("on.png", &on)] {
        use sha2::{Digest, Sha256};
        let digest = format!(
            "{:x}",
            Sha256::digest(fs::read(fixture.path(name)).unwrap())
        );
        assert_eq!(result["output"]["sha256"], digest);
    }
    assert_eq!(off["terminalOutcome"], on["terminalOutcome"]);
    let report = fixture.report("render.json");
    assert_eq!(report["operation"]["kind"], "render");
    for name in ["file_write", "publication", "result_prepare"] {
        assert!(
            report["phases"]
                .as_array()
                .unwrap()
                .iter()
                .any(|phase| phase["name"] == name)
        );
    }
}
