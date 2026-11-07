use super::thumbnail_worker::ThumbnailKey;
use super::thumbnails::Status;
use super::*;
use crate::theme;

pub struct Browser {
    pub state: BrowserState,
    /// Session-only presentation state; deliberately outside BrowserState.
    pub(crate) query: String,
    pub(super) search: Option<(String, Vec<usize>)>,
    /// Row to scroll into view after a successful addition.
    pub(super) reveal: Option<(PresetGroup, String)>,
    /// Preset-library generation; candidate runtime state is rebuilt when it
    /// changes, so referenced documents are re-read.
    pub(super) refresh_epoch: u64,
    thumbnails: super::thumbnails::Thumbnails,
    pub(crate) library: PresetLibrary,
    pub(crate) entries: Vec<LibraryEntry>,
    pub(crate) scan_error: Option<String>,
    pub(crate) name_dialog: Option<NameDialog>,
    pub(crate) delete: Option<String>,
    /// Session-only narrow-window reopen; deliberately outside BrowserState,
    /// so resizing never rewrites the stored `expanded` preference.
    pub(crate) narrow_open: bool,
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
            reveal: None,
            refresh_epoch: 0,
            thumbnails: Default::default(),
            library,
            entries: vec![],
            scan_error: None,
            name_dialog: None,
            delete: None,
            narrow_open: false,
            dialog_focus: None,
            dialog_open: false,
            dialog_composing: false,
        };
        browser.refresh();
        browser
    }
    pub(crate) fn refresh(&mut self) {
        self.search = None;
        self.thumbnails.update_content(&[]);
        self.refresh_epoch = self.refresh_epoch.wrapping_add(1);
        match self.library.scan() {
            Ok(entries) => {
                self.entries = entries;
                self.scan_error = None;
                self.thumbnails.update_content(&self.entries);
            }
            Err(e) => {
                self.entries.clear();
                self.scan_error = Some(e);
            }
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
        // Responsive collapse: at compact workspace widths the library lives in
        // the narrow rail unless the user reopens it for the session.
        // `state.expanded` is the stored preference: a resize never writes it,
        // and widening the window returns to it.
        let wide = ctx.screen_rect().width() > theme::WIDTH_COMPACT_WORKSPACE;
        let expanded = if wide {
            self.presets.state.expanded
        } else {
            self.presets.narrow_open
        };
        let panel_id = match (expanded, wide) {
            (false, _) => "presets-rail",
            (true, false) => "presets-narrow",
            (true, true) => "presets",
        };
        let panel = egui::SidePanel::left(panel_id)
            .resizable(expanded)
            .default_width(self.presets.state.width)
            .min_width(if expanded { 180.0 } else { RAIL_WIDTH });
        let panel = if expanded {
            panel
        } else {
            panel.exact_width(RAIL_WIDTH)
        };
        let response = panel.show(ctx, |ui| {
            // Content cannot resize the user's panel or its saved preference.
            let panel_rect = ui.available_rect_before_wrap();
            ui.allocate_space(panel_rect.size());
            let mut content = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("preset-content")
                    .max_rect(panel_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            content.set_clip_rect(panel_rect);
            let ui = &mut content;
            if self.input_transition_pending() {
                ui.disable();
            }
            if expanded {
                // Title and the named save action share one compact row; the
                // collapse and library controls stay reachable at its end.
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.presets.title"))
                            .size(theme::SIZE_TITLE)
                            .strong(),
                    );
                    let save_name = crate::i18n::text(ui.ctx(), "surfaces.presets.save");
                    let save = ui
                        .add(egui::Button::new(save_name.as_ref()).truncate())
                        .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.presets.save_help"));
                    crate::controls::locate_row(ui, "presets.save", &save);
                    if save.clicked() {
                        self.save_preset_as();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let toggle_name = crate::i18n::text(ui.ctx(), "surfaces.presets.toggle");
                        let toggle = quiet_button(ui, |ui| {
                            crate::controls::icon_button(
                                ui,
                                toggle_name.as_ref(),
                                crate::controls::Icon::ChevronLeft,
                            )
                        });
                        if toggle.clicked() {
                            if wide {
                                self.presets.state.expanded = false;
                            } else {
                                self.presets.narrow_open = false;
                            }
                        }
                        self.preset_library_menu(ui);
                    });
                });
            } else {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.add_space(theme::SPACE_2);
                    let toggle_name = crate::i18n::text(ui.ctx(), "surfaces.presets.toggle");
                    if quiet_button(ui, |ui| {
                        crate::controls::icon_button(
                            ui,
                            toggle_name.as_ref(),
                            crate::controls::Icon::ChevronRight,
                        )
                    })
                    .clicked()
                    {
                        if wide {
                            self.presets.state.expanded = true;
                        } else {
                            self.presets.narrow_open = true;
                        }
                    }
                    ui.label(
                        egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.presets.title"))
                            .size(theme::SIZE_HINT)
                            .color(theme::FG_MUTED),
                    );
                });
            }
            if !expanded {
                self.presets.thumbnails.suspend();
                return;
            }
            ui.horizontal(|ui| {
                let search_response = ui.add_sized(
                    egui::vec2(
                        (ui.available_width() - theme::HEIGHT_CONTROL - theme::SPACE_2).max(0.0),
                        theme::HEIGHT_CONTROL,
                    ),
                    egui::TextEdit::singleline(&mut self.presets.query)
                        .id_salt("preset search")
                        .margin(egui::vec2(theme::SPACE_2, 5.0))
                        .desired_width(f32::INFINITY)
                        .hint_text(crate::i18n::text(ui.ctx(), "surfaces.presets.search_hint")),
                );
                // The hint states the wording on screen, so the redundant visible
                // label is gone while the field keeps the same accessible name.
                let search_name = crate::i18n::text(ui.ctx(), "surfaces.presets.search");
                ui.ctx().accesskit_node_builder(search_response.id, |node| {
                    node.set_label(search_name.as_ref());
                });
                crate::controls::locate_row(ui, "presets.search", &search_response);
                if !self.presets.query.is_empty()
                    && ui
                        .add_sized(
                            egui::vec2(theme::HEIGHT_CONTROL, theme::HEIGHT_CONTROL),
                            egui::Button::new("×"),
                        )
                        .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.presets.clear_search"))
                        .clicked()
                {
                    self.presets.query.clear();
                }
            });
            // The summary is independent of the search. Matching the default
            // parameters never manufactures an applied preset identity.
            let current = egui::Frame::NONE
                .inner_margin(theme::SPACE_2)
                .show(ui, |ui| {
                    ui.small(crate::i18n::text(
                        ui.ctx(),
                        "surfaces.presets.current_title",
                    ));
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
                        let group_label = crate::controls::localized(ui.ctx(), group);
                        let summary = crate::tr!(
                            ui.ctx(),
                            "surfaces.presets.current",
                            group = group_label.clone(),
                            name = &applied.name,
                            state = crate::controls::localized(ui.ctx(), state)
                        );
                        // Group identity stays visible even when a search removes
                        // the applied row itself; names may repeat across groups.
                        ui.horizontal(|ui| {
                            let (slot, _) = ui.allocate_exact_size(
                                egui::vec2(CHECK_ICON, theme::SIZE_LABEL),
                                egui::Sense::hover(),
                            );
                            paint_check(ui.painter(), slot);
                            ui.label(
                                egui::RichText::new(group_label.as_ref())
                                    .small()
                                    .color(theme::FG_MUTED),
                            );
                            ui.label(egui::RichText::new("/").small().color(theme::FG_MUTED));
                            ui.add(
                                egui::Label::new(egui::RichText::new(&applied.name).strong())
                                    .truncate(),
                            )
                            .on_hover_text(summary.as_ref());
                        });
                        ui.small(crate::controls::localized(ui.ctx(), state));
                        Some(summary)
                    } else {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(crate::i18n::text(
                                    ui.ctx(),
                                    "surfaces.presets.unapplied",
                                ))
                                .size(theme::SIZE_HINT)
                                .color(theme::FG_SECONDARY),
                            )
                            .wrap(),
                        );
                        None
                    }
                });
            if let Some(summary) = current.inner {
                current.response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Other,
                        ui.is_enabled(),
                        summary.as_ref(),
                    )
                });
                ui.ctx()
                    .accesskit_node_builder(current.response.id, |node| {
                        node.set_label(summary.as_ref());
                    });
            }
            self.presets.update_search(&self.films, &self.papers);
            if !self.presets.query.is_empty() && self.presets.visible_indices().is_empty() {
                ui.label(crate::i18n::text(ui.ctx(), "surfaces.presets.no_matches"));
                if ui
                    .button(crate::i18n::text(ui.ctx(), "surfaces.presets.clear_search"))
                    .clicked()
                {
                    self.presets.query.clear();
                }
            }
            let signature = self.presets.thumbnails.accepted_signature(self);
            if let Some(recheck) = self.presets.thumbnails.display_recheck_at() {
                ctx.request_repaint_after(
                    recheck
                        .saturating_duration_since(std::time::Instant::now())
                        .max(std::time::Duration::from_millis(1)),
                );
            }
            if self.presets.thumbnails.signature() != signature {
                let now = ui.ctx().input(|input| input.time);
                let context = signature.map(|_| self.presets.thumbnails.accepted_context(self));
                self.presets.thumbnails.reset(signature, context, now);
            }
            let visible = self.presets.visible_indices().to_vec();
            // Keyboard order follows the presentation order across both groups,
            // so arrow keys never stop at a group boundary.
            let ordering = presentation_order(&visible, &self.presets.entries);
            let mut navigation = vec![[None, None]; self.presets.entries.len()];
            for (position, index) in ordering.iter().enumerate() {
                navigation[*index] = [
                    position
                        .checked_sub(1)
                        .map(|previous| card_id(ordering[previous])),
                    ordering.get(position + 1).copied().map(card_id),
                ];
            }
            // A card can only request a render for the accepted context it is
            // already cached under; listing keys is otherwise free.
            let mut keys: Vec<Option<ThumbnailKey>> = vec![None; self.presets.entries.len()];
            for index in &visible {
                let entry = &self.presets.entries[*index];
                if let Some(signature) = signature {
                    keys[*index] = Some(self.presets.thumbnails.candidate_key(signature, entry));
                }
            }
            let mut demand: Vec<(ThumbnailKey, Preset)> = Vec::new();
            egui::ScrollArea::vertical()
                .id_salt("preset browser scroll")
                .show(ui, |ui| {
                    if let Some(error) = &self.presets.scan_error {
                        ui.colored_label(
                            theme::ERROR,
                            crate::tr!(ui.ctx(), "surfaces.presets.scan_error", detail = error),
                        );
                    }
                    if self.presets.entries.iter().any(|e| e.fault.is_some()) {
                        // Same disclosure presentation as the inspector's
                        // disclosures: a section-title label and an arrow that
                        // shows the expanded state without relying on color.
                        egui::CollapsingHeader::new(
                            egui::RichText::new(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.presets.issues",
                            ))
                            .color(theme::FG)
                            .size(theme::SIZE_TITLE)
                            .strong(),
                        )
                        .id_salt("Issues")
                        .default_open(true)
                        .show(ui, |ui| {
                            for entry in self.presets.entries.iter().filter(|e| e.fault.is_some()) {
                                show_fault(ui, entry);
                            }
                        });
                    }
                    for group in [PresetGroup::BuiltIn, PresetGroup::User] {
                        ui.add_space(theme::SPACE_2);
                        ui.label(
                            egui::RichText::new(crate::i18n::text(
                                ui.ctx(),
                                if group == PresetGroup::BuiltIn {
                                    "surfaces.presets.builtin"
                                } else {
                                    "surfaces.presets.mine"
                                },
                            ))
                            .size(theme::SIZE_HINT)
                            .color(theme::FG_SECONDARY)
                            .strong(),
                        );
                        let indices: Vec<usize> = visible
                            .iter()
                            .copied()
                            .filter(|index| self.presets.entries[*index].group == group)
                            .collect();
                        if indices.is_empty() {
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
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(crate::i18n::text(
                                            ui.ctx(),
                                            "surfaces.presets.empty",
                                        ))
                                        .size(theme::SIZE_HINT)
                                        .color(theme::FG_SECONDARY),
                                    )
                                    .wrap(),
                                );
                            }
                        }
                        // Fit the columns inside the scroll viewport itself; the
                        // reserved scrollbar is not part of the usable width. A
                        // narrow panel keeps one reachable column.
                        let usable = (ui.clip_rect().width() - 2.0 * theme::SPACE_2).max(120.0);
                        let columns = if usable >= 250.0 { 2 } else { 1 };
                        let card_width = ((usable - theme::SPACE_2 * (columns - 1) as f32)
                            / columns as f32)
                            .max(110.0);
                        let mut action = None;
                        egui::Grid::new(("preset cards", group == PresetGroup::User))
                            .num_columns(columns)
                            .spacing(egui::vec2(theme::SPACE_2, theme::SPACE_2))
                            .show(ui, |ui| {
                                for (position, index) in indices.iter().enumerate() {
                                    let nav = navigation[*index];
                                    if let Some(selected) = self.preset_card(
                                        ui,
                                        *index,
                                        keys[*index].as_ref(),
                                        nav,
                                        card_width,
                                        &mut demand,
                                    ) {
                                        action = Some((*index, selected));
                                    }
                                    if (position + 1) % columns == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                        if let Some((index, action)) = action {
                            self.apply_preset_action(index, action);
                        }
                    }
                });
            self.presets.reveal = None;
            let foreground_busy =
                self.render_job.is_some() || self.dirty || self.export_job.is_some();
            self.presets
                .thumbnails
                .prepare(ui.ctx(), &demand, foreground_busy);
        });
        if expanded && wide {
            self.presets.state.width = response.response.rect.width();
        }
        self.preset_dialogs(ctx);
    }

    /// One unambiguous entry point for library-wide actions.
    fn preset_library_menu(&mut self, ui: &mut egui::Ui) {
        let popup_id = ui.make_persistent_id("preset-library-actions");
        let button = ui.add(
            egui::Button::new("")
                .min_size(egui::vec2(theme::HEIGHT_CONTROL, theme::HEIGHT_CONTROL)),
        );
        paint_actions(ui.painter(), button.rect);
        crate::controls::locate_row(ui, "presets.library", &button);
        let accessible_name = crate::i18n::text(ui.ctx(), "surfaces.presets.library");
        button.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                accessible_name.as_ref(),
            )
        });
        ui.ctx()
            .accesskit_node_builder(button.id, |node| node.set_label(accessible_name.as_ref()));
        crate::controls::focus_feedback(ui, &button);
        let mut selected = None;
        let button = button.on_hover_text(accessible_name);
        if button.clicked() {
            ui.memory_mut(|memory| memory.toggle_popup(popup_id));
        }
        let above = button.rect.bottom() + 160.0 > ui.ctx().screen_rect().bottom();
        egui::popup::popup_above_or_below_widget(
            ui,
            popup_id,
            &button,
            if above {
                egui::AboveOrBelow::Above
            } else {
                egui::AboveOrBelow::Below
            },
            egui::popup::PopupCloseBehavior::CloseOnClick,
            |ui| {
                ui.set_min_width(180.0);
                ui.spacing_mut().interact_size.y = theme::HEIGHT_CONTROL;
                ui.spacing_mut().button_padding.y = theme::SPACE_1;
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(self.presets.library.directory().display().to_string())
                            .size(theme::SIZE_HINT)
                            .color(theme::FG_MUTED),
                    )
                    .truncate(),
                );
                if ui
                    .button(crate::i18n::text(ui.ctx(), "surfaces.presets.import"))
                    .clicked()
                {
                    selected = Some(0_u8);
                    ui.close_menu();
                }
                if ui
                    .button(crate::i18n::text(
                        ui.ctx(),
                        "surfaces.presets.show_directory",
                    ))
                    .clicked()
                {
                    selected = Some(1_u8);
                    ui.close_menu();
                }
                if ui
                    .button(crate::i18n::text(ui.ctx(), "surfaces.presets.refresh"))
                    .clicked()
                {
                    selected = Some(2_u8);
                    ui.close_menu();
                }
            },
        );
        match selected {
            Some(0) => self.import_preset(),
            Some(1) => {
                let directory = self.presets.library.directory().to_path_buf();
                if let Err(e) = open_directory(&directory) {
                    self.status = e;
                }
            }
            Some(2) => {
                self.presets.refresh();
                self.status = crate::app::Message::new("surfaces.status.presets_refreshed");
            }
            _ => {}
        }
    }

    fn preset_card(
        &self,
        ui: &mut egui::Ui,
        index: usize,
        key: Option<&ThumbnailKey>,
        nav: [Option<egui::Id>; 2],
        card_width: f32,
        demand: &mut Vec<(ThumbnailKey, Preset)>,
    ) -> Option<PresetAction> {
        let entry = &self.presets.entries[index];
        let active = self
            .presets
            .state
            .applied
            .as_ref()
            .is_some_and(|a| a.group == entry.group && a.name == entry.name);
        let state = active.then(|| {
            match marker(
                self.presets.state.applied.as_ref().unwrap(),
                &self.presets.entries,
                &self.params,
                &self.film_name,
                &self.print_name,
            ) {
                Marker::Modified => "modified",
                Marker::Missing => "missing",
                Marker::Applied => "applied",
            }
        });
        let mut action = None;
        let inner = (card_width - 2.0 * theme::SPACE_1).max(0.0);
        let thumbnail_height = (inner * 2.0 / 3.0).max(44.0);
        // Reserve the whole card before laying out its rows: the full-width
        // thumbnail consumes the cell, so the later rows need their own region.
        let card_height = 2.0 * theme::SPACE_1
            + thumbnail_height
            + 2.0
            + theme::HEIGHT_CONTROL
            + if active { 2.0 + theme::SIZE_HINT } else { 0.0 };
        let (card_rect, _) =
            ui.allocate_exact_size(egui::vec2(card_width, card_height), egui::Sense::hover());
        if active {
            ui.painter().rect_filled(
                card_rect,
                egui::CornerRadius::same(theme::RADIUS_CARD),
                theme::SURFACE2,
            );
            ui.painter().rect_stroke(
                card_rect,
                egui::CornerRadius::same(theme::RADIUS_CARD),
                theme::stroke(theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        {
            let mut card = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(card_rect.shrink(theme::SPACE_1))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            let ui = &mut card;
            ui.spacing_mut().item_spacing = egui::vec2(theme::SPACE_1, 2.0);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(inner, thumbnail_height), egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(2), theme::SURFACE3);
            let status_text = match key {
                Some(key) => {
                    let text = match self.presets.thumbnails.status(key) {
                        Status::Ready(texture) => {
                            let available = rect.size();
                            let size = texture.size_vec2();
                            let scale = (available.x / size.x).min(available.y / size.y).min(1.0);
                            let target = egui::Rect::from_center_size(rect.center(), size * scale);
                            ui.painter().image(
                                texture.id(),
                                target,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                            None
                        }
                        Status::Pending => {
                            let pending =
                                crate::i18n::text(ui.ctx(), "surfaces.presets.thumbnail_pending");
                            paint_status_owned(ui, rect, pending.as_ref());
                            Some(pending.to_string())
                        }
                        Status::Failed(message) => {
                            let unavailable = crate::i18n::text(
                                ui.ctx(),
                                "surfaces.presets.thumbnail_unavailable",
                            );
                            paint_status_owned(ui, rect, unavailable.as_ref());
                            Some(format!("{unavailable}: {}", message.localized(ui.ctx())))
                        }
                    };
                    if ui.is_rect_visible(rect)
                        && let Some(preset) = &entry.preset
                    {
                        demand.push((key.clone(), preset.clone()));
                    }
                    text
                }
                None => {
                    let text = crate::i18n::text(ui.ctx(), "surfaces.presets.thumbnail_no_photo")
                        .to_string();
                    paint_status_owned(ui, rect, &text);
                    Some(text)
                }
            };
            // Thumbnail and name row are one activation surface; the actions
            // control is drawn later and therefore stays on top.
            let hit_rect = egui::Rect::from_min_size(
                rect.min,
                egui::vec2(inner, thumbnail_height + 2.0 + theme::HEIGHT_CONTROL),
            );
            let card_click = ui.interact(hit_rect, card_id(index), egui::Sense::click());
            crate::controls::locate_row(ui, "presets.card", &card_click);
            crate::controls::focus_feedback(ui, &card_click);
            if card_click.gained_focus() {
                card_click.scroll_to_me(Some(egui::Align::Center));
            }
            if card_click.clicked() {
                action = Some(PresetAction::Load);
            }
            if card_click.has_focus() {
                // Arrow keys move focus only; Enter applies, matching the
                // keyboard contract for the focused card.
                let (previous, next) = ui.input_mut(|input| {
                    let previous = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
                        || input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
                    let next = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
                        || input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
                    (previous, next)
                });
                let target = if previous {
                    nav[0]
                } else if next {
                    nav[1]
                } else {
                    None
                };
                if let Some(target) = target {
                    ui.memory_mut(|memory| memory.request_focus(target));
                }
                if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                {
                    action = Some(PresetAction::Load);
                }
            }
            card_click.context_menu(|ui| {
                if let Some(selected) = self.preset_actions(ui, entry) {
                    card_click.request_focus();
                    action = Some(selected);
                }
            });
            let controls = theme::HEIGHT_CONTROL + ui.spacing().item_spacing.x * 2.0;
            let name_width =
                (inner - controls - if active { CHECK_ICON + 2.0 } else { 0.0 }).max(20.0);
            ui.horizontal(|ui| {
                if active {
                    let (slot, _) = ui.allocate_exact_size(
                        egui::vec2(CHECK_ICON, theme::SIZE_LABEL),
                        egui::Sense::hover(),
                    );
                    paint_check(ui.painter(), slot);
                }
                // Painted text keeps the card as the only node carrying the
                // preset name in the accessibility tree.
                let mut job = egui::text::LayoutJob::default();
                job.append(
                    &entry.name,
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::proportional(theme::SIZE_LABEL),
                        color: theme::FG,
                        ..Default::default()
                    },
                );
                job.wrap.max_width = name_width;
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                let galley = ui.fonts(|fonts| fonts.layout_job(job));
                let (name_rect, _) = ui.allocate_exact_size(
                    egui::vec2(name_width, theme::SIZE_LABEL + 4.0),
                    egui::Sense::hover(),
                );
                ui.painter().galley(name_rect.min, galley, theme::FG);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let entry_id = ui.id().with((
                        "preset actions",
                        entry.group == PresetGroup::User,
                        &entry.name,
                    ));
                    if let Some(selected) = self.preset_actions_button(ui, entry_id, entry) {
                        action = Some(selected);
                    }
                });
            });
            if let Some(state) = state {
                ui.label(
                    egui::RichText::new(crate::controls::localized(ui.ctx(), state))
                        .size(theme::SIZE_HINT)
                        .color(theme::FG_SECONDARY),
                );
            }
            // Materials stay in the card's accessible description and hover
            // text rather than a repeated paragraph on every card.
            let mut label = entry.name.clone();
            if let Some(state) = state {
                label.push_str(" • ");
                label.push_str(&crate::controls::localized(ui.ctx(), state));
            }
            let mut description = String::new();
            for (role, reference, profiles) in [
                ("Film", &entry.film_profile, &self.films),
                ("Print", &entry.print_profile, &self.papers),
            ] {
                if let Some(reference) = reference {
                    if !description.is_empty() {
                        description.push_str("; ");
                    }
                    description.push_str(&crate::tr!(
                        ui.ctx(),
                        "surfaces.presets.role",
                        role = crate::controls::localized(ui.ctx(), role),
                        name = crate::profiles::selected_display(profiles, &reference.stock)
                    ));
                }
            }
            if let Some(text) = &status_text {
                if !description.is_empty() {
                    description.push_str("; ");
                }
                description.push_str(text);
            }
            card_click.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label.as_str())
            });
            ui.ctx().accesskit_node_builder(card_click.id, |node| {
                node.set_label(label.as_str());
                node.set_description(description.as_str());
            });
            card_click.on_hover_text(description);
            if self
                .presets
                .reveal
                .as_ref()
                .is_some_and(|(group, name)| *group == entry.group && name == &entry.name)
            {
                ui.scroll_to_rect(rect, Some(egui::Align::Center));
            }
        }
        if active {
            // The applied look's left rail: the solid cue that keeps company
            // with the check and the state word instead of a filled name.
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(card_rect.left(), card_rect.top() + theme::SPACE_1),
                    egui::pos2(
                        card_rect.left() + CARD_RAIL,
                        card_rect.bottom() - theme::SPACE_1,
                    ),
                ),
                egui::CornerRadius::same(1),
                theme::ACCENT,
            );
        }
        action
    }

    fn preset_actions_button(
        &self,
        ui: &mut egui::Ui,
        entry_id: egui::Id,
        entry: &LibraryEntry,
    ) -> Option<PresetAction> {
        ui.push_id(entry_id, |ui| {
            let actions = quiet_button(ui, |ui| {
                let popup_id = ui.make_persistent_id("preset-actions");
                let button = ui.add(
                    egui::Button::new("")
                        .min_size(egui::vec2(theme::HEIGHT_CONTROL, CARD_ACTION_HEIGHT)),
                );
                if button.clicked() {
                    ui.memory_mut(|memory| memory.toggle_popup(popup_id));
                }
                let above = button.rect.bottom() + 220.0 > ui.ctx().screen_rect().bottom();
                let selected = egui::popup::popup_above_or_below_widget(
                    ui,
                    popup_id,
                    &button,
                    if above {
                        egui::AboveOrBelow::Above
                    } else {
                        egui::AboveOrBelow::Below
                    },
                    egui::popup::PopupCloseBehavior::CloseOnClick,
                    |ui| {
                        ui.set_min_width(180.0);
                        ui.spacing_mut().interact_size.y = theme::HEIGHT_CONTROL;
                        ui.spacing_mut().button_padding.y = theme::SPACE_1;
                        self.preset_actions(ui, entry)
                    },
                );
                egui::InnerResponse::new(selected, button)
            });
            paint_actions(ui.painter(), actions.response.rect);
            let accessible_name = crate::tr!(
                ui.ctx(),
                "surfaces.presets.actions_name",
                name = &entry.name
            );
            actions.response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    accessible_name.as_ref(),
                )
            });
            ui.ctx()
                .accesskit_node_builder(actions.response.id, |node| {
                    node.set_label(accessible_name);
                });
            crate::controls::focus_feedback(ui, &actions.response);
            if actions.inner.as_ref().is_some_and(Option::is_some) {
                actions.response.request_focus();
            }
            if actions.response.gained_focus() {
                actions.response.scroll_to_me(Some(egui::Align::Center));
            }
            actions.response.on_hover_text(crate::tr!(
                ui.ctx(),
                "surfaces.presets.actions_help",
                name = &entry.name
            ));
            actions.inner.flatten()
        })
        .inner
    }

    fn preset_actions(&self, ui: &mut egui::Ui, entry: &LibraryEntry) -> Option<PresetAction> {
        let preset = entry.preset.as_ref()?;
        let mut action = None;
        if entry.group == PresetGroup::User
            && preset.kind() == Ok(PresetKind::Look)
            && ui
                .button(crate::i18n::text(ui.ctx(), "surfaces.presets.rename"))
                .clicked()
        {
            action = Some(PresetAction::Rename);
            ui.close_menu();
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
            // The key that activated a menu action belongs to that action,
            // not to the newly opened dialog's editor or default Save button.
            ctx.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                input.consume_key(egui::Modifiers::NONE, egui::Key::Space);
            });
        }
        if let Some(mut dialog) = self.presets.name_dialog.take() {
            let mut cancel = false;
            let mut submit = false;
            let mut replace = false;
            let mut collision = false;
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
                    // Saving over an existing look is only ever an explicit
                    // replacement: the collision disables Save and names the
                    // document it would replace.
                    collision = matches!(&dialog.action, NameAction::Save(_))
                        && self.presets.entries.iter().any(|entry| {
                            entry.group == PresetGroup::User
                                && entry.name.to_lowercase() == dialog.name.to_lowercase()
                                && entry.preset.is_some()
                        });
                    let invalid = if collision {
                        None
                    } else {
                        name_error(&dialog.name, &self.presets.entries, rename_from)
                    };
                    if let Some(e) = &invalid {
                        ui.colored_label(theme::ERROR, e.localized(ui.ctx()));
                    }
                    if let Some(e) = &dialog.error {
                        ui.colored_label(theme::ERROR, e.localized(ui.ctx()));
                    }
                    ui.horizontal(|ui| {
                        submit = ui
                            .add_enabled(
                                invalid.is_none() && !collision,
                                egui::Button::new(crate::i18n::text(
                                    ui.ctx(),
                                    "surfaces.action.save",
                                )),
                            )
                            .clicked();
                        if collision {
                            replace = ui
                                .button(crate::tr!(
                                    ui.ctx(),
                                    "surfaces.presets.replace",
                                    name = &dialog.name
                                ))
                                .clicked();
                        }
                        cancel = ui
                            .button(crate::i18n::text(ui.ctx(), "surfaces.action.cancel"))
                            .clicked();
                    });
                    if !ime_frame && ui.memory(|memory| memory.allows_interaction(ui.layer_id())) {
                        cancel |= ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                        });
                        if invalid.is_none() && !collision {
                            submit |= ui.input_mut(|input| {
                                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                            });
                        }
                    }
                });
            if submit || replace {
                let result = match &dialog.action {
                    NameAction::Save(preset) if collision => {
                        let mut preset = preset.clone();
                        preset.name = dialog.name.clone();
                        self.presets
                            .library
                            .replace_look_detailed(&preset, &self.data_dir)
                            .map_err(|error| library_error_message(&error))
                    }
                    NameAction::Save(preset) => {
                        let mut preset = preset.clone();
                        preset.name = dialog.name.clone();
                        self.presets
                            .library
                            .save_new_detailed(&preset, &self.data_dir)
                            .map_err(|error| library_error_message(&error))
                    }
                    NameAction::Rename(old) => self
                        .presets
                        .library
                        .rename_detailed(old, &dialog.name, &self.data_dir)
                        .map_err(|error| library_error_message(&error)),
                    NameAction::Duplicate(preset) => self
                        .presets
                        .library
                        .duplicate_detailed(preset, &dialog.name, &self.data_dir)
                        .map_err(|error| library_error_message(&error)),
                    NameAction::Import(path) => self.import_look_preset(path, Some(&dialog.name)),
                };
                match result {
                    Ok(path) => {
                        let published = path
                            .file_stem()
                            .and_then(|stem| stem.to_str())
                            .unwrap_or(&dialog.name)
                            .to_owned();
                        if let NameAction::Rename(old) = &dialog.action
                            && let Some(active) = self.presets.state.applied.as_mut()
                            && active.group == PresetGroup::User
                            && active.name == *old
                        {
                            active.name = published.clone();
                        }
                        if matches!(
                            &dialog.action,
                            NameAction::Save(preset) if preset.kind() == Ok(PresetKind::Look)
                        ) {
                            self.presets.state.applied = Some(AppliedPreset {
                                group: PresetGroup::User,
                                name: published.clone(),
                            });
                        }
                        self.status = crate::app::Message::new("surfaces.status.preset_saved")
                            .arg("name", &published)
                            .arg("path", path.display());
                        self.presets.query.clear();
                        self.presets.reveal = Some((PresetGroup::User, published));
                        self.presets.refresh();
                    }
                    Err(e) => {
                        let detail = e;
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
                    if ui.memory(|memory| memory.allows_interaction(ui.layer_id()))
                        && ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                        })
                    {
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
/// The collapsed library rail: a 56 pt strip leaves room for the visible
/// Presets label.
const RAIL_WIDTH: f32 = 56.0;
/// The visible actions control inside a card row.
const CARD_ACTION_HEIGHT: f32 = 22.0;
/// The size of the current look's painted check.
const CHECK_ICON: f32 = 12.0;
/// The current look's side rail width.
const CARD_RAIL: f32 = 2.0;

/// The prototype's quiet button: transparent at rest with a `SURFACE3` hover
/// fill, so a card of controls stays recessive behind the viewer while every
/// control keeps an ordinary hover cue. Callers paint their own icon on top.
fn quiet_button<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        let widgets = &mut ui.style_mut().visuals.widgets;
        widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        widgets.inactive.bg_stroke = egui::Stroke::NONE;
        add_contents(ui)
    })
    .inner
}

