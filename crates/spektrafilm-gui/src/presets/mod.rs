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
            width: 280.0,
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(layout_only(embedded));
            }
            Err(error) => return Err(error.into()),
        };
        let carrier: serde_json::Value = serde_json::from_str(&text)?;
        anyhow::ensure!(
            carrier["rust"]["version"] == 1,
            crate::app::Message::new("surfaces.presets.browser_version")
                .arg("version", &carrier["rust"]["version"])
                .arg("path", path.display())
        );
        // The global carrier restores layout only: an old applied name must
        // never label unrelated controls after a restart.
        Ok(layout_only(serde_json::from_value(
            carrier["rust"]["presets"].clone(),
        )?))
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        // The global carrier owns layout only; a last-used applied identity
        // would label unrelated controls after a restart.
        let carrier = serde_json::json!({"rust": {"version": 1, "presets": {
            "width": self.width,
            "expanded": self.expanded,
        }}});
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
        .filter(|p| {
            matches!(
                p.kind(),
                Ok(PresetKind::Look | PresetKind::Lut | PresetKind::Effects)
            )
        })
    else {
        return Marker::Missing;
    };
    if preset.kind() == Ok(PresetKind::Effects) {
        return if params.workflow.mode == spektrafilm_core::params::WorkflowMode::Off
            && preset.effects_parameters() == Some(&params.effects.clone().unwrap_or_default())
        {
            Marker::Applied
        } else {
            Marker::Modified
        };
    }
    if preset.kind() == Ok(PresetKind::Lut) {
        return if params.workflow.mode == spektrafilm_core::params::WorkflowMode::Lut
            && preset.lut.as_ref() == params.lut_render_params().as_ref()
        {
            if preset.lut.as_ref().is_some_and(|lut| lut.load().is_ok()) {
                Marker::Applied
            } else {
                Marker::Missing
            }
        } else {
            Marker::Modified
        };
    }
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

/// Preset names are 1-64 portable characters; the dialog proposes names that
/// already satisfy the validation instead of opening with an invalid default.
const MAX_PRESET_NAME_CHARS: usize = 64;

/// A free name in `My Presets`, so the name dialog proposes an available copy
/// instead of an invalid one.
pub(super) fn available_name(base: &str, entries: &[LibraryEntry]) -> String {
    let mut base: String = base.trim().chars().take(MAX_PRESET_NAME_CHARS).collect();
    while !base.is_empty() && validate_name(&base).is_err() {
        base.pop();
    }
    if base.is_empty() {
        base = "Preset".to_owned();
    }
    if name_error(&base, entries, None).is_none() {
        return base;
    }
    (2..1000)
        .find_map(|suffix| {
            let suffix = format!(" {suffix}");
            let keep = MAX_PRESET_NAME_CHARS.saturating_sub(suffix.chars().count());
            let mut candidate: String = base.chars().take(keep).collect();
            candidate.push_str(&suffix);
            name_error(&candidate, entries, None)
                .is_none()
                .then_some(candidate)
        })
        .unwrap_or(base)
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
    run_opener(reveal_command(path), path, "surfaces.presets.reveal_error")
}

/// Open a directory itself rather than selecting or revealing its parent.
fn open_directory(path: &Path) -> Result<(), crate::app::Message> {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        std::process::Command::new("explorer")
    } else {
        std::process::Command::new("xdg-open")
    };
    command.arg(path);
    // A missing preset directory is an empty library, not an unusable one;
    // create it so a fresh installation can open the folder it advertises.
    std::fs::create_dir_all(path).map_err(|detail| {
        crate::app::Message::new("surfaces.presets.open_error")
            .arg("path", path.display())
            .arg("detail", detail)
    })?;
    run_opener(command, path, "surfaces.presets.open_error")
}

fn run_opener(
    mut command: std::process::Command,
    path: &Path,
    error_key: &'static str,
) -> Result<(), crate::app::Message> {
    let status = command.status().map_err(|detail| {
        crate::app::Message::new(error_key)
            .arg("path", path.display())
            .arg("detail", detail)
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(crate::app::Message::new(error_key)
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

fn layout_only(mut state: BrowserState) -> BrowserState {
    state.applied = None;
    state
}

pub(crate) enum PresetAction {
    Load,
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
mod errors;
mod render;
mod thumbnail_worker;
mod thumbnails;
pub use render::*;

#[cfg(test)]
mod tests;
