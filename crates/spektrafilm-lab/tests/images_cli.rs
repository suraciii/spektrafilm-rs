//! `spektrafilm-lab compare` through the real binary.
//!
//! The product case: two renderer builds wrote their rasters elsewhere, and the
//! lab quantifies and reports them without a suite, a target, or a renderer.

use serde_json::json;
use std::path::Path;
use std::process::Command;

fn lab() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-lab"))
}

fn png(path: &Path, width: u32, height: u32, sample: impl Fn(u32, u32) -> [u8; 3]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut image = image::RgbImage::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = image::Rgb(sample(x, y));
    }
    image.save(path).unwrap();
}

fn chart(path: &Path) {
    png(path, 16, 16, |x, y| {
        [40 + x as u8 * 4, 60 + y as u8 * 4, 90]
    });
}

fn chart_changed(path: &Path) {
    png(path, 16, 16, |x, y| {
        if (x + y) % 4 == 0 {
            [90, 60 + y as u8 * 4, 90]
        } else {
            [40 + x as u8 * 4, 60 + y as u8 * 4, 90]
        }
    });
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn a_single_pair_publishes_a_comparison_and_a_report() {
    let temp = tempfile::tempdir().unwrap();
    let baseline = temp.path().join("v1/portrait.png");
    let candidate = temp.path().join("v2/portrait.png");
    chart(&baseline);
    chart_changed(&candidate);
    let out = temp.path().join("comparison");
    let output = lab()
        .args(["compare", "--baseline"])
        .arg(&baseline)
        .arg("--candidate")
        .arg(&candidate)
        .args([
            "--declaration",
            "sRGB:encoded",
            "--roi",
            "skin=2,2,6,6",
            "--analysis",
            "lab-color-tone-v1",
            "--report",
            "--out",
        ])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["command"], "compare");
    assert_eq!(result["execution_status"], "completed");
    assert_eq!(result["cases"], 1);
    assert!(out.join("report/index.html").is_file());

    let comparison = read_json(&out.join("comparison.json"));
    assert_eq!(comparison["baseline"]["kind"], "images");
    assert!(comparison.get("mode").is_none());
    // One delivered image set is one realisation: the record states no seed
    // population and no judgement.
    assert!(comparison.get("evaluation_status").is_none());
    assert!(comparison.get("findings").is_none());
    assert!(comparison["cases"][0].get("seed").is_none());
    assert!(
        comparison["cases"][0]["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r.get("seed_population").is_none())
    );
    let regions = read_json(&out.join("regions.json"));
    assert_eq!(regions["regions"][0]["id"], "skin");
    assert_eq!(regions["regions"][0]["geometry"]["space"], "render");
}

#[test]
fn a_directory_pair_compares_every_name() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["a.png", "b.png"] {
        chart(&temp.path().join("v1").join(name));
        chart_changed(&temp.path().join("v2").join(name));
    }
    let out = temp.path().join("comparison");
    let output = lab()
        .args(["compare", "--baseline"])
        .arg(temp.path().join("v1"))
        .arg("--candidate")
        .arg(temp.path().join("v2"))
        .args(["--declaration", "sRGB:encoded", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["cases"], 2);
    let comparison = read_json(&out.join("comparison.json"));
    assert_eq!(comparison["cases"].as_array().unwrap().len(), 2);
    assert!(!out.join("report").exists(), "no report unless requested");
}

#[test]
fn an_unpaired_name_fails_and_names_both_sides() {
    let temp = tempfile::tempdir().unwrap();
    chart(&temp.path().join("v1/a.png"));
    chart(&temp.path().join("v2/b.png"));
    let out = temp.path().join("comparison");
    let output = lab()
        .args(["compare", "--baseline"])
        .arg(temp.path().join("v1"))
        .arg("--candidate")
        .arg(temp.path().join("v2"))
        .args(["--declaration", "sRGB:encoded", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["errors"][0]["code"], "incompatible_image_sets");
    let message = result["errors"][0]["message"].as_str().unwrap();
    assert!(
        message.contains("a.png") && message.contains("b.png"),
        "{message}"
    );
    assert!(!out.join("comparison.json").exists());
}

#[test]
fn an_unknown_analysis_profile_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let baseline = temp.path().join("a.png");
    let candidate = temp.path().join("b.png");
    chart(&baseline);
    chart(&candidate);
    let output = lab()
        .args(["compare", "--baseline"])
        .arg(&baseline)
        .arg("--candidate")
        .arg(&candidate)
        .args([
            "--declaration",
            "sRGB:encoded",
            "--analysis",
            "nope-v1",
            "--out",
        ])
        .arg(temp.path().join("comparison"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["execution_status"], "failed");
    assert_eq!(result["errors"][0]["code"], "unknown_analysis");
    let message = result["errors"][0]["message"].as_str().unwrap();
    assert!(message.contains("lab-appearance-v1"), "{message}");
}

#[test]
fn an_authored_regions_document_must_declare_render_geometry() {
    let temp = tempfile::tempdir().unwrap();
    let baseline = temp.path().join("a.png");
    let candidate = temp.path().join("b.png");
    chart(&baseline);
    chart_changed(&candidate);
    let regions = temp.path().join("regions.json");
    let document = |space: &str| {
        json!({
            "schema_version": 1,
            "id": "authored",
            "regions": [{
                "id": "patch",
                "role": "other",
                "measurements": [],
                "geometry": {"x": 1, "y": 1, "width": 4, "height": 4, "space": space}
            }]
        })
    };
    let run = |space: &str, out: &str| {
        std::fs::write(&regions, serde_json::to_vec(&document(space)).unwrap()).unwrap();
        lab()
            .args(["compare", "--baseline"])
            .arg(&baseline)
            .arg("--candidate")
            .arg(&candidate)
            .args(["--declaration", "sRGB:encoded", "--regions"])
            .arg(&regions)
            .arg("--out")
            .arg(temp.path().join(out))
            .output()
            .unwrap()
    };

    let refused = run("source", "refused");
    assert_eq!(refused.status.code(), Some(1));
    let result: serde_json::Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(result["errors"][0]["code"], "unsupported_coordinate_space");
    assert!(!temp.path().join("refused/comparison.json").exists());

    let accepted = run("render", "accepted");
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let retained: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("accepted/regions.json")).unwrap())
            .unwrap();
    assert_eq!(retained["regions"][0]["id"], "patch");
}
