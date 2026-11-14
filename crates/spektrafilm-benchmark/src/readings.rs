//! Reading a retained measurement and publishing its record.
//!
//! A run publishes stage-evidence bundles beside its record; a reading
//! endpoint reads the requested stages from one bundle and summarizes them,
//! then writes its own record beside the readings it took. The renderer
//! writes these bytes and never reads them back: this module is the research
//! side of that boundary, and it depends on nothing the renderer owns.
//!
//! The stage and entry vocabularies live here because they name what a
//! retained buffer is, not what the renderer does with one.

use std::fs::OpenOptions;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::common::{Error, Result};
use crate::documents;

impl MeasureEntry {
    /// Every entry in chain order.
    pub const ALL: [MeasureEntry; 4] = [
        MeasureEntry::Spectra,
        MeasureEntry::RgbIn,
        MeasureEntry::LogEFilm,
        MeasureEntry::CmyFilm,
    ];

    /// The declared name of the entry, as the command surface spells it.
    pub const fn name(self) -> &'static str {
        match self {
            MeasureEntry::Spectra => "spectra",
            MeasureEntry::RgbIn => "rgb_in",
            MeasureEntry::LogEFilm => "log_e_film",
            MeasureEntry::CmyFilm => "cmy_film",
        }
    }

    /// The stimulus domain the entry requires.
    pub const fn domain(self) -> &'static str {
        match self {
            MeasureEntry::Spectra => "reflectance",
            MeasureEntry::RgbIn => "scene_rgb",
            MeasureEntry::LogEFilm => "film_log_e",
            MeasureEntry::CmyFilm => "film_density",
        }
    }
}

impl MeasureStage {
    /// Every readable stage in chain order.
    pub const ALL: [MeasureStage; 6] = [
        MeasureStage::RgbIn,
        MeasureStage::LogEFilm,
        MeasureStage::CmyFilm,
        MeasureStage::LogEPrint,
        MeasureStage::CmyPrint,
        MeasureStage::RgbOut,
    ];

