//! Reading a retained measurement and publishing its record.
//!
//! A reading endpoint summarizes the values it read and writes its own record
//! beside them. The renderer writes these bytes and never reads them back:
//! this module is the research side of that boundary, and it depends on
//! nothing the renderer owns.

use std::fs::OpenOptions;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::common::{Error, Result};

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

/// One group summary of finite values.
pub struct Statistics {
    pub count: usize,
    pub mean: f64,
    pub median: f64,
    /// 95th percentile with the workspace's interpolating estimator.
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
    // One estimator for the whole workspace: the shared interpolating
    // quantile (`spektrafilm_lab::metrics::quantile`), which this CLI
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

fn create(path: &Path, text: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("creating {}: {e}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("writing {}: {e}", path.display()))
}
