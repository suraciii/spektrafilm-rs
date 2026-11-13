//! `compare images` product behaviour.

use serde_json::json;
use spektrafilm_benchmark::analysis;
use spektrafilm_benchmark::images::{self, Pair, Request};
use std::path::Path;

fn png(path: &Path, width: u32, height: u32, sample: impl Fn(u32, u32) -> [f64; 3]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Sixteen);
    let mut writer = encoder.write_header().unwrap();
    let mut bytes = Vec::with_capacity(width as usize * height as usize * 6);
    for y in 0..height {
        for x in 0..width {
            for value in sample(x, y) {
                line(&mut bytes, value);
            }
        }
    }
    writer.write_image_data(&bytes).unwrap();
}

fn line(bytes: &mut Vec<u8>, value: f64) {
    let scaled = (value * 65535.0).round().clamp(0.0, 65535.0) as u16;
    bytes.extend_from_slice(&scaled.to_be_bytes());
}

fn flat(value: f64) -> [f64; 3] {
    [value, value, value]
}

fn request(baseline: &Path, candidate: &Path) -> Request {
    let profile = analysis::profile("lab-appearance-v1").unwrap();
    Request {
        baseline_id: "v1".into(),
        candidate_id: "v2".into(),
        color_space: "sRGB".into(),
        cctf_decoding: true,
        pairs: vec![Pair {
            case_id: "chart".into(),
            baseline: baseline.into(),
            candidate: candidate.into(),
        }],
        regions: analysis::regions(
            profile,
            &[analysis::Region {
                id: "center".into(),
                x: 2,
                y: 2,
                width: 4,
                height: 4,
                role: "other".into(),
            }],
        ),
        protocol: analysis::protocol(profile),
    }
}

fn comparison(root: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(root.join("comparison.json")).unwrap()).unwrap()
}

#[test]
fn identical_images_measure_zero_and_publish_an_image_source() {
    let temp = tempfile::tempdir().unwrap();
    let left = temp.path().join("v1").join("chart.png");
    let right = temp.path().join("v2").join("chart.png");
    png(&left, 8, 8, |x, y| flat(0.2 + 0.05 * ((x + y) as f64)));
    png(&right, 8, 8, |x, y| flat(0.2 + 0.05 * ((x + y) as f64)));
    let out = temp.path().join("comparison");
    let (result, status) = images::compare(&request(&left, &right), &out).unwrap();
    assert_eq!(status, 0);
    assert_eq!(result["execution_status"], "completed");

    let document = comparison(&out);
    assert_eq!(document["baseline"]["kind"], "images");
    assert_eq!(document["candidate"]["kind"], "images");
    assert!(document.get("mode").is_none());
    assert_eq!(document["evaluation_status"], "not_evaluated");
    assert_eq!(document["baseline"]["declaration"]["color_space"], "sRGB");
    assert_eq!(document["baseline"]["files"][0]["case_id"], "chart");
    assert_eq!(document["baseline"]["files"][0]["width"], 8);

    let case = &document["cases"][0];
    assert_eq!(case["case_id"], "chart");
    assert_eq!(case["status"], "completed");
    assert!(case["review"]["baseline"]["path"].is_string());
    let measurements = case["measurements"].as_array().unwrap();
    assert!(!measurements.is_empty());
    let report = temp.path().join("report");
    spektrafilm_benchmark::report::build_report(&out, &report).unwrap();
    assert!(report.join("index.html").is_file());
    for row in measurements {
        if row["availability"] == "available" {
            let metric = row["metric"].as_str().unwrap();
            if metric.starts_with("numeric.") || metric.starts_with("delta_e00.") {
                assert_eq!(row["value"], json!(0.0), "{metric} on identical images");
            }
        } else {
            assert!(row["reason"].as_str().is_some_and(|r| !r.is_empty()));
        }
    }
}

