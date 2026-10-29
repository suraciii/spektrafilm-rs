use super::*;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresetGroup {
    #[serde(rename = "Built-in")]
    BuiltIn,
    #[serde(rename = "My Presets")]
    User,
}
/// Machine-readable library failure with the established English diagnostic.
#[derive(Debug, Clone)]
pub enum LibraryFault {
    DirectoryRead { detail: String },
    Document { detail: PresetDocumentError },
    NameMismatch { name: String, stem: String },
    DuplicateName { name: String },
}
impl std::fmt::Display for LibraryFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DirectoryRead { detail } => write!(f, "reading My Presets directory: {detail}"),
            Self::Document { detail } => std::fmt::Display::fmt(detail, f),
            Self::NameMismatch { name, stem } => {
                write!(f, "document name {name:?} differs from file stem {stem:?}")
            }
            Self::DuplicateName { name } => write!(f, "duplicate My Presets name {name:?}"),
        }
    }
}
impl Serialize for LibraryFault {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
#[derive(Debug, Clone)]
pub enum LibraryErrorReason {
    Document {
        path: Option<PathBuf>,
        error: Box<PresetDocumentError>,
    },
    Fault {
        path: PathBuf,
        fault: Box<LibraryFault>,
    },
    NameConflict {
        name: String,
    },
    UnknownName {
        name: String,
        suggestions: Vec<String>,
    },
    MissingPath,
    Io {
        operation: &'static str,
        path: PathBuf,
        detail: String,
    },
    ExportName {
        path: PathBuf,
        name: String,
    },
}
#[derive(Debug, Clone)]
pub struct LibraryError {
    pub reason: LibraryErrorReason,
    diagnostic: String,
}
impl LibraryError {
    fn new(reason: LibraryErrorReason, diagnostic: impl Into<String>) -> Self {
        Self {
            reason,
            diagnostic: diagnostic.into(),
        }
    }
    fn io(operation: &'static str, path: &Path, error: std::io::Error, diagnostic: String) -> Self {
        Self::new(
            LibraryErrorReason::Io {
                operation,
                path: path.to_owned(),
                detail: error.to_string(),
            },
            diagnostic,
        )
    }
}
impl From<PresetDocumentError> for LibraryError {
    fn from(error: PresetDocumentError) -> Self {
        let diagnostic = error.to_string();
        Self::new(
            LibraryErrorReason::Document {
                path: None,
                error: Box::new(error),
            },
            diagnostic,
        )
    }
}
impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.diagnostic)
    }
}
impl std::error::Error for LibraryError {}
#[derive(Debug, Clone, Serialize)]
pub struct LibraryEntry {
    pub group: PresetGroup,
    pub name: String,
    pub film_profile: Option<ProfileRef>,
    pub print_profile: Option<ProfileRef>,
    pub path: Option<PathBuf>,
    pub fault: Option<LibraryFault>,
    pub preset: Option<Preset>,
}
#[derive(Debug, Clone)]
pub struct PresetSelection {
    pub group: Option<PresetGroup>,
    pub path: Option<PathBuf>,
    pub shadowed_builtin: bool,
    pub preset: Preset,
}
#[derive(Debug, Clone)]
pub struct PresetLibrary {
    directory: PathBuf,
}
impl Default for PresetLibrary {
    fn default() -> Self {
        Self::new(crate::config::config_dir().join("presets"))
    }
}
pub(crate) fn name_order(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}
pub fn validate_name(name: &str) -> Result<(), String> {
    let lower = name.to_lowercase();
    let base = lower.split('.').next().unwrap_or("");
    let reserved = matches!(base, "con" | "prn" | "aux" | "nul")
        || ["com", "lpt"].iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
    if !(1..=64).contains(&name.chars().count())
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
        || name.ends_with('.')
        || lower.ends_with(".toml")
        || lower.ends_with(".json")
        || reserved
    {
        return Err(format!(
            "invalid preset name {name:?}: use 1–64 portable filename characters, without a carrier extension or reserved device name"
        ));
    }
    Ok(())
}
fn document_entry(group: PresetGroup, path: Option<PathBuf>, preset: Preset) -> LibraryEntry {
    LibraryEntry {
        group,
        name: preset.name.clone(),
        film_profile: preset.film_profile.clone(),
        print_profile: preset.print_profile.clone(),
        path,
        fault: None,
        preset: Some(preset),
    }
}
fn carrier(path: &Path) -> Result<bool, String> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("toml") => Ok(true),
        Some("json") => Ok(false),
        _ => Err(format!("unsupported preset carrier {}", path.display())),
    }
}
pub fn load_file(path: &Path) -> Result<Preset, String> {
    load_document(path).map_err(|error| format!("{}: {error}", path.display()))
}
pub fn load_file_detailed(path: &Path) -> Result<Preset, LibraryError> {
    load_document(path).map_err(|error| {
        let diagnostic = format!("{}: {error}", path.display());
        LibraryError::new(
            LibraryErrorReason::Document {
                path: Some(path.to_owned()),
                error: Box::new(error),
            },
            diagnostic,
        )
    })
}
fn carrier_detailed(path: &Path) -> Result<bool, LibraryError> {
    carrier(path).map_err(|diagnostic| {
        PresetDocumentError::new(
            PresetDocumentReason::Carrier {
                path: path.to_owned(),
            },
            diagnostic,
        )
        .into()
    })
}
fn validate_name_detailed(name: &str) -> Result<(), LibraryError> {
    validate_name(name).map_err(|diagnostic| {
        PresetDocumentError::new(
            PresetDocumentReason::Name {
                name: name.to_owned(),
            },
            diagnostic,
        )
        .into()
    })
}
fn load_document(path: &Path) -> Result<Preset, PresetDocumentError> {
    let is_toml = carrier(path).map_err(|detail| {
        PresetDocumentError::new(
            PresetDocumentReason::Carrier {
                path: path.to_owned(),
            },
            detail,
        )
    })?;
    let text = std::fs::read_to_string(path).map_err(|error| {
        PresetDocumentError::new(
            PresetDocumentReason::Read {
                path: path.to_owned(),
                detail: error.to_string(),
            },
            error.to_string(),
        )
    })?;
    if is_toml {
        Preset::from_toml_str_detailed(&text)
    } else {
        Preset::from_json_str_detailed(&text)
    }
}
fn user_scope_fault(directory: &Path, error: std::io::Error) -> LibraryEntry {
    LibraryEntry {
        group: PresetGroup::User,
        name: "My Presets".into(),
        film_profile: None,
        print_profile: None,
        path: Some(directory.to_owned()),
        fault: Some(LibraryFault::DirectoryRead {
            detail: error.to_string(),
        }),
        preset: None,
    }
}
fn name_conflict(name: &str) -> String {
    format!("My Presets name {name:?} already exists; choose a name for a renamed copy")
}
fn should_remove_old_path(old: &Path, new: &Path) -> Result<bool, LibraryError> {
    let Ok(old_entry) = std::fs::canonicalize(old) else {
        return Ok(false);
    };
    let new_entry = std::fs::canonicalize(new).map_err(|error| {
        let diagnostic = format!("resolving {}: {error}", new.display());
        LibraryError::io("resolve", new, error, diagnostic)
    })?;
    Ok(old_entry != new_entry)
}

