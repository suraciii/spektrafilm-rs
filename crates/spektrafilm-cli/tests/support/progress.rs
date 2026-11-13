//! The progress stream a run or comparison prints while it works.
use serde_json::Value;

/// Assert the stream starts and finishes every case and seed once, in manifest
/// order, and closes with one summary that counts them.
pub fn assert_case_seed_progress(stderr: &[u8], operation: &str, manifest: &Value) {
    let stderr = std::str::from_utf8(stderr).unwrap();
    let expected: Vec<_> = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            (
                format!(
                    "{} seed {}",
                    case["case_id"].as_str().unwrap(),
                    case["seed"]
                ),
                case["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let started: Vec<_> = stderr
        .lines()
        .filter_map(|line| line.strip_prefix(&format!("Starting {operation} ")))
        .map(str::to_owned)
        .collect();
    let finished: Vec<_> = stderr
        .lines()
        .filter_map(|line| line.strip_prefix(&format!("Completed {operation} ")))
        .map(|line| {
            let (item, detail) = line.split_once(": status=").unwrap();
            let (status, _) = detail
                .split_once(" elapsed_seconds=")
                .unwrap_or((detail, ""));
            (item.to_owned(), status.to_owned())
        })
        .collect();
    assert_eq!(
        started,
        expected
            .iter()
            .map(|(item, _)| item.clone())
            .collect::<Vec<_>>(),
        "{stderr}"
    );
    assert_eq!(finished, expected, "{stderr}");
    let completed = expected
        .iter()
        .filter(|(_, status)| matches!(status.as_str(), "completed" | "validated"))
        .count();
    let summaries: Vec<_> = stderr
        .lines()
        .filter_map(|line| line.strip_prefix(&format!("{operation} summary: ")))
        .collect();
    assert_eq!(summaries.len(), 1, "{stderr}");
    assert!(
        summaries[0].starts_with(&format!(
            "completed={completed} failed={} ",
            expected.len() - completed
        )),
        "{stderr}"
    );
}