    /// The stage-evidence identifier this stage reads.
    pub const fn id(self) -> &'static str {
        match self {
            MeasureStage::RgbIn => "rgb_in",
            MeasureStage::LogEFilm => "log_e_film",
            MeasureStage::CmyFilm => "cmy_film",
            MeasureStage::LogEPrint => "log_e_print",
            MeasureStage::CmyPrint => "cmy_print",
            MeasureStage::RgbOut => "rgb_out",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum MeasureEntry {
    /// Measured reflectance spectra.
    #[value(name = "spectra")]
    Spectra,
    /// Scene RGB after input decoding.
    #[value(name = "rgb_in")]
    RgbIn,
    /// Film raw log exposure (CMY).
    #[value(name = "log_e_film")]
    LogEFilm,
    /// Film density (CMY).
    #[value(name = "cmy_film")]
    CmyFilm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum MeasureStage {
    /// Scene RGB as the chain reads it after input decoding.
    #[value(name = "rgb_in")]
    RgbIn,
    /// Film raw log exposure.
    #[value(name = "log_e_film")]
    LogEFilm,
    /// Film density.
    #[value(name = "cmy_film")]
    CmyFilm,
    /// Print raw log exposure.
    #[value(name = "log_e_print")]
    LogEPrint,
    /// Print density.
    #[value(name = "cmy_print")]
    CmyPrint,
    /// Final pipeline output before saving.
    #[value(name = "rgb_out")]
    RgbOut,
}

/// Streaming SHA-256 of one file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        Error::new(
            "dependency_read",
            format!("hashing {}: {error}", path.display()),
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            Error::new(
                "dependency_read",
                format!("hashing {}: {error}", path.display()),
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub struct StageReading {
    pub id: String,
    pub domain: String,
    pub units: String,
    pub channel_order: String,
    /// The declared color space of the values, when the stage declares one.
    pub color_space: Option<String>,
    /// Whether the stored values are transfer-decoded, when the stage
    /// declares a color space.
    pub cctf_decoding: Option<bool>,
    /// The producer-declared coordinate space of the captured buffer.
    pub coordinate_space: String,
    pub width: u32,
    pub height: u32,
    pub sha256: String,
    pub values: Vec<f64>,
}

impl StageReading {
    /// The three channel values of one sample of a one-row measurement.
    pub fn sample(&self, index: usize) -> [f64; 3] {
        let base = index * 3;
        [
            self.values[base],
            self.values[base + 1],
            self.values[base + 2],
        ]
    }
}

/// A stage record of the executed run that carries no values.
pub struct StageAvailability {
    pub id: String,
    pub availability: String,
    pub reason: Option<String>,
}

/// Read the requested stages from a published stage-evidence bundle.
pub fn read_stage_bundle(
    directory: &Path,
    wanted: &[String],
) -> Result<(Vec<StageReading>, Vec<StageAvailability>), String> {
    let manifest_path = directory.join("stage-evidence.json");
    let bytes = std::fs::read(&manifest_path)
        .map_err(|e| format!("reading {}: {e}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("parsing {}: {e}", manifest_path.display()))?;
    if manifest.get("execution_status").and_then(Value::as_str) != Some("completed") {
        return Err(format!(
            "{} is not a completed stage capture",
            manifest_path.display()
        ));
    }
    let records = manifest
        .get("stages")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{} has no stage records", manifest_path.display()))?;
    let mut readings = Vec::new();
    let mut availability = Vec::new();
    for record in records {
        let id = record
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if !wanted.iter().any(|stage| stage == &id) {
            continue;
        }
        match record.get("availability").and_then(Value::as_str) {
            Some("available") => readings.push(read_stage(directory, record, id)?),
            other => availability.push(StageAvailability {
                id,
                availability: other.unwrap_or("unknown").to_owned(),
                reason: record
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            }),
        }
    }
    Ok((readings, availability))
}

fn read_stage(directory: &Path, record: &Value, id: String) -> Result<StageReading, String> {
    let layout = record
        .get("layout")
        .ok_or_else(|| format!("stage {id} carries no layout"))?;
    let width = layout
        .get("width")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("stage {id} layout has no width"))? as u32;
    let height = layout
        .get("height")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("stage {id} layout has no height"))? as u32;
    if layout.get("dtype").and_then(Value::as_str) != Some("f64")
        || layout.get("endianness").and_then(Value::as_str) != Some("little")
        || layout.get("channels").and_then(Value::as_u64) != Some(3)
    {
        return Err(format!("stage {id} has an unsupported buffer layout"));
    }
    let relative_path = record
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("stage {id} has no buffer path"))?;
    let path = documents::safe_artifact_path(directory, relative_path)
        .map_err(|error| error.to_string())?;
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("reading stage {id} buffer {}: {error}", path.display()))?;
    let expected = width as usize * height as usize * 3;
    if bytes.len() != expected * 8 {
        return Err(format!(
            "stage {id} buffer {} has {} bytes, expected {}",
            path.display(),
            bytes.len(),
            expected * 8
        ));
    }
    // The declaration must stay true: a buffer that no longer matches the
    // digest its manifest records cannot be attributed to this run.
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let declared = record
        .get("sha256")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if declared != digest {
        return Err(format!(
            "stage {id} buffer {} hashes {digest} but the manifest declares {declared}",
            path.display()
        ));
    }
    let values = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| f64::from_le_bytes(*chunk))
        .collect();
    Ok(StageReading {
        id,
        width,
        height,
        domain: record
            .get("domain")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        units: record
            .get("units")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        channel_order: layout
            .get("channel_order")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        color_space: record
            .get("color")
            .and_then(|color| color.get("color_space"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        cctf_decoding: record
            .get("color")
            .and_then(|color| color.get("cctf_decoding"))
            .and_then(Value::as_bool),
        coordinate_space: record
            .get("geometry")
            .and_then(|geometry| geometry.get("coordinate_space"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        sha256: record
            .get("sha256")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        values,
    })
}

/// One group summary of finite values.
pub struct Statistics {
    pub count: usize,
    pub mean: f64,
    pub median: f64,
    /// 95th percentile with the benchmark's interpolating estimator.
    pub p95: f64,
    pub min: f64,
    pub max: f64,
}

/// Summarize the finite values of one group; `NaN` counts report zero.
pub fn statistics(values: impl Iterator<Item = f64>) -> Statistics {
    let mut finite: Vec<f64> = values.filter(|value| value.is_finite()).collect();
    let count = finite.len();
    if count == 0 {
        return Statistics {
            count: 0,
            mean: f64::NAN,
            median: f64::NAN,
            p95: f64::NAN,
            min: f64::NAN,
            max: f64::NAN,
        };
    }
    let mean = finite.iter().sum::<f64>() / count as f64;
    finite.sort_by(|a, b| a.partial_cmp(b).expect("finite values"));
    let middle = count / 2;
    let median = if count.is_multiple_of(2) {
        (finite[middle - 1] + finite[middle]) / 2.0
    } else {
        finite[middle]
    };
    // One estimator for the whole workspace: the benchmark's interpolating
    // quantile (`spektrafilm_benchmark::metrics::quantile`), which this CLI
    // already depends on under `precision-f64`.
    let p95 = crate::metrics::quantile(&finite, 0.95).unwrap_or(f64::NAN);
    Statistics {
        count,
        mean,
        median,
        p95,
        min: finite[0],
        max: finite[count - 1],
    }
}

/// Write the named records into `root`, refusing to replace any existing
/// record and unlinking whatever this call created on failure.
///
/// `replace` permits overwriting a record a previous run left behind; each
/// file is still written as a complete unit through a temporary sibling.
pub fn write_named(
    root: &Path,
    records: &[(&str, &str)],
    refusal: &str,
    replace: bool,
) -> Result<(), String> {
    if !replace {
        for (name, _) in records {
            if root.join(name).exists() {
                return Err(format!("{refusal} in {}", root.display()));
            }
        }
    }
    std::fs::create_dir_all(root).map_err(|e| format!("creating {}: {e}", root.display()))?;
    let mut created: Vec<PathBuf> = Vec::with_capacity(records.len());
    let result = records.iter().try_for_each(|(name, text)| {
        let path = root.join(name);
        replace_file(&path, text).map(|()| created.push(path))
    });
    if let Err(error) = result {
        for path in created {
            let _ = std::fs::remove_file(path);
        }
        return Err(error);
    }
    Ok(())
}

/// Write one record as a complete unit: full write to a sibling temporary
/// file, then an atomic rename over the destination.
fn replace_file(path: &Path, text: &str) -> Result<(), String> {
    let temporary = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("json")
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|e| format!("creating {}: {e}", temporary.display()))?;
        file.write_all(text.as_bytes())
            .map_err(|e| format!("writing {}: {e}", temporary.display()))?;
        file.sync_all()
            .map_err(|e| format!("flushing {}: {e}", temporary.display()))?;
        std::fs::rename(&temporary, path).map_err(|e| format!("publishing {}: {e}", path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Quote a CSV field when it carries a separator, a quote, or a line break,
/// doubling any quote inside it. The artifact writers and their readers share
/// this rule: a sample identity may carry any of the three.
pub fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

/// A numeric CSV field, empty for a value that is not finite. The writers must
/// not invent a placeholder number for a value the chain could not produce.
pub fn number_field(value: f64) -> String {
    if value.is_finite() {
        format!("{value}")
    } else {
        String::new()
    }
}

/// Write the report pair into `root`, refusing to replace existing files.
///
/// A failed write unlinks whatever this call already created so no partial
/// report remains.
pub fn write(root: &Path, report: &str, samples: &str) -> Result<(), String> {
    write_named(
        root,
        &[("report.json", report), ("samples.csv", samples)],
        "refusing to replace an existing report",
        false,
    )
}

/// Write one named artifact set into `root`, refusing to replace existing
/// files. A failed write unlinks whatever this call already created.
pub fn write_measure_named(root: &Path, files: &[(&str, String)]) -> Result<(), String> {
    if let Some((name, _)) = files.iter().find(|(name, _)| root.join(name).exists()) {
        return Err(format!(
            "refusing to replace an existing {} in {}",
            name,
            root.display()
        ));
    }
    std::fs::create_dir_all(root).map_err(|e| format!("creating {}: {e}", root.display()))?;
    let mut created: Vec<std::path::PathBuf> = Vec::with_capacity(files.len());
    for (name, text) in files {
        let path = root.join(name);
        // Ownership is recorded before the write: a write that fails partway
        // still leaves a file that this call must unlink.
        if let Err(error) = create(&path, text) {
            let _ = std::fs::remove_file(&path);
            for path in created {
                let _ = std::fs::remove_file(path);
            }
            return Err(error);
        }
        created.push(path);
    }
    Ok(())
}

/// Publish a staged directory and a named artifact set together.
///
/// A measurement runs into `staging` and only names it when every check has
/// passed: on any failure nothing of the measurement survives, which is what
/// "no artifact unless the measurement completes" means for a reader that
/// retries the same destination. The staged directory is renamed to
/// `root/evidence` first; if the files then fail to write, the rename is undone.
pub fn publish_staged(root: &Path, staging: &Path, files: &[(&str, String)]) -> Result<(), String> {
    let evidence = root.join("evidence");
    std::fs::create_dir_all(root).map_err(|e| format!("creating {}: {e}", root.display()))?;
    std::fs::rename(staging, &evidence).map_err(|e| {
        format!(
            "publishing {} as {}: {e}",
            staging.display(),
            evidence.display()
        )
    })?;
    match write_measure_named(root, files) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_dir_all(&evidence);
            Err(error)
        }
    }
}

fn create(path: &Path, text: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("creating {}: {e}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("writing {}: {e}", path.display()))
}
