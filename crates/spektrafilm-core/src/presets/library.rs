use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresetGroup {
    #[serde(rename = "Built-in")]
    BuiltIn,
    #[serde(rename = "My Presets")]
    User,
}
#[derive(Debug, Clone, Serialize)]
pub struct LibraryEntry {
    pub group: PresetGroup,
    pub name: String,
    pub kind: Option<PresetKind>,
    pub film_profile: Option<ProfileRef>,
    pub print_profile: Option<ProfileRef>,
    pub path: Option<PathBuf>,
    pub fault: Option<String>,
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
        kind: preset.kind().ok(),
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
    let is_toml = carrier(path)?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if is_toml {
        Preset::from_toml_str(&text)
    } else {
        Preset::from_json_str(&text)
    }
    .map_err(|e| format!("{}: {e}", path.display()))
}
fn user_scope_fault(directory: &Path, error: std::io::Error) -> LibraryEntry {
    LibraryEntry {
        group: PresetGroup::User,
        name: "My Presets".into(),
        kind: None,
        film_profile: None,
        print_profile: None,
        path: Some(directory.to_owned()),
        fault: Some(format!("{}: {error}", directory.display())),
        preset: None,
    }
}
fn name_conflict(name: &str) -> String {
    format!("My Presets name {name:?} already exists; choose a name for a renamed copy")
}
fn claim_destination(path: &Path, name: &str) -> Result<(), String> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(|_| ())
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                name_conflict(name)
            } else {
                format!("claiming {}: {error}", path.display())
            }
        })
}
fn should_remove_old_path(old: &Path, new: &Path) -> Result<bool, String> {
    let Ok(old_entry) = std::fs::canonicalize(old) else {
        return Ok(false);
    };
    let new_entry = std::fs::canonicalize(new)
        .map_err(|error| format!("resolving {}: {error}", new.display()))?;
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
            match load_file(&path).and_then(|p| {
                if p.name == stem {
                    Ok(p)
                } else {
                    Err(format!(
                        "document name {:?} differs from file stem {stem:?}",
                        p.name
                    ))
                }
            }) {
                Ok(p) => entries.push(document_entry(PresetGroup::User, Some(path), p)),
                Err(e) => entries.push(LibraryEntry {
                    group: PresetGroup::User,
                    name: stem,
                    kind: None,
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
                e.fault = Some(format!("duplicate My Presets name {:?}", e.name));
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
        if selector.ends_with(".toml") || selector.ends_with(".json") {
            let path = PathBuf::from(selector);
            return Ok(PresetSelection {
                group: None,
                preset: load_file(&path)?,
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
        validate_name(name)?;
        let entries = if group == Some(PresetGroup::BuiltIn) {
            builtin_presets()
                .into_iter()
                .map(|preset| document_entry(PresetGroup::BuiltIn, None, preset))
                .collect()
        } else {
            self.scan()?
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
                    return Err(format!(
                        "{}: {fault}",
                        entry.path.as_ref().unwrap().display()
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
        Err(format!(
            "unknown preset name {selector:?}; {}",
            crate::suggest::error_suffix(
                &suggestions,
                "use spektrafilm preset list to list presets"
            )
        ))
    }
    pub fn load(&self, group: PresetGroup, name: &str) -> Result<PresetSelection, String> {
        self.resolve_selector(&format!(
            "{}:{name}",
            if group == PresetGroup::User {
                "user"
            } else {
                "built-in"
            }
        ))
    }
    fn validate_write(preset: &Preset, data_dir: &Path) -> Result<(), String> {
        preset.validate()?;
        match preset.kind()? {
            PresetKind::Look => {
                preset.resolve(data_dir, &RuntimeParams::default())?;
            }
            PresetKind::Section(_) => {
                preset.apply_section(&RuntimeParams::default())?;
            }
        }
        Ok(())
    }
    fn user_path(&self, name: &str) -> Result<PathBuf, String> {
        self.load(PresetGroup::User, name)?
            .path
            .ok_or_else(|| "user document has no path".into())
    }
    fn ensure_new(&self, name: &str) -> Result<(), String> {
        validate_name(name)?;
        if self
            .scan()?
            .iter()
            .any(|e| e.group == PresetGroup::User && e.name.to_lowercase() == name.to_lowercase())
        {
            return Err(name_conflict(name));
        }
        Ok(())
    }
    pub fn save_new(&self, preset: &Preset, data_dir: &Path) -> Result<PathBuf, String> {
        self.save_new_format(preset, data_dir, true)
    }
    /// Prefer the requested carrier, falling back to JSON for JSON-only sections.
    pub fn save_new_format(
        &self,
        preset: &Preset,
        data_dir: &Path,
        toml: bool,
    ) -> Result<PathBuf, String> {
        Self::validate_write(preset, data_dir)?;
        self.ensure_new(&preset.name)?;
        let (toml, text) = if toml {
            match preset.to_toml() {
                Ok(text) => (true, text),
                Err(_) => (false, preset.to_json()?),
            }
        } else {
            (false, preset.to_json()?)
        };
        std::fs::create_dir_all(&self.directory).map_err(|e| e.to_string())?;
        let path = self.directory.join(format!(
            "{}.{}",
            preset.name,
            if toml { "toml" } else { "json" }
        ));
        claim_destination(&path, &preset.name)?;
        atomic_write(&path, &text)?;
        Ok(path)
    }
    pub fn update(&self, preset: &Preset, data_dir: &Path) -> Result<PathBuf, String> {
        Self::validate_write(preset, data_dir)?;
        let path = self.user_path(&preset.name)?;
        let text = if carrier(&path)? {
            preset.to_toml()?
        } else {
            preset.to_json()?
        };
        atomic_write(&path, &text)?;
        Ok(path)
    }
    pub fn rename(&self, name: &str, new_name: &str, data_dir: &Path) -> Result<PathBuf, String> {
        validate_name(new_name)?;
        let old = self.user_path(name)?;
        let mut preset = load_file(&old)?;
        preset.name = new_name.into();
        Self::validate_write(&preset, data_dir)?;
        if preset.name.to_lowercase() != name.to_lowercase() {
            self.ensure_new(new_name)?;
        }
        let new = self.directory.join(format!(
            "{new_name}.{}",
            old.extension().unwrap().to_str().unwrap()
        ));
        let text = if carrier(&old)? {
            preset.to_toml()?
        } else {
            preset.to_json()?
        };
        atomic_write(&new, &text)?;
        if should_remove_old_path(&old, &new)?
            && let Err(e) = std::fs::remove_file(&old)
        {
            let _ = std::fs::remove_file(&new);
            return Err(format!("renaming {}: {e}", old.display()));
        }
        Ok(new)
    }
    pub fn duplicate(
        &self,
        preset: &Preset,
        new_name: &str,
        data_dir: &Path,
    ) -> Result<PathBuf, String> {
        let source_path = self
            .scan()?
            .into_iter()
            .find(|entry| entry.preset.as_ref() == Some(preset) && entry.path.is_some())
            .and_then(|entry| entry.path);
        let toml = source_path
            .as_deref()
            .map(carrier)
            .transpose()?
            .unwrap_or(true);
        let mut copy = preset.clone();
        copy.name = new_name.into();
        self.save_new_format(&copy, data_dir, toml)
    }
    pub fn delete(&self, name: &str) -> Result<(), String> {
        let path = self.user_path(name)?;
        std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
    }
    pub fn import(
        &self,
        path: &Path,
        new_name: Option<&str>,
        data_dir: &Path,
    ) -> Result<PathBuf, String> {
        let mut preset = load_file(path)?;
        if let Some(name) = new_name {
            preset.name = name.into();
        }
        self.save_new_format(&preset, data_dir, carrier(path)?)
    }
    pub fn export(&self, preset: &Preset, path: &Path, data_dir: &Path) -> Result<PathBuf, String> {
        Self::validate_write(preset, data_dir)?;
        let path = if path.is_dir() {
            path.join(format!("{}.toml", preset.name))
        } else {
            path.to_owned()
        };
        let text = if carrier(&path)? {
            preset.to_toml()?
        } else {
            preset.to_json()?
        };
        atomic_write(&path, &text)?;
        Ok(path)
    }
}
pub(super) fn atomic_write(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().unwrap_or(Path::new("."));
    let temp = parent.join(format!(
        ".preset-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })()
    .map_err(|e| format!("writing {}: {e}", path.display()));
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn data() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
    }
    fn grain(name: &str, amount: Value) -> Preset {
        Preset::from_json_str(
            &serde_json::json!({
                "schema_version": 2,
                "name": name,
                "parameters": {"film_render": {"grain": {"v2_amount": amount}}},
                "provenance": {"implementation_version": "test", "model_version": "test"}
            })
            .to_string(),
        )
        .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn rename_removal_skips_alias_of_same_entry() {
        let temp = Temp::new();
        let old = temp.0.join("Grain.json");
        let alias = temp.0.join("grain.json");
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

    #[test]
    fn destination_claim_rejects_existing_document_without_scan() {
        let temp = Temp::new();
        let destination = temp.0.join("Occupied.toml");
        std::fs::write(&destination, "other creator's document").unwrap();
        assert_eq!(
            claim_destination(&destination, "Occupied").unwrap_err(),
            name_conflict("Occupied")
        );
        assert_eq!(
            std::fs::read_to_string(destination).unwrap(),
            "other creator's document"
        );
    }

    #[test]
    fn claimed_unpublished_destination_is_a_visible_fault() {
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.clone());
        claim_destination(&temp.0.join("Interrupted.toml"), "Interrupted").unwrap();
        let entries = library.scan().unwrap();
        let interrupted = entries
            .iter()
            .find(|entry| entry.name == "Interrupted")
            .unwrap();
        assert!(interrupted.fault.is_some());
        assert!(
            library
                .ensure_new("Interrupted")
                .unwrap_err()
                .contains("already exists")
        );
    }

    #[test]
    fn json_null_survives_duplicate_and_rename() {
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.join("library"));
        let source = temp.0.join("Clear amount.json");
        let preset = grain("Clear amount", Value::Null);
        std::fs::write(&source, preset.to_json().unwrap()).unwrap();
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
    fn new_null_document_uses_json_and_json_source_keeps_carrier() {
        let temp = Temp::new();
        let library = PresetLibrary::new(temp.0.clone());
        let null = grain("Null", Value::Null);
        let saved = library.save_new(&null, &data()).unwrap();
        assert_eq!(saved.extension().unwrap(), "json");
        assert_eq!(load_file(&saved).unwrap(), null);
        let finite = grain("Finite", Value::from(0.5));
        library.save_new_format(&finite, &data(), false).unwrap();
        let copied = library.duplicate(&finite, "Finite copy", &data()).unwrap();
        assert_eq!(copied.extension().unwrap(), "json");
        let default = library
            .save_new(&grain("Default", Value::from(0.5)), &data())
            .unwrap();
        assert_eq!(default.extension().unwrap(), "toml");
    }

    #[test]
    fn unusable_user_library_keeps_builtins_and_one_scope_fault() {
        let temp = Temp::new();
        let directory = temp.0.join("presets");
        std::fs::write(&directory, "not a directory").unwrap();
        let library = PresetLibrary::new(directory.clone());
        let entries = library.scan().unwrap();
        assert_eq!(entries.len(), 6);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.group == PresetGroup::BuiltIn && entry.fault.is_none())
                .count(),
            5
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
