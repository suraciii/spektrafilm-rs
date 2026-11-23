//! Concise, read-only views of validated retained comparison evidence.
use crate::{
    common::{Error, Result},
    documents::{load_json_document, safe_artifact_path, validate_generated_document},
    report::rows,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

fn selected(filters: &[String], value: &Value) -> bool {
    filters.is_empty()
        || value
            .as_str()
            .is_some_and(|v| filters.iter().any(|f| f == v))
}
fn matches(
    row: &Value,
    cases: &[String],
    regions: &[String],
    metrics: &[String],
    sides: &[String],
) -> bool {
    selected(cases, &row["case_id"])
        && selected(regions, &row["region_id"])
        && selected(metrics, &row["metric"])
        && selected(sides, &row["side"])
}
fn choices<'a>(values: impl Iterator<Item = &'a Value>, field: &str) -> BTreeSet<String> {
    values
        .filter_map(|v| v[field].as_str().map(str::to_owned))
        .collect()
}
fn validate_selection(kind: &str, filters: &[String], available: &BTreeSet<String>) -> Result<()> {
    for filter in filters {
        if !available.contains(filter) {
            return Err(Error::new(
                "unknown_selection",
                format!(
                    "unknown {kind} {filter:?}; available {kind} choices: {}",
                    available.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            ));
        }
    }
    Ok(())
}
fn failed(case: &Value, findings: &[Value], artifacts: &BTreeMap<&str, &Value>) -> Value {
    let mut value = json!({"case_id":case["case_id"],"status":case["status"]});
    for key in ["reason", "error", "errors"] {
        if let Some(field) = case.get(key) {
            value[key] = field.clone();
        }
    }
    // A failed case names only evidence this comparison retains: a finding
    // cannot point at a file the reader does not have.
    let evidence: BTreeSet<&str> = findings
        .iter()
        .filter(|finding| finding["case_id"] == case["case_id"] && finding["metric"] == "execution")
        .flat_map(|finding| {
            rows(&finding["evidence"]).iter().filter_map(|reference| {
                let path = reference.as_str().or_else(|| reference["path"].as_str())?;
                artifacts
                    .get(path)
                    .filter(|artifact| {
                        artifact
                            .get("case_id")
                            .is_none_or(|id| id == &case["case_id"])
                    })
                    .map(|_| path)
            })
        })
        .collect();
    value["artifact_refs"] = json!(evidence);
    value
}

fn measurement_rows(cases: &[Value]) -> impl Iterator<Item = (&Value, &Value)> {
    cases.iter().flat_map(move |case| {
        rows(&case["measurements"])
            .iter()
            .map(move |row| (row, case))
    })
}

fn retained_readouts(
    cases: &[Value],
    key: &str,
    case_ids: &[String],
    region_ids: &[String],
) -> Vec<Value> {
    let mut result = Vec::new();
    for case in cases.iter().filter(|c| selected(case_ids, &c["case_id"])) {
        for row in rows(&case[key])
            .iter()
            .filter(|r| selected(region_ids, &r["region_id"]))
        {
            let mut row = row.clone();
            row["case_id"] = case["case_id"].clone();
            result.push(row);
        }
    }
    result
}

/// Read and validate a retained comparison, without rendering or measuring.
/// Repeated exact filters are OR within each selector and AND across selectors;
/// unknown choices and empty filtered intersections are errors. Artifact
/// references are relative to `artifact_base`, the canonical comparison directory.
/// Inspection completes even when the retained comparison's execution failed.
pub fn inspect(
    comparison: &Path,
    case_ids: &[String],
    region_ids: &[String],
    metrics: &[String],
    sides: &[String],
) -> Result<Value> {
    let root = comparison.canonicalize()?;
    let document = safe_artifact_path(&root, "comparison.json")?;
    let comparison = load_json_document(&document)?;
    validate_generated_document("comparison", &comparison, &root)?;
    let cases = rows(&comparison["cases"]);
    let findings = rows(&comparison["findings"]);
    let available_cases = choices(cases.iter(), "case_id");
    let all_rows = || {
        measurement_rows(cases)
            .map(|(row, _)| row)
            .chain(findings.iter())
    };
    let mut available_regions = choices(all_rows(), "region_id");
    for case in cases {
        for region in rows(&case["regions"])
            .iter()
            .chain(rows(&case["automatic_regions"]).iter())
        {
            if let Some(id) = region["id"].as_str() {
                available_regions.insert(id.into());
            }
        }
    }
    let available_metrics = choices(all_rows(), "metric");
    let available_sides = choices(all_rows(), "side");
    validate_selection("case", case_ids, &available_cases)?;
    validate_selection("region", region_ids, &available_regions)?;
    validate_selection("metric", metrics, &available_metrics)?;
    validate_selection("side", sides, &available_sides)?;
    let selected_measurements: Vec<Value> = measurement_rows(cases)
        .filter(|(row, _)| matches(row, case_ids, region_ids, metrics, sides))
        .map(|(row, _)| row.clone())
        .collect();
    let selected_findings: Vec<&Value> = findings
        .iter()
        .filter(|row| matches(row, case_ids, region_ids, metrics, sides))
        .collect();
    // Metric selectors apply to scalar measurements, not to the independent
    // retained coordinate and native-stage records for the selected regions.
    let region_readouts = retained_readouts(cases, "region_readouts", case_ids, region_ids);
    let readout_status: Vec<Value> = cases
        .iter()
        .filter(|c| selected(case_ids, &c["case_id"]))
        .map(|case| {
            json!({
                "case_id": case["case_id"],
                "region_readouts": if case.get("region_readouts").is_some() { "retained" } else { "not_retained" },
            })
        })
        .collect();
    if (!region_ids.is_empty() || !metrics.is_empty())
        && selected_measurements.is_empty()
        && selected_findings.is_empty()
        && (!metrics.is_empty() || region_readouts.is_empty())
    {
        return Err(Error::new(
            "empty_selection",
            format!(
                "no retained measurements or findings match the combined filters; available regions: {}; available metrics: {}",
                available_regions
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
                available_metrics
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    let artifact_index: BTreeMap<&str, &Value> = rows(&comparison["artifacts"])
        .iter()
        .filter_map(|artifact| artifact["path"].as_str().map(|path| (path, artifact)))
        .collect();
    let mut failed_results = Vec::new();
    for case in cases.iter().filter(|c| selected(case_ids, &c["case_id"])) {
        if case["status"] != "completed" {
            failed_results.push(failed(case, findings, &artifact_index));
        }
    }
    Ok(
        json!({"schema_version":1,"command":"inspect","execution_status":"completed",
        "comparison_id":comparison["id"],"comparison_execution_status":comparison["execution_status"],
        "evaluation_status":comparison.get("evaluation_status").cloned().unwrap_or(json!("not_evaluated")),"condition_differences":comparison["condition_differences"],
        "summary":comparison["summary"],"artifact_base":root,
        "available":{"case_ids":available_cases,"region_ids":available_regions,"metrics":available_metrics},
        "region_readouts":region_readouts,"readout_status":readout_status,
        "failed_results":failed_results,"findings":selected_findings,
        "measurements":selected_measurements}),
    )
}
