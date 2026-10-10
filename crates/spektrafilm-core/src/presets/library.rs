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
            Err(e) => return Err(format!("{}: {e}", self.directory.display())),
        };
        for item in listing {
            let item = item.map_err(|e| format!("{}: {e}", self.directory.display()))?;
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
        let entries = self.scan()?;
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
        Err(format!(
            "unknown preset name {selector:?}; {}",
            crate::suggest::error_suffix(
                &self.suggest(name),
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
            return Err(format!(
                "My Presets name {name:?} already exists; choose a name for a renamed copy"
            ));
        }
        Ok(())
    }
    pub fn save_new(&self, preset: &Preset, data_dir: &Path) -> Result<PathBuf, String> {
        self.save_new_format(preset, data_dir, true)
    }
    /// Explicit carrier choice for JSON-only sections carrying null edits.
    pub fn save_new_format(
        &self,
        preset: &Preset,
        data_dir: &Path,
        toml: bool,
    ) -> Result<PathBuf, String> {
        Self::validate_write(preset, data_dir)?;
        self.ensure_new(&preset.name)?;
        let text = if toml {
            preset.to_toml()?
        } else {
            preset.to_json()?
        };
        std::fs::create_dir_all(&self.directory).map_err(|e| e.to_string())?;
        let path = self.directory.join(format!(
            "{}.{}",
            preset.name,
            if toml { "toml" } else { "json" }
        ));
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
        if new != old {
            if let Err(e) = std::fs::remove_file(&old) {
                let _ = std::fs::remove_file(&new);
                return Err(format!("renaming {}: {e}", old.display()));
            }
        }
        Ok(new)
    }
    pub fn duplicate(
        &self,
        preset: &Preset,
        new_name: &str,
        data_dir: &Path,
    ) -> Result<PathBuf, String> {
        let mut copy = preset.clone();
        copy.name = new_name.into();
        self.save_new(&copy, data_dir)
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
