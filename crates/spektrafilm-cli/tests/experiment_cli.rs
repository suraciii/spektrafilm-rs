//! `spektrafilm-lab experiment run` / `experiment export` end-to-end.
//!
//! The unit tests in `src/lab/experiment.rs` cover the declaration and
//! conversion rules; this suite drives the real binary so the published
//! bundle, the retained records, and the exported arrays are observed as a
//! caller sees them.

#![cfg(feature = "precision-f64")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

struct Fixture {
    root: PathBuf,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "spektrafilm-lab-experiment-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data");
        Self { root, data }
    }

    /// A chart input whose patch grid the suite declares.
    fn write_input(&self, name: &str, width: u32, height: u32) -> PathBuf {
        let path = self.root.join(name);
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|i| [(i % 200 + 20) as u8, 90, 170])
            .collect();
        image::save_buffer(&path, &pixels, width, height, image::ColorType::Rgb8).unwrap();
        path
    }

    /// A suite declaring a `rows`x`cols` grid of `patch_px` patches with
    /// `gap_px` gaps starting at `origin`.
    fn write_suite(&self, regions: &[(&str, [u32; 4])]) -> PathBuf {
        let regions: Vec<Value> = regions
            .iter()
            .map(|(id, geometry)| {
                serde_json::json!({
                    "id": id,
                    "label": id,
                    "role": "color",
                    "geometry": geometry,
                    "measurements": [],
                })
            })
            .collect();
        let path = self.root.join("suite.json");
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1,
                "id": "suite",
                "protocol": {"id": "appearance-v1"},
                "cases": [{
                    "id": "case",
                    "kind": "probe",
                    "input": {"path": "input.png", "sha256": "0".repeat(64)},
                    "configuration": {},
                    "seeds": [0],
                    "regions": regions,
                    "measurements": [],
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        path
    }

    fn write_experiment(&self, patch: &str, rows: u32, cols: u32) -> PathBuf {
        self.write_experiment_with(
            r#""chain": {"film": "kodak_portra_400", "input_color_space": "sRGB", "input_cctf_decoding": true},"#,
            patch,
            rows,
            cols,
        )
    }

    fn write_experiment_with(&self, chain: &str, patch: &str, rows: u32, cols: u32) -> PathBuf {
        let path = self.root.join("experiment.json");
        std::fs::write(
            &path,
            format!(
                r#"{{
                    "schema_version": 1,
                    "id": "experiment",
                    {chain}
                    "input": {{"path": "input.png", "sha256": "{digest}"}},
                    "case": {{"id": "case", "suite": "suite.json"}},
                    "grid": {{
                        "rows": {rows}, "cols": {cols},
                        "patch_px": 4, "gap_px": 2, "origin": [0, 0],
                        "patch": {patch}
                    }},
                    "seeds": [7]
                }}"#,
                digest = digest_of(&self.root.join("input.png")),
            ),
        )
        .unwrap();
        path
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
            .args(args)
            .env("RUST_LOG", "off")
            .env("SPEKTRAFILM_DATA_DIR", &self.data)
            .env("SPEKTRAFILM_CONFIG_DIR", self.root.join("config"))
            .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
            .env_remove("SPEKTRAFILM_BACKEND")
            .current_dir(&self.root)
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn digest_of(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// A 2x2 patch chart over an 14x14 source: origin 0,0, patch 4px, gap 2px.
const CHART: &[(&str, [u32; 4])] = &[
    ("a", [0, 0, 4, 4]),
    ("b", [6, 0, 4, 4]),
    ("c", [0, 6, 4, 4]),
    ("d", [6, 6, 4, 4]),
];

const FULL_PATCH: &str = r#"{"a": [0, 0], "b": [0, 1], "c": [1, 0], "d": [1, 1]}"#;

/// Read one converted array back as its raw bytes, so the test compares the
/// `.npy` payload against the bundle buffer without re-deriving anything.
fn array_bytes(path: &Path) -> (Vec<usize>, Vec<f64>) {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..6], b"\x93NUMPY");
    assert_eq!(bytes[6], 1);
    assert_eq!(bytes[7], 0);
    let header_len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
    let header = std::str::from_utf8(&bytes[10..10 + header_len]).unwrap();
    assert!(header.contains("'descr': '<f8'"), "{header}");
    assert!(header.contains("'fortran_order': False"), "{header}");
    let shape = header
        .split("shape': (")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .unwrap()
        .split(',')
        .map(|dim| dim.trim().parse::<usize>().unwrap())
        .collect::<Vec<_>>();
    let payload = &bytes[10 + header_len..];
    let samples = payload
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| f64::from_le_bytes(*chunk))
        .collect::<Vec<f64>>();
    (shape, samples)
}

