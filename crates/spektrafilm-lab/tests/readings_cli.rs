#![cfg(feature = "precision-f64")]

//! `sample` and `compare readings` through the real lab binary.
//!
//! Every reading here comes from a raster the test wrote, so no renderer runs
//! and the reader's contract is what is on trial.

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn lab() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
}

/// A regions document: one rectangle per declared identity, in the coordinate
/// space the source declares. Geometry is never guessed.
fn plan(destination: &Path, name: &str, space: &str, regions: &[(&str, [u64; 4])]) -> PathBuf {
    let path = destination.join(format!("{name}.json"));
    let declared: Vec<Value> = regions
        .iter()
        .map(|(id, [x, y, width, height])| {
            json!({
                "id": id, "partition": "all", "source": "chart",
                "geometry": {"x": x, "y": y, "width": width, "height": height, "space": space},
                "role": "other",
            })
        })
        .collect();
    spektrafilm_lab::common::dump(
        &path,
        &json!({"schema_version": 1, "id": name, "name": name, "regions": declared}),
    )
    .unwrap();
    path
}

fn image(path: &Path, rgb: [u8; 3]) {
    let pixels: Vec<u8> = (0..8 * 8).flat_map(|_| rgb).collect();
    image::save_buffer(path, &pixels, 8, 8, image::ColorType::Rgb8).unwrap();
}

fn sample_source(source: &Path, space: &str, decoded: bool, regions: &Path, out: &Path) -> PathBuf {
    let result = lab()
        .args(["sample", "--source"])
        .arg(source)
        .args(["--input-color-space", space, "--input-cctf-decoding"])
        .arg(if decoded { "true" } else { "false" })
        .arg("--regions")
        .arg(regions)
        .arg("--out")
        .arg(out)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    out.to_path_buf()
}

fn readings(baseline: &Path, candidate: &Path, reference: Option<&Path>, out: &Path) -> Value {
    let mut command = lab();
    command
        .args(["compare", "readings", "--baseline"])
        .arg(baseline)
        .arg("--candidate")
        .arg(candidate);
    if let Some(reference) = reference {
        command.arg("--reference").arg(reference);
    }
    let result = command.arg("--out").arg(out).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&std::fs::read(out.join("comparison.json")).unwrap()).unwrap()
}

fn columns(csv: &str) -> BTreeMap<String, usize> {
    csv.lines()
        .next()
        .unwrap()
        .split(',')
        .enumerate()
        .map(|(index, field)| (field.to_owned(), index))
        .collect()
}

fn rows(csv: &str) -> Vec<Vec<String>> {
    csv.lines()
        .skip(1)
        .map(|line| line.split(',').map(str::to_owned).collect())
        .collect()
}

/// A raster is read by the lab reader: the declared decoding governs the
/// values, a sidecar beside the image changes nothing, and the record states
/// the source it read.
#[test]
fn sample_reads_a_raster_through_the_lab_reader() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("image.png");
    image(&source, [10, 20, 30]);
    let regions = plan(
        temp.path(),
        "window",
        "source_pixels",
        &[("w0", [1, 1, 4, 4])],
    );

    let read = |name: &str| {
        let out = sample_source(&source, "sRGB", false, &regions, &temp.path().join(name));
        let csv = std::fs::read_to_string(out.join("readings.csv")).unwrap();
        let columns = columns(&csv);
        let row = &rows(&csv)[0];
        ["c0", "c1", "c2"].map(|name| row[columns[name]].parse::<f64>().unwrap())
    };
    let stored = read("stored");
    for (channel, value) in [10.0, 20.0, 30.0].into_iter().enumerate() {
        let deviation = (stored[channel] - value / 255.0).abs();
        assert!(deviation < 1e-12, "channel {channel}: {stored:?}");
    }

    std::fs::write(
        temp.path().join("image.png.json"),
        r#"{"color_space":"ACES2065-1","cctf_decoding":true}"#,
    )
    .unwrap();
    assert_eq!(stored, read("with-sidecar"));
}

/// Two readings of the same pixels: every delta is zero, and the record keeps
/// the identity it compared instead of dropping the stage.
#[test]
fn compare_readings_reports_zero_deltas_for_one_source() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("image.png");
    image(&source, [64, 96, 128]);
    let regions = plan(temp.path(), "patch", "source", &[("p00", [0, 0, 4, 4])]);
    let baseline = sample_source(&source, "sRGB", false, &regions, &temp.path().join("a"));
    let candidate = sample_source(&source, "sRGB", false, &regions, &temp.path().join("b"));
    let out = temp.path().join("compare");
    let report = readings(&baseline, &candidate, None, &out);
    assert_eq!(report["identity_differences"], json!([]), "{report}");

    let csv = std::fs::read_to_string(out.join("deltas.csv")).unwrap();
    let columns = columns(&csv);
    for row in rows(&csv) {
        for name in ["d0", "d1", "d2"] {
            assert_eq!(row[columns[name]], "0", "{row:?}");
        }
        assert_eq!(row[columns["stage"]], "source", "{row:?}");
    }
}

