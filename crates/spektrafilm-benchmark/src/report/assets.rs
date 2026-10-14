//! Portable offline presentation of retained evidence; no rendering or measurement.
use crate::{
    common::{Error, Result, sha256_file},
    documents::safe_artifact_path,
    image_data::read_npy,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use super::escape::*;
use super::html::retained_mask;

pub(crate) struct Assets {
    pub(crate) root: PathBuf,
    pub(crate) out: PathBuf,
    pub(crate) records: Vec<Value>,
    pub(crate) index: BTreeMap<String, Value>,
    pub(crate) copied: BTreeMap<String, Value>,
    pub(crate) manifest: Vec<Value>,
}
impl Assets {
    pub(crate) fn new(root: &Path, out: &Path, records: Vec<Value>) -> Result<Self> {
        let mut this = Self {
            root: root.into(),
            out: out.into(),
            records: vec![],
            index: BTreeMap::new(),
            copied: BTreeMap::new(),
            manifest: vec![],
        };
        for record in records {
            let path = record["path"]
                .as_str()
                .ok_or_else(|| Error::new("invalid_document", "artifact must identify a path"))?;
            if ["comparison.json", "report.json", "index.html"].contains(&path) {
                return Err(Error::new(
                    "invalid_artifact",
                    format!("artifact path conflicts with report output: {path}"),
                ));
            }
            safe_artifact_path(root, path)?;
            let retained = if let Some(prior) = this.index.get(path) {
                if !prior["sha256"].is_null()
                    && !record["sha256"].is_null()
                    && prior["sha256"] != record["sha256"]
                {
                    return Err(Error::new(
                        "artifact_hash",
                        format!("conflicting hashes for artifact path: {path}"),
                    ));
                }
                prior.clone()
            } else {
                this.records.push(record.clone());
                this.index.insert(path.into(), record.clone());
                record.clone()
            };
            if let Some(id) = record["id"].as_str() {
                if this
                    .index
                    .get(id)
                    .is_some_and(|v| v["path"] != retained["path"])
                {
                    return Err(Error::new(
                        "invalid_document",
                        format!("duplicate artifact id: {id}"),
                    ));
                }
                this.index.insert(id.into(), retained);
            }
        }
        Ok(this)
    }
    pub(crate) fn record(&self, reference: &Value) -> Option<Value> {
        if let Some(s) = reference.as_str() {
            return Some(
                self.index
                    .get(s)
                    .cloned()
                    .unwrap_or_else(|| json!({"path":s})),
            );
        }
        if let Some(path) = reference["path"].as_str() {
            let mut value = self.index.get(path).cloned().unwrap_or_else(|| json!({}));
            if let Some(map) = reference.as_object() {
                for (k, v) in map {
                    value[k] = v.clone();
                }
            }
            return Some(value);
        }
        reference["id"]
            .as_str()
            .and_then(|id| self.index.get(id).cloned())
    }
    pub(crate) fn copy(&mut self, reference: &Value) -> Result<Option<Value>> {
        let Some(mut entry) = self.record(reference) else {
            return Ok(None);
        };
        let path = get(&entry, "path", "").to_owned();
        let source = safe_artifact_path(&self.root, &path)?;
        if let Some(entry) = self.copied.get(&path) {
            return Ok(Some(entry.clone()));
        }
        entry["source_path"] = json!(path);
        let redistribution = entry.get("redistribution");
        let permitted = redistribution.is_none()
            || redistribution == Some(&json!(true))
            || redistribution
                .and_then(Value::as_str)
                .is_some_and(|v| ["permitted", "allowed", "CC0", "public_domain"].contains(&v));
        if !permitted {
            entry["path"] = Value::Null;
            entry["availability"] = json!("unavailable");
            entry["reason"] = json!("redistribution_not_permitted");
        } else if !source.is_file() {
            entry["path"] = Value::Null;
            entry["availability"] = json!("unavailable");
            entry["reason"] = json!("missing_artifact");
        } else {
            let digest = sha256_file(&source)?;
            if entry["sha256"].as_str().is_some_and(|v| v != digest) {
                return Err(Error::new(
                    "artifact_hash",
                    format!("artifact hash mismatch: {path}"),
                ));
            }
            let target = safe_artifact_path(&self.out, &path)?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source, &target)?;
            if sha256_file(&source)? != digest || sha256_file(&target)? != digest {
                return Err(Error::new(
                    "artifact_hash",
                    format!("artifact changed while copying: {path}"),
                ));
            }
            if entry.get("id").is_none() {
                entry["id"] = json!(format!("artifact-{:06}", self.manifest.len()));
            }
            entry["sha256"] = json!(digest);
            if entry.get("availability").is_none() {
                entry["availability"] = json!("available");
            }
        }
        self.manifest.push(entry.clone());
        self.copied.insert(path, entry.clone());
        Ok(Some(entry))
    }
    pub(crate) fn generated_path(&self, suffix: &str) -> Result<(String, PathBuf)> {
        let mut index = self.manifest.len();
        loop {
            let path = format!("artifacts/{index:06}-{suffix}");
            if !self.index.contains_key(&path) {
                let target = safe_artifact_path(&self.out, &path)?;
                if !target.exists() {
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    return Ok((path, target));
                }
            }
            index += 1;
        }
    }
    pub(crate) fn image(&mut self, reference: &Value) -> Result<Option<Value>> {
        if self
            .record(reference)
            .is_some_and(|entry| entry["availability"] == "unavailable")
        {
            return Ok(None);
        }
        let Some(entry) = self.copy(reference)? else {
            return Ok(None);
        };
        if entry["availability"] != "available" {
            return Ok(None);
        };
        let path = self.out.join(get(&entry, "path", ""));
        let suffix = path.extension().and_then(|v| v.to_str()).unwrap_or("");
        if suffix == "npy" && entry["role"] == "difference_map" {
            let (values, shape) = read_npy(&path)?;
            if shape.len() != 2 || values.iter().any(|v| !v.is_finite()) {
                return Err(Error::new(
                    "invalid_artifact",
                    "difference map must contain finite numeric two-dimensional samples",
                ));
            }
            let signed = !matches!(get(&entry, "metric", ""), "delta_e00" | "flip");
            let high = if entry["metric"] == "flip" { 1.0 } else { 10.0 };
            let low = if signed { -high } else { 0.0 };
            let valid = if let Some(mask) = entry.get("valid_mask_artifact_id") {
                let Some(mask) = self
                    .copy(mask)?
                    .filter(|e| e["availability"] == "available")
                else {
                    return Ok(None);
                };
                let (valid, width, height) = retained_mask(&self.out.join(get(&mask, "path", "")))?;
                if width != shape[1] || height != shape[0] {
                    return Err(Error::new(
                        "invalid_artifact",
                        "difference map mask dimensions differ",
                    ));
                }
                Some(valid)
            } else {
                None
            };
            let mut image = image::RgbaImage::new(shape[1] as u32, shape[0] as u32);
            for (i, (pixel, value)) in image.pixels_mut().zip(values).enumerate() {
                let value = value.clamp(low, high);
                let intensity = value.abs() / high;
                let rgb = if signed {
                    [
                        1.0 - if value < 0.0 { intensity } else { 0.0 },
                        1.0 - intensity,
                        1.0 - if value > 0.0 { intensity } else { 0.0 },
                    ]
                } else {
                    [intensity; 3]
                };
                let [r, g, b] = rgb.map(|v| (v * 255.0).round_ties_even() as u8);
                *pixel = image::Rgba([
                    r,
                    g,
                    b,
                    if valid.as_ref().is_none_or(|mask| mask[i]) {
                        255
                    } else {
                        0
                    },
                ]);
            }
            let (relative, target) = self.generated_path("map.png")?;
            image
                .save(target)
                .map_err(|e| Error::new("image_write", e.to_string()))?;
            let mut display = entry.clone();
            display["path"] = json!(relative);
            display["sha256"] = json!(sha256_file(&self.out.join(&relative))?);
            display["media_type"] = json!("image/png");
            display["source_sha256"] = entry["sha256"].clone();
            display["role"] = json!("difference_display");
            display["scale"] = json!([low, high]);
            display["clipping"] = json!([low, high]);
            display["amplification"] = json!(1);
            display["palette"] = json!(if signed {
                "negative blue / zero white / positive red".to_owned()
            } else {
                format!("black zero / white {high}")
            });
            self.manifest.push(display.clone());
            return Ok(Some(display));
        }
        Ok(if ["png", "jpg", "jpeg", "gif", "webp"].contains(&suffix) {
            Some(entry)
        } else {
            None
        })
    }
    pub(crate) fn crop(
        &mut self,
        entry: Option<&Value>,
        rect: Option<[u32; 4]>,
    ) -> Result<Option<Value>> {
        let (Some(entry), Some([x, y, w, h])) = (entry, rect) else {
            return Ok(None);
        };
        let image = image::open(self.out.join(get(entry, "path", "")))
            .map_err(|e| Error::new("invalid_artifact", e.to_string()))?;
        if x.checked_add(w).is_none_or(|v| v > image.width())
            || y.checked_add(h).is_none_or(|v| v > image.height())
        {
            return Err(Error::new(
                "region_bounds",
                "region outside retained full image",
            ));
        }
        let (path, target) = self.generated_path("detail.png")?;
        image
            .crop_imm(x, y, w, h)
            .save(target)
            .map_err(|e| Error::new("image_write", e.to_string()))?;
        let result = json!({"path":path,"sha256":sha256_file(&self.out.join(&path))?,"media_type":"image/png","role":"detail","availability":"available","dimensions":[w,h],"source_path":entry["source_path"],"source_sha256":entry["sha256"],"rectangle":[x,y,w,h],"method":"crop_completed_full_image","resampling":"none","bit_depth":entry.get("bit_depth").cloned().unwrap_or(json!(8))});
        self.manifest.push(result.clone());
        Ok(Some(result))
    }
    pub(crate) fn plot(&mut self, reference: &Value, label: &str) -> Result<Option<String>> {
        let Some(entry) = self.copy(reference)? else {
            return Ok(None);
        };
        if entry["availability"] != "available" {
            return Ok(None);
        };
        let path = self.out.join(get(&entry, "path", ""));
        let mut series: Vec<(Vec<f64>, Vec<f64>, String)> = vec![];
        if path.extension().is_some_and(|v| v == "npy") {
            let (data, shape) = read_npy(&path)?;
            if shape.len() == 1 {
                series.push((
                    (0..data.len()).map(|i| i as f64).collect(),
                    data,
                    label.into(),
                ));
            } else if shape.len() == 2 && shape[1] >= 2 {
                let columns = rows(&entry["columns"]);
                for col in 1..shape[1] {
                    if columns
                        .get(col)
                        .and_then(Value::as_str)
                        .is_some_and(|v| ["count", "pixel_count", "bin_count"].contains(&v))
                    {
                        continue;
                    }
                    let x = data.chunks_exact(shape[1]).map(|r| r[0]).collect();
                    let y = data.chunks_exact(shape[1]).map(|r| r[col]).collect();
                    series.push((
                        x,
                        y,
                        columns
                            .get(col)
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("{label} {col}")),
                    ));
                }
            }
        } else if path.extension().is_some_and(|v| v == "json") {
            let value: Value = serde_json::from_slice(&fs::read(&path)?)?;
            let numeric = |v: &Value| -> Option<Vec<f64>> {
                v.as_array()?.iter().map(Value::as_f64).collect()
            };
            if value.is_object() {
                if entry["role"] == "reference_regional_statistics" {
                    if let (Some(x), Some(curves)) = (
                        numeric(&value["lightness_quantile_probabilities"]),
                        value["lightness_quantiles"].as_object(),
                    ) {
                        for (name, curve) in curves {
                            if let Some(y) = numeric(curve) {
                                series.push((x.clone(), y, name.clone()));
                            }
                        }
                    }
                } else {
                    let xvalue = if entry["role"] == "reference_material_curve" {
                        &value["coordinates"]
                    } else {
                        value
                            .get("x")
                            .or_else(|| value.get("coordinates"))
                            .or_else(|| value.get("exposure"))
                            .unwrap_or(&Value::Null)
                    };
                    if let Some(x) = numeric(xvalue) {
                        for (name, curve) in value.as_object().unwrap() {
                            if [
                                "x",
                                "coordinates",
                                "exposure",
                                "units",
                                "x_units",
                                "y_units",
                            ]
                            .contains(&name.as_str())
                            {
                                continue;
                            }
                            if entry["role"] == "reference_material_curve"
                                && !["baseline", "candidate", "reference", "simulation"]
                                    .contains(&name.as_str())
                            {
                                continue;
                            }
                            if let Some(y) = numeric(curve) {
                                series.push((x.clone(), y, name.clone()));
                            }
                        }
                    }
                }
            } else if let Some(data) = value.as_array() {
                let pairs: Option<Vec<_>> = data
                    .iter()
                    .map(|r| Some((r.get(0)?.as_f64()?, r.get(1)?.as_f64()?)))
                    .collect();
                if let Some(pairs) = pairs {
                    series.push((
                        pairs.iter().map(|v| v.0).collect(),
                        pairs.iter().map(|v| v.1).collect(),
                        label.into(),
                    ));
                }
            }
        }
        let points: Vec<_> = series
            .iter()
            .flat_map(|(x, y, _)| x.iter().copied().zip(y.iter().copied()))
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .collect();
        if points.is_empty() {
            return Ok(None);
        };
        let xmin = points.iter().map(|v| v.0).fold(f64::INFINITY, f64::min);
        let mut xmax = points.iter().map(|v| v.0).fold(f64::NEG_INFINITY, f64::max);
        let ymin = points.iter().map(|v| v.1).fold(f64::INFINITY, f64::min);
        let mut ymax = points.iter().map(|v| v.1).fold(f64::NEG_INFINITY, f64::max);
        if xmin == xmax {
            xmax = xmin + 1.0;
        }
        if ymin == ymax {
            ymax = ymin + 1.0;
        }
        let colors = ["#8dccff", "#ffca69", "#ff8d8d", "#9ee493"];
        let mut paths = String::new();
        for (i, (xs, ys, name)) in series.iter().enumerate() {
            let coords = xs
                .iter()
                .zip(ys)
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .map(|(x, y)| {
                    format!(
                        "{:.2},{:.2}",
                        40.0 + 520.0 * (x - xmin) / (xmax - xmin),
                        220.0 - 180.0 * (y - ymin) / (ymax - ymin)
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            if !coords.is_empty() {
                paths.push_str(&format!("<polyline fill=\"none\" stroke=\"{}\" points=\"{}\"/><text x=\"45\" y=\"{}\" fill=\"{}\">{}</text>",colors[i%4],coords,35+i*16,colors[i%4],escape(name)));
            }
        }
        let units = format!(
            "{} / {}",
            entry["columns"]
                .get(0)
                .and_then(Value::as_str)
                .unwrap_or(get(&entry, "x_units", "sample coordinate")),
            get(&entry, "units", get(&entry, "domain", "declared response"))
        );
        Ok(Some(format!(
            "<svg class=\"evidence-plot\" viewBox=\"0 0 600 260\" role=\"img\" aria-label=\"{}\"><rect x=\"0\" y=\"0\" width=\"600\" height=\"260\" fill=\"#090b10\"/><path d=\"M40 220H560M40 20V220\" stroke=\"#8794aa\"/><text x=\"45\" y=\"250\" fill=\"#b9c6d8\">{} · x [{xmin},{xmax}] · y [{ymin},{ymax}]</text>{paths}</svg>",
            escape(label),
            escape(&units)
        )))
    }
}
pub(crate) fn role_reference(case: &Value, role: &str, assets: &Assets) -> Value {
    if case.get("pair_id").is_some() {
        let key = match role {
            "baseline" => "left",
            "candidate" => "right",
            other => other,
        };
        let value = &case["review"][key];
        return value
            .get("display_artifact_id")
            .or_else(|| value.get("artifact_id"))
            .unwrap_or(value)
            .clone();
    }
    let names: &[&str] = match role {
        "baseline" => &["baseline_review", "baseline_image", "baseline"],
        "candidate" => &["candidate_review", "candidate_image", "candidate"],
        "source" => &["source_review", "source_image", "source"],
        "reference" => &["reference_review", "reference_image", "reference"],
        _ => &[
            "difference_map",
            "difference_image",
            "difference",
            "delta_e00_map",
        ],
    };
    if let Some(value) = case["review"].get(role) {
        if value.is_object() {
            return value
                .get("display_artifact_id")
                .or_else(|| value.get("artifact_id"))
                .or_else(|| value.get("path"))
                .unwrap_or(value)
                .clone();
        }
        if !value.is_null() {
            return value.clone();
        }
    }
    for name in names {
        if let Some(value) = case["artifacts"].get(name).or_else(|| case.get(name))
            && !value.is_null()
            && assets.record(value).is_some()
        {
            return value.clone();
        }
    }
    for reference in rows(&case["artifacts"]) {
        if let Some(record) = assets.record(reference)
            && names.contains(&get(&record, "role", ""))
        {
            return record;
        }
    }
    for record in &assets.records {
        if record["case_id"]
            == case
                .get("case_id")
                .or_else(|| case.get("id"))
                .cloned()
                .unwrap_or(Value::Null)
            && (!record.get("seed").is_some() || record["seed"] == case["seed"])
            && names.contains(&get(record, "role", ""))
        {
            return record.clone();
        }
    }
    Value::Null
}