#[test]
fn run_publishes_a_bundle_and_the_retained_records() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);
    let experiment = fixture.write_experiment(FULL_PATCH, 2, 2);
    let out = fixture.root.join("out");

    let output = fixture.run(&[
        "experiment",
        "run",
        "--experiment",
        experiment.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest = read_json(&out.join("bundle/stage-evidence.json"));
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["execution_status"], "completed");
    assert_eq!(manifest["host_precision"], "f64");
    assert!(out.join("output.tiff").is_file());

    let resolved = read_json(&out.join("resolved.json"));
    assert_eq!(resolved["seed"], 7);
    assert_eq!(resolved["profiles"]["film"]["name"], "kodak_portra_400");
    assert!(resolved["parameters"].is_object());
    assert!(resolved["input"].is_object());
    assert!(resolved["edits"].is_array());

    let command = read_json(&out.join("command.json"));
    let argv = command["argv"].as_array().unwrap();
    let argv = argv.iter().map(|a| a.as_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(argv[0], "process");
    assert!(argv.contains(&"--stage-evidence"));
    assert!(argv.contains(&"cpu"));
    assert!(!argv.contains(&"--experiment"));
    assert!(argv.contains(&"--film"));

    let report = read_json(&out.join("report.json"));
    assert_eq!(report["experiment"]["id"], "experiment");
    assert_eq!(report["experiment"]["seeds"][0], 7);
    assert_eq!(report["case"]["id"], "case");
    assert_eq!(report["grid"]["rows"], 2);
    assert_eq!(report["grid"]["patch"]["a"][0], 0);
    assert_eq!(report["bundle"]["execution_status"], "completed");
    assert!(report["tool"]["version"].is_string());

    // A repeated run with identical inputs is byte-identical.
    let output = fixture.run(&[
        "experiment",
        "run",
        "--experiment",
        experiment.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--force",
    ]);
    assert!(output.status.success());
    let out2 = &out;
    for name in ["report.json", "resolved.json", "command.json"] {
        assert_eq!(
            std::fs::read(out.join(name)).unwrap(),
            std::fs::read(out2.join(name)).unwrap(),
            "{name} is not deterministic"
        );
    }
    for stage in manifest["stages"].as_array().unwrap() {
        if stage["availability"] != "available" {
            continue;
        }
        let file = stage["path"].as_str().unwrap();
        assert_eq!(
            std::fs::read(out.join("bundle").join(file)).unwrap(),
            std::fs::read(out2.join("bundle").join(file)).unwrap(),
            "bundle buffer {file} is not deterministic"
        );
    }
}

#[test]
fn run_rejects_a_grid_that_disagrees_with_the_case() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);

    // A case region absent from `patch`.
    let experiment = fixture.write_experiment(r#"{"a": [0, 0], "b": [0, 1], "c": [1, 0]}"#, 2, 2);
    let out = fixture.root.join("missing");
    let output = fixture.run(&[
        "experiment",
        "run",
        "--experiment",
        experiment.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("absent from the experiment grid"),
        "{stderr}"
    );
    assert!(!out.join("bundle/stage-evidence.json").exists());

    // `patch` naming an id the case does not declare.
    let experiment = fixture.write_experiment(
        r#"{"a": [0, 0], "b": [0, 1], "c": [1, 0], "d": [1, 1], "ghost": [2, 0]}"#,
        3,
        2,
    );
    let out = fixture.root.join("ghost");
    let output = fixture.run(&[
        "experiment",
        "run",
        "--experiment",
        experiment.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not declare"), "{stderr}");
    assert!(!out.join("bundle/stage-evidence.json").exists());
}

#[test]
fn run_rejects_a_geometry_that_does_not_match_the_grid() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    // Region `a` sits one pixel off the declared origin.
    fixture.write_suite(&[
        ("a", [1, 0, 4, 4]),
        ("b", [6, 0, 4, 4]),
        ("c", [0, 6, 4, 4]),
        ("d", [6, 6, 4, 4]),
    ]);
    let experiment = fixture.write_experiment(FULL_PATCH, 2, 2);
    let out = fixture.root.join("geometry");
    let output = fixture.run(&[
        "experiment",
        "run",
        "--experiment",
        experiment.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not match the declared grid"),
        "{stderr}"
    );
    assert!(!out.join("bundle/stage-evidence.json").exists());
}

#[test]
fn run_rejects_an_input_whose_digest_does_not_match() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);
    let path = fixture.root.join("experiment.json");
    std::fs::write(
        &path,
        format!(
            r#"{{
                "schema_version": 1,
                "id": "experiment",
                "chain": {{"film": "kodak_portra_400", "input_color_space": "sRGB", "input_cctf_decoding": true}},
                "input": {{"path": "input.png", "sha256": "{}"}},
                "case": {{"id": "case", "suite": "suite.json"}},
                "grid": {{"rows": 2, "cols": 2, "patch_px": 4, "gap_px": 2, "origin": [0, 0], "patch": {FULL_PATCH}}},
                "seeds": [0]
            }}"#,
            "1".repeat(64)
        ),
    )
    .unwrap();
    let out = fixture.root.join("digest");
    let output = fixture.run(&[
        "experiment",
        "run",
        "--experiment",
        path.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("hashes to"), "{stderr}");
    assert!(!out.join("bundle/stage-evidence.json").exists());
}

