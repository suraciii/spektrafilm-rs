//! State file loading, saving, and startup defaults.
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use super::GuiState;

impl GuiState {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            crate::app::Message::new("operations.state.read")
                .arg("path", path.display())
                .context(error)
        })?;
        Self::from_value(
            serde_json::from_str(&text).map_err(|error| {
                crate::app::Message::new("operations.state.json").context(error)
            })?,
        )
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
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
            bail!(crate::app::Message::new("operations.state.dialog_object"));
        }
        if !state.sections["rust"].is_object() {
            state.sections["rust"] = json!({"version": super::CURRENT_STATE_VERSION});
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
