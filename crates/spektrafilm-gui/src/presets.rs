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
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let mut command = std::process::Command::new(program);
    command.arg(path);
    command
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
    }
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
    pub fn new(extras: &serde_json::Value) -> Self {
        let mut browser = Self {
            state: serde_json::from_value(extras["rust"]["presets"].clone()).unwrap_or_default(),
            library: PresetLibrary::default(),
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
        let response =
            panel.show(ctx, |ui| {
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
                        let entries: Vec<_> = self
                            .presets
                            .entries
                            .iter()
                            .filter(|e| {
                                e.group == group
                                    && e.kind == Some(PresetKind::Look)
                                    && e.fault.is_none()
                            })
                            .cloned()
                            .collect();
                        if entries.is_empty() && group == PresetGroup::User {
                            ui.weak("Use + to save the current look.");
                        }
                        for entry in entries {
                            let active =
                                self.presets.state.applied.as_ref().is_some_and(|a| {
                                    a.group == entry.group && a.name == entry.name
                                });
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
                                self.load_preset(&entry);
                            }
                            response.context_menu(|ui| self.preset_actions(ui, &entry, true));
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
                    let faults: Vec<_> = self
                        .presets
                        .entries
                        .iter()
                        .filter(|e| e.fault.is_some())
                        .cloned()
                        .collect();
                    if !faults.is_empty() {
                        ui.collapsing("Issues", |ui| {
                            for entry in faults {
                                show_fault(ui, &entry);
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
    fn load_preset(&mut self, entry: &LibraryEntry) {
        let result = self
            .presets
            .library
            .load(entry.group, &entry.name)
            .and_then(|selected| {
                match selected.preset.kind()? {
                    PresetKind::Look => {
                        let resolved = selected.preset.resolve(&self.data_dir, &self.params)?;
                        self.film_name = resolved.film_name;
                        self.print_name = resolved.print_name;
                        self.params = resolved.params;
                        self.film_dev_times = resolved.film.data.development_time;
                        self.print_dev_times = resolved.print.data.development_time;
                        self.presets.state.applied = Some(AppliedPreset {
                            group: entry.group,
                            name: entry.name.clone(),
                        });
                    }
                    PresetKind::Section(_) => {
                        self.params = selected.preset.apply_section(&self.params)?;
                    }
                }
                self.invalidate_look_render();
                Ok(())
            });
        self.status = match result {
            Ok(()) => format!("Applied preset '{}'", entry.name),
            Err(e) => format!("Preset error: {e}"),
        };
    }
    fn save_preset_as(&mut self, section: Option<Section>) {
        let result = match section {
            Some(section) => {
                Preset::capture_section("New preset", section, &self.params, provenance())
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
            let entries: Vec<_> = self
                .presets
                .entries
                .iter()
                .filter(|e| e.kind == Some(PresetKind::Section(section)))
                .cloned()
                .collect();
            if entries.is_empty() {
                ui.weak("No presets. Save as, or place a preset file in the library.");
            }
            for group in [PresetGroup::BuiltIn, PresetGroup::User] {
                ui.label(if group == PresetGroup::BuiltIn {
                    "Built-in"
                } else {
                    "My Presets"
                });
                for entry in entries.iter().filter(|e| e.group == group) {
                    if entry.fault.is_some() {
                        show_fault(ui, entry);
                        continue;
                    }
                    ui.horizontal(|ui| {
                        if ui.button(&entry.name).clicked() {
                            self.load_preset(entry);
                            ui.close_menu();
                        }
                        ui.menu_button("…", |ui| self.preset_actions(ui, entry, false));
                    });
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
    fn preset_actions(&mut self, ui: &mut egui::Ui, entry: &LibraryEntry, look: bool) {
        let Some(preset) = &entry.preset else {
            return;
        };
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
                ui.close_menu();
            }
            if look && ui.button("Revert").clicked() {
                self.load_preset(entry);
                ui.close_menu();
            }
            if ui.button("Rename…").clicked() {
                self.presets.name_dialog = Some(NameDialog {
                    action: NameAction::Rename(entry.name.clone()),
                    name: entry.name.clone(),
                    error: None,
                });
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
            self.presets.name_dialog = Some(NameDialog {
                action: NameAction::Duplicate(preset.clone()),
                name: format!("{} copy", entry.name),
                error: None,
            });
            ui.close_menu();
        }
        if ui.button("Export…").clicked() {
            if let Some(path) = self
                .file_dialog("preset")
                .set_file_name(format!("{}.toml", entry.name))
                .add_filter("Preset", &["toml", "json"])
                .save_file()
            {
                self.remember_dialog("preset", &path);
                self.status = match self.presets.library.export(preset, &path, &self.data_dir) {
                    Ok(path) => format!("Exported preset to {}", path.display()),
                    Err(e) => e,
                };
            }
            ui.close_menu();
        }
        if let Some(path) = &entry.path
            && ui.button("Show file").clicked()
        {
            if let Err(e) = reveal(path) {
                self.status = e;
            }
            ui.close_menu();
        }
        if entry.group == PresetGroup::User && ui.button("Delete…").clicked() {
            self.presets.delete = Some(entry.name.clone());
            ui.close_menu();
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
    use spektrafilm_core::presets::builtin_presets;
    #[test]
    fn marker_transitions_and_capture_roundtrip() {
        let p = builtin_presets().remove(0);
        let mut params = RuntimeParams::default();
        let look = p.look_parameters().unwrap();
        params.enlarger = look.enlarger.clone();
        params.scanner = look.scanner.clone();
        params.film_render = look.film_render.clone();
        params.print_render = look.print_render.clone();
        params.camera.diffusion_filter = look.camera_diffusion_filter.clone();
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
    fn reveal_preserves_path_argument() {
        let path = Path::new("/tmp/My Presets/a look.toml");
        assert_eq!(
            reveal_command(path).get_args().collect::<Vec<_>>(),
            vec![path.as_os_str()]
        );
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
        let b = Browser::new(&restored.sections);
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
        let applied = preset.apply_section(&RuntimeParams::default()).unwrap();
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
