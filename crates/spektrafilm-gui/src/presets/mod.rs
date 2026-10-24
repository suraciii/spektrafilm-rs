//! Preset browser presentation and library actions. Documents and writes belong to core.
use crate::App;
use eframe::egui;
use serde::{Deserialize, Serialize};
use spektrafilm_core::{
    params::RuntimeParams,
    presets::{
        LibraryEntry, Preset, PresetGroup, PresetKind, PresetLibrary, Provenance, validate_name,
    },
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AppliedPreset {
    pub group: PresetGroup,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BrowserState {
    pub width: f32,
    pub expanded: bool,
    pub applied: Option<AppliedPreset>,
}
impl Default for BrowserState {
    fn default() -> Self {
        Self {
            width: 260.0,
            expanded: true,
            applied: None,
        }
    }
}
impl BrowserState {
    pub fn restore(
        extras: &serde_json::Value,
        explicit: bool,
        path: &Path,
    ) -> anyhow::Result<Self> {
        let embedded =
            serde_json::from_value(extras["rust"]["presets"].clone()).unwrap_or_default();
        if explicit {
            return Ok(embedded);
        }
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(embedded),
            Err(error) => return Err(error.into()),
        };
        let carrier: serde_json::Value = serde_json::from_str(&text)?;
        anyhow::ensure!(
            carrier["rust"]["version"] == 1,
            crate::app::Message::new("surfaces.presets.browser_version")
                .arg("version", &carrier["rust"]["version"])
                .arg("path", path.display())
        );
        Ok(serde_json::from_value(carrier["rust"]["presets"].clone())?)
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let carrier = serde_json::json!({"rust": {"version": 1, "presets": self}});
        std::fs::write(path, serde_json::to_vec_pretty(&carrier)?)?;
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub enum Marker {
    Applied,
    Modified,
    Missing,
}
pub fn marker(
    applied: &AppliedPreset,
    entries: &[LibraryEntry],
    params: &RuntimeParams,
    film: &str,
    print: &str,
) -> Marker {
    let Some(preset) = entries
        .iter()
        .find(|e| e.group == applied.group && e.name == applied.name)
        .and_then(|e| e.preset.as_ref())
        .filter(|p| p.kind() == Ok(PresetKind::Look))
    else {
        return Marker::Missing;
    };
    if preset
        .film_profile
        .as_ref()
        .is_some_and(|p| p.stock == film)
        && preset
            .print_profile
            .as_ref()
            .is_some_and(|p| p.stock == print)
        && preset
            .look_parameters()
            .is_some_and(|look| look.matches(params))
    {
        Marker::Applied
    } else {
        Marker::Modified
    }
}
pub fn name_error(
    name: &str,
    entries: &[LibraryEntry],
    rename_from: Option<&str>,
) -> Option<crate::app::Message> {
    validate_name(name)
        .err()
        .map(|_| crate::app::Message::new("surfaces.presets.name_guidance").arg("name", name))
        .or_else(|| {
            entries
                .iter()
                .any(|e| {
                    e.group == PresetGroup::User
                        && e.name.to_lowercase() == name.to_lowercase()
                        && Some(e.name.as_str()) != rename_from
                })
                .then(|| {
                    crate::app::Message::new("surfaces.presets.duplicate_name").arg("name", name)
                })
        })
}
pub fn reveal_command(path: &Path) -> std::process::Command {
    if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg("-R").arg(path);
        command
    } else if cfg!(windows) {
        let mut argument = std::ffi::OsString::from("/select,");
        argument.push(path);
        let mut command = std::process::Command::new("explorer");
        command.arg(argument);
        command
    } else {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        );
        command
    }
}
fn reveal(path: &Path) -> Result<(), crate::app::Message> {
    let status = reveal_command(path).status().map_err(|e| {
        crate::app::Message::new("surfaces.presets.reveal_error")
            .arg("path", path.display())
            .arg("detail", e)
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(crate::app::Message::new("surfaces.presets.reveal_error")
            .arg("path", path.display())
            .arg("detail", status))
    }
}
fn provenance() -> Provenance {
    Provenance {
        implementation_version: env!("CARGO_PKG_VERSION").into(),
        model_version: "spektrafilm".into(),
        scanner: None,
    }
}

pub(crate) enum PresetAction {
    Load,
    Update,
    Rename,
    Duplicate,
    Export,
    Reveal,
    Delete,
}

pub(crate) enum NameAction {
    Save(Preset),
    Rename(String),
    Duplicate(Preset),
    Import(PathBuf),
}
pub(crate) struct NameDialog {
    pub(crate) action: NameAction,
    pub(crate) name: String,
    pub(crate) error: Option<crate::app::Message>,
}

mod actions;
mod discovery;
mod render;
mod thumbnails;
pub use render::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod discovery_tests;
