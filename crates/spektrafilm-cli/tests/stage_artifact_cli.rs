use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}
fn binaries() -> Vec<&'static str> {
    let mut bins = vec![env!("CARGO_BIN_EXE_spektrafilm")];
    #[cfg(feature = "precision-f64")]
    bins.push(env!("CARGO_BIN_EXE_spektrafilm-f64"));
    bins
}
fn invoke(bin: &str, root: &Path, input: &str, flags: &[&str]) -> Output {
    Command::new(bin)
        .arg("process")
        .arg(root.join(input))
        .args(["--backend", "cpu", "--data-dir"])
        .arg(data())
        .args(flags)
        .current_dir(root)
        .env("RUST_LOG", "off")
        .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
        .env_remove("SPEKTRAFILM_DATA_DIR")
        .env_remove("SPEKTRAFILM_BACKEND")
        .output()
        .unwrap()
}
fn success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let pixels: Vec<u8> = (0..8 * 6)
        .flat_map(|i| [30 + (i * 3) as u8, 80, 170])
        .collect();
    image::save_buffer(
        root.path().join("input.png"),
        &pixels,
        8,
        6,
        image::ColorType::Rgb8,
    )
    .unwrap();
    root
}
const IMAGE: &[&str] = &[
    "--film",
    "kodak_portra_400",
    "--input-color-space",
    "sRGB",
    "--input-cctf-decoding",
    "true",
    "--set",
    "film_render.grain.active=false",
];

#[test]
fn debug_is_boolean_and_retired_capture_is_not_a_parser_option() {
    for bin in binaries() {
        for flags in [
            vec!["--debug=true"],
            vec!["--debug=false"],
            vec!["--debug", "--debug"],
            vec!["--debug", "directory"],
            vec!["--stage-evidence", "capture"],
        ] {
            let root = fixture();
            let output = invoke(
                bin,
                root.path(),
                "input.png",
                &[IMAGE, &["--dry-run"], &flags].concat(),
            );
            assert!(
                !output.status.success(),
                "accepted invalid flag form {flags:?}"
            );
            assert!(output.stdout.is_empty());
            assert!(!root.path().join("capture").exists());
        }
    }
}

