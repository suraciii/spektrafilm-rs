//! Native observation publication belongs to experiments, never ordinary process.

#![cfg(feature = "precision-f64")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        use sha2::{Digest, Sha256};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "spektrafilm-native-observation-cli-{}-{}",
            std::process::id(),
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
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(root.join("input.png")).unwrap())
        );
        std::fs::write(
            root.join("suite.json"),
            r#"{"cases":[{"id":"chart","regions":[{"id":"patch","geometry":[0,0,4,4]}]}]}"#,
        )
        .unwrap();
        std::fs::write(root.join("experiment.json"), serde_json::to_vec(&serde_json::json!({
            "schema_version": 1, "id": "chart",
            "chain": {"film": "kodak_portra_400", "input_color_space": "sRGB", "input_cctf_decoding": true},
            "input": {"path": "input.png", "sha256": digest},
            "case": {"id": "chart", "suite": "suite.json"},
            "grid": {"rows": 1, "cols": 1, "patch_px": 4, "gap_px": 0, "origin": [0,0], "patch": {"patch": [0,0]}},
            "seeds": [0]
        })).unwrap()).unwrap();
        Self(root)
    }

    fn command(&self, binary: &str) -> Command {
        let mut command = Command::new(binary);
        command
            .env("RUST_LOG", "off")
            .env("SPEKTRAFILM_CONFIG_DIR", self.0.join("config"))
            .env_remove("SPEKTRAFILM_DATA_DIR")
            .env_remove("SPEKTRAFILM_BACKEND")
            .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS");
        command
    }

    fn observe(&self, extra: &[&str]) -> Output {
        self.command(env!("CARGO_BIN_EXE_spektrafilm-lab"))
            .args(["experiment", "run", "--experiment"])
            .arg(self.0.join("experiment.json"))
            .arg("--out")
            .arg(self.0.join("observed"))
            .args(["--output-format", "png", "--force", "--data-dir"])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data"))
            .args(extra)
            .output()
            .unwrap()
    }

    fn plain(&self, extra: &[&str]) -> Output {
        self.command(env!("CARGO_BIN_EXE_spektrafilm-f64"))
            .arg("process")
            .arg(self.0.join("input.png"))
            .arg("--output")
            .arg(self.0.join("plain.png"))
            .args([
                "--film",
                "kodak_portra_400",
                "--backend",
                "cpu",
                "--input-color-space",
                "sRGB",
                "--input-cctf-decoding",
                "true",
                "--seed",
                "0",
                "--data-dir",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data"))
            .args(extra)
            .output()
            .unwrap()
    }

    fn bundle(&self) -> PathBuf {
        self.0.join("observed/bundle")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn manifest(root: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root.join("stage-evidence.json")).unwrap()).unwrap()
}

fn stage<'a>(manifest: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    manifest["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == id)
        .unwrap()
}

#[test]
fn experiment_observation_matches_uncaptured_raw_output() {
    let fixture = Fixture::new();
    assert!(!fixture.0.join("observed").exists());
    assert!(!fixture.bundle().exists());
    let observed = fixture.observe(&[]);
    assert!(
        observed.status.success(),
        "{}",
        String::from_utf8_lossy(&observed.stderr)
    );
    assert!(fixture.bundle().is_dir());
    let raw = fixture.0.join("plain.f64");
    let plain = fixture.plain(&["--raw-out", raw.to_str().unwrap()]);
    assert!(
        plain.status.success(),
        "{}",
        String::from_utf8_lossy(&plain.stderr)
    );
    assert_eq!(
        std::fs::read(fixture.bundle().join("rgb_out.f64")).unwrap(),
        std::fs::read(raw).unwrap()
    );
    let manifest = manifest(&fixture.bundle());
    assert_eq!(manifest["execution_status"], "completed");
    assert_eq!(manifest["host_precision"], "f64");
    assert!(manifest["errors"].as_array().unwrap().is_empty());
    for id in [
        "rgb_in",
        "log_e_film",
        "cmy_film",
        "log_e_print",
        "cmy_print",
        "rgb_out",
    ] {
        let record = stage(&manifest, id);
        assert_eq!(record["availability"], "available", "{id}");
        let path = Path::new(record["path"].as_str().unwrap());
        assert!(!path.is_absolute());
        assert!(fixture.bundle().join(path).is_file());
    }
    assert_eq!(
        manifest["exposure"]["source_dimensions"],
        serde_json::json!([32, 24])
    );
    assert!(!fixture.0.join("stage-evidence").exists());
}

