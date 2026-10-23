use super::*;
use crate::theme;

pub struct Browser {
    pub state: BrowserState,
    /// Session-only presentation state; deliberately outside BrowserState.
    pub(crate) query: String,
    search: Option<(String, Vec<usize>)>,
    pub(crate) library: PresetLibrary,
    pub(crate) entries: Vec<LibraryEntry>,
    pub(crate) scan_error: Option<String>,
    pub(crate) name_dialog: Option<NameDialog>,
    pub(crate) delete: Option<String>,
    dialog_focus: Option<egui::Id>,
    dialog_open: bool,
    dialog_composing: bool,
}
impl Browser {
    pub fn new(extras: &serde_json::Value, library: PresetLibrary) -> Self {
        let mut browser = Self {
            state: serde_json::from_value(extras["rust"]["presets"].clone()).unwrap_or_default(),
            query: String::new(),
            search: None,
            library,
            entries: vec![],
            scan_error: None,
            name_dialog: None,
            delete: None,
            dialog_focus: None,
            dialog_open: false,
            dialog_composing: false,
        };
        browser.refresh();
        browser
    }
    pub(crate) fn refresh(&mut self) {
        self.search = None;
        match self.library.scan() {
            Ok(entries) => {
                self.entries = entries;
                self.scan_error = None;
            }
            Err(e) => self.scan_error = Some(e),
        }
    }

