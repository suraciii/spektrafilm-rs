//! `spektrafilm-lab analyze` through the real binary.
//!
//! The product case it exists for: one render exists and its color science has
//! to be quantified without inventing a second one to compare it against.

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
                "measurements": ["color_noise"],
                "geometry": {"x": 0, "y": 0, "width": 32, "height": 32, "space": "render"},
            }],
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

fn protocol(root: &Path) -> PathBuf {
    let path = root.join("protocol.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "id": "color-noise-check",
            "measurements": ["color_noise"],
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
        .arg("--protocol")
        .arg(protocol(temp.path()))
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
    let protocol = protocol(temp.path());
    let analysis = temp.path().join("analysis");
    assert!(
        lab()
            .args(["analyze", "--source"])
            .arg(&source)
            .args(["--declaration", "sRGB:encoded", "--regions"])
            .arg(&regions)
            .arg("--protocol")
            .arg(&protocol)
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
            .arg("--protocol")
            .arg(&protocol)
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
