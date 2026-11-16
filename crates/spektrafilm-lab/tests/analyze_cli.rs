//! `spektrafilm-lab analyze` through the real binary.
//!
//! The product case it exists for: one render exists and its color science has
//! to be quantified without inventing a second one to compare it against.

#![cfg(feature = "precision-f64")]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn lab() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
}

fn png(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut image = image::RgbImage::new(32, 32);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let dither = if (x / 2 + y / 2) % 2 == 0 { 20 } else { 0 };
        *pixel = image::Rgb([120 + dither, 120 + dither, 120 + dither]);
    }
    image.save(path).unwrap();
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// A flat field, so the population has the structure `color_noise` requires.
fn regions(root: &Path) -> PathBuf {
    let path = root.join("regions.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "id": "flat",
            "regions": [{
                "id": "field",
                "role": "flat_field",
                "geometry": {"x": 0, "y": 0, "width": 32, "height": 32, "space": "render"},
            }],
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

#[test]
fn one_image_is_quantified_without_a_second_one() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("v1").join("field.png");
    png(&source);
    let out = temp.path().join("analysis");
    let output = lab()
        .args(["analyze", "--source"])
        .arg(&source)
        .args(["--declaration", "sRGB:encoded", "--regions"])
        .arg(regions(temp.path()))
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["command"], "analyze");
    assert_eq!(result["execution_status"], "completed");

    let record = read(&out.join("analysis.json"));
    assert_eq!(record["kind"], "analysis");
    assert_eq!(record["source"]["kind"], "images");
    // One source means one realisation: the record carries no seed, states no
    // judgement, and has no second side to name.
    assert!(record.get("evaluation_status").is_none());
    assert!(record.get("findings").is_none());
    assert!(record.get("candidate").is_none());
    let case = &record["cases"][0];
    assert!(case.get("seed").is_none());
    let measurements = case["measurements"].as_array().unwrap();
    assert!(!measurements.is_empty());
    for row in measurements {
        assert!(
            !row["metric"].as_str().unwrap().contains(".baseline."),
            "a single source names no side: {}",
            row["metric"]
        );
        assert!(row.get("seed_population").is_none());
    }
    assert!(
        measurements
            .iter()
            .any(|r| r["metric"] == "color_noise.luma_std" && r["value"].is_f64())
    );
}

/// The number an analysis reports is the number a comparison reports for the
/// same side. Otherwise the two endpoints would measure different quantities
/// and a caller could not move between them.
#[test]
fn analysis_and_comparison_report_the_same_measurement() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("v1").join("field.png");
    png(&source);
    let regions = regions(temp.path());
    let analysis = temp.path().join("analysis");
    assert!(
        lab()
            .args(["analyze", "--source"])
            .arg(&source)
            .args(["--declaration", "sRGB:encoded", "--regions"])
            .arg(&regions)
            .arg("--out")
            .arg(&analysis)
            .output()
            .unwrap()
            .status
            .success()
    );
    let comparison = temp.path().join("comparison");
    assert!(
        lab()
            .args(["compare", "--baseline"])
            .arg(&source)
            .args(["--candidate"])
            .arg(&source)
            .args(["--declaration", "sRGB:encoded", "--regions"])
            .arg(&regions)
            // The endpoints hold equal values for one profile, so the test
            // pins the profile rather than relying on each default.
            .args(["--analysis", "lab-analysis-v1"])
            .arg("--out")
            .arg(&comparison)
            .output()
            .unwrap()
            .status
            .success()
    );
    let single = read(&analysis.join("analysis.json"));
    let paired = read(&comparison.join("comparison.json"));
    let value = |record: &Value, metric: &str| {
        record["cases"][0]["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["metric"] == metric)
            .unwrap_or_else(|| panic!("missing {metric}"))["value"]
            .as_f64()
    };
    for name in [
        "mean_r",
        "luma_std",
        "chroma_rms",
        "chroma_to_luminance_ratio",
        "channel_correlation_rg",
        "radial_frequency_centroid",
    ] {
        let alone = value(&single, &format!("color_noise.{name}"));
        let side = value(&paired, &format!("color_noise.baseline.{name}"));
        assert_eq!(alone, side, "{name}");
    }
}

