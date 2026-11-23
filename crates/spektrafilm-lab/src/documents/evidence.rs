use super::{err, finite, obj, req, safe_artifact_path, text, unique, validate_schema};
use crate::common::{Availability, Result, digest, sha256_file};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct MeasurementRecord {
    pub case_id: String,
    pub region_id: String,
    pub metric: String,
    pub domain: String,
    pub units: String,
    pub estimator: String,
    /// Absent when the measurement did not run: a refusal has no count, and law
    /// 4 forbids writing one for a quantity that does not exist.
    #[serde(default)]
    pub sample_count: Option<u64>,
    pub availability: Availability,
    /// Absent for a delivered image, which is one realisation and has no seed
    /// population. A rendered run always states one.
    #[serde(default)]
    pub seed_population: Vec<u64>,
    pub artifact_refs: Vec<Value>,
    pub value: Option<f64>,
    pub reason: Option<String>,
}

/// An artifact reference must resolve in the manifest's index and agree with
/// its recorded hash.
fn indexed_reference(index: &std::collections::BTreeMap<&str, &Value>, item: &Value) -> Result<()> {
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
}

pub fn validate_generated_document(kind: &str, v: &Value, root: &Path) -> Result<()> {
    finite(v)?;
    validate_schema(kind, v)
        .map_err(|e| e.file(root.join(format!("{kind}.json")).display().to_string()))?;
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
    let reference = |item: &Value| indexed_reference(&index, item);
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
                "availability",
                "artifact_refs",
            ]
            .iter()
            .any(|k| !r.contains_key(*k))
                // A delivered image has no seed population; a rendered run
                // always states one.
                || (r.contains_key("seed_population")
                    && !row["seed_population"].is_array())
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
    let readouts = |owner: &Value| -> Result<()> {
        for readout in owner["region_readouts"].as_array().into_iter().flatten() {
            for artifact in readout["artifact_refs"].as_array().into_iter().flatten() {
                reference(artifact)?;
            }
            if let Some(mask) = readout.get("common_area_artifact_id") {
                reference(mask)?;
            }
            if owner.get("left_role").is_some() && readout["pair_id"] != owner["id"] {
                return Err(err(
                    "readout_identity",
                    "Color readout differs from its comparison pair",
                ));
            }
            let expected_domain = if readout["estimator"] == "bounded_lab_channel_median" {
                "bounded_srgb_d65_lab"
            } else {
                "unclipped_linear_srgb_mean_lab"
            };
            if readout["domain"] != expected_domain {
                return Err(err(
                    "readout_domain",
                    "Color readout domain differs from its estimator",
                ));
            }
            if readout["availability"] != "available" {
                continue;
            }
            let sides = readout["sides"].as_object().unwrap();
            let reference_review = &owner["reference"];
            let is_reference = readout["pair_id"] == "reference";
            if is_reference
                && (reference_review["scope"] == "visual"
                    || reference_review["color_available"] == false
                    || (reference_review["scope"] == "aligned_scene"
                        && reference_review["geometry"] != "pixel_aligned"))
            {
                return Err(err(
                    "readout_applicability",
                    "Color readout conflicts with retained reference qualification",
                ));
            }
            for displacement in readout["displacements"].as_array().unwrap() {
                let from = displacement["from_role"].as_str().unwrap();
                let to = displacement["to_role"].as_str().unwrap();
                if from == to
                    || !sides.contains_key(from)
                    || !sides.contains_key(to)
                    || (is_reference && from != "reference")
                    || (readout["pair_id"] == "input" && (from != "input" || to != "simulation"))
                    || (readout["pair_id"] == "baseline/candidate"
                        && (from != "baseline" || to != "candidate"))
                {
                    return Err(err(
                        "readout_identity",
                        "Color displacement must name the retained comparison direction",
                    ));
                }
                for (eligible, keys) in [
                    (
                        "tone_target_eligible",
                        &["delta_lab", "delta_l_star", "delta_e76", "delta_e00"][..],
                    ),
                    (
                        "color_diagnostic_eligible",
                        &["delta_c_star", "delta_h_degrees", "equal_l_delta_e00"][..],
                    ),
                ] {
                    if is_reference
                        && reference_review[eligible] != true
                        && keys
                            .iter()
                            .any(|key| displacement[*key]["availability"] == "available")
                    {
                        return Err(err(
                            "readout_applicability",
                            "Color displacement exceeds retained reference eligibility",
                        ));
                    }
                }
            }
        }
        Ok(())
    };
    if kind == "analysis" {
        // One source, one realisation: no seed population, no policy, and no
        // difference to evaluate.
        let source = obj(req(obj(v)?, "source")?)?;
        if source.get("kind") != Some(&json!("images")) {
            return Err(err(
                "source_kind",
                "an analysis source is a delivered image set",
            ));
        }
        for field in ["policy", "run", "mode", "evaluation_status", "findings"] {
            if v.get(field).is_some() {
                return Err(err(
                    "inapplicable_field",
                    format!("an analysis record carries no {field}"),
                ));
            }
        }
        for case in v["cases"].as_array().unwrap() {
            measurements(&case["measurements"])?;
            if case.get("seed").is_some() {
                return Err(err(
                    "inapplicable_field",
                    "a delivered image is one realisation and carries no seed",
                ));
            }
        }
    } else if kind == "comparison" {
        if matches!(
            v.get("evaluation_status").and_then(Value::as_str),
            Some(status) if status != "not_evaluated"
        ) {
            return Err(err(
                "evaluation_status",
                "an image comparison is never evaluated",
            ));
        }
        for case in v["cases"].as_array().unwrap() {
            measurements(&case["measurements"])?;
            readouts(case)?;

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
        if let Some(rows) = v.get("seed_measurements") {
            measurements(rows)?;
        }
        // An image comparison ranks nothing, so a finding has no rule behind
        // it; the array stays empty and a populated one must still resolve.
        let findings = v
            .get("findings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        unique(&findings, "id")?;
        for finding in &findings {
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
    }
    Ok(())
}
