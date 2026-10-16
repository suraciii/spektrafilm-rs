//! Report writing for `spectral evaluate`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

/// Write the report pair into `root`, refusing to replace existing files.
///
/// The evaluation completes in memory before this is called; a failed write
/// unlinks whatever this call already created so no partial report remains.
pub fn write(root: &Path, report: &str, samples: &str) -> Result<(), String> {
    let report_path = root.join("report.json");
    let samples_path = root.join("samples.csv");
    if report_path.exists() || samples_path.exists() {
        return Err(format!(
            "refusing to replace an existing report in {}",
            root.display()
        ));
    }
    std::fs::create_dir_all(root).map_err(|e| format!("creating {}: {e}", root.display()))?;
    create(&report_path, report)
        .and_then(|()| create(&samples_path, samples))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&report_path);
            let _ = std::fs::remove_file(&samples_path);
        })
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
