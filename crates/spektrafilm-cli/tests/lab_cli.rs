//! `spektrafilm-lab spectral evaluate` and `spektrafilm-lab run` end-to-end.

#![cfg(feature = "precision-f64")]

use serde_json::{Value, json};
use std::collections::BTreeMap;
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
                "color_space": "ACES2065-1",
                "cctf_decoding": false,
                "condition": {"reference": "D55", "target": "D65"},
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
    let spectra = f.root.join("manifest-mismatch.json");
    std::fs::write(
        &spectra,
        r#"{
            "schema_version": 1,
            "name": "synthetic roundtrip",
            "color_space": "ACES2065-1",
            "cctf_decoding": false,
            "condition": {"reference": "T", "target": "D65"},
            "grid_nm": {"start": 380.0, "end": 780.0, "step": 5.0},
            "valid_band": [380.0, 780.0],
            "reflectances": {"path": "reflectances.npy"},
            "samples": [
                {"id": "midgray", "source": "synthetic", "partition": "neutral", "row": 0}
            ]
        }"#,
    )
    .unwrap();
    let profile = f.data.join("profiles/kodak_portra_400.json");
    let result = f.run(&[
        "spectral",
        "evaluate",
        "--spectra",
        spectra.to_str().unwrap(),
        "--profile",
        profile.to_str().unwrap(),
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

// ---------------------------------------------------------------------------
// measurement: `spektrafilm-lab run`, `sample`, `compare`, `verify`
// ---------------------------------------------------------------------------

/// The chart is one row of twelve samples: a measurement executes its
/// stimulus as a `1 x N` image, so a `12 x 1` raster is the same shape and the
/// two paths must agree stage for stage.
const CHART_WIDTH: u32 = 12;
const CHART_HEIGHT: u32 = 1;
const CHART: [[u8; 3]; 12] = [
    [0, 0, 0],
    [255, 255, 255],
    [32, 64, 128],
    [200, 150, 100],
    [18, 18, 18],
    [64, 64, 64],
    [128, 128, 128],
    [192, 192, 192],
    [230, 120, 60],
    [60, 230, 120],
    [120, 60, 230],
    [140, 140, 10],
];

/// Disables every stage that mixes samples, meters the whole frame, or draws
/// stochastic values, so the executed chain is point-wise.
const POINT_WISE: [&str; 18] = [
    "--set",
    "film_render.grain.active=false",
    "--set",
    "film_render.dir_couplers.active=false",
    "--set",
    "film_render.halation.active=false",
    "--set",
    "camera.auto_exposure=false",
    "--set",
    "print_render.glare.active=false",
    "--set",
    "enlarger.lens_blur=0",
    "--set",
    "enlarger.diffusion_filter.active=false",
    "--set",
    "scanner.lens_blur=0",
    "--set",
    "scanner.unsharp_mask=[0,0]",
];

/// The same chart in two forms: the sRGB raster the user binary reads, and the
/// declared linear scene RGB the measurement declares.
struct MeasureFixture {
    root: PathBuf,
    data: PathBuf,
    stimuli: PathBuf,
    plan: PathBuf,
    working_plan: PathBuf,
    /// The same values declared in the film log-exposure domain: no color
    /// space and no transfer state, as the film domains require.
    log_e_manifest: PathBuf,
    /// The same values declared as film densities.
    density_manifest: PathBuf,
}

impl MeasureFixture {
    fn new() -> Self {
        static NEXT_MEASURE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "spektrafilm-lab-measure-{}-{}",
            std::process::id(),
            NEXT_MEASURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");

        let pixels: Vec<u8> = CHART.iter().flatten().copied().collect();
        image::save_buffer(
            root.join("input.png"),
            &pixels,
            CHART_WIDTH,
            CHART_HEIGHT,
            image::ColorType::Rgb8,
        )
        .unwrap();
        let linear: Vec<Vec<f64>> = CHART
            .iter()
            .map(|pixel| {
                pixel
                    .iter()
                    .map(|value| {
                        spektrafilm_math::colorspace::srgb_decode_f64(*value as f64 / 255.0)
                    })
                    .collect()
            })
            .collect();
        write_npy_matrix(&root.join("points.npy"), &linear);

        let labels: Vec<Value> = (0..linear.len())
            .map(|index| {
                serde_json::json!({
                    "id": format!("p{index:02}"),
                    "source": "chart",
                    "partition": "all",
                    "row": index
                })
            })
            .collect();
        std::fs::write(
            root.join("points.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1,
                "name": "dyadic chart",
                "domain": "scene_rgb",
                "color_space": "sRGB",
                "cctf_decoding": false,
                "points": {"path": "points.npy"},
                "samples": labels,
            }))
            .unwrap(),
        )
        .unwrap();
        let windows: Vec<Value> = (0..linear.len())
            .map(|index| {
                serde_json::json!({
                    "id": format!("p{index:02}"),
                    "label": format!("p{index:02}"),
                    "source": "chart",
                    "partition": "all",
                    "role": "other",
                    "measurements": ["numeric", "color"],
                    "geometry": {"x": index as u32, "y": 0u32, "width": 1, "height": 1, "space": "source"},
                })
            })
            .collect();
        let plan = root.join("plan.json");
        std::fs::write(
            &plan,
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1,
                "id": "chart-pixels",
                "name": "chart pixels",
                "regions": windows,
            }))
            .unwrap(),
        )
        .unwrap();
        let working_plan = root.join("working-plan.json");
        let mut working_regions = serde_json::from_value::<Value>(
            serde_json::json!({"schema_version": 1, "id": "working", "regions": []}),
        )
        .unwrap();
        working_regions["regions"] = windows
            .clone()
            .into_iter()
            .map(|mut region| {
                region["geometry"]["space"] = json!("working_pixels");
                region
            })
            .collect();
        std::fs::write(
            &working_plan,
            serde_json::to_vec_pretty(&working_regions).unwrap(),
        )
        .unwrap();
        let stimuli = root.join("points.json");
        let log_e_manifest = root.join("log-e.json");
        let density_manifest = root.join("density.json");
        std::fs::write(
            &density_manifest,
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1,
                "name": "film density",
                "domain": "film_density",
                "points": {"path": "points.npy"},
                "samples": labels,
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            &log_e_manifest,
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1,
                "name": "film log exposure",
                "domain": "film_log_e",
                "points": {"path": "points.npy"},
                "samples": labels,
            }))
            .unwrap(),
        )
        .unwrap();
        Self {
            root,
            data,
            stimuli,
            plan,
            working_plan,
            log_e_manifest,
            density_manifest,
        }
    }

    fn lab(&self, args: &[&str]) -> Output {
        self.command(env!("CARGO_BIN_EXE_spektrafilm-lab"), args)
    }

    fn panel(&self, args: &[&str]) -> Output {
        self.command(env!("CARGO_BIN_EXE_spektrafilm-f64"), args)
    }

    fn command(&self, binary: &str, args: &[&str]) -> Output {
        self.command_env(binary, args, &[])
    }

    /// The same command with extra environment entries. They are set after the
    /// removals above, so a test can declare the data directory by environment.
    fn command_env(&self, binary: &str, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut command = Command::new(binary);
        command
            .args(args)
            .env("RUST_LOG", "off")
            .env("SPEKTRAFILM_CONFIG_DIR", self.root.join("config"))
            .env_remove("SPEKTRAFILM_DATA_DIR")
            .env_remove("SPEKTRAFILM_INTERNAL_PRESERVE_USER_EDITS")
            .env_remove("SPEKTRAFILM_BACKEND");
        for (key, value) in env {
            command.env(key, value);
        }
        command.output().unwrap()
    }

    /// Run the lab binary with one environment entry declared.
    fn lab_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        self.command_env(env!("CARGO_BIN_EXE_spektrafilm-lab"), args, env)
    }

    /// Measure the declared scene RGB through the real chain.
    fn measure(&self, out: &Path) -> Output {
        self.measure_with(&self.stimuli.clone(), out)
    }

    /// Measure an otherwise identical declaration with another stimulus
    /// document.
    fn measure_with(&self, stimuli: &Path, out: &Path) -> Output {
        let mut args = vec![
            "run",
            "--stimuli",
            stimuli.to_str().unwrap(),
            "--enter",
            "rgb_in",
            "--read",
            "log_e_film,cmy_film,rgb_out",
            "--film",
            "kodak_portra_400",
            "--data-dir",
            self.data.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ];
        args.extend_from_slice(&POINT_WISE);
        self.lab(&args)
    }

    /// Execute the same values as a raster through the user binary.
    fn process(&self, bundle: &Path) -> Output {
        let mut args = vec![
            "process".to_owned(),
            self.root.join("input.png").display().to_string(),
            "--output".to_owned(),
            self.root.join("out.png").display().to_string(),
            "--stage-evidence".to_owned(),
            bundle.display().to_string(),
            "--film".to_owned(),
            "kodak_portra_400".to_owned(),
            "--backend".to_owned(),
            "cpu".to_owned(),
            "--input-color-space".to_owned(),
            "sRGB".to_owned(),
            "--input-cctf-decoding".to_owned(),
            "true".to_owned(),
            "--data-dir".to_owned(),
            self.data.display().to_string(),
        ];
        args.extend(POINT_WISE.iter().map(|value| (*value).to_owned()));
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        self.panel(&borrowed)
    }
}

