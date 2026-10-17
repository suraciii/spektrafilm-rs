//! Stimulus documents: measured spectra, or independent sample points at a
//! declared chain boundary.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::super::cli::MeasureEntry;
use super::super::spectra::{self, SampleLabel, SpectraSet};

/// A loaded stimulus set.
pub enum Stimuli {
    /// Measured reflectance spectra on the profile's wavelength grid.
    Spectra(Box<SpectraSet>),
    /// Independent sample points for a numeric boundary.
    Points(Box<PointsSet>),
}

impl Stimuli {
    pub fn sample_count(&self) -> usize {
        match self {
            Stimuli::Spectra(set) => set.sample_count(),
            Stimuli::Points(set) => set.sample_count(),
        }
    }

    pub fn labels(&self) -> &[SampleLabel] {
        match self {
            Stimuli::Spectra(set) => &set.labels,
            Stimuli::Points(set) => &set.labels,
        }
    }

    pub fn manifest_record(&self) -> serde_json::Value {
        match self {
            Stimuli::Spectra(set) => serde_json::json!({
                "kind": "stimuli",
                "domain": "reflectance",
                "path": set.manifest_path.display().to_string(),
                "sha256": set.manifest_sha256,
                "array": {
                    "path": set.array_path.display().to_string(),
                    "key": set.array_key,
                    "sha256": set.array_sha256,
                },
                "name": set.name,
                "color_space": set.color_space,
                "cctf_decoding": set.cctf_decoding,
                "condition": match (&set.condition_reference, &set.condition_target) {
                    (Some(reference), Some(target)) => serde_json::json!({
                        "reference": reference,
                        "target": target,
                    }),
                    _ => serde_json::Value::Null,
                },
            }),
            Stimuli::Points(set) => serde_json::json!({
                "kind": "stimuli",
                "domain": set.domain,
                "color_space": set.color_space,
                "cctf_decoding": set.cctf_decoding,
                "path": set.manifest_path.display().to_string(),
                "sha256": set.manifest_sha256,
                "array": {
                    "path": set.array_path.display().to_string(),
                    "key": set.array_key,
                    "sha256": set.array_sha256,
                },
                "name": set.name,
            }),
        }
    }
}

/// A points manifest: `N` by 3 samples for one numeric boundary.
pub struct PointsSet {
    pub name: Option<String>,
    pub domain: String,
    pub color_space: Option<String>,
    /// Whether the stored values are transfer-encoded, exactly as a raster
    /// input to `process` declares it.
    pub cctf_decoding: Option<bool>,
    pub manifest_path: PathBuf,
    pub manifest_sha256: String,
    pub array_path: PathBuf,
    pub array_key: Option<String>,
    pub array_sha256: String,
    pub labels: Vec<SampleLabel>,
    data: Vec<f64>,
    rows: Vec<usize>,
}

impl PointsSet {
    pub fn sample_count(&self) -> usize {
        self.labels.len()
    }

    /// Whether the chain must decode the declared values before filming.
    pub fn cctf_decoding(&self) -> bool {
        self.cctf_decoding.unwrap_or(false)
    }

    /// The sample's three channel values.
    pub fn row(&self, index: usize) -> [f64; 3] {
        let base = self.rows[index] * 3;
        [self.data[base], self.data[base + 1], self.data[base + 2]]
    }

    /// `false` when the sample carries a non-finite value. Such a sample is
    /// invalid; it is not clipped, extrapolated, or repaired.
    pub fn row_valid(&self, index: usize) -> bool {
        self.row(index).iter().all(|value| value.is_finite())
    }
}

#[derive(Deserialize)]
struct RawPointsManifest {
    schema_version: u32,
    #[serde(default)]
    name: Option<String>,
    domain: String,
    #[serde(default)]
    color_space: Option<String>,
    #[serde(default)]
    cctf_decoding: Option<bool>,
    points: RawArray,
    samples: Vec<SampleLabel>,
}

#[derive(Deserialize)]
struct RawArray {
    path: PathBuf,
    #[serde(default)]
    key: Option<String>,
}

