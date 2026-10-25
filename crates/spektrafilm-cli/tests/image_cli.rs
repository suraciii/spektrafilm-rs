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
    fn command(&self, output: &str, extra: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_spektrafilm"));
        command
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
            .env_remove("SPEKTRAFILM_BACKEND");
        command
    }
    fn run(&self, output: &str, extra: &[&str]) -> Output {
        self.command(output, extra).output().unwrap()
    }
    fn run_without_output(&self, extra: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_spektrafilm"));
        command.arg("process").arg(self.0.join("input.png"));
        let has_film = extra.windows(2).any(|pair| pair.first() == Some(&"--film"))
            || extra
                .windows(2)
                .any(|pair| pair == ["--engine", "fotufilm"]);
        if !has_film {
            command.args(["--film", "kodak_portra_400"]);
        }
        command
            .args(["--backend", "cpu", "--data-dir"])
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
    fn report_without_output(&self, extra: &[&str]) -> Value {
        let result = self.run_without_output(extra);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout).unwrap()
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
fn process_publishes_and_replaces_png_jpeg_tiff_and_openexr() {
    let f = Fixture::new();
    let directory = f.0.join("publication");
    std::fs::create_dir(&directory).unwrap();
    let sentinel = b"previous successfully published image";
    for (extension, depth, encoded) in [
        ("png", "8", "true"),
        ("jpg", "8", "true"),
        ("tif", "16", "true"),
        ("exr", "32", "false"),
    ] {
        let relative = format!("publication/output.{extension}");
        let output = f.0.join(&relative);
        for existing in [false, true] {
            if existing {
                std::fs::write(&output, sentinel).unwrap();
            }
            // The input-only route exercises the real process saving path
            // without running film simulation or stochastic effects.
            let result = f.run(
                &relative,
                &[
                    "--route",
                    "input",
                    "--input-color-space",
                    "sRGB",
                    "--input-cctf-decoding",
                    "true",
                    "--saving-color-space",
                    "sRGB",
                    "--saving-cctf-encoding",
                    encoded,
                    "--bit-depth",
                    depth,
                    "--set",
                    "io.output_color_space=sRGB,io.output_cctf_encoding=true",
                ],
            );
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(result.status.success(), "{extension}: {stderr}");
            assert!(result.stdout.is_empty(), "{extension}: {:?}", result.stdout);
            assert!(
                stderr.contains(&format!("Saved: {} (32x24)", output.display())),
                "{extension}: {stderr}"
            );
            assert_ne!(std::fs::read(&output).unwrap(), sentinel);
            let loaded = spektrafilm_core::image_io::load(&output).unwrap();
            assert_eq!((loaded.image.width, loaded.image.height), (32, 24));
            assert_eq!(loaded.image.data.len(), 32 * 24 * 3);
            assert!(loaded.image.data.iter().all(|sample| sample.is_finite()));
            if extension != "exr" {
                // EXR deliberately bypasses the Exiv2 metadata boundary.
                assert!(loaded.metadata.is_some(), "{extension} metadata missing");
                assert!(!stderr.contains("Warning:"), "{extension}: {stderr}");
            }
            // Sample-storage precision is covered at the owning core writer
            // boundary; this test proves entry-point publication and loading.
            let entries: Vec<_> = std::fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            assert_eq!(entries, vec![output.clone()], "staging files survived");
        }
        std::fs::remove_file(output).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn process_native_write_failure_preserves_absent_and_existing_outputs() {
    use std::os::unix::process::CommandExt;

    let f = Fixture::new();
    let directory = f.0.join("native-write-failure");
    std::fs::create_dir(&directory).unwrap();
    let sentinel = b"previous successfully published image";
    for extension in ["png", "jpg", "tif", "exr"] {
        let relative = format!("native-write-failure/output.{extension}");
        let output = f.0.join(&relative);
        for existing in [false, true] {
            if existing {
                std::fs::write(&output, sentinel).unwrap();
            }
            let mut command = f.command(
                &relative,
                &[
                    "--route",
                    "input",
                    "--input-color-space",
                    "sRGB",
                    "--input-cctf-decoding",
                    "true",
                    "--saving-color-space",
                    "sRGB",
                    "--saving-cctf-encoding",
                    if extension == "exr" { "false" } else { "true" },
                    "--set",
                    "io.output_color_space=sRGB,io.output_cctf_encoding=true",
                ],
            );
            // Only the exec child inherits this limit and signal disposition.
            // Every container's header/payload exceeds 64 bytes even for this
            // cheap 32x24 input. Inputs and sentinels are prepared beforehand;
            // no diagnostics/raw sidecar is requested under the file limit.
            unsafe {
                command.pre_exec(|| {
                    if libc::signal(libc::SIGXFSZ, libc::SIG_IGN) == libc::SIG_ERR {
                        return Err(std::io::Error::last_os_error());
                    }
                    let limit = libc::rlimit {
                        rlim_cur: 64,
                        rlim_max: 64,
                    };
                    if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let result = command.output().unwrap();
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(!result.status.success(), "{extension}: {stderr}");
            assert!(
                result.status.code().is_some(),
                "child died by signal: {stderr}"
            );
            assert!(result.stdout.is_empty(), "{extension}: {:?}", result.stdout);
            // Require both the CLI saving context and the native save error:
            // a parser, setup, staging-create or rendering error cannot pass.
            assert!(
                stderr.contains(&format!("saving image: {}", output.display())),
                "{extension}: {stderr}"
            );
            assert!(
                stderr.contains(&format!("Image save failed for {}:", output.display())),
                "{extension}: {stderr}"
            );
            assert!(!stderr.contains("Saved:"), "{extension}: {stderr}");
            if existing {
                assert_eq!(std::fs::read(&output).unwrap(), sentinel);
            } else {
                assert!(!output.exists(), "{extension} published failed output");
            }
            let entries: Vec<_> = std::fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            let expected = if existing {
                vec![output.clone()]
            } else {
                Vec::new()
            };
            assert_eq!(entries, expected, "{extension}: staging files survived");
        }
        std::fs::remove_file(output).unwrap();
    }
}

#[test]
fn dry_run_without_output_reports_unresolved_destination_without_writing() {
    let f = Fixture::new();
    let report = f.report_without_output(&[
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--dry-run",
    ]);
    assert_eq!(report["output"]["path"], Value::Null);
    assert_eq!(report["output"]["format"], Value::Null);
    assert_eq!(report["output"]["bit_depth"], Value::Null);
    assert_eq!(report["output"]["compression"], Value::Null);
    assert_eq!(report["output"]["jpeg_quality"], Value::Null);
    assert_eq!(report["output"]["jpeg_subsampling"], Value::Null);
    assert_eq!(report["unchecked"][4], "output_format");
    assert!(!f.0.join("output.tif").exists());
}

#[test]
fn dry_run_without_output_accepts_explicit_format_and_reports_writer_defaults() {
    let f = Fixture::new();
    let report = f.report_without_output(&[
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--format",
        "png",
        "--dry-run",
    ]);
    assert_eq!(report["output"]["path"], Value::Null);
    assert_eq!(report["output"]["format"], "png");
    assert_eq!(report["output"]["bit_depth"], 8);
    assert_eq!(report["output"]["jpeg_quality"], Value::Null);
    assert_eq!(report["output"]["compression"], Value::Null);
    assert!(!f.0.join("output.tif").exists());
}

#[test]
fn dry_run_without_output_preserves_raw_output_file() {
    let f = Fixture::new();
    let raw = f.0.join("buffer.raw");
    std::fs::write(&raw, b"existing-buffer").unwrap();
    let result = f.run_without_output(&[
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--dry-run",
        "--raw-out",
        raw.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read(raw).unwrap(), b"existing-buffer");
}

#[test]
fn dry_run_without_output_accepts_writer_flags_for_explicit_format() {
    let f = Fixture::new();
    let report = f.report_without_output(&[
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--format",
        "jpeg",
        "--jpeg-quality",
        "80",
        "--jpeg-subsampling",
        "420",
        "--dry-run",
    ]);
    assert_eq!(report["output"]["format"], "jpeg");
    assert_eq!(report["output"]["jpeg_quality"], 80);
    assert_eq!(report["output"]["jpeg_subsampling"], "420");
    assert_eq!(report["output"]["path"], Value::Null);
    assert!(!f.0.join("output.tif").exists());
}
#[test]
fn dry_run_without_output_matches_supplied_destination_parameters() {
    let f = Fixture::new();
    let with_output = f.report(&[
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--dry-run",
    ]);
    let without_output = f.report_without_output(&[
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--format",
        "tiff",
        "--dry-run",
    ]);
    assert_eq!(with_output["parameters"], without_output["parameters"]);
    for field in [
        "format",
        "bit_depth",
        "color_space",
        "cctf_encoding",
        "compression",
        "jpeg_quality",
        "jpeg_subsampling",
    ] {
        assert_eq!(
            with_output["output"][field], without_output["output"][field],
            "{field}"
        );
    }
    assert!(with_output["output"]["path"].is_string());
    assert_eq!(without_output["output"]["path"], Value::Null);
}

#[test]
fn dry_run_without_output_rejects_container_specific_options_without_format() {
    let f = Fixture::new();
    for args in [
        vec!["--jpeg-quality", "90"],
        vec!["--jpeg-subsampling", "420"],
        vec!["--compression", "zip"],
    ] {
        let mut full = vec![
            "--input-color-space",
            "sRGB",
            "--input-cctf-decoding",
            "true",
            "--dry-run",
        ];
        full.extend(args);
        let result = f.run_without_output(&full);
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("requires --format"),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!f.0.join("output.tif").exists());
    }
}

#[test]
fn ordinary_process_still_requires_output_at_argument_parsing() {
    let f = Fixture::new();
    let result = f.run_without_output(&[
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("--output"));
    assert!(result.stdout.is_empty());
}

#[test]
fn dry_run_text_without_output_does_not_fabricate_destination() {
    let f = Fixture::new();
    let result = f.run_without_output(&[
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--dry-run",
        "--dry-run-format",
        "text",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("OUTPUT"));
    assert!(text.contains("path: not selected"), "{text}");
    assert!(text.contains("format: unresolved"), "{text}");
    assert!(!text.contains("output.tif"), "{text}");
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
fn fotufilm_dry_run_reports_authored_controls_without_selecting_an_executor() {
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
    assert_eq!(provenance["implementation"], "rust_film");
    assert_eq!(provenance["effective_backend"], serde_json::Value::Null);
    assert!(
        provenance["note"]
            .as_str()
            .is_some_and(|note| note.contains("selected executor"))
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
fn dye_dry_run_does_not_claim_an_unchecked_executor() {
    let f = Fixture::new();
    let report = f.report(&[
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--set",
        "film_render.grain.engine=dye,film_render.grain.v3_dye_support_um=8",
        "--dry-run",
    ]);
    assert_eq!(report["film_grain_dye"]["active"], true);
    assert!(report["film_grain_dye"]["effective_backend"].is_null());
    assert_eq!(report["backend"]["requested"], "cpu");
    assert!(
        report["unchecked"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("compute_device"))
    );
    assert!(!f.0.join("output.tif").exists());
}

#[test]
fn dye_bw_reports_one_record_and_preserves_inactive_correlation() {
    let fixture = Fixture::new();
    for stock in ["kodak_doublex", "kodak_trix"] {
        let report = fixture.report_without_output(&[
            "--film",
            stock,
            "--scan-film",
            "--input-color-space",
            "sRGB",
            "--input-cctf-decoding",
            "true",
            "--set",
            "film_render.grain.engine=dye,film_render.grain.channel_correlation=0.35",
            "--dry-run",
        ]);
        let dye = &report["film_grain_dye"];
        assert_eq!(dye["record_layout"], "single_record");
        assert_eq!(dye["calibration_status"], "uncalibrated");
        assert!(dye["effective_backend"].is_null());
        assert_eq!(dye["channel_correlation"]["requested"], 0.35);
        assert!(dye["channel_correlation"]["effective"].is_null());
        assert_eq!(dye["channel_correlation"]["status"], "inactive");
        let edit = report["edits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|edit| edit["path"] == "film_render.grain.channel_correlation")
            .unwrap();
        assert_eq!(edit["status"], "inactive");
        assert_eq!(
            report["parameters"]["film_render"]["grain"]["channel_correlation"],
            0.35
        );
    }
}
#[test]
fn dye_positive_films_accept_direct_scan_and_preserve_native_polarity() {
    let f = Fixture::new();
    for film in ["fujifilm_provia_100f", "fujifilm_velvia_100"] {
        let result = f.run_without_output(&[
            "--film",
            film,
            "--scan-film",
            "--input-color-space",
            "sRGB",
            "--input-cctf-decoding",
            "true",
            "--set",
            "film_render.grain.engine=dye",
            "--dry-run",
        ]);
        assert!(
            result.status.success(),
            "{film}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["film_grain_dye"]["active"], true);
        assert_eq!(
            report["parameters"]["workflow"]["route"],
            "input > film > scan"
        );
        assert_eq!(
            report["parameters"]["scanner"]["scan_output"],
            "direct_scan"
        );
    }
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
fn feature_profiles_report_typed_values_precedence_and_inactive_status() {
    let f = Fixture::new();
    let report = f.report(&[
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--feature-profile",
        "grain.procedural=35mm50",
        "--feature-profile",
        "grain.procedural=35mm250",
        "--feature-profile",
        "halation=still_weak",
        "--set",
        "film_render.halation.active=false",
        "--set",
        "film_render.grain.v2_amount=25",
        "--dry-run",
    ]);
    let profiles = report["feature_profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 3);
    assert_eq!(profiles[0]["status"], "overridden");
    assert_eq!(profiles[1]["status"], "overridden");
    assert_eq!(
        profiles[1]["requested"]["film_render.grain.v2_profile"],
        "35mm250"
    );
    assert_eq!(
        profiles[1]["requested"]["film_render.grain.v2_amount"],
        Value::Null
    );
    assert_eq!(
        profiles[1]["resolved"]["film_render.grain.v2_profile"],
        "35mm250"
    );
    assert_eq!(profiles[2]["status"], "inactive");
    assert_eq!(
        report["parameters"]["film_render"]["grain"]["v2_amount"],
        25.0
    );
}

#[test]
fn feature_profiles_reject_removed_dye_family_without_output() {
    let f = Fixture::new();
    for value in ["grain=reference", "grain.dye", "grain.dye=unknown"] {
        let result = f.run_without_output(&[
            "--input-color-space",
            "sRGB",
            "--input-cctf-decoding",
            "true",
            "--feature-profile",
            value,
            "--dry-run",
        ]);
        assert!(!result.status.success(), "accepted {value}");
        assert!(!String::from_utf8_lossy(&result.stderr).is_empty());
        assert!(!f.0.join("output.tif").exists());
    }
}

#[test]
fn route_conflicts_and_dry_run_format_fail_before_rendering() {
    let f = Fixture::new();
    for args in [
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

/// Once automatic selection chooses WGPU, an impossible device working set
/// must fail before writing output, both with and without diagnostics.
#[cfg(not(feature = "precision-f64"))]
#[test]
fn selected_automatic_gpu_resource_failure_never_publishes_or_replaces_output() {
    let f = Fixture::new();
    let output = f.0.join("gpu.tif");
    let sentinel = b"previous successfully published image";
    for diagnostics in [false, true] {
        if diagnostics {
            std::fs::write(&output, sentinel).unwrap();
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_spektrafilm"));
        command
            .arg("process")
            .arg(f.0.join("input.png"))
            .arg("--output")
            .arg(&output)
            .args(["--film", "kodak_portra_400", "--data-dir"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"))
            .args([
                "--input-color-space",
                "sRGB",
                "--input-cctf-decoding",
                "false",
            ])
            .env("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "1")
            .env("RUST_LOG", "off")
            .env("SPEKTRAFILM_CONFIG_DIR", f.0.join("config"))
            .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
            .env_remove("SPEKTRAFILM_DATA_DIR")
            .env_remove("SPEKTRAFILM_BACKEND");
        let report_path = f.0.join("resource-failure.json");
        if diagnostics {
            command.arg("--diagnostics").arg(&report_path);
        }
        let result = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            !result.status.success(),
            "selected device failure succeeded: {stderr}"
        );
        assert!(
            stderr.contains("binding") || stderr.contains("working set"),
            "not a device resource failure: {stderr}"
        );
        if diagnostics {
            assert_eq!(
                std::fs::read(&output).unwrap(),
                sentinel,
                "failed GPU render replaced prior output"
            );
            let report: Value =
                serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
            assert_eq!(report["execution"]["backend_requested"], "auto");
            assert_eq!(
                report["execution"]["backend_selected"], "wgpu",
                "this acceptance requires a usable WGPU adapter"
            );
            assert_eq!(report["operation"]["outcome"], "failed");
            assert!(report["configuration"]["published_dimensions"].is_null());
            assert!(
                report["phases"].as_array().unwrap().iter().all(|phase| {
                    !matches!(
                        phase["name"].as_str(),
                        Some("saving_convert" | "file_write")
                    )
                }),
                "failed render entered output publication: {report}"
            );
        } else {
            assert!(!output.exists(), "failed GPU render published a new output");
        }
    }
}

/// Hardware absence is an initial selection decision, not a selected-device
/// failure: automatic CPU still publishes, while explicit WGPU must not.
#[cfg(target_os = "linux")]
#[test]
fn unavailable_adapter_keeps_initial_automatic_cpu_selection_valid() {
    let f = Fixture::new();
    let output = f.0.join("selection.tif");
    let run = |explicit: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_spektrafilm"));
        command
            .arg("process")
            .arg(f.0.join("input.png"))
            .arg("--output")
            .arg(&output)
            .args(["--film", "kodak_portra_400", "--data-dir"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"))
            .args([
                "--input-color-space",
                "sRGB",
                "--input-cctf-decoding",
                "false",
            ])
            .env("VK_ICD_FILENAMES", "/dev/null")
            .env("VK_DRIVER_FILES", "/dev/null")
            .env("RUST_LOG", "off")
            .env("SPEKTRAFILM_CONFIG_DIR", f.0.join("config"))
            .env_remove("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES")
            .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
            .env_remove("SPEKTRAFILM_DATA_DIR");
        if explicit {
            command.env("SPEKTRAFILM_BACKEND", "wgpu");
        } else {
            command.env_remove("SPEKTRAFILM_BACKEND");
        }
        command.output().unwrap()
    };
    let automatic = run(false);
    assert!(
        automatic.status.success(),
        "{}",
        String::from_utf8_lossy(&automatic.stderr)
    );
    assert!(
        output.exists(),
        "initial automatic CPU selection did not publish"
    );
    let published = std::fs::read(&output).unwrap();
    let explicit = run(true);
    assert!(!explicit.status.success());
    assert!(String::from_utf8_lossy(&explicit.stderr).contains("no usable WGPU adapter"));
    assert_eq!(
        std::fs::read(output).unwrap(),
        published,
        "unavailable explicit GPU replaced output"
    );
}

/// `SPEKTRAFILM_BACKEND=wgpu` is the same explicit request as `--backend gpu`:
/// when no adapter can serve it, the process fails with an actionable error
/// instead of silently rendering on CPU and publishing output. The test does
/// not depend on an adapter: it is skipped where one exists, because then the
/// explicit request legitimately succeeds.
#[test]
fn explicit_env_gpu_request_never_silently_renders_on_cpu() {
    let f = Fixture::new();
    let automatic = Command::new(env!("CARGO_BIN_EXE_spektrafilm"))
        .arg("process")
        .arg(f.0.join("input.png"))
        .arg("--output")
        .arg(f.0.join("auto.tif"))
        .args(["--film", "kodak_portra_400", "--data-dir"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"))
        .args([
            "--input-color-space",
            "sRGB",
            "--input-cctf-decoding",
            "false",
        ])
        .env("RUST_LOG", "off")
        .env("SPEKTRAFILM_CONFIG_DIR", f.0.join("config"))
        .env("SPEKTRAFILM_BACKEND", "wgpu")
        .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
        .env_remove("SPEKTRAFILM_DATA_DIR")
        .output()
        .unwrap();
    if automatic.status.success() {
        // An adapter is present: the explicit request ran on GPU, which is
        // the other accepted outcome.
        assert!(
            f.0.join("auto.tif").exists(),
            "an explicit GPU render published no output"
        );
        return;
    }
    let stderr = String::from_utf8_lossy(&automatic.stderr);
    assert!(
        stderr.contains("explicit GPU request"),
        "unexpected failure: {stderr}"
    );
    assert!(
        !f.0.join("auto.tif").exists(),
        "a failed explicit GPU request wrote an output file: {stderr}"
    );
}

#[test]
fn standard_alias_renders_identically_and_emits_only_the_canonical_style() {
    let f = Fixture::new();
    let common = [
        "--input-color-space",
        "sRGB",
        "--input-cctf-decoding",
        "true",
        "--scan-film",
        "--scan-output",
        "negative_to_positive",
        "--set",
        "camera.auto_exposure=false,debug.deactivate_spatial_effects=true,debug.deactivate_stochastic_effects=true",
    ];
    let mut args = common.to_vec();
    args.extend_from_slice(&["--set", "scanner.conversion_style=standard"]);
    let result = f.run("standard.png", &args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    *args.last_mut().unwrap() = "scanner.conversion_style=neutral";
    let result = f.run("alias.png", &args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        image::open(f.0.join("standard.png")).unwrap().to_rgb8(),
        image::open(f.0.join("alias.png")).unwrap().to_rgb8()
    );
    args.push("--dry-run");
    let result = f.run("dry.png", &args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(
        report["parameters"]["scanner"]["conversion_style"],
        "standard"
    );
    assert_eq!(report["scanner_provenance"]["model"], "lab_scan_v1");
}