/// A points manifest that declares the retained `rgb_in` boundary of a stage
/// capture verbatim: the same numbers, in the state that bundle declares.
fn rgb_in_manifest(fixture: &MeasureFixture, bundle: &Path, name: &str) -> PathBuf {
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(bundle.join("stage-evidence.json")).unwrap())
            .unwrap();
    let record = manifest["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == "rgb_in")
        .unwrap();
    assert_eq!(record["availability"], "available", "{record}");
    let decoding = record["color"]["cctf_decoding"].as_bool().unwrap();
    let color_space = record["color"]["color_space"].as_str().unwrap();
    let width = record["layout"]["width"].as_u64().unwrap() as usize;
    let height = record["layout"]["height"].as_u64().unwrap() as usize;
    let bytes = std::fs::read(bundle.join(record["path"].as_str().unwrap())).unwrap();
    let values: Vec<f64> = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| f64::from_le_bytes(*chunk))
        .collect();
    let rows: Vec<Vec<f64>> = values.chunks(3).map(<[f64]>::to_vec).collect();
    assert_eq!(rows.len(), width * height);
    let array = fixture.root.join(format!("{name}.npy"));
    write_npy_matrix(&array, &rows);
    let samples: Vec<Value> = (0..rows.len())
        .map(|index| {
            serde_json::json!({
                "id": format!("p{index:02}"),
                "source": "chart",
                "partition": "all",
                "row": index
            })
        })
        .collect();
    let path = fixture.root.join(format!("{name}.json"));
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "name": name,
            "domain": "scene_rgb",
            "color_space": color_space,
            "cctf_decoding": decoding,
            "points": {"path": array.file_name().unwrap().to_str().unwrap()},
            "samples": samples,
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

/// One reading row per sample of a stage, from a `readings.csv` artifact.
fn read_points(path: &Path, stage: &str) -> BTreeMap<String, [f64; 3]> {
    let text = std::fs::read_to_string(path).unwrap();
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap().split(',').collect();
    let field = |line: &str, name: &str| -> String {
        let index = header.iter().position(|column| *column == name).unwrap();
        line.split(',').nth(index).unwrap().to_owned()
    };
    let mut rows = BTreeMap::new();
    for line in lines {
        if field(line, "stage") != stage {
            continue;
        }
        let number = |name: &str| {
            let text = field(line, name);
            if text.is_empty() {
                f64::NAN
            } else {
                text.parse().unwrap()
            }
        };
        rows.insert(
            field(line, "sample_id"),
            [number("c0"), number("c1"), number("c2")],
        );
    }
    rows
}

/// The samples of one available stage of a stage-evidence bundle, in row-major
/// order.
/// One stage record of a retained bundle manifest.
fn bundle_stage_record(directory: &Path, id: &str) -> Value {
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(directory.join("stage-evidence.json")).unwrap())
            .unwrap();
    manifest["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == id)
        .unwrap_or_else(|| panic!("no {id} stage"))
        .clone()
}

/// Copy one retained directory, so a test can damage the copy.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        match entry.file_type().unwrap().is_dir() {
            true => copy_tree(&entry.path(), &target),
            false => {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
}

fn bundle_stage(directory: &Path, id: &str) -> Vec<[f64; 3]> {
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(directory.join("stage-evidence.json")).unwrap())
            .unwrap();
    let record = manifest["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == id)
        .unwrap_or_else(|| panic!("no {id} stage"));
    assert_eq!(record["availability"], "available", "{record}");
    let width = record["layout"]["width"].as_u64().unwrap();
    let height = record["layout"]["height"].as_u64().unwrap();
    let bytes = std::fs::read(directory.join(record["path"].as_str().unwrap())).unwrap();
    assert_eq!(bytes.len(), (width * height * 3) as usize * 8);
    let values: Vec<f64> = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| f64::from_le_bytes(*chunk))
        .collect();
    values.chunks(3).map(|c| [c[0], c[1], c[2]]).collect()
}

fn assert_close(label: &str, got: [f64; 3], want: [f64; 3]) {
    for channel in 0..3 {
        let deviation = (got[channel] - want[channel]).abs() / want[channel].abs().max(1e-12);
        assert!(
            deviation < 1e-9,
            "{label} channel {channel}: {got:?} vs {want:?} (relative {deviation:e})"
        );
    }
}

