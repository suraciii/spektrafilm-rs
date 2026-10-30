//! Independent application presentation preferences for endpoint warnings.
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ClippingPreferences {
    pub shadows: bool,
    pub highlights: bool,
}

pub(crate) fn preference_path(directory: &Path) -> PathBuf {
    directory.join("clipping_preferences.json")
}

/// Return `None` only when the preference file does not exist.
pub(crate) fn load(directory: &Path) -> Result<Option<ClippingPreferences>, String> {
    let path = preference_path(directory);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// Atomically replace the preference file with a complete sibling temporary file.
pub(crate) fn save(directory: &Path, value: &ClippingPreferences) -> Result<(), String> {
    let path = preference_path(directory);
    let write = || -> Result<(), Box<dyn std::error::Error>> {
        let bytes = serde_json::to_vec_pretty(value)?;
        std::fs::create_dir_all(directory)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".clipping-preferences-")
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

    #[test]
    fn absent_defaults_and_independent_flags_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(load(directory.path()).unwrap(), None);
        assert_eq!(
            ClippingPreferences::default(),
            ClippingPreferences {
                shadows: false,
                highlights: false
            }
        );
        for preferences in [
            ClippingPreferences {
                shadows: true,
                highlights: false,
            },
            ClippingPreferences {
                shadows: false,
                highlights: true,
            },
            ClippingPreferences {
                shadows: true,
                highlights: true,
            },
        ] {
            save(directory.path(), &preferences).unwrap();
            assert_eq!(load(directory.path()).unwrap(), Some(preferences));
        }
        std::fs::write(preference_path(directory.path()), b"{\"highlights\":true}").unwrap();
        assert_eq!(
            load(directory.path()).unwrap(),
            Some(ClippingPreferences {
                shadows: false,
                highlights: true
            })
        );
    }

    #[test]
    fn malformed_and_unknown_fields_are_reported_without_replacing_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = preference_path(directory.path());
        for bytes in [
            b"not json".as_slice(),
            b"{\"shadows\":true,\"unexpected\":false}".as_slice(),
            b"null".as_slice(),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(load(directory.path()).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn failed_publish_keeps_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let path = preference_path(directory.path());
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("sentinel"), b"unchanged").unwrap();
        assert!(
            save(
                directory.path(),
                &ClippingPreferences {
                    shadows: true,
                    highlights: false
                }
            )
            .is_err()
        );
        assert_eq!(std::fs::read(path.join("sentinel")).unwrap(), b"unchanged");
    }
}
