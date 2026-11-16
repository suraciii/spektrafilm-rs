//! `spektrafilm-bench inspect` and `report comparison` over a retained
//! comparison, through the real binary.
//!
//! The comparison is built through the library the lab's `compare` also calls,
//! so the two packages never need each other's executable.

use serde_json::Value;
use spektrafilm_benchmark::images;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bench() -> Command {
    Command::new(env!("CARGO_BIN_EXE_spektrafilm-bench"))
}

fn png(path: &Path, sample: impl Fn(u32, u32) -> [u8; 3]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut image = image::RgbImage::new(16, 16);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = image::Rgb(sample(x, y));
    }
    image.save(path).unwrap();
}

/// One comparison of two 16x16 charts, built the way the lab's `compare` builds
/// it: a declaration, an empty region set, and the built-in analysis.
fn comparison(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let baseline = root.join("a.png");
    let candidate = root.join("b.png");
    png(&baseline, |x, y| [40 + x as u8 * 4, 60 + y as u8 * 4, 90]);
    png(&candidate, |x, y| {
        if (x + y) % 4 == 0 {
            [90, 60 + y as u8 * 4, 90]
        } else {
            [40 + x as u8 * 4, 60 + y as u8 * 4, 90]
        }
    });
    let out = root.join("comparison");
    let request = images::Request {
        baseline_id: "a".to_owned(),
        candidate_id: "b".to_owned(),
        color_space: "sRGB".to_owned(),
        cctf_decoding: true,
        pairs: vec![images::Pair {
            case_id: "chart".to_owned(),
            baseline: baseline.clone(),
            candidate: candidate.clone(),
        }],
        regions: Vec::new(),
        protocol: spektrafilm_benchmark::analysis::protocol(
            spektrafilm_benchmark::analysis::profile("lab-appearance-v1").unwrap(),
        ),
        map_scale: 1,
    };
    images::compare(&request, &out).unwrap();
    (out, baseline, candidate)
}

#[test]
fn inspect_reads_the_comparison_without_reading_an_image() {
    let temp = tempfile::tempdir().unwrap();
    let (out, baseline, candidate) = comparison(temp.path());
    // The originals are gone; the record still answers.
    std::fs::remove_file(&baseline).unwrap();
    std::fs::remove_file(&candidate).unwrap();
    let output = bench()
        .args(["inspect", "--comparison"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inspected: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inspected["command"], "inspect");
    assert_eq!(inspected["execution_status"], "completed");
    assert!(!inspected["measurements"].as_array().unwrap().is_empty());
}

/// A reader selects a quantity and the source it stands for, not a spelling of
/// the source inside the quantity's name.
#[test]
fn inspect_selects_one_side_of_one_quantity() {
    let temp = tempfile::tempdir().unwrap();
    let (out, _, _) = comparison(temp.path());
    let output = bench()
        .args(["inspect", "--comparison"])
        .arg(&out)
        .args(["--metric", "tone.l_star.q05", "--side", "baseline"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inspected: Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = inspected["measurements"].as_array().unwrap();
    assert!(
        !rows.is_empty(),
        "the quantity was not found by its own name"
    );
    assert!(
        rows.iter()
            .all(|row| row["metric"] == "tone.l_star.q05" && row["side"] == "baseline"),
        "{inspected}"
    );
    // A side the record never measured is outside its vocabulary.
    let unknown = bench()
        .args(["inspect", "--comparison"])
        .arg(&out)
        .args(["--side", "nonsense"])
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(1));
    let refusal: Value = serde_json::from_slice(&unknown.stdout).unwrap();
    assert_eq!(refusal["errors"][0]["code"], "unknown_selection");
    // A recorded side on a quantity that exists once is an empty intersection,
    // not a silent guess at the only row.
    let empty = bench()
        .args(["inspect", "--comparison"])
        .arg(&out)
        .args(["--metric", "numeric.rmse", "--side", "delta"])
        .output()
        .unwrap();
    assert_eq!(empty.status.code(), Some(1));
    let refusal: Value = serde_json::from_slice(&empty.stdout).unwrap();
    assert_eq!(refusal["errors"][0]["code"], "empty_selection");
    let published = std::fs::read(out.join("comparison.json")).unwrap();
    assert!(String::from_utf8_lossy(&published).contains("tone.l_star.q05"));
}

#[test]
fn a_report_failure_leaves_no_report_and_keeps_the_comparison() {
    let temp = tempfile::tempdir().unwrap();
    let (out, _, _) = comparison(temp.path());
    // A comparison whose retained artifact is gone cannot be reported. The
    // refusal happens before the report destination exists, so a failed report
    // leaves no report behind and changes no byte of the comparison.
    let before = std::fs::read(out.join("comparison.json")).unwrap();
    std::fs::remove_file(out.join("cases/chart/baseline-source.png")).unwrap();
    let report = temp.path().join("report");
    let refused = bench()
        .args(["report", "comparison", "--comparison"])
        .arg(&out)
        .arg("--out")
        .arg(&report)
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert_eq!(result["command"], "report comparison");
    assert_eq!(result["errors"][0]["code"], "missing_artifact");
    assert!(!report.exists(), "a refused report creates no destination");
    assert_eq!(
        std::fs::read(out.join("comparison.json")).unwrap(),
        before,
        "a refused report does not rewrite the comparison"
    );
}
