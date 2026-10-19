use super::*;
use crate::theme;

pub struct Browser {
    pub state: BrowserState,
    pub(crate) library: PresetLibrary,
    pub(crate) entries: Vec<LibraryEntry>,
    pub(crate) scan_error: Option<String>,
    pub(crate) name_dialog: Option<NameDialog>,
    pub(crate) delete: Option<String>,
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
    pub(crate) fn refresh(&mut self) {
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
                }
            });
            if !expanded {
                return;
            }
            // The save action sits on its own row rather than beside the
            // heading: a labeled button needs the panel's width, and the
            // browser is collapsible down to a narrow strip.
            if ui
                .button("Save preset…")
                .on_hover_text("Save current look as a preset")
                .clicked()
            {
                self.save_preset_as();
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
                        .filter(|(_, e)| e.group == group && e.fault.is_none())
                        .peekable();
                    if entries.peek().is_none() && group == PresetGroup::User {
                        ui.weak("Use Save preset to save the current look.");
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
                        // Card-style preset entry with route metadata
                        let card_frame = if active {
                            egui::Frame::NONE
                                .fill(theme::ACCENT)
                                .stroke(egui::Stroke::new(1.0_f32, theme::ACCENT))
                                .corner_radius(6.0)
                                .inner_margin(8.0)
                        } else {
                            egui::Frame::NONE
                                .fill(theme::SURFACE2)
                                .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
                                .corner_radius(6.0)
                                .inner_margin(8.0)
                        };
                        card_frame.show(ui, |ui| {
                            let response = ui.selectable_label(
                                active,
                                egui::RichText::new(format!("{}{suffix}", entry.name))
                                    .color(if active {
                                        theme::CONTRAST_ON_ACCENT
                                    } else {
                                        theme::FG_SECONDARY
                                    })
                                    .size(11.0),
                            );
                            if response.clicked() {
                                action = Some((index, PresetAction::Load));
                            }
                            response.context_menu(|ui| {
                                if let Some(selected) = self.preset_actions(ui, entry) {
                                    action = Some((index, selected));
                                }
                            });
                            // Film → print route metadata
                            ui.horizontal(|ui| {
                                // Resolve through the same profile entries the
                                // header and the combo use, so one stock has
                                // one visual identifier.
                                let film_label = entry
                                    .film_profile
                                    .as_ref()
                                    .map(|p| {
                                        crate::profiles::selected_display(&self.films, &p.stock)
                                    })
                                    .unwrap_or_else(|| "?".to_owned());
                                let print_label = entry
                                    .print_profile
                                    .as_ref()
                                    .map(|p| {
                                        crate::profiles::selected_display(&self.papers, &p.stock)
                                    })
                                    .unwrap_or_else(|| "?".to_owned());
                                ui.label(
                                    egui::RichText::new(film_label)
                                        .color(theme::ACCENT_DIM)
                                        .size(theme::SIZE_HINT),
                                );
                                ui.label(
                                    egui::RichText::new("→")
                                        .color(theme::FG_MUTED)
                                        .size(theme::SIZE_HINT),
                                );
                                ui.label(
                                    egui::RichText::new(print_label)
                                        .color(theme::ACCENT_DIM)
                                        .size(theme::SIZE_HINT),
                                );
                            });
                            // A visible action control beside the entry name. The
                            // context menu stays as an additional route, but the
                            // same actions are reachable here by pointer and by
                            // keyboard: the button takes focus, and Space or
                            // Enter opens its menu.
                            let entry_id = ui.id().with(("preset actions", &entry.name));
                            let actions = ui.push_id(entry_id, |ui| {
                                let actions = ui.menu_button(
                                    egui::RichText::new("preset actions")
                                        .color(theme::FG_MUTED)
                                        .size(theme::SIZE_HINT),
                                    |ui| self.preset_actions(ui, entry),
                                );
                                // Name the control after its entry, so the
                                // action menu of one card is addressable among
                                // the cards.
                                ui.ctx()
                                    .accesskit_node_builder(actions.response.id, |node| {
                                        node.set_label(format!(
                                            "preset actions for {}",
                                            entry.name
                                        ));
                                    });
                                actions.response.on_hover_text(format!(
                                    "Update, rename, duplicate, export, reveal or delete {}",
                                    entry.name
                                ));
                                actions.inner.flatten()
                            });
                            if let Some(selected) = actions.inner {
                                action = Some((index, selected));
                            }
                        });
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
                    ui.colored_label(theme::ERROR, error);
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
    fn preset_actions(&self, ui: &mut egui::Ui, entry: &LibraryEntry) -> Option<PresetAction> {
        entry.preset.as_ref()?;
        let mut action = None;
        if entry.group == PresetGroup::User {
            let active = self
                .presets
                .state
                .applied
                .as_ref()
                .is_some_and(|a| a.group == entry.group && a.name == entry.name);
            if ui
                .add_enabled(active, egui::Button::new("Update"))
                .clicked()
            {
                action = Some(PresetAction::Update);
                ui.close_menu();
            }
            if ui.button("Revert").clicked() {
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
                        ui.colored_label(theme::ERROR, e);
                    }
                    if let Some(e) = &dialog.error {
                        ui.colored_label(theme::ERROR, e);
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
                        if matches!(&dialog.action, NameAction::Save(_)) {
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
        theme::ERROR,
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
