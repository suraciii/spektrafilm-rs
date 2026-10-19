//! Concise, read-only views of validated retained comparison evidence.
use crate::{
    common::{Error, Result},
    documents::{load_json_document, safe_artifact_path, validate_generated_document},
    report::rows,
};
use serde_json::{Map, Value, json};
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
fn matches(row: &Value, cases: &[String], regions: &[String], metrics: &[String]) -> bool {
    selected(cases, &row["case_id"])
        && selected(regions, &row["region_id"])
        && selected(metrics, &row["metric"])
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
fn compact(row: &Value) -> Value {
    let mut result = Map::new();
    for key in [
        "case_id",
        "seed",
        "seed_population",
        "pair_id",
        "region_id",
        "metric",
        "domain",
        "units",
        "estimator",
        "sample_count",
        "availability",
        "value",
        "reason",
        "artifact_refs",
    ] {
        if let Some(value) = row.get(key) {
            result.insert(key.into(), value.clone());
        }
    }
    Value::Object(result)
}
fn failed(
    case: &Value,
    pair: Option<&Value>,
    findings: &[Value],
    artifacts: &BTreeMap<&str, &Value>,
) -> Value {
    let result = pair.unwrap_or(case);
    let mut value =
        json!({"case_id":case["case_id"],"seed":case["seed"],"status":result["status"]});
    if let Some(pair) = pair {
        value["pair_id"] = pair["id"].clone();
    }
    for key in ["reason", "error", "errors"] {
        if let Some(field) = result.get(key) {
            value[key] = field.clone();
        }
    }
    let seed_root = format!(
        "cases/{}/seed-{}/",
        case["case_id"].as_str().unwrap_or(""),
        case["seed"]
    );
    let pair_root = pair
        .and_then(|p| p["id"].as_str())
        .map(|id| format!("{seed_root}{id}/"));
    let shared_failure = format!("{seed_root}render-failure.json");
    let seed_root = seed_root.as_str();
    let pair_root = pair_root.as_deref();
    let shared_failure = shared_failure.as_str();
    let evidence: BTreeSet<&str> = findings
        .iter()
        .filter(|f| {
            f["case_id"] == case["case_id"]
                && f["metric"] == "execution"
                && pair.is_none_or(|p| f["pair_id"] == p["id"])
                && f.get("seed").is_none_or(|seed| seed == &case["seed"])
        })
        .flat_map(|finding| {
            rows(&finding["evidence"])
                .iter()
                .filter_map(move |reference| {
                    let path = reference.as_str().or_else(|| reference["path"].as_str())?;
                    let artifact = artifacts.get(path)?;
                    if artifact
                        .get("case_id")
                        .is_some_and(|id| id != &case["case_id"])
                        || artifact
                            .get("seed")
                            .is_some_and(|seed| seed != &case["seed"])
                        || pair.is_some_and(|p| {
                            artifact.get("pair_id").is_some_and(|id| id != &p["id"])
                        })
                    {
                        return None;
                    }
                    let explicit = finding.get("seed").is_some()
                        || (artifact["case_id"] == case["case_id"]
                            && artifact["seed"] == case["seed"]);
                    let owned_path = pair_root.map_or_else(
                        || path.starts_with(seed_root),
                        |root| path.starts_with(root) || path == shared_failure,
                    );
                    (explicit || owned_path).then_some(path)
                })
        })
        .collect();
    value["artifact_refs"] = json!(evidence);
    value
}

fn measurement_rows(
    cases: &[Value],
    single: bool,
) -> impl Iterator<Item = (&Value, &Value, Option<&Value>)> {
    cases.iter().flat_map(move |case| {
        rows(&case["measurements"])
            .iter()
            .filter(move |_| !single)
            .map(move |row| (row, case, None))
            .chain(rows(&case["pairs"]).iter().flat_map(move |pair| {
                rows(&pair["measurements"])
                    .iter()
                    .map(move |row| (row, case, Some(pair)))
            }))
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
        for owner in std::iter::once(case).chain(rows(&case["pairs"]).iter()) {
            for row in rows(&owner[key])
                .iter()
                .filter(|r| selected(region_ids, &r["region_id"]))
            {
                let mut row = row.clone();
                row["case_id"] = case["case_id"].clone();
                row["seed"] = case["seed"].clone();
                result.push(row);
            }
        }
    }
    result
}

/// Read and validate a retained comparison, without rendering or measuring.
/// Repeated exact filters are OR within each selector and AND across selectors;
/// unknown choices and empty filtered intersections are errors. Per-seed/pair
/// measurements remain separate from retained cross-seed aggregates. Artifact
/// references are relative to `artifact_base`, the canonical comparison directory.
/// Inspection completes even when the retained comparison's execution failed.
pub fn inspect(
    comparison: &Path,
    case_ids: &[String],
    region_ids: &[String],
    metrics: &[String],
) -> Result<Value> {
    let root = comparison.canonicalize()?;
    let document = safe_artifact_path(&root, "comparison.json")?;
    let comparison = load_json_document(&document)?;
    validate_generated_document("comparison", &comparison, &root)?;
    let cases = rows(&comparison["cases"]);
    let findings = rows(&comparison["findings"]);
    let single = comparison["mode"] == "single_run";
    let aggregates = rows(&comparison["seed_measurements"]);
    let available_cases = choices(cases.iter(), "case_id");
    let all_rows = || {
        measurement_rows(cases, single)
            .map(|(row, _, _)| row)
            .chain(aggregates.iter())
            .chain(findings.iter())
    };
    let mut available_regions = choices(all_rows(), "region_id");
    for case in cases {
        for owner in std::iter::once(case).chain(rows(&case["pairs"]).iter()) {
            for region in rows(&owner["regions"])
                .iter()
                .chain(rows(&owner["automatic_regions"]).iter())
            {
                if let Some(id) = region["id"].as_str() {
                    available_regions.insert(id.into());
                }
            }
        }
    }
    let available_metrics = choices(all_rows(), "metric");
    validate_selection("case", case_ids, &available_cases)?;
    validate_selection("region", region_ids, &available_regions)?;
    validate_selection("metric", metrics, &available_metrics)?;
    let selected_measurements: Vec<Value> = measurement_rows(cases, single)
        .filter(|(row, _, _)| matches(row, case_ids, region_ids, metrics))
        .map(|(row, case, pair)| {
            let mut row = compact(row);
            row["seed"] = case["seed"].clone();
            if let Some(pair) = pair {
                row["pair_id"] = pair["id"].clone();
            }
            row
        })
        .collect();
    let seed_measurements: Vec<Value> = aggregates
        .iter()
        .filter(|row| matches(row, case_ids, region_ids, metrics))
        .map(compact)
        .collect();
    let selected_findings: Vec<&Value> = findings
        .iter()
        .filter(|row| matches(row, case_ids, region_ids, metrics))
        .collect();
    // Metric selectors apply to scalar measurements, not to the independent
    // retained coordinate and native-stage records for the selected regions.
    let region_readouts = retained_readouts(cases, "region_readouts", case_ids, region_ids);
    let stage_readouts = retained_readouts(cases, "stage_readouts", case_ids, region_ids);
    let readout_status: Vec<Value> = cases.iter().filter(|c| selected(case_ids, &c["case_id"]))
        .flat_map(|case| {
            let owners: Vec<&Value> = if single { rows(&case["pairs"]).iter().collect() } else { vec![case] };
            owners.into_iter().map(move |owner| json!({
                "case_id": case["case_id"], "seed": case["seed"],
                "pair_id": owner.get("id").cloned().unwrap_or(json!("baseline/candidate")),
                "region_readouts": if owner.get("region_readouts").is_some() { "retained" } else { "not_retained" },
                "stage_readouts": if case.get("stage_readouts").is_some() { "retained" } else { "not_retained" },
            }))
        }).collect();
    if (!region_ids.is_empty() || !metrics.is_empty())
        && selected_measurements.is_empty()
        && seed_measurements.is_empty()
        && selected_findings.is_empty()
        && (!metrics.is_empty() || (region_readouts.is_empty() && stage_readouts.is_empty()))
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
    // Retained run conditions and stage context, straight from the validated
    // run evidence: nothing here is recomputed, and unobserved stays named.
    let mut case_conditions = Vec::new();
    for case in cases.iter().filter(|c| selected(case_ids, &c["case_id"])) {
        let mut roles = Map::new();
        for (role, payload) in case["run_evidence"].as_object().into_iter().flatten() {
            let stage = &payload["stage_evidence"];
            let links = |record: &Value| record["path"].clone();
            roles.insert(
                role.clone(),
                json!({
                    "run": payload["run"],
                    "target": payload["target"],
                    "input": payload["input"],
                    "preparation": payload.get("preparation").cloned().unwrap_or(Value::Null),
                    "conditions": {
                        "command": payload["command"],
                        "source_command": payload["source_command"],
                        "environment": payload["environment"],
                        "precision": payload["precision"],
                        "execution": payload["execution"],
                        "data_inventory": payload["data_inventory"],
                    },
                    "declared": payload["declared"],
                    "observed": payload["observed"],
                    "stage_evidence": {
                        "requested": stage["requested"],
                        "availability": stage["availability"],
                        "reason": stage.get("reason").cloned().unwrap_or(Value::Null),
                        "manifest": links(&stage["manifest"]),
                        "buffers": stage["buffers"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(links)
                            .collect::<Vec<_>>(),
                    },
                    "diagnostic_evidence": {
                        "diagnostics": links(&payload["diagnostics"]),
                        "resolved_configuration": links(&payload["resolved_configuration"]),
                    },
                }),
            );
        }
        case_conditions.push(json!({"case_id":case["case_id"],"seed":case["seed"],"roles":roles}));
    }
    let mut failed_results = Vec::new();
    for case in cases.iter().filter(|c| selected(case_ids, &c["case_id"])) {
        let pairs = rows(&case["pairs"]);
        if pairs.is_empty() && case["status"] != "completed" {
            failed_results.push(failed(case, None, findings, &artifact_index));
        }
        for pair in pairs.iter().filter(|p| p["status"] != "completed") {
            failed_results.push(failed(case, Some(pair), findings, &artifact_index));
        }
    }
    Ok(
        json!({"schema_version":1,"command":"inspect","execution_status":"completed",
        "comparison_id":comparison["id"],"comparison_execution_status":comparison["execution_status"],
        "evaluation_status":comparison["evaluation_status"],"condition_differences":comparison["condition_differences"],
        "summary":comparison["summary"],"artifact_base":root,
        "available":{"case_ids":available_cases,"region_ids":available_regions,"metrics":available_metrics},
        "case_conditions":case_conditions,
        "region_readouts":region_readouts,"stage_readouts":stage_readouts,"readout_status":readout_status,
        "failed_results":failed_results,"findings":selected_findings,
        "measurements":selected_measurements,"seed_measurements":seed_measurements}),
    )
}