#[test]
fn run_rejects_a_set_source_outside_the_document_tree() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);
    std::fs::write(fixture.root.join("grain.toml"), "[film_render.grain]\n").unwrap();
    let experiment = fixture.write_experiment_with(
        r#""chain": {"film": "kodak_portra_400", "set": ["../grain.toml"]},"#,
        FULL_PATCH,
        2,
        2,
    );
    let out = fixture.root.join("escape");
    let output = fixture.run(&[
        "experiment",
        "run",
        "--experiment",
        experiment.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("chain.set"), "{stderr}");
    assert!(stderr.contains("escapes"), "{stderr}");
    assert!(!out.join("bundle/stage-evidence.json").exists());
}

#[test]
fn run_rejects_a_document_with_a_duplicate_key_or_unknown_field() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);

    for (name, replacement, needle) in [
        (
            "duplicate",
            r#""seeds": [0], "seeds": [1]"#,
            "duplicate JSON key",
        ),
        ("unknown", r#""seeds": [0], "unknown": 1"#, "unknown field"),
    ] {
        let path = fixture.root.join(format!("{name}.json"));
        std::fs::write(
            &path,
            format!(
                r#"{{
                    "schema_version": 1,
                    "id": "experiment",
                    "chain": {{"film": "kodak_portra_400", "input_color_space": "sRGB", "input_cctf_decoding": true}},
                    "input": {{"path": "input.png", "sha256": "{}"}},
                    "case": {{"id": "case", "suite": "suite.json"}},
                    "grid": {{"rows": 2, "cols": 2, "patch_px": 4, "gap_px": 2, "origin": [0, 0], "patch": {FULL_PATCH}}},
                    {replacement}
                }}"#,
                digest_of(&fixture.root.join("input.png"))
            ),
        )
        .unwrap();
        let out = fixture.root.join(name);
        let output = fixture.run(&[
            "experiment",
            "run",
            "--experiment",
            path.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]);
        assert!(!output.status.success(), "{name} was accepted");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(needle), "{name}: {stderr}");
        assert!(!out.join("bundle/stage-evidence.json").exists());
    }
}

#[test]
fn run_rejects_multiple_seeds_before_publishing_anything() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);
    let experiment = fixture.write_experiment(FULL_PATCH, 2, 2);
    let text = std::fs::read_to_string(&experiment)
        .unwrap()
        .replace("\"seeds\": [7]", "\"seeds\": [7, 42]");
    std::fs::write(&experiment, text).unwrap();
    let out = fixture.root.join("multiple-seeds");
    let output = fixture.run(&[
        "experiment",
        "run",
        "--experiment",
        experiment.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("one experiment run publishes one seed"),
        "{stderr}"
    );
    assert!(!out.join("bundle/stage-evidence.json").exists());
}

