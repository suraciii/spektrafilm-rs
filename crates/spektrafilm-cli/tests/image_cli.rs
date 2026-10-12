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
fn fixed_input_precedence_and_winning_edit_feedback_are_observable() {
    let f = Fixture::new();
    let report = f.report(&[
        "--input-color-space", "sRGB", "--input-cctf-decoding", "true",
        "--set", "io.input_color_space=ProPhoto RGB,io.input_cctf_decoding=false",
        "--set", "film_render.grain.engine=v2,film_render.grain.v2_profile=35mm250,film_render.grain.v2_size=12",
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
            "--set", "camera.auto_exposure=false,film_render.grain.engine=v2,film_render.grain.v2_profile=custom,film_render.grain.v2_amount=75",
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
