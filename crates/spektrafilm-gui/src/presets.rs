//! Preset browser presentation and library actions. Documents and writes belong to core.
use crate::App;
use eframe::egui;
use serde::{Deserialize, Serialize};
use spektrafilm_core::{
    params::RuntimeParams,
    presets::{
        LibraryEntry, LookParameters, Preset, PresetGroup, PresetKind, PresetLibrary, Provenance,
        Section, matching_section_preset, validate_name,
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
            "Unsupported browser state extension version"
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
        && preset.look_parameters() == Some(&LookParameters::capture(params))
    {
        Marker::Applied
    } else {
        Marker::Modified
    }
}
pub fn section_label(
    section: Section,
    entries: &[LibraryEntry],
    params: &RuntimeParams,
) -> Result<String, String> {
    Ok(matching_section_preset(
        section,
        entries.iter().filter_map(|e| e.preset.as_ref()),
        params,
    )?
    .map(|p| p.name.clone())
    .unwrap_or_else(|| "custom".into()))
}
pub fn name_error(
    name: &str,
    entries: &[LibraryEntry],
    rename_from: Option<&str>,
) -> Option<String> {
    validate_name(name).err().or_else(|| {
        entries
            .iter()
            .any(|e| {
                e.group == PresetGroup::User
                    && e.name.to_lowercase() == name.to_lowercase()
                    && Some(e.name.as_str()) != rename_from
            })
            .then(|| format!("My Presets already contains '{name}'. Choose another name."))
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
fn reveal(path: &Path) -> Result<(), String> {
    let status = reveal_command(path)
        .status()
        .map_err(|e| format!("Cannot reveal {}: {e}", path.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Cannot reveal {}: {status}", path.display()))
    }
}
fn provenance() -> Provenance {
    Provenance {
        implementation_version: env!("CARGO_PKG_VERSION").into(),
        model_version: "spektrafilm".into(),
        scanner: None,
    }
}

enum PresetAction {
    Load,
    Update,
    Rename,
    Duplicate,
    Export,
    Reveal,
    Delete,
}

enum NameAction {
    Save(Preset),
    Rename(String),
    Duplicate(Preset),
    Import(PathBuf),
}
struct NameDialog {
    action: NameAction,
    name: String,
    error: Option<String>,
}
pub struct Browser {
    pub state: BrowserState,
    library: PresetLibrary,
    entries: Vec<LibraryEntry>,
    scan_error: Option<String>,
    name_dialog: Option<NameDialog>,
    delete: Option<String>,
}
impl Browser {
    pub fn new(extras: &serde_json::Value, library: PresetLibrary) -> Self {
        let mut browser = Self {
            state: serde_json::from_value(extras["rust"]["presets"].clone()).unwrap_or_default(),
            library,
            entries: vec![],
            scan_error: None,
            name_dialog: None,
            delete: None,
        };
        browser.refresh();
        browser
    }
    fn refresh(&mut self) {
        match self.library.scan() {
            Ok(entries) => {
                self.entries = entries;
                self.scan_error = None;
            }
            Err(e) => self.scan_error = Some(e),
        }
    }
}
impl App {
    pub(crate) fn preset_panel(&mut self, ctx: &egui::Context) {
        let expanded = self.presets.state.expanded;
        let panel = egui::SidePanel::left("presets")
            .resizable(expanded)
            .default_width(self.presets.state.width)
            .min_width(if expanded { 180.0 } else { 38.0 });
        let panel = if expanded {
            panel
        } else {
            panel.exact_width(38.0)
        };
        let response = panel.show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .button(if expanded { "◀" } else { "▶" })
                    .on_hover_text("Collapse or expand Presets")
                    .clicked()
                {
                    self.presets.state.expanded = !expanded;
                }
                if expanded {
                    ui.heading("Presets");
                    if ui
                        .button("+")
                        .on_hover_text("Save current look as a preset")
                        .clicked()
                    {
                        self.save_preset_as(None);
                    }
                }
            });
            if !expanded {
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for group in [PresetGroup::BuiltIn, PresetGroup::User] {
                    ui.separator();
                    ui.heading(if group == PresetGroup::BuiltIn {
                        "Built-in"
                    } else {
                        "My Presets"
                    });
                    if group == PresetGroup::User {
                        let directory = self.presets.library.directory().to_path_buf();
                        ui.small(directory.display().to_string());
                        ui.horizontal(|ui| {
                            if ui.button("Show directory").clicked()
                                && let Err(e) = reveal(&directory)
                            {
                                self.status = e;
                            }
                            if ui.button("Refresh").clicked() {
                                self.presets.refresh();
                                self.status = "Refreshed preset library".into();
                            }
                            if ui.button("Import").clicked() {
                                self.import_preset();
                            }
                        });
                    }
                    let mut action = None;
                    let mut entries = self
                        .presets
                        .entries
                        .iter()
                        .enumerate()
                        .filter(|(_, e)| {
                            e.group == group
                                && e.kind == Some(PresetKind::Look)
                                && e.fault.is_none()
                        })
                        .peekable();
                    if entries.peek().is_none() && group == PresetGroup::User {
                        ui.weak("Use + to save the current look.");
                    }
                    for (index, entry) in entries {
                        let active = self
                            .presets
                            .state
                            .applied
                            .as_ref()
                            .is_some_and(|a| a.group == entry.group && a.name == entry.name);
                        let suffix = if active {
                            match marker(
                                self.presets.state.applied.as_ref().unwrap(),
                                &self.presets.entries,
                                &self.params,
                                &self.film_name,
                                &self.print_name,
                            ) {
                                Marker::Modified => " • modified",
                                Marker::Missing => " • missing",
                                Marker::Applied => " • applied",
                            }
                        } else {
                            ""
                        };
                        let response =
                            ui.selectable_label(active, format!("{}{suffix}", entry.name));
                        if response.clicked() {
                            action = Some((index, PresetAction::Load));
                        }
                        response.context_menu(|ui| {
                            if let Some(selected) = self.preset_actions(ui, entry, true) {
                                action = Some((index, selected));
                            }
                        });
                        ui.small(format!(
                            "{} → {}",
                            entry
                                .film_profile
                                .as_ref()
                                .map(|p| p.stock.as_str())
                                .unwrap_or("?"),
                            entry
                                .print_profile
                                .as_ref()
                                .map(|p| p.stock.as_str())
                                .unwrap_or("?")
                        ));
                    }
                    if let Some((index, action)) = action {
                        self.apply_preset_action(index, action);
                    }
                }
                if let Some(applied) = &self.presets.state.applied
                    && marker(
                        applied,
                        &self.presets.entries,
                        &self.params,
                        &self.film_name,
                        &self.print_name,
                    ) == Marker::Missing
                {
                    ui.label(format!("{} • missing", applied.name));
                }
                if let Some(error) = &self.presets.scan_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                if self.presets.entries.iter().any(|e| e.fault.is_some()) {
                    egui::CollapsingHeader::new("Issues")
                        .default_open(true)
                        .show(ui, |ui| {
                            for entry in self.presets.entries.iter().filter(|e| e.fault.is_some()) {
                                show_fault(ui, entry);
                            }
                        });
                }
            });
        });
        if expanded {
            self.presets.state.width = response.response.rect.width();
        }
        self.preset_dialogs(ctx);
    }
    fn load_preset(&mut self, group: PresetGroup, name: &str) {
        let result = self.presets.library.load(group, name).and_then(|selected| {
            let mut current = self.params.clone();
            if let Some((white, black, glare)) = self.scan_for_print_snapshot {
                current.scanner.white_correction = white;
                current.scanner.black_correction = black;
                current.print_render.glare.active = glare;
            }
            match selected.preset.kind()? {
                PresetKind::Look => {
                    let resolved = selected.preset.resolve(&self.data_dir, &current)?;
                    self.film_medium = crate::scan_medium(&resolved.film);
                    self.film_name = resolved.film_name;
                    self.print_name = resolved.print_name;
                    self.params = resolved.params;
                    self.film_dev_times = resolved.film.data.development_time;
                    self.print_dev_times = resolved.print.data.development_time;
                    self.presets.state.applied = Some(AppliedPreset {
                        group,
                        name: name.to_owned(),
                    });
                }
                PresetKind::Section(Section::Fotufilm) => {
                    self.params = selected
                        .preset
                        .apply_fotufilm_section(&current, &self.data_dir)?;
                }
                PresetKind::Section(_) => {
                    let film = spektrafilm_core::profile::load_profile_by_name(
                        &self.data_dir,
                        &self.film_name,
                    )
                    .map_err(|error| error.to_string())?;
                    self.params = selected.preset.apply_section(&current, &film)?;
                    self.film_medium = crate::scan_medium(&film);
                }
            }
            self.invalidate_look_render();
            Ok(())
        });
        self.status = match result {
            Ok(()) => format!("Applied preset '{name}'"),
            Err(e) => format!("Preset error: {e}"),
        };
    }
    fn save_preset_as(&mut self, section: Option<Section>) {
        let result = match section {
            Some(section) => {
                Preset::capture_section("New preset", section, &self.params, provenance())
            }
            None if self.params.engine == spektrafilm_core::params::SimulationEngine::Fotufilm => {
                Preset::capture_section("New preset", Section::Fotufilm, &self.params, provenance())
            }
            None => Ok(Preset::capture(
                "New preset",
                &self.film_name,
                &self.print_name,
                &self.params,
                provenance(),
            )),
        };
        match result {
            Ok(preset) => {
                self.presets.name_dialog = Some(NameDialog {
                    action: NameAction::Save(preset),
                    name: String::new(),
                    error: None,
                })
            }
            Err(e) => self.status = e,
        }
    }
    pub(crate) fn section_preset_menu(&mut self, ui: &mut egui::Ui, section: Section) {
        let label =
            section_label(section, &self.presets.entries, &self.params).unwrap_or_else(|e| {
                self.status = e;
                "custom".into()
            });
        ui.menu_button(format!("Preset: {label}"), |ui| {
            if ui.button("Save as…").clicked() {
                self.save_preset_as(Some(section));
                ui.close_menu();
            }
            let mut action = None;
            if !self
                .presets
                .entries
                .iter()
                .any(|e| e.kind == Some(PresetKind::Section(section)))
            {
                ui.weak("No presets. Save as, or place a preset file in the library.");
            }
            for group in [PresetGroup::BuiltIn, PresetGroup::User] {
                ui.label(if group == PresetGroup::BuiltIn {
                    "Built-in"
                } else {
                    "My Presets"
                });
                for (index, entry) in self.presets.entries.iter().enumerate().filter(|(_, e)| {
                    e.group == group && e.kind == Some(PresetKind::Section(section))
                }) {
                    if entry.fault.is_some() {
                        show_fault(ui, entry);
                        continue;
                    }
                    ui.horizontal(|ui| {
                        if ui.button(&entry.name).clicked() {
                            action = Some((index, PresetAction::Load));
                            ui.close_menu();
                        }
                        ui.menu_button("…", |ui| {
                            if let Some(selected) = self.preset_actions(ui, entry, false) {
                                action = Some((index, selected));
                            }
                        });
                    });
                }
                if let Some((index, selected)) = action.take() {
                    self.apply_preset_action(index, selected);
                }
            }
            // Unclassified parse faults cannot be assigned a section; make them visible here too.
            for entry in self
                .presets
                .entries
                .iter()
                .filter(|e| e.fault.is_some() && e.kind.is_none())
            {
                show_fault(ui, entry);
            }
        });
    }
    fn preset_actions(
        &self,
        ui: &mut egui::Ui,
        entry: &LibraryEntry,
        look: bool,
    ) -> Option<PresetAction> {
        entry.preset.as_ref()?;
        let mut action = None;
        if entry.group == PresetGroup::User {
            let active = self
                .presets
                .state
                .applied
                .as_ref()
                .is_some_and(|a| a.group == entry.group && a.name == entry.name);
            if look
                && ui
                    .add_enabled(active, egui::Button::new("Update"))
                    .clicked()
            {
                action = Some(PresetAction::Update);
                ui.close_menu();
            }
            if look && ui.button("Revert").clicked() {
                action = Some(PresetAction::Load);
                ui.close_menu();
            }
            if ui.button("Rename…").clicked() {
                action = Some(PresetAction::Rename);
                ui.close_menu();
            }
        }
        if ui
            .button(if entry.group == PresetGroup::BuiltIn {
                "Duplicate to My Presets…"
            } else {
                "Duplicate…"
            })
            .clicked()
        {
            action = Some(PresetAction::Duplicate);
            ui.close_menu();
        }
        if ui.button("Export…").clicked() {
            action = Some(PresetAction::Export);
            ui.close_menu();
        }
        if entry.path.is_some() && ui.button("Show file").clicked() {
            action = Some(PresetAction::Reveal);
            ui.close_menu();
        }
        if entry.group == PresetGroup::User && ui.button("Delete…").clicked() {
            action = Some(PresetAction::Delete);
            ui.close_menu();
        }
        action
    }
    fn apply_preset_action(&mut self, index: usize, action: PresetAction) {
        let entry = &self.presets.entries[index];
        match action {
            PresetAction::Load => {
                let group = entry.group;
                let name = entry.name.clone();
                self.load_preset(group, &name);
            }
            PresetAction::Update => {
                let preset = entry.preset.as_ref().unwrap();
                let mut captured = Preset::capture(
                    &entry.name,
                    &self.film_name,
                    &self.print_name,
                    &self.params,
                    preset.provenance.clone(),
                );
                captured.author = preset.author.clone();
                captured.description = preset.description.clone();
                self.status = match self.presets.library.update(&captured, &self.data_dir) {
                    Ok(_) => format!("Updated preset '{}'", entry.name),
                    Err(e) => e,
                };
                self.presets.refresh();
            }
            PresetAction::Rename => {
                self.presets.name_dialog = Some(NameDialog {
                    action: NameAction::Rename(entry.name.clone()),
                    name: entry.name.clone(),
                    error: None,
                });
            }
            PresetAction::Duplicate => {
                self.presets.name_dialog = Some(NameDialog {
                    action: NameAction::Duplicate(entry.preset.as_ref().unwrap().clone()),
                    name: format!("{} copy", entry.name),
                    error: None,
                });
            }
            PresetAction::Export => {
                if let Some(path) = self
                    .file_dialog("preset")
                    .set_file_name(format!("{}.toml", entry.name))
                    .add_filter("Preset", &["toml", "json"])
                    .save_file()
                {
                    self.remember_dialog("preset", &path);
                    let preset = self.presets.entries[index].preset.as_ref().unwrap();
                    self.status = match self.presets.library.export(preset, &path, &self.data_dir) {
                        Ok(path) => format!("Exported preset to {}", path.display()),
                        Err(e) => e,
                    };
                }
            }
            PresetAction::Reveal => {
                if let Err(e) = reveal(entry.path.as_ref().unwrap()) {
                    self.status = e;
                }
            }
            PresetAction::Delete => self.presets.delete = Some(entry.name.clone()),
        }
    }
    fn import_preset(&mut self) {
        if let Some(path) = self
            .file_dialog("preset")
            .add_filter("Preset", &["toml", "json"])
            .pick_file()
        {
            self.remember_dialog("preset", &path);
            match self.presets.library.import(&path, None, &self.data_dir) {
                Ok(path) => {
                    self.status = format!("Imported preset to {}", path.display());
                    self.presets.refresh();
                }
                Err(e) => {
                    self.status = e.clone();
                    self.presets.name_dialog = Some(NameDialog {
                        action: NameAction::Import(path),
                        name: String::new(),
                        error: Some(format!("{e}\nImport a renamed copy:")),
                    });
                }
            }
        }
    }
    fn preset_dialogs(&mut self, ctx: &egui::Context) {
        if let Some(mut dialog) = self.presets.name_dialog.take() {
            let mut cancel = false;
            let mut submit = false;
            egui::Window::new("Preset name")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Name in My Presets");
                    ui.text_edit_singleline(&mut dialog.name);
                    let rename_from = match &dialog.action {
                        NameAction::Rename(old) => Some(old.as_str()),
                        _ => None,
                    };
                    let invalid = name_error(&dialog.name, &self.presets.entries, rename_from);
                    if let Some(e) = &invalid {
                        ui.colored_label(egui::Color32::LIGHT_RED, e);
                    }
                    if let Some(e) = &dialog.error {
                        ui.colored_label(egui::Color32::LIGHT_RED, e);
                    }
                    ui.horizontal(|ui| {
                        submit = ui
                            .add_enabled(invalid.is_none(), egui::Button::new("Save"))
                            .clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
            if submit {
                let result = match &dialog.action {
                    NameAction::Save(preset) => {
                        let mut preset = preset.clone();
                        preset.name = dialog.name.clone();
                        self.presets.library.save_new(&preset, &self.data_dir)
                    }
                    NameAction::Rename(old) => {
                        self.presets
                            .library
                            .rename(old, &dialog.name, &self.data_dir)
                    }
                    NameAction::Duplicate(preset) => {
                        self.presets
                            .library
                            .duplicate(preset, &dialog.name, &self.data_dir)
                    }
                    NameAction::Import(path) => {
                        self.presets
                            .library
                            .import(path, Some(&dialog.name), &self.data_dir)
                    }
                };
                match result {
                    Ok(path) => {
                        if let NameAction::Rename(old) = &dialog.action
                            && let Some(active) = self.presets.state.applied.as_mut()
                            && active.group == PresetGroup::User
                            && active.name == *old
                        {
                            active.name = dialog.name.clone();
                        }
                        if matches!(&dialog.action, NameAction::Save(p) if p.kind() == Ok(PresetKind::Look))
                        {
                            self.presets.state.applied = Some(AppliedPreset {
                                group: PresetGroup::User,
                                name: dialog.name.clone(),
                            });
                        }
                        self.status =
                            format!("Saved preset '{}' to {}", dialog.name, path.display());
                        self.presets.refresh();
                    }
                    Err(e) => {
                        self.status = e.clone();
                        dialog.error = Some(e);
                        self.presets.name_dialog = Some(dialog);
                    }
                }
            } else if !cancel {
                self.presets.name_dialog = Some(dialog);
            }
        }
        if let Some(name) = self.presets.delete.clone() {
            egui::Window::new("Delete preset?")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!("Delete '{name}' from My Presets?"));
                    ui.horizontal(|ui| {
                        if ui.button("Delete").clicked() {
                            self.status = match self.presets.library.delete(&name) {
                                Ok(()) => format!("Deleted preset '{name}'"),
                                Err(e) => e,
                            };
                            self.presets.delete = None;
                            self.presets.refresh();
                        }
                        if ui.button("Cancel").clicked() {
                            self.presets.delete = None;
                        }
                    });
                });
        }
    }
}
fn show_fault(ui: &mut egui::Ui, entry: &LibraryEntry) {
    ui.colored_label(
        egui::Color32::LIGHT_RED,
        format!(
            "{}: {}",
            entry
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| entry.name.clone()),
            entry.fault.as_deref().unwrap_or("Invalid document")
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state;
    fn test_app(label: &str) -> (App, PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("spektrafilm-gui-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let state_path = directory.join("state.json");
        state::GuiState::factory().save(&state_path).unwrap();
        let mut app = App::new_with_browser_path(
            std::sync::Arc::new(spektrafilm_gpu::cpu_backend::CpuBackend),
            None,
            Some(state_path),
            None,
            directory.join("preset_browser_state.json"),
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"),
        );
        app.presets.library = PresetLibrary::new(directory.join("presets"));
        app.presets.refresh();
        (app, directory)
    }

    #[test]
    fn failed_look_and_section_preserve_entire_application_transaction() {
        let (mut app, directory) = test_app("failed-transaction");
        let mut look = builtin_presets().remove(0);
        look.name = "Missing profile".into();
        look.film_profile.as_mut().unwrap().stock = "missing_profile".into();
        let section = Preset::from_json_str(r#"{"schema_version":2,"name":"Incompatible scan","parameters":{"scanner":{"scan_output":"negative_to_positive"}},"provenance":{"implementation_version":"test","model_version":"test"}}"#).unwrap();
        std::fs::create_dir_all(directory.join("presets")).unwrap();
        for preset in [look, section] {
            std::fs::write(
                directory
                    .join("presets")
                    .join(format!("{}.json", preset.name)),
                preset.to_json().unwrap(),
            )
            .unwrap();
        }
        app.presets.refresh();
        app.presets.state.applied = Some(AppliedPreset {
            group: PresetGroup::BuiltIn,
            name: "Remembered".into(),
        });
        app.params.scanner.white_correction = false;
        app.params.scanner.black_correction = false;
        app.params.print_render.glare.active = true;
        app.toggle_scan_for_print();
        app.dirty_since = Some(std::time::Instant::now());
        for name in ["Missing profile", "Incompatible scan"] {
            let entry = app
                .presets
                .entries
                .iter()
                .find(|entry| entry.name == name)
                .unwrap()
                .clone();
            let before = serde_json::to_value(&app.params).unwrap();
            let state = app.current_state().unwrap().sections;
            let browser = app.presets.state.clone();
            let film = app.film_name.clone();
            let print = app.print_name.clone();
            let film_times = app.film_dev_times.clone();
            let print_times = app.print_dev_times.clone();
            let snapshot = app.scan_for_print_snapshot;
            let revision = app.parameter_revision;
            let epoch = app.calibration_epoch;
            let dirty = app.dirty;
            let dirty_since = app.dirty_since;
            let force_preview = app.force_preview;
            app.load_preset(entry.group, &entry.name);
            assert!(app.status.starts_with("Preset error:"), "{}", app.status);
            assert_eq!(serde_json::to_value(&app.params).unwrap(), before);
            assert_eq!(app.current_state().unwrap().sections, state);
            assert_eq!(app.presets.state, browser);
            assert_eq!(app.film_name, film);
            assert_eq!(app.print_name, print);
            assert_eq!(app.film_dev_times, film_times);
            assert_eq!(app.print_dev_times, print_times);
            assert_eq!(app.scan_for_print_snapshot, snapshot);
            assert_eq!(app.parameter_revision, revision);
            assert_eq!(app.calibration_epoch, epoch);
            assert_eq!(app.dirty, dirty);
            assert_eq!(app.dirty_since, dirty_since);
            assert_eq!(app.force_preview, force_preview);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ordinary_restart_restores_browser_only_and_explicit_state_wins() {
        let (mut app, directory) = test_app("browser-restart");
        app.params.enlarger.print_exposure = 7.123;
        let startup = app.current_state().unwrap();
        startup
            .save(&directory.join("gui_default_state.json"))
            .unwrap();
        let entry = app
            .presets
            .entries
            .iter()
            .find(|entry| entry.group == PresetGroup::BuiltIn)
            .unwrap()
            .clone();
        app.load_preset(entry.group, &entry.name);
        assert!(app.status.starts_with("Applied preset"), "{}", app.status);
        app.params.enlarger.print_exposure += 2.0;
        app.presets.state.width = 327.0;
        app.presets.state.expanded = false;
        let remembered = app.presets.state.clone();
        app.persist_browser_state(true);
        let carrier: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&app.browser_state_path).unwrap()).unwrap();
        assert!(carrier["rust"]["presets"].get("modified").is_none());
        assert!(carrier["rust"].get("runtime").is_none());
        let reopen = |initial_state| {
            App::new_with_browser_path(
                std::sync::Arc::new(spektrafilm_gpu::cpu_backend::CpuBackend),
                None,
                initial_state,
                None,
                directory.join("preset_browser_state.json"),
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"),
            )
        };
        let mut restarted = reopen(None);
        assert_eq!(
            serde_json::to_value(&restarted.params).unwrap(),
            serde_json::to_value(startup.runtime_params().unwrap()).unwrap()
        );
        assert_eq!(restarted.film_name, startup.film());
        assert_eq!(restarted.print_name, startup.paper());
        assert_eq!(restarted.presets.state, remembered);
        assert!(!restarted.dirty);
        assert_eq!(restarted.parameter_revision, 0);
        assert!(restarted.scan_for_print_snapshot.is_none());
        assert_eq!(
            marker(
                remembered.applied.as_ref().unwrap(),
                &restarted.presets.entries,
                &restarted.params,
                &restarted.film_name,
                &restarted.print_name
            ),
            Marker::Modified
        );
        let mut explicit = state::GuiState::factory();
        explicit.sections["rust"] = serde_json::json!({"version":1,"presets":{"width":211.0,"expanded":true,"applied":{"group":PresetGroup::User,"name":"Absent"}}});
        let explicit_path = directory.join("explicit.json");
        explicit.save(&explicit_path).unwrap();
        let explicit_app = reopen(Some(explicit_path));
        assert_eq!(
            BrowserState::restore(
                &state::GuiState::factory().sections,
                true,
                &app.browser_state_path
            )
            .unwrap(),
            BrowserState::default()
        );
        assert_eq!(explicit_app.presets.state.width, 211.0);
        assert!(explicit_app.presets.state.expanded);
        assert_eq!(
            marker(
                explicit_app.presets.state.applied.as_ref().unwrap(),
                &explicit_app.presets.entries,
                &explicit_app.params,
                &explicit_app.film_name,
                &explicit_app.print_name
            ),
            Marker::Missing
        );
        restarted.apply_state(explicit.clone()).unwrap();
        restarted.persist_browser_state(true);
        assert_eq!(reopen(None).presets.state, explicit_app.presets.state);
        restarted.apply_state(state::GuiState::factory()).unwrap();
        restarted.persist_browser_state(true);
        assert_eq!(reopen(None).presets.state, BrowserState::default());
        std::fs::remove_file(directory.join("preset_browser_state.json")).unwrap();
        assert_eq!(reopen(None).presets.state, BrowserState::default());
        std::fs::write(directory.join("preset_browser_state.json"), "broken JSON").unwrap();
        assert!(reopen(None).status.starts_with("Browser state load error:"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn browser_file_errors_are_visible_and_explicit_state_ignores_bad_carrier() {
        let (mut app, directory) = test_app("browser-errors");
        std::fs::write(&app.browser_state_path, "broken JSON").unwrap();
        assert!(
            BrowserState::restore(&app.gui_state.sections, false, &app.browser_state_path).is_err()
        );
        assert_eq!(
            BrowserState::restore(&app.gui_state.sections, true, &app.browser_state_path).unwrap(),
            BrowserState::default()
        );
        let blocked = directory.join("blocked");
        std::fs::write(&blocked, "not a directory").unwrap();
        app.browser_state_path = blocked.join("browser.json");
        app.presets.state.width += 1.0;
        app.persist_browser_state(false);
        assert!(app.browser_state_changed_at.is_some());
        app.persist_browser_state(true);
        assert!(
            app.status.starts_with("Browser state save error:"),
            "{}",
            app.status
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn scanner_transitions_preserve_points_and_reset_illegal_output() {
        let (mut app, directory) = test_app("scanner-transitions");
        app.params.scanner.black_point = [0.1, 0.2, 0.3];
        app.params.scanner.white_point = [0.7, 0.8, 0.9];
        app.params.workflow.route = "input > film > scan".into();
        app.params.scanner.scan_output = "negative_to_positive".into();
        app.params.scanner.receiver_profile =
            spektrafilm_core::params::ReceiverProfile::LabScanApprox;
        app.params.scanner.positive_interpretation =
            spektrafilm_core::params::PositiveInterpretation::Coupled;
        app.params.scanner.auto_setup = spektrafilm_core::params::ScannerAutoSetup::PerFrame;
        app.params.scanner.finish = spektrafilm_core::params::ScannerFinish::LabScan;
        app.dirty = false;
        let revision = app.parameter_revision;
        app.params.workflow.route = "input > film > print > scan".into();
        app.finish_route_change();
        assert_eq!(app.params.scanner.scan_output, "direct_scan");
        assert_eq!(
            app.params.scanner.receiver_profile,
            spektrafilm_core::params::ReceiverProfile::LegacyColorimetric
        );
        assert_eq!(
            app.params.scanner.positive_interpretation,
            spektrafilm_core::params::PositiveInterpretation::Density
        );
        assert_eq!(
            app.params.scanner.auto_setup,
            spektrafilm_core::params::ScannerAutoSetup::Off
        );
        assert_eq!(
            app.params.scanner.finish,
            spektrafilm_core::params::ScannerFinish::Neutral
        );
        assert!(app.dirty);
        assert_eq!(app.parameter_revision, revision.wrapping_add(1));
        app.params.workflow.route = "input > convert-film > scan".into();
        app.finish_route_change();
        app.params.scanner.scan_output = "negative_to_positive".into();
        app.params.scanner.white_correction = false;
        app.params.scanner.black_correction = false;
        let revision = app.parameter_revision;
        app.toggle_scan_for_print();
        assert!(app.scan_for_print_snapshot.is_none());
        assert!(!app.params.scanner.white_correction);
        assert!(!app.params.scanner.black_correction);
        assert_eq!(app.parameter_revision, revision);
        app.params.scanner.receiver_profile =
            spektrafilm_core::params::ReceiverProfile::LabScanApprox;
        app.params.scanner.positive_interpretation =
            spektrafilm_core::params::PositiveInterpretation::Coupled;
        app.params.scanner.auto_setup = spektrafilm_core::params::ScannerAutoSetup::PerFrame;
        app.params.scanner.finish = spektrafilm_core::params::ScannerFinish::LabScan;
        app.select_film_profile("kodak_ektachrome_100".into());
        assert_eq!(app.film_medium, crate::controls::ScanMedium::Slide);
        assert_eq!(app.params.scanner.scan_output, "direct_scan");
        assert_eq!(
            app.params.scanner.receiver_profile,
            spektrafilm_core::params::ReceiverProfile::LabScanApprox
        );
        assert_eq!(
            app.params.scanner.positive_interpretation,
            spektrafilm_core::params::PositiveInterpretation::Density
        );
        assert_eq!(
            app.params.scanner.auto_setup,
            spektrafilm_core::params::ScannerAutoSetup::Off
        );
        assert_eq!(
            app.params.scanner.finish,
            spektrafilm_core::params::ScannerFinish::Neutral
        );
        let restored = app.current_state().unwrap().runtime_params().unwrap();
        assert_eq!(restored.scanner.black_point, [0.1, 0.2, 0.3]);
        assert_eq!(restored.scanner.white_point, [0.7, 0.8, 0.9]);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn slide_startup_rejects_positive_output_with_visible_error() {
        let (app, directory) = test_app("slide-startup");
        let mut params = app.params.clone();
        params.workflow.route = "input > film > scan".into();
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.white_correction = false;
        params.scanner.black_correction = false;
        params.scanner.black_point = [0.1, 0.2, 0.3];
        let state = state::GuiState::from_runtime(
            &params,
            "kodak_ektachrome_100",
            &app.print_name,
            &app.gui_state.sections,
        )
        .unwrap();
        let path = directory.join("slide.json");
        state.save(&path).unwrap();
        let restarted = App::new_with_browser_path(
            std::sync::Arc::new(spektrafilm_gpu::cpu_backend::CpuBackend),
            None,
            Some(path),
            None,
            directory.join("preset_browser_state.json"),
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"),
        );
        assert!(
            restarted
                .status
                .contains("negative filming-stage film profile")
        );
        assert_eq!(
            restarted.params,
            state::GuiState::factory().runtime_params().unwrap()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn scanner_points_survive_section_preset_and_restart() {
        let (mut app, directory) = test_app("scanner-points");
        app.params.workflow.route = "input > film > scan".into();
        app.finish_route_change();
        app.params.scanner.scan_output = "negative_to_positive".into();
        app.params.scanner.receiver_profile =
            spektrafilm_core::params::ReceiverProfile::LabScanApprox;
        app.params.scanner.positive_interpretation =
            spektrafilm_core::params::PositiveInterpretation::Coupled;
        app.params.scanner.auto_setup = spektrafilm_core::params::ScannerAutoSetup::PerFrame;
        app.params.scanner.finish = spektrafilm_core::params::ScannerFinish::LabScan;
        app.params.scanner.finish_strength = 0.0;
        app.params.scanner.black_point = [0.1, 0.2, 0.3];
        app.params.scanner.white_point = [0.7, 0.8, 0.9];
        let preset =
            Preset::capture_section("Film points", Section::Scanner, &app.params, provenance())
                .unwrap();
        assert_eq!(
            serde_json::to_value(&preset).unwrap()["parameters"]["scanner"]["scan_output"],
            "negative_to_positive"
        );
        app.presets
            .library
            .save_new(&preset, &app.data_dir)
            .unwrap();
        app.presets.refresh();
        let entry = app
            .presets
            .entries
            .iter()
            .find(|entry| entry.name == "Film points")
            .unwrap()
            .clone();
        app.params.scanner.black_point = [0.0; 3];
        app.params.scanner.white_point = [1.0; 3];
        app.params.scanner.receiver_profile =
            spektrafilm_core::params::ReceiverProfile::LegacyColorimetric;
        app.params.scanner.positive_interpretation =
            spektrafilm_core::params::PositiveInterpretation::Separable;
        app.params.scanner.finish = spektrafilm_core::params::ScannerFinish::Neutral;
        app.params.scanner.finish_strength = 1.0;
        app.params.scanner.scan_output = "direct_scan".into();
        app.load_preset(entry.group, &entry.name);
        assert!(app.status.starts_with("Applied preset"), "{}", app.status);
        assert_eq!(app.params.scanner.scan_output, "negative_to_positive");
        assert_eq!(
            app.params.scanner.receiver_profile,
            spektrafilm_core::params::ReceiverProfile::LabScanApprox
        );
        assert_eq!(
            app.params.scanner.positive_interpretation,
            spektrafilm_core::params::PositiveInterpretation::Coupled
        );
        assert_eq!(
            app.params.scanner.auto_setup,
            spektrafilm_core::params::ScannerAutoSetup::PerFrame
        );
        assert_eq!(
            app.params.scanner.finish,
            spektrafilm_core::params::ScannerFinish::LabScan
        );
        assert_eq!(app.params.scanner.finish_strength, 0.0);
        let path = directory.join("saved.json");
        app.current_state().unwrap().save(&path).unwrap();
        let mut restarted = App::new_with_browser_path(
            std::sync::Arc::new(spektrafilm_gpu::cpu_backend::CpuBackend),
            None,
            Some(path),
            None,
            directory.join("preset_browser_state.json"),
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"),
        );
        assert_eq!(restarted.params.scanner.scan_output, "negative_to_positive");
        assert_eq!(restarted.params.scanner.black_point, [0.1, 0.2, 0.3]);
        assert_eq!(restarted.params.scanner.white_point, [0.7, 0.8, 0.9]);
        assert_eq!(
            restarted.params.scanner.receiver_profile,
            spektrafilm_core::params::ReceiverProfile::LabScanApprox
        );
        assert_eq!(
            restarted.params.scanner.positive_interpretation,
            spektrafilm_core::params::PositiveInterpretation::Coupled
        );
        assert_eq!(
            restarted.params.scanner.auto_setup,
            spektrafilm_core::params::ScannerAutoSetup::PerFrame
        );
        assert_eq!(
            restarted.params.scanner.finish,
            spektrafilm_core::params::ScannerFinish::LabScan
        );
        assert_eq!(restarted.params.scanner.finish_strength, 0.0);
        restarted.params.scanner.black_point = [0.0; 3];
        restarted.apply_state(app.current_state().unwrap()).unwrap();
        assert_eq!(restarted.params.scanner.black_point, [0.1, 0.2, 0.3]);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn section_application_restores_transient_scan_controls() {
        let (mut app, directory) = test_app("scan-section");
        app.params.scanner.white_correction = false;
        app.params.scanner.black_correction = false;
        app.params.print_render.glare.active = true;
        for section in [Section::Grain, Section::Base] {
            let preset =
                Preset::capture_section("Section", section, &app.params, provenance()).unwrap();
            app.presets
                .library
                .save_new(&preset, &app.data_dir)
                .unwrap();
            app.presets.refresh();
            let entry = app
                .presets
                .entries
                .iter()
                .find(|entry| entry.name == "Section")
                .unwrap()
                .clone();
            app.toggle_scan_for_print();
            assert_eq!(app.scan_for_print_snapshot, Some((false, false, true)));
            let revision = app.parameter_revision;
            let epoch = app.calibration_epoch;
            app.load_preset(entry.group, &entry.name);
            assert!(app.status.starts_with("Applied preset"), "{}", app.status);
            assert!(app.scan_for_print_snapshot.is_none());
            assert_eq!(app.parameter_revision, revision.wrapping_add(1));
            assert_eq!(app.calibration_epoch, epoch.wrapping_add(1));
            assert!(app.dirty);
            let restored = app.current_state().unwrap().runtime_params().unwrap();
            assert!(!restored.scanner.white_correction);
            assert!(!restored.scanner.black_correction);
            assert!(restored.print_render.glare.active);
            app.presets.library.delete("Section").unwrap();
        }
        let entry = app
            .presets
            .entries
            .iter()
            .find(|entry| entry.group == PresetGroup::BuiltIn)
            .unwrap()
            .clone();
        app.toggle_scan_for_print();
        app.load_preset(entry.group, &entry.name);
        assert!(app.status.starts_with("Applied preset"), "{}", app.status);
        assert!(app.scan_for_print_snapshot.is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unusable_library_path_keeps_builtins_and_renders_fault() {
        let (mut app, directory) = test_app("library-fault");
        let path = directory.join("presets");
        std::fs::write(&path, "not a directory").unwrap();
        app.presets.refresh();
        assert!(app.presets.scan_error.is_none());
        assert_eq!(
            app.presets
                .entries
                .iter()
                .filter(|entry| entry.group == PresetGroup::BuiltIn)
                .count(),
            builtin_presets().len()
        );
        let fault = app
            .presets
            .entries
            .iter()
            .find(|entry| entry.fault.is_some())
            .unwrap();
        assert_eq!(fault.path.as_ref(), Some(&path));
        let fault_text = format!("{}: {}", path.display(), fault.fault.as_ref().unwrap());
        let builtin_name = app
            .presets
            .entries
            .iter()
            .find(|entry| entry.group == PresetGroup::BuiltIn)
            .unwrap()
            .name
            .clone();
        let context = egui::Context::default();
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1460.0, 2000.0),
                )),
                ..Default::default()
            },
            |context| app.preset_panel(context),
        );
        fn texts(shape: &egui::epaint::Shape, output: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(text) => output.push(text.galley.text().to_owned()),
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        texts(shape, output);
                    }
                }
                _ => {}
            }
        }
        let mut rendered = vec![];
        for shape in output.shapes {
            texts(&shape.shape, &mut rendered);
        }
        assert!(rendered.contains(&fault_text), "{rendered:?}");
        assert!(
            rendered.iter().any(|text| text.contains(&builtin_name)),
            "{rendered:?}"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
    use spektrafilm_core::presets::builtin_presets;
    #[test]
    fn marker_transitions_and_capture_roundtrip() {
        let p = builtin_presets().remove(0);
        let mut params = RuntimeParams::default();
        let look = p.look_parameters().unwrap();
        look.apply_to(&mut params);
        let film = &p.film_profile.as_ref().unwrap().stock;
        let print = &p.print_profile.as_ref().unwrap().stock;
        let entries = vec![LibraryEntry {
            group: PresetGroup::BuiltIn,
            name: p.name.clone(),
            kind: Some(PresetKind::Look),
            film_profile: p.film_profile.clone(),
            print_profile: p.print_profile.clone(),
            path: None,
            fault: None,
            preset: Some(p.clone()),
        }];
        let applied = AppliedPreset {
            group: PresetGroup::BuiltIn,
            name: p.name.clone(),
        };
        assert_eq!(
            marker(&applied, &entries, &params, film, print),
            Marker::Applied
        );
        assert_eq!(
            Preset::capture("Roundtrip", film, print, &params, provenance()).look_parameters(),
            p.look_parameters()
        );
        params.io.crop = true;
        assert_eq!(
            marker(&applied, &entries, &params, film, print),
            Marker::Applied
        );
        params.enlarger.print_exposure += 1.0;
        assert_eq!(
            marker(&applied, &entries, &params, film, print),
            Marker::Modified
        );
        assert_eq!(marker(&applied, &[], &params, film, print), Marker::Missing);
    }
    #[test]
    fn section_labels_and_names() {
        let params = RuntimeParams::default();
        let mut entries = vec![];
        for name in ["Zulu", "alpha"] {
            let preset =
                Preset::capture_section(name, Section::Grain, &params, provenance()).unwrap();
            entries.push(LibraryEntry {
                group: PresetGroup::User,
                name: name.into(),
                kind: Some(PresetKind::Section(Section::Grain)),
                film_profile: None,
                print_profile: None,
                path: None,
                fault: None,
                preset: Some(preset),
            });
        }
        assert_eq!(
            section_label(Section::Grain, &entries, &params).unwrap(),
            "alpha"
        );
        assert_eq!(
            section_label(Section::Base, &entries, &params).unwrap(),
            "custom"
        );
        assert!(name_error("ALPHA", &entries, None).is_some());
        assert!(name_error("bad/name", &entries, None).is_some());
        assert!(name_error("alpha", &entries, Some("alpha")).is_none());
    }
    #[test]
    fn reveal_uses_platform_file_selection_arguments() {
        use std::ffi::OsString;

        let path = Path::new("/tmp/My Presets/a look.toml");
        let command = reveal_command(path);
        let (program, args) = if cfg!(target_os = "macos") {
            (
                "open",
                vec![OsString::from("-R"), path.as_os_str().to_owned()],
            )
        } else if cfg!(windows) {
            let mut argument = OsString::from("/select,");
            argument.push(path);
            ("explorer", vec![argument])
        } else {
            (
                "xdg-open",
                vec![path.parent().unwrap().as_os_str().to_owned()],
            )
        };
        assert_eq!(command.get_program(), program);
        assert_eq!(command.get_args().collect::<Vec<_>>(), args);
    }
    #[test]
    fn browser_extension_preserves_existing_fields() {
        let mut extras = state::GuiState::factory().sections;
        extras["rust"]["presets"] = serde_json::to_value(BrowserState {
            width: 312.0,
            expanded: false,
            applied: Some(AppliedPreset {
                group: PresetGroup::User,
                name: "Remembered".into(),
            }),
        })
        .unwrap();
        extras["rust"]["viewer"] = serde_json::json!({"zoom":2});
        let s = state::GuiState::from_runtime(
            &RuntimeParams::default(),
            "kodak_gold_200",
            "kodak_portra_endura",
            &extras,
        )
        .unwrap();
        let restored = state::GuiState::from_value(s.sections.clone()).unwrap();
        let b = Browser::new(&restored.sections, PresetLibrary::default());
        assert_eq!(b.state.width, 312.0);
        assert!(!b.state.expanded);
        assert_eq!(b.state.applied.unwrap().name, "Remembered");
        assert_eq!(
            restored.sections["rust"]["viewer"],
            extras["rust"]["viewer"]
        );
        assert!(
            restored.sections["rust"]["presets"]
                .get("modified")
                .is_none()
        );
    }
    #[test]
    fn section_explicitness_survives_state_roundtrip() {
        let mut params = RuntimeParams::default();
        params.enlarger.m_filter_neutral = 31.0;
        let preset =
            Preset::capture_section("Filters", Section::Enlarger, &params, provenance()).unwrap();
        let film = spektrafilm_core::profile::load_profile_by_name(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"),
            "kodak_gold_200",
        )
        .unwrap();
        let applied = preset
            .apply_section(&RuntimeParams::default(), &film)
            .unwrap();
        let state = state::GuiState::from_runtime(
            &applied,
            "kodak_gold_200",
            "kodak_portra_endura",
            &state::GuiState::factory().sections,
        )
        .unwrap();
        let restored = state::GuiState::from_value(state.sections)
            .unwrap()
            .runtime_params()
            .unwrap();
        assert_eq!(
            restored.render_preparation_state(),
            applied.render_preparation_state()
        );
        assert_eq!(restored.enlarger.m_filter_neutral, 31.0);
        assert_eq!(
            restored.settings.neutral_print_filters_from_database,
            applied.settings.neutral_print_filters_from_database
        );
    }
}