#[test]
fn measure_run_matches_process_stage_evidence_on_log_e_film() {
    let f = MeasureFixture::new();
    let bundle = f.root.join("bundle");
    let result = f.process(&bundle);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    // The reading must reproduce the executed chain's own signal: the
    // stimulus is the retained `rgb_in` boundary of that run, declared in the
    // state the bundle declares, so both sides consume the same numbers.
    let stimuli = rgb_in_manifest(&f, &bundle, "boundary");
    let lab = f.root.join("lab");
    let result = f.measure_with(&stimuli, &lab);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    assert_eq!(
        read_points(&lab.join("readings.csv"), "log_e_film").len(),
        CHART.len()
    );
    for stage in ["log_e_film", "cmy_film", "rgb_out"] {
        let evidence = bundle_stage(&bundle, stage);
        let read = read_points(&lab.join("readings.csv"), stage);
        for (index, expected) in evidence.iter().enumerate() {
            let id = format!("p{index:02}");
            assert_close(&format!("{stage} {id}"), read[&id], *expected);
        }
    }

    // Repeated runs on identical inputs produce identical artifacts.
    let repeat = f.root.join("lab-repeat");
    assert!(f.measure_with(&stimuli, &repeat).status.success());
    for name in ["run.json", "readings.csv"] {
        assert_eq!(
            std::fs::read(lab.join(name)).unwrap(),
            std::fs::read(repeat.join(name)).unwrap(),
            "{name} differs between identical runs"
        );
    }
    assert!(
        !f.measure_with(&stimuli, &lab).status.success(),
        "an existing artifact must not be replaced"
    );

    let record: Value =
        serde_json::from_slice(&std::fs::read(lab.join("run.json")).unwrap()).unwrap();
    assert_eq!(record["kind"], "run");
    assert_eq!(record["executed"], true);
    assert_eq!(record["declaration"]["enter"], "rgb_in");
    assert_eq!(
        record["identity"]["input_declaration"]["cctf_decoding"],
        true
    );
    assert_eq!(
        record["identity"]["input_declaration"]["color_space"],
        "sRGB"
    );
    assert!(
        record["executor"]["backend"]
            .as_str()
            .unwrap()
            .contains("CPU"),
        "{}",
        record["executor"]
    );
    assert_eq!(record["executor"]["precision"], "f64");
    assert_eq!(
        record["identity"]["parameters"]["digest_mode"],
        "apply_stock_specifics"
    );
    assert_eq!(
        record["identity"]["film_profile"]["name"],
        "kodak_portra_400"
    );
    let stages = record["stages"].as_array().unwrap();
    assert!(stages.iter().any(|stage| {
        stage["id"] == "log_e_film"
            && stage["availability"] == "available"
            && stage["units"] == "log10"
    }));
    assert!(stages.iter().any(|stage| {
        stage["id"] == "rgb_out"
            && stage["color_space"] == "sRGB"
            && stage["cctf_decoding"] == false
    }));
}

#[test]
fn measure_run_fails_on_a_point_wise_violation_and_writes_nothing() {
    let f = MeasureFixture::new();
    let out = f.root.join("violating");
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.stimuli.to_str().unwrap(),
        "--enter",
        "rgb_in",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    for path in [
        "film_render.grain.active",
        "film_render.dir_couplers.active",
        "film_render.halation.active",
        "camera.auto_exposure",
        "scanner.unsharp_mask",
    ] {
        assert!(error.contains(path), "no mention of {path}: {error}");
    }
    assert!(
        error.contains("--set film_render.grain.active=false"),
        "no remediation: {error}"
    );
    assert!(!out.join("run.json").exists());
    assert!(!out.join("evidence").exists());

    // Reading the injected boundary itself needs no disabled effect at all:
    // every operation that would mix samples changes a later stage.
    let boundary = f.root.join("rgb-in-only");
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.stimuli.to_str().unwrap(),
        "--enter",
        "rgb_in",
        "--read",
        "rgb_in",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        boundary.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    // The couplers, grain, glare, and the scanner optics change nothing at or
    // before `log_e_film`, so they need not be disabled to read it: measured
    // positions put halation and the camera controls at `log_e_film`, the
    // couplers and grain at `cmy_film` (8.5e-01 and 2.9e+00 relative on a 12x4
    // chart), and glare, `scanner.lens_blur`, and `scanner.unsharp_mask` at
    // `rgb_out` only.
    let limited = f.root.join("log-e-only");
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.stimuli.to_str().unwrap(),
        "--enter",
        "rgb_in",
        "--read",
        "log_e_film",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        limited.to_str().unwrap(),
        "--set",
        "camera.auto_exposure=false",
        "--set",
        "film_render.halation.active=false",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let record: Value =
        serde_json::from_slice(&std::fs::read(limited.join("run.json")).unwrap()).unwrap();
    // The range is recorded through the shared tap controls, so the retained
    // effective document describes what executed.
    assert_eq!(
        record["identity"]["parameters"]["effective"]["taps"]["inject"],
        "rgb_in"
    );
    assert_eq!(
        record["identity"]["parameters"]["effective"]["taps"]["collect"],
        "log_e_film"
    );

    // Entering at the film density bypasses the development stage, so only the
    // print and scan controls reach the reading; the camera and the film
    // operations the default chain leaves active are not named.
    let density = f.root.join("density-entry");
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.density_manifest.to_str().unwrap(),
        "--enter",
        "cmy_film",
        "--read",
        "cmy_film,rgb_out",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        density.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    for path in ["print_render.glare.active", "scanner.unsharp_mask"] {
        assert!(error.contains(path), "no mention of {path}: {error}");
    }
    for path in [
        "film_render.grain.active",
        "film_render.dir_couplers.active",
        "film_render.halation.active",
        "camera.auto_exposure",
    ] {
        assert!(
            !error.contains(path),
            "{path} cannot reach a cmy_film entry: {error}"
        );
    }
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.density_manifest.to_str().unwrap(),
        "--enter",
        "cmy_film",
        "--read",
        "cmy_film,rgb_out",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        density.to_str().unwrap(),
        "--set",
        "print_render.glare.active=false",
        "--set",
        "scanner.unsharp_mask=[0,0]",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn measure_refuses_a_declared_entry_or_collection_it_cannot_honour() {
    let f = MeasureFixture::new();
    let declared = f.root.join("declared");
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.stimuli.to_str().unwrap(),
        "--enter",
        "rgb_in",
        "--read",
        "log_e_film",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        declared.to_str().unwrap(),
        "--set",
        "taps.inject=log_e_film",
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(
        error.contains("taps.inject=log_e_film") && error.contains("--enter rgb_in"),
        "{error}"
    );
    assert!(!declared.exists());

    let truncated = f.root.join("truncated");
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.stimuli.to_str().unwrap(),
        "--enter",
        "rgb_in",
        "--read",
        "rgb_out",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        truncated.to_str().unwrap(),
        "--set",
        "taps.collect=cmy_film",
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(
        error.contains("taps.collect") && error.contains("rgb_out"),
        "{error}"
    );
    assert!(!truncated.exists());

    // A reading the entry cannot reach is refused before executing.
    let upstream = f.root.join("upstream");
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.log_e_manifest.to_str().unwrap(),
        "--enter",
        "log_e_film",
        "--read",
        "rgb_in",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        upstream.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("cannot be measured"), "{error}");
}

