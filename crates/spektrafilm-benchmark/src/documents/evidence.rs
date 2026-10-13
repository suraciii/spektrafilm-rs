use super::{
    err, finite, identifier, load_json_document, obj, req, safe_artifact_path, text, unique,
    validate_schema,
};
use crate::common::{Result, digest, sha256_file};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    Unavailable,
    NotApplicable,
}
#[derive(Debug, Deserialize)]
pub struct MeasurementRecord {
    pub case_id: String,
    pub region_id: String,
    pub metric: String,
    pub domain: String,
    pub units: String,
    pub estimator: String,
    pub sample_count: u64,
    pub availability: Availability,
    pub seed_population: Vec<u64>,
    pub artifact_refs: Vec<Value>,
    pub value: Option<f64>,
    pub reason: Option<String>,
}
pub fn validate_generated_document(kind: &str, v: &Value, root: &Path) -> Result<()> {
    finite(v)?;
    validate_schema(kind, v)?;
    let o = obj(v)?;
    let body = Value::Object(
        o.iter()
            .filter(|(k, _)| k.as_str() != "id")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    );
    if text(o, "id")? != digest(&body)? {
        return Err(err(
            "manifest_identity",
            "manifest identity does not match contents",
        ));
    }
    let mut index = std::collections::BTreeMap::new();
    for a in v["artifacts"].as_array().unwrap() {
        let ao = obj(a)?;
        let p = text(ao, "path")?;
        if index.insert(p, a).is_some() {
            return Err(err(
                "duplicate_artifact",
                "artifact index paths must be unique",
            ));
        }
        let file = safe_artifact_path(root, p)?;
        if !file.is_file() {
            return Err(err("missing_artifact", "artifact must be a regular file"));
        }
        if sha256_file(&file)? != text(ao, "sha256")? {
            return Err(err("hash_mismatch", "artifact hash mismatch"));
        }
    }
    let reference = |item: &Value| -> Result<()> {
        let p = item
            .as_str()
            .or_else(|| item.get("path").and_then(Value::as_str))
            .ok_or_else(|| {
                err(
                    "unindexed_artifact",
                    "artifact reference must resolve in index",
                )
            })?;
        let a = index.get(p).ok_or_else(|| {
            err(
                "unindexed_artifact",
                format!("artifact reference absent: {p}"),
            )
        })?;
        if item.is_object() && item.get("sha256") != a.get("sha256") {
            return Err(err(
                "artifact_identity",
                "artifact reference hash differs from index",
            ));
        }
        Ok(())
    };
    let measurements = |rows: &Value| -> Result<()> {
        for row in rows.as_array().into_iter().flatten() {
            let r = obj(row)?;
            if [
                "case_id",
                "region_id",
                "metric",
                "domain",
                "units",
                "estimator",
                "sample_count",
                "availability",
                "seed_population",
                "artifact_refs",
            ]
            .iter()
            .any(|k| !r.contains_key(*k))
                || row["sample_count"].as_u64().is_none()
            {
                return Err(err(
                    "measurement_fields",
                    "measurement requires identity, population and estimator",
                ));
            }
            if !matches!(
                row["availability"].as_str(),
                Some("available" | "unavailable" | "not_applicable")
            ) {
                return Err(err(
                    "measurement_availability",
                    "invalid measurement availability",
                ));
            }
            let parsed: MeasurementRecord = serde_json::from_value(row.clone())
                .map_err(|e| err("measurement_fields", e.to_string()))?;
            match parsed.availability {
                Availability::Available => {
                    if parsed.value.is_none_or(|x| !x.is_finite()) || row.get("reason").is_some() {
                        return Err(err(
                            "measurement_value",
                            "available measurement requires one finite value",
                        ));
                    }
                }
                Availability::Unavailable | Availability::NotApplicable => {
                    if row.get("value").is_some()
                        || parsed.reason.as_deref().is_none_or(str::is_empty)
                    {
                        return Err(err(
                            "measurement_availability",
                            "unavailable measurement requires reason and no value",
                        ));
                    }
                }
            }
            for a in &parsed.artifact_refs {
                reference(a)?
            }
        }
        Ok(())
    };
    if kind == "run" {
        reference(&v["suite"])?;
        reference(&v["target"])?;
        let suite = load_json_document(&safe_artifact_path(
            root,
            v["suite"]["path"].as_str().unwrap(),
        )?)?;
        validate_schema("suite", &suite)?;
        if digest(&suite)? != v["suite"]["digest"].as_str().unwrap_or("")
            || suite["protocol"] != v["protocol"]
        {
            return Err(err(
                "suite_identity",
                "suite snapshot and protocol mismatch",
            ));
        }
        let selected = v["selection"]["case_ids"]
            .as_array()
            .ok_or_else(|| err("run_selection", "selection case_ids must be array"))?;
        let all = suite["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].clone())
            .collect::<Vec<_>>();
        let set = selected.iter().map(|x| x.as_str()).collect::<BTreeSet<_>>();
        if selected.is_empty()
            || set.len() != selected.len()
            || selected.iter().any(|x| !all.contains(x))
            || v["selection"]["full_suite"].as_bool() != Some(selected == &all)
        {
            return Err(err(
                "run_selection",
                "selection must identify distinct suite cases and full-suite state",
            ));
        }
        let mut expected = BTreeSet::new();
        for c in suite["cases"].as_array().unwrap() {
            if selected.contains(&c["id"]) {
                for seed in c["seeds"].as_array().unwrap() {
                    expected.insert((
                        c["id"].as_str().unwrap().to_string(),
                        seed.as_u64().unwrap(),
                    ));
                }
            }
        }
        let mut observed = BTreeSet::new();
        for row in v["cases"].as_array().unwrap() {
            let ro = obj(row)?;
            identifier(ro, "case_id")?;
            let seed = row["seed"]
                .as_u64()
                .filter(|x| *x <= u32::MAX as u64)
                .ok_or_else(|| err("run_case", "invalid case seed"))?;
            if !observed.insert((text(ro, "case_id")?.to_string(), seed)) {
                return Err(err("run_selection", "duplicate rendered case/seed"));
            }
            match row["status"].as_str() {
                Some("completed") => {
                    for k in [
                        "input",
                        "raw",
                        "raw_metadata",
                        "master",
                        "diagnostics",
                        "resolved",
                        "stdout",
                        "stderr",
                    ] {
                        let record = obj(req(ro, k)?)?;
                        text(record, "path")?;
                        reference(req(ro, k)?)?
                    }
                    if row.get("source_raw").is_some() || row.get("source_raw_metadata").is_some() {
                        for key in [
                            "source_raw",
                            "source_raw_metadata",
                            "source_resolved",
                            "source_diagnostics",
                            "source_master",
                        ] {
                            let record = obj(req(ro, key)?)?;
                            text(record, "path")?;
                            reference(req(ro, key)?)?;
                        }
                        let source = &row["source_raw"];
                        let layout = &source["layout"];
                        let width = layout["width"]
                            .as_u64()
                            .filter(|v| *v > 0)
                            .ok_or_else(|| err("raw_layout", "Invalid source width"))?;
                        let height = layout["height"]
                            .as_u64()
                            .filter(|v| *v > 0)
                            .ok_or_else(|| err("raw_layout", "Invalid source height"))?;
                        let bytes = width
                            .checked_mul(height)
                            .and_then(|v| v.checked_mul(24))
                            .ok_or_else(|| err("raw_layout", "Source dimensions overflow"))?;
                        if source["color"]
                            != json!({"color_space":"ACES2065-1","transfer":"linear"})
                            || layout["channels"] != 3
                            || layout["dtype"] != "f64"
                            || layout["channel_order"] != "RGB"
                            || fs::metadata(safe_artifact_path(
                                root,
                                source["path"].as_str().unwrap(),
                            )?)?
                            .len()
                                != bytes
                        {
                            return Err(err(
                                "raw_layout",
                                "Retained source layout, color, or byte count is invalid",
                            ));
                        }
                        let metadata = load_json_document(&safe_artifact_path(
                            root,
                            row["source_raw_metadata"]["path"].as_str().unwrap(),
                        )?)?;
                        let mut expected = layout.clone();
                        expected
                            .as_object_mut()
                            .unwrap()
                            .extend(source["color"].as_object().unwrap().clone());
                        let diagnostics = load_json_document(&safe_artifact_path(
                            root,
                            row["source_diagnostics"]["path"].as_str().unwrap(),
                        )?)?;
                        if metadata != expected
                            || diagnostics["configuration"]["input_dimensions"]
                                != row["source_dimensions"]
                            || diagnostics["configuration"]["published_dimensions"]
                                != json!([width, height])
                            || row["source_geometry"]["published_dimensions"]
                                != json!([width, height])
                            || row["simulation_geometry"]["published_dimensions"]
                                != row["dimensions"]
                        {
                            return Err(err(
                                "source_geometry",
                                "Source evidence metadata and observed geometry disagree",
                            ));
                        }
                        let case = suite["cases"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|c| c["id"] == row["case_id"])
                            .ok_or_else(|| err("run_selection", "Unknown retained case"))?;
                        if case.get("preparation") != row.get("preparation") {
                            return Err(err(
                                "preparation_identity",
                                "Run preparation differs from suite provenance",
                            ));
                        }
                    }
                    let layout = &row["raw"]["layout"];
                    let w = layout["width"]
                        .as_u64()
                        .ok_or_else(|| err("raw_layout", "invalid width"))?;
                    let h = layout["height"]
                        .as_u64()
                        .ok_or_else(|| err("raw_layout", "invalid height"))?;
                    let bytes = w
                        .checked_mul(h)
                        .and_then(|x| x.checked_mul(24))
                        .ok_or_else(|| err("raw_layout", "raw byte count overflow"))?;
                    if fs::metadata(safe_artifact_path(
                        root,
                        row["raw"]["path"].as_str().unwrap(),
                    )?)?
                    .len()
                        != bytes
                    {
                        return Err(err("raw_layout", "raw byte count differs from dimensions"));
                    }
                    let d = load_json_document(&safe_artifact_path(
                        root,
                        row["diagnostics"]["path"].as_str().unwrap(),
                    )?)?;
                    if d["configuration"]["published_dimensions"] != row["dimensions"] {
                        return Err(err(
                            "render_geometry",
                            "diagnostics and retained dimensions disagree",
                        ));
                    }
                }
                Some("failed") => {
                    if row
                        .get("error")
                        .is_none_or(|x| !x.is_object() || x.as_object().unwrap().is_empty())
                    {
                        return Err(err(
                            "missing_failure",
                            "failed render requires structured error",
                        ));
                    }
                    if v["execution_status"] == "completed" {
                        return Err(err("run_status", "completed run contains failed cases"));
                    }
                }
                _ => return Err(err("run_status", "invalid run case status")),
            }
        }
        if observed != expected {
            return Err(err(
                "run_selection",
                "rendered case/seed population differs from suite",
            ));
        }
        for x in v["dependencies"].as_array().unwrap() {
            if x["role"] != "renderer" {
                reference(x)?
            }
        }
    } else if kind == "comparison" {
        if let Some(p) = v.get("policy") {
            reference(p)?
        } else if v["evaluation_status"] != "not_evaluated" {
            return Err(err(
                "evaluation_status",
                "comparison without policy must remain not_evaluated",
            ));
        }
        for case in v["cases"].as_array().unwrap() {
            measurements(&case["measurements"])?;
            if v["mode"] == "single_run" {
                let pairs = case["pairs"].as_array().ok_or_else(|| {
                    err(
                        "comparison_pair",
                        "Single-run case requires comparison pairs",
                    )
                })?;
                unique(pairs, "id")?;
                if !pairs.iter().any(|pair| pair["id"] == v["default_pair"]) {
                    return Err(err("comparison_pair", "Default pair is absent from case"));
                }
                for pair in pairs {
                    if pair["left_role"] != pair["id"] || pair["right_role"] != "simulation" {
                        return Err(err(
                            "comparison_pair",
                            "Pair roles must identify input/reference and simulation",
                        ));
                    }
                    if case["status"] == "completed" && pair["status"] != "completed" {
                        return Err(err(
                            "comparison_status",
                            "Completed case contains an incomplete pair",
                        ));
                    }
                    if pair["id"] == "reference" && pair["status"] == "completed" {
                        let scope = pair["reference"]["scope"].as_str().ok_or_else(|| {
                            err("comparison_pair", "Reference pair requires retained scope")
                        })?;
                        let aligned = scope == "aligned_scene"
                            && pair["reference"]["geometry"] == "pixel_aligned";
                        if pair["pixel_aligned"] != aligned {
                            return Err(err(
                                "comparison_correspondence",
                                "Pair alignment conflicts with retained reference correspondence",
                            ));
                        }
                        if pair["measurements"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|row| {
                                row["availability"] == "available"
                                    && (scope == "visual"
                                        || (scope == "aligned_scene" && !aligned)
                                        || pair["reference"]["color_available"] == false)
                            })
                        {
                            return Err(err(
                                "measurement_applicability",
                                "Available reference score conflicts with retained scope or interpretation",
                            ));
                        }
                    }
                    measurements(&pair["measurements"])?;
                    for measurement in pair["measurements"].as_array().into_iter().flatten() {
                        if measurement["case_id"] != case["case_id"]
                            || measurement["pair_id"] != pair["id"]
                        {
                            return Err(err(
                                "comparison_pair",
                                "Measurement identity differs from its pair",
                            ));
                        }
                    }
                    for item in pair["review"]
                        .as_object()
                        .into_iter()
                        .flat_map(|map| map.values())
                    {
                        reference(item)?;
                    }
                    for item in pair["artifacts"].as_array().into_iter().flatten() {
                        reference(item)?;
                    }
                    if let Some(item) = pair.get("measurement_artifact") {
                        reference(item)?;
                    }
                    for key in ["left_master", "right_master", "right_resolved"] {
                        if let Some(item) = pair.get(key) {
                            reference(item)?;
                        }
                    }
                }
            }
            for x in case["review"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
            {
                reference(x)?
            }
            for x in case["artifacts"].as_array().into_iter().flatten() {
                reference(x)?
            }
            if v["execution_status"] == "completed" && case["status"] == "failed" {
                return Err(err(
                    "comparison_status",
                    "completed comparison contains failed measurements",
                ));
            }
        }
        measurements(&v["seed_measurements"])?;
        unique(v["findings"].as_array().unwrap(), "id")?;
        for finding in v["findings"].as_array().unwrap() {
            let f = obj(finding)?;
            for k in [
                "id",
                "case_id",
                "region_id",
                "metric",
                "decision",
                "evidence",
            ] {
                req(f, k)?;
            }
            let e = finding["evidence"]
                .as_array()
                .filter(|x| !x.is_empty())
                .ok_or_else(|| err("finding_evidence", "finding requires inspectable evidence"))?;
            for x in e {
                reference(x)?
            }
        }
        if v["evaluation_status"] == "passed" && v["execution_status"] != "completed" {
            return Err(err("evaluation_status", "failed execution cannot qualify"));
        }
    }
    Ok(())
}