/// A reference is a third role with its own arithmetic, not a relabelled
/// baseline or candidate.
#[test]
fn compare_readings_pairs_three_roles_under_their_own_arithmetic() {
    let temp = tempfile::tempdir().unwrap();
    let regions = plan(temp.path(), "patch", "source", &[("p00", [0, 0, 4, 4])]);
    let sample = |name: &str, rgb: [u8; 3]| {
        let path = temp.path().join(format!("{name}.png"));
        image(&path, rgb);
        sample_source(&path, "sRGB", false, &regions, &temp.path().join(name))
    };
    let baseline = sample("baseline", [0, 0, 0]);
    let candidate = sample("candidate", [51, 0, 0]);
    let reference = sample("reference", [102, 0, 0]);
    let out = temp.path().join("compare");
    let report = readings(&baseline, &candidate, Some(&reference), &out);
    assert_eq!(report["execution_status"], "completed", "{report}");

    let csv = std::fs::read_to_string(out.join("deltas.csv")).unwrap();
    let columns = columns(&csv);
    let pair = |name: &str| {
        rows(&csv)
            .into_iter()
            .find(|row| row[columns["pair"]] == name)
            .unwrap_or_else(|| panic!("no {name} pair"))
    };
    let primary = pair("candidate_minus_baseline");
    let against = pair("candidate_minus_reference");
    let d0 = |row: &[String]| row[columns["d0"]].parse::<f64>().unwrap();
    assert!((d0(&primary) - 51.0 / 255.0).abs() < 1e-12, "{primary:?}");
    assert!((d0(&against) + 51.0 / 255.0).abs() < 1e-12, "{against:?}");
}

/// A record that declares another unit is evidence: the comparison reports
/// the difference and still publishes the numbers it could compare.
#[test]
fn compare_readings_reports_a_changed_identity_as_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("image.png");
    image(&source, [64, 96, 128]);
    let regions = plan(temp.path(), "patch", "source", &[("p00", [0, 0, 4, 4])]);
    let baseline = sample_source(&source, "sRGB", false, &regions, &temp.path().join("a"));
    let candidate = sample_source(&source, "sRGB", false, &regions, &temp.path().join("b"));

    let record = std::fs::read(candidate.join("run.json")).unwrap();
    let mut document: Value = serde_json::from_slice(&record).unwrap();
    document["stages"][0]["units"] = json!("log10");
    spektrafilm_lab::common::dump(&candidate.join("run.json"), &document).unwrap();

    let report = readings(&baseline, &candidate, None, &temp.path().join("compare"));
    let differences = report["identity_differences"].as_array().unwrap();
    assert!(
        differences.iter().any(|entry| entry["path"]
            .as_str()
            .is_some_and(|path| path.starts_with("stages[0]"))),
        "{report}"
    );
}

/// A refused invocation is a command result too.
#[test]
fn rejected_invocations_exit_two_and_failed_executions_exit_one() {
    let temp = tempfile::tempdir().unwrap();
    let regions = plan(temp.path(), "patch", "source", &[("p00", [0, 0, 4, 4])]);
    let result = lab()
        .args(["sample", "--from-evidence", "bundle", "--regions"])
        .arg(&regions)
        .arg("--out")
        .arg(temp.path().join("out"))
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let refused: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(refused["execution_status"], "failed");
    assert_eq!(refused["errors"][0]["code"], "argument_syntax");
    assert!(!result.stderr.is_empty(), "it must say why it was rejected");

    let executed = lab()
        .args(["sample", "--source"])
        .arg(temp.path().join("no-such-image.png"))
        .args([
            "--input-color-space",
            "sRGB",
            "--input-cctf-decoding",
            "false",
        ])
        .arg("--regions")
        .arg(&regions)
        .arg("--out")
        .arg(temp.path().join("out"))
        .output()
        .unwrap();
    assert_eq!(
        executed.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    let report: Value = serde_json::from_slice(&executed.stdout).unwrap();
    assert_eq!(report["execution_status"], "failed", "{report}");
}
