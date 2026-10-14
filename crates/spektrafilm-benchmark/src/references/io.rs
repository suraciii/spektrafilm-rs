use crate::common::{Error, Result};
use crate::image_data::ImageData;
use crate::{documents, image_data};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn err(code: &str, message: impl Into<String>) -> Error {
    Error::new(code, message.into())
}
pub fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.get(key)
        .ok_or_else(|| err("invalid_reference_evidence", format!("Missing {key}")))
}
pub fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    field(v, key)?.as_str().ok_or_else(|| {
        err(
            "invalid_reference_evidence",
            format!("{key} must be a string"),
        )
    })
}
pub fn optional_array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.),
    }
}
pub fn path_of(value: &Value, base: &Path) -> Result<PathBuf> {
    let s = value
        .as_str()
        .or_else(|| value.get("path").and_then(Value::as_str))
        .ok_or_else(|| err("invalid_reference_evidence", "Evidence requires a path"))?;
    Ok(if Path::new(s).is_absolute() {
        PathBuf::from(s)
    } else {
        base.join(s)
    })
}
pub fn read_json(value: &Value, base: &Path) -> Result<Value> {
    let p = path_of(value, base)?;
    let v: Value = serde_json::from_slice(
        &fs::read(&p).map_err(|e| err("invalid_reference_evidence", e.to_string()))?,
    )
    .map_err(|e| err("invalid_reference_evidence", e.to_string()))?;
    if !v.is_object() {
        return Err(err(
            "invalid_reference_evidence",
            "Reference evidence must be a JSON object",
        ));
    }
    Ok(v)
}
pub fn read_reference(reference: &Value, base: &Path) -> Result<ImageData> {
    let mut color = reference.get("color").cloned().unwrap_or(json!({}));
    if color.get("icc").and_then(|v| v.get("path")).is_some() {
        color["icc"]["path"] = json!(path_of(&color["icc"], base)?);
    }
    image_data::load_image(&json!({"path":path_of(field(reference,"path")?,base)?,"color":color}))
}
pub fn mask(geometry: &Value, dims: [usize; 2], base: &Path) -> Result<Vec<bool>> {
    documents::reference_geometry_mask(geometry, dims[0], dims[1], base)
}
pub fn finite_rgb(v: &[[f64; 3]], code: &str, message: &str) -> Result<()> {
    if v.iter().flatten().any(|x| !x.is_finite()) {
        Err(err(code, message))
    } else {
        Ok(())
    }
}
pub fn clipped_lab(v: &[[f64; 3]]) -> Vec<[f64; 3]> {
    let bounded: Vec<[f64; 3]> = v.iter().map(|p| p.map(|x| x.clamp(0., 1.))).collect();
    image_data::rgb_to_lab(&bounded)
}
pub fn clipping_fraction(v: &[[f64; 3]]) -> f64 {
    v.iter().flatten().filter(|x| **x < 0. || **x > 1.).count() as f64 / (v.len() * 3) as f64
}
pub fn percentile(mut values: Vec<f64>, q: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    let index = q * (values.len() - 1) as f64;
    let lo = index.floor() as usize;
    let hi = index.ceil() as usize;
    let fraction = index - lo as f64;
    // Match numpy's linear interpolation, including its upper-half formulation.
    let diff = values[hi] - values[lo];
    if fraction >= 0.5 {
        values[hi] - diff * (1. - fraction)
    } else {
        values[lo] + diff * fraction
    }
}
pub fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}
pub fn population(mask: &[bool]) -> Vec<usize> {
    mask.iter()
        .enumerate()
        .filter_map(|(i, b)| b.then_some(i))
        .collect()
}
pub fn processing_eligibility(reference: &Value) -> Result<(bool, bool)> {
    let processing = reference.get("processing").cloned().unwrap_or(json!({}));
    let normalize = |key: &str| -> Result<String> {
        let value = match processing.get(key) {
            Some(v) => v.as_str().ok_or_else(|| {
                err(
                    "invalid_reference_evidence",
                    "Processing history must be text",
                )
            })?,
            None => "unknown",
        };
        Ok(value.trim().to_lowercase())
    };
    let grading = normalize("grading")?;
    let histogram = normalize("histogram_matching")?;
    let clean = ["none", "unmodified", "not_applied"];
    Ok((
        clean.contains(&grading.as_str()) && clean.contains(&histogram.as_str()),
        clean.contains(&grading.as_str())
            && (clean.contains(&histogram.as_str()) || histogram == "brightness_only"),
    ))
}
