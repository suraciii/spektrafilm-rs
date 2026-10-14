use crate::common::{
    self, Availability, Error, EvidenceBuilder, Measurement as SharedMeasurement, Result,
};
use crate::image_data;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use super::{COLOR_TRANSFORM, METRIC_VERSION, err};

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRole {
    DifferenceMap,
    GrainSpectrum,
    SpectrumCoordinates,
    GrainRadialSpectrum,
    GrainAutocovariance,
    GrainAutocorrelation,
    GrainResidual,
    GrainSignedDifference,
    HalationEdgeSpillProfile,
    HalationProfile,
    EdgeResponse,
    EdgeSfr,
    LocalContrastMap,
    ToneResponse,
}
/// Appearance measurements add the case identity the shared metric payload
/// does not carry; `serde(flatten)` keeps the published key set unchanged.
#[derive(Serialize)]
struct Measurement {
    case_id: Option<String>,
    seed_population: Vec<Value>,
    #[serde(flatten)]
    shared: common::Measurement,
}
#[derive(Serialize)]
struct Artifact {
    id: String,
    path: String,
    sha256: String,
    media_type: &'static str,
    role: ArtifactRole,
    shape: Vec<usize>,
    dtype: &'static str,
    coordinate_space: &'static str,
    #[serde(flatten)]
    metadata: BTreeMap<String, Value>,
}
#[derive(Serialize)]
struct Finding {
    id: String,
    case_id: Option<String>,
    region_id: String,
    metric: String,
    decision: &'static str,
    code: String,
    message: String,
    evidence: Vec<String>,
}
pub struct Evidence {
    out: PathBuf,
    case_id: Option<String>,
    seeds: Vec<Value>,
    body: EvidenceBuilder<Measurement, Finding, Artifact>,
    pub extra: BTreeMap<String, Value>,
}
impl Evidence {
    pub fn new(protocol: &Value, out: &Path) -> Self {
        Self {
            out: out.into(),
            case_id: protocol
                .get("case_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            seeds: protocol
                .get("seed_population")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            body: EvidenceBuilder::default(),
            extra: BTreeMap::new(),
        }
    }
    pub fn failure(
        &mut self,
        region: &str,
        metric: &str,
        error: &Error,
        refs: &[String],
        execution: bool,
    ) {
        self.body.findings.push(Finding {
            id: format!("{region}:{metric}:{}", error.code),
            case_id: self.case_id.clone(),
            region_id: region.into(),
            metric: metric.into(),
            decision: "unavailable",
            code: error.code.clone(),
            message: error.message.clone(),
            evidence: refs.to_vec(),
        });
        if execution {
            self.body.mark_failed();
        }
    }
    // Numeric payload is kept explicit at this boundary; grouping it would obscure metric schema fields.
    #[allow(clippy::too_many_arguments)]
    pub fn scalar(
        &mut self,
        region: &str,
        metric: &str,
        value: Option<f64>,
        count: usize,
        domain: &str,
        units: &str,
        estimator: &str,
        refs: &[String],
        reason: Option<&str>,
    ) {
        let valid = reason.is_none() && value.is_some_and(f64::is_finite);
        if !valid && reason.is_none() {
            self.failure(
                region,
                metric,
                &err("computation_failure", "Scalar is nonfinite"),
                refs,
                true,
            );
        }
        self.body.measurements.push(Measurement {
            case_id: self.case_id.clone(),
            seed_population: self.seeds.clone(),
            shared: SharedMeasurement {
                region_id: region.into(),
                metric: metric.into(),
                metric_version: METRIC_VERSION,
                domain: domain.into(),
                units: units.into(),
                estimator: estimator.into(),
                sample_count: count,
                artifact_refs: refs.to_vec(),
                availability: if valid {
                    Availability::Available
                } else {
                    Availability::Unavailable
                },
                value: if valid { value } else { None },
                reason: if valid {
                    None
                } else {
                    Some(reason.unwrap_or("computation_failure").into())
                },
            },
        });
    }
    pub fn artifact(
        &mut self,
        name: &str,
        values: &[f64],
        shape: &[usize],
        role: ArtifactRole,
        metadata: Value,
    ) -> Result<String> {
        if !values.iter().all(|v| v.is_finite()) {
            return Err(err(
                "computation_failure",
                format!("Artifact contains nonfinite samples: {name}"),
            ));
        }
        fs::create_dir_all(&self.out)?;
        let filename = format!(
            "{}.npy",
            &format!("{:x}", Sha256::digest(name.as_bytes()))[..16]
        );
        let path = self.out.join(&filename);
        if path.exists() {
            return Err(err(
                "computation_failure",
                format!("Artifact already exists: {}", path.display()),
            ));
        }
        image_data::write_npy(&path, values, shape)?;
        let id = format!("{name}_artifact");
        self.body.artifacts.push(Artifact {
            id: id.clone(),
            path: filename,
            sha256: crate::common::sha256_file(&path)?,
            media_type: "application/x-npy",
            role,
            shape: shape.to_vec(),
            dtype: "float64",
            coordinate_space: "native_render",
            metadata: metadata
                .as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default(),
        });
        Ok(id)
    }
    pub fn finish(self) -> Value {
        let mut extra = self.extra;
        extra.insert("metric_version".into(), json!(METRIC_VERSION));
        extra.insert("color_transform".into(), json!(COLOR_TRANSFORM));
        self.body.finish(extra)
    }
}
pub struct GrainRetained {
    pub psd: Vec<f64>,
    pub radial: Vec<f64>,
    pub autocovariance: Vec<f64>,
    pub residual: Vec<f64>,
}
