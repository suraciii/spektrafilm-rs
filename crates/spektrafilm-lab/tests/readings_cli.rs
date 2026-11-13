//! `sample` and `compare readings` through the real lab binary.
//!
//! The readings come from the recorded stage bundle the repository keeps as
//! the example of retained evidence: a test copies it, so no renderer runs and
//! the reader's contract is what is on trial.

#![cfg(feature = "precision-f64")]
use serde_json::{Value, json};
use spektrafilm_benchmark::common;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn lab() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
}

fn retained_bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../spektrafilm-benchmark/tests/fixtures/retained-run/cases/probe/seed-0/stage-evidence",
    )
}

/// A copy of the recorded bundle whose named stages carry an offset, with the
/// manifest digests re-derived so the copy is a valid retained capture.
fn shifted_bundle(destination: &Path, offsets: &[(&str, f64)]) -> PathBuf {
    let bundle = destination.join(format!("bundle-{}", offsets.len()));
    std::fs::create_dir_all(&bundle).unwrap();
    for entry in std::fs::read_dir(retained_bundle()).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), bundle.join(entry.file_name())).unwrap();
    }
    let manifest = bundle.join("stage-evidence.json");
    let mut document: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    for (stage, offset) in offsets {
        let record = document["stages"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|record| record["id"] == *stage)
            .unwrap_or_else(|| panic!("no {stage} stage"));
        let buffer = bundle.join(record["path"].as_str().unwrap());
        let bytes = std::fs::read(&buffer).unwrap();
        let values: Vec<f64> = bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|chunk| f64::from_le_bytes(*chunk) + offset)
            .collect();
        let rewritten: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        std::fs::write(&buffer, &rewritten).unwrap();
        record["sha256"] = json!(common::sha256_file(&buffer).unwrap());
    }
    common::dump(&manifest, &document).unwrap();
    bundle
}

/// A regions document: one rectangle per declared identity, in the coordinate
/// space the source declares. Geometry is never guessed.
fn plan(destination: &Path, name: &str, space: &str, regions: &[(&str, [u64; 4])]) -> PathBuf {
    labelled(destination, name, space, "chart", regions)
}

/// The same document under a different provenance label.
fn labelled(
    destination: &Path,
    name: &str,
    space: &str,
    source: &str,
    regions: &[(&str, [u64; 4])],
) -> PathBuf {
    let path = destination.join(format!("{name}.json"));
    let declared: Vec<Value> = regions
        .iter()
        .map(|(id, [x, y, width, height])| {
            json!({
                "id": id, "partition": "all", "source": source,
                "geometry": {"x": x, "y": y, "width": width, "height": height, "space": space},
                "role": "other", "measurements": ["numeric"],
            })
        })
        .collect();
    common::dump(
        &path,
        &json!({"schema_version": 1, "id": name, "name": name, "regions": declared}),
    )
    .unwrap();
    path
}

