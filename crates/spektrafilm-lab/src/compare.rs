use crate::{
    Error, Result,
    common::{self, Evidence, text},
    documents,
    image_data::{self, ImageData},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

pub(crate) fn review_image(
    image: &ImageData,
    destination: &Path,
    evidence: &mut Evidence,
    role: &str,
    case_id: &str,
    seed: u64,
) -> Result<Value> {
    let samples = image_data::display_rgb(image)?;
    if samples.iter().flatten().any(|v| !v.is_finite()) {
        return Err(Error::new(
            "nonfinite_review",
            "Review image has nonfinite display samples",
        ));
    }
    image_data::write_png16(destination, &samples, image.width, image.height)?;
    let (fit, w, h) = image_data::resize_display(&samples, image.width, image.height, 1600)?;
    let fit_path = destination.with_file_name(format!(
        "{}-fit.png",
        destination
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(role)
    ));
    image_data::write_png16(&fit_path, &fit, w, h)?;
    let fit_record=evidence.add(&fit_path,"fit_preview","image/png",json!({"width":w,"height":h,"dimensions":[w,h],"source_dimensions":image.dimensions(),"bit_depth":16,"resampling":"Lanczos in encoded sRGB before quantization","quantization":"16-bit rounded; no dither","color":{"color_space":"sRGB","transfer":"srgb"},"case_id":case_id,"seed":seed}))?;
    evidence.add(destination,role,"image/png",json!({"width":image.width,"height":image.height,"dimensions":image.dimensions(),"source_dimensions":image.dimensions(),"bit_depth":16,"resampling":"none","quantization":"16-bit rounded; no dither","color":{"color_space":"sRGB","transfer":"srgb"},"fit_preview":fit_record["path"],"case_id":case_id,"seed":seed}))
}
// All generated readouts refer to immutable artifacts inside the comparison.
pub(crate) fn relative_paths(value: &mut Value, root: &Path) {
    match value {
        Value::Object(map) => {
            if let Some(path) = map
                .get("path")
                .and_then(Value::as_str)
                .and_then(|p| Path::new(p).strip_prefix(root).ok())
            {
                map.insert(
                    "path".into(),
                    json!(path.to_string_lossy().replace('\\', "/")),
                );
            }
            for child in map.values_mut() {
                relative_paths(child, root);
            }
        }
        Value::Array(values) => {
            for child in values {
                relative_paths(child, root);
            }
        }
        _ => {}
    }
}

pub(crate) fn retain_metrics(
    mut measured: Value,
    folder: &Path,
    evidence: &mut Evidence,
    case_id: &str,
    seed: u64,
) -> Result<Value> {
    let mut refs = BTreeMap::<String, String>::new();
    let mut records = Vec::new();
    for artifact in measured["artifacts"].as_array().into_iter().flatten() {
        let mut metadata = artifact.clone();
        let obj = metadata
            .as_object_mut()
            .ok_or_else(|| Error::new("artifact_identity", "Artifact must be object"))?;
        for key in [
            "id",
            "path",
            "sha256",
            "media_type",
            "role",
            "case_id",
            "seed",
        ] {
            obj.remove(key);
        }
        let id = artifact["id"].as_str().unwrap_or(text(artifact, "path")?);
        metadata["id"] = json!(format!("{case_id}/seed-{seed}/{id}"));
        metadata["case_id"] = json!(case_id);
        metadata["seed"] = json!(seed);
        let mut record = evidence.add(
            &documents::safe_artifact_path(folder, text(artifact, "path")?)?,
            text(artifact, "role")?,
            artifact["media_type"]
                .as_str()
                .unwrap_or("application/octet-stream"),
            metadata,
        )?;
        if let Some(value) = record
            .get("valid_mask_artifact_id")
            .and_then(Value::as_str)
            .and_then(|id| refs.get(id))
            .cloned()
        {
            record["valid_mask_artifact_id"] = json!(value);
            evidence
                .records
                .insert(text(&record, "path")?.into(), record.clone());
        }
        refs.insert(id.into(), text(&record, "path")?.into());
        records.push(record);
    }
    fn rewrite_list(value: &mut Value, refs: &BTreeMap<String, String>) {
        if let Some(items) = value.as_array_mut() {
            for item in items {
                if let Some(path) = item.as_str().and_then(|s| refs.get(s)) {
                    *item = json!(path);
                }
            }
        }
    }
    if let Some(rows) = measured["measurements"].as_array_mut() {
        for row in rows {
            rewrite_list(&mut row["artifact_refs"], &refs);
            row["case_id"] = json!(case_id);
        }
    }
    if let Some(rows) = measured["findings"].as_array_mut() {
        for row in rows {
            rewrite_list(&mut row["evidence"], &refs);
        }
    }
    if let Some(regions) = measured["automatic_regions"].as_array_mut() {
        for region in regions {
            rewrite_list(&mut region["artifact_refs"], &refs);
        }
    }
    if let Some(readouts) = measured["region_readouts"].as_array_mut() {
        for readout in readouts {
            rewrite_list(&mut readout["artifact_refs"], &refs);
            if let Some(path) = readout["common_area_artifact_id"]
                .as_str()
                .and_then(|id| refs.get(id))
            {
                readout["common_area_artifact_id"] = json!(path);
            }
        }
    }
    if let Some(review) = measured["reference_review"].as_object_mut() {
        for (key, value) in &mut *review {
            if key.ends_with("_artifact_id") {
                if let Some(path) = value.as_str().and_then(|s| refs.get(s)) {
                    *value = json!(path);
                }
            } else if key == "artifact_refs" {
                rewrite_list(value, &refs);
            }
        }
        if let Some(regions) = review
            .get_mut("automatic_regions")
            .and_then(Value::as_array_mut)
        {
            for region in regions {
                rewrite_list(&mut region["artifact_refs"], &refs);
            }
        }
    }
    measured["artifacts"] = json!(records);
    Ok(measured)
}

/// Rewrites each finding identity with the owning scope and defaults a missing
/// evidence reference to the published measurement artifact.
pub(crate) fn publish_measurement_findings(
    rows: &mut [Value],
    scope: &[Value],
    measurement_path: &Value,
) -> Result<Vec<Value>> {
    let mut published = Vec::with_capacity(rows.len());
    for finding in rows {
        let mut identity = scope.to_vec();
        identity.push(finding["id"].clone());
        finding["id"] = json!(common::digest(&Value::Array(identity))?);
        if finding["evidence"]
            .as_array()
            .is_none_or(|refs| refs.is_empty())
        {
            finding["evidence"] = json!([measurement_path]);
        }
        published.push(finding.clone());
    }
    Ok(published)
}
