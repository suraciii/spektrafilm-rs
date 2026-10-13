//! Portable offline presentation of retained evidence; no rendering or measurement.
use crate::{
    common::{Error, Result, create_destination, dump, sha256_file},
    documents::{load_json_document, safe_artifact_path, validate_generated_document},
    image_data::read_npy,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
const CSS: &str = include_str!("../assets/report.css");
const JS: &str = include_str!("../assets/report.js");
fn text(value: &Value) -> String {
    let value = match value {
        Value::String(s) => s.clone(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string_pretty(value).unwrap_or_default()
        }
        _ => value.to_string(),
    };
    escape(&value)
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
fn get<'a>(value: &'a Value, key: &str, default: &'a str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or(default)
}
fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .map(text)
        .unwrap_or_else(|| "unavailable".into())
}
fn rows(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn measurements(case: &Value) -> &[Value] {
    let value = case.get("metrics").or_else(|| case.get("measurements"));
    match value {
        Some(Value::Object(map)) => map.get("measurements").map(rows).unwrap_or(&[]),
        Some(v) => rows(v),
        None => &[],
    }
}
fn human(id: &str) -> String {
    match id {
        "color_chart_30mp" => "30 MP 色彩测试图".into(),
        "color_chart" => "色彩测试图".into(),
        "kodak_dc50_scene" => "自然场景 · Kodak DC50".into(),
        "exposure_response" => "灰阶响应".into(),
        "grain_fields" => "平场颗粒".into(),
        "halation_point" => "点光源与光晕".into(),
        "edge_response" => "边缘响应".into(),
        "skin" => "肤色".into(),
        "shadow" => "阴影".into(),
        "texture" => "纹理".into(),
        "neutral" => "中性灰".into(),
        _ => id
            .replace(['_', '-'], " ")
            .split_whitespace()
            .map(|p| {
                let mut c = p.chars();
                c.next().map_or(String::new(), |v| {
                    v.to_uppercase().collect::<String>() + c.as_str()
                })
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}
fn url(entry: &Value) -> String {
    let mut encoded = String::new();
    for b in get(entry, "path", "").bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    escape(&encoded)
}
fn normalized_artifacts(value: &Value) -> Result<Vec<Value>> {
    match value {
        Value::Array(v) => Ok(v.clone()),
        Value::Object(v) => Ok(v
            .iter()
            .map(|(id, v)| {
                let mut r = if v.is_object() {
                    v.clone()
                } else {
                    json!({"path":v})
                };
                r["id"] = json!(id);
                r
            })
            .collect()),
        Value::Null => Ok(vec![]),
        _ => Err(Error::new(
            "invalid_document",
            "artifacts must be an array or object",
        )),
    }
}
struct Assets {
    root: PathBuf,
    out: PathBuf,
    records: Vec<Value>,
    index: BTreeMap<String, Value>,
    copied: BTreeMap<String, Value>,
    manifest: Vec<Value>,
}
impl Assets {
    fn new(root: &Path, out: &Path, records: Vec<Value>) -> Result<Self> {
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
    fn record(&self, reference: &Value) -> Option<Value> {
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
    fn copy(&mut self, reference: &Value) -> Result<Option<Value>> {
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
    fn generated_path(&self, suffix: &str) -> Result<(String, PathBuf)> {
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
    fn image(&mut self, reference: &Value) -> Result<Option<Value>> {
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
    fn crop(&mut self, entry: Option<&Value>, rect: Option<[u32; 4]>) -> Result<Option<Value>> {
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
    fn plot(&mut self, reference: &Value, label: &str) -> Result<Option<String>> {
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
fn role_reference(case: &Value, role: &str, assets: &Assets) -> Value {
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
fn image_html(entry: Option<&Value>, label: &str) -> String {
    match entry {
        None => format!(
            "<div class=\"unavailable\">{}: unavailable</div>",
            escape(label)
        ),
        Some(e) => format!(
            "<figure><img src=\"{}\" alt=\"{}\" loading=\"lazy\"><figcaption>{} · <a href=\"{}\">individual image</a></figcaption></figure>",
            url(e.get("fit_entry").filter(|v| !v.is_null()).unwrap_or(e)),
            escape(label),
            escape(label),
            url(e)
        ),
    }
}
fn viewer(
    a: Option<&Value>,
    b: Option<&Value>,
    detail: bool,
    labels: (&str, &str),
    aligned: bool,
) -> String {
    let (Some(a), Some(b)) = (a, b) else {
        return format!(
            "<div class=\"grid\">{}{}</div>",
            image_html(a, labels.0),
            image_html(b, labels.1)
        );
    };
    let left = url(a);
    let right = url(b);
    let left_fit = url(a.get("fit_entry").filter(|v| !v.is_null()).unwrap_or(a));
    let right_fit = url(b.get("fit_entry").filter(|v| !v.is_null()).unwrap_or(b));
    let attrs = |native: &str, fit: &str| format!("data-native=\"{native}\" data-fit=\"{fit}\"");
    let la = attrs(&left, &left_fit);
    let ra = attrs(&right, &right_fit);
    let ls = if detail { &left } else { &left_fit };
    let rs = if detail { &right } else { &right_fit };
    let options = [("fit", "Fit"), ("1", "100%"), ("2", "200%"), ("4", "400%")]
        .iter()
        .map(|(v, l)| {
            format!(
                "<option value=\"{v}\"{}>{l}</option>",
                if *v == if detail { "1" } else { "fit" } {
                    " selected"
                } else {
                    ""
                }
            )
        })
        .collect::<String>();
    format!(
        r#"<div class="viewer" data-mode="{mode}" data-aligned="{aligned}"><div class="controls"><label>查看 <select data-mode>{split_option}<option value="side">左右对照</option><option value="baseline">{left_label}</option><option value="candidate">{right_label}</option></select></label><button type="button" data-alternate>切换 A/B</button><label>倍率 <select data-zoom>{options}</select></label><label {split_hidden}>分割 <input type="range" min="0" max="100" value="50" data-split aria-label="A/B split position"></label><span data-scale-label class="small"></span></div><div class="panes"><div class="side-pane" data-side="baseline"><p>{left_label}</p><div class="viewport" tabindex="0" aria-label="{left_label} pan area"><div class="canvas"><img src="{ls}" {la} alt="{left_label}" loading="lazy"></div></div></div><div class="side-pane" data-side="candidate"><p>{right_label}</p><div class="viewport" tabindex="0" aria-label="{right_label} pan area"><div class="canvas"><img src="{rs}" {ra} alt="{right_label}" loading="lazy"></div></div></div><div class="split-pane"><p>{left_label}在左 / {right_label}在右 · 拖动分割线</p><div class="viewport" tabindex="0" aria-label="Split pan area"><div class="canvas split"><img src="{ls}" {la} alt="{left_label}" loading="lazy"><img class="candidate" src="{rs}" {ra} alt="{right_label}" loading="lazy"><button type="button" class="split-handle" data-split-handle role="slider" aria-label="拖动分割线" aria-valuemin="0" aria-valuemax="100" aria-valuenow="50" style="left:50%">↔</button></div></div></div></div><p><a href="{left}">{left_label} image</a> · <a href="{right}">{right_label} image</a>{}</p></div>"#,
        if a["bit_depth"] == 16 {
            " · 16-bit PNG，无抖动；屏幕显示精度由浏览器与显示设备决定。"
        } else {
            ""
        },
        mode = if aligned { "split" } else { "side" },
        split_option = if aligned {
            "<option value=\"split\">分割对照</option>"
        } else {
            ""
        },
        split_hidden = if aligned { "" } else { "hidden" },
        left_label = escape(labels.0),
        right_label = escape(labels.1)
    )
}
fn rectangle(region: &Value) -> Result<Option<[u32; 4]>> {
    let geometry = &region["geometry"];
    let rect = region
        .get("rectangle")
        .or_else(|| region.get("rect"))
        .or_else(|| {
            if geometry.is_array() {
                Some(geometry)
            } else {
                geometry.get("rectangle").or_else(|| geometry.get("rect"))
            }
        });
    let Some(rect) = rect else { return Ok(None) };
    let values = rect.as_array().filter(|v| v.len() == 4).ok_or_else(|| {
        Error::new(
            "region_bounds",
            "detail rectangle must contain four integers",
        )
    })?;
    let mut result = [0; 4];
    for (i, v) in values.iter().enumerate() {
        result[i] = v
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| {
                Error::new(
                    "region_bounds",
                    "detail rectangle must contain four nonnegative integers",
                )
            })?;
    }
    if result[2] == 0 || result[3] == 0 {
        return Err(Error::new(
            "region_bounds",
            "detail rectangle must have positive size",
        ));
    }
    Ok(Some(result))
}
fn metric_table(case: &Value) -> String {
    let mut body = String::new();
    for row in measurements(case) {
        if !row.is_object() {
            continue;
        }
        body.push_str("<tr>");
        for key in ["region_id", "metric", "units", "estimator", "sample_count"] {
            body.push_str(&format!("<td>{}</td>", field(row, key)));
        }
        body.push_str(&format!(
            "<td>{}</td></tr>",
            field(
                row,
                if get(row, "availability", "available") == "available" {
                    "value"
                } else {
                    "reason"
                }
            )
        ));
    }
    if body.is_empty() {
        "<div class=\"unavailable\">Metric table: unavailable</div>".into()
    } else {
        format!(
            "<table><thead><tr><th>Region</th><th>Metric</th><th>Units</th><th>Estimator</th><th>Samples</th><th>Value/status</th></tr></thead><tbody>{body}</tbody></table>"
        )
    }
}
fn metric_summary(case: &Value, labels: (&str, &str)) -> String {
    let mut cards = String::new();
    for (metric, label, direction) in [
        (
            "delta_e00.p95",
            "色差 P95",
            format!("越小越接近{}", labels.0),
        ),
        (
            "flip.mean",
            "FLIP 平均差异",
            format!("越小越接近{}", labels.0),
        ),
        (
            "delta_l_star.mean",
            "平均亮度变化",
            format!("正值更亮；{} − {}", labels.1, labels.0),
        ),
        (
            "numeric.rmse",
            "像素 RMS 差异",
            format!("越小越接近{}", labels.0),
        ),
        (
            "reference.delta_e00.p95.simulation_distance",
            "对比图色差 P95",
            "越小越接近对比图".into(),
        ),
        (
            "reference.flip.mean.simulation_distance",
            "对比图 FLIP 平均差异",
            "越小越接近对比图".into(),
        ),
    ] {
        if let Some(row) = measurements(case).iter().find(|r| {
            r["region_id"] == "full_image"
                && r["metric"] == metric
                && get(r, "availability", "available") == "available"
                && r["value"].as_f64().is_some()
        }) {
            let value = row["value"].as_f64().unwrap();
            let rounded = if value.abs() >= 0.001 || value == 0.0 {
                format!("{value:.3}")
            } else {
                format!("{value:.2e}")
            };
            cards.push_str(&format!("<div title=\"{}\"><span>{label}</span><strong>{rounded} <small>{}</small></strong><small>{direction}</small></div>",escape(get(row,"domain","未声明域")),escape(get(row,"units",""))));
        }
    }
    if cards.is_empty() {
        cards
    } else {
        format!("<div class=\"metric-summary\" aria-label=\"全图差异摘要\">{cards}</div>")
    }
}
fn changed(left: &Value, right: &Value, path: &str, changes: &mut Vec<String>) {
    if let (Some(a), Some(b)) = (left.as_object(), right.as_object()) {
        let keys: BTreeSet<_> = a.keys().chain(b.keys()).collect();
        for key in keys {
            changed(
                &left[key],
                &right[key],
                &if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                },
                changes,
            );
        }
    } else if left != right {
        let name = if path == "camera.exposure_compensation_ev" {
            "曝光补偿（EV）"
        } else {
            path
        };
        changes.push(format!("{name}: {} → {}", short(left), short(right)));
    }
}
fn short(value: &Value) -> String {
    let s = if let Some(s) = value.as_str() {
        s.to_owned()
    } else {
        value.to_string()
    };
    if s.chars().count() > 64 {
        s.chars().take(61).collect::<String>() + "…"
    } else {
        s
    }
}
fn change_summary(case: &Value, comparison: &Value) -> String {
    let mut changes = vec![];
    for row in rows(&comparison["condition_differences"]) {
        if !row["case_id"].is_null()
            && row["case_id"]
                != case
                    .get("case_id")
                    .or_else(|| case.get("id"))
                    .cloned()
                    .unwrap_or(Value::Null)
        {
            continue;
        }
        if row.get("seed").is_some() && row["seed"] != case["seed"] {
            continue;
        }
        let name = get(row, "field", "condition");
        if name == "resolved_parameters" {
            changed(&row["baseline"], &row["candidate"], "", &mut changes);
        } else {
            let (a, b) = if row.get("baseline").is_some() && row.get("candidate").is_some() {
                (&row["baseline"], &row["candidate"])
            } else {
                (&row["requested"], &row["observed"])
            };
            changes.push(format!("{}: {} → {}", human(name), short(a), short(b)));
        }
    }
    let mut result = if changes.is_empty() {
        "保留的比较条件未记录变化。".into()
    } else {
        changes
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ")
    };
    if changes.len() > 3 {
        result.push_str(&format!(" · 另 {} 项（见支持证据）", changes.len() - 3));
    }
    format!(
        "<p class=\"change-summary\"><strong>基准 → 候选</strong> · {}</p>",
        escape(&result)
    )
}
fn folded(label: &str, value: &Value) -> String {
    format!(
        "<details><summary>{}</summary><pre>{}</pre></details>",
        escape(label),
        text(value)
    )
}
fn retained_mask(path: &Path) -> Result<(Vec<bool>, usize, usize)> {
    if path.extension().is_some_and(|v| v == "npy") {
        let (data, shape) = read_npy(path)?;
        if shape.len() != 2 || shape[0] == 0 || shape[1] == 0 || data.iter().any(|v| !v.is_finite())
        {
            return Err(Error::new(
                "invalid_artifact",
                "retained region mask must contain finite two-dimensional samples",
            ));
        }
        Ok((
            data.into_iter().map(|v| v != 0.0).collect(),
            shape[1],
            shape[0],
        ))
    } else {
        let image = image::open(path)
            .map_err(|e| Error::new("invalid_artifact", e.to_string()))?
            .to_luma32f();
        Ok((
            image.pixels().map(|p| p[0] != 0.0).collect(),
            image.width() as usize,
            image.height() as usize,
        ))
    }
}
fn geometry_rectangle(geometry: &Value, assets: &mut Assets) -> Result<Option<[u32; 4]>> {
    if let Some(path) = geometry["path"].as_str() {
        let Some(entry) = assets
            .copy(&json!(path))?
            .filter(|e| e["availability"] == "available")
        else {
            return Ok(None);
        };
        let (mask, w, h) = retained_mask(&assets.out.join(get(&entry, "path", "")))?;
        let mut bounds = [w, h, 0, 0];
        let mut found = false;
        for (i, valid) in mask.iter().enumerate() {
            if *valid {
                found = true;
                bounds[0] = bounds[0].min(i % w);
                bounds[1] = bounds[1].min(i / w);
                bounds[2] = bounds[2].max(i % w);
                bounds[3] = bounds[3].max(i / w);
            }
        }
        if !found {
            return Err(Error::new("region_bounds", "retained region mask is empty"));
        }
        return Ok(Some([
            bounds[0] as u32,
            bounds[1] as u32,
            (bounds[2] - bounds[0] + 1) as u32,
            (bounds[3] - bounds[1] + 1) as u32,
        ]));
    }
    rectangle(&json!({"geometry":geometry}))
}

fn pair_case(case: &Value, pair: &Value) -> Value {
    let mut view = pair.clone();
    for key in [
        "case_id",
        "seed",
        "source_dimensions",
        "source_geometry",
        "preparation",
        "input",
    ] {
        view[key] = case[key].clone();
    }
    view["pair_id"] = pair["id"].clone();
    view
}

/// Publish a self-contained offline report from a validated comparison directory.
pub fn build_report(comparison: &Path, out: &Path) -> Result<()> {
    if fs::symlink_metadata(out).is_ok() {
        return Err(Error::new(
            "destination_exists",
            format!("report destination already exists: {}", out.display()),
        ));
    }
    let root = comparison.canonicalize()?;
    let document = safe_artifact_path(&root, "comparison.json")?;
    let snapshot = fs::read(&document)?;
    let comparison = load_json_document(&document)?;
    if comparison["schema_version"] != 1 {
        return Err(Error::new(
            "unsupported_schema",
            "report requires comparison schema_version 1",
        ));
    }
    validate_generated_document("comparison", &comparison, &root)?;
    let cases = comparison["cases"]
        .as_array()
        .ok_or_else(|| Error::new("invalid_document", "comparison must contain a cases array"))?;
    let records = normalized_artifacts(&comparison["artifacts"])?;
    let mut assets = Assets::new(&root, out, records)?;
    create_destination(out)?;
    fs::create_dir_all(out.join("artifacts"))?;
    for record in assets.records.clone() {
        assets.copy(&record)?;
    }
    fs::write(out.join("comparison.json"), &snapshot)?;
    assets.manifest.push(json!({"path":"comparison.json","source_path":"comparison.json","sha256":format!("{:x}",Sha256::digest(&snapshot)),"role":"comparison","media_type":"application/json","availability":"available"}));
    let ordered: Vec<_> = cases.iter().collect();
    if ordered.iter().any(|c| !c.is_object()) {
        return Err(Error::new(
            "invalid_document",
            "case result must be an object",
        ));
    }
    let mut html = format!(
        "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'; base-uri 'none'; form-action 'none'\"><title>SpektraFilm benchmark review</title><style>{CSS}</style><body><h1>SpektraFilm 人工审查</h1><p>选择场景查看 A/B；细节、差异图和完整数据按需展开。</p><nav class=\"case-nav\" aria-label=\"案例与种子\"><label>案例 / 种子 <select data-case-nav>"
    );
    for (index, case) in ordered.iter().enumerate() {
        html.push_str(&format!(
            "<option value=\"case-{index}\">{} · seed {} · {}</option>",
            escape(&human(get(case, "case_id", get(case, "id", "unavailable")))),
            field(case, "seed"),
            escape(get(case, "status", "unknown"))
        ));
    }
    html.push_str("</select></label><span class=\"small\">所有案例和失败状态均可访问</span></nav><p class=\"small\">");
    html.push_str(
        &[
            ("selected_cases", "案例"),
            ("completed_results", "完成"),
            ("failed_results", "失败"),
        ]
        .iter()
        .map(|(key, label)| {
            format!(
                "{label}：{}",
                comparison["summary"].get(key).map(text).unwrap_or_else(|| {
                    if comparison["mode"] != "single_run" {
                        return "未记录".into();
                    }
                    match *key {
                        "selected_cases" => cases
                            .iter()
                            .filter_map(|case| case["case_id"].as_str())
                            .collect::<BTreeSet<_>>()
                            .len()
                            .to_string(),
                        "completed_results" => cases
                            .iter()
                            .filter(|case| case["status"] == "completed")
                            .count()
                            .to_string(),
                        "failed_results" => cases
                            .iter()
                            .filter(|case| case["status"] == "failed")
                            .count()
                            .to_string(),
                        _ => "未记录".into(),
                    }
                })
            )
        })
        .collect::<Vec<_>>()
        .join(" · "),
    );
    html.push_str("</p><details class=\"supporting\"><summary>比较摘要与跨种子证据</summary><p>选中案例后，打开下方支持证据查看原始来源、协议、规则、聚合和 findings。</p>");
    for (key, label) in [
        ("summary", "汇总"),
        ("selection", "选择"),
        ("protocol", "协议"),
        ("condition_differences", "条件变化"),
        ("rules", "规则"),
        ("baseline", "基准来源"),
        ("candidate", "候选来源"),
        ("run", "运行来源"),
    ] {
        if comparison["mode"] == "single_run" && ["baseline", "candidate"].contains(&key) {
            continue;
        }
        html.push_str(&folded(
            label,
            comparison.get(key).unwrap_or(&json!("不可用")),
        ));
    }
    if !rows(&comparison["seed_measurements"]).is_empty() {
        html.push_str(&format!(
            "<details><summary>跨种子完整测量</summary>{}</details>",
            metric_table(&json!({"measurements":comparison["seed_measurements"]}))
        ));
    }
    html.push_str("</details>");
    for (index, case) in ordered.iter().enumerate() {
        let id = get(case, "case_id", get(case, "id", "unavailable"));
        html.push_str(&format!("<section id=\"case-{index}\" class=\"case-panel\" data-case-panel=\"case-{index}\"{}><h2>{} <small>· seed {}</small></h2>", if index == 0 { "" } else { " hidden" }, escape(&human(id)), field(case, "seed")));
        for (key, label) in [
            ("source_dimensions", "原始输入尺寸"),
            ("source_geometry", "输入解码与模拟几何"),
            ("preparation", "输入准备来源与操作记录"),
            ("input", "输入来源与文件身份"),
        ] {
            if let Some(value) = case.get(key).filter(|value| !value.is_null()) {
                html.push_str(&folded(label, value));
            }
        }
        let pairs = rows(&case["pairs"]);
        let views: Vec<Value> = if pairs.is_empty() {
            vec![(**case).clone()]
        } else {
            pairs.iter().map(|pair| pair_case(case, pair)).collect()
        };
        let default = get(&comparison, "default_pair", "input");
        if !pairs.is_empty() {
            html.push_str("<nav class=\"pair-nav\" aria-label=\"比较对象\"><label>比较对象 <select data-pair-nav>");
            for (pair_index, pair) in pairs.iter().enumerate() {
                html.push_str(&format!(
                    "<option value=\"pair-{index}-{pair_index}\"{}>{} / 模拟</option>",
                    if pair["id"] == default {
                        " selected"
                    } else {
                        ""
                    },
                    if pair["left_role"] == "input" {
                        "输入"
                    } else {
                        "对比图"
                    }
                ));
            }
            html.push_str("</select></label></nav>");
        }
        for (pair_index, case) in views.iter().enumerate() {
            let labels = if case.get("pair_id").is_some() {
                (
                    if case["left_role"] == "input" {
                        "输入"
                    } else {
                        "对比图"
                    },
                    "模拟",
                )
            } else {
                ("基准", "候选")
            };
            html.push_str(&format!("<div data-pair-panel=\"pair-{index}-{pair_index}\"{}><p class=\"status\">状态：{} {}</p>", if pairs.is_empty() || case["pair_id"] == default { "" } else { " hidden" }, escape(get(case, "status", "unavailable")), case.get("reason").map(text).unwrap_or_default()));
            let mut images = BTreeMap::new();
            for role in ["source", "baseline", "candidate", "reference", "difference"] {
                let reference = role_reference(case, role, &assets);
                let mut image = assets.image(&reference)?;
                if let Some(entry) = image.as_mut()
                    && let Some(preview) = entry.get("fit_preview").cloned()
                    && let Some(fit) = assets.image(&preview)?
                {
                    entry["fit_entry"] = fit;
                }
                images.insert(role, image);
            }
            let a = images["baseline"].as_ref();
            let b = images["candidate"].as_ref();
            if case.get("pair_id").is_none() {
                html.push_str(&change_summary(case, &comparison));
            } else {
                html.push_str(&folded("比较角色、对应关系与适用范围", case));
            }
            if case["pixel_aligned"] != false
                && let (Some(a), Some(b)) = (a, b)
            {
                let da = image::image_dimensions(out.join(get(a, "path", "")))
                    .map_err(|e| Error::new("invalid_artifact", e.to_string()))?;
                let db = image::image_dimensions(out.join(get(b, "path", "")))
                    .map_err(|e| Error::new("invalid_artifact", e.to_string()))?;
                if da == db {
                    html.push_str(&viewer(Some(a), Some(b), false, labels, true));
                } else {
                    html.push_str("<p class=\"unavailable\">图像尺寸不同，仅提供并列检查，未声明共同观察尺度。</p><div class=\"grid\">");
                    html.push_str(&viewer(Some(a), Some(b), false, labels, false));
                    html.push_str("</div>");
                }
            } else {
                html.push_str(
                    "<p class=\"unavailable\">同步 A/B 不可用：证据缺失或图像未对齐。</p>",
                );
                html.push_str(&viewer(a, b, false, labels, false));
            }
            html.push_str(&metric_summary(case, labels));
            let mut regions: Vec<(Value, String)> = rows(&case["regions"])
                .iter()
                .cloned()
                .map(|v| (v, "固定语义区域".into()))
                .collect();
            regions.extend(
                rows(
                    case.get("automatic_regions")
                        .or_else(|| case.get("hotspots"))
                        .unwrap_or(&Value::Null),
                )
                .iter()
                .cloned()
                .map(|v| (v, "自动差异区域".into())),
            );
            regions.extend(rows(&case["detail_regions"]).iter().cloned().map(|v| {
                let method =
                    get(&v, "selection_method", get(&v, "method", "保留的细节区域")).to_owned();
                (v, method)
            }));
            if !regions.is_empty() {
                html.push_str("<details><summary>细节区域（坐标、定位和 A/B 裁剪）</summary>");
            }
            for (mut region, method) in regions.iter().cloned() {
                let mut mask_entry = None;
                if let Some(path) = region["geometry"]["path"].as_str() {
                    if case["status"] != "completed" && !assets.index.contains_key(path) {
                        html.push_str(&format!(
                            "<p class=\"unavailable\">{}：渲染失败，区域掩膜证据不可用。</p>",
                            field(&region, "label")
                        ));
                        continue;
                    }
                    let entry = assets.copy(&json!(path))?;
                    if entry
                        .as_ref()
                        .is_none_or(|e| e["availability"] != "available")
                    {
                        html.push_str(&format!(
                            "<p class=\"unavailable\">{}：区域掩膜证据不可用。</p>",
                            field(&region, "label")
                        ));
                        continue;
                    }
                    let entry = entry.unwrap();
                    let (mask, w, h) = retained_mask(&out.join(get(&entry, "path", "")))?;
                    let mut xmin = w;
                    let mut ymin = h;
                    let mut xmax = 0;
                    let mut ymax = 0;
                    let mut count = 0;
                    for (i, v) in mask.iter().enumerate() {
                        if *v {
                            xmin = xmin.min(i % w);
                            ymin = ymin.min(i / w);
                            xmax = xmax.max(i % w);
                            ymax = ymax.max(i / w);
                            count += 1;
                        }
                    }
                    if count == 0 {
                        return Err(Error::new("region_bounds", "retained region mask is empty"));
                    }
                    region["rectangle"] = json!([xmin, ymin, xmax - xmin + 1, ymax - ymin + 1]);
                    mask_entry = Some(entry);
                }
                let rect = rectangle(&region)?;
                let region_id = get(&region, "id", get(&region, "region_id", "不可用")).to_owned();
                html.push_str(&format!("<details><summary>{}</summary><p>坐标 [x, y, width, height]：{} · 选择方式：{}</p>",escape(get(&region,"label",&human(&region_id))),rect.map(|v|text(&json!(v))).unwrap_or_else(||"不可用".into()),escape(&method)));
                html.push_str(&folded("选择依据与区域元数据", &region));
                if let Some(entry) = mask_entry {
                    html.push_str(&format!(
                        "<p>测量使用保留掩膜（含排除像素）：<a href=\"{}\">区域掩膜</a></p>",
                        url(&entry)
                    ));
                }
                for (image, geometry_key, label) in [
                    (a, "left_geometry", labels.0),
                    (b, "right_geometry", labels.1),
                ] {
                    let locator_rect = if let Some(geometry) = region.get(geometry_key) {
                        geometry_rectangle(geometry, &mut assets)?
                    } else if case["pixel_aligned"] != false {
                        rect
                    } else {
                        None
                    };
                    if let (Some(image), Some([x, y, w, h])) = (image, locator_rect) {
                        let (width, height) =
                            image::image_dimensions(out.join(get(image, "path", "")))
                                .map_err(|e| Error::new("invalid_artifact", e.to_string()))?;
                        html.push_str(&format!("<figure class=\"locator\"><img src=\"{}\" alt=\"{}区域定位\"><span class=\"rectangle\" style=\"left:{}%;top:{}%;width:{}%;height:{}%\"></span><figcaption>{} · {}</figcaption></figure>",url(image),escape(label),100.0*x as f64/width as f64,100.0*y as f64/height as f64,100.0*w as f64/width as f64,100.0*h as f64/height as f64,escape(label),text(&json!([x,y,w,h]))));
                    } else {
                        html.push_str(&format!(
                            "<div class=\"unavailable\">{}区域定位：不可用</div>",
                            escape(label)
                        ));
                    }
                }
                let mut crops = vec![];
                for (role, image) in [("baseline", a), ("candidate", b)] {
                    if case.get("pair_id").is_some() {
                        region["pair_id"] = case["pair_id"].clone();
                    }
                    let reference = role_reference(&region, role, &assets);
                    let retained = assets.image(&reference)?.filter(|e| {
                        !matches!(
                            Path::new(get(e, "path", ""))
                                .extension()
                                .and_then(|v| v.to_str()),
                            Some("jpg" | "jpeg")
                        )
                    });
                    crops.push(if retained.is_some() {
                        retained
                    } else {
                        let geometry_key = if role == "baseline" {
                            "left_geometry"
                        } else {
                            "right_geometry"
                        };
                        let crop_rect = if let Some(geometry) = region.get(geometry_key) {
                            geometry_rectangle(geometry, &mut assets)?
                        } else if case["pixel_aligned"] != false {
                            rect
                        } else {
                            None
                        };
                        assets.crop(image, crop_rect)?
                    });
                }
                html.push_str(&viewer(
                    crops[0].as_ref(),
                    crops[1].as_ref(),
                    true,
                    labels,
                    case["pixel_aligned"] != false,
                ));
                let findings: Vec<_> = rows(&comparison["findings"])
                    .iter()
                    .filter(|f| {
                        f["case_id"] == id
                            && f["region_id"] == region_id
                            && (case.get("pair_id").is_none() || f["pair_id"] == case["pair_id"])
                    })
                    .cloned()
                    .collect();
                html.push_str(&folded("该区域 findings", &json!(findings)));
                html.push_str("</details>");
            }
            if !regions.is_empty() {
                html.push_str("</details>");
            }
            html.push_str("<details><summary>来源输入与参考图</summary><div class=\"grid\">");
            for (role, label) in [("source", "来源输入"), ("reference", "参考图")] {
                html.push_str(&image_html(images[role].as_ref(), label));
            }
            html.push_str("</div>");
            if images["reference"].is_some() || case.get("reference").is_some() {
                html.push_str(&format!(
                    "<pre>{}</pre>",
                    case.get("reference")
                        .map(text)
                        .unwrap_or_else(|| "参考范围和来源限制不可用".into())
                ));
            }
            html.push_str("</details><details><summary>差异图与显示元数据</summary>");
            html.push_str(&image_html(images["difference"].as_ref(), "差异图"));
            let mut metadata = json!({});
            for key in [
                "metric",
                "units",
                "scale",
                "clipping",
                "amplification",
                "palette",
            ] {
                metadata[key] = images["difference"]
                    .as_ref()
                    .and_then(|e| e.get(key))
                    .cloned()
                    .unwrap_or(json!("不可用"));
            }
            html.push_str(&format!("<pre>{}</pre></details>", text(&metadata)));
            let plot_roles = [
                "tone_response",
                "grain_radial_spectrum",
                "halation_profile",
                "halation_edge_spill_profile",
                "edge_response",
                "edge_sfr",
                "reference_material_curve",
                "reference_regional_statistics",
            ];
            let mut plots = String::new();
            for artifact in rows(&case["artifacts"]) {
                if plot_roles.contains(&get(artifact, "role", "")) {
                    let label = get(artifact, "id", get(artifact, "role", ""));
                    if let Some(plot) = assets.plot(artifact, label)?
                        && let Some(entry) = assets.copy(artifact)?
                    {
                        plots.push_str(&format!("<details><summary>{}</summary>{plot}<a href=\"{}\">保留的数值样本</a></details>",escape(&human(label)),url(&entry)));
                    }
                }
            }
            if !plots.is_empty() {
                html.push_str(&format!(
                    "<details><summary>曲线与频谱证据</summary>{plots}</details>"
                ));
            }
            for (role, label) in if case.get("pair_id").is_some() {
                [("left", labels.0), ("right", labels.1)]
            } else {
                [("baseline", labels.0), ("candidate", labels.1)]
            } {
                let entry =
                    assets.copy(case.get(format!("{role}_resolved")).unwrap_or(&Value::Null))?;
                html.push_str(&format!(
                    "<details><summary>{label}执行与渲染配置</summary><pre>{}</pre>",
                    case.get(format!("{role}_execution"))
                        .map(text)
                        .unwrap_or_else(|| "不可用".into())
                ));
                if let Some(entry) = entry.filter(|e| e["availability"] == "available") {
                    html.push_str(&format!("<a href=\"{}\">已解析配置</a>", url(&entry)));
                }
                html.push_str("</details>");
            }
            html.push_str("<details><summary>完整测量（原样保留）</summary>");
            html.push_str(&metric_table(case));
            for (key, label) in [
                ("curves", "Retained response curves"),
                ("spectra", "Retained spectra"),
            ] {
                if let Some(value) = case.get(key) {
                    html.push_str(&format!("<h4>{label}</h4><pre>{}</pre>", text(value)));
                }
            }
            html.push_str("</details></div>");
        }
        html.push_str("</section>");
    }
    let omissions: Vec<_> = assets
        .manifest
        .iter()
        .filter(|entry| entry["availability"] == "unavailable")
        .cloned()
        .collect();
    if !omissions.is_empty() {
        html.push_str(&folded(
            "不可用与禁止再分发的证据（原样保留）",
            &json!(omissions),
        ));
    }
    html.push_str(&folded(
        "全部 findings（原样保留）",
        comparison.get("findings").unwrap_or(&json!([])),
    ));
    html.push_str(&format!("<p><a href=\"comparison.json\">比较快照</a> · <a href=\"report.json\">相对 artifact manifest</a></p><script>{JS}</script></body></html>"));
    fs::write(out.join("index.html"), html)?;
    assets.manifest.push(json!({"path":"index.html","sha256":sha256_file(&out.join("index.html"))?,"role":"report","media_type":"text/html","availability":"available"}));
    dump(
        &out.join("report.json"),
        &json!({"schema_version":1,"command":"report","execution_status":"completed","comparison_id":comparison.get("id"),"artifact":"index.html","artifacts":assets.manifest}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redistribution_denial_never_copies_bytes() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::write(source.path().join("private.png"), b"private").unwrap();
        let mut assets = Assets::new(
            source.path(),
            out.path(),
            vec![json!({"id":"private","path":"private.png","redistribution":false})],
        )
        .unwrap();
        let entry = assets.copy(&json!("private")).unwrap().unwrap();
        assert_eq!(entry["reason"], "redistribution_not_permitted");
        assert!(!out.path().join("private.png").exists());
    }
    #[test]
    fn hash_mismatch_is_rejected_before_publication() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::write(source.path().join("data.json"), b"{}").unwrap();
        let mut assets = Assets::new(
            source.path(),
            out.path(),
            vec![json!({"path":"data.json","sha256":"incorrect"})],
        )
        .unwrap();
        assert_eq!(
            assets.copy(&json!("data.json")).unwrap_err().code,
            "artifact_hash"
        );
        assert!(!out.path().join("data.json").exists());
    }
    #[test]
    fn png16_crops_preserve_every_integer_code() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::create_dir(out.path().join("artifacts")).unwrap();
        let codes = [
            [1, 257, 65535],
            [65534, 32769, 17],
            [19, 20, 21],
            [22, 23, 24],
        ];
        crate::image_data::write_png16_codes(&source.path().join("full.png"), &codes, 2, 2)
            .unwrap();
        let mut assets = Assets::new(
            source.path(),
            out.path(),
            vec![json!({"id":"full","path":"full.png","bit_depth":16})],
        )
        .unwrap();
        let full = assets.image(&json!("full")).unwrap().unwrap();
        let crop = assets
            .crop(Some(&full), Some([1, 0, 1, 2]))
            .unwrap()
            .unwrap();
        let image = image::open(out.path().join(crop["path"].as_str().unwrap()))
            .unwrap()
            .to_rgb16();
        assert_eq!(image.get_pixel(0, 0).0, codes[1]);
        assert_eq!(image.get_pixel(0, 1).0, codes[3]);
    }
    #[test]
    fn single_pair_missing_assets_never_fall_back_to_other_pair() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::write(source.path().join("wrong.png"), b"another pair").unwrap();
        let mut assets = Assets::new(
            source.path(),
            out.path(),
            vec![
                json!({"id":"wrong","path":"wrong.png","role":"baseline_review","case_id":"local"}),
            ],
        )
        .unwrap();
        let case = pair_case(
            &json!({"case_id":"local","seed":0,"baseline_review":"wrong"}),
            &json!({"id":"reference","review":{"left":"missing.png"}}),
        );
        let reference = role_reference(&case, "baseline", &assets);
        assert_eq!(reference, "missing.png");
        assert!(assets.image(&reference).unwrap().is_none());
        assert_eq!(assets.manifest[0]["reason"], "missing_artifact");
        assert!(role_reference(&case, "candidate", &assets).is_null());
        assert!(role_reference(&case, "difference", &assets).is_null());
    }

    #[test]
    fn corresponding_region_masks_keep_independent_crop_geometry() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::create_dir(out.path().join("artifacts")).unwrap();
        crate::image_data::write_npy(
            &source.path().join("mask.npy"),
            &[0.0, 1.0, 1.0, 0.0, 0.0, 0.0],
            &[2, 3],
        )
        .unwrap();
        let mut assets =
            Assets::new(source.path(), out.path(), vec![json!({"path":"mask.npy"})]).unwrap();
        assert_eq!(
            geometry_rectangle(&json!({"path":"mask.npy"}), &mut assets).unwrap(),
            Some([1, 0, 2, 1])
        );
        assert_eq!(
            geometry_rectangle(&json!([0, 1, 1, 1]), &mut assets).unwrap(),
            Some([0, 1, 1, 1])
        );
    }

    #[test]
    fn reference_difference_display_keeps_valid_mask_exclusions() {
        let source = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        fs::create_dir(out.path().join("artifacts")).unwrap();
        crate::image_data::write_npy(&source.path().join("map.npy"), &[1.0, 2.0], &[1, 2]).unwrap();
        crate::image_data::write_npy(&source.path().join("mask.npy"), &[1.0, 0.0], &[1, 2])
            .unwrap();
        let mut assets = Assets::new(source.path(), out.path(), vec![json!({"id":"map","path":"map.npy","role":"difference_map","metric":"delta_e00","valid_mask_artifact_id":"mask"}),json!({"id":"mask","path":"mask.npy"})]).unwrap();
        let display = assets.image(&json!("map")).unwrap().unwrap();
        let pixels = image::open(out.path().join(display["path"].as_str().unwrap()))
            .unwrap()
            .to_rgba8();
        assert_eq!(pixels.get_pixel(0, 0)[3], 255);
        assert_eq!(pixels.get_pixel(1, 0)[3], 0);
    }

    #[test]
    fn malformed_rectangles_are_errors() {
        assert_eq!(
            rectangle(&json!({"geometry":[0,0,0,10]})).unwrap_err().code,
            "region_bounds"
        );
        assert!(rectangle(&json!({"geometry":[1,2,3,4]})).unwrap().is_some());
    }
}