#[test]
fn experiment_grain_observation_keeps_delivered_geometry() {
    for engine in ["dye", "fotufilm"] {
        let fixture = Fixture::new();
        let result = fixture.observe(&[
            "--set",
            "film_render.grain.active=true",
            "--set",
            &format!("film_render.grain.engine={engine}"),
            "--set",
            "io.crop=true",
            "--set",
            "io.crop_size=[0.5,0.5]",
            "--set",
            "io.upscale_factor=0.5",
            "--set",
            "scanner.unsharp_mask=[0,0]",
            "--set",
            "scanner.lens_blur=0",
            "--set",
            "print_render.glare.active=false",
            "--set",
            "enlarger.diffusion_filter.active=false",
        ]);
        assert!(
            result.status.success(),
            "{engine}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let manifest = manifest(&fixture.bundle());
        let output = stage(&manifest, "rgb_out");
        let full_source = stage(&manifest, "log_e_film");
        assert_eq!(full_source["layout"]["width"], 32);
        assert_eq!(full_source["layout"]["height"], 24);
        assert_ne!(full_source["layout"]["width"], output["layout"]["width"]);
        for id in ["cmy_film", "log_e_print", "cmy_print"] {
            let record = stage(&manifest, id);
            assert_eq!(
                record["geometry"]["coordinate_space"], "output_pixels",
                "{engine}: {id}"
            );
            assert_eq!(record["layout"]["width"], output["layout"]["width"]);
            assert_eq!(record["layout"]["height"], output["layout"]["height"]);
        }
    }
}

#[test]
fn experiment_invalid_destination_fails_without_complete_manifest() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.bundle().join("log_e_film.f64")).unwrap();
    let result = fixture.observe(&[]);
    assert!(!result.status.success());
    assert!(!fixture.0.join("observed/output.png").exists());
    assert!(!fixture.bundle().join("stage-evidence.json").exists());
}

#[test]
fn experiment_publication_clears_stale_manifest_before_failed_capture() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.bundle().join("log_e_film.f64")).unwrap();
    std::fs::write(
        fixture.bundle().join("stage-evidence.json"),
        r#"{"execution_status":"completed"}"#,
    )
    .unwrap();
    let failed = fixture.observe(&[]);
    assert!(!failed.status.success());
    assert!(!fixture.bundle().join("stage-evidence.json").exists());
    std::fs::remove_dir(fixture.bundle().join("log_e_film.f64")).unwrap();
    let completed = fixture.observe(&[]);
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    assert_eq!(manifest(&fixture.bundle())["execution_status"], "completed");
}

#[test]
fn experiment_failed_image_write_never_publishes_observation() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.0.join("observed/output.png")).unwrap();
    let result = fixture.observe(&[]);
    assert!(!result.status.success());
    assert!(!fixture.bundle().join("stage-evidence.json").exists());
}

#[test]
fn ordinary_process_rejects_retired_capture_before_output() {
    let fixture = Fixture::new();
    let result = fixture.plain(&["--stage-evidence", fixture.bundle().to_str().unwrap()]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("--stage-evidence"));
    assert!(!fixture.0.join("plain.png").exists());
    assert!(!fixture.bundle().exists());
}