    pub(crate) fn update_search(
        &mut self,
        films: &[crate::profiles::ProfileEntry],
        prints: &[crate::profiles::ProfileEntry],
    ) {
        if self
            .search
            .as_ref()
            .is_some_and(|(query, _)| query == &self.query)
        {
            return;
        }
        let query = self.query.to_lowercase();
        let matches = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let is_look = entry.fault.is_none()
                    && entry
                        .preset
                        .as_ref()
                        .is_some_and(|preset| preset.kind() == Ok(PresetKind::Look));
                if !is_look {
                    return None;
                }
                let stock_matches = |stock: &str, profiles: &[crate::profiles::ProfileEntry]| {
                    stock.to_lowercase().contains(&query)
                        || profiles
                            .iter()
                            .find(|profile| profile.stock == stock)
                            .is_some_and(|profile| profile.display.to_lowercase().contains(&query))
                };
                (query.is_empty()
                    || entry.name.to_lowercase().contains(&query)
                    || entry
                        .film_profile
                        .as_ref()
                        .is_some_and(|profile| stock_matches(&profile.stock, films))
                    || entry
                        .print_profile
                        .as_ref()
                        .is_some_and(|profile| stock_matches(&profile.stock, prints)))
                .then_some(index)
            })
            .collect();
        self.search = Some((self.query.clone(), matches));
    }

    pub(crate) fn visible_indices(&self) -> &[usize] {
        self.search
            .as_ref()
            .map(|(_, matches)| matches.as_slice())
            .unwrap_or_default()
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
                    .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.presets.toggle"))
                    .clicked()
                {
                    self.presets.state.expanded = !expanded;
                }
                if expanded {
                    ui.heading(crate::i18n::text(ui.ctx(), "surfaces.presets.title"));
                }
            });
            if !expanded {
                return;
            }
            // The save action sits on its own row rather than beside the
            // heading: a labeled button needs the panel's width, and the
            // browser is collapsible down to a narrow strip.
            if ui
                .button(crate::i18n::text(ui.ctx(), "surfaces.presets.save"))
                .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.presets.save_help"))
                .clicked()
            {
                self.save_preset_as();
            }
            let search_label = ui.label(crate::i18n::text(ui.ctx(), "surfaces.presets.search"));
            ui.add(
                egui::TextEdit::singleline(&mut self.presets.query)
                    .id_salt("preset search")
                    .desired_width(ui.available_width())
                    .hint_text(crate::i18n::text(ui.ctx(), "surfaces.presets.search_hint")),
            )
            .labelled_by(search_label.id);
            if !self.presets.query.is_empty()
                && ui
                    .button(crate::i18n::text(ui.ctx(), "surfaces.presets.clear_search"))
                    .clicked()
            {
                self.presets.query.clear();
            }
            if let Some(applied) = &self.presets.state.applied {
                let state = match marker(
                    applied,
                    &self.presets.entries,
                    &self.params,
                    &self.film_name,
                    &self.print_name,
                ) {
                    Marker::Applied => "applied",
                    Marker::Modified => "modified",
                    Marker::Missing => "missing",
                };
                let group = if applied.group == PresetGroup::BuiltIn {
                    "Built-in"
                } else {
                    "My Presets"
                };
                ui.add(
                    egui::Label::new(crate::tr!(
                        ui.ctx(),
                        "surfaces.presets.current",
                        group = crate::controls::localized(ui.ctx(), group),
                        name = &applied.name,
                        state = crate::controls::localized(ui.ctx(), state)
                    ))
                    .wrap(),
                );
            }
            self.presets.update_search(&self.films, &self.papers);
            if !self.presets.query.is_empty() && self.presets.visible_indices().is_empty() {
                ui.label(crate::i18n::text(ui.ctx(), "surfaces.presets.no_matches"));
            }
            egui::ScrollArea::vertical()
                .id_salt("preset browser scroll")
                .show(ui, |ui| {
                    for group in [PresetGroup::BuiltIn, PresetGroup::User] {
                        ui.separator();
                        ui.heading(crate::i18n::text(
                            ui.ctx(),
                            if group == PresetGroup::BuiltIn {
                                "surfaces.presets.builtin"
                            } else {
                                "surfaces.presets.mine"
                            },
                        ));
                        if group == PresetGroup::User {
                            let directory = self.presets.library.directory().to_path_buf();
                            ui.add(egui::Label::new(directory.display().to_string()).wrap());
                            ui.horizontal_wrapped(|ui| {
                                if ui
                                    .button(crate::i18n::text(
                                        ui.ctx(),
                                        "surfaces.presets.show_directory",
                                    ))
                                    .clicked()
                                    && let Err(e) = reveal(&directory)
                                {
                                    self.status = e;
                                }
                                if ui
                                    .button(crate::i18n::text(ui.ctx(), "surfaces.presets.refresh"))
                                    .clicked()
                                {
                                    self.presets.refresh();
                                    self.status = crate::app::Message::new(
                                        "surfaces.status.presets_refreshed",
                                    );
                                }
                                if ui
                                    .button(crate::i18n::text(ui.ctx(), "surfaces.presets.import"))
                                    .clicked()
                                {
                                    self.import_preset();
                                }
                            });
                        }
                        self.presets.update_search(&self.films, &self.papers);
                        let mut action = None;
                        let mut entries = self
                            .presets
                            .visible_indices()
                            .iter()
                            .copied()
                            .map(|index| (index, &self.presets.entries[index]))
                            .filter(|(_, entry)| entry.group == group)
                            .peekable();
                        if entries.peek().is_none() {
                            let has_looks = self.presets.entries.iter().any(|entry| {
                                entry.group == group
                                    && entry
                                        .preset
                                        .as_ref()
                                        .is_some_and(|preset| preset.kind() == Ok(PresetKind::Look))
                            });
                            if has_looks {
                                ui.weak(crate::i18n::text(
                                    ui.ctx(),
                                    "surfaces.presets.group_no_matches",
                                ));
                            } else if group == PresetGroup::User {
                                ui.weak(crate::i18n::text(ui.ctx(), "surfaces.presets.empty"));
                            }
                        }
                        for (index, entry) in entries {
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
                                let response = ui.add(
                                    egui::Button::new(
                                        egui::RichText::new(format!(
                                            "{}{}",
                                            entry.name,
                                            crate::controls::localized(ui.ctx(), suffix)
                                        ))
                                        .color(if active {
                                            theme::CONTRAST_ON_ACCENT
                                        } else {
                                            theme::FG_SECONDARY
                                        })
                                        .size(11.0),
                                    )
                                    .selected(active)
                                    .wrap(),
                                );
                                if response.clicked() {
                                    action = Some((index, PresetAction::Load));
                                }
                                response.context_menu(|ui| {
                                    if let Some(selected) = self.preset_actions(ui, entry) {
                                        action = Some((index, selected));
                                    }
                                });
                                for (role, reference, profiles) in [
                                    ("Film", &entry.film_profile, &self.films),
                                    ("Print", &entry.print_profile, &self.papers),
                                ] {
                                    if let Some(reference) = reference {
                                        let display = crate::profiles::selected_display(
                                            profiles,
                                            &reference.stock,
                                        );
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(crate::tr!(
                                                    ui.ctx(),
                                                    "surfaces.presets.role",
                                                    role =
                                                        crate::controls::localized(ui.ctx(), role),
                                                    name = display
                                                ))
                                                .color(if active {
                                                    theme::CONTRAST_ON_ACCENT
                                                } else {
                                                    theme::ACCENT_DIM
                                                })
                                                .size(theme::SIZE_HINT),
                                            )
                                            .wrap(),
                                        )
                                        .on_hover_text(&reference.stock);
                                    }
                                }
                                // A visible action control beside the entry name. The
                                // context menu stays as an additional route, but the
                                // same actions are reachable here by pointer and by
                                // keyboard: the button takes focus, and Space or
                                // Enter opens its menu.
                                let entry_id = ui.id().with((
                                    "preset actions",
                                    entry.group == PresetGroup::User,
                                    &entry.name,
                                ));
                                let actions = ui.push_id(entry_id, |ui| {
                                    let actions = ui.menu_button(
                                        egui::RichText::new(crate::i18n::text(
                                            ui.ctx(),
                                            "surfaces.presets.actions",
                                        ))
                                        .color(theme::FG_MUTED)
                                        .size(theme::SIZE_HINT),
                                        |ui| self.preset_actions(ui, entry),
                                    );
                                    // Name the control after its entry, so the
                                    // action menu of one card is addressable among
                                    // the cards.
                                    let accessible_name = crate::tr!(
                                        ui.ctx(),
                                        "surfaces.presets.actions_name",
                                        name = &entry.name
                                    );
                                    ui.ctx()
                                        .accesskit_node_builder(actions.response.id, |node| {
                                            node.set_label(accessible_name);
                                        });
                                    actions.response.on_hover_text(crate::tr!(
                                        ui.ctx(),
                                        "surfaces.presets.actions_help",
                                        name = &entry.name
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
                    if let Some(error) = &self.presets.scan_error {
                        ui.colored_label(
                            theme::ERROR,
                            crate::tr!(ui.ctx(), "surfaces.presets.scan_error", detail = error),
                        );
                    }
                    if self.presets.entries.iter().any(|e| e.fault.is_some()) {
                        egui::CollapsingHeader::new(crate::i18n::text(
                            ui.ctx(),
                            "surfaces.presets.issues",
                        ))
                        .id_salt("Issues")
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
    /// One menu at the serialized section's primary module, not its reset scope.
    pub(crate) fn section_preset_menu(&mut self, ui: &mut egui::Ui, module: &str) {
        let Some((section, scope)) = section_anchor(module) else {
            return;
        };
        let matching = spektrafilm_core::presets::matching_section_preset_detailed(
            section,
            self.presets
                .entries
                .iter()
                .filter_map(|entry| entry.preset.as_ref()),
            &self.params,
        );
        let current = match &matching {
            Ok(Some(preset)) => std::borrow::Cow::Borrowed(preset.name.as_str()),
            _ => crate::i18n::text(ui.ctx(), "surfaces.presets.custom"),
        };
        let mut action = None;
        let mut save = false;
        ui.push_id(("section preset", module), |ui| {
            ui.menu_button(
                crate::tr!(ui.ctx(), "surfaces.presets.section", name = current),
                |ui| {
                    ui.add(
                        egui::Label::new(crate::tr!(
                            ui.ctx(),
                            "surfaces.presets.scope",
                            scope = crate::controls::localized(ui.ctx(), scope)
                        ))
                        .wrap(),
                    );
                    if let Err(error) = &matching {
                        ui.colored_label(
                            theme::ERROR,
                            document_error_message(error).localized(ui.ctx()),
                        );
                    }
                    if ui
                        .button(crate::i18n::text(ui.ctx(), "surfaces.presets.save_as"))
                        .clicked()
                    {
                        save = true;
                        ui.close_menu();
                    }
                    let mut any = false;
                    for group in [PresetGroup::BuiltIn, PresetGroup::User] {
                        for (index, entry) in
                            self.presets
                                .entries
                                .iter()
                                .enumerate()
                                .filter(|(_, entry)| {
                                    entry.group == group
                                        && entry.fault.is_none()
                                        && entry.preset.as_ref().is_some_and(|preset| {
                                            preset.kind() == Ok(PresetKind::Section(section))
                                        })
                                })
                        {
                            if !any {
                                ui.separator();
                            }
                            any = true;
                            let group_name = if group == PresetGroup::BuiltIn {
                                "Built-in"
                            } else {
                                "My Presets"
                            };
                            let identity = format!(
                                "{} / {}",
                                crate::controls::localized(ui.ctx(), group_name),
                                entry.name
                            );
                            if ui
                                .button(crate::tr!(
                                    ui.ctx(),
                                    "surfaces.presets.apply",
                                    identity = identity
                                ))
                                .clicked()
                            {
                                action = Some((index, PresetAction::Load));
                                ui.close_menu();
                            }
                            let management = [
                                (PresetAction::Rename, "Rename…", group == PresetGroup::User),
                                (
                                    PresetAction::Duplicate,
                                    if group == PresetGroup::BuiltIn {
                                        "Duplicate to My Presets…"
                                    } else {
                                        "Duplicate…"
                                    },
                                    true,
                                ),
                                (PresetAction::Export, "Export…", true),
                                (PresetAction::Reveal, "Show file", entry.path.is_some()),
                                (PresetAction::Delete, "Delete…", group == PresetGroup::User),
                            ];
                            for (selected, label, visible) in management {
                                if visible
                                    && ui
                                        .button(crate::tr!(
                                            ui.ctx(),
                                            "surfaces.presets.manage",
                                            action = crate::controls::localized(ui.ctx(), label),
                                            identity = identity
                                        ))
                                        .clicked()
                                {
                                    action = Some((index, selected));
                                    ui.close_menu();
                                }
                            }
                        }
                    }
                    if !any {
                        ui.label(crate::i18n::text(
                            ui.ctx(),
                            "surfaces.presets.section_empty",
                        ));
                    }
                },
            );
        });
        if save {
            self.save_section_preset_as(section);
        }
        if let Some((index, action)) = action {
            self.apply_preset_action(index, action);
        }
    }

    fn preset_actions(&self, ui: &mut egui::Ui, entry: &LibraryEntry) -> Option<PresetAction> {
        let preset = entry.preset.as_ref()?;
        let mut action = None;
        if entry.group == PresetGroup::User {
            let active = self
                .presets
                .state
                .applied
                .as_ref()
                .is_some_and(|a| a.group == entry.group && a.name == entry.name);
            if preset.kind() == Ok(PresetKind::Look) {
                if ui
                    .add_enabled(
                        active,
                        egui::Button::new(crate::i18n::text(ui.ctx(), "surfaces.presets.update")),
                    )
                    .clicked()
                {
                    action = Some(PresetAction::Update);
                    ui.close_menu();
                }
                if ui
                    .button(crate::i18n::text(ui.ctx(), "surfaces.presets.revert"))
                    .clicked()
                {
                    action = Some(PresetAction::Load);
                    ui.close_menu();
                }
            }
            if ui
                .button(crate::i18n::text(ui.ctx(), "surfaces.presets.rename"))
                .clicked()
            {
                action = Some(PresetAction::Rename);
                ui.close_menu();
            }
        }
        if ui
            .button(crate::i18n::text(
                ui.ctx(),
                if entry.group == PresetGroup::BuiltIn {
                    "surfaces.presets.duplicate_builtin"
                } else {
                    "surfaces.presets.duplicate"
                },
            ))
            .clicked()
        {
            action = Some(PresetAction::Duplicate);
            ui.close_menu();
        }
        if ui
            .button(crate::i18n::text(ui.ctx(), "surfaces.presets.export"))
            .clicked()
        {
            action = Some(PresetAction::Export);
            ui.close_menu();
        }
        if entry.path.is_some()
            && ui
                .button(crate::i18n::text(ui.ctx(), "surfaces.presets.show_file"))
                .clicked()
        {
            action = Some(PresetAction::Reveal);
            ui.close_menu();
        }
        if entry.group == PresetGroup::User
            && ui
                .button(crate::i18n::text(
                    ui.ctx(),
                    "surfaces.presets.delete_action",
                ))
                .clicked()
        {
            action = Some(PresetAction::Delete);
            ui.close_menu();
        }
        action
    }
    fn preset_dialogs(&mut self, ctx: &egui::Context) {
        let mut composing = self.presets.dialog_composing;
        let mut ime_frame = composing;
        ctx.input(|input| {
            for event in &input.events {
                if let egui::Event::Ime(event) = event {
                    ime_frame = true;
                    match event {
                        egui::ImeEvent::Enabled | egui::ImeEvent::Preedit(_) => composing = true,
                        egui::ImeEvent::Commit(_) | egui::ImeEvent::Disabled => composing = false,
                    }
                }
            }
        });
        self.presets.dialog_composing = composing;
        if self.presets.name_dialog.is_some() && ime_frame {
            // The IME owns confirmation/cancellation while composing. Filter
            // these keys before TextEdit sees them so it retains focus and its
            // composition range for the following native commit event.
            ctx.input_mut(|input| {
                input.events.retain(|event| {
                    !matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::Enter | egui::Key::Escape,
                            ..
                        }
                    )
                });
            });
        }
        let opening = !self.presets.dialog_open
            && (self.presets.name_dialog.is_some() || self.presets.delete.is_some());
        if opening {
            self.presets.dialog_focus = ctx.memory(|memory| memory.focused());
            self.presets.dialog_open = true;
        }
        if let Some(mut dialog) = self.presets.name_dialog.take() {
            let mut cancel = false;
            let mut submit = false;
            egui::Window::new(crate::i18n::text(ctx, "surfaces.presets.name_title"))
                .id(egui::Id::new("Preset name"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    let label =
                        ui.label(crate::i18n::text(ui.ctx(), "surfaces.presets.name_label"));
                    let editor = ui
                        .text_edit_singleline(&mut dialog.name)
                        .labelled_by(label.id);
                    if opening {
                        editor.request_focus();
                    }
                    let rename_from = match &dialog.action {
                        NameAction::Rename(old) => Some(old.as_str()),
                        _ => None,
                    };
                    let invalid = name_error(&dialog.name, &self.presets.entries, rename_from);
                    if let Some(e) = &invalid {
                        ui.colored_label(theme::ERROR, e.localized(ui.ctx()));
                    }
                    if let Some(e) = &dialog.error {
                        ui.colored_label(theme::ERROR, e.localized(ui.ctx()));
                    }
                    ui.horizontal(|ui| {
                        submit = ui
                            .add_enabled(
                                invalid.is_none(),
                                egui::Button::new(crate::i18n::text(
                                    ui.ctx(),
                                    "surfaces.action.save",
                                )),
                            )
                            .clicked();
                        cancel = ui
                            .button(crate::i18n::text(ui.ctx(), "surfaces.action.cancel"))
                            .clicked();
                    });
                    if !ime_frame {
                        cancel |= ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                        });
                        if invalid.is_none() {
                            submit |= ui.input_mut(|input| {
                                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                            });
                        }
                    }
                });
            if submit {
                let result = match &dialog.action {
                    NameAction::Save(preset) => {
                        let mut preset = preset.clone();
                        preset.name = dialog.name.clone();
                        self.presets
                            .library
                            .save_new_detailed(&preset, &self.data_dir)
                    }
                    NameAction::Rename(old) => {
                        self.presets
                            .library
                            .rename_detailed(old, &dialog.name, &self.data_dir)
                    }
                    NameAction::Duplicate(preset) => self.presets.library.duplicate_detailed(
                        preset,
                        &dialog.name,
                        &self.data_dir,
                    ),
                    NameAction::Import(path) => self.presets.library.import_detailed(
                        path,
                        Some(&dialog.name),
                        &self.data_dir,
                    ),
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
                        if matches!(&dialog.action, NameAction::Save(preset) if preset.kind() == Ok(PresetKind::Look))
                        {
                            self.presets.state.applied = Some(AppliedPreset {
                                group: PresetGroup::User,
                                name: dialog.name.clone(),
                            });
                        }
                        self.status = crate::app::Message::new("surfaces.status.preset_saved")
                            .arg("name", &dialog.name)
                            .arg("path", path.display());
                        self.presets.refresh();
                    }
                    Err(e) => {
                        let detail = library_error_message(&e);
                        self.status = crate::app::Message::new("surfaces.status.preset_error")
                            .message_arg("error", detail.clone());
                        dialog.error = Some(detail);
                        self.presets.name_dialog = Some(dialog);
                    }
                }
            } else if !cancel {
                self.presets.name_dialog = Some(dialog);
            }
        }
        if let Some(name) = self.presets.delete.clone() {
            egui::Window::new(crate::i18n::text(ctx, "surfaces.presets.delete_title"))
                .id(egui::Id::new("Delete preset?"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(crate::tr!(
                        ui.ctx(),
                        "surfaces.presets.delete_prompt",
                        name = &name
                    ));
                    ui.horizontal(|ui| {
                        if ui
                            .button(crate::i18n::text(ui.ctx(), "surfaces.action.delete"))
                            .clicked()
                        {
                            self.status = match self.presets.library.delete_detailed(&name) {
                                Ok(()) => {
                                    crate::app::Message::new("surfaces.status.preset_deleted")
                                        .arg("name", &name)
                                }
                                Err(e) => crate::app::Message::new("surfaces.status.preset_error")
                                    .message_arg("error", library_error_message(&e)),
                            };
                            self.presets.delete = None;
                            self.presets.refresh();
                        }
                        if ui
                            .button(crate::i18n::text(ui.ctx(), "surfaces.action.cancel"))
                            .clicked()
                        {
                            self.presets.delete = None;
                        }
                    });
                    if ui.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                    }) {
                        self.presets.delete = None;
                    }
                });
        }
        if self.presets.dialog_open
            && self.presets.name_dialog.is_none()
            && self.presets.delete.is_none()
        {
            self.presets.dialog_open = false;
            if let Some(id) = self.presets.dialog_focus.take() {
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
        }
    }
}
pub(super) fn document_error_message(
    error: &spektrafilm_core::presets::PresetDocumentError,
) -> crate::app::Message {
    use spektrafilm_core::presets::PresetDocumentReason;
    match &error.reason {
        PresetDocumentReason::Parser { detail } => {
            crate::app::Message::new("surfaces.presets.document_invalid").arg("detail", detail)
        }
        PresetDocumentReason::Carrier { path } => {
            crate::app::Message::new("surfaces.presets.carrier_invalid").arg("path", path.display())
        }
        PresetDocumentReason::Schema { version } => {
            crate::app::Message::new("surfaces.presets.schema_version").arg("version", version)
        }
        PresetDocumentReason::Name { name } => {
            crate::app::Message::new("surfaces.presets.name_guidance").arg("name", name)
        }
        PresetDocumentReason::ProvenanceVersions => {
            crate::app::Message::new("surfaces.presets.provenance_versions")
        }
        PresetDocumentReason::Shape => crate::app::Message::new("surfaces.presets.shape"),
        PresetDocumentReason::Stock { kind, stock } => {
            crate::app::Message::new("surfaces.presets.stock_invalid")
                .message_arg("kind", crate::app::Message::control(kind))
                .arg("stock", stock)
        }
        PresetDocumentReason::ContentIdentity { kind, identity } => {
            crate::app::Message::new("surfaces.presets.identity_invalid")
                .message_arg("kind", crate::app::Message::control(kind))
                .arg("identity", identity)
        }
        PresetDocumentReason::ScannerProvenance => {
            crate::app::Message::new("surfaces.presets.scanner_provenance")
        }
        PresetDocumentReason::RootObject => {
            crate::app::Message::new("surfaces.presets.root_object")
        }
        PresetDocumentReason::ParametersObject => {
            crate::app::Message::new("surfaces.presets.parameters_object")
        }
        PresetDocumentReason::LegacyField { field } => {
            crate::app::Message::new("surfaces.presets.legacy_field").arg("field", field)
        }
        PresetDocumentReason::UnownedField { path } => {
            crate::app::Message::new("surfaces.presets.unowned_field").arg("path", path)
        }
        PresetDocumentReason::RetiredField { path, replacement } => {
            crate::app::Message::new("surfaces.presets.retired_field")
                .arg("path", path)
                .arg("replacement", replacement)
        }
        PresetDocumentReason::EmptySection => {
            crate::app::Message::new("surfaces.presets.empty_section")
        }
        PresetDocumentReason::MultipleSections => {
            crate::app::Message::new("surfaces.presets.multiple_sections")
        }
        PresetDocumentReason::TomlNull { paths } => {
            crate::app::Message::new("surfaces.presets.toml_null").arg("paths", paths.join(", "))
        }
        PresetDocumentReason::ExpectedLook => {
            crate::app::Message::new("surfaces.presets.expected_look")
        }
        PresetDocumentReason::ExpectedSection => {
            crate::app::Message::new("surfaces.presets.expected_section")
        }
        PresetDocumentReason::Profile { kind, stock, error } => {
            crate::app::Message::new("surfaces.presets.profile_load")
                .message_arg("kind", crate::app::Message::control(kind))
                .arg("stock", stock)
                .message_arg("detail", crate::app::Message::profile(error.as_ref()))
        }
        PresetDocumentReason::ProfileRead {
            stock,
            path,
            detail,
        } => crate::app::Message::new("surfaces.presets.profile_read")
            .arg("stock", stock)
            .arg("path", path.display())
            .arg("detail", detail),
        PresetDocumentReason::Read { path, detail } => {
            crate::app::Message::new("surfaces.presets.document_read")
                .arg("path", path.display())
                .arg("detail", detail)
        }
        PresetDocumentReason::ProfileIdentity {
            stock,
            expected,
            actual,
        } => crate::app::Message::new("surfaces.presets.profile_identity")
            .arg("stock", stock)
            .arg("expected", expected)
            .arg("actual", actual),
        PresetDocumentReason::ProfileStock { kind, stock } => {
            crate::app::Message::new("surfaces.presets.profile_stock")
                .message_arg("kind", crate::app::Message::control(kind))
                .arg("stock", stock)
        }
        PresetDocumentReason::ProfileSupport { kind, stock } => {
            crate::app::Message::new("surfaces.presets.profile_support")
                .message_arg("kind", crate::app::Message::control(kind))
                .arg("stock", stock)
        }
        PresetDocumentReason::ProfileVersion {
            kind,
            stock,
            expected,
            actual,
        } => crate::app::Message::new("surfaces.presets.profile_version")
            .message_arg("kind", crate::app::Message::control(kind))
            .arg("stock", stock)
            .arg("expected", expected)
            .arg("actual", actual),
        PresetDocumentReason::ProfileUnverified { kind, stock } => {
            crate::app::Message::new("surfaces.presets.profile_unverified")
                .message_arg("kind", crate::app::Message::control(kind))
                .arg("stock", stock)
        }
        PresetDocumentReason::Runtime(error) => crate::app::Message::validation(error),
    }
}
pub(super) fn library_error_message(
    error: &spektrafilm_core::presets::LibraryError,
) -> crate::app::Message {
    use spektrafilm_core::presets::LibraryErrorReason;
    match &error.reason {
        LibraryErrorReason::Document { path, error } => match path {
            Some(path) => crate::app::Message::new("surfaces.presets.fault")
                .arg("path", path.display())
                .message_arg("detail", document_error_message(error)),
            None => document_error_message(error),
        },
        LibraryErrorReason::Fault { path, fault } => {
            crate::app::Message::new("surfaces.presets.fault")
                .arg("path", path.display())
                .message_arg("detail", library_fault_message(Some(fault)))
        }
        LibraryErrorReason::NameConflict { name } => {
            crate::app::Message::new("surfaces.presets.duplicate_name").arg("name", name)
        }
        LibraryErrorReason::UnknownName { name, suggestions } => {
            crate::app::Message::new("surfaces.presets.unknown_name")
                .arg("name", name)
                .arg("suggestions", suggestions.join(", "))
        }
        LibraryErrorReason::MissingPath => {
            crate::app::Message::new("surfaces.presets.missing_path")
        }
        LibraryErrorReason::Io {
            operation,
            path,
            detail,
        } => crate::app::Message::new(match *operation {
            "create" => "surfaces.presets.io_create",
            "resolve" => "surfaces.presets.io_resolve",
            "open_lock" => "surfaces.presets.io_open_lock",
            "lock" => "surfaces.presets.io_lock",
            "rename" => "surfaces.presets.io_rename",
            "remove" => "surfaces.presets.io_remove",
            "write" => "surfaces.presets.io_write",
            _ => unreachable!("library IO operation is declared at its owner"),
        })
        .arg("path", path.display())
        .arg("detail", detail),
        LibraryErrorReason::ExportName { path, name } => {
            crate::app::Message::new("surfaces.presets.export_name")
                .arg("path", path.display())
                .arg("name", name)
        }
    }
}
fn library_fault_message(
    fault: Option<&spektrafilm_core::presets::LibraryFault>,
) -> crate::app::Message {
    use spektrafilm_core::presets::LibraryFault;
    match fault {
        Some(LibraryFault::DirectoryRead { detail }) => {
            crate::app::Message::new("surfaces.presets.directory_read").arg("detail", detail)
        }
        Some(LibraryFault::Document { detail }) => document_error_message(detail),
        Some(LibraryFault::NameMismatch { name, stem }) => {
            crate::app::Message::new("surfaces.presets.name_mismatch")
                .arg("name", name)
                .arg("stem", stem)
        }
        Some(LibraryFault::DuplicateName { name }) => {
            crate::app::Message::new("surfaces.presets.duplicate_library_name").arg("name", name)
        }
        None => crate::app::Message::new("surfaces.presets.invalid_document"),
    }
}
fn show_fault(ui: &mut egui::Ui, entry: &LibraryEntry) {
    let detail = library_fault_message(entry.fault.as_ref());
    ui.colored_label(
        theme::ERROR,
        crate::tr!(
            ui.ctx(),
            "surfaces.presets.fault",
            path = entry
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| entry.name.clone()),
            detail = detail.localized(ui.ctx())
        ),
    );
}
