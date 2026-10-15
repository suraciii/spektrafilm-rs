//! `process --stage-evidence` end-to-end behavior through the real CLI:
//! bundle publication and path layout, raw-out equivalence of the captured
//! `rgb_out`, capture-off output equality, and visible failures that must not
//! look like complete evidence.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "spektrafilm-stage-evidence-cli-{}-{}-{}",
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
                "--input-color-space",
                "sRGB",
                "--input-cctf-decoding",
                "true",
                "--data-dir",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data"))
            .args(extra)
            .env("RUST_LOG", "off")
            .env("SPEKTRAFILM_CONFIG_DIR", self.0.join("config"))
            .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
            .env_remove("SPEKTRAFILM_DATA_DIR")
            .env_remove("SPEKTRAFILM_BACKEND")
            .output()
            .unwrap()
    }
}

fn manifest(root: &std::path::Path) -> serde_json::Value {
    let text = std::fs::read_to_string(root.join("stage-evidence.json")).unwrap();
    serde_json::from_str(&text).unwrap()
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
fn stage_evidence_bundle_matches_raw_out_and_capture_off_output() {
    let fixture = Fixture::new();
    let bundle = fixture.0.join("bundle");
    let raw = fixture.0.join("raw.f64");
    let captured = fixture.run(
        "captured.png",
        &[
            "--stage-evidence",
            bundle.to_str().unwrap(),
            "--raw-out",
            raw.to_str().unwrap(),
        ],
    );
    assert!(
        captured.status.success(),
        "{}",
        String::from_utf8_lossy(&captured.stderr)
    );
    let plain_raw = fixture.0.join("plain.f64");
    let plain = fixture.run("plain.png", &["--raw-out", plain_raw.to_str().unwrap()]);
    assert!(plain.status.success());

    // Compare native samples, not file metadata (the writer records timestamps).
    assert_eq!(
        std::fs::read(&raw).unwrap(),
        std::fs::read(plain_raw).unwrap()
    );

    let manifest = manifest(&bundle);
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["execution_status"], "completed");
    assert_eq!(manifest["normalization_mode"], "profile_prepared");
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
        // Buffer paths are bundle-relative.
        let path = std::path::Path::new(record["path"].as_str().unwrap());
        assert!(!path.is_absolute(), "{id}");
        assert!(bundle.join(path).is_file(), "{id}");
    }
    // The captured final stage is the raw-out-equivalent pre-saving output.
    assert_eq!(
        std::fs::read(bundle.join("rgb_out.f64")).unwrap(),
        std::fs::read(&raw).unwrap()
    );
    // Full-source metering source equals the rgb_in grid.
    assert_eq!(
        manifest["exposure"]["source_dimensions"],
        serde_json::json!([
            stage(&manifest, "rgb_in")["layout"]["width"],
            stage(&manifest, "rgb_in")["layout"]["height"]
        ])
    );
}

#[test]
fn stage_evidence_grain_readout_keeps_its_delivered_geometry() {
    for engine in ["dye", "fotufilm"] {
        let fixture = Fixture::new();
        let bundle = fixture.0.join("bundle");
        let result = fixture.run(
            "captured.png",
            &[
                "--stage-evidence",
                bundle.to_str().unwrap(),
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
            ],
        );
        assert!(
            result.status.success(),
            "{engine}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let manifest = manifest(&bundle);
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
            assert_eq!(
                record["layout"]["width"], output["layout"]["width"],
                "{engine}: {id}"
            );
            assert_eq!(
                record["layout"]["height"], output["layout"]["height"],
                "{engine}: {id}"
            );
        }
    }
}

#[test]
fn stage_evidence_invalid_destination_fails_before_rendering() {
    let fixture = Fixture::new();
    let bundle = fixture.0.join("bundle");
    std::fs::create_dir_all(bundle.join("log_e_film.f64")).unwrap();
    let result = fixture.run(
        "output.png",
        &["--stage-evidence", bundle.to_str().unwrap()],
    );
    assert!(
        !result.status.success(),
        "a failed stage write must fail the command"
    );
    assert!(!fixture.0.join("output.png").exists());
    assert!(!bundle.join("stage-evidence.json").exists());
}

#[test]
fn stage_evidence_publication_is_terminal_over_a_stale_manifest() {
    let fixture = Fixture::new();
    let bundle = fixture.0.join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    // A stale manifest from an earlier run must not survive into a run that
    // fails before publication.
    std::fs::write(
        bundle.join("stage-evidence.json"),
        "{\"execution_status\":\"completed\"}",
    )
    .unwrap();
    std::fs::create_dir_all(bundle.join("log_e_film.f64")).unwrap();
    let failed = fixture.run(
        "output.png",
        &["--stage-evidence", bundle.to_str().unwrap()],
    );
    assert!(!failed.status.success());
    assert!(!bundle.join("stage-evidence.json").exists());
    std::fs::remove_dir_all(bundle.join("log_e_film.f64")).unwrap();
    let ok = fixture.run(
        "second.png",
        &["--stage-evidence", bundle.to_str().unwrap()],
    );
    assert!(ok.status.success());
    assert_eq!(manifest(&bundle)["execution_status"], "completed");
}

#[test]
fn stage_evidence_dry_run_writes_no_bundle() {
    let fixture = Fixture::new();
    let bundle = fixture.0.join("bundle");
    let result = fixture.run(
        "unused.png",
        &["--dry-run", "--stage-evidence", bundle.to_str().unwrap()],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!bundle.join("stage-evidence.json").exists());
}

#[test]
fn stage_evidence_rejects_preview_mode_actionably() {
    let fixture = Fixture::new();
    let bundle = fixture.0.join("bundle");
    let result = fixture.run(
        "output.png",
        &[
            "--stage-evidence",
            bundle.to_str().unwrap(),
            "--set",
            "settings.preview_mode=true",
            "--set",
            "settings.preview_max_size=16",
        ],
    );
    assert!(
        !result.status.success(),
        "preview mode must not produce stage evidence"
    );
    assert!(!bundle.join("stage-evidence.json").exists());
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn stage_evidence_rejects_raw_output_alias_before_publication() {
    let fixture = Fixture::new();
    let bundle = fixture.0.join("bundle");
    let alias = bundle.join("rgb_in.f64");
    let result = fixture.run(
        "output.png",
        &[
            "--stage-evidence",
            bundle.to_str().unwrap(),
            "--raw-out",
            alias.to_str().unwrap(),
        ],
    );
    assert!(
        !result.status.success(),
        "raw-out must not overwrite a captured stage"
    );
    assert!(!bundle.join("stage-evidence.json").exists());
    assert!(!fixture.0.join("output.png").exists());
}