#[test]
fn measure_sample_reproduces_a_retained_stage_and_reports_a_plan_violation() {
    let f = MeasureFixture::new();
    let bundle = f.root.join("bundle");
    assert!(f.process(&bundle).status.success());

    let sample = f.root.join("sample");
    let result = f.lab(&[
        "sample",
        "--from-evidence",
        bundle.to_str().unwrap(),
        "--stage",
        "log_e_film",
        "--regions",
        f.working_plan.to_str().unwrap(),
        "--out",
        sample.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let evidence = bundle_stage(&bundle, "log_e_film");
    let read = read_points(&sample.join("readings.csv"), "log_e_film");
    assert_eq!(read.len(), CHART.len());
    for (index, expected) in evidence.iter().enumerate() {
        let id = format!("p{index:02}");
        assert_close(&format!("sampled {id}"), read[&id], *expected);
    }

    // A reading states the state its own values are in. A film-domain stage
    // declares no transfer state and the reading does not invent one.
    let record: Value =
        serde_json::from_slice(&std::fs::read(sample.join("run.json")).unwrap()).unwrap();
    assert_eq!(record["executed"], false);
    assert_eq!(record["stages"][0]["cctf_decoding"], Value::Null);
    assert_eq!(record["source"]["cctf_decoding"], Value::Null);

    // The retained `rgb_in` boundary already stores decoded values, so
    // sampling passes them through: the reading keeps the decoded state and
    // the source block reports the bundle's own declaration verbatim.
    let encoded = f.root.join("sample-rgb-in");
    let result = f.lab(&[
        "sample",
        "--from-evidence",
        bundle.to_str().unwrap(),
        "--stage",
        "rgb_in",
        "--regions",
        f.plan.to_str().unwrap(),
        "--out",
        encoded.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let record: Value =
        serde_json::from_slice(&std::fs::read(encoded.join("run.json")).unwrap()).unwrap();
    let bundle_record = bundle_stage_record(&bundle, "rgb_in");
    assert_eq!(bundle_record["color"]["cctf_decoding"], true);
    assert_eq!(record["source"]["cctf_decoding"], true);
    assert_eq!(record["stages"][0]["cctf_decoding"], true);
    assert_eq!(record["stages"][0]["color_space"], "sRGB");
    let read = read_points(&encoded.join("readings.csv"), "rgb_in");
    for (index, stored) in bundle_stage(&bundle, "rgb_in").iter().enumerate() {
        assert_close(
            &format!("decoded p{index:02}"),
            read[&format!("p{index:02}")],
            *stored,
        );
    }

    // A buffer that no longer matches the digest its manifest declares cannot
    // be attributed to the run, and sampling refuses it.
    let tampered = f.root.join("tampered");
    copy_tree(&bundle, &tampered);
    let mut bytes = std::fs::read(tampered.join("log_e_film.f64")).unwrap();
    bytes[0] ^= 1;
    std::fs::write(tampered.join("log_e_film.f64"), &bytes).unwrap();
    let result = f.lab(&[
        "sample",
        "--from-evidence",
        tampered.to_str().unwrap(),
        "--stage",
        "log_e_film",
        "--regions",
        f.plan.to_str().unwrap(),
        "--out",
        f.root.join("tampered-out").to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("hashes"), "{error}");
    assert!(!f.root.join("tampered-out").exists());

    let outside = f.root.join("plan-outside.json");
    std::fs::write(
        &outside,
        r#"{"schema_version": 1, "id": "outside", "regions": [
            {"id": "edge", "label": "edge", "source": "chart", "partition": "all", "role": "other", "measurements": ["numeric"], "geometry": {"x": 11, "y": 0, "width": 2, "height": 1, "space": "source"}}
        ]}"#,
    )
    .unwrap();
    let rejected = f.root.join("rejected");
    let result = f.lab(&[
        "sample",
        "--from-evidence",
        bundle.to_str().unwrap(),
        "--stage",
        "log_e_film",
        "--regions",
        outside.to_str().unwrap(),
        "--out",
        rejected.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("edge"), "{error}");
    assert!(!rejected.join("readings.csv").exists());
}

#[test]
fn measure_compare_reports_deltas_and_identity_drift_beside_them() {
    let f = MeasureFixture::new();
    let lab = f.root.join("lab");
    let repeat = f.root.join("lab-repeat");
    assert!(f.measure(&lab).status.success());
    assert!(f.measure(&repeat).status.success());

    let compare = f.root.join("compare");
    let result = f.lab(&[
        "compare",
        "readings",
        "--baseline",
        lab.to_str().unwrap(),
        "--candidate",
        repeat.to_str().unwrap(),
        "--out",
        compare.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(compare.join("comparison.json")).unwrap()).unwrap();
    assert!(
        report["identity_differences"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let csv = std::fs::read_to_string(compare.join("deltas.csv")).unwrap();
    let header: Vec<&str> = csv.lines().next().unwrap().split(',').collect();
    let column = |name: &str| header.iter().position(|field| *field == name).unwrap();
    for line in csv.lines().skip(1) {
        let fields: Vec<&str> = line.split(',').collect();
        for name in ["d0", "d1", "d2"] {
            assert_eq!(fields[column(name)], "0", "{line}");
        }
        match fields[column("stage")] {
            // A stage that declares no color space reports no distance at all.
            "log_e_film" | "cmy_film" => {
                assert!(fields[column("d_e00")].is_empty(), "{line}");
            }
            // A colorimetric stage reports a real zero for identical values.
            "rgb_out" => assert_eq!(fields[column("d_e00")], "0", "{line}"),
            other => panic!("unexpected stage {other}"),
        }
    }
    // Availability is decided per pair and per metric, and the reason is part
    // of the record: a film-domain stage has no color space to compare in.
    let unavailable = report["unavailable"].as_array().unwrap();
    assert!(
        unavailable.iter().any(|entry| {
            entry["pair"] == "candidate_minus_baseline"
                && entry["stage"] == "log_e_film"
                && entry["metric"] == "delta_e_2000"
                && entry["reason"] == "no declared color space"
        }),
        "no per-pair unavailable reason: {report}"
    );
    // The summaries name the metric and the domain it was computed in.
    let summaries = report["delta_e00_summaries"].as_array().unwrap();
    assert!(
        summaries.iter().any(|summary| {
            summary["stage"] == "rgb_out"
                && summary["metric"] == "delta_e_2000"
                && summary["metric_domain"] == "sRGB-encoded D65 (bounded display)"
        }),
        "{report}"
    );

    // A differing record is evidence, not an error: the numbers still publish.
    // The two sides here consume the same retained boundary, so the values
    // must agree exactly while the records differ.
    let sample = f.root.join("sample");
    let bundle = f.root.join("bundle");
    assert!(f.process(&bundle).status.success());
    let stimuli = rgb_in_manifest(&f, &bundle, "boundary");
    let measured = f.root.join("measured");
    assert!(f.measure_with(&stimuli, &measured).status.success());
    assert!(
        f.lab(&[
            "sample",
            "--from-evidence",
            bundle.to_str().unwrap(),
            "--stage",
            "log_e_film",
            "--regions",
            f.working_plan.to_str().unwrap(),
            "--out",
            sample.to_str().unwrap(),
        ])
        .status
        .success()
    );
    let mixed = f.root.join("mixed");
    let result = f.lab(&[
        "compare",
        "readings",
        "--baseline",
        measured.to_str().unwrap(),
        "--candidate",
        sample.to_str().unwrap(),
        "--out",
        mixed.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(mixed.join("comparison.json")).unwrap()).unwrap();
    assert!(
        !report["identity_differences"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let csv = std::fs::read_to_string(mixed.join("deltas.csv")).unwrap();
    for line in csv.lines().skip(1) {
        let fields: Vec<&str> = line.split(',').collect();
        let value = |name: &str| -> f64 {
            let column = header.iter().position(|field| *field == name).unwrap();
            fields[column].parse().unwrap()
        };
        assert_eq!(fields[column("valid")], "true", "{line}");
        for name in ["d0", "d1", "d2"] {
            let deviation = value(name).abs() / value(&name.replace('d', "b")).abs().max(1e-12);
            assert!(deviation < 1e-9, "{line} ({name} {deviation:e})");
        }
    }
}

#[test]
fn measure_compare_pairs_a_reference_under_its_own_role() {
    let f = MeasureFixture::new();
    let baseline = f.root.join("reference-baseline");
    let candidate = f.root.join("reference-candidate");
    let reference = f.root.join("reference-third");
    assert!(f.measure(&baseline).status.success());
    // The candidate sits 0.5 EV above the baseline and the reference 1.0 EV
    // above it, so each comparison's direction is observable on the raw
    // exposure: `log_e_film` is linear in EV, and one stop is 0.30103 log10.
    let shifted = |compensation: &str, out: &Path| -> Vec<String> {
        let mut args: Vec<String> = vec![
            "run".to_owned(),
            "--stimuli".to_owned(),
            f.stimuli.display().to_string(),
            "--enter".to_owned(),
            "rgb_in".to_owned(),
            "--read".to_owned(),
            "log_e_film,cmy_film,rgb_out".to_owned(),
            "--film".to_owned(),
            "kodak_portra_400".to_owned(),
            "--data-dir".to_owned(),
            f.data.display().to_string(),
            "--out".to_owned(),
            out.display().to_string(),
        ];
        args.extend(POINT_WISE.iter().map(|value| (*value).to_owned()));
        args.push("--set".to_owned());
        args.push(format!("camera.exposure_compensation_ev={compensation}"));
        args
    };
    let args = shifted("0.5", &candidate);
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = f.lab(&borrowed);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let args = shifted("1.0", &reference);
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = f.lab(&borrowed);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    let out = f.root.join("reference-compare");
    let result = f.lab(&[
        "compare",
        "readings",
        "--baseline",
        baseline.to_str().unwrap(),
        "--candidate",
        candidate.to_str().unwrap(),
        "--reference",
        reference.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(out.join("comparison.json")).unwrap()).unwrap();
    let summaries = report["summaries"].as_array().unwrap();
    let median = |side: &str, stage: &str| -> f64 {
        summaries
            .iter()
            .find(|summary| summary["side"] == side && summary["stage"] == stage)
            .unwrap_or_else(|| panic!("no {side} summary for {stage}: {report}"))["median"]
            .as_f64()
            .unwrap()
    };
    // Every pairing reports all three roles, and each name is its own
    // arithmetic: candidate - baseline, candidate - reference, baseline -
    // reference.
    let half_stop = std::f64::consts::LOG10_2 / 2.0;
    // The margin is the measurement's own f64 spread over a 12-sample chart
    // (measured 7.4e-9 from the exact half stop).
    assert!(
        (median("candidate_minus_baseline", "log_e_film") - half_stop).abs() < 1e-6,
        "{report}"
    );
    assert!(
        (median("candidate_minus_reference", "log_e_film") + half_stop).abs() < 1e-6,
        "the reference is brighter, so the candidate minus it is negative: {report}"
    );
    assert!(
        (median("baseline_minus_reference", "log_e_film") + 2.0 * half_stop).abs() < 1e-6,
        "{report}"
    );
    // A stage whose response is not affine in the compensation still carries
    // the roles in the direction their names declare.
    assert!(
        median("candidate_minus_baseline", "cmy_film") > 0.0,
        "{report}"
    );
    assert!(
        median("candidate_minus_reference", "cmy_film") < 0.0,
        "{report}"
    );
    assert!(
        median("baseline_minus_reference", "cmy_film") < 0.0,
        "{report}"
    );
    assert!(
        median("candidate_minus_baseline", "rgb_out") != 0.0,
        "{report}"
    );
    // The reference's own identity differences are not relabelled as the
    // candidate's.
    let differences = report["identity_differences"].as_array().unwrap();
    assert!(
        differences
            .iter()
            .any(|difference| difference["comparison"] == "candidate_minus_reference"),
        "{report}"
    );
    // Differences are reported under the roles they were taken from: the
    // candidate/reference pairing names its two sides.
    assert!(
        differences.iter().any(|difference| {
            difference["comparison"] == "candidate_minus_reference"
                && difference.get("candidate").is_some()
                && difference.get("reference").is_some()
        }),
        "{report}"
    );
    assert!(
        differences.iter().all(|difference| {
            difference.get("candidate").is_some() || difference.get("baseline").is_some()
        }),
        "every difference names the side it was taken from: {report}"
    );
}

#[test]
fn measure_reads_back_identities_that_carry_separators() {
    let f = MeasureFixture::new();
    let bundle = f.root.join("bundle-separators");
    assert!(f.process(&bundle).status.success());
    let plan = f.root.join("plan-separators.json");
    let awkward = "patch_01_A";
    std::fs::write(
        &plan,
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "id": "separators",
            "name": "separators",
            "regions": [
                {
                    "id": awkward,
                    "partition": "all",
                    "source": "chart",
                    "geometry": {"x": 0, "y": 0, "width": 1, "height": 1, "space": "working_pixels"},
                    "role": "other",
                    "measurements": ["numeric"],
                },
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    let sample = f.root.join("sample-separators");
    let result = f.lab(&[
        "sample",
        "--from-evidence",
        bundle.to_str().unwrap(),
        "--stage",
        "log_e_film",
        "--regions",
        plan.to_str().unwrap(),
        "--out",
        sample.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // Comparing the reading with itself must read back the identity it wrote.
    let out = f.root.join("compare-separators");
    let result = f.lab(&[
        "compare",
        "readings",
        "--baseline",
        sample.to_str().unwrap(),
        "--candidate",
        sample.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(out.join("comparison.json")).unwrap()).unwrap();
    let summaries = report["summaries"].as_array().unwrap();
    assert!(!summaries.is_empty(), "{report}");
    assert_eq!(summaries[0]["source"], "chart");
    assert_eq!(summaries[0]["partition"], "all");
    let csv = std::fs::read_to_string(out.join("deltas.csv")).unwrap();
    // The written identity remains stable through sampling and comparison.
    assert!(csv.contains("patch_01_A"), "{csv}");
    assert!(
        csv.starts_with("pair,sample_id,source,partition,stage,"),
        "{csv}"
    );
}

#[test]
fn measure_gates_a_narrower_range_by_the_path_it_executes() {
    let f = MeasureFixture::new();
    // A narrower range walks the tap topology from the entry to the last
    // reading, and the nodes on that path fire whatever the route says: the
    // `input` route skips the film stage only when the whole chain runs.
    let short = f.root.join("short-input-route");
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.stimuli.to_str().unwrap(),
        "--enter",
        "rgb_in",
        "--read",
        "log_e_film",
        "--route",
        "input",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        short.to_str().unwrap(),
    ]);
    assert!(
        !result.status.success(),
        "a short range on the input route meters the stimulus"
    );
    let error = String::from_utf8_lossy(&result.stderr);
    for path in ["camera.auto_exposure", "film_render.halation.active"] {
        assert!(error.contains(path), "no mention of {path}: {error}");
    }
    assert!(!short.exists(), "a refused run writes nothing");

    let result = f.lab(&[
        "run",
        "--stimuli",
        f.stimuli.to_str().unwrap(),
        "--enter",
        "rgb_in",
        "--read",
        "log_e_film",
        "--route",
        "input",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        short.to_str().unwrap(),
        "--set",
        "camera.auto_exposure=false",
        "--set",
        "film_render.halation.active=false",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    // The same holds on a convert route: the conversion replaces the film
    // exposure and the metering, but the development stage still runs on this
    // path, so its controls are listed.
    let converted = f.root.join("short-convert-route");
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.stimuli.to_str().unwrap(),
        "--enter",
        "rgb_in",
        "--read",
        "cmy_film",
        "--route",
        "input > convert-film > scan",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        converted.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    for path in [
        "film_render.halation.active",
        "film_render.dir_couplers.active",
        "film_render.grain.active",
    ] {
        assert!(error.contains(path), "no mention of {path}: {error}");
    }
    assert!(
        !error.contains("camera.auto_exposure"),
        "the convert route skips the metering on every path: {error}"
    );
}

#[test]
fn measure_compare_keeps_incomparable_pairs_out_of_the_deltas() {
    let f = MeasureFixture::new();
    // Two samples of the same frame read in different color spaces: the
    // comparison can report both readings, but not a channel delta between
    // them, and its CSV must say so.
    let mut sides = Vec::new();
    for (space, name) in [("sRGB", "sample-srgb"), ("ProPhoto RGB", "sample-prophoto")] {
        let out = f.root.join(name);
        let result = f.lab(&[
            "sample",
            "--source",
            f.root.join("input.png").to_str().unwrap(),
            "--input-color-space",
            space,
            "--input-cctf-decoding",
            "false",
            "--regions",
            f.plan.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        sides.push(out);
    }
    let out = f.root.join("incomparable-compare");
    let result = f.lab(&[
        "compare",
        "readings",
        "--baseline",
        sides[0].to_str().unwrap(),
        "--candidate",
        sides[1].to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(out.join("comparison.json")).unwrap()).unwrap();
    assert!(
        report["unavailable"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["metric"] == "channel_delta"
                    && entry["stage"] == "source"
                    && entry["reason"]
                        .as_str()
                        .is_some_and(|reason| reason.starts_with("color interpretation differs"))
            }),
        "{report}"
    );
    // The record calls the metric unavailable, so the file must not carry the
    // numbers a consumer would aggregate.
    let csv = std::fs::read_to_string(out.join("deltas.csv")).unwrap();
    let header: Vec<&str> = csv.lines().next().unwrap().split(',').collect();
    let column = |name: &str| header.iter().position(|field| *field == name).unwrap();
    for line in csv.lines().skip(1) {
        let fields: Vec<&str> = line.split(',').collect();
        for name in ["d0", "d1", "d2"] {
            assert!(fields[column(name)].is_empty(), "{line}");
        }
        assert_eq!(fields[column("valid")], "false", "{line}");
    }
}

#[test]
fn measure_compare_refuses_an_incompatible_shared_key() {
    let f = MeasureFixture::new();
    let baseline = f.root.join("clash-baseline");
    let candidate = f.root.join("clash-candidate");
    assert!(f.measure(&baseline).status.success());
    assert!(f.measure(&candidate).status.success());

    // A shared key whose own declaration disagrees is not a comparison of the
    // same quantity: the command names the key, the field, and both sides
    // instead of dropping the key or truncating the join.
    let points = baseline.join("readings.csv");
    let text = std::fs::read_to_string(&points).unwrap();
    let mut lines = text.lines();
    let header = lines.next().unwrap().to_owned();
    let units = header
        .split(',')
        .position(|field| field == "units")
        .unwrap();
    let mut rewritten = vec![header];
    for (index, line) in lines.enumerate() {
        let mut fields: Vec<String> = line.split(',').map(str::to_owned).collect();
        if index == 0 {
            fields[units] = "density".to_owned();
        }
        rewritten.push(fields.join(","));
    }
    std::fs::write(&points, format!("{}\n", rewritten.join("\n"))).unwrap();

    let out = f.root.join("clash-compare");
    let result = f.lab(&[
        "compare",
        "readings",
        "--baseline",
        baseline.to_str().unwrap(),
        "--candidate",
        candidate.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("declares units density"), "{error}");
    assert!(error.contains("baseline side"), "{error}");
    assert!(error.contains("candidate side"), "{error}");
    assert!(!out.exists(), "an incompatible comparison writes nothing");
}

#[test]
fn measure_gates_the_operations_the_route_executes() {
    let f = MeasureFixture::new();
    // The route decides which boundaries run, so the guard follows the
    // pipeline rather than the flag names: a convert route builds the film
    // density by inverting the scan model, and the `input` route runs neither
    // the film, the print, nor the scanner stage.
    let route_args = |out: &Path, route: Option<&str>, quiet: bool| -> Vec<String> {
        let mut args: Vec<String> = vec![
            "run".to_owned(),
            "--stimuli".to_owned(),
            f.stimuli.display().to_string(),
            "--enter".to_owned(),
            "rgb_in".to_owned(),
            "--read".to_owned(),
            "rgb_out".to_owned(),
            "--film".to_owned(),
            "kodak_portra_400".to_owned(),
            "--data-dir".to_owned(),
            f.data.display().to_string(),
            "--out".to_owned(),
            out.display().to_string(),
        ];
        if let Some(route) = route {
            args.push("--route".to_owned());
            args.push(route.to_owned());
        }
        if quiet {
            for value in ["scanner.lens_blur=0", "scanner.unsharp_mask=[0,0]"] {
                args.push("--set".to_owned());
                args.push(value.to_owned());
            }
        }
        args
    };
    let run = |args: &[String]| -> (bool, String) {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let result = f.lab(&borrowed);
        (
            result.status.success(),
            String::from_utf8_lossy(&result.stderr).into_owned(),
        )
    };

    // The camera and development controls cannot reach a convert-route
    // reading, so leaving them active is not a violation.
    let (ok, stderr) = run(&route_args(
        &f.root.join("route-convert"),
        Some("input > convert-film > scan"),
        true,
    ));
    assert!(ok, "{stderr}");

    // The `input` route executes no film, print, or scanner stage, so even the
    // controls the fixture leaves active cannot change the reading.
    let (ok, stderr) = run(&route_args(
        &f.root.join("route-input"),
        Some("input"),
        false,
    ));
    assert!(ok, "{stderr}");

    // The default route still refuses the same chain, naming the scanner
    // control that would change the reading.
    let (ok, stderr) = run(&route_args(&f.root.join("route-film"), None, false));
    assert!(
        !ok,
        "the default route must refuse an active scanner control"
    );
    assert!(stderr.contains("scanner.unsharp_mask"), "{stderr}");
}

#[test]
fn measure_compare_names_a_stage_whose_readings_are_all_invalid() {
    let f = MeasureFixture::new();
    let bundle = f.root.join("bundle");
    let sample = f.root.join("invalid-sample");
    assert!(f.process(&bundle).status.success());
    let result = f.lab(&[
        "sample",
        "--from-evidence",
        bundle.to_str().unwrap(),
        "--stage",
        "rgb_in",
        "--regions",
        f.plan.to_str().unwrap(),
        "--out",
        sample.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );

    // The writer's own representation of a non-finite reading: empty channel
    // fields with `valid` false. A colorimetric stage like this has an
    // eligible interpretation and no computable distance, so the comparison
    // must name the reason rather than drop the stage.
    let points = sample.join("readings.csv");
    let text = std::fs::read_to_string(&points).unwrap();
    let mut lines = text.lines();
    let header = lines.next().unwrap().to_owned();
    let columns: Vec<String> = header.split(',').map(str::to_owned).collect();
    let column = |name: &str| {
        columns
            .iter()
            .position(|field| field == name)
            .unwrap_or_else(|| panic!("no {name} column in {header}"))
    };
    let (c0, c1, c2, valid) = (column("c0"), column("c1"), column("c2"), column("valid"));
    let mut rewritten = vec![header];
    for line in lines {
        let mut fields: Vec<String> = line.split(',').map(str::to_owned).collect();
        for index in [c0, c1, c2] {
            fields[index] = String::new();
        }
        fields[valid] = "false".to_owned();
        rewritten.push(fields.join(","));
    }
    std::fs::write(&points, format!("{}\n", rewritten.join("\n"))).unwrap();

    let out = f.root.join("invalid-compare");
    let result = f.lab(&[
        "compare",
        "readings",
        "--baseline",
        sample.to_str().unwrap(),
        "--candidate",
        sample.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value =
        serde_json::from_slice(&std::fs::read(out.join("comparison.json")).unwrap()).unwrap();
    assert_eq!(report["stages"][0]["colorimetric"], true);
    assert!(
        report["unavailable"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["metric"] == "delta_e_2000" && entry["reason"] == "all readings invalid"
            }),
        "{report}"
    );
}

#[test]
fn publish_staged_leaves_nothing_when_the_files_cannot_be_written() {
    let root = std::env::temp_dir().join(format!("spektrafilm-lab-publish-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let staging = root.with_file_name(format!(".publish-staging-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("log_e_film.f64"), b"staged").unwrap();
    // The destination already holds one of the artifacts: the write must fail
    // and the staged capture must not be published.
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("run.json"), "{}").unwrap();

    let result = spektrafilm_cli::lab::report::publish_staged(
        &root,
        &staging,
        &[
            ("run.json", "{}".to_owned()),
            ("readings.csv", String::new()),
        ],
    );
    assert!(result.is_err(), "an existing artifact must not be replaced");
    assert!(
        !root.join("evidence").exists(),
        "the staged capture stays unpublished"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("run.json")).unwrap(),
        "{}",
        "the existing artifact is untouched"
    );
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&staging);
}

#[test]
fn measure_refuses_a_stimulus_whose_sample_identity_is_ambiguous() {
    let f = MeasureFixture::new();
    let duplicated = f.root.join("duplicated.json");
    std::fs::write(
        &duplicated,
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "name": "duplicated",
            "domain": "scene_rgb",
            "color_space": "sRGB",
            "cctf_decoding": false,
            "points": {"path": "points.npy"},
            "samples": [
                {"id": "p", "source": "chart", "partition": "all", "row": 0},
                {"id": "p", "source": "chart", "partition": "all", "row": 1},
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    let out = f.root.join("duplicated-out");
    let result = f.lab(&[
        "run",
        "--stimuli",
        duplicated.to_str().unwrap(),
        "--enter",
        "rgb_in",
        "--read",
        "rgb_in",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(
        error.contains("duplicate sample id") && error.contains("\"p\""),
        "{error}"
    );
    assert!(!out.exists());
}

#[test]
fn measure_records_the_parameter_files_it_declared() {
    let f = MeasureFixture::new();
    let edits = f.root.join("edits.toml");
    std::fs::write(
        &edits,
        "[film_render.grain]\nactive = false\n[film_render.halation]\nactive = false\n\
         [print_render.glare]\nactive = false\n",
    )
    .unwrap();
    let out = f.root.join("with-file");
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.stimuli.to_str().unwrap(),
        "--enter",
        "rgb_in",
        "--read",
        "rgb_in",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--set",
        edits.to_str().unwrap(),
        "--set",
        "camera.auto_exposure=false",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let record: Value =
        serde_json::from_slice(&std::fs::read(out.join("run.json")).unwrap()).unwrap();
    let files = record["identity"]["parameters"]["files"]
        .as_array()
        .unwrap();
    assert_eq!(files.len(), 1, "{record}");
    let path = files[0]["path"].as_str().unwrap();
    assert!(path.ends_with("edits.toml"), "{path}");
    assert!(
        Path::new(path).is_absolute(),
        "a declared file is recorded absolutely: {path}"
    );
    assert_eq!(
        files[0]["sha256"].as_str().unwrap().len(),
        64,
        "the file's content is hashed"
    );
    // A declaration re-resolves the same file from any working directory.
    assert_eq!(
        record["declaration"]["sources"][0].as_str().unwrap(),
        path,
        "the declaration carries the canonical path"
    );
    assert!(record["identity"]["renderer"]["sha256"].is_string());
}

#[test]
fn measure_resolves_the_data_directory_from_the_environment() {
    let f = MeasureFixture::new();
    let out = f.root.join("env-data-dir");
    let mut args = vec![
        "run",
        "--stimuli",
        f.stimuli.to_str().unwrap(),
        "--enter",
        "rgb_in",
        "--film",
        "kodak_portra_400",
        "--read",
        "log_e_film",
        "--dry-run",
        "--out",
        out.to_str().unwrap(),
    ];
    args.extend_from_slice(&POINT_WISE);
    let result = f.lab_with_env(&args, &[("SPEKTRAFILM_DATA_DIR", f.data.to_str().unwrap())]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let record: Value =
        serde_json::from_slice(&std::fs::read(out.join("run.json")).unwrap()).unwrap();
    let resolved = std::fs::canonicalize(&f.data).unwrap();
    assert_eq!(
        record["declaration"]["data_dir"],
        resolved.to_str().unwrap()
    );
    assert_eq!(
        record["identity"]["data_dir"]["path"],
        resolved.to_str().unwrap()
    );
    assert!(
        record["identity"]["data_dir"]["manifest_sha256"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64),
        "{record}"
    );
}

#[test]
fn measure_verify_reports_identity_differences_without_failing() {
    let f = MeasureFixture::new();
    let lab = f.root.join("lab");
    assert!(f.measure(&lab).status.success());

    let result = f.lab(&[
        "verify",
        "--record",
        lab.to_str().unwrap(),
        "--data-dir",
        f.data.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["kind"], "verify");
    assert!(
        report["identity_differences"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(report["buffer_differences"].as_array().unwrap().is_empty());

    // Without `--data-dir` the command re-resolves against the directory the
    // run recorded, so a bare re-verification reproduces the recorded chain.
    let result = f.lab(&["verify", "--record", lab.to_str().unwrap()]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(
        report["identity_differences"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{report}"
    );
    assert_eq!(
        report["data_dir_differences"]["recorded"],
        report["data_dir_differences"]["current"]
    );

    // The same inventory reached through another path spelling resolves to the
    // same identity: the difference vocabulary is about the directory's
    // identity and content, not about how it was spelled.
    let odd = f.data.join(".");
    let result = f.lab(&[
        "verify",
        "--record",
        lab.to_str().unwrap(),
        "--data-dir",
        odd.to_str().unwrap(),
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(
        report["identity_differences"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{report}"
    );

    // Every retained buffer is re-hashed, not only the requested stages: a
    // reading that asked for `log_e_film` still retains `rgb_out` at its
    // default collection boundary, and damaging it must be reported.
    let limited = f.root.join("verify-limited");
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.stimuli.to_str().unwrap(),
        "--enter",
        "rgb_in",
        "--read",
        "log_e_film",
        "--film",
        "kodak_portra_400",
        "--data-dir",
        f.data.to_str().unwrap(),
        "--out",
        limited.to_str().unwrap(),
        "--set",
        "camera.auto_exposure=false",
        "--set",
        "film_render.halation.active=false",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let untouched = f.lab(&["verify", "--record", limited.to_str().unwrap()]);
    let report: Value = serde_json::from_slice(&untouched.stdout).unwrap();
    assert!(
        report["buffer_differences"].as_array().unwrap().is_empty(),
        "{report}"
    );
    let retained = limited.join("evidence").join("rgb_in.f64");
    assert!(
        retained.is_file(),
        "the collection boundary retains rgb_out"
    );
    let mut bytes = std::fs::read(&retained).unwrap();
    bytes[0] ^= 1;
    std::fs::write(&retained, &bytes).unwrap();
    let damaged = f.lab(&["verify", "--record", limited.to_str().unwrap()]);
    assert!(damaged.status.success());
    let report: Value = serde_json::from_slice(&damaged.stdout).unwrap();
    let differences = report["buffer_differences"].as_array().unwrap();
    assert_eq!(differences.len(), 1, "{report}");
    assert_eq!(differences[0]["stage"], "rgb_in");

    // A record that no longer matches the re-resolution is a reported
    // difference, not a failure: the command still exits zero and writes
    // nothing.
    let record_path = lab.join("run.json");
    let mut tampered: Value =
        serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    tampered["identity"]["renderer"]["sha256"] = Value::String("0".repeat(64));
    let tampered_bytes = serde_json::to_vec_pretty(&tampered).unwrap();
    std::fs::write(&record_path, &tampered_bytes).unwrap();
    let result = f.lab(&["verify", "--record", lab.to_str().unwrap()]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    let differences = report["identity_differences"].as_array().unwrap();
    assert_eq!(differences.len(), 1, "{report}");
    assert_eq!(differences[0]["path"], "renderer.sha256");
    assert!(report["buffer_differences"].as_array().unwrap().is_empty());
    assert_eq!(
        std::fs::read(&record_path).unwrap(),
        tampered_bytes,
        "verify must not rewrite the run it verifies"
    );
}

/// A comparison record is verified as a comparison: the record file is found
/// rather than assumed, the sides it names are re-hashed from their own
/// `run.json`, and each difference names the side it belongs to. A comparison
/// directory holds `comparison.json`, never `run.json`.
#[test]
fn measure_verify_reads_a_comparison_record_and_rehashes_its_sides() {
    let f = MeasureFixture::new();
    let bundle = f.root.join("verify-bundle");
    assert!(
        f.process(&bundle).status.success(),
        "{}",
        String::from_utf8_lossy(&f.process(&bundle).stderr)
    );
    let lab = f.root.join("verify-lab");
    assert!(
        f.measure_with(&f.stimuli, &lab).status.success(),
        "{}",
        String::from_utf8_lossy(&f.measure_with(&f.stimuli, &lab).stderr)
    );
    let sample = f.root.join("verify-sample");
    assert!(
        f.lab(&[
            "sample",
            "--from-evidence",
            bundle.to_str().unwrap(),
            "--stage",
            "log_e_film",
            "--regions",
            f.working_plan.to_str().unwrap(),
            "--out",
            sample.to_str().unwrap(),
        ])
        .status
        .success()
    );
    let reference = f.root.join("verify-reference");
    assert!(
        f.measure_with(&f.stimuli, &reference).status.success(),
        "{}",
        String::from_utf8_lossy(&f.measure_with(&f.stimuli, &reference).stderr)
    );
    let comparison = f.root.join("verify-comparison");
    assert!(
        f.lab(&[
            "compare",
            "readings",
            "--baseline",
            lab.to_str().unwrap(),
            "--candidate",
            sample.to_str().unwrap(),
            "--reference",
            reference.to_str().unwrap(),
            "--out",
            comparison.to_str().unwrap(),
        ])
        .status
        .success(),
        "a reference reading selects the reading comparison"
    );

    // An untouched comparison reports no difference, and its own record is
    // never rewritten.
    let record_path = comparison.join("comparison.json");
    let recorded = std::fs::read(&record_path).unwrap();
    let untouched = f.lab(&["verify", "--record", comparison.to_str().unwrap()]);
    assert!(
        untouched.status.success(),
        "{}",
        String::from_utf8_lossy(&untouched.stderr)
    );
    let report: Value = serde_json::from_slice(&untouched.stdout).unwrap();
    assert_eq!(report["kind"], "verify");
    assert!(
        report["identity_differences"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{report}"
    );
    assert!(
        report["buffer_differences"].as_array().unwrap().is_empty(),
        "{report}"
    );
    assert_eq!(
        std::fs::read(&record_path).unwrap(),
        recorded,
        "verify must not rewrite the comparison it verifies"
    );

    // A side whose retained reading no longer matches the hash the comparison
    // recorded is reported for that side, and the command still exits zero.
    let mut document: Value = serde_json::from_slice(&recorded).unwrap();
    document["baseline"]["measure_sha256"] = Value::String("0".repeat(64));
    let tampered_bytes = serde_json::to_vec_pretty(&document).unwrap();
    std::fs::write(&record_path, &tampered_bytes).unwrap();
    let result = f.lab(&["verify", "--record", comparison.to_str().unwrap()]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    let differences = report["identity_differences"].as_array().unwrap();
    assert_eq!(differences.len(), 1, "{report}");
    assert_eq!(differences[0]["field"], "baseline.measure_sha256");
    assert_eq!(
        differences[0]["path"],
        lab.to_str().unwrap().to_owned(),
        "each difference names the side it belongs to"
    );
    assert_eq!(differences[0]["recorded"], "0".repeat(64));
    assert!(
        report["buffer_differences"].as_array().unwrap().is_empty(),
        "{report}"
    );
}

/// A rejected invocation exits 2, not 1: the exit status separates an
/// argument syntax error from a failure to execute. The spec grammar makes
/// `--enter` mandatory in the `--stimuli` form and `--stage` mandatory in the
/// `--from-evidence` form, so omitting either is a rejected invocation rather
/// than a failed execution.
#[test]
fn rejected_invocations_exit_two_and_failed_executions_exit_one() {
    let f = MeasureFixture::new();
    // A grammar violation the parser rejects before any execution reports the
    // usage error on stderr and exits 2.
    for args in [
        vec![
            "sample",
            "--from-evidence",
            "bundle",
            "--regions",
            f.plan.to_str().unwrap(),
            "--out",
            "out",
        ],
        vec![
            "sample",
            "--source",
            "image.png",
            "--regions",
            f.plan.to_str().unwrap(),
            "--out",
            "out",
        ],
    ] {
        let result = f.lab(&args);
        assert_eq!(
            result.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            result.stdout.is_empty(),
            "a rejected invocation writes no command result"
        );
        assert!(!result.stderr.is_empty(), "it must say why it was rejected");
    }
    // A form that parses but cannot execute still reaches the command, which
    // reports the failed execution on stdout and exits 2 as well, because a
    // missing mandatory companion is an argument syntax error, not a failure.
    let result = f.lab(&[
        "run",
        "--stimuli",
        f.stimuli.to_str().unwrap(),
        "--out",
        "out",
    ]);
    assert_eq!(
        result.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["execution_status"], "failed", "{report}");
    assert_eq!(report["errors"][0]["code"], "argument_syntax", "{report}");
    // A form that parses but cannot execute reports a failed execution with
    // status 1, so the two failures stay distinguishable.
    let executed = f.lab(&[
        "sample",
        "--from-evidence",
        "no-such-bundle",
        "--stage",
        "log_e_film",
        "--regions",
        f.plan.to_str().unwrap(),
        "--out",
        "out",
    ]);
    assert_eq!(
        executed.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
}
