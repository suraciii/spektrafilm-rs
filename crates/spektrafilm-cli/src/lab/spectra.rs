//! The measured-spectra set: a JSON manifest plus the arrays beside it.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// One declared sample: identity, source and partition labels, array row.
#[derive(Deserialize, Clone)]
pub struct SampleLabel {
    pub id: String,
    pub source: String,
    pub partition: String,
    #[serde(default)]
    pub row: Option<usize>,
}

#[derive(Deserialize)]
struct RawManifest {
    schema_version: u32,
    #[serde(default)]
    name: Option<String>,
    grid_nm: RawGrid,
    valid_band: [f64; 2],
    reflectances: RawArray,
    samples: Vec<SampleLabel>,
}

#[derive(Deserialize)]
struct RawGrid {
    start: f64,
    end: f64,
    step: f64,
}

#[derive(Deserialize)]
struct RawArray {
    path: PathBuf,
    #[serde(default)]
    key: Option<String>,
}

/// A loaded measured-spectra set.
pub struct SpectraSet {
    pub name: Option<String>,
    pub manifest_path: PathBuf,
    pub manifest_sha256: String,
    pub array_path: PathBuf,
    pub array_key: Option<String>,
    pub array_sha256: String,
    pub wavelengths: Vec<f64>,
    pub labels: Vec<SampleLabel>,
    data: Vec<f64>,
    rows: Vec<usize>,
    row_stride: usize,
}

impl SpectraSet {
    pub fn sample_count(&self) -> usize {
        self.labels.len()
    }

    /// The sample's reflectance over the declared wavelength grid.
    pub fn row(&self, index: usize) -> &[f64] {
        let start = self.rows[index] * self.row_stride;
        &self.data[start..start + self.row_stride]
    }

    /// `false` when the reflectance is missing (non-finite) anywhere on the
    /// declared grid. Such a sample is invalid; it is not clipped,
    /// extrapolated, or repaired.
    pub fn band_valid(&self, index: usize) -> bool {
        self.row(index).iter().all(|value| value.is_finite())
    }
}

