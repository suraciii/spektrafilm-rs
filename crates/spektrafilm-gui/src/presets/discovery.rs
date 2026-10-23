//! Application browsing preferences, deliberately absent from GuiState and presets.
use super::*;
use std::io::Write;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Filter {
    #[default]
    All,
    Favorites,
    Recent,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Preferences {
    pub(crate) favorites: Vec<AppliedPreset>,
    /// Most recently applied first; presentation still uses library order.
    pub(crate) recent: Vec<AppliedPreset>,
    pub(crate) filter: Filter,
}
impl Preferences {
    pub(crate) fn contains_favorite(&self, identity: &AppliedPreset) -> bool {
        self.favorites.contains(identity)
    }
    pub(crate) fn toggle_favorite(&mut self, identity: AppliedPreset) {
        if self.contains_favorite(&identity) {
            self.favorites.retain(|entry| entry != &identity);
        } else {
            self.favorites.push(identity);
        }
    }
    pub(crate) fn applied(&mut self, identity: AppliedPreset) {
        self.recent.retain(|entry| entry != &identity);
        self.recent.insert(0, identity);
        self.recent.truncate(10);
    }
    pub(crate) fn renamed(&mut self, old: &str, new: &str) {
        for entries in [&mut self.favorites, &mut self.recent] {
            for entry in entries.iter_mut() {
                if entry.group == PresetGroup::User && entry.name == old {
                    entry.name = new.to_owned();
                }
            }
            // A stale reference to the rename target must not create duplicates.
            let mut index = 0;
            while index < entries.len() {
                if entries[..index].contains(&entries[index]) {
                    entries.remove(index);
                } else {
                    index += 1;
                }
            }
        }
    }
    pub(crate) fn includes(&self, entry: &LibraryEntry) -> bool {
        let contains = |entries: &[AppliedPreset]| {
            entries
                .iter()
                .any(|identity| identity.group == entry.group && identity.name == entry.name)
        };
        match self.filter {
            Filter::All => true,
            Filter::Favorites => contains(&self.favorites),
            Filter::Recent => contains(&self.recent),
        }
    }
    pub(crate) fn load(path: &Path) -> anyhow::Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.into()),
        };
        let mut preferences: Self = serde_json::from_slice(&bytes)?;
        // Canonicalize only membership; never rebind missing names to another group.
        let recent = std::mem::take(&mut preferences.recent);
        for identity in recent.into_iter().rev() {
            preferences.applied(identity);
        }
        let favorites = std::mem::take(&mut preferences.favorites);
        for identity in favorites {
            if !preferences.favorites.contains(&identity) {
                preferences.favorites.push(identity);
            }
        }
        Ok(preferences)
    }
    pub(crate) fn save(&self, path: &Path) -> anyhow::Result<()> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".preset-discovery-")
            .suffix(".tmp")
            .tempfile_in(parent)?;
        temporary.write_all(&serde_json::to_vec_pretty(self)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist(path)?;
        Ok(())
    }
}

impl Browser {
    fn preferences_path(&self) -> PathBuf {
        self.library
            .directory()
            .parent()
            .unwrap_or(Path::new("."))
            .join("preset_discovery_preferences.json")
    }
    pub(crate) fn save_preferences(&mut self) {
        self.search = None;
        self.preference_error = self
            .preferences
            .save(&self.preferences_path())
            .err()
            .map(|error| error.to_string());
    }
    pub(crate) fn record_application(&mut self, group: PresetGroup, name: &str) {
        self.preferences.applied(AppliedPreset {
            group,
            name: name.to_owned(),
        });
        self.save_preferences();
    }
    pub(crate) fn record_rename(&mut self, old: &str, new: &str) {
        self.preferences.renamed(old, new);
        self.save_preferences();
    }
}
