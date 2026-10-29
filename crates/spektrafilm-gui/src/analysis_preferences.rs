//! Independent application presentation preferences for image analysis.
//!
//! These preferences never enter photographic records, presets, or exports.
use std::io::Write;
use std::path::{Path, PathBuf};

pub(crate) fn preference_path(directory: &Path) -> PathBuf {
    directory.join("analysis_preferences.json")
}

/// Return `None` only when no preference file exists; malformed and unreadable
/// files are left untouched and reported to the caller.
pub(crate) fn load(directory: &Path) -> Result<Option<serde_json::Value>, String> {
    let path = preference_path(directory);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    if !value.is_object() {
        return Err(format!("{}: expected an object", path.display()));
    }
    Ok(Some(value))
}

/// Publish a complete sibling temporary file, leaving the previous file intact
/// if serialization or writing fails.
pub(crate) fn save(directory: &Path, value: &serde_json::Value) -> Result<(), String> {
    let path = preference_path(directory);
    let write = || -> Result<(), Box<dyn std::error::Error>> {
        if !value.is_object() {
            return Err("analysis preferences must be an object".into());
        }
        let bytes = serde_json::to_vec_pretty(value)?;
        std::fs::create_dir_all(directory)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".analysis-preferences-")
            .suffix(".tmp")
            .tempfile_in(directory)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&path)?;
        Ok(())
    };
    write().map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn absent_preferences_round_trip_without_using_photograph_state() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(load(directory.path()).unwrap(), None);
        let preferences = json!({"visible": true, "mode": "Waveform", "height": 280});
        save(directory.path(), &preferences).unwrap();
        assert_eq!(load(directory.path()).unwrap(), Some(preferences));
    }

    #[test]
    fn invalid_preferences_are_reported_without_replacing_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = preference_path(directory.path());
        std::fs::write(&path, b"not json").unwrap();
        assert!(load(directory.path()).is_err());
        assert!(save(directory.path(), &json!(null)).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"not json");
    }
}
