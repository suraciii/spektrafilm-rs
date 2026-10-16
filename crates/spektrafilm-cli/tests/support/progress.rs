use serde_json::Value;

pub fn assert_case_seed_progress(stderr: &[u8], operation: &str, manifest: &Value) {
    let stderr = std::str::from_utf8(stderr).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    let mut started = Vec::new();
    let mut finished = Vec::new();
    let mut summaries = Vec::new();
    for line in stderr.lines() {
        if let Some(item) = line.strip_prefix(&format!("Starting {operation} ")) {
            started.push(item.to_owned());
        }
        if let Some(item) = line.strip_prefix(&format!("Completed {operation} ")) {
            let (item, detail) = item.split_once(": status=").unwrap();
            let (status, elapsed) = detail.split_once(" elapsed_seconds=").unwrap();
            let elapsed = elapsed.parse::<f64>().unwrap();
            assert!(elapsed.is_finite() && elapsed >= 0.0, "{line}");
            finished.push((item.to_owned(), status.to_owned()));
        }
        if let Some(summary) = line.strip_prefix(&format!("{operation} summary: ")) {
            summaries.push(summary);
        }
    }
    let expected: Vec<_> = cases
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
    assert_eq!(
        started,
        expected
            .iter()
            .map(|(item, _)| item.clone())
            .collect::<Vec<_>>(),
        "{stderr}"
    );
    assert_eq!(finished, expected, "{stderr}");
    assert_eq!(summaries.len(), 1, "{stderr}");
    let completed = cases
        .iter()
        .filter(|case| case["status"] == "completed" || case["status"] == "validated")
        .count();
    let failed = cases.len() - completed;
    let fields: Vec<_> = summaries[0].split_whitespace().collect();
    assert_eq!(fields[0], format!("completed={completed}"));
    assert_eq!(fields[1], format!("failed={failed}"));
    let elapsed = fields[2]
        .strip_prefix("elapsed_seconds=")
        .unwrap()
        .parse::<f64>()
        .unwrap();
    assert!(elapsed.is_finite() && elapsed >= 0.0);
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.starts_with(&format!("{operation} summary:")))
            .count(),
        1
    );
}
