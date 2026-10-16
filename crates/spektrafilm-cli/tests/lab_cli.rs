//! `spektrafilm-lab spectral evaluate` end-to-end.

#![cfg(feature = "precision-f64")]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture {
    root: PathBuf,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "spektrafilm-lab-cli-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");

        let mut missing = vec![0.184f64; 81];
        missing[40] = f64::NAN;
        write_npy_matrix(
            &root.join("reflectances.npy"),
            &[vec![0.184f64; 81], vec![0.05f64; 81], missing],
        );
        std::fs::write(
            root.join("manifest.json"),
            r#"{
                "schema_version": 1,
                "name": "synthetic roundtrip",
                "grid_nm": {"start": 380.0, "end": 780.0, "step": 5.0},
                "valid_band": [380.0, 780.0],
                "reflectances": {"path": "reflectances.npy"},
                "samples": [
                    {"id": "midgray", "source": "synthetic", "partition": "neutral", "row": 0},
                    {"id": "dark", "source": "synthetic", "partition": "neutral", "row": 1},
                    {"id": "missing", "source": "synthetic", "partition": "neutral", "row": 2}
                ]
            }"#,
        )
        .unwrap();
        Self { root, data }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
            .args(args)
            .env("RUST_LOG", "off")
            .env("SPEKTRAFILM_DATA_DIR", &self.data)
            .env("SPEKTRAFILM_CONFIG_DIR", self.root.join("config"))
            .output()
            .unwrap()
    }

    fn evaluate(&self, out: &Path, extra: &[&str]) -> Output {
        let spectra = self.root.join("manifest.json");
        let profile = self.data.join("profiles/kodak_portra_400.json");
        let mut args = vec![
            "spectral",
            "evaluate",
            "--spectra",
            spectra.to_str().unwrap(),
            "--profile",
            profile.to_str().unwrap(),
            "--condition",
            "D55=D65",
            "--out",
            out.to_str().unwrap(),
            "--data-dir",
            self.data.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        self.run(&args)
    }
}

fn write_npy_matrix(path: &Path, rows: &[Vec<f64>]) {
    let columns = rows[0].len();
    let header = format!(
        "{{'descr': '<f8', 'fortran_order': False, 'shape': ({}, {columns}), }}",
        rows.len()
    );
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x93NUMPY");
    bytes.push(1);
    bytes.push(0);
    bytes.extend_from_slice(&(header.len() as u16).to_le_bytes());
    bytes.extend_from_slice(header.as_bytes());
    for row in rows {
        for value in row {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(path, bytes).unwrap();
}

fn csv_row<'a>(text: &'a str, id: &str) -> Vec<&'a str> {
    text.lines()
        .find(|line| line.starts_with(&format!("{id},")))
        .unwrap_or_else(|| panic!("no csv row for {id}"))
        .split(',')
        .collect()
}

#[test]
fn evaluates_and_writes_a_deterministic_report() {
    let f = Fixture::new();
    let first = f.root.join("first");
    let result = f.evaluate(&first, &[]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    let report: Value =
        serde_json::from_slice(&std::fs::read(first.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["counts"]["total"], 3);
    assert_eq!(report["counts"]["valid"], 2);
    assert_eq!(report["counts"]["invalid"], 1);
    let anchors = report["normalization"]["true_anchor_raw_rgb"]
        .as_array()
        .unwrap();
    assert_eq!(anchors.len(), 3);
    assert!(anchors.iter().all(|value| value.as_f64().unwrap() > 0.0));
    let groups = report["groups"].as_object().unwrap();
    for name in ["all", "source:synthetic", "partition:neutral"] {
        assert!(groups.contains_key(name), "missing group {name}");
    }
    assert_eq!(report["groups"]["all"]["valid"], 2);
    assert_eq!(report["condition"]["reference"], "D55");
    assert_eq!(report["condition"]["target"], "D65");
    assert!(report["groups"]["all"]["rho"]["max"].as_f64().unwrap() >= 0.0);

    let csv = std::fs::read_to_string(first.join("samples.csv")).unwrap();
    let midgray = csv_row(&csv, "midgray");
    assert_eq!(midgray[4], "true");
    let rho: f64 = midgray[5].parse().unwrap();
    assert!(rho < 1e-9, "flat mid-gray sample rho {rho}");
    let missing = csv_row(&csv, "missing");
    assert_eq!(missing[4], "false");
    assert!(missing[5].is_empty(), "invalid rows carry no metrics");

    // Repeated runs on identical inputs produce identical outputs.
    let second = f.root.join("second");
    let result = f.evaluate(&second, &[]);
    assert!(result.status.success());
    for name in ["report.json", "samples.csv"] {
        assert_eq!(
            std::fs::read(first.join(name)).unwrap(),
            std::fs::read(second.join(name)).unwrap(),
            "{name} differs between identical runs"
        );
    }

    // The command refuses to replace an existing report.
    let result = f.evaluate(&first, &[]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("refusing to replace"), "{error}");
}

#[test]
fn preset_mode_resolves_the_chain_and_reports_mismatches() {
    let f = Fixture::new();
    let out = f.root.join("preset");
    let result = f.evaluate(&out, &["--preset", "Kodak Portra 400"]);
    // --profile and --preset conflict; rerun without --profile.
    assert!(!result.status.success());

    let result = f.run(&[
        "spectral",
        "evaluate",
        "--spectra",
        f.root.join("manifest.json").to_str().unwrap(),
        "--preset",
        "Kodak Portra 400",
        "--condition",
        "D55=D65",
        "--out",
        out.to_str().unwrap(),
        "--data-dir",
        f.data.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["chain"]["film"]["stock"], "kodak_portra_400");

    // A reference illuminant that does not match the profile is rejected.
    let mismatch = f.root.join("mismatch");
    let spectra = f.root.join("manifest.json");
    let profile = f.data.join("profiles/kodak_portra_400.json");
    let result = f.run(&[
        "spectral",
        "evaluate",
        "--spectra",
        spectra.to_str().unwrap(),
        "--profile",
        profile.to_str().unwrap(),
        "--condition",
        "T=D65",
        "--out",
        mismatch.to_str().unwrap(),
        "--data-dir",
        f.data.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("reference illuminant"), "{error}");
    assert!(!mismatch.join("report.json").exists());
}