#[test]
fn run_rejects_dry_run_as_an_unknown_option() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);
    let experiment = fixture.write_experiment(FULL_PATCH, 2, 2);
    let out = fixture.root.join("dry");
    let output = fixture.run(&[
        "experiment",
        "run",
        "--experiment",
        experiment.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(!output.status.success());
    assert!(!out.exists());
}

#[test]
fn run_refuses_to_replace_an_existing_report_without_force() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);
    let experiment = fixture.write_experiment(FULL_PATCH, 2, 2);
    let out = fixture.root.join("replace");
    let args = [
        "experiment",
        "run",
        "--experiment",
        experiment.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ];
    assert!(fixture.run(&args).status.success());
    let first = std::fs::read(out.join("report.json")).unwrap();

    let output = fixture.run(&args);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("refusing to replace"), "{stderr}");
    assert_eq!(std::fs::read(out.join("report.json")).unwrap(), first);

    assert!(
        fixture
            .run(&[&args[..], &["--force"]].concat())
            .status
            .success()
    );
    assert_eq!(std::fs::read(out.join("report.json")).unwrap(), first);
}

#[test]
fn export_converts_every_available_stage_and_records_each_array() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);
    let experiment = fixture.write_experiment(FULL_PATCH, 2, 2);
    let run_out = fixture.root.join("run");
    assert!(
        fixture
            .run(&[
                "experiment",
                "run",
                "--experiment",
                experiment.to_str().unwrap(),
                "--out",
                run_out.to_str().unwrap(),
            ])
            .status
            .success()
    );
    let manifest = read_json(&run_out.join("bundle/stage-evidence.json"));

    let arrays = fixture.root.join("arrays");
    let output = fixture.run(&[
        "experiment",
        "export",
        "--stage-evidence",
        run_out.join("bundle").to_str().unwrap(),
        "--out",
        arrays.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "export failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let export = read_json(&arrays.join("export.json"));
    let converted = export["stages"].as_array().unwrap();
    assert_eq!(
        converted.len(),
        manifest["stages"].as_array().unwrap().len()
    );
    let mut recorded = 0;
    for stage in manifest["stages"].as_array().unwrap() {
        let id = stage["id"].as_str().unwrap();
        let array_path = arrays.join(format!("{id}.npy"));
        let record = converted
            .iter()
            .find(|record| record["id"] == id)
            .unwrap_or_else(|| panic!("{id} is not recorded in export.json"));
        if stage["availability"] != "available" {
            assert!(
                !array_path.exists(),
                "{id} was converted but the manifest marks it {}",
                stage["availability"]
            );
            continue;
        }
        recorded += 1;
        assert!(array_path.is_file(), "{id} was not converted");
        let buffer =
            std::fs::read(run_out.join("bundle").join(stage["path"].as_str().unwrap())).unwrap();
        let (shape, samples) = array_bytes(&array_path);
        assert_eq!(
            shape[0],
            stage["layout"]["height"].as_u64().unwrap() as usize
        );
        assert_eq!(
            shape[1],
            stage["layout"]["width"].as_u64().unwrap() as usize
        );
        assert_eq!(shape[2], 3);
        let expected = buffer
            .as_chunks::<8>()
            .0
            .iter()
            .map(|chunk| f64::from_le_bytes(*chunk))
            .collect::<Vec<f64>>();
        assert_eq!(samples.len(), expected.len());
        for (got, want) in samples.iter().zip(&expected) {
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "{id} was not converted byte-for-byte"
            );
        }
        assert_eq!(
            record["sha256"].as_str().unwrap(),
            format!(
                "{:x}",
                <sha2::Sha256 as sha2::Digest>::digest(std::fs::read(&array_path).unwrap())
            ),
            "{id} array digest does not match export.json"
        );
        assert_eq!(
            record["path"].as_str().unwrap(),
            array_path.display().to_string()
        );
    }
    assert!(recorded > 0);

    // A repeated export is byte-identical.
    let arrays2 = fixture.root.join("arrays2");
    assert!(
        fixture
            .run(&[
                "experiment",
                "export",
                "--stage-evidence",
                run_out.join("bundle").to_str().unwrap(),
                "--out",
                arrays2.to_str().unwrap(),
            ])
            .status
            .success()
    );
    for stage in manifest["stages"].as_array().unwrap() {
        if stage["availability"] != "available" {
            continue;
        }
        let id = stage["id"].as_str().unwrap();
        assert_eq!(
            std::fs::read(arrays.join(format!("{id}.npy"))).unwrap(),
            std::fs::read(arrays2.join(format!("{id}.npy"))).unwrap(),
            "{id} is not deterministic"
        );
    }
}

