//! Report writing for the lab commands: the report pair of `spectral evaluate`
//! and the named artifacts of `measure`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

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
/// The evaluation completes in memory before this is called; a failed write
/// unlinks whatever this call already created so no partial report remains.
pub fn write(root: &Path, report: &str, samples: &str) -> Result<(), String> {
    write_named(
        root,
        &[
            ("report.json", report.to_owned()),
            ("samples.csv", samples.to_owned()),
        ],
    )
}

/// Write one named artifact set into `root`, refusing to replace existing
/// files. A failed write unlinks whatever this call already created.
pub fn write_named(root: &Path, files: &[(&str, String)]) -> Result<(), String> {
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
    match write_named(root, files) {
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