#[test]
fn film_terminal_dry_run_has_no_print_or_image_writer_dependency_and_debug_preserves_stdout() {
    for bin in binaries() {
        let root = fixture();
        let flags = [IMAGE, &["--route", "input > film", "--dry-run"]].concat();
        let plain = success(invoke(bin, root.path(), "input.png", &flags));
        let debug = success(invoke(
            bin,
            root.path(),
            "input.png",
            &[flags.as_slice(), &["--debug"]].concat(),
        ));
        assert_eq!(plain.stdout, debug.stdout);
        assert!(String::from_utf8_lossy(&debug.stderr).contains("Debug:"));
        let report: Value = serde_json::from_slice(&plain.stdout).unwrap();
        assert!(report["print_profile"].is_null());
        assert_eq!(report["output"]["kind"], "stage_artifact");
        assert_eq!(report["output"]["format"], "sfstage");
        assert!(report["output"]["path"].is_null());
        assert!(report["output"]["bit_depth"].is_null());
        assert_eq!(
            report["execution"]["planned_operations"],
            serde_json::json!(["film"])
        );
        let explicit_capture = success(invoke(
            bin,
            root.path(),
            "input.png",
            &[
                flags.as_slice(),
                &["--set", "camera.capture_illuminant=D65"],
            ]
            .concat(),
        ));
        let explicit_capture: Value = serde_json::from_slice(&explicit_capture.stdout).unwrap();
        assert_eq!(explicit_capture["capture_illuminant"]["source"], "set");
        for flag in [
            vec!["--format", "tiff"],
            vec!["--bit-depth", "32"],
            vec!["--saving-cctf-encoding", "false"],
            vec!["--compression", "zip"],
        ] {
            assert!(
                !invoke(
                    bin,
                    root.path(),
                    "input.png",
                    &[flags.as_slice(), &flag].concat()
                )
                .status
                .success()
            );
        }
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

#[test]
fn all_six_stage_routes_continue_actual_material_and_preserve_direct_scan_pixels() {
    for bin in binaries() {
        let root = fixture();
        let root = root.path();
        success(invoke(
            bin,
            root,
            "input.png",
            &[
                IMAGE,
                &[
                    "--route",
                    "input > film",
                    "--seed",
                    "42",
                    "-o",
                    "film.sfstage",
                ],
            ]
            .concat(),
        ));
        let header = success(invoke(
            bin,
            root,
            "film.sfstage",
            &["--route", "film > scan", "--dry-run"],
        ));
        let report: Value = serde_json::from_slice(&header.stdout).unwrap();
        assert_eq!(report["seed"], 42);
        assert_eq!(report["seed_source"], "artifact");
        assert_eq!(report["input"]["kind"], "stage_artifact");
        assert!(report["input"]["declaration"].is_null());
        assert!(
            report["unchecked"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "artifact_integrity")
        );
        success(invoke(
            bin,
            root,
            "film.sfstage",
            &[
                "--route",
                "film > scan",
                "-o",
                "film.tif",
                "--raw-out",
                "split.raw",
            ],
        ));
        success(invoke(
            bin,
            root,
            "input.png",
            &[
                IMAGE,
                &[
                    "--route",
                    "input > film > scan",
                    "--seed",
                    "42",
                    "-o",
                    "direct.tif",
                    "--raw-out",
                    "direct.raw",
                ],
            ]
            .concat(),
        ));
        assert_eq!(
            fs::read(root.join("split.raw")).unwrap(),
            fs::read(root.join("direct.raw")).unwrap()
        );
        // Final workflow choices apply after sparse sources: an intermediate
        // scan interpretation must not reject an otherwise valid print route.
        let composed = success(invoke(
            bin,
            root,
            "film.sfstage",
            &[
                "--route",
                "film > print",
                "--paper",
                "kodak_portra_endura",
                "--set",
                "scanner.scan_output=negative_to_positive",
                "--scan-output",
                "direct_scan",
                "--dry-run",
            ],
        ));
        let composed: Value = serde_json::from_slice(&composed.stdout).unwrap();
        assert_eq!(
            composed["parameters"]["scanner"]["scan_output"],
            "direct_scan"
        );
        success(invoke(
            bin,
            root,
            "film.sfstage",
            &[
                "--route",
                "film > print",
                "--paper",
                "kodak_portra_endura",
                "-o",
                "print.sfstage",
            ],
        ));
        success(invoke(
            bin,
            root,
            "print.sfstage",
            &[
                "--route",
                "print > scan",
                "-o",
                "print.tif",
                "--raw-out",
                "print.raw",
            ],
        ));
        success(invoke(
            bin,
            root,
            "film.sfstage",
            &[
                "--route",
                "film > print > scan",
                "--paper",
                "kodak_portra_endura",
                "-o",
                "continued.tif",
                "--raw-out",
                "continued.raw",
            ],
        ));
        assert_eq!(
            fs::read(root.join("print.raw")).unwrap(),
            fs::read(root.join("continued.raw")).unwrap()
        );
        success(invoke(
            bin,
            root,
            "input.png",
            &[
                IMAGE,
                &[
                    "--route",
                    "input > film > print",
                    "--paper",
                    "kodak_portra_endura",
                    "--seed",
                    "42",
                    "-o",
                    "image-print.sfstage",
                ],
            ]
            .concat(),
        ));
        success(invoke(
            bin,
            root,
            "image-print.sfstage",
            &[
                "--route",
                "print > scan",
                "-o",
                "image-print.tif",
                "--raw-out",
                "image-print.raw",
            ],
        ));
        assert_eq!(
            fs::read(root.join("print.raw")).unwrap(),
            fs::read(root.join("image-print.raw")).unwrap()
        );
        for flags in [
            vec!["--dry-run"],
            vec!["--route", "print > scan", "--dry-run"],
            vec![
                "--route",
                "film > scan",
                "--input-color-space",
                "sRGB",
                "--dry-run",
            ],
            vec![
                "--route",
                "film > scan",
                "--raw-white-balance",
                "as-shot",
                "--dry-run",
            ],
            vec![
                "--route",
                "film > scan",
                "--set",
                "camera.exposure_compensation_ev=1",
                "--set",
                "camera.exposure_compensation_ev=0",
                "--dry-run",
            ],
        ] {
            assert!(
                !invoke(bin, root, "film.sfstage", &flags).status.success(),
                "accepted {flags:?}"
            );
        }
    }
}