/// Load the stimulus document a declared entry requires.
pub fn load(path: &Path, entry: MeasureEntry) -> Result<Stimuli, String> {
    if entry == MeasureEntry::Spectra {
        // The spectra path reuses the shared loader, so its labels are read
        // here: a stimulus set joins its readings by sample identity, and a
        // repeated identity would join one row to two.
        let set = spectra::load(path)?;
        reject_duplicate_ids(&set.labels)?;
        return Ok(Stimuli::Spectra(Box::new(set)));
    }
    let manifest_bytes = std::fs::read(path)
        .map_err(|e| format!("reading points manifest {}: {e}", path.display()))?;
    let manifest: RawPointsManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("parsing points manifest {}: {e}", path.display()))?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported points manifest schema_version {} (expected 1)",
            manifest.schema_version
        ));
    }
    let expected = entry.domain();
    if manifest.domain != expected {
        return Err(format!(
            "points manifest declares domain {:?} but --enter {} reads {expected:?}",
            manifest.domain,
            entry.name()
        ));
    }
    if expected == "scene_rgb" {
        if manifest.color_space.is_none() {
            return Err(
                "a scene_rgb points manifest must declare color_space; the values are read in that space"
                    .into(),
            );
        }
        if manifest.cctf_decoding.is_none() {
            return Err(
                "a scene_rgb points manifest must declare cctf_decoding; the values are read in that transfer state"
                    .into(),
            );
        }
    } else {
        if manifest.color_space.is_some() {
            return Err(format!(
                "a {expected} points manifest must not declare color_space"
            ));
        }
        if manifest.cctf_decoding.is_some() {
            return Err(format!(
                "a {expected} points manifest must not declare cctf_decoding; its values are film-domain numbers"
            ));
        }
    }
    if manifest.samples.is_empty() {
        return Err("points manifest declares no samples".into());
    }
    reject_duplicate_ids(&manifest.samples)?;
    let manifest_sha256 = format!("{:x}", Sha256::digest(&manifest_bytes));
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let array_path = directory.join(&manifest.points.path);
    let array_key = manifest.points.key.clone();
    let (shape, data) = super::super::arrays::read_array(&array_path, array_key.as_deref())?;
    if shape.len() != 2 || shape[1] != 3 {
        return Err(format!(
            "points array in {} must have shape N x 3, got {shape:?}",
            array_path.display()
        ));
    }
    let rows_in_array = shape[0];
    if rows_in_array == 0 || data.len() != rows_in_array * 3 {
        return Err(format!(
            "points array in {} declares {rows_in_array} rows but carries {} values",
            array_path.display(),
            data.len()
        ));
    }
    let mut rows = Vec::with_capacity(manifest.samples.len());
    for (index, label) in manifest.samples.iter().enumerate() {
        let row = label.row.unwrap_or(index);
        if row >= rows_in_array {
            return Err(format!(
                "sample {:?} names row {row} but the points array has {rows_in_array} rows",
                label.id
            ));
        }
        rows.push(row);
    }
    let array_bytes = std::fs::read(&array_path)
        .map_err(|e| format!("reading points array {}: {e}", array_path.display()))?;
    Ok(Stimuli::Points(Box::new(PointsSet {
        name: manifest.name,
        domain: manifest.domain,
        color_space: manifest.color_space,
        cctf_decoding: manifest.cctf_decoding,
        manifest_path: path.to_path_buf(),
        manifest_sha256,
        array_path,
        array_key,
        array_sha256: format!("{:x}", Sha256::digest(&array_bytes)),
        labels: manifest.samples,
        data,
        rows,
    })))
}

/// One `SampleLabel`'s group key.
pub fn group_of(label: &SampleLabel) -> (String, String) {
    (label.source.clone(), label.partition.clone())
}

/// A stimulus set joins its readings by sample identity, so one identity must
/// name one sample. A repeated id is a second authority for the same reading,
/// and it fails before any execution or publication.
fn reject_duplicate_ids(labels: &[SampleLabel]) -> Result<(), String> {
    for (index, label) in labels.iter().enumerate() {
        if let Some(earlier) = labels[..index]
            .iter()
            .position(|other| other.id == label.id)
        {
            return Err(format!(
                "stimulus declares duplicate sample id {:?} at rows {} and {}",
                label.id,
                earlier + 1,
                index + 1
            ));
        }
    }
    Ok(())
}