/// The current-look check, drawn rather than typeset so it cannot go missing
/// with a font.
fn paint_check(painter: &egui::Painter, rect: egui::Rect) {
    let center = rect.center();
    painter.line(
        vec![
            center + egui::vec2(-4.0, 0.0),
            center + egui::vec2(-1.0, 3.0),
            center + egui::vec2(4.5, -3.5),
        ],
        egui::Stroke::new(1.6_f32, theme::ACCENT),
    );
}

/// Stable card identity across frames and grid cells.
fn card_id(index: usize) -> egui::Id {
    egui::Id::new(("preset card", index))
}

/// Visual keyboard order: groups render one after the other, so arrows must
/// cross the Built-in/My Presets boundary in that same order.
pub(super) fn presentation_order(visible: &[usize], entries: &[LibraryEntry]) -> Vec<usize> {
    let mut ordering = Vec::with_capacity(visible.len());
    for group in [PresetGroup::BuiltIn, PresetGroup::User] {
        ordering.extend(
            visible
                .iter()
                .copied()
                .filter(|index| entries[*index].group == group),
        );
    }
    ordering
}

/// The entry-actions affordance, drawn rather than typeset.
fn paint_actions(painter: &egui::Painter, rect: egui::Rect) {
    let center = rect.center();
    for offset in [-4.0, 0.0, 4.0] {
        painter.circle_filled(center + egui::vec2(offset, 0.0), 1.5, theme::FG_MUTED);
    }
}

fn paint_status_owned(ui: &egui::Ui, rect: egui::Rect, text: &str) {
    ui.painter().with_clip_rect(rect).text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(theme::SIZE_HINT),
        theme::FG_MUTED,
    );
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
        LibraryErrorReason::ExpectedLook { path } => {
            crate::app::Message::new("surfaces.presets.expected_look").arg("path", path.display())
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
