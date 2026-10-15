use super::io::{field, text};
use super::{COLOR_TRANSFORM, METRIC_VERSION, SIDES};
use crate::common::{self, Availability, EvidenceBuilder, Result};
use crate::image_data;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub enum ArtifactRole {
    Native,
    AlignedDisplay,
    Aligned,
    CommonArea,
    BaselineMaps,
    CandidateMaps,
    RegionalStatistics,
    RegionPairStatistics,
    MaterialSource,
    MaterialCurve,
}
impl ArtifactRole {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Native => "reference_native",
            Self::AlignedDisplay => "reference_aligned_display",
            Self::Aligned => "reference_aligned",
            Self::CommonArea => "reference_common_area_mask",
            Self::BaselineMaps => "reference_baseline_distance_maps",
            Self::CandidateMaps => "reference_candidate_distance_maps",
            Self::RegionalStatistics => "reference_regional_statistics",
            Self::RegionPairStatistics => "reference_region_pair_statistics",
            Self::MaterialSource => "reference_material_source",
            Self::MaterialCurve => "reference_material_curve",
        }
    }
}
/// Reference measurements extend the shared row with the frozen protocol and
/// reference identity; `serde(flatten)` merges the shared keys into the same
/// object, so the serialized key set is unchanged.
#[derive(Serialize)]
pub struct Measurement {
    case_id: String,
    seed_population: Value,
    #[serde(flatten)]
    pub(super) shared: common::Measurement,
    reference_id: String,
    protocol_id: Value,
}
pub struct Comparison<'a> {
    pub(super) reference: &'a Value,
    case_id: String,
    seeds: Value,
    protocol_id: Value,
    prefix: String,
    out: &'a Path,
    sequence: usize,
    pub(super) body: EvidenceBuilder<Measurement, Value, Value>,
    pub(super) review: Value,
    pub(super) single: bool,
    pub(super) protocol: Value,
}
impl<'a> Comparison<'a> {
    pub(super) fn new(
        reference: &'a Value,
        case: &Value,
        protocol: &Value,
        out: &'a Path,
    ) -> Result<Self> {
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
    pub(super) fn limitation(&mut self, message: impl Into<String>) {
        self.review["limitations"]
            .as_array_mut()
            .unwrap()
            .push(json!(message.into()));
    }
    pub(super) fn review_ref(&mut self, id: &str) {
        self.review["artifact_refs"]
            .as_array_mut()
            .unwrap()
            .push(json!(id));
    }
    pub(super) fn finding(
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
    // Per-capture entries carry their own availability: a `None` capture is
    // recorded with `reason` while available captures keep their values.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn distances(
        &mut self,
        region: &str,
        metric: &str,
        domain: &str,
        units: &str,
        estimator: &str,
        count: usize,
        values: Option<Vec<Option<f64>>>,
        reason: Option<&str>,
        refs: &[String],
    ) {
        let mut reason = reason.map(str::to_owned);
        let captures = if self.single { 1 } else { 2 };
        let mut rows = vec![None; if self.single { 1 } else { 3 }];
        if let Some(values) = values {
            if values.len() != captures {
                reason = Some("computation_failure".into());
                self.finding(region, metric, "computation_failure", true, refs);
            } else {
                let mut expanded = values;
                if !self.single {
                    expanded.push(match (expanded[0], expanded[1]) {
                        (Some(a), Some(b)) => Some(b - a),
                        _ => None,
                    });
                }
                if expanded.iter().flatten().any(|x| !x.is_finite()) {
                    reason = Some("computation_failure".into());
                    self.finding(region, metric, "computation_failure", true, refs);
                } else {
                    if expanded.iter().any(Option::is_none) {
                        reason = Some(reason.unwrap_or("missing_evidence".into()));
                    }
                    rows = expanded;
                }
            }
        }
        let sides: &[&str] = if self.single {
            &["simulation_distance"]
        } else {
            &SIDES
        };
        for (i, side) in sides.iter().enumerate() {
            let available = rows[i].is_some();
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
                    availability: if available {
                        Availability::Available
                    } else {
                        Availability::Unavailable
                    },
                    value: rows[i],
                    reason: if available {
                        None
                    } else {
                        Some(reason.clone().unwrap_or("missing_evidence".into()))
                    },
                },
                reference_id: self.reference["id"].as_str().unwrap_or("").into(),
                protocol_id: self.protocol_id.clone(),
            });
        }
        if rows.iter().all(Option::is_some) {
            let v: Vec<f64> = rows.iter().flatten().copied().collect();
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
    /// Reference-owned anchor row without side expansion: the retained
    /// relation value every capture is compared against.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn relation_row(
        &mut self,
        region: &str,
        metric: &str,
        domain: &str,
        units: &str,
        estimator: &str,
        count: usize,
        value: Option<f64>,
        reason: Option<&str>,
        refs: &[String],
    ) {
        let full = format!("{metric}.{}", super::catalog::RELATION_SIDE);
        let mut reason = reason.map(str::to_owned);
        let mut value = value;
        if value.is_some_and(|v| !v.is_finite()) {
            reason = Some("computation_failure".into());
            value = None;
            self.finding(region, &full, "computation_failure", true, refs);
        }
        let available = value.is_some();
        self.body.measurements.push(Measurement {
            case_id: self.case_id.clone(),
            seed_population: self.seeds.clone(),
            shared: common::Measurement {
                region_id: region.into(),
                metric: full.clone(),
                metric_version: METRIC_VERSION,
                domain: domain.into(),
                units: units.into(),
                estimator: estimator.into(),
                sample_count: count,
                artifact_refs: refs.to_vec(),
                availability: if available {
                    Availability::Available
                } else {
                    Availability::Unavailable
                },
                value,
                reason: if available {
                    None
                } else {
                    Some(reason.clone().unwrap_or("missing_evidence".into()))
                },
            },
            reference_id: self.reference["id"].as_str().unwrap_or("").into(),
            protocol_id: self.protocol_id.clone(),
        });
        if available {
            self.body.findings.push(json!({"id":format!("{}:{region}:{full}:measured",self.prefix),"case_id":self.case_id,"region_id":region,"metric":full,"decision":"measured","evidence":refs,"reference_relation":value,"interpretation":"Reference pair separation anchor; pair closeness alone does not imply correct endpoint colors; no acceptance threshold is inferred."}));
        } else {
            self.finding(
                region,
                &full,
                reason.as_deref().unwrap_or("missing_evidence"),
                false,
                refs,
            );
        }
    }
    pub(super) fn artifact_row(
        &self,
        id: &str,
        path: &Path,
        role: &str,
        media: &str,
    ) -> Result<Value> {
        Ok(
            json!({"id":id,"path":path.file_name().unwrap().to_string_lossy(),"sha256":common::sha256_file(path)?,"media_type":media,"role":role,"reference_id":self.reference["id"],"provenance":self.reference.get("provenance").cloned().unwrap_or(json!({})),"redistribution":self.reference.get("provenance").and_then(|x|x.get("redistribution")).cloned().unwrap_or(json!("unknown"))}),
        )
    }
    pub(super) fn next_artifact(
        &mut self,
        role: ArtifactRole,
        extension: &str,
    ) -> (String, PathBuf) {
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
    pub(super) fn json_artifact(&mut self, role: ArtifactRole, payload: &Value) -> Result<String> {
        let (id, path) = self.next_artifact(role, "json");
        common::dump(&path, payload)?;
        self.body
            .artifacts
            .push(self.artifact_row(&id, &path, role.name(), "application/json")?);
        Ok(id)
    }
    pub(super) fn numeric_artifact(
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
    pub(super) fn rgb_artifact(
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
    pub(super) fn mask_artifact(&mut self, values: &[bool], dims: [usize; 2]) -> Result<String> {
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
    pub(super) fn preview(
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
    pub(super) fn finish(self) -> Value {
        let mut extra = BTreeMap::new();
        extra.insert("metric_version".into(), json!(METRIC_VERSION));
        extra.insert("reference_review".into(), self.review);
        self.body.finish(extra)
    }
}
pub fn array_metadata(row: &mut Value, shape: &[usize], dtype: &str) {
    row["layout"] = json!({"dtype":dtype,"axis_order":if shape.len()==3{"height,width,channel"}else{"height,width"}});
    row["dimensions"] = json!(shape);
    if shape.len() >= 2 {
        row["coordinates"] = json!({"system":"pixel_centers","x":{"start":0.5,"step":1,"count":shape[1]},"y":{"start":0.5,"step":1,"count":shape[0]}});
    }
}
pub fn save_display(path: &Path, values: &[[f64; 3]], dims: [usize; 2]) -> Result<()> {
    let codes: Vec<[u16; 3]> = values
        .iter()
        .map(|p| p.map(|x| (x.clamp(0., 1.) * 65535.).round_ties_even() as u16))
        .collect();
    image_data::write_png16_codes(path, &codes, dims[0], dims[1])
}
pub fn write_bool_npy(path: &Path, values: &[bool], shape: &[usize]) -> Result<()> {
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