fn sample_from(bundle: &Path, stage: &str, regions: &Path, out: &Path) -> PathBuf {
    let result = lab()
        .args(["sample", "--from-evidence"])
        .arg(bundle)
        .args(["--stage", stage, "--regions"])
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

fn sample_source(image: &Path, space: &str, decoded: bool, regions: &Path, out: &Path) -> PathBuf {
    let result = lab()
        .args(["sample", "--source"])
        .arg(image)
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

/// The retained stage, read back: sampling reproduces the buffer it names.
#[test]
fn sample_reproduces_the_retained_stage_and_refuses_a_tampered_buffer() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = shifted_bundle(temp.path(), &[]);
    let regions = plan(
        temp.path(),
        "patch",
        "film_frame_pixels",
        &[("p00", [0, 0, 4, 4])],
    );
    let out = sample_from(
        &bundle,
        "log_e_film",
        &regions,
        &temp.path().join("reading"),
    );

    let csv = std::fs::read_to_string(out.join("readings.csv")).unwrap();
    let columns = columns(&csv);
    let row = &rows(&csv)[0];
    let read: Vec<f64> = ["c0", "c1", "c2"]
        .iter()
        .map(|name| row[columns[*name]].parse().unwrap())
        .collect();
    let buffer = std::fs::read(bundle.join("log_e_film.f64")).unwrap();
    let stored: Vec<f64> = buffer
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| f64::from_le_bytes(*chunk))
        .collect();
    for channel in 0..3 {
        let deviation = (read[channel] - stored[channel]).abs();
        assert!(
            deviation < 1e-12,
            "channel {channel}: {read:?} vs {stored:?}"
        );
    }

    // A buffer that no longer matches the digest its manifest declares cannot
    // be attributed to the capture, and sampling refuses it.
    let tampered = temp.path().join("tampered");
    std::fs::create_dir_all(&tampered).unwrap();
    for entry in std::fs::read_dir(&bundle).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), tampered.join(entry.file_name())).unwrap();
    }
    let mut bytes = std::fs::read(tampered.join("log_e_film.f64")).unwrap();
    bytes[0] ^= 1;
    std::fs::write(tampered.join("log_e_film.f64"), &bytes).unwrap();
    let refused = lab()
        .args(["sample", "--from-evidence"])
        .arg(&tampered)
        .args(["--stage", "log_e_film", "--regions"])
        .arg(&regions)
        .arg("--out")
        .arg(temp.path().join("refused"))
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("hashes"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(!temp.path().join("refused/readings.csv").exists());

    // A rectangle outside the source is refused before it reads a pixel.
    let outside = plan(
        temp.path(),
        "outside",
        "film_frame_pixels",
        &[("edge", [11, 0, 2, 1])],
    );
    let refused = lab()
        .args(["sample", "--from-evidence"])
        .arg(&bundle)
        .args(["--stage", "log_e_film", "--regions"])
        .arg(&outside)
        .arg("--out")
        .arg(temp.path().join("out-of-bounds"))
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("edge"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
}

/// A raster source is read by the lab reader, and a declaration beside the
/// file is not an input.
#[test]
fn sample_reads_a_raster_through_the_lab_reader() {
    let temp = tempfile::tempdir().unwrap();
    let image = temp.path().join("image.png");
    let pixels: Vec<u8> = (0..8 * 8).flat_map(|_| [10u8, 20, 30]).collect();
    image::save_buffer(&image, &pixels, 8, 8, image::ColorType::Rgb8).unwrap();
    let regions = plan(
        temp.path(),
        "window",
        "source_pixels",
        &[("w0", [1, 1, 4, 4])],
    );

    let read = |name: &str| {
        let out = sample_source(&image, "sRGB", false, &regions, &temp.path().join(name));
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

/// Two readings of the same capture: the deltas are zero, and a domain with
/// no colour space reports no distance instead of one.
#[test]
fn compare_readings_reports_zero_deltas_and_the_domains_it_cannot_compare() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = shifted_bundle(temp.path(), &[]);
    for (stage, distance, space) in [
        ("log_e_film", false, "film_frame_pixels"),
        ("rgb_out", true, "output_pixels"),
    ] {
        let regions = plan(temp.path(), stage, space, &[("p00", [0, 0, 4, 4])]);
        let baseline = sample_from(
            &bundle,
            stage,
            &regions,
            &temp.path().join(format!("{stage}-a")),
        );
        let candidate = sample_from(
            &bundle,
            stage,
            &regions,
            &temp.path().join(format!("{stage}-b")),
        );
        let out = temp.path().join(format!("{stage}-compare"));
        let report = readings(&baseline, &candidate, None, &out);
        assert!(
            report["identity_differences"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{stage}: {report}"
        );
        let csv = std::fs::read_to_string(out.join("deltas.csv")).unwrap();
        let columns = columns(&csv);
        for row in rows(&csv) {
            for name in ["d0", "d1", "d2"] {
                assert_eq!(row[columns[name]], "0", "{stage} {row:?}");
            }
            assert_eq!(
                row[columns["d_e00"]].is_empty(),
                !distance,
                "{stage} reports its colour distance only where one exists: {row:?}"
            );
        }
        let unavailable = report["unavailable"].as_array().unwrap();
        assert_eq!(
            unavailable.iter().any(|entry| {
                entry["stage"] == stage
                    && entry["metric"] == "delta_e_2000"
                    && entry["reason"] == "no declared color space"
            }),
            !distance,
            "{stage}: {report}"
        );
        let summaries = report["summaries"].as_array().unwrap();
        assert!(
            summaries.iter().any(|summary| summary["stage"] == stage),
            "{report}"
        );
    }
}

/// The same values under a different record identity: a differing record is
/// evidence, not an error.
#[test]
fn compare_readings_treats_a_different_identity_as_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = shifted_bundle(temp.path(), &[]);
    let first = plan(
        temp.path(),
        "first",
        "output_pixels",
        &[("p00", [0, 0, 4, 4])],
    );
    let second = labelled(
        temp.path(),
        "second",
        "output_pixels",
        "chart-rev",
        &[("p00", [0, 0, 4, 4])],
    );
    let baseline = sample_from(&bundle, "rgb_out", &first, &temp.path().join("a"));
    let candidate = sample_from(&bundle, "rgb_out", &second, &temp.path().join("b"));
    let out = temp.path().join("compare");
    let report = readings(&baseline, &candidate, None, &out);
    assert!(
        !report["identity_differences"]
            .as_array()
            .unwrap()
            .is_empty(),
        "the two records declare different regions documents: {report}"
    );
    for row in rows(&std::fs::read_to_string(out.join("deltas.csv")).unwrap()) {
        let columns = columns(&std::fs::read_to_string(out.join("deltas.csv")).unwrap());
        assert_eq!(row[columns["valid"]], "true", "{row:?}");
    }
}

/// Three readings one stop apart: every pairing reports all three roles under
/// its own arithmetic.
#[test]
fn compare_readings_pairs_three_roles_under_their_own_arithmetic() {
    let temp = tempfile::tempdir().unwrap();
    let regions = plan(
        temp.path(),
        "patch",
        "film_frame_pixels",
        &[("p00", [0, 0, 4, 4])],
    );
    let half_stop = std::f64::consts::LOG10_2 / 2.0;
    let baseline = sample_from(
        &shifted_bundle(temp.path(), &[]),
        "log_e_film",
        &regions,
        &temp.path().join("baseline"),
    );
    let candidate = sample_from(
        &shifted_bundle(temp.path(), &[]),
        "log_e_film",
        &regions,
        &temp.path().join("candidate"),
    );
    let reference = sample_from(
        &shifted_bundle(temp.path(), &[("log_e_film", half_stop)]),
        "log_e_film",
        &regions,
        &temp.path().join("reference"),
    );
    let out = temp.path().join("compare");
    let report = readings(&baseline, &candidate, Some(&reference), &out);
    let median = |side: &str| -> f64 {
        report["summaries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|summary| summary["side"] == side && summary["stage"] == "log_e_film")
            .unwrap_or_else(|| panic!("no {side} summary: {report}"))["median"]
            .as_f64()
            .unwrap()
    };
    assert!(
        (median("candidate_minus_baseline")).abs() < 1e-12,
        "{report}"
    );
    assert!(
        (median("candidate_minus_reference") + half_stop).abs() < 1e-12,
        "{report}"
    );
    assert!(
        (median("baseline_minus_reference") + half_stop).abs() < 1e-12,
        "{report}"
    );
    let differences = report["identity_differences"].as_array().unwrap();
    assert!(
        differences
            .iter()
            .all(|difference| difference.get("candidate").is_some()
                || difference.get("baseline").is_some()),
        "every difference names the side it was taken from: {report}"
    );
}

/// An identity that carries separators survives sampling and comparison.
#[test]
fn compare_readings_round_trips_identities_that_carry_separators() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = shifted_bundle(temp.path(), &[]);
    let regions = plan(
        temp.path(),
        "separators",
        "film_frame_pixels",
        &[("patch_01_A", [0, 0, 1, 1])],
    );
    let sample = sample_from(
        &bundle,
        "log_e_film",
        &regions,
        &temp.path().join("reading"),
    );
    let out = temp.path().join("compare");
    readings(&sample, &sample, None, &out);
    let report: Value =
        serde_json::from_slice(&std::fs::read(out.join("comparison.json")).unwrap()).unwrap();
    assert_eq!(report["summaries"][0]["source"], "chart");
    assert_eq!(report["summaries"][0]["partition"], "all");
    let csv = std::fs::read_to_string(out.join("deltas.csv")).unwrap();
    assert!(csv.contains("patch_01_A"), "{csv}");
    assert!(
        csv.starts_with("pair,sample_id,source,partition,stage,"),
        "{csv}"
    );
}

/// Two readings of one frame in different colour spaces: both readings
/// publish, and no channel delta exists between them.
#[test]
fn compare_readings_keeps_incomparable_pairs_out_of_the_deltas() {
    let temp = tempfile::tempdir().unwrap();
    let image = temp.path().join("image.png");
    let pixels: Vec<u8> = (0..8 * 8).flat_map(|_| [10u8, 20, 30]).collect();
    image::save_buffer(&image, &pixels, 8, 8, image::ColorType::Rgb8).unwrap();
    let regions = plan(
        temp.path(),
        "window",
        "source_pixels",
        &[("w0", [1, 1, 4, 4])],
    );
    let baseline = sample_source(&image, "sRGB", false, &regions, &temp.path().join("srgb"));
    let candidate = sample_source(
        &image,
        "ProPhoto RGB",
        false,
        &regions,
        &temp.path().join("prophoto"),
    );
    let out = temp.path().join("compare");
    let report = readings(&baseline, &candidate, None, &out);
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
    let csv = std::fs::read_to_string(out.join("deltas.csv")).unwrap();
    let columns = columns(&csv);
    for row in rows(&csv) {
        for name in ["d0", "d1", "d2"] {
            assert!(row[columns[name]].is_empty(), "{row:?}");
        }
        assert_eq!(row[columns["valid"]], "false", "{row:?}");
    }
}

/// A shared key whose own declaration disagrees is not a comparison of the
/// same quantity.
#[test]
fn compare_readings_refuses_an_incompatible_shared_key() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = shifted_bundle(temp.path(), &[]);
    let regions = plan(
        temp.path(),
        "patch",
        "film_frame_pixels",
        &[("p00", [0, 0, 4, 4])],
    );
    let baseline = sample_from(&bundle, "log_e_film", &regions, &temp.path().join("a"));
    let candidate = sample_from(&bundle, "log_e_film", &regions, &temp.path().join("b"));

    let points = baseline.join("readings.csv");
    let csv = std::fs::read_to_string(&points).unwrap();
    let columns = columns(&csv);
    let mut rewritten = vec![];
    for (index, line) in csv.lines().enumerate() {
        let mut fields: Vec<String> = line.split(',').map(str::to_owned).collect();
        if index == 1 {
            fields[columns["units"]] = "density".to_owned();
        }
        rewritten.push(fields.join(","));
    }
    std::fs::write(&points, format!("{}\n", rewritten.join("\n"))).unwrap();

    let out = temp.path().join("compare");
    let result = lab()
        .args(["compare", "readings", "--baseline"])
        .arg(&baseline)
        .arg("--candidate")
        .arg(&candidate)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("declares units density"), "{error}");
    assert!(error.contains("baseline side"), "{error}");
    assert!(error.contains("candidate side"), "{error}");
    assert!(!out.exists(), "an incompatible comparison writes nothing");
}