/// A difference is not a measurement of one source, and the record says so
/// instead of subtracting the image from itself.
#[test]
fn a_difference_family_names_the_second_source_it_needs() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("v1").join("field.png");
    png(&source);
    let out = temp.path().join("analysis");
    let output = lab()
        .args(["analyze", "--source"])
        .arg(&source)
        .args(["--declaration", "sRGB:encoded", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(output.status.success());
    let record = read(&out.join("analysis.json"));
    let rows = record["cases"][0]["measurements"].as_array().unwrap();
    for metric in ["numeric.max_abs_difference", "numeric.rmse"] {
        let row = rows
            .iter()
            .find(|row| row["metric"] == metric)
            .unwrap_or_else(|| panic!("missing {metric}"));
        assert_eq!(row["reason"], "requires_second_source");
        assert!(row["value"].is_null());
    }
    // The statistics that are not differences still measure.
    assert!(
        rows.iter()
            .any(|r| r["metric"] == "numeric.above_one_fraction" && r["value"].is_f64())
    );
}

/// The record the endpoint publishes is a document the instrument can check.
#[test]
fn the_published_record_validates_as_an_analysis() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("v1").join("field.png");
    png(&source);
    let out = temp.path().join("analysis");
    assert!(
        lab()
            .args(["analyze", "--source"])
            .arg(&source)
            .args(["--declaration", "sRGB:encoded", "--out"])
            .arg(&out)
            .output()
            .unwrap()
            .status
            .success()
    );
    let output = lab()
        .args(["validate", "--kind", "analysis", "--file"])
        .arg(out.join("analysis.json"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["kind"], "analysis");
    assert_eq!(result["execution_status"], "completed");
}

/// The directory form names a case the way the pairing form does, so a record
/// from either endpoint over the same directory can be joined to the other.
#[test]
fn the_directory_form_names_cases_the_way_a_comparison_does() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("v1").join("field.png");
    png(&source);
    let analysis = temp.path().join("analysis");
    assert!(
        lab()
            .args(["analyze", "--source"])
            .arg(source.parent().unwrap())
            .args(["--declaration", "sRGB:encoded", "--out"])
            .arg(&analysis)
            .output()
            .unwrap()
            .status
            .success()
    );
    let comparison = temp.path().join("comparison");
    assert!(
        lab()
            .args(["compare", "--baseline"])
            .arg(source.parent().unwrap())
            .args(["--candidate"])
            .arg(source.parent().unwrap())
            .args(["--declaration", "sRGB:encoded", "--out"])
            .arg(&comparison)
            .output()
            .unwrap()
            .status
            .success()
    );
    let alone = read(&analysis.join("analysis.json"));
    let paired = read(&comparison.join("comparison.json"));
    assert_eq!(alone["cases"][0]["case_id"], "field");
    assert_eq!(
        alone["cases"][0]["case_id"], paired["cases"][0]["case_id"],
        "one directory, one case identity"
    );
}

/// Two names that reduce to one identity are one case, and the record refuses
/// rather than reporting two cases for one name.
#[test]
fn two_names_that_reduce_to_one_identity_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("v1");
    png(&directory.join("field.png"));
    png(&directory.join("field.tif"));
    let output = lab()
        .args(["analyze", "--source"])
        .arg(&directory)
        .args(["--declaration", "sRGB:encoded", "--out"])
        .arg(temp.path().join("analysis"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("duplicate_case_identity"), "{error}");
}