#[test]
fn ordinary_suite_and_image_runs_retain_absence_without_capture_files() {
    use serde_json::json;
    let fixture = Fixture::new();
    let renderer = PathBuf::from(env!("CARGO_BIN_EXE_spektrafilm-f64"));
    let target = fixture.0.join("target.json");
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .canonicalize()
        .unwrap();
    std::fs::write(&target, serde_json::to_vec(&json!({
        "schema_version": 1, "id": "target",
        "renderer": {"path": renderer, "sha256": spektrafilm_benchmark::common::sha256_file(&renderer).unwrap()},
        "data_dir": data, "backend": "cpu", "changes": {"set_files": [], "set": []}
    })).unwrap()).unwrap();
    let configuration =
        json!({"film": "kodak_portra_400", "set": ["film_render.grain.active=false"]});
    let config = fixture.0.join("configuration.json");
    std::fs::write(&config, serde_json::to_vec(&configuration).unwrap()).unwrap();
    let suite = fixture.0.join("ordinary-suite.json");
    std::fs::write(&suite, serde_json::to_vec(&json!({
        "schema_version": 1, "id": "ordinary",
        "protocol": {"id": "appearance-v1", "pixels_per_degree": 60.0, "region_sampling_scales": [1.0],
            "grain": {"residual": {"method": "flat_mean", "high_pass_radius_px": 4.0, "border_px": 1}, "spectrum": {"window": "hann"}},
            "hotspots": {"count": 0, "min_area_px": 1}},
        "cases": [{"id": "probe", "kind": "probe", "configuration": configuration, "seeds": [0], "regions": [], "measurements": [],
            "input": {"kind": "raster", "path": "input.png", "sha256": spektrafilm_benchmark::common::sha256_file(&fixture.0.join("input.png")).unwrap(), "color_space": "sRGB", "cctf_decoding": true}}]
    })).unwrap()).unwrap();
    for mode in ["suite", "image"] {
        let out = fixture.0.join(mode);
        let mut command = fixture.command(env!("CARGO_BIN_EXE_spektrafilm-lab"));
        command
            .args(["run", mode, "--target"])
            .arg(&target)
            .arg("--out")
            .arg(&out);
        if mode == "suite" {
            command.arg("--suite").arg(&suite);
        } else {
            command
                .arg("--input")
                .arg(fixture.0.join("input.png"))
                .arg("--configuration")
                .arg(&config)
                .args([
                    "--input-color-space",
                    "sRGB",
                    "--input-cctf-decoding",
                    "true",
                ]);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let run: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("run.json")).unwrap()).unwrap();
        for row in run["cases"].as_array().unwrap() {
            assert_eq!(row["status"], "completed");
            assert_eq!(row["stage_evidence"]["requested"], false);
            assert_eq!(row["stage_evidence"]["availability"], "not_applicable");
            let observed = &row["stage_evidence"]["context"];
            assert_eq!(observed["exposure"]["availability"], "unavailable");
            assert!(observed["exposure"].get("metered_ev").is_none());
            assert_eq!(
                observed["print_normalization"]["availability"],
                "unavailable"
            );
            for stage in observed["stages"].as_array().unwrap() {
                assert_eq!(stage["availability"], "unavailable");
                assert!(stage.get("path").is_none());
            }
            for key in ["command", "source_command"] {
                assert!(
                    !row[key]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|value| value == "--stage-evidence")
                );
            }
        }
        assert!(
            !run["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|artifact| artifact["role"]
                    .as_str()
                    .is_some_and(|role| role.starts_with("stage_evidence")))
        );
    }
}

#[test]
fn experiment_observer_rejects_declaration_alias_without_overwriting_it() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.bundle()).unwrap();
    let declaration = fixture.0.join("experiment.json");
    let original = std::fs::read(&declaration).unwrap();
    std::fs::hard_link(&declaration, fixture.bundle().join("rgb_in.f64")).unwrap();
    let result = fixture.observe(&[]);
    assert!(!result.status.success());
    assert_eq!(std::fs::read(&declaration).unwrap(), original);
    assert!(!fixture.bundle().join("stage-evidence.json").exists());
    assert!(!fixture.0.join("observed/output.png").exists());
}
