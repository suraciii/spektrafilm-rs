//! GUI state file IO: startup defaults, load and save.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use super::GuiState;
impl GuiState {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Read GUI state {}", path.display()))?;
        Self::from_value(serde_json::from_str(&text).context("Invalid GUI state JSON")?)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.runtime_params()?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(&self.sections)?)?;
        Ok(())
    }
}

/// Startup default state inside an already resolved session directory.
/// Production resolves that directory to the platform configuration
/// directory; acceptance supplies an isolated one.
pub fn default_path(directory: &Path) -> PathBuf {
    directory.join("gui_default_state.json")
}
pub fn startup(directory: &Path) -> Result<GuiState> {
    let path = directory.join("gui_default_state.json");
    let mut state = if path.exists() {
        GuiState::load(&path)?
    } else {
        GuiState::factory()
    };
    let directories = directory.join("dialog_dirs.json");
    if directories.exists() {
        let value: Value = serde_json::from_str(&std::fs::read_to_string(directories)?)?;
        if !value.is_object() {
            bail!("File dialog directories must be a JSON object");
        }
        if !state.sections["rust"].is_object() {
            state.sections["rust"] = json!({"version":1});
        }
        state.sections["rust"]["dialog_dirs"] = value;
    }
    Ok(state)
}
pub fn reset_factory(directory: &Path) -> Result<GuiState> {
    let path = default_path(directory);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(GuiState::factory())
}