#[test]
fn a_changed_pixel_produces_a_nonzero_delta() {
    let temp = tempfile::tempdir().unwrap();
    let left = temp.path().join("a.png");
    let right = temp.path().join("b.png");
    png(&left, 8, 8, |_, _| flat(0.25));
    png(&right, 8, 8, |x, y| {
        if x < 4 && y < 4 {
            flat(0.35)
        } else {
            flat(0.25)
        }
    });
    let out = temp.path().join("comparison");
    images::compare(&request(&left, &right), &out).unwrap();
    let document = comparison(&out);
    let delta = document["cases"][0]["measurements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["metric"] == "delta_e00.p95")
        .unwrap();
    assert_eq!(delta["availability"], "available");
    assert!(delta["value"].as_f64().unwrap() > 0.0);
}

#[test]
fn a_dimension_mismatch_fails_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let left = temp.path().join("a.png");
    let right = temp.path().join("b.png");
    png(&left, 8, 8, |_, _| flat(0.5));
    png(&right, 4, 4, |_, _| flat(0.5));
    let out = temp.path().join("comparison");
    let error = images::compare(&request(&left, &right), &out).unwrap_err();
    assert_eq!(error.code, "dimension_mismatch");
    assert!(
        !out.join("comparison.json").exists(),
        "a failed comparison publishes no terminal record"
    );
}

#[test]
fn a_sidecar_beside_a_measured_image_is_not_read() {
    let temp = tempfile::tempdir().unwrap();
    let left = temp.path().join("a.png");
    let right = temp.path().join("b.png");
    png(&left, 8, 8, |_, _| flat(0.4));
    png(&right, 8, 8, |_, _| flat(0.4));
    // A sidecar that would change the interpretation if the lab merged it.
    std::fs::write(
        temp.path().join("a.png.json"),
        br#"{"color_space":"ACES2065-1","layout":{"channels":9}}"#,
    )
    .unwrap();
    let out = temp.path().join("comparison");
    let (_, status) = images::compare(&request(&left, &right), &out).unwrap();
    assert_eq!(status, 0);
    let document = comparison(&out);
    assert_eq!(document["baseline"]["declaration"]["color_space"], "sRGB");
}

#[test]
fn declared_geometry_is_checked_before_measurement() {
    let temp = tempfile::tempdir().unwrap();
    let left = temp.path().join("a.png");
    let right = temp.path().join("b.png");
    png(&left, 8, 8, |_, _| flat(0.5));
    png(&right, 8, 8, |_, _| flat(0.5));

    let mut out_of_bounds = request(&left, &right);
    out_of_bounds.regions[0]["geometry"]["width"] = serde_json::json!(50);
    let out = temp.path().join("out-of-bounds");
    assert_eq!(
        images::compare(&out_of_bounds, &out).unwrap_err().code,
        "region_out_of_bounds"
    );
    assert!(
        !out.join("comparison.json").exists(),
        "a refused geometry publishes no terminal record"
    );

    let mut stage_space = request(&left, &right);
    stage_space.regions[0]["geometry"]["space"] = serde_json::json!("film_frame_pixels");
    let out = temp.path().join("stage-space");
    assert_eq!(
        images::compare(&stage_space, &out).unwrap_err().code,
        "unsupported_coordinate_space"
    );

    let mut reserved = request(&left, &right);
    reserved.regions[0]["id"] = serde_json::json!("full_image");
    let out = temp.path().join("reserved");
    assert_eq!(
        images::compare(&reserved, &out).unwrap_err().code,
        "invalid_region_identity"
    );
}

#[test]
fn the_declaration_decides_whether_samples_are_decoded() {
    let temp = tempfile::tempdir().unwrap();
    let left = temp.path().join("left.png");
    let right = temp.path().join("right.png");
    png(&left, 8, 8, |_, _| flat(0.5));
    png(&right, 8, 8, |_, _| flat(0.75));

    let stored = |value: f64| (value * 65535.0).round() / 65535.0;
    let decoded = |value: f64| {
        let x = stored(value);
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    for (declared_encoded, expected) in [
        (false, (stored(0.5) - stored(0.75)).abs()),
        (true, (decoded(0.5) - decoded(0.75)).abs()),
    ] {
        let mut request = request(&left, &right);
        request.cctf_decoding = declared_encoded;
        let out = temp.path().join(format!("declared-{declared_encoded}"));
        images::compare(&request, &out).unwrap();
        let comparison = comparison(&out);
        let row = comparison["cases"][0]["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["metric"] == "numeric.rmse" && row["region_id"] == "full_image")
            .unwrap();
        let value = row["value"].as_f64().unwrap();
        assert!(
            (value - expected).abs() < 1e-9,
            "encoded={declared_encoded}: measured {value}, expected the {} signal {expected}",
            if declared_encoded {
                "decoded"
            } else {
                "stored"
            }
        );
    }
}