/// Load a spectra set from its manifest path.
pub fn load(path: &Path) -> Result<SpectraSet, String> {
    let manifest_bytes = std::fs::read(path)
        .map_err(|e| format!("reading spectra manifest {}: {e}", path.display()))?;
    let manifest: RawManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("parsing spectra manifest {}: {e}", path.display()))?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported spectra manifest schema_version {} (expected 1)",
            manifest.schema_version
        ));
    }
    let manifest_sha256 = sha256_hex(&manifest_bytes);

    let grid = &manifest.grid_nm;
    if !grid.start.is_finite()
        || !grid.end.is_finite()
        || !grid.step.is_finite()
        || grid.step <= 0.0
        || grid.end <= grid.start
    {
        return Err("spectra manifest grid_nm needs finite start < end and a positive step".into());
    }
    let intervals = (grid.end - grid.start) / grid.step;
    if (intervals - intervals.round()).abs() > 1e-8 || intervals > 100_000.0 {
        return Err("spectra manifest grid_nm needs an integral number of steps".into());
    }
    let columns = intervals.round() as usize + 1;
    let wavelengths: Vec<f64> = (0..columns)
        .map(|index| grid.start + grid.step * index as f64)
        .collect();

    let band = manifest.valid_band;
    let tolerance = 1e-9 * grid.step.abs().max(1.0);
    if !band[0].is_finite()
        || !band[1].is_finite()
        || (band[0] - grid.start).abs() > tolerance
        || (band[1] - grid.end).abs() > tolerance
    {
        return Err(format!(
            "spectra manifest valid_band {band:?} must cover the declared grid \
             {}..{} nm; declare the grid at the measured support",
            grid.start, grid.end
        ));
    }

    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let array_path = directory.join(&manifest.reflectances.path);
    let array_key = manifest.reflectances.key.clone();
    let (shape, data) = super::arrays::read_array(&array_path, array_key.as_deref())?;
    if shape.len() != 2 {
        return Err(format!(
            "reflectances array in {} must be two-dimensional, got {shape:?}",
            array_path.display()
        ));
    }
    let (rows_in_array, columns_in_array) = (shape[0], shape[1]);
    if columns_in_array != columns {
        return Err(format!(
            "reflectances array has {columns_in_array} columns but the declared grid \
             has {columns} samples"
        ));
    }
    if rows_in_array == 0 || data.len() != rows_in_array * columns_in_array {
        return Err(format!(
            "reflectances array in {} is malformed",
            array_path.display()
        ));
    }
    let array_sha256 = sha256_hex(
        &std::fs::read(&array_path)
            .map_err(|e| format!("hashing {}: {e}", array_path.display()))?,
    );

    if manifest.samples.is_empty() {
        return Err("spectra manifest declares no samples".into());
    }
    let mut rows = Vec::with_capacity(manifest.samples.len());
    for (index, label) in manifest.samples.iter().enumerate() {
        if label.id.is_empty() || label.source.is_empty() || label.partition.is_empty() {
            return Err(format!(
                "sample {index} declares an empty id, source, or partition"
            ));
        }
        let row = label.row.unwrap_or(index);
        if row >= rows_in_array {
            return Err(format!(
                "sample {:?} row {row} is outside the array ({rows_in_array} rows)",
                label.id
            ));
        }
        rows.push(row);
    }

    Ok(SpectraSet {
        name: manifest.name,
        manifest_path: path.to_path_buf(),
        manifest_sha256,
        array_path,
        array_key,
        array_sha256,
        wavelengths,
        labels: manifest.samples,
        data,
        rows,
        row_stride: columns,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_minimal_set_and_flags_missing_values() {
        let dir =
            std::env::temp_dir().join(format!("spektrafilm-lab-spectra-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Two samples, three bands; the second sample has a missing value.
        let header = "{'descr': '<f8', 'fortran_order': False, 'shape': (2, 3), }";
        let mut npy = Vec::new();
        npy.extend_from_slice(b"\x93NUMPY");
        npy.push(1);
        npy.push(0);
        npy.extend_from_slice(&(header.len() as u16).to_le_bytes());
        npy.extend_from_slice(header.as_bytes());
        for value in [0.1f64, 0.2, 0.3, 0.4, f64::NAN, 0.6] {
            npy.extend_from_slice(&value.to_le_bytes());
        }
        std::fs::write(dir.join("reflectances.npy"), npy).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{
                "schema_version": 1,
                "grid_nm": {"start": 380.0, "end": 390.0, "step": 5.0},
                "valid_band": [380.0, 390.0],
                "reflectances": {"path": "reflectances.npy"},
                "samples": [
                    {"id": "a", "source": "set", "partition": "warm"},
                    {"id": "b", "source": "set", "partition": "cool", "row": 1}
                ]
            }"#,
        )
        .unwrap();
        let set = load(&dir.join("manifest.json")).unwrap();
        assert_eq!(set.wavelengths, vec![380.0, 385.0, 390.0]);
        assert_eq!(set.sample_count(), 2);
        assert!(set.band_valid(0));
        assert!(!set.band_valid(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_a_grid_that_cannot_step() {
        let dir = std::env::temp_dir().join(format!("spektrafilm-lab-grid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The grid is validated before the array is read, so the referenced
        // array never has to exist.
        let cases = [
            (380.0, 390.0, 0.0, "grid_nm"),
            (380.0, 390.0, -5.0, "grid_nm"),
            (390.0, 380.0, 5.0, "grid_nm"),
            (380.0, 391.0, 5.0, "integral"),
        ];
        for (start, end, step, expected) in cases {
            std::fs::write(
                dir.join("manifest.json"),
                format!(
                    r#"{{
                        "schema_version": 1,
                        "grid_nm": {{"start": {start}, "end": {end}, "step": {step}}},
                        "valid_band": [380.0, 390.0],
                        "reflectances": {{"path": "missing.npy"}},
                        "samples": [{{"id": "a", "source": "set", "partition": "warm"}}]
                    }}"#
                ),
            )
            .unwrap();
            let Err(error) = load(&dir.join("manifest.json")) else {
                panic!("start {start} end {end} step {step} was accepted");
            };
            assert!(
                error.contains(expected),
                "start {start} end {end} step {step}: {error}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