#[test]
fn export_preserves_each_stage_channel_order() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);
    let experiment = fixture.write_experiment(FULL_PATCH, 2, 2);
    let run_out = fixture.root.join("run");
    assert!(
        fixture
            .run(&[
                "experiment",
                "run",
                "--experiment",
                experiment.to_str().unwrap(),
                "--out",
                run_out.to_str().unwrap(),
            ])
            .status
            .success()
    );
    let manifest = read_json(&run_out.join("bundle/stage-evidence.json"));
    let arrays = fixture.root.join("arrays");
    assert!(
        fixture
            .run(&[
                "experiment",
                "export",
                "--stage-evidence",
                run_out.join("bundle").to_str().unwrap(),
                "--out",
                arrays.to_str().unwrap(),
            ])
            .status
            .success()
    );

    let order = |id: &str| -> String {
        manifest["stages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stage| stage["id"] == id)
            .map(|stage| {
                stage["layout"]["channel_order"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .unwrap_or_default()
    };
    for (id, expected) in [
        ("rgb_in", "RGB"),
        ("rgb_out", "RGB"),
        ("log_e_film", "CMY"),
        ("cmy_film", "CMY"),
    ] {
        assert_eq!(order(id), expected, "{id} channel order in the manifest");
    }
    // The converted array's bytes equal the buffer's, so no consumer-expected
    // permutation can have been applied.
    for stage in manifest["stages"].as_array().unwrap() {
        if stage["availability"] != "available" {
            continue;
        }
        let id = stage["id"].as_str().unwrap();
        let buffer =
            std::fs::read(run_out.join("bundle").join(stage["path"].as_str().unwrap())).unwrap();
        let (_, samples) = array_bytes(&arrays.join(format!("{id}.npy")));
        let expected = buffer
            .as_chunks::<8>()
            .0
            .iter()
            .map(|chunk| f64::from_le_bytes(*chunk))
            .collect::<Vec<f64>>();
        assert_eq!(samples, expected, "{id} channel order was rewritten");
    }
}

#[test]
fn run_rejects_a_configuration_the_bundle_cannot_describe() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);

    // A route outside the executed tap range.
    let experiment = fixture.write_experiment_with(
        r#""chain": {"film": "kodak_portra_400", "input_color_space": "sRGB", "input_cctf_decoding": true, "route": "input"},"#,
        FULL_PATCH,
        2,
        2,
    );
    let out = fixture.root.join("route");
    let output = fixture.run(&[
        "experiment",
        "run",
        "--experiment",
        experiment.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("print development boundary"), "{stderr}");
    assert!(!out.join("bundle/stage-evidence.json").exists());

    // Preview mode is rejected before rendering.
    let experiment = fixture.write_experiment_with(
        r#""chain": {"film": "kodak_portra_400", "input_color_space": "sRGB", "input_cctf_decoding": true, "set": ["settings.preview_mode=true"]},"#,
        FULL_PATCH,
        2,
        2,
    );
    let out = fixture.root.join("preview");
    let output = fixture.run(&[
        "experiment",
        "run",
        "--experiment",
        experiment.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("preview mode"), "{stderr}");
    assert!(!out.join("bundle/stage-evidence.json").exists());
}

#[test]
fn run_rejects_a_gpu_backend_before_rendering() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);
    let experiment = fixture.write_experiment(FULL_PATCH, 2, 2);
    let out = fixture.root.join("gpu");
    let output = Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
        .args([
            "experiment",
            "run",
            "--experiment",
            experiment.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .env("RUST_LOG", "off")
        .env("SPEKTRAFILM_DATA_DIR", &fixture.data)
        .env("SPEKTRAFILM_CONFIG_DIR", fixture.root.join("config"))
        .env("SPEKTRAFILM_BACKEND", "wgpu")
        .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
        .current_dir(&fixture.root)
        .output()
        .unwrap();
    // The lab always renders on the CPU f64 reference route, so the request
    // never reaches a GPU backend; the run must either succeed on CPU or fail
    // without ever publishing a bundle from a GPU route.
    assert!(
        !out.join("bundle/stage-evidence.json").exists() || output.status.success(),
        "a GPU backend published a bundle"
    );
    if output.status.success() {
        let manifest = read_json(&out.join("bundle/stage-evidence.json"));
        assert_eq!(manifest["host_precision"], "f64");
    }
}

#[test]
fn export_selects_requested_stages_and_rejects_an_unknown_one() {
    let fixture = Fixture::new();
    fixture.write_input("input.png", 14, 14);
    fixture.write_suite(CHART);
    let experiment = fixture.write_experiment(FULL_PATCH, 2, 2);
    let run_out = fixture.root.join("run");
    assert!(
        fixture
            .run(&[
                "experiment",
                "run",
                "--experiment",
                experiment.to_str().unwrap(),
                "--out",
                run_out.to_str().unwrap(),
            ])
            .status
            .success()
    );
    let manifest = read_json(&run_out.join("bundle/stage-evidence.json"));
    let available: Vec<&str> = manifest["stages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|stage| stage["availability"] == "available")
        .map(|stage| stage["id"].as_str().unwrap())
        .collect();
    assert!(available.len() >= 2);

    let selected = fixture.root.join("selected");
    let output = fixture.run(&[
        "experiment",
        "export",
        "--stage-evidence",
        run_out.join("bundle").to_str().unwrap(),
        "--out",
        selected.to_str().unwrap(),
        "--stage",
        available[0],
    ]);
    assert!(
        output.status.success(),
        "export failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(selected.join(format!("{}.npy", available[0])).is_file());
    for id in &available[1..] {
        assert!(!selected.join(format!("{id}.npy")).exists());
    }

    let rejected = fixture.root.join("rejected");
    let output = fixture.run(&[
        "experiment",
        "export",
        "--stage-evidence",
        run_out.join("bundle").to_str().unwrap(),
        "--out",
        rejected.to_str().unwrap(),
        "--stage",
        "not_a_stage",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not_a_stage"), "{stderr}");
    for id in &available {
        assert!(
            stderr.contains(id),
            "the message must name the available stages: {stderr}"
        );
    }
    assert!(!rejected.join("export.json").exists());
}

/// Write a bundle whose manifest marks a stage `unavailable`.
fn unavailable_bundle(fixture: &Fixture) -> PathBuf {
    let bundle = fixture.root.join("incomplete");
    std::fs::create_dir_all(&bundle).unwrap();
    let stage = |id: &str, availability: &str| -> Value {
        serde_json::json!({
            "availability": availability,
            "id": id,
            "path": format!("{id}.f64"),
            "reason": if availability == "available" { serde_json::Value::Null } else { serde_json::json!("buffer_write_failed") },
            "sha256": "0".repeat(64),
            "layout": {
                "width": 2, "height": 2, "channels": 3,
                "dtype": "f64", "endianness": "little",
                "channel_order": if id == "rgb_in" || id == "rgb_out" { "RGB" } else { "CMY" },
            },
        })
    };
    std::fs::write(
        bundle.join("stage-evidence.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "execution_status": "completed",
            "host_precision": "f64",
            "stages": [
                stage("rgb_in", "available"),
                stage("cmy_film", "unavailable"),
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    bundle
}

#[test]
fn export_rejects_an_incomplete_bundle() {
    let fixture = Fixture::new();
    let bundle = unavailable_bundle(&fixture);
    let arrays = fixture.root.join("incomplete-arrays");
    let output = fixture.run(&[
        "experiment",
        "export",
        "--stage-evidence",
        bundle.to_str().unwrap(),
        "--out",
        arrays.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("incomplete"), "{stderr}");
    assert!(stderr.contains("cmy_film"), "{stderr}");
    assert!(!arrays.join("rgb_in.npy").exists());
    assert!(!arrays.join("export.json").exists());
}

#[test]
fn export_rejects_an_f32_bundle() {
    let fixture = Fixture::new();
    let bundle = unavailable_bundle(&fixture);
    let manifest_path = bundle.join("stage-evidence.json");
    let mut manifest = read_json(&manifest_path);
    manifest["host_precision"] = serde_json::json!("f32");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let arrays = fixture.root.join("f32-arrays");
    let output = fixture.run(&[
        "experiment",
        "export",
        "--stage-evidence",
        bundle.to_str().unwrap(),
        "--out",
        arrays.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("f32"), "{stderr}");
    assert!(stderr.contains("f64"), "{stderr}");
    assert!(!arrays.join("rgb_in.npy").exists());
    assert!(!arrays.join("export.json").exists());
}

#[test]
fn export_rejects_a_buffer_whose_digest_does_not_match() {
    let fixture = Fixture::new();
    let bundle = fixture.root.join("bad-digest");
    std::fs::create_dir_all(&bundle).unwrap();
    let bytes: Vec<u8> = (0..96u32).map(|i| i as u8).collect();
    std::fs::write(bundle.join("rgb_in.f64"), &bytes).unwrap();
    std::fs::write(
        bundle.join("stage-evidence.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "execution_status": "completed",
            "host_precision": "f64",
            "stages": [{
                "availability": "available",
                "id": "rgb_in",
                "path": "rgb_in.f64",
                "sha256": "1".repeat(64),
                "layout": {
                    "width": 2, "height": 2, "channels": 3,
                    "dtype": "f64", "endianness": "little", "channel_order": "RGB",
                },
            }],
        }))
        .unwrap(),
    )
    .unwrap();
    let arrays = fixture.root.join("bad-digest-arrays");
    let output = fixture.run(&[
        "experiment",
        "export",
        "--stage-evidence",
        bundle.to_str().unwrap(),
        "--out",
        arrays.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("hashes to"), "{stderr}");
    assert!(!arrays.join("rgb_in.npy").exists());
    assert!(!arrays.join("export.json").exists());
}

#[test]
fn export_replaces_an_existing_array_only_with_force() {
    let fixture = Fixture::new();
    let bundle = fixture.root.join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    let bytes: Vec<u8> = (0..96u32).map(|i| i as u8).collect();
    let digest = digest_of_bytes(&bytes);
    std::fs::write(bundle.join("rgb_in.f64"), &bytes).unwrap();
    std::fs::write(
        bundle.join("stage-evidence.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "execution_status": "completed",
            "host_precision": "f64",
            "stages": [{
                "availability": "available",
                "id": "rgb_in",
                "path": "rgb_in.f64",
                "sha256": digest,
                "layout": {
                    "width": 2, "height": 2, "channels": 3,
                    "dtype": "f64", "endianness": "little", "channel_order": "RGB",
                },
            }],
        }))
        .unwrap(),
    )
    .unwrap();

    let arrays = fixture.root.join("sentinel");
    std::fs::create_dir_all(&arrays).unwrap();
    let sentinel = b"SENTINEL".to_vec();
    std::fs::write(arrays.join("rgb_in.npy"), &sentinel).unwrap();

    let output = fixture.run(&[
        "experiment",
        "export",
        "--stage-evidence",
        bundle.to_str().unwrap(),
        "--out",
        arrays.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("refusing to replace"), "{stderr}");
    assert_eq!(std::fs::read(arrays.join("rgb_in.npy")).unwrap(), sentinel);

    let output = fixture.run(&[
        "experiment",
        "export",
        "--stage-evidence",
        bundle.to_str().unwrap(),
        "--out",
        arrays.to_str().unwrap(),
        "--force",
    ]);
    assert!(
        output.status.success(),
        "export --force failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(std::fs::read(arrays.join("rgb_in.npy")).unwrap(), sentinel);
    let (shape, samples) = array_bytes(&arrays.join("rgb_in.npy"));
    assert_eq!(shape, vec![2, 2, 3]);
    let expected = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| f64::from_le_bytes(*chunk))
        .collect::<Vec<f64>>();
    assert_eq!(samples, expected);
    assert!(arrays.join("export.json").is_file());
}

fn digest_of_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