impl PresetLibrary {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn scan(&self) -> Result<Vec<LibraryEntry>, String> {
        self.scan_detailed().map_err(|error| error.to_string())
    }
    fn scan_detailed(&self) -> Result<Vec<LibraryEntry>, LibraryError> {
        let mut entries = builtin_presets()
            .into_iter()
            .map(|p| document_entry(PresetGroup::BuiltIn, None, p))
            .collect::<Vec<_>>();
        let listing = match std::fs::read_dir(&self.directory) {
            Ok(listing) => listing,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(entries),
            Err(e) => {
                entries.push(user_scope_fault(&self.directory, e));
                entries.sort_by(|a, b| name_order(&a.name, &b.name));
                return Ok(entries);
            }
        };
        for item in listing {
            let item = match item {
                Ok(item) => item,
                Err(e) => {
                    entries.push(user_scope_fault(&self.directory, e));
                    break;
                }
            };
            let path = item.path();
            if carrier(&path).is_err() {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_owned();
            match load_document(&path)
                .map_err(|detail| LibraryFault::Document { detail })
                .and_then(|p| {
                    if p.name == stem {
                        Ok(p)
                    } else {
                        Err(LibraryFault::NameMismatch {
                            name: p.name,
                            stem: stem.clone(),
                        })
                    }
                }) {
                Ok(p) => entries.push(document_entry(PresetGroup::User, Some(path), p)),
                Err(e) => entries.push(LibraryEntry {
                    group: PresetGroup::User,
                    name: stem,
                    film_profile: None,
                    print_profile: None,
                    path: Some(path),
                    fault: Some(e),
                    preset: None,
                }),
            }
        }
        let mut counts = std::collections::BTreeMap::new();
        for e in &entries {
            if e.group == PresetGroup::User {
                *counts.entry(e.name.to_lowercase()).or_insert(0usize) += 1;
            }
        }
        for e in &mut entries {
            if e.group == PresetGroup::User && counts[&e.name.to_lowercase()] > 1 {
                e.fault = Some(LibraryFault::DuplicateName {
                    name: e.name.clone(),
                });
                e.preset = None;
            }
        }
        entries.sort_by(|a, b| name_order(&a.name, &b.name).then_with(|| a.path.cmp(&b.path)));
        Ok(entries)
    }
    pub fn suggest(&self, input: &str) -> Vec<String> {
        crate::suggest::closest(
            input,
            self.scan()
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.fault.is_none())
                .map(|e| e.name),
            crate::suggest::MAX_SUGGESTIONS,
        )
    }
    pub fn resolve_selector(&self, selector: &str) -> Result<PresetSelection, String> {
        self.resolve_selector_detailed(selector)
            .map_err(|error| error.to_string())
    }
    pub fn resolve_selector_detailed(
        &self,
        selector: &str,
    ) -> Result<PresetSelection, LibraryError> {
        if selector.ends_with(".toml") || selector.ends_with(".json") {
            let path = PathBuf::from(selector);
            return Ok(PresetSelection {
                group: None,
                preset: load_file_detailed(&path)?,
                path: Some(path),
                shadowed_builtin: false,
            });
        }
        let (group, name) = if let Some(name) = selector.strip_prefix("user:") {
            (Some(PresetGroup::User), name)
        } else if let Some(name) = selector.strip_prefix("built-in:") {
            (Some(PresetGroup::BuiltIn), name)
        } else {
            (None, selector)
        };
        validate_name_detailed(name)?;
        let entries = if group == Some(PresetGroup::BuiltIn) {
            builtin_presets()
                .into_iter()
                .map(|preset| document_entry(PresetGroup::BuiltIn, None, preset))
                .collect()
        } else {
            self.scan_detailed()?
        };
        let builtin = entries
            .iter()
            .any(|e| e.group == PresetGroup::BuiltIn && e.name.eq_ignore_ascii_case(name));
        for wanted in [PresetGroup::User, PresetGroup::BuiltIn] {
            if group.is_some_and(|g| g != wanted) {
                continue;
            }
            if let Some(entry) = entries
                .iter()
                .find(|e| e.group == wanted && e.name.to_lowercase() == name.to_lowercase())
            {
                if let Some(fault) = &entry.fault {
                    let path = entry.path.as_ref().unwrap();
                    return Err(LibraryError::new(
                        LibraryErrorReason::Fault {
                            path: path.clone(),
                            fault: Box::new(fault.clone()),
                        },
                        format!("{}: {fault}", path.display()),
                    ));
                }
                return Ok(PresetSelection {
                    group: Some(wanted),
                    path: entry.path.clone(),
                    shadowed_builtin: group.is_none() && wanted == PresetGroup::User && builtin,
                    preset: entry.preset.clone().unwrap(),
                });
            }
        }
        let suggestions = if group == Some(PresetGroup::BuiltIn) {
            crate::suggest::closest(
                name,
                entries.into_iter().map(|entry| entry.name),
                crate::suggest::MAX_SUGGESTIONS,
            )
        } else {
            self.suggest(name)
        };
        let diagnostic = format!(
            "unknown preset name {selector:?}; {}",
            crate::suggest::error_suffix(
                &suggestions,
                "use spektrafilm preset list to list presets"
            )
        );
        Err(LibraryError::new(
            LibraryErrorReason::UnknownName {
                name: selector.to_owned(),
                suggestions,
            },
            diagnostic,
        ))
    }
    pub fn load(&self, group: PresetGroup, name: &str) -> Result<PresetSelection, String> {
        self.load_detailed(group, name)
            .map_err(|error| error.to_string())
    }
    pub fn load_detailed(
        &self,
        group: PresetGroup,
        name: &str,
    ) -> Result<PresetSelection, LibraryError> {
        self.resolve_selector_detailed(&format!(
            "{}:{name}",
            if group == PresetGroup::User {
                "user"
            } else {
                "built-in"
            }
        ))
    }
    fn validate_write(preset: &Preset, data_dir: &Path) -> Result<(), LibraryError> {
        preset.validate_detailed()?;
        if matches!(preset.parameters, PresetParameters::Look(_)) {
            preset.resolve_detailed(data_dir, &RuntimeParams::default())?;
        }
        Ok(())
    }
    fn user_path(&self, name: &str) -> Result<PathBuf, LibraryError> {
        self.load_detailed(PresetGroup::User, name)?
            .path
            .ok_or_else(|| {
                LibraryError::new(LibraryErrorReason::MissingPath, "user document has no path")
            })
    }
    // Keep this file in place: unlinking it could split concurrent lock owners.
    fn lock_mutations(&self, create_directory: bool) -> Result<std::fs::File, LibraryError> {
        if create_directory {
            std::fs::create_dir_all(&self.directory).map_err(|error| {
                let diagnostic = error.to_string();
                LibraryError::io("create", &self.directory, error, diagnostic)
            })?;
        }
        let path = self.directory.join(".preset-library.lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| {
                let diagnostic = format!("opening library lock {}: {error}", path.display());
                LibraryError::io("open_lock", &path, error, diagnostic)
            })?;
        fs2::FileExt::lock_exclusive(&file).map_err(|error| {
            let diagnostic = format!("locking My Presets {}: {error}", self.directory.display());
            LibraryError::io("lock", &self.directory, error, diagnostic)
        })?;
        Ok(file)
    }
    fn ensure_new(&self, name: &str) -> Result<(), LibraryError> {
        validate_name_detailed(name)?;
        if self
            .scan_detailed()?
            .iter()
            .any(|e| e.group == PresetGroup::User && e.name.to_lowercase() == name.to_lowercase())
        {
            return Err(LibraryError::new(
                LibraryErrorReason::NameConflict {
                    name: name.to_owned(),
                },
                name_conflict(name),
            ));
        }
        Ok(())
    }
    pub fn save_new(&self, preset: &Preset, data_dir: &Path) -> Result<PathBuf, String> {
        self.save_new_detailed(preset, data_dir)
            .map_err(|error| error.to_string())
    }
    pub fn save_new_detailed(
        &self,
        preset: &Preset,
        data_dir: &Path,
    ) -> Result<PathBuf, LibraryError> {
        self.save_new_format_detailed(preset, data_dir, true)
    }
    /// Prefer the requested carrier, falling back to JSON for JSON-only sections.
    pub fn save_new_format(
        &self,
        preset: &Preset,
        data_dir: &Path,
        toml: bool,
    ) -> Result<PathBuf, String> {
        self.save_new_format_detailed(preset, data_dir, toml)
            .map_err(|error| error.to_string())
    }
    pub fn save_new_format_detailed(
        &self,
        preset: &Preset,
        data_dir: &Path,
        toml: bool,
    ) -> Result<PathBuf, LibraryError> {
        Self::validate_write(preset, data_dir)?;
        let _lock = self.lock_mutations(true)?;
        self.ensure_new(&preset.name)?;
        let (toml, text) = if toml {
            match preset.to_toml_detailed() {
                Ok(text) => (true, text),
                Err(_) => (false, preset.to_json_detailed()?),
            }
        } else {
            (false, preset.to_json_detailed()?)
        };
        let path = self.directory.join(format!(
            "{}.{}",
            preset.name,
            if toml { "toml" } else { "json" }
        ));
        atomic_write_new_detailed(&path, &text)?;
        Ok(path)
    }
    pub fn update(&self, preset: &Preset, data_dir: &Path) -> Result<PathBuf, String> {
        self.update_detailed(preset, data_dir)
            .map_err(|error| error.to_string())
    }
    pub fn update_detailed(
        &self,
        preset: &Preset,
        data_dir: &Path,
    ) -> Result<PathBuf, LibraryError> {
        Self::validate_write(preset, data_dir)?;
        let _lock = self.lock_mutations(false)?;
        let path = self.user_path(&preset.name)?;
        let text = if carrier_detailed(&path)? {
            preset.to_toml_detailed()?
        } else {
            preset.to_json_detailed()?
        };
        atomic_write_detailed(&path, &text)?;
        Ok(path)
    }
    pub fn rename(&self, name: &str, new_name: &str, data_dir: &Path) -> Result<PathBuf, String> {
        self.rename_detailed(name, new_name, data_dir)
            .map_err(|error| error.to_string())
    }
    pub fn rename_detailed(
        &self,
        name: &str,
        new_name: &str,
        data_dir: &Path,
    ) -> Result<PathBuf, LibraryError> {
        validate_name_detailed(new_name)?;
        let _lock = self.lock_mutations(false)?;
        let old = self.user_path(name)?;
        let mut preset = load_file_detailed(&old)?;
        preset.name = new_name.into();
        Self::validate_write(&preset, data_dir)?;
        if preset.name.to_lowercase() != name.to_lowercase() {
            self.ensure_new(new_name)?;
        }
        let new = self.directory.join(format!(
            "{new_name}.{}",
            old.extension().unwrap().to_str().unwrap()
        ));
        let text = if carrier_detailed(&old)? {
            preset.to_toml_detailed()?
        } else {
            preset.to_json_detailed()?
        };
        let same_entry = old == new
            || std::fs::canonicalize(&old)
                .ok()
                .zip(std::fs::canonicalize(&new).ok())
                .is_some_and(|(old, new)| old == new);
        if same_entry {
            atomic_write_detailed(&new, &text)?;
        } else {
            atomic_write_new_detailed(&new, &text)?;
        }
        if should_remove_old_path(&old, &new)?
            && let Err(e) = std::fs::remove_file(&old)
        {
            let _ = std::fs::remove_file(&new);
            let diagnostic = format!("renaming {}: {e}", old.display());
            return Err(LibraryError::io("rename", &old, e, diagnostic));
        }
        Ok(new)
    }
    pub fn duplicate(
        &self,
        preset: &Preset,
        new_name: &str,
        data_dir: &Path,
    ) -> Result<PathBuf, String> {
        self.duplicate_detailed(preset, new_name, data_dir)
            .map_err(|error| error.to_string())
    }
    pub fn duplicate_detailed(
        &self,
        preset: &Preset,
        new_name: &str,
        data_dir: &Path,
    ) -> Result<PathBuf, LibraryError> {
        let source_path = self
            .scan_detailed()?
            .into_iter()
            .find(|entry| entry.preset.as_ref() == Some(preset) && entry.path.is_some())
            .and_then(|entry| entry.path);
        let toml = source_path
            .as_deref()
            .map(carrier_detailed)
            .transpose()?
            .unwrap_or(true);
        let mut copy = preset.clone();
        copy.name = new_name.into();
        self.save_new_format_detailed(&copy, data_dir, toml)
    }
    pub fn delete(&self, name: &str) -> Result<(), String> {
        self.delete_detailed(name)
            .map_err(|error| error.to_string())
    }
    pub fn delete_detailed(&self, name: &str) -> Result<(), LibraryError> {
        let _lock = self.lock_mutations(false)?;
        let path = self.user_path(name)?;
        std::fs::remove_file(&path).map_err(|error| {
            let diagnostic = format!("{}: {error}", path.display());
            LibraryError::io("remove", &path, error, diagnostic)
        })
    }
    pub fn import(
        &self,
        path: &Path,
        new_name: Option<&str>,
        data_dir: &Path,
    ) -> Result<PathBuf, String> {
        self.import_detailed(path, new_name, data_dir)
            .map_err(|error| error.to_string())
    }
    pub fn import_detailed(
        &self,
        path: &Path,
        new_name: Option<&str>,
        data_dir: &Path,
    ) -> Result<PathBuf, LibraryError> {
        let mut preset = load_file_detailed(path)?;
        if let Some(name) = new_name {
            preset.name = name.into();
        }
        self.save_new_format_detailed(&preset, data_dir, carrier_detailed(path)?)
    }
    pub fn export(&self, preset: &Preset, path: &Path, data_dir: &Path) -> Result<PathBuf, String> {
        self.export_detailed(preset, path, data_dir)
            .map_err(|error| error.to_string())
    }
    pub fn export_detailed(
        &self,
        preset: &Preset,
        path: &Path,
        data_dir: &Path,
    ) -> Result<PathBuf, LibraryError> {
        Self::validate_write(preset, data_dir)?;
        let path = if path.is_dir() {
            path.join(format!("{}.toml", preset.name))
        } else {
            path.to_owned()
        };
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| {
            let diagnostic = format!("creating {}: {error}", parent.display());
            LibraryError::io("create", parent, error, diagnostic)
        })?;
        let destination_directory = std::fs::canonicalize(parent).map_err(|error| {
            let diagnostic = format!("resolving {}: {error}", parent.display());
            LibraryError::io("resolve", parent, error, diagnostic)
        })?;
        let in_library = std::fs::canonicalize(&self.directory)
            .is_ok_and(|directory| directory == destination_directory);
        let _lock = if in_library {
            let lock = self.lock_mutations(true)?;
            if path.file_stem().and_then(|stem| stem.to_str()) != Some(preset.name.as_str()) {
                return Err(LibraryError::new(
                    LibraryErrorReason::ExportName {
                        path: path.clone(),
                        name: preset.name.clone(),
                    },
                    "library export filename must match the preset name",
                ));
            }
            self.ensure_new(&preset.name)?;
            Some(lock)
        } else {
            None
        };
        let text = if carrier_detailed(&path)? {
            preset.to_toml_detailed()?
        } else {
            preset.to_json_detailed()?
        };
        if in_library {
            atomic_write_new_detailed(&path, &text)?;
        } else {
            atomic_write_detailed(&path, &text)?;
        }
        Ok(path)
    }
}
pub(super) fn atomic_write_detailed(path: &Path, text: &str) -> Result<(), LibraryError> {
    write_document(path, text, true)
}
fn atomic_write_new_detailed(path: &Path, text: &str) -> Result<(), LibraryError> {
    write_document(path, text, false)
}
fn write_document(path: &Path, text: &str, replace: bool) -> Result<(), LibraryError> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let result = (|| -> std::io::Result<()> {
        let mut file = tempfile::Builder::new()
            .prefix(".preset-")
            .suffix(".tmp")
            .tempfile_in(parent)?;
        file.write_all(text.as_bytes())?;
        file.as_file().sync_all()?;
        if replace {
            file.persist(path)
        } else {
            file.persist_noclobber(path)
        }
        .map_err(|error| error.error)?;
        Ok(())
    })();
    result.map_err(|error| {
        let diagnostic = format!("writing {}: {error}", path.display());
        LibraryError::io("write", path, error, diagnostic)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutation_errors_retain_document_path_and_name_conflict() {
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.join("library"));
        let path = temp.0.join("Unsupported.json");
        let mut preset = grain("Unsupported", Value::from(0.5));
        preset.schema_version = PRESET_SCHEMA_VERSION + 1;
        std::fs::write(&path, serde_json::to_string(&preset).unwrap()).unwrap();
        for name in [None, Some("Renamed")] {
            let error = library.import_detailed(&path, name, &data()).unwrap_err();
            assert!(
                matches!(&error.reason, LibraryErrorReason::Document { path: Some(actual), error } if actual == &path && matches!(error.reason, PresetDocumentReason::Schema { .. }))
            );
            assert_eq!(
                error.to_string(),
                library.import(&path, name, &data()).unwrap_err()
            );
        }
        preset.schema_version = PRESET_SCHEMA_VERSION;
        library.save_new(&preset, &data()).unwrap();
        let error = library.save_new_detailed(&preset, &data()).unwrap_err();
        assert!(
            matches!(&error.reason, LibraryErrorReason::NameConflict { name } if name == &preset.name)
        );
        assert_eq!(
            error.to_string(),
            library.save_new(&preset, &data()).unwrap_err()
        );
        assert!(!library.directory().join("Renamed.json").exists());
    }

    #[test]
    fn document_fault_retains_owned_validation_reasons_and_cli_text() {
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.clone());
        let mut preset = grain("Validation", Value::from(0.5));
        preset.schema_version = PRESET_SCHEMA_VERSION + 1;
        let path = temp.0.join("Validation.json");
        std::fs::write(&path, serde_json::to_string(&preset).unwrap()).unwrap();
        let entries = library.scan().unwrap();
        let fault = entries
            .iter()
            .find(|entry| entry.path.as_ref() == Some(&path))
            .unwrap()
            .fault
            .as_ref()
            .unwrap();
        assert!(
            matches!(fault, LibraryFault::Document { detail } if matches!(detail.reason, PresetDocumentReason::Schema { version } if version == PRESET_SCHEMA_VERSION + 1))
        );
        assert_eq!(
            fault.to_string(),
            format!(
                "unsupported preset schema version {}",
                PRESET_SCHEMA_VERSION + 1
            )
        );
        assert_eq!(
            serde_json::to_value(fault).unwrap(),
            serde_json::json!(fault.to_string())
        );
        preset.schema_version = PRESET_SCHEMA_VERSION;
        preset.provenance.model_version.clear();
        let text = serde_json::to_string(&preset).unwrap();
        let error = Preset::from_json_str_detailed(&text).unwrap_err();
        assert!(matches!(
            error.reason,
            PresetDocumentReason::ProvenanceVersions
        ));
        assert_eq!(error.to_string(), Preset::from_json_str(&text).unwrap_err());
    }

    #[test]
    fn external_export_does_not_require_a_usable_library() {
        let temp = Temp::new();
        let blocked = temp.0.join("config-file");
        std::fs::write(&blocked, "not a directory").unwrap();
        let library = PresetLibrary::new(blocked.join("presets"));
        let preset = grain("Exported", Value::from(0.5));
        let destination = temp.0.join("Exported.json");
        library.export(&preset, &destination, &data()).unwrap();
        assert_eq!(load_file(&destination).unwrap(), preset);
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn export_to_missing_case_alias_obeys_library_name_rules() {
        let temp = Temp::new();
        std::fs::write(temp.0.join("CaseProbe"), "probe").unwrap();
        if !temp.0.join("caseprobe").exists() {
            return;
        }
        let library = PresetLibrary::new(temp.0.join("Presets"));
        let preset = grain("Original", Value::from(0.5));
        let destination = temp.0.join("presets/Different.json");
        assert!(library.export(&preset, &destination, &data()).is_err());
        assert!(!destination.exists());
        library
            .export(&preset, &temp.0.join("presets/Original.json"), &data())
            .unwrap();
        assert_eq!(
            library.load(PresetGroup::User, "Original").unwrap().preset,
            preset
        );
    }

    #[cfg(unix)]
    #[test]
    fn export_through_missing_library_alias_uses_name_ownership() {
        let temp = Temp::new();
        let real = temp.0.join("real");
        let alias = temp.0.join("alias");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let library = PresetLibrary::new(real.join("library"));
        let original = grain("Original", Value::from(0.5));
        let destination = alias.join("library/Original.json");
        library.export(&original, &destination, &data()).unwrap();
        assert_eq!(
            library.load(PresetGroup::User, "Original").unwrap().preset,
            original
        );
        assert!(
            library
                .export(
                    &grain("original", Value::from(0.7)),
                    &alias.join("library/original.toml"),
                    &data()
                )
                .is_err()
        );
        assert_eq!(load_file(&destination).unwrap(), original);
    }
    #[test]
    fn creator_ignoring_lock_cannot_be_overwritten_after_identity_check() {
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.clone());
        let _lock = library.lock_mutations(false).unwrap();
        let candidate = grain("Late", Value::from(0.3));
        let other = grain("Late", Value::from(0.7));
        let destination = temp.0.join("Late.json");
        library.ensure_new("Late").unwrap();
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                // Deliberately bypass the library lock after the identity check.
                std::fs::write(&destination, other.to_json().unwrap()).unwrap();
                barrier.wait();
            });
            barrier.wait();
            assert!(
                atomic_write_new_detailed(&destination, &candidate.to_json().unwrap()).is_err()
            );
        });
        assert_eq!(load_file(&destination).unwrap(), other);
        assert_eq!(
            library.load(PresetGroup::User, "Late").unwrap().preset,
            other
        );
        assert!(
            library
                .scan()
                .unwrap()
                .iter()
                .all(|entry| entry.fault.is_none())
        );
    }

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "spektrafilm-library-writes-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    use crate::test_support::data_dir as data;
    /// A valid look carrying both profiles, so library mechanics are exercised
    /// against a document the loader accepts. The `amount` varies the optional
    /// grain value so two documents differ, which is what the rename and
    /// duplicate cases need; `Value::Null` leaves it unset, which is what the
    /// JSON-carrier case needs.
    fn grain(name: &str, amount: Value) -> Preset {
        let mut params = RuntimeParams::default();
        params.film_render.grain.procedural_amount = amount.as_f64().map(|v| v as f32);
        let mut preset = Preset::capture(
            name,
            "kodak_portra_400",
            "kodak_supra_endura",
            &params,
            Provenance {
                implementation_version: "test".into(),
                model_version: "test".into(),
                scanner: None,
            },
        );
        if amount.is_null() {
            preset
                .look_parameters_mut()
                .expect("captured look preset")
                .film_render
                .grain
                .procedural_amount = None;
            let object = serde_json::to_value(&preset).unwrap();
            let mut map = object.as_object().unwrap().clone();
            map["parameters"]["film_render"]["grain"]
                .as_object_mut()
                .unwrap()
                .insert("procedural_amount".into(), Value::Null);
            return Preset::from_value(Value::Object(map)).unwrap();
        }
        preset
    }

    #[test]
    fn document_fault_stores_cause_and_selector_prefixes_path_once() {
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.clone());
        let path = temp.0.join("Native Grain Check.json");
        std::fs::write(
            &path,
            r#"{"schema_version":2,"name":"Native Grain Check","parameters":{"film_render":{"grain":{"procedural_amount":null}}}}"#,
        )
        .unwrap();
        let entries = library.scan().unwrap();
        let entry = entries
            .iter()
            .find(|entry| entry.path.as_ref() == Some(&path))
            .unwrap();
        let cause = "invalid preset: missing field `provenance`";
        assert_eq!(
            entry.fault.as_ref().map(ToString::to_string).as_deref(),
            Some(cause)
        );
        assert!(
            matches!(&entry.fault, Some(LibraryFault::Document { detail }) if matches!(detail.reason, PresetDocumentReason::Parser { .. }) && detail.to_string() == cause)
        );
        assert_eq!(
            serde_json::to_value(&entry.fault).unwrap(),
            serde_json::json!(cause)
        );
        let diagnostic = format!("{}: {cause}", path.display());
        assert_eq!(load_file(&path).unwrap_err(), diagnostic);
        assert_eq!(
            library
                .resolve_selector("user:Native Grain Check")
                .unwrap_err(),
            diagnostic
        );
    }

    #[test]
    fn scope_fault_stores_directory_in_path_and_cause_in_message() {
        let temp = Temp::new();
        let directory = temp.0.join("presets");
        std::fs::write(&directory, "not a directory").unwrap();
        let error = std::fs::read_dir(&directory).unwrap_err();
        let expected = format!("reading My Presets directory: {error}");
        let entries = PresetLibrary::new(directory.clone()).scan().unwrap();
        let fault = entries.iter().find(|entry| entry.fault.is_some()).unwrap();
        assert_eq!(fault.path.as_ref(), Some(&directory));
        assert_eq!(
            fault.fault.as_ref().map(ToString::to_string).as_deref(),
            Some(expected.as_str())
        );
        assert!(
            matches!(&fault.fault, Some(LibraryFault::DirectoryRead { detail }) if detail == &error.to_string())
        );
        assert_eq!(
            serde_json::to_value(&fault.fault).unwrap(),
            serde_json::json!(expected)
        );
    }

    #[cfg(unix)]
    #[test]
    fn rename_removal_skips_alias_of_same_entry() {
        let temp = Temp::new();
        let old = temp.0.join("Grain.json");
        let alias = temp.0.join("grain-alias.json");
        std::fs::write(&old, "document").unwrap();
        std::os::unix::fs::symlink(&old, &alias).unwrap();
        assert!(!should_remove_old_path(&old, &alias).unwrap());
        assert!(!should_remove_old_path(&temp.0.join("missing.json"), &alias).unwrap());
    }

    #[test]
    fn normal_rename_removes_old_document() {
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.clone());
        let old = library
            .save_new(&grain("Original", Value::from(0.5)), &data())
            .unwrap();
        let new = library.rename("Original", "Renamed", &data()).unwrap();
        assert!(!old.exists());
        assert_eq!(load_file(&new).unwrap().name, "Renamed");
    }

    // Separate test processes exercise OS locking, rather than a shared Rust mutex.
    #[test]
    fn mutation_process() {
        let Ok(directory) = std::env::var("SPEKTRAFILM_MUTATION_DIRECTORY") else {
            return;
        };
        use std::io::{BufRead, Write};
        println!("MUTATION_READY");
        std::io::stdout().flush().unwrap();
        let mut start = String::new();
        std::io::stdin().lock().read_line(&mut start).unwrap();
        let library = PresetLibrary::new(PathBuf::from(directory));
        let operation = std::env::var("SPEKTRAFILM_MUTATION_OPERATION").unwrap();
        let result = match operation.as_str() {
            "create-toml" => {
                library.save_new_format(&grain("Contended", Value::from(0.3)), &data(), true)
            }
            "create-json" => {
                library.save_new_format(&grain("contended", Value::from(0.7)), &data(), false)
            }
            "rename" => library.rename("Original", "Contended", &data()),
            _ => panic!("unknown mutation operation"),
        };
        std::fs::write(
            std::env::var("SPEKTRAFILM_MUTATION_RESULT").unwrap(),
            if result.is_ok() {
                "success"
            } else {
                "conflict"
            },
        )
        .unwrap();
        if let Err(error) = result {
            assert!(error.contains("already exists"), "{error}");
        }
    }

    fn competing_mutations(first: &str, second: &str, rename: bool) {
        use std::io::{BufRead, Write};
        use std::process::{Command, Stdio};
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.join("library"));
        let original = grain("Original", Value::from(0.5));
        if rename {
            library.save_new(&original, &data()).unwrap();
        }
        let lock = library.lock_mutations(true).unwrap();
        let mut children = Vec::new();
        for (index, operation) in [first, second].into_iter().enumerate() {
            let result = temp.0.join(format!("result-{index}"));
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "presets::library::tests::mutation_process",
                    "--nocapture",
                ])
                .env("SPEKTRAFILM_MUTATION_DIRECTORY", library.directory())
                .env("SPEKTRAFILM_MUTATION_OPERATION", operation)
                .env("SPEKTRAFILM_MUTATION_RESULT", &result)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
            let mut line = String::new();
            loop {
                assert_ne!(
                    output.read_line(&mut line).unwrap(),
                    0,
                    "child exited before barrier"
                );
                if line.contains("MUTATION_READY") {
                    break;
                }
                line.clear();
            }
            children.push((child, output, result));
        }
        // Both independent processes are ready before either mutation is released.
        for (child, _, _) in &mut children {
            child.stdin.take().unwrap().write_all(b"start\n").unwrap();
        }
        drop(lock);
        let mut successes = Vec::new();
        for (mut child, mut output, result) in children {
            let mut remaining = String::new();
            std::io::Read::read_to_string(&mut output, &mut remaining).unwrap();
            assert!(child.wait().unwrap().success(), "{remaining}");
            successes.push(std::fs::read_to_string(result).unwrap() == "success");
        }
        assert_eq!(successes.iter().filter(|success| **success).count(), 1);
        let survivor = library.load(PresetGroup::User, "CONTENDED").unwrap().preset;
        let expected = if successes[0] {
            grain("Contended", Value::from(if rename { 0.5 } else { 0.3 }))
        } else {
            grain("contended", Value::from(0.7))
        };
        assert_eq!(survivor, expected);
        let entries = library.scan().unwrap();
        assert!(entries.iter().all(|entry| entry.fault.is_none()));
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.group == PresetGroup::User)
                .count(),
            if rename && !successes[0] { 2 } else { 1 }
        );
        if rename && !successes[0] {
            assert_eq!(
                library.load(PresetGroup::User, "Original").unwrap().preset,
                original
            );
        }
    }

    #[test]
    fn competing_create_has_one_owner_across_case_and_carriers() {
        competing_mutations("create-toml", "create-json", false);
    }

    #[test]
    fn competing_rename_and_create_preserve_the_winner_and_failed_source() {
        competing_mutations("rename", "create-json", true);
    }

    #[test]
    fn failed_update_and_rename_retain_existing_documents() {
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.clone());
        let original = grain("Original", Value::from(0.5));
        let path = library.save_new(&original, &data()).unwrap();
        let old_bytes = std::fs::read(&path).unwrap();
        // A document that names a stock the data directory does not carry
        // cannot replace the existing one.
        let mut broken = grain("Original", Value::from(0.5));
        broken.film_profile.as_mut().unwrap().stock = "not_a_stock".into();
        assert!(library.update(&broken, &data()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), old_bytes);
        std::fs::create_dir(temp.0.join("Blocked.toml")).unwrap();
        assert!(library.rename("Original", "Blocked", &data()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), old_bytes);
        let renamed = library.rename("Original", "original", &data()).unwrap();
        assert_eq!(
            load_file(&renamed).unwrap(),
            grain("original", Value::from(0.5))
        );
        assert_eq!(
            library
                .scan()
                .unwrap()
                .iter()
                .filter(|entry| entry.name.eq_ignore_ascii_case("original"))
                .count(),
            1
        );
    }

    #[test]
    fn export_to_library_obeys_name_identity_and_keeps_old_document() {
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.join("library"));
        let original = grain("Original", Value::from(0.5));
        let path = library.save_new(&original, &data()).unwrap();
        assert!(
            library
                .export(
                    &grain("original", Value::from(0.7)),
                    &library.directory().join("original.json"),
                    &data()
                )
                .is_err()
        );
        assert_eq!(load_file(&path).unwrap(), original);
        let exported = grain("Exported", Value::from(0.3));
        library
            .export(
                &exported,
                &library.directory().join("Exported.json"),
                &data(),
            )
            .unwrap();
        let mut unpublished = grain("Unpublished", Value::from(0.3));
        unpublished.film_profile.as_mut().unwrap().stock = "not_a_stock".into();
        let failed = library.directory().join("Unpublished.toml");
        assert!(library.export(&unpublished, &failed, &data()).is_err());
        assert!(!failed.exists());
        assert!(
            library
                .scan()
                .unwrap()
                .iter()
                .all(|entry| entry.fault.is_none())
        );
        assert_eq!(
            library.load(PresetGroup::User, "Exported").unwrap().preset,
            exported
        );
    }

    #[test]
    fn discovery_does_not_create_missing_library_directory() {
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.join("missing"));
        assert!(
            library
                .scan()
                .unwrap()
                .iter()
                .all(|entry| entry.group == PresetGroup::BuiltIn)
        );
        assert!(!library.directory().exists());
    }

    #[test]
    fn json_null_survives_duplicate_and_rename() {
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.join("library"));
        let source = temp.0.join("Clear amount.json");
        let preset = grain("Clear amount", Value::Null);
        std::fs::write(&source, preset.to_json_detailed().unwrap()).unwrap();
        let imported = library.import(&source, None, &data()).unwrap();
        assert_eq!(imported.extension().unwrap(), "json");
        let copy = library
            .duplicate(&load_file(&imported).unwrap(), "Copy", &data())
            .unwrap();
        assert_eq!(copy.extension().unwrap(), "json");
        let renamed = library.rename("Copy", "Renamed", &data()).unwrap();
        assert_eq!(renamed.extension().unwrap(), "json");
        let mut expected = preset;
        expected.name = "Renamed".into();
        assert_eq!(load_file(&renamed).unwrap(), expected);
        assert!(!copy.exists());
    }

    #[test]
    fn unusable_user_library_keeps_builtins_and_one_scope_fault() {
        let temp = Temp::new();
        let directory = temp.0.join("presets");
        std::fs::write(&directory, "not a directory").unwrap();
        let library = PresetLibrary::new(directory.clone());
        let entries = library.scan().unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.group == PresetGroup::BuiltIn && entry.fault.is_none())
                .map(|entry| entry.name.as_str())
                .collect::<std::collections::BTreeSet<_>>(),
            BUILTIN_ASSETS
                .iter()
                .map(|(name, _)| *name)
                .collect::<std::collections::BTreeSet<_>>()
        );
        let faults: Vec<_> = entries
            .iter()
            .filter(|entry| entry.fault.is_some())
            .collect();
        assert_eq!(faults.len(), 1);
        assert_eq!(faults[0].group, PresetGroup::User);
        assert_eq!(faults[0].path.as_ref(), Some(&directory));
        assert_eq!(
            library
                .resolve_selector("built-in:Kodak Portra 400")
                .unwrap()
                .preset
                .name,
            "Kodak Portra 400"
        );
    }
}