/// A refusal claims nothing: the destination the caller named is still free
/// afterwards, so the same invocation can be retried once the input is fixed.
#[test]
fn a_refused_input_leaves_the_destination_unclaimed() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("analysis");
    let missing = temp.path().join("v1").join("absent.png");
    let output = lab()
        .args(["analyze", "--source"])
        .arg(&missing)
        .args(["--declaration", "sRGB:encoded", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!out.exists(), "a refusal must not claim a destination");

    // The same path works once the input exists, which is the point.
    png(&missing);
    assert!(
        lab()
            .args(["analyze", "--source"])
            .arg(&missing)
            .args(["--declaration", "sRGB:encoded", "--out"])
            .arg(&out)
            .output()
            .unwrap()
            .status
            .success()
    );
}

/// A container the instrument does not measure is refused by name, and no
/// destination is created for it.
#[test]
fn an_unmeasured_container_is_refused_by_name_and_claims_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("v1").join("field.png");
    png(&source);
    std::fs::write(source.parent().unwrap().join("notes.txt"), b"not an image").unwrap();
    let out = temp.path().join("analysis");
    let output = lab()
        .args(["analyze", "--source"])
        .arg(source.parent().unwrap())
        .args(["--declaration", "sRGB:encoded", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!out.exists(), "a refusal must not claim a destination");
    let error = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(error.contains("notes.txt"), "{error}");
    // The cause is the cause; the remedy is its own line.
    assert!(
        error.contains("not a container this instrument measures"),
        "{error}"
    );
}

/// A repeated region identity is refused where it is declared, not discovered
/// later as a measurement failure.
#[test]
fn a_repeated_region_identity_is_refused_before_measuring() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("v1").join("field.png");
    png(&source);
    let out = temp.path().join("comparison");
    let output = lab()
        .args(["compare", "--baseline"])
        .arg(&source)
        .arg("--candidate")
        .arg(&source)
        .args([
            "--declaration",
            "sRGB:encoded",
            "--roi",
            "field=0,0,16,16:flat_field",
            "--roi",
            "field=16,16,16,16:texture",
            "--out",
        ])
        .arg(&out)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!out.exists());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["errors"][0]["code"], "duplicate_region_identity");
}

/// A refusal a caller must act on states the cause and the remedy apart, and
/// both reach the human line and the machine record.
#[test]
fn a_refusal_states_its_cause_and_its_remedy_apart() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("v1").join("field.png");
    png(&source);
    let out = temp.path().join("analysis");
    assert!(
        lab()
            .args(["analyze", "--source"])
            .arg(&source)
            .args(["--declaration", "sRGB:encoded", "--out"])
            .arg(&out)
            .output()
            .unwrap()
            .status
            .success()
    );
    let repeat = lab()
        .args(["analyze", "--source"])
        .arg(&source)
        .args(["--declaration", "sRGB:encoded", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(repeat.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&repeat.stdout).unwrap();
    let error = &result["errors"][0];
    assert_eq!(error["code"], "destination_exists");
    let message = error["message"].as_str().unwrap();
    let remedy = error["remedy"].as_str().unwrap();
    assert!(message.contains("already exists"), "{message}");
    assert!(
        !message.contains("choose"),
        "the cause must not carry the remedy"
    );
    assert!(remedy.contains("choose"), "{remedy}");
    let stderr = String::from_utf8_lossy(&repeat.stderr);
    assert!(stderr.contains(message) && stderr.contains(remedy));
}

/// A region declares where it is and what it is a population of. It cannot
/// declare families, because the profile owns the one table that decides them.
#[test]
fn a_region_cannot_declare_the_families_it_measures() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("v1").join("field.png");
    png(&source);
    let regions = temp.path().join("declared.json");
    std::fs::write(
        &regions,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "id": "declared",
            "regions": [{
                "id": "field",
                "role": "flat_field",
                "measurements": ["color_noise"],
                "geometry": {"x": 0, "y": 0, "width": 32, "height": 32, "space": "render"},
            }],
        }))
        .unwrap(),
    )
    .unwrap();
    let out = temp.path().join("analysis");
    let output = lab()
        .args(["analyze", "--source"])
        .arg(&source)
        .args(["--declaration", "sRGB:encoded", "--regions"])
        .arg(&regions)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!out.exists(), "a refused document claims no destination");
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("measurements"), "{error}");
}