/// A stage whose readings are all invalid keeps its name instead of dropping
/// out of the comparison.
#[test]
fn compare_readings_names_a_stage_whose_readings_are_all_invalid() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = shifted_bundle(temp.path(), &[]);
    let regions = plan(
        temp.path(),
        "patch",
        "source_pixels",
        &[("p00", [0, 0, 4, 4])],
    );
    let sample = sample_from(&bundle, "rgb_in", &regions, &temp.path().join("reading"));

    let points = sample.join("readings.csv");
    let csv = std::fs::read_to_string(&points).unwrap();
    let columns = columns(&csv);
    let mut rewritten = vec![];
    for (index, line) in csv.lines().enumerate() {
        let mut fields: Vec<String> = line.split(',').map(str::to_owned).collect();
        if index > 0 {
            for name in ["c0", "c1", "c2"] {
                fields[columns[name]] = String::new();
            }
            fields[columns["valid"]] = "false".to_owned();
        }
        rewritten.push(fields.join(","));
    }
    std::fs::write(&points, format!("{}\n", rewritten.join("\n"))).unwrap();

    let out = temp.path().join("compare");
    let report = readings(&sample, &sample, None, &out);
    assert!(
        report["unavailable"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["stage"] == "rgb_in" && entry["metric"] == "delta_e_2000" }),
        "{report}"
    );
}

/// The two failures stay distinguishable: a rejected invocation exits 2 and
/// writes no command result, a failed execution exits 1 and reports itself.
#[test]
fn rejected_invocations_exit_two_and_failed_executions_exit_one() {
    let temp = tempfile::tempdir().unwrap();
    let regions = plan(
        temp.path(),
        "patch",
        "film_frame_pixels",
        &[("p00", [0, 0, 4, 4])],
    );
    for args in [
        vec![
            "sample",
            "--from-evidence",
            "bundle",
            "--regions",
            regions.to_str().unwrap(),
            "--out",
            "out",
        ],
        vec![
            "sample",
            "--source",
            "image.png",
            "--regions",
            regions.to_str().unwrap(),
            "--out",
            "out",
        ],
    ] {
        let result = lab().args(args).output().unwrap();
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

    let executed = lab()
        .args(["sample", "--from-evidence"])
        .arg(temp.path().join("no-such-bundle"))
        .args(["--stage", "log_e_film", "--regions"])
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
