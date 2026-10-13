use crate::common::{self, Availability, Error, EvidenceBuilder, Result};
use crate::image_data::ImageData;
use crate::{documents, image_data};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const METRIC_VERSION: &str = "spektrafilm-reference-v1";
const DOMAIN: &str = "bounded_srgb_d65_lab";
const COLOR_TRANSFORM: &str = "srgb-d65-cie1931-2deg-bounded-v1";
const SIDES: [&str; 3] = ["baseline_distance", "candidate_distance", "distance_delta"];

fn err(code: &str, message: impl Into<String>) -> Error {
    Error::new(code, message.into())
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.get(key)
        .ok_or_else(|| err("invalid_reference_evidence", format!("Missing {key}")))
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    field(v, key)?.as_str().ok_or_else(|| {
        err(
            "invalid_reference_evidence",
            format!("{key} must be a string"),
        )
    })
}
fn optional_array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.),
    }
}
fn path_of(value: &Value, base: &Path) -> Result<PathBuf> {
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
fn read_json(value: &Value, base: &Path) -> Result<Value> {
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
fn read_reference(reference: &Value, base: &Path) -> Result<ImageData> {
    let mut color = reference.get("color").cloned().unwrap_or(json!({}));
    if color.get("icc").and_then(|v| v.get("path")).is_some() {
        color["icc"]["path"] = json!(path_of(&color["icc"], base)?);
    }
    image_data::load_image(&json!({"path":path_of(field(reference,"path")?,base)?,"color":color}))
}
fn mask(geometry: &Value, dims: [usize; 2], base: &Path) -> Result<Vec<bool>> {
    documents::reference_geometry_mask(geometry, dims[0], dims[1], base)
}
fn finite_rgb(v: &[[f64; 3]], code: &str, message: &str) -> Result<()> {
    if v.iter().flatten().any(|x| !x.is_finite()) {
        Err(err(code, message))
    } else {
        Ok(())
    }
}
fn clipped_lab(v: &[[f64; 3]]) -> Vec<[f64; 3]> {
    let bounded: Vec<[f64; 3]> = v.iter().map(|p| p.map(|x| x.clamp(0., 1.))).collect();
    image_data::rgb_to_lab(&bounded)
}
fn clipping_fraction(v: &[[f64; 3]]) -> f64 {
    v.iter().flatten().filter(|x| **x < 0. || **x > 1.).count() as f64 / (v.len() * 3) as f64
}
fn percentile(mut values: Vec<f64>, q: f64) -> f64 {
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
fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}
fn population(mask: &[bool]) -> Vec<usize> {
    mask.iter()
        .enumerate()
        .filter_map(|(i, b)| b.then_some(i))
        .collect()
}
fn processing_eligibility(reference: &Value) -> Result<(bool, bool)> {
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

#[derive(Clone, Copy)]
enum ArtifactRole {
    Native,
    AlignedDisplay,
    Aligned,
    CommonArea,
    BaselineMaps,
    CandidateMaps,
    RegionalStatistics,
    MaterialSource,
    MaterialCurve,
}
impl ArtifactRole {
    fn name(self) -> &'static str {
        match self {
            Self::Native => "reference_native",
            Self::AlignedDisplay => "reference_aligned_display",
            Self::Aligned => "reference_aligned",
            Self::CommonArea => "reference_common_area_mask",
            Self::BaselineMaps => "reference_baseline_distance_maps",
            Self::CandidateMaps => "reference_candidate_distance_maps",
            Self::RegionalStatistics => "reference_regional_statistics",
            Self::MaterialSource => "reference_material_source",
            Self::MaterialCurve => "reference_material_curve",
        }
    }
}
/// Reference measurements extend the shared row with the frozen protocol and
/// reference identity; `serde(flatten)` merges the shared keys into the same
/// object, so the serialized key set is unchanged.
#[derive(Serialize)]
struct Measurement {
    case_id: String,
    seed_population: Value,
    #[serde(flatten)]
    shared: common::Measurement,
    reference_id: String,
    protocol_id: Value,
}
struct Comparison<'a> {
    reference: &'a Value,
    case_id: String,
    seeds: Value,
    protocol_id: Value,
    prefix: String,
    out: &'a Path,
    sequence: usize,
    body: EvidenceBuilder<Measurement, Value, Value>,
    review: Value,
    single: bool,
    protocol: Value,
}
impl<'a> Comparison<'a> {
    fn new(reference: &'a Value, case: &Value, protocol: &Value, out: &'a Path) -> Result<Self> {
        let case_id = text(case, "id")?.to_owned();
        let seeds = protocol
            .get("seed_population")
            .cloned()
            .unwrap_or_else(|| case.get("seeds").cloned().unwrap_or(json!([])));
        let hash = format!("{:x}", Sha256::digest(common::canonical_json(&seeds)?));
        let prefix = format!("{case_id}:reference:{}", &hash[..12]);
        let review = json!({"id":field(reference,"id")?,"scope":field(reference,"scope")?,"source_sha256":reference.get("sha256"),"provenance":reference.get("provenance").cloned().unwrap_or(json!({})),"processing":reference.get("processing").cloned().unwrap_or(json!({})),"color":reference.get("color").cloned().unwrap_or(json!({})),"limitations":["Distance describes the selected reference, not physical film accuracy.","Local research access does not grant redistribution of originals or derivatives."],"geometry":"side_by_side","artifact_refs":[],"split":case.get("split"),"split_group":case.get("split_group"),"color_transform":COLOR_TRANSFORM});
        Ok(Self {
            reference,
            case_id,
            seeds,
            protocol_id: protocol.get("id").cloned().unwrap_or(Value::Null),
            prefix,
            out,
            sequence: 0,
            body: EvidenceBuilder::default(),
            review,
            single: false,
            protocol: protocol.clone(),
        })
    }
    fn limitation(&mut self, message: impl Into<String>) {
        self.review["limitations"]
            .as_array_mut()
            .unwrap()
            .push(json!(message.into()));
    }
    fn review_ref(&mut self, id: &str) {
        self.review["artifact_refs"]
            .as_array_mut()
            .unwrap()
            .push(json!(id));
    }
    fn finding(
        &mut self,
        region: &str,
        metric: &str,
        reason: &str,
        failure: bool,
        refs: &[String],
    ) {
        self.body.findings.push(json!({"id":format!("{}:{region}:{metric}:{reason}",self.prefix),"case_id":self.case_id,"region_id":region,"metric":metric,"decision":"unavailable","code":reason,"evidence":refs}));
        if failure {
            self.body.mark_failed();
        }
    }
    // Keep metric fields explicit: each argument is part of the frozen measurement payload.
    #[allow(clippy::too_many_arguments)]
    fn distances(
        &mut self,
        region: &str,
        metric: &str,
        domain: &str,
        units: &str,
        estimator: &str,
        count: usize,
        values: Option<Vec<f64>>,
        reason: Option<&str>,
        refs: &[String],
    ) {
        let mut reason = reason.map(str::to_owned);
        let mut expanded = None;
        if let Some(v) = values {
            if !v.iter().all(|x| x.is_finite()) {
                reason = Some("computation_failure".into());
                self.finding(region, metric, "computation_failure", true, refs);
            }
            if reason.is_none() {
                expanded = Some(if self.single {
                    v
                } else {
                    vec![v[0], v[1], v[1] - v[0]]
                });
            }
        }
        if expanded
            .as_ref()
            .is_some_and(|v| v.iter().any(|x| !x.is_finite()))
        {
            reason = Some("computation_failure".into());
            expanded = None;
            self.finding(region, metric, "computation_failure", true, refs);
        }
        let sides: &[&str] = if self.single {
            &["simulation_distance"]
        } else {
            &SIDES
        };
        for (i, side) in sides.iter().enumerate() {
            self.body.measurements.push(Measurement {
                case_id: self.case_id.clone(),
                seed_population: self.seeds.clone(),
                shared: common::Measurement {
                    region_id: region.into(),
                    metric: format!("{metric}.{side}"),
                    metric_version: METRIC_VERSION,
                    domain: domain.into(),
                    units: units.into(),
                    estimator: estimator.into(),
                    sample_count: count,
                    artifact_refs: refs.to_vec(),
                    availability: if expanded.is_some() {
                        Availability::Available
                    } else {
                        Availability::Unavailable
                    },
                    value: expanded.as_ref().map(|v| v[i]),
                    reason: if expanded.is_some() {
                        None
                    } else {
                        Some(reason.clone().unwrap_or("missing_evidence".into()))
                    },
                },
                reference_id: self.reference["id"].as_str().unwrap_or("").into(),
                protocol_id: self.protocol_id.clone(),
            });
        }
        if let Some(v) = expanded {
            if self.single {
                self.body.findings.push(json!({"id":format!("{}:{region}:{metric}:measured",self.prefix),"case_id":self.case_id,"region_id":region,"metric":format!("{metric}.simulation_distance"),"decision":"measured","evidence":refs,"simulation_distance":v[0],"interpretation":"Simulation distance to the selected reference; no acceptance threshold is inferred."}));
            } else {
                self.body.findings.push(json!({"id":format!("{}:{region}:{metric}:measured",self.prefix),"case_id":self.case_id,"region_id":region,"metric":format!("{metric}.distance_delta"),"decision":"measured","evidence":refs,"baseline_distance":v[0],"candidate_distance":v[1],"distance_delta":v[2],"interpretation":"Negative distance_delta means closer to this reference; no acceptance threshold is inferred."}));
            }
        } else {
            self.finding(
                region,
                metric,
                reason.as_deref().unwrap_or("missing_evidence"),
                false,
                refs,
            );
        }
    }
    fn artifact_row(&self, id: &str, path: &Path, role: &str, media: &str) -> Result<Value> {
        Ok(
            json!({"id":id,"path":path.file_name().unwrap().to_string_lossy(),"sha256":common::sha256_file(path)?,"media_type":media,"role":role,"reference_id":self.reference["id"],"provenance":self.reference.get("provenance").cloned().unwrap_or(json!({})),"redistribution":self.reference.get("provenance").and_then(|x|x.get("redistribution")).cloned().unwrap_or(json!("unknown"))}),
        )
    }
    fn next_artifact(&mut self, role: ArtifactRole, extension: &str) -> (String, PathBuf) {
        self.sequence += 1;
        (
            format!("{}:{}", self.prefix, self.sequence),
            self.out.join(format!(
                "reference-{:03}-{}.{}",
                self.sequence,
                role.name(),
                extension
            )),
        )
    }
    fn json_artifact(&mut self, role: ArtifactRole, payload: &Value) -> Result<String> {
        let (id, path) = self.next_artifact(role, "json");
        common::dump(&path, payload)?;
        self.body
            .artifacts
            .push(self.artifact_row(&id, &path, role.name(), "application/json")?);
        Ok(id)
    }
    fn numeric_artifact(
        &mut self,
        role: ArtifactRole,
        values: &[f64],
        shape: &[usize],
        domain: &str,
        units: Option<&str>,
    ) -> Result<String> {
        let (id, path) = self.next_artifact(role, "npy");
        image_data::write_npy(&path, values, shape)?;
        let mut row = self.artifact_row(&id, &path, role.name(), "application/x-npy")?;
        array_metadata(&mut row, shape, "float64");
        row["domain"] = json!(domain);
        if let Some(u) = units {
            row["units"] = json!(u);
        }
        self.body.artifacts.push(row);
        Ok(id)
    }
    fn rgb_artifact(
        &mut self,
        role: ArtifactRole,
        values: &[[f64; 3]],
        dims: [usize; 2],
        domain: &str,
        display: bool,
    ) -> Result<String> {
        if display {
            let (id, path) = self.next_artifact(role, "png");
            save_display(&path, values, dims)?;
            let mut row = self.artifact_row(&id, &path, role.name(), "image/png")?;
            array_metadata(&mut row, &[dims[1], dims[0], 3], "float64");
            row["domain"] = json!(domain);
            row["width"] = json!(dims[0]);
            row["height"] = json!(dims[1]);
            row["bit_depth"] = json!(16);
            row["quantization"] = json!("16-bit rounded; no dither");
            self.body.artifacts.push(row);
            Ok(id)
        } else {
            self.numeric_artifact(
                role,
                &values.iter().flatten().copied().collect::<Vec<_>>(),
                &[dims[1], dims[0], 3],
                domain,
                None,
            )
        }
    }
    fn mask_artifact(&mut self, values: &[bool], dims: [usize; 2]) -> Result<String> {
        let (id, path) = self.next_artifact(ArtifactRole::CommonArea, "npy");
        write_bool_npy(&path, values, &[dims[1], dims[0]])?;
        let mut row = self.artifact_row(
            &id,
            &path,
            ArtifactRole::CommonArea.name(),
            "application/x-npy",
        )?;
        array_metadata(&mut row, &[dims[1], dims[0]], "bool");
        row["domain"] = json!("render_pixel_coordinates");
        self.body.artifacts.push(row);
        Ok(id)
    }
    fn preview(
        &mut self,
        values: &[[f64; 3]],
        dims: [usize; 2],
        color_available: bool,
    ) -> Result<String> {
        let path = self.out.join("reference-display.png");
        save_display(&path, values, dims)?;
        let id = format!("{}:display", self.prefix);
        let mut row = self.artifact_row(&id, &path, "reference_display", "image/png")?;
        row["bit_depth"] = json!(16);
        row["quantization"] = json!("16-bit rounded; no dither");
        row["width"] = json!(dims[0]);
        row["height"] = json!(dims[1]);
        row["color_available"] = json!(color_available);
        self.body.artifacts.push(row);
        Ok(id)
    }
    fn finish(self) -> Value {
        let mut extra = BTreeMap::new();
        extra.insert("metric_version".into(), json!(METRIC_VERSION));
        extra.insert("reference_review".into(), self.review);
        self.body.finish(extra)
    }
}
fn array_metadata(row: &mut Value, shape: &[usize], dtype: &str) {
    row["layout"] = json!({"dtype":dtype,"axis_order":if shape.len()==3{"height,width,channel"}else{"height,width"}});
    row["dimensions"] = json!(shape);
    if shape.len() >= 2 {
        row["coordinates"] = json!({"system":"pixel_centers","x":{"start":0.5,"step":1,"count":shape[1]},"y":{"start":0.5,"step":1,"count":shape[0]}});
    }
}
fn save_display(path: &Path, values: &[[f64; 3]], dims: [usize; 2]) -> Result<()> {
    let codes: Vec<[u16; 3]> = values
        .iter()
        .map(|p| p.map(|x| (x.clamp(0., 1.) * 65535.).round_ties_even() as u16))
        .collect();
    image_data::write_png16_codes(path, &codes, dims[0], dims[1])
}
fn write_bool_npy(path: &Path, values: &[bool], shape: &[usize]) -> Result<()> {
    let shape = shape
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let header = format!("{{'descr': '|b1', 'fortran_order': False, 'shape': ({shape}), }}");
    let padding = (64 - (10 + header.len() + 1) % 64) % 64;
    let header = format!("{header}{}\n", " ".repeat(padding));
    let mut f = fs::File::create(path)?;
    f.write_all(b"\x93NUMPY\x01\x00")?;
    f.write_all(&(header.len() as u16).to_le_bytes())?;
    f.write_all(header.as_bytes())?;
    f.write_all(&values.iter().map(|b| u8::from(*b)).collect::<Vec<_>>())?;
    Ok(())
}

#[derive(Clone, Copy)]
enum Interpolation {
    Nearest,
    Bilinear,
}
struct FrozenTransform {
    inverse: [[f64; 3]; 3],
    interpolation: Interpolation,
}
impl FrozenTransform {
    fn parse(value: &Value) -> Result<Self> {
        if value.get("direction").and_then(Value::as_str) != Some("reference_to_render")
            || value.get("coordinate_system").and_then(Value::as_str) != Some("pixel_centers")
        {
            return Err(err(
                "invalid_correspondence",
                "Transform must declare direction, pixel centers and interpolation",
            ));
        }
        let interpolation = match value.get("interpolation").and_then(Value::as_str) {
            Some("nearest") => Interpolation::Nearest,
            Some("bilinear") => Interpolation::Bilinear,
            _ => {
                return Err(err(
                    "invalid_correspondence",
                    "Transform must declare direction, pixel centers and interpolation",
                ));
            }
        };
        let a = value
            .get("matrix")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                err(
                    "invalid_correspondence",
                    "Transform matrix must be finite and invertible 3 x 3",
                )
            })?;
        if a.len() != 3 || a.iter().any(|r| r.as_array().is_none_or(|a| a.len() != 3)) {
            return Err(err(
                "invalid_correspondence",
                "Transform matrix must be finite and invertible 3 x 3",
            ));
        }
        let m: [[f64; 3]; 3] =
            std::array::from_fn(|i| std::array::from_fn(|j| a[i][j].as_f64().unwrap_or(f64::NAN)));
        let d = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        if m.iter().flatten().any(|x| !x.is_finite()) || !d.is_finite() || d.abs() < 1e-12 {
            return Err(err(
                "invalid_correspondence",
                "Transform matrix must be finite and invertible 3 x 3",
            ));
        }
        let inverse = [
            [
                (m[1][1] * m[2][2] - m[1][2] * m[2][1]) / d,
                (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / d,
                (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / d,
            ],
            [
                (m[1][2] * m[2][0] - m[1][0] * m[2][2]) / d,
                (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / d,
                (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / d,
            ],
            [
                (m[1][0] * m[2][1] - m[1][1] * m[2][0]) / d,
                (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / d,
                (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / d,
            ],
        ];
        Ok(Self {
            inverse,
            interpolation,
        })
    }
    fn warp(
        &self,
        values: &[[f64; 3]],
        source: [usize; 2],
        target: [usize; 2],
    ) -> Result<(Vec<[f64; 3]>, Vec<bool>)> {
        let [sw, sh] = source;
        let [w, h] = target;
        if sw == 0 || sh == 0 || values.len() != sw * sh {
            return Err(err(
                "invalid_correspondence",
                "Reference samples and dimensions differ",
            ));
        }
        let m = self.inverse;
        let mut warped = Vec::with_capacity(w * h);
        let mut support = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let px = x as f64 + 0.5;
                let py = y as f64 + 0.5;
                let q = [
                    m[0][0] * px + m[0][1] * py + m[0][2],
                    m[1][0] * px + m[1][1] * py + m[1][2],
                    m[2][0] * px + m[2][1] * py + m[2][2],
                ];
                let divisor_good = q[2].is_finite() && q[2].abs() > 1e-12;
                let divisor = if divisor_good { q[2] } else { 1. };
                let sx = q[0] / divisor - 0.5;
                let sy = q[1] / divisor - 0.5;
                let good = divisor_good
                    && sx.is_finite()
                    && sy.is_finite()
                    && sx >= 0.
                    && sy >= 0.
                    && sx <= (sw - 1) as f64
                    && sy <= (sh - 1) as f64;
                let sx = if good { sx } else { 0. };
                let sy = if good { sy } else { 0. };
                support.push(good);
                let p = match self.interpolation {
                    Interpolation::Nearest => {
                        values[(sy + 0.5).floor() as usize * sw + (sx + 0.5).floor() as usize]
                    }
                    Interpolation::Bilinear => {
                        let x0 = sx.floor() as usize;
                        let y0 = sy.floor() as usize;
                        let x1 = (x0 + 1).min(sw - 1);
                        let y1 = (y0 + 1).min(sh - 1);
                        let fx = sx - x0 as f64;
                        let fy = sy - y0 as f64;
                        std::array::from_fn(|c| {
                            (1. - fy)
                                * ((1. - fx) * values[y0 * sw + x0][c]
                                    + fx * values[y0 * sw + x1][c])
                                + fy * ((1. - fx) * values[y1 * sw + x0][c]
                                    + fx * values[y1 * sw + x1][c])
                        })
                    }
                };
                warped.push(p);
            }
        }
        Ok((warped, support))
    }
}

pub(crate) fn validate_correspondence_documents(reference: &Value, base: &Path) -> Result<()> {
    if reference["scope"] != "aligned_scene" {
        return Ok(());
    }
    if reference.get("transform").is_some_and(truthy)
        == reference.get("aligned_derivative").is_some_and(truthy)
    {
        return Err(err(
            "invalid_correspondence",
            "Provide exactly one frozen transform or aligned derivative",
        ));
    }
    if let Some(transform) = reference.get("transform") {
        FrozenTransform::parse(&read_json(transform, base)?)?;
    }
    let review = read_json(field(reference, "correspondence_review")?, base)?;
    if review["status"] != "approved"
        || review["reviewer"]
            .as_str()
            .is_none_or(|v| v.trim().is_empty())
        || !review["limitations"].is_array()
        || !review["excluded_regions"].is_array()
    {
        return Err(err(
            "invalid_correspondence",
            "Correspondence requires approved reviewer, exclusions and limitations",
        ));
    }
    for exclusion in review["excluded_regions"].as_array().unwrap() {
        if !["motion", "occlusion", "parallax", "invalid_border"]
            .contains(&exclusion["reason"].as_str().unwrap_or(""))
            || !exclusion
                .get("geometry")
                .is_some_and(|v| v.is_object() || v.is_array())
        {
            return Err(err(
                "invalid_correspondence",
                "Correspondence exclusion requires a supported reason and geometry",
            ));
        }
    }
    Ok(())
}

// Keep source/reference arrays and frozen comparison controls explicit at this boundary.
#[allow(clippy::too_many_arguments)]
fn aligned(
    c: &mut Comparison,
    source: &ImageData,
    encoded: &[[f64; 3]],
    labs: &[Vec<[f64; 3]>],
    rendered: &[Vec<[f64; 3]>],
    dims: [usize; 2],
    case: &Value,
    base: &Path,
    tone: bool,
    chroma: bool,
) -> Result<()> {
    let reference = c.reference;
    let has_transform = reference.get("transform").is_some_and(truthy);
    let has_derivative = reference.get("aligned_derivative").is_some_and(truthy);
    if has_transform == has_derivative {
        return Err(err(
            "invalid_correspondence",
            "Provide exactly one frozen transform or aligned derivative",
        ));
    }
    if !reference.get("correspondence_review").is_some_and(truthy)
        || !reference.get("common_area_mask").is_some_and(truthy)
    {
        return Err(err(
            "invalid_correspondence",
            "Pixel comparison requires reviewed correspondence and common-area mask",
        ));
    }
    let record = read_json(&reference["correspondence_review"], base)?;
    if record.get("status").and_then(Value::as_str) != Some("approved")
        || record
            .get("reviewer")
            .and_then(Value::as_str)
            .is_none_or(|x| x.trim().is_empty())
    {
        return Err(err(
            "invalid_correspondence",
            "Correspondence must be approved by an identified reviewer",
        ));
    }
    let excluded = record
        .get("excluded_regions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            err(
                "invalid_correspondence",
                "Review must record exclusions and limitations explicitly",
            )
        })?;
    let limitations = record
        .get("limitations")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            err(
                "invalid_correspondence",
                "Review must record exclusions and limitations explicitly",
            )
        })?;
    let (encoded, support) = if has_derivative {
        let mut declaration = reference.clone();
        declaration["path"] = field(&reference["aligned_derivative"], "path")?.clone();
        let aligned = read_reference(&declaration, base)?;
        if aligned.dimensions() != dims {
            return Err(err(
                "invalid_correspondence",
                "Aligned derivative has incorrect geometry",
            ));
        }
        let rgb = image_data::display_rgb(&aligned)?;
        c.review["aligned_display_clipped_sample_fraction"] = json!(clipping_fraction(&rgb));
        (rgb, vec![true; dims[0] * dims[1]])
    } else {
        FrozenTransform::parse(&read_json(&reference["transform"], base)?)?.warp(
            encoded,
            source.dimensions(),
            dims,
        )?
    };
    finite_rgb(
        &encoded,
        "computation_failure",
        "Aligned reference has nonfinite samples",
    )?;
    let valid = mask(&reference["common_area_mask"], dims, base)?;
    if valid.iter().zip(&support).any(|(v, s)| *v && !*s) {
        return Err(err(
            "invalid_correspondence",
            "Common-area mask includes unsupported transform borders",
        ));
    }
    for exclusion in excluded {
        if !["motion", "occlusion", "parallax", "invalid_border"].contains(
            &exclusion
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ) {
            return Err(err(
                "invalid_correspondence",
                "Unsupported correspondence exclusion reason",
            ));
        }
        let excluded = mask(field(exclusion, "geometry")?, dims, base)?;
        if valid.iter().zip(excluded).any(|(v, e)| *v && e) {
            return Err(err(
                "invalid_correspondence",
                "Common-area mask retains reviewed invalid correspondence",
            ));
        }
    }
    if !valid.iter().any(|v| *v) {
        return Err(err(
            "invalid_correspondence",
            "Common-area mask has no eligible pixels",
        ));
    }
    let ref_lab = clipped_lab(&encoded);
    let display_id = c.rgb_artifact(
        ArtifactRole::AlignedDisplay,
        &encoded,
        dims,
        "bounded_encoded_srgb",
        true,
    )?;
    c.review["display_artifact_id"] = json!(display_id);
    c.review_ref(&display_id);
    let reference_id = c.rgb_artifact(
        ArtifactRole::Aligned,
        &encoded,
        dims,
        "unclipped_encoded_srgb",
        false,
    )?;
    let valid_id = c.mask_artifact(&valid, dims)?;
    c.review["geometry"] = json!("pixel_aligned");
    c.review["aligned_artifact_id"] = json!(reference_id);
    c.review["common_area_artifact_id"] = json!(valid_id);
    c.review["correspondence_review"] = record.clone();
    c.review_ref(&reference_id);
    c.review_ref(&valid_id);
    c.review["limitations"]
        .as_array_mut()
        .unwrap()
        .extend(limitations.iter().cloned());
    let maps: Vec<(Vec<f64>, Vec<f64>)> = labs
        .iter()
        .map(|lab| {
            (
                lab.iter()
                    .zip(&ref_lab)
                    .map(|(a, b)| image_data::delta_e_2000(*a, *b))
                    .collect(),
                lab.iter()
                    .zip(&ref_lab)
                    .map(|(a, b)| ((a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt())
                    .collect(),
            )
        })
        .collect();
    if maps
        .iter()
        .flat_map(|(a, b)| a.iter().chain(b))
        .any(|x| !x.is_finite())
    {
        return Err(err(
            "computation_failure",
            "Reference distance maps contain nonfinite samples",
        ));
    }
    let mut refs = vec![reference_id, valid_id];
    for s in 0..labs.len() {
        let role = if c.single || s == 1 {
            ArtifactRole::CandidateMaps
        } else {
            ArtifactRole::BaselineMaps
        };
        let mut v = Vec::with_capacity(dims[0] * dims[1] * 4);
        for (i, lab) in labs[s].iter().enumerate() {
            v.extend_from_slice(&[maps[s].0[i], maps[s].1[i], lab[0], ref_lab[i][0]]);
        }
        refs.push(c.numeric_artifact(
            role,
            &v,
            &[dims[1], dims[0], 4],
            DOMAIN,
            Some("DE00; Lab a*/b* distance; rendered L*; reference L*"),
        )?);
    }
    if c.single {
        let (id, path) = c.next_artifact(ArtifactRole::CandidateMaps, "npy");
        image_data::write_npy(&path, &maps[0].0, &[dims[1], dims[0]])?;
        let mut row = c.artifact_row(&id, &path, "difference_map", "application/x-npy")?;
        row["metric"] = json!("delta_e00");
        row["dimensions"] = json!(dims);
        row["valid_mask_artifact_id"] = json!(refs[1]);
        row["availability"] = json!(if tone { "available" } else { "unavailable" });
        if !tone {
            row["reason"] = json!("tone_contaminated_reference");
        }
        c.body.artifacts.push(row);
        c.review["difference_artifact_id"] = json!(id);
        c.review_ref(&id);
        if tone {
            let ranked: Vec<_> = maps[0]
                .0
                .iter()
                .zip(&valid)
                .map(|(value, valid)| if *valid { *value } else { 0. })
                .collect();
            let mut hotspots = crate::metrics::select_hotspots(
                &ranked,
                dims[0],
                dims[1],
                &c.protocol["hotspots"],
                "reference.delta_e00",
                std::slice::from_ref(&id),
            );
            hotspots.retain(|region| {
                let rectangle: Vec<_> = region["rectangle"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as usize)
                    .collect();
                (rectangle[1]..rectangle[1] + rectangle[3]).all(|y| {
                    (rectangle[0]..rectangle[0] + rectangle[2]).all(|x| valid[y * dims[0] + x])
                })
            });
            c.review["automatic_regions"] = json!(hotspots);
        }
        let clean = ["none", "unmodified", "not_applied"];
        let flip_allowed = tone
            && ["compression", "sharpening", "noise_reduction", "resampling"]
                .iter()
                .all(|key| {
                    reference["processing"][key]
                        .as_str()
                        .is_some_and(|v| clean.contains(&v.trim().to_lowercase().as_str()))
                })
            && valid.iter().all(|v| *v);
        let reason = if flip_allowed {
            None
        } else {
            Some("reference_processing_or_filter_support_unavailable")
        };
        let mut flip_refs = refs.clone();
        let flip = if flip_allowed {
            let map = image_data::official_flip(
                &encoded,
                &rendered[0],
                dims[0],
                dims[1],
                c.protocol["pixels_per_degree"].as_f64().unwrap_or(60.),
            )?;
            let (id, path) = c.next_artifact(ArtifactRole::CandidateMaps, "npy");
            image_data::write_npy(&path, &map, &[dims[1], dims[0]])?;
            let mut row = c.artifact_row(&id, &path, "difference_map", "application/x-npy")?;
            row["metric"] = json!("flip");
            row["dimensions"] = json!(dims);
            c.body.artifacts.push(row);
            flip_refs.push(id);
            Some(map)
        } else {
            None
        };
        for (name, q) in [("mean", None), ("p95", Some(0.95)), ("max", Some(1.))] {
            let value = flip
                .as_ref()
                .map(|map| vec![q.map_or_else(|| mean(map), |q| percentile(map.clone(), q))]);
            c.distances(
                "full_image",
                &format!("reference.flip.{name}"),
                "bounded_encoded_srgb",
                "FLIP error",
                "official LDR-FLIP; complete valid filter support",
                if flip_allowed { valid.len() } else { 0 },
                value,
                reason,
                &flip_refs,
            );
        }
    }
    let full = json!({"id":"full_image"});
    for region in std::iter::once(&full).chain(optional_array(case, "regions")) {
        let id = text(region, "id")?;
        let region_mask = if id == "full_image" {
            valid.clone()
        } else {
            valid
                .iter()
                .zip(mask(field(region, "geometry")?, dims, base)?)
                .map(|(a, b)| *a && b)
                .collect()
        };
        let pop = population(&region_mask);
        let count = pop.len();
        let missing = (count == 0).then_some("empty_reference_population");
        for (name, q) in [("median", 0.5), ("p95", 0.95), ("p99", 0.99), ("max", 1.)] {
            let values = (count > 0 && tone).then(|| {
                maps.iter()
                    .map(|map| percentile(pop.iter().map(|i| map.0[*i]).collect(), q))
                    .collect()
            });
            c.distances(
                id,
                &format!("reference.delta_e00.{name}"),
                DOMAIN,
                "delta E 00",
                "pixel quantile method=linear; common-area mask",
                count,
                values,
                missing.or((!tone).then_some("tone_contaminated_reference")),
                &refs,
            );
        }
        for q in [5, 25, 50, 75, 95] {
            let values = (count > 0 && tone).then(|| {
                let ref_q = percentile(
                    pop.iter().map(|i| ref_lab[*i][0]).collect(),
                    q as f64 / 100.,
                );
                labs.iter()
                    .map(|lab| {
                        (percentile(pop.iter().map(|i| lab[*i][0]).collect(), q as f64 / 100.)
                            - ref_q)
                            .abs()
                    })
                    .collect()
            });
            c.distances(
                id,
                &format!("reference.tone_l_star.q{q:02}"),
                DOMAIN,
                "L*",
                "absolute difference of fixed-population lightness quantiles",
                count,
                values,
                missing.or((!tone).then_some("tone_contaminated_reference")),
                &refs,
            );
        }
        let values = (count > 0 && chroma).then(|| {
            maps.iter()
                .map(|map| percentile(pop.iter().map(|i| map.1[*i]).collect(), 0.5))
                .collect()
        });
        c.distances(
            id,
            "reference.chroma_ab.median",
            DOMAIN,
            "Lab a*/b* distance",
            "median pixel chromatic distance; excludes lightness; diagnostic only",
            count,
            values,
            missing.or((!chroma).then_some("unknown_or_graded_color_reference")),
            &refs,
        );
    }
    Ok(())
}

// Keep regional arrays and frozen comparison controls explicit at this boundary.
#[allow(clippy::too_many_arguments)]
fn regional(
    c: &mut Comparison,
    ref_lab: &[[f64; 3]],
    reference_dims: [usize; 2],
    labs: &[Vec<[f64; 3]>],
    dims: [usize; 2],
    case: &Value,
    base: &Path,
    tone: bool,
    chroma: bool,
) -> Result<()> {
    let reference = c.reference;
    let corresponding = optional_array(reference, "regions");
    if corresponding.is_empty() {
        return Err(err(
            "missing_reference_regions",
            "Regional scope requires fixed corresponding regions",
        ));
    }
    let mut seen = BTreeSet::new();
    for r in corresponding {
        if !seen.insert(text(r, "id")?) {
            return Err(err(
                "invalid_reference_regions",
                "Reference region identities must be unique",
            ));
        }
    }
    c.limitation("Corresponding region statistics do not establish pixel correspondence or full-scene fidelity.");
    c.review["regions"] = json!([]);
    for region in corresponding {
        let id = text(region, "id")?;
        let render = optional_array(case, "regions")
            .iter()
            .rev()
            .find(|r| r.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| {
                err(
                    "invalid_reference_regions",
                    "Reference region has no matching render region",
                )
            })?;
        let render_pop = population(&mask(field(render, "geometry")?, dims, base)?);
        let ref_pop = population(&mask(field(region, "geometry")?, reference_dims, base)?);
        if render_pop.is_empty() || ref_pop.is_empty() {
            return Err(err(
                "empty_reference_population",
                "Corresponding region has no eligible samples",
            ));
        }
        let populations: Vec<_> = labs
            .iter()
            .map(|lab| (&lab[..], &render_pop))
            .chain(std::iter::once((ref_lab, &ref_pop)))
            .collect();
        let centers: Vec<[f64; 3]> = populations
            .iter()
            .map(|population| {
                std::array::from_fn(|ch| {
                    percentile(
                        population.1.iter().map(|i| population.0[*i][ch]).collect(),
                        0.5,
                    )
                })
            })
            .collect();
        let quantiles: Vec<Vec<f64>> = populations
            .iter()
            .map(|population| {
                (0..=100)
                    .map(|q| {
                        percentile(
                            population.1.iter().map(|i| population.0[*i][0]).collect(),
                            q as f64 / 100.,
                        )
                    })
                    .collect()
            })
            .collect();
        let reference_index = labs.len();
        let mut payload = json!({"region_id":id,"render_geometry":render["geometry"],"reference_geometry":region["geometry"],"render_sample_count":render_pop.len(),"reference_sample_count":ref_pop.len(),"lab_median":{"reference":centers[reference_index]},"lightness_quantile_probabilities":(0..=100).map(|q|q as f64/100.).collect::<Vec<_>>(),"lightness_quantiles":{"reference":quantiles[reference_index]},"domain":DOMAIN,"units":"L*,a*,b*"});
        for s in 0..labs.len() {
            let role = if c.single {
                "simulation"
            } else if s == 0 {
                "baseline"
            } else {
                "candidate"
            };
            payload["lab_median"][role] = json!(centers[s]);
            payload["lightness_quantiles"][role] = json!(quantiles[s]);
        }
        let evidence = c.json_artifact(ArtifactRole::RegionalStatistics, &payload)?;
        c.review["regions"].as_array_mut().unwrap().push(payload);
        c.review_ref(&evidence);
        let refs = [evidence];
        let de: Vec<_> = (0..labs.len())
            .map(|s| image_data::delta_e_2000(centers[s], centers[reference_index]))
            .collect();
        let ld: Vec<_> = (0..labs.len())
            .map(|s| {
                mean(
                    &quantiles[s]
                        .iter()
                        .zip(&quantiles[reference_index])
                        .map(|(a, b)| (a - b).abs())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let ch: Vec<_> = (0..labs.len())
            .map(|s| {
                ((centers[s][1] - centers[reference_index][1]).powi(2)
                    + (centers[s][2] - centers[reference_index][2]).powi(2))
                .sqrt()
            })
            .collect();
        c.distances(
            id,
            "reference.patch_delta_e00",
            DOMAIN,
            "delta E 00",
            "DE00 between independent Lab channel medians; robust interior patch",
            render_pop.len(),
            tone.then_some(de),
            (!tone).then_some("tone_contaminated_reference"),
            &refs,
        );
        c.distances(id,"reference.distribution_l_star",DOMAIN,"L*","mean absolute distance of 101 independent marginal lightness quantiles; linear quantiles",render_pop.len(),tone.then_some(ld),(!tone).then_some("tone_contaminated_reference"),&refs);
        c.distances(
            id,
            "reference.patch_chroma_ab",
            DOMAIN,
            "Lab a*/b* distance",
            "distance between independent median a*/b*; excludes lightness; diagnostic only",
            render_pop.len(),
            chroma.then_some(ch),
            (!chroma).then_some("unknown_or_graded_color_reference"),
            &refs,
        );
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ProbeDomain {
    Native,
    Lab,
    Luminance,
}
#[derive(Clone, Copy)]
enum Reduction {
    Mean,
    Median,
}
struct ProbeEstimator {
    name: String,
    domain: ProbeDomain,
    reduction: Reduction,
}
impl ProbeEstimator {
    fn parse(name: &str) -> Result<Self> {
        let (domain, reduction) = match name {
            "native_channel_mean" => (ProbeDomain::Native, Reduction::Mean),
            "native_channel_median" => (ProbeDomain::Native, Reduction::Median),
            "lab_channel_mean" => (ProbeDomain::Lab, Reduction::Mean),
            "lab_channel_median" => (ProbeDomain::Lab, Reduction::Median),
            "linear_luminance_mean" => (ProbeDomain::Luminance, Reduction::Mean),
            "linear_luminance_median" => (ProbeDomain::Luminance, Reduction::Median),
            _ => {
                return Err(err(
                    "invalid_probe_response",
                    "Unknown model response estimator",
                ));
            }
        };
        Ok(Self {
            name: name.into(),
            domain,
            reduction,
        })
    }
    fn channel(&self, name: &str) -> Option<usize> {
        match self.domain {
            ProbeDomain::Native => match name {
                "R" => Some(0),
                "G" => Some(1),
                "B" => Some(2),
                _ => None,
            },
            ProbeDomain::Lab => match name {
                "L" => Some(0),
                "a" => Some(1),
                "b" => Some(2),
                _ => None,
            },
            ProbeDomain::Luminance => (name == "Y").then_some(0),
        }
    }
    fn reduce(&self, values: Vec<f64>) -> f64 {
        match self.reduction {
            Reduction::Mean => mean(&values),
            Reduction::Median => percentile(values, 0.5),
        }
    }
}
fn unique_curves(curves: &[Value]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for curve in curves {
        if !ids.insert(text(curve, "id")?) {
            return Err(err(
                "invalid_material_measurement",
                "Curve identities must be unique",
            ));
        }
    }
    Ok(())
}
fn material_arrays(
    c: &mut Comparison,
    estimator: &ProbeEstimator,
    images: &[&ImageData],
) -> Result<Vec<Vec<[f64; 3]>>> {
    let reference = c.reference;
    let domain = text(reference, "domain")?;
    let units = text(reference, "units")?;
    match estimator.domain {
        ProbeDomain::Native => {
            if domain == "native_rgb" && units == "native channel value" {
                if images.iter().any(|image| {
                    (image.color_space.as_str(), image.transfer.as_str())
                        != (images[0].color_space.as_str(), images[0].transfer.as_str())
                }) {
                    return Err(err(
                        "incompatible_material_conditions",
                        "Native RGB probe declarations differ",
                    ));
                }
                c.limitation("Native RGB response is a rendered diagnostic, not density, spectral response, or physical sensitometry.");
            } else {
                for image in images {
                    let declaration = image.metadata.get("material_measurement");
                    if ["domain", "units", "instrument", "conditions"]
                        .iter()
                        .any(|f| declaration.and_then(|d| d.get(*f)) != reference.get(*f))
                    {
                        return Err(err(
                            "incompatible_material_conditions",
                            "Native probe lacks matching measurement-domain/instrument declaration; RGB is not a density proxy",
                        ));
                    }
                }
            }
            Ok(images.iter().map(|image| image.samples.clone()).collect())
        }
        ProbeDomain::Lab => {
            if domain != DOMAIN || !["L*", "a*", "b*"].contains(&units) {
                return Err(err(
                    "incompatible_material_conditions",
                    "Lab response requires declared D65 Lab domain and channel units",
                ));
            }
            images
                .iter()
                .map(|image| Ok(clipped_lab(&image_data::display_rgb(image)?)))
                .collect()
        }
        ProbeDomain::Luminance => {
            if domain != "linear_srgb_relative_luminance" || units != "relative luminance" {
                return Err(err(
                    "incompatible_material_conditions",
                    "Luminance response must declare relative linear sRGB luminance",
                ));
            }
            let convert = |image: &ImageData| -> Result<Vec<[f64; 3]>> {
                Ok(image_data::display_rgb(image)?
                    .iter()
                    .map(|p| {
                        [
                            image_data::srgb_decode(p[0]) * 0.2126
                                + image_data::srgb_decode(p[1]) * 0.7152
                                + image_data::srgb_decode(p[2]) * 0.0722,
                            0.,
                            0.,
                        ]
                    })
                    .collect())
            };
            images.iter().map(|image| convert(image)).collect()
        }
    }
}
fn numeric_curve(v: Option<&Value>) -> Option<Vec<f64>> {
    v?.as_array()?.iter().map(Value::as_f64).collect()
}
fn material(c: &mut Comparison, images: &[&ImageData], case: &Value, base: &Path) -> Result<()> {
    let reference = c.reference;
    let source = read_json(field(reference, "path")?, base)?;
    let source_id = c.json_artifact(ArtifactRole::MaterialSource, &source)?;
    c.review_ref(&source_id);
    c.review["master_artifact_id"] = json!(source_id);
    let response = case
        .get("probe_response")
        .filter(|v| v.is_object())
        .ok_or_else(|| {
            err(
                "missing_probe_response",
                "Material evidence requires a declared matching model probe",
            )
        })?;
    if source.get("schema_version") != Some(&json!(1)) {
        return Err(err(
            "invalid_material_measurement",
            "Material measurement must use schema_version 1",
        ));
    }
    for f in ["domain", "units", "instrument", "conditions"] {
        let value = reference.get(f).filter(|v| truthy(v)).ok_or_else(|| {
            err(
                "incompatible_material_conditions",
                format!("Source and model response must agree on {f}"),
            )
        })?;
        if source.get(f) != Some(value) || response.get(f) != Some(value) {
            return Err(err(
                "incompatible_material_conditions",
                format!("Source and model response must agree on {f}"),
            ));
        }
        if value
            .as_str()
            .is_some_and(|s| s.to_lowercase() == "unknown")
        {
            return Err(err(
                "incompatible_material_conditions",
                "Material measurement conditions must be known",
            ));
        }
    }
    let coordinate = source
        .get("coordinate")
        .filter(|v| v.is_object())
        .ok_or_else(|| {
            err(
                "incompatible_material_coordinates",
                "Material coordinate identity and units must match",
            )
        })?;
    if !coordinate.get("name").is_some_and(truthy)
        || !coordinate.get("units").is_some_and(truthy)
        || response.get("coordinate") != Some(coordinate)
    {
        return Err(err(
            "incompatible_material_coordinates",
            "Material coordinate identity and units must match",
        ));
    }
    let source_curves = optional_array(&source, "curves");
    let response_curves = optional_array(response, "curves");
    if source_curves.is_empty() || response_curves.is_empty() {
        return Err(err(
            "missing_material_curves",
            "Source and model response require curves",
        ));
    }
    unique_curves(source_curves)?;
    unique_curves(response_curves)?;
    for image in images {
        if image.dimensions() != images[0].dimensions() {
            return Err(err("dimension_mismatch", "Probe dimensions differ"));
        }
        finite_rgb(
            &image.samples,
            "nonfinite_samples",
            "Material probe contains nonfinite samples",
        )?;
    }
    let estimator = ProbeEstimator::parse(
        response
            .get("estimator")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )?;
    let arrays = material_arrays(c, &estimator, images)?;
    for a in &arrays {
        finite_rgb(
            a,
            "computation_failure",
            "Material domain conversion is nonfinite",
        )?;
    }
    for f in ["domain", "units", "instrument", "conditions"] {
        c.review[f] = reference[f].clone();
    }
    c.review["geometry"] = json!("measurement_curves");
    c.review["coordinate"] = coordinate.clone();
    c.limitation("This objective compares declared model probe response to measured curves under matching conditions.");
    for curve in response_curves {
        let identity = text(curve, "id")?;
        if identity.is_empty()
            || !identity
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(err(
                "invalid_material_measurement",
                "Curve identity must be a stable metric token",
            ));
        }
        let expected = source_curves
            .iter()
            .find(|v| v.get("id").and_then(Value::as_str) == Some(identity))
            .ok_or_else(|| {
                err(
                    "incompatible_material_curves",
                    "Source/model curve identity and channel must match",
                )
            })?;
        let channel = curve
            .get("channel")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                err(
                    "incompatible_material_curves",
                    "Source/model curve identity and channel must match",
                )
            })?;
        let ch = estimator.channel(channel).ok_or_else(|| {
            err(
                "incompatible_material_curves",
                "Source/model curve identity and channel must match",
            )
        })?;
        if expected.get("channel") != curve.get("channel") {
            return Err(err(
                "incompatible_material_curves",
                "Source/model curve identity and channel must match",
            ));
        }
        if matches!(estimator.domain, ProbeDomain::Lab)
            && format!("{channel}*") != text(reference, "units")?
        {
            return Err(err(
                "incompatible_material_conditions",
                "Lab response channel does not match declared units",
            ));
        }
        let coordinates = numeric_curve(expected.get("coordinates")).ok_or_else(|| {
            err(
                "invalid_material_measurement",
                "Curve requires finite, strictly increasing coordinates and matching values",
            )
        })?;
        let values = numeric_curve(expected.get("values")).ok_or_else(|| {
            err(
                "invalid_material_measurement",
                "Curve requires finite, strictly increasing coordinates and matching values",
            )
        })?;
        if coordinates.len() < 2
            || coordinates.len() != values.len()
            || coordinates.iter().chain(&values).any(|x| !x.is_finite())
            || coordinates.windows(2).any(|w| w[1] <= w[0])
        {
            return Err(err(
                "invalid_material_measurement",
                "Curve requires finite, strictly increasing coordinates and matching values",
            ));
        }
        let samples = optional_array(curve, "samples");
        let declared: Option<Vec<f64>> = samples
            .iter()
            .map(|s| s.get("coordinate").and_then(Value::as_f64))
            .collect();
        if declared.as_ref() != Some(&coordinates) {
            return Err(err(
                "incompatible_material_coordinates",
                "Probe coordinates must exactly match frozen source coordinates",
            ));
        }
        let mut measured = vec![Vec::new(); images.len()];
        let mut pixel_counts = Vec::new();
        let mut regions = Vec::new();
        for sample in samples {
            let region_id = text(sample, "region_id")?;
            let pop = if region_id == "full_image" {
                (0..images[0].samples.len()).collect()
            } else {
                let region = optional_array(case, "regions")
                    .iter()
                    .rev()
                    .find(|r| r.get("id").and_then(Value::as_str) == Some(region_id))
                    .ok_or_else(|| {
                        err(
                            "invalid_probe_response",
                            "Probe sample names a missing region",
                        )
                    })?;
                population(&mask(
                    field(region, "geometry")?,
                    images[0].dimensions(),
                    base,
                )?)
            };
            if pop.is_empty() {
                return Err(err(
                    "empty_reference_population",
                    "Probe region contains no samples",
                ));
            }
            pixel_counts.push(pop.len());
            regions.push(region_id);
            for s in 0..images.len() {
                measured[s].push(estimator.reduce(pop.iter().map(|i| arrays[s][*i][ch]).collect()));
            }
        }
        let residual: Vec<Vec<f64>> = measured
            .iter()
            .map(|samples| samples.iter().zip(&values).map(|(a, b)| a - b).collect())
            .collect();
        if measured
            .iter()
            .chain(&residual)
            .flatten()
            .any(|x| !x.is_finite())
        {
            return Err(err(
                "computation_failure",
                "Material response or residual estimation is nonfinite",
            ));
        }
        let mut payload = json!({"curve_id":identity,"coordinates":coordinates,"coordinate":coordinate,"reference":values,"domain":reference["domain"],"units":reference["units"],"instrument":reference["instrument"],"conditions":reference["conditions"],"estimator":estimator.name,"region_pixel_counts":pixel_counts,"regions":regions});
        for s in 0..images.len() {
            let role = if c.single {
                "simulation"
            } else if s == 0 {
                "baseline"
            } else {
                "candidate"
            };
            payload[role] = json!(measured[s]);
            payload[format!("{role}_residual")] = json!(residual[s]);
        }
        let evidence = c.json_artifact(ArtifactRole::MaterialCurve, &payload)?;
        c.review_ref(&evidence);
        let rmse = residual
            .iter()
            .map(|samples| {
                (samples.iter().map(|x| x * x).sum::<f64>() / values.len() as f64).sqrt()
            })
            .collect();
        c.distances(
            "full_image",
            &format!("reference.material.{identity}"),
            text(reference, "domain")?,
            text(reference, "units")?,
            &format!(
                "RMSE over matched source coordinates; probe {}",
                estimator.name
            ),
            coordinates.len(),
            Some(rmse),
            None,
            &[evidence],
        );
    }
    Ok(())
}
fn compare_scope(
    c: &mut Comparison,
    images: &[&ImageData],
    case: &Value,
    base: &Path,
) -> Result<()> {
    let reference = c.reference;
    let scope = text(reference, "scope")?;
    if scope == "material" {
        return material(c, images, case, base);
    }
    let source = read_reference(reference, base)?;
    finite_rgb(
        &source.samples,
        "nonfinite_samples",
        "Reference contains nonfinite samples",
    )?;
    let master = c.rgb_artifact(
        ArtifactRole::Native,
        &source.samples,
        source.dimensions(),
        if source.color_space.is_empty() {
            "unknown"
        } else {
            &source.color_space
        },
        false,
    )?;
    c.review["master_artifact_id"] = json!(master);
    c.review["native_dimensions"] = json!(source.dimensions());
    c.review["decode_metadata"] = source.metadata.clone();
    c.review_ref(&master);
    let converted = image_data::display_rgb(&source).and_then(|v| {
        finite_rgb(
            &v,
            "computation_failure",
            "Reference color conversion produced nonfinite samples",
        )?;
        Ok(v)
    });
    let (encoded, color_reason) = match converted {
        Ok(v) => {
            c.review["display_clipped_sample_fraction"] = json!(clipping_fraction(&v));
            c.review["color_available"] = json!(true);
            (v, None)
        }
        Err(e) => {
            c.review["color_available"] = json!(false);
            c.limitation("Preview uses uninterpreted native RGB; color fidelity is unavailable.");
            (source.samples.clone(), Some(e.code))
        }
    };
    let display = c.preview(&encoded, source.dimensions(), color_reason.is_none())?;
    c.review["display_artifact_id"] = json!(display);
    c.review["native_display_artifact_id"] = json!(display);
    c.review_ref(&display);
    if scope == "visual" {
        c.limitation(
            "Visual reference only; no numerical fidelity or exact scene correspondence claim.",
        );
        if c.single {
            c.distances(
                "full_image",
                "reference.visual",
                "visual_review",
                "not applicable",
                "visual reference; numerical distance prohibited",
                0,
                None,
                Some("visual_only_reference"),
                &[display],
            );
        }
        return Ok(());
    }
    for image in images {
        if image.dimensions() != images[0].dimensions() {
            return Err(err("dimension_mismatch", "Rendered geometry differs"));
        }
        finite_rgb(
            &image.samples,
            "nonfinite_samples",
            "Rendered evidence contains nonfinite samples",
        )?;
    }
    if let Some(reason) = color_reason {
        return Err(err(
            &reason,
            "Reference color interpretation is unavailable",
        ));
    }
    let mut rendered = Vec::with_capacity(images.len());
    let mut labs = Vec::with_capacity(images.len());
    for image in images {
        let rgb = image_data::display_rgb(image)?;
        finite_rgb(
            &rgb,
            "computation_failure",
            "Rendered display conversion contains nonfinite samples",
        )?;
        labs.push(clipped_lab(&rgb));
        rendered.push(rgb);
    }
    let (tone, chroma) = processing_eligibility(reference)?;
    c.review["tone_target_eligible"] = json!(tone);
    c.review["color_diagnostic_eligible"] = json!(chroma);
    if !tone {
        c.limitation("Grading/histogram history prevents an unmodified tone or full DE00 fidelity objective.");
    }
    match scope {
        "aligned_scene" => aligned(
            c,
            &source,
            &encoded,
            &labs,
            &rendered,
            images[0].dimensions(),
            case,
            base,
            tone,
            chroma,
        ),
        "regional" => regional(
            c,
            &clipped_lab(&encoded),
            source.dimensions(),
            &labs,
            images[0].dimensions(),
            case,
            base,
            tone,
            chroma,
        ),
        _ => Err(err(
            "unsupported_reference_scope",
            "Unknown reference scope",
        )),
    }
}
fn image_specs(c: &mut Comparison, region: &str, reason: &str, regional: bool) {
    let mut specs: Vec<(String, &str)> = if regional {
        vec![
            ("reference.patch_delta_e00".into(), "delta E 00"),
            ("reference.distribution_l_star".into(), "L*"),
            ("reference.patch_chroma_ab".into(), "Lab a*/b* distance"),
        ]
    } else {
        ["median", "p95", "p99", "max"]
            .iter()
            .map(|s| (format!("reference.delta_e00.{s}"), "delta E 00"))
            .collect()
    };
    if !regional {
        specs.extend(
            [5, 25, 50, 75, 95]
                .iter()
                .map(|q| (format!("reference.tone_l_star.q{q:02}"), "L*")),
        );
        specs.push(("reference.chroma_ab.median".into(), "Lab a*/b* distance"));
    }
    for (metric, units) in specs {
        c.distances(
            region,
            &metric,
            DOMAIN,
            units,
            "frozen reference population",
            0,
            None,
            Some(reason),
            &[],
        );
    }
}
pub fn compare_reference(
    baseline: &ImageData,
    candidate: &ImageData,
    case: &Value,
    protocol: &Value,
    out_dir: &Path,
    suite_base: &Path,
) -> Result<Value> {
    compare_reference_population(
        &[baseline, candidate],
        case,
        protocol,
        out_dir,
        suite_base,
        false,
    )
}

pub fn compare_reference_single(
    simulation: &ImageData,
    case: &Value,
    protocol: &Value,
    out_dir: &Path,
    suite_base: &Path,
) -> Result<Value> {
    compare_reference_population(&[simulation], case, protocol, out_dir, suite_base, true)
}

fn compare_reference_population(
    images: &[&ImageData],
    case: &Value,
    protocol: &Value,
    out_dir: &Path,
    suite_base: &Path,
    single: bool,
) -> Result<Value> {
    let Some(reference) = case.get("reference").filter(|v| !v.is_null()) else {
        return Ok(
            json!({"measurements":[],"findings":[],"artifacts":[],"execution_status":"completed","metric_version":METRIC_VERSION,"reference_review":null}),
        );
    };
    let protocol = if protocol.is_object() {
        protocol.clone()
    } else {
        json!({"id":protocol})
    };
    let mut c = Comparison::new(reference, case, &protocol, out_dir)?;
    c.single = single;
    fs::create_dir_all(out_dir)?;
    if let Err(error) = compare_scope(&mut c, images, case, suite_base) {
        // Python distinguishes typed image errors from generic I/O/document failures.
        let reason = if [
            "operation_failed",
            "document_read",
            "document_type",
            "image_read_failed",
        ]
        .contains(&error.code.as_str())
        {
            "invalid_reference_evidence".to_owned()
        } else {
            error.code.clone()
        };
        c.finding(
            "full_image",
            "reference",
            &reason,
            matches!(reason.as_str(), "nonfinite_samples" | "computation_failure"),
            &[],
        );
        c.review["unavailable_reason"] = json!(reason);
        c.limitation(error.message);
        let existing: BTreeSet<String> = c
            .body
            .measurements
            .iter()
            .map(|m| m.shared.region_id.clone())
            .collect();
        match reference.get("scope").and_then(Value::as_str).unwrap_or("") {
            "aligned_scene" => {
                for id in std::iter::once("full_image").chain(
                    optional_array(case, "regions")
                        .iter()
                        .filter_map(|r| r.get("id").and_then(Value::as_str)),
                ) {
                    if !existing.contains(id) {
                        image_specs(&mut c, id, &reason, false);
                    }
                }
            }
            "regional" => {
                let ids: Vec<&str> = optional_array(reference, "regions")
                    .iter()
                    .filter_map(|r| r.get("id").and_then(Value::as_str))
                    .collect();
                if ids.is_empty() {
                    image_specs(&mut c, "full_image", &reason, true);
                } else {
                    for id in ids {
                        if !existing.contains(id) {
                            image_specs(&mut c, id, &reason, true);
                        }
                    }
                }
            }
            "material" => {
                let mut ids: Vec<&str> = case
                    .get("probe_response")
                    .map(|p| {
                        optional_array(p, "curves")
                            .iter()
                            .filter_map(|r| r.get("id").and_then(Value::as_str))
                            .collect()
                    })
                    .unwrap_or_default();
                if ids.is_empty() {
                    ids.push("response");
                }
                for id in ids {
                    let metric = format!("reference.material.{id}");
                    if !c
                        .body
                        .measurements
                        .iter()
                        .any(|m| m.shared.metric.starts_with(&format!("{metric}.")))
                    {
                        c.distances(
                            "full_image",
                            &metric,
                            reference
                                .get("domain")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown"),
                            reference
                                .get("units")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown"),
                            "matched coordinate RMSE",
                            0,
                            None,
                            Some(&reason),
                            &[],
                        );
                    }
                }
            }
            _ => {}
        }
    }
    Ok(c.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn transform(matrix: Value, interpolation: &str) -> FrozenTransform {
        FrozenTransform::parse(&json!({"matrix":matrix,"direction":"reference_to_render","coordinate_system":"pixel_centers","interpolation":interpolation})).unwrap()
    }
    #[test]
    fn rectangular_identity_retains_geometry_and_pixels() {
        let pixels: Vec<[f64; 3]> = (0..6).map(|i| [i as f64; 3]).collect();
        let t = transform(json!([[1, 0, 0], [0, 1, 0], [0, 0, 1]]), "nearest");
        let (v, s) = t.warp(&pixels, [3, 2], [3, 2]).unwrap();
        assert_eq!(v, pixels);
        assert_eq!(s, vec![true; 6]);
    }
    #[test]
    fn translation_support_uses_pixel_centers_and_retains_border_sample() {
        let pixels = vec![[1.; 3], [2.; 3], [3.; 3]];
        let t = transform(json!([[1, 0, 1], [0, 1, 0], [0, 0, 1]]), "nearest");
        let (v, s) = t.warp(&pixels, [3, 1], [3, 1]).unwrap();
        assert_eq!(s, vec![false, true, true]);
        assert_eq!(v, vec![[1.; 3], [1.; 3], [2.; 3]]);
    }
    #[test]
    fn bilinear_half_pixel_is_interpolated_in_rgb_before_lab() {
        let t = transform(json!([[1, 0, -0.5], [0, 1, 0], [0, 0, 1]]), "bilinear");
        let (v, s) = t.warp(&[[0.; 3], [1.; 3]], [2, 1], [2, 1]).unwrap();
        assert_eq!(v[0], [0.5; 3]);
        assert_eq!(s, vec![true, false]);
    }
    #[test]
    fn singular_and_undeclared_transforms_are_rejected() {
        assert!(FrozenTransform::parse(&json!({"matrix":[[1,0,0],[0,0,0],[0,0,1]],"direction":"reference_to_render","coordinate_system":"pixel_centers","interpolation":"nearest"})).is_err());
        assert!(FrozenTransform::parse(&json!({"matrix":[[1,0,0],[0,1,0],[0,0,1]]})).is_err());
    }
    #[test]
    fn brightness_only_retains_diagnostic_chroma_but_blocks_tone() {
        assert_eq!(
            processing_eligibility(
                &json!({"processing":{"grading":" NONE ","histogram_matching":"brightness_only"}})
            )
            .unwrap(),
            (false, true)
        );
        assert_eq!(
            processing_eligibility(
                &json!({"processing":{"grading":"unknown","histogram_matching":"none"}})
            )
            .unwrap(),
            (false, false)
        );
        assert_eq!(
            processing_eligibility(
                &json!({"processing":{"grading":"unmodified","histogram_matching":"not_applied"}})
            )
            .unwrap(),
            (true, true)
        );
    }
    #[test]
    fn material_estimators_reduce_population_and_enforce_channel_identity() {
        let med = ProbeEstimator::parse("native_channel_median").unwrap();
        assert_eq!(med.reduce(vec![0., 1., 1., 10.]), 1.);
        assert_eq!(med.channel("R"), Some(0));
        assert_eq!(med.channel("Y"), None);
        let mean = ProbeEstimator::parse("linear_luminance_mean").unwrap();
        assert_eq!(mean.reduce(vec![1., 3.]), 2.);
        assert_eq!(mean.channel("Y"), Some(0));
        assert!(ProbeEstimator::parse("density_from_rgb").is_err());
    }
    #[test]
    fn overflow_distance_fails_and_never_serializes_a_numeric_value() {
        let reference = json!({"id":"r","scope":"material"});
        let case = json!({"id":"c","seeds":[0]});
        let mut c = Comparison::new(&reference, &case, &json!({"id":"p"}), Path::new(".")).unwrap();
        c.distances(
            "full_image",
            "reference.material.a",
            "native_rgb",
            "native channel value",
            "test",
            2,
            Some(vec![-f64::MAX, f64::MAX]),
            None,
            &[],
        );
        let out = c.finish();
        assert_eq!(out["execution_status"], "failed");
        for row in out["measurements"].as_array().unwrap() {
            assert_eq!(row["availability"], "unavailable");
            assert!(row.get("value").is_none());
            assert_eq!(row["reason"], "computation_failure");
        }
    }
    fn image(values: Vec<[f64; 3]>, width: usize, height: usize) -> ImageData {
        ImageData {
            samples: values,
            width,
            height,
            color_space: "sRGB".into(),
            transfer: "encoded".into(),
            metadata: json!({}),
            path: None,
        }
    }
    #[test]
    fn material_loads_json_before_raster_and_retains_signed_rmse_curve() {
        let temp = tempfile::tempdir().unwrap();
        let coordinate = json!({"name":"exposure","units":"stops"});
        let conditions = json!({"temperature_c":20});
        let source = json!({"schema_version":1,"domain":"native_rgb","units":"native channel value","instrument":"fixture","conditions":conditions,"coordinate":coordinate,"curves":[{"id":"red","channel":"R","coordinates":[0.,1.],"values":[0.,1.]}]});
        common::dump(&temp.path().join("material.json"), &source).unwrap();
        let reference = json!({"id":"measured","scope":"material","path":"material.json","domain":"native_rgb","units":"native channel value","instrument":"fixture","conditions":conditions});
        let response = json!({"domain":"native_rgb","units":"native channel value","instrument":"fixture","conditions":conditions,"coordinate":coordinate,"estimator":"native_channel_mean","curves":[{"id":"red","channel":"R","samples":[{"coordinate":0.,"region_id":"left"},{"coordinate":1.,"region_id":"right"}]}]});
        let case = json!({"id":"material","seeds":[0],"reference":reference,"probe_response":response,"regions":[{"id":"left","geometry":[0,0,1,1]},{"id":"right","geometry":[1,0,1,1]}]});
        let baseline = image(vec![[0.2; 3], [0.8; 3]], 2, 1);
        let candidate = image(vec![[0.; 3], [1.; 3]], 2, 1);
        let output = temp.path().join("output");
        let result = compare_reference(
            &baseline,
            &candidate,
            &case,
            &json!({"id":"protocol"}),
            &output,
            temp.path(),
        )
        .unwrap();
        assert_eq!(result["execution_status"], "completed");
        assert_eq!(result["measurements"].as_array().unwrap().len(), 3);
        assert!((result["measurements"][0]["value"].as_f64().unwrap() - 0.2).abs() < 1e-15);
        assert_eq!(result["measurements"][1]["value"], 0.);
        assert!(result["measurements"][2]["value"].as_f64().unwrap() < 0.);
        let artifact = result["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["role"] == "reference_material_curve")
            .unwrap();
        let curve: Value = serde_json::from_slice(
            &fs::read(output.join(artifact["path"].as_str().unwrap())).unwrap(),
        )
        .unwrap();
        assert_eq!(curve["region_pixel_counts"], json!([1, 1]));
        assert_eq!(curve["candidate_residual"], json!([0., 0.]));
    }
    #[test]
    fn regional_reference_has_independent_raster_population_size() {
        let temp = tempfile::tempdir().unwrap();
        let reference =
            json!({"id":"r","scope":"regional","regions":[{"id":"patch","geometry":[0,0,3,1]}]});
        let case = json!({"id":"c","regions":[{"id":"patch","geometry":[0,0,2,1]}]});
        let mut c = Comparison::new(&reference, &case, &json!({"id":"p"}), temp.path()).unwrap();
        regional(
            &mut c,
            &[[10., 0., 0.], [20., 0., 0.], [30., 0., 0.]],
            [3, 1],
            &[vec![[20., 0., 0.]; 2], vec![[20., 0., 0.]; 2]],
            [2, 1],
            &case,
            temp.path(),
            true,
            true,
        )
        .unwrap();
        assert_eq!(c.review["regions"][0]["render_sample_count"], 2);
        assert_eq!(c.review["regions"][0]["reference_sample_count"], 3);
        assert_eq!(c.body.measurements.len(), 9);
    }

    #[test]
    fn single_regional_distance_uses_simulation_without_baseline_fields() {
        let temp = tempfile::tempdir().unwrap();
        let reference =
            json!({"id":"r","scope":"regional","regions":[{"id":"patch","geometry":[0,0,3,1]}]});
        let case = json!({"id":"c","seeds":[7],"regions":[{"id":"patch","geometry":[0,0,2,1]}]});
        let mut c = Comparison::new(&reference, &case, &json!({"id":"p"}), temp.path()).unwrap();
        c.single = true;
        regional(
            &mut c,
            &[[10., 0., 0.], [20., 0., 0.], [30., 0., 0.]],
            [3, 1],
            &[vec![[35., 0., 0.]; 2]],
            [2, 1],
            &case,
            temp.path(),
            true,
            true,
        )
        .unwrap();
        let out = c.finish();
        assert_eq!(out["measurements"].as_array().unwrap().len(), 3);
        assert!(out["measurements"].as_array().unwrap().iter().all(|m| {
            m["metric"]
                .as_str()
                .unwrap()
                .ends_with(".simulation_distance")
        }));
        assert!(out["measurements"][0]["value"].as_f64().unwrap() > 0.);
        let statistics = &out["reference_review"]["regions"][0];
        assert_eq!(statistics["lab_median"]["simulation"], json!([35., 0., 0.]));
        assert!(statistics["lab_median"].get("baseline").is_none());
        assert!(
            out["findings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|f| f.get("distance_delta").is_none() && f.get("baseline_distance").is_none())
        );
    }

    #[test]
    fn single_processing_limits_never_emit_numeric_unavailable_distance() {
        let temp = tempfile::tempdir().unwrap();
        let reference =
            json!({"id":"r","scope":"regional","regions":[{"id":"patch","geometry":[0,0,1,1]}]});
        let case = json!({"id":"c","regions":[{"id":"patch","geometry":[0,0,1,1]}]});
        let mut c = Comparison::new(&reference, &case, &json!({"id":"p"}), temp.path()).unwrap();
        c.single = true;
        regional(
            &mut c,
            &[[20., 0., 0.]],
            [1, 1],
            &[vec![[35., 0., 0.]]],
            [1, 1],
            &case,
            temp.path(),
            false,
            false,
        )
        .unwrap();
        let out = c.finish();
        assert!(
            out["measurements"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m["availability"] == "unavailable"
                    && m.get("value").is_none()
                    && m["reason"].is_string())
        );
    }
    #[test]
    fn single_visual_retains_native_review_without_numeric_score() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("reference.png");
        image_data::write_png16(&path, &[[0.2; 3]; 6], 3, 2).unwrap();
        let case = json!({"id":"c","seeds":[0],"reference":{"id":"r","path":path,"sha256":common::sha256_file(&path).unwrap(),"scope":"visual","color":{"color_space":"sRGB","cctf_decoding":true}}});
        let result = compare_reference_single(
            &image(vec![[0.8; 3]; 4], 2, 2),
            &case,
            &json!({"id":"p"}),
            &temp.path().join("out"),
            temp.path(),
        )
        .unwrap();
        assert_eq!(result["execution_status"], "completed");
        assert_eq!(
            result["reference_review"]["native_dimensions"],
            json!([3, 2])
        );
        assert!(result["reference_review"]["native_display_artifact_id"].is_string());
        assert_eq!(result["measurements"][0]["reason"], "visual_only_reference");
        assert!(result["measurements"][0].get("value").is_none());
    }

    #[test]
    fn single_aligned_respects_mask_and_reports_actual_distance_and_flip_limits() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("reference.png");
        image_data::write_png16(&path, &[[0.2; 3]; 4], 2, 2).unwrap();
        common::dump(
            &temp.path().join("review.json"),
            &json!({"status":"approved","reviewer":"test","excluded_regions":[],"limitations":[]}),
        )
        .unwrap();
        let case = json!({"id":"c","seeds":[0],"regions":[],"reference":{"id":"r","path":path,"scope":"aligned_scene","color":{"color_space":"sRGB","cctf_decoding":true},"processing":{"grading":"none","histogram_matching":"none","compression":"none","sharpening":"none","noise_reduction":"none","resampling":"none"},"aligned_derivative":{"path":path},"correspondence_review":{"path":"review.json"},"common_area_mask":[0,0,1,2]}});
        let result = compare_reference_single(
            &image(vec![[0.8; 3]; 4], 2, 2),
            &case,
            &json!({"id":"p","pixels_per_degree":60,"hotspots":{"count":1,"min_area_px":1}}),
            &temp.path().join("out"),
            temp.path(),
        )
        .unwrap();
        assert_eq!(result["reference_review"]["geometry"], "pixel_aligned");
        let measurements = result["measurements"].as_array().unwrap();
        let distance = measurements
            .iter()
            .find(|m| m["metric"] == "reference.delta_e00.median.simulation_distance")
            .unwrap();
        assert_eq!(distance["sample_count"], 2);
        assert!(distance["value"].as_f64().unwrap() > 0.);
        let flip = measurements
            .iter()
            .find(|m| m["metric"] == "reference.flip.mean.simulation_distance")
            .unwrap();
        assert_eq!(flip["availability"], "unavailable");
        assert!(flip.get("value").is_none());
        assert!(
            result["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["role"] == "reference_common_area_mask")
        );
        assert!(
            measurements
                .iter()
                .all(|m| !m["metric"].as_str().unwrap().contains("baseline"))
        );
    }

    #[test]
    fn single_material_curve_retains_real_simulation_residual_only() {
        let temp = tempfile::tempdir().unwrap();
        let coordinate = json!({"name":"exposure","units":"stops"});
        let conditions = json!({"temperature_c":20});
        common::dump(&temp.path().join("material.json"),&json!({"schema_version":1,"domain":"native_rgb","units":"native channel value","instrument":"fixture","conditions":conditions,"coordinate":coordinate,"curves":[{"id":"red","channel":"R","coordinates":[0.,1.],"values":[0.,1.]}]})).unwrap();
        let reference = json!({"id":"r","path":"material.json","scope":"material","domain":"native_rgb","units":"native channel value","instrument":"fixture","conditions":conditions});
        let case = json!({"id":"c","seeds":[0],"reference":reference,"regions":[{"id":"left","geometry":[0,0,1,1]},{"id":"right","geometry":[1,0,1,1]}],"probe_response":{"domain":"native_rgb","units":"native channel value","instrument":"fixture","conditions":conditions,"coordinate":coordinate,"estimator":"native_channel_mean","curves":[{"id":"red","channel":"R","samples":[{"coordinate":0.,"region_id":"left"},{"coordinate":1.,"region_id":"right"}]}]}});
        let output = temp.path().join("out");
        let result = compare_reference_single(
            &image(vec![[0.2; 3], [0.8; 3]], 2, 1),
            &case,
            &json!({"id":"p"}),
            &output,
            temp.path(),
        )
        .unwrap();
        assert!(result["measurements"][0]["value"].as_f64().unwrap() > 0.);
        let artifact = result["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["role"] == "reference_material_curve")
            .unwrap();
        let curve = documents::load_json_document(&output.join(artifact["path"].as_str().unwrap()))
            .unwrap();
        assert_eq!(curve["simulation"], json!([0.2, 0.8]));
        assert!(curve.get("baseline").is_none());
        assert!(curve.get("simulation_residual").is_some());
    }
}