/// The family a number belongs to is in the record, so a reader groups by
/// concept instead of by a metric-name prefix.
#[test]
fn every_measurement_names_its_family() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("v1").join("field.png");
    png(&source);
    let out = temp.path().join("comparison");
    assert!(
        lab()
            .args(["compare", "--baseline"])
            .arg(&source)
            .arg("--candidate")
            .arg(&source)
            .args(["--declaration", "sRGB:encoded", "--roi"])
            .arg("field=0,0,32,32:flat_field")
            .arg("--out")
            .arg(&out)
            .output()
            .unwrap()
            .status
            .success()
    );
    let record = read(&out.join("comparison.json"));
    let protocol = &record["protocol"]["families"];
    let rows = record["cases"][0]["measurements"].as_array().unwrap();
    assert!(!rows.is_empty());
    for row in rows {
        let family = row["family"]
            .as_str()
            .unwrap_or_else(|| panic!("row names no family: {row}"));
        // The family's metric names are its own; a colour row is `delta_*`,
        // which is exactly why the prefix cannot be the answer.
        assert!(
            protocol["full_image"]
                .as_array()
                .unwrap()
                .iter()
                .chain(protocol["flat_field"].as_array().unwrap())
                .any(|name| name == family),
            "{family} is not a family this protocol selects"
        );
    }
}

/// One vocabulary, one list: the kinds `schema` prints and the kinds `validate`
/// decides come from the same table, and a kind outside it fails the same way
/// from either endpoint.
#[test]
fn the_document_vocabulary_has_one_owner() {
    let schema_list = lab().args(["schema", "--help"]).output().unwrap();
    let schema_list = String::from_utf8_lossy(&schema_list.stdout).to_string();
    let validate_list = lab().args(["validate", "--help"]).output().unwrap();
    let validate_list = String::from_utf8_lossy(&validate_list.stdout).to_string();
    // `validate` is a subset of `schema`: a document can be printed without a
    // rule to decide it, but not decided without a schema to check it against.
    for kind in ["regions", "analysis", "comparison", "suite", "target"] {
        assert!(schema_list.contains(kind), "schema cannot print {kind}");
        assert!(
            validate_list.contains(kind),
            "validate cannot decide {kind}"
        );
    }
    // A kind that is printable but not decidable appears in one list only.
    assert!(schema_list.contains("preflight"), "{schema_list}");
    assert!(!validate_list.contains("preflight"), "{validate_list}");
}

/// A measurement that did not run has no count to report. Writing zero would
/// state a fact about the population that the code never established.
#[test]
fn an_unavailable_measurement_reports_no_sample_count() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("v1").join("field.png");
    png(&source);
    let out = temp.path().join("analysis");
    assert!(
        lab()
            .args(["analyze", "--source"])
            .arg(&source)
            .args(["--declaration", "sRGB:encoded", "--out"])
            .arg(&out)
            .output()
            .unwrap()
            .status
            .success()
    );
    let record = read(&out.join("analysis.json"));
    let rows = record["cases"][0]["measurements"].as_array().unwrap();
    let unavailable: Vec<_> = rows
        .iter()
        .filter(|row| row["availability"] != "available")
        .collect();
    assert!(!unavailable.is_empty(), "the record states a refusal");
    for row in unavailable {
        assert!(
            row.get("sample_count").is_none(),
            "a refusal has no sample count: {row}"
        );
    }
    // The command result counts the regions the caller declared; the implicit
    // whole image is an identity, not a region anyone wrote.
    let result = read(&out.join("analysis.json"));
    assert_eq!(result["kind"], "analysis");
}
