//! Settings, state files, diagnostics and display profile surfaces.
use crate::*;

impl App {
    fn diagnostics_panel(&mut self, ui: &mut egui::Ui) {
        let active = *self.render_worker.active.lock();
        let render = active.map(|(_, kind, mode)| (kind.label(), mode));
        let queued = self
            .render_job
            .as_ref()
            .filter(|job| active.is_none_or(|(id, _, _)| id != job.request_id))
            .map(|job| (job.kind.label(), job.collection_mode));
        let export = self
            .export_job
            .as_ref()
            .map(|job| ("Export", job.operation.mode()));
        match self.diagnostics.show(ui, render, queued, export) {
            Some(diagnostics::Action::SaveSelected) => {
                if let Some(path) = self.chooser.save_file(
                    rfd::FileDialog::new()
                        .add_filter("JSON report", &["json"])
                        .set_file_name("spektrafilm-report.json"),
                ) {
                    self.status = match self.diagnostics.save_selected(&path) {
                        Ok(()) => crate::app::Message::new("surfaces.status.report_saved"),
                        Err(error) => crate::app::Message::new("surfaces.status.report_error")
                            .arg("error", error),
                    };
                }
            }
            Some(diagnostics::Action::OpenDirectory) => {
                if let Some(directory) = self.diagnostics.publication_directory() {
                    // Open the captured directory in the platform file manager
                    // as one argument, without waiting on the manager's lifetime.
                    let mut command = if cfg!(target_os = "macos") {
                        std::process::Command::new("open")
                    } else if cfg!(windows) {
                        std::process::Command::new("explorer")
                    } else {
                        std::process::Command::new("xdg-open")
                    };
                    command.arg(directory);
                    match command.spawn() {
                        Ok(mut child) => {
                            // Dropping Child does not reap it on Unix.
                            std::thread::spawn(move || {
                                let _ = child.wait();
                            });
                        }
                        Err(error) => {
                            let detail = format!("{}: {}", directory.display(), error);
                            self.diagnostics.record_publication_error(detail);
                        }
                    }
                }
            }
            None => {}
        }
    }

    fn state_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let save = ui.add(egui::Button::new(
                egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.state.save_file"))
                    .color(theme::FG_SECONDARY),
            ));
            controls::locate_row(ui, "action.save_state", &save);
            if save.clicked() {
                self.save_state_with_dialog();
            }
            let load = ui.add(egui::Button::new(
                egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.state.load_file"))
                    .color(theme::FG_SECONDARY),
            ));
            controls::locate_row(ui, "action.load_state", &load);
            if load.clicked() {
                self.load_state_with_dialog();
            }
            let startup = ui
                .add(egui::Button::new(
                    egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.state.save_default"))
                        .color(theme::FG_SECONDARY),
                ))
                .on_hover_text(crate::i18n::text(
                    ui.ctx(),
                    "surfaces.state.save_default_help",
                ));
            controls::locate_row(ui, "action.save_startup", &startup);
            if startup.clicked() {
                let result = self
                    .current_state()
                    .and_then(|s| s.save(&state::default_path(&self.config_directory)));
                self.status = match result {
                    Ok(()) => crate::app::Message::new("surfaces.status.startup_saved"),
                    Err(e) => crate::app::Message::new("surfaces.status.startup_error")
                        .message_arg("error", crate::app::Message::error(&e)),
                };
            }
            let restore = ui
                .add(egui::Button::new(
                    egui::RichText::new(crate::i18n::text(
                        ui.ctx(),
                        "surfaces.state.restore_factory",
                    ))
                    .color(theme::FG_SECONDARY),
                ))
                .on_hover_text(crate::i18n::text(
                    ui.ctx(),
                    "surfaces.state.restore_factory_help",
                ));
            controls::locate_row(ui, "action.restore_factory", &restore);
            if restore.clicked() {
                self.factory_reset_confirm = true;
            }
        });
    }

    /// Confirmation for the destructive restore-factory-default action.
    ///
    /// The action replaces every current control and removes the saved startup
    /// default, so it states both consequences before it applies. The modal is
    /// transient state on `App`: nothing is persisted, and the header status
    /// remains the one surface that reports the outcome.
    pub(super) fn factory_reset_confirmation(&mut self, ctx: &egui::Context) {
        if !self.factory_reset_confirm {
            return;
        }
        let mut confirmed = false;
        let mut canceled = false;
        let modal = egui::Modal::new(egui::Id::new("Restore factory default?")).show(ctx, |ui| {
            ui.set_max_width(theme::WIDTH_CONTROL_PANEL);
            ui.heading(crate::i18n::text(ctx, "surfaces.state.restore_title"));
            ui.label(crate::i18n::text(
                ui.ctx(),
                "surfaces.state.restore_warning",
            ));
            ui.horizontal(|ui| {
                confirmed = ui
                    .button(crate::i18n::text(ui.ctx(), "surfaces.action.restore"))
                    .clicked();
                canceled = ui
                    .button(crate::i18n::text(ui.ctx(), "surfaces.action.cancel"))
                    .clicked();
            });
        });
        if confirmed {
            self.factory_reset_confirm = false;
            let result = state::reset_factory(&self.config_directory).and_then(|state| {
                self.apply_state_with_source(state, |app| {
                    app.status = crate::app::Message::new("surfaces.status.factory_restored");
                })
            });
            if let Err(error) = result {
                self.status = crate::app::Message::new("surfaces.status.factory_error")
                    .message_arg("error", crate::app::Message::error(&error));
            }
        } else if canceled
            || (modal.is_top_modal
                && !modal.any_popup_open
                && ctx
                    .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)))
        {
            self.factory_reset_confirm = false;
        }
    }

    fn save_state_with_dialog(&mut self) {
        if let Some(path) = self.chooser.save_file(
            self.file_dialog("state")
                .set_file_name("gui_state.json")
                .add_filter("JSON", &["json"]),
        ) {
            self.remember_dialog("state", &path);
            let result = self.current_state().and_then(|s| s.save(&path));
            self.status = match result {
                Ok(()) => crate::app::Message::new("surfaces.status.state_saved")
                    .arg("path", path.display()),
                Err(e) => crate::app::Message::new("surfaces.status.state_save_error")
                    .message_arg("error", crate::app::Message::error(&e)),
            };
        }
    }

    fn load_state_with_dialog(&mut self) {
        if let Some(path) = self
            .chooser
            .pick_file(self.file_dialog("state").add_filter("JSON", &["json"]))
        {
            self.request_photo_action(crate::photograph::PhotoAction::LoadState(path));
        }
    }

    fn display_profile_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let select = ui
                .button(crate::i18n::text(
                    ui.ctx(),
                    "surfaces.display.select_profile",
                ))
                .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.display.select_help"));
            controls::locate_row(ui, "extras.rust.display_profile", &select);
            if select.clicked()
                && let Some(path) = self.chooser.pick_file(
                    self.file_dialog("display")
                        .add_filter("ICC profile", &["icc", "icm"]),
                )
            {
                self.gui_state.set_display_profile(path.to_str());
                self.refresh_viewing_artifacts();
                self.status = crate::app::Message::new("surfaces.status.display_profile")
                    .arg("path", path.display());
            }
            let clear = ui
                .add_enabled(
                    self.gui_state.display_profile().is_some(),
                    egui::Button::new(crate::i18n::text(ui.ctx(), "surfaces.action.clear")),
                )
                .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.display.clear_help"));
            controls::locate_row(ui, "action.clear_display_profile", &clear);
            if clear.clicked() {
                self.gui_state.set_display_profile(None);
                self.refresh_viewing_artifacts();
                self.status = crate::app::Message::new("surfaces.status.display_cleared");
            }
        });
        if let Some(profile) = self.gui_state.display_profile() {
            ui.label(crate::tr!(
                ui.ctx(),
                "surfaces.display.profile",
                profile = profile
            ));
        } else {
            ui.label(crate::i18n::text(ui.ctx(), "surfaces.display.fallback"));
        }
        ui.label(self.viewer.transform_status.localized(ui.ctx()));
        // The display transform and its profile are viewer-only: they change
        // what the viewer shows and nothing the pipeline retains or writes.
        ui.label(
            egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.display.isolation"))
                .color(theme::FG_MUTED)
                .small(),
        );
    }

    pub(super) fn settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            ctx.data_mut(|data| data.remove_temp::<bool>(egui::Id::new("settings-modal-open")));
            return;
        }
        let mut close = false;
        let first_frame = !ctx.data_mut(|data| {
            let previous = data
                .get_temp::<bool>(egui::Id::new("settings-modal-open"))
                .unwrap_or(false);
            data.insert_temp(egui::Id::new("settings-modal-open"), true);
            previous
        });
        let locating = ctx.data(|data| {
            data.get_temp::<(String, String)>(egui::Id::new("control_focus_request"))
                .is_some()
        });
        let display_before = (
            self.viewer.settings.use_display_transform,
            self.params.settings.preview_max_size,
        );
        let modal = egui::Modal::new(egui::Id::new("application-settings"))
            .frame(theme::dialog_frame())
            .show(ctx, |ui| {
            let screen = ctx.screen_rect();
            let border = 2.0 * theme::STROKE_DEFAULT;
            let width = ((screen.width() - 2.0 * theme::SPACE_DIALOG).min(520.0) - border).max(1.0);
            let max_height = (screen.height() * 0.9)
                .min(screen.height() - 2.0 * theme::SPACE_DIALOG) - border;
            ui.set_min_width(width);
            ui.set_max_width(width);
            ui.set_max_height(max_height);
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::Frame::NONE
                .inner_margin(egui::Margin::symmetric(theme::SPACE_DIALOG as i8,
                    ((theme::HEIGHT_DIALOG_HEADER - theme::HEIGHT_CONTROL) / 2.0) as i8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_min_height(theme::HEIGHT_CONTROL);
                        ui.label(egui::RichText::new(crate::i18n::text(ctx, "surfaces.settings.title"))
                            .size(theme::SIZE_TITLE).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let label = crate::i18n::text(ctx, "surfaces.settings.close");
                            let response = controls::icon_button(ui, label.as_ref(), controls::Icon::Close);
                            if first_frame && !locating { response.request_focus(); }
                            close = response.clicked();
                        });
                    });
                });
            ui.separator();
            let body_height = (max_height - theme::HEIGHT_DIALOG_HEADER - theme::HEIGHT_DIALOG_FOOTER
                - 2.0 * theme::SPACE_DIALOG - 2.0 * theme::STROKE_DEFAULT).max(1.0);
            let scroll = egui::Frame::NONE.inner_margin(theme::SPACE_DIALOG as i8)
                .show(ui, |ui| egui::ScrollArea::vertical()
                .id_salt("settings-scroll")
                .max_height(body_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(theme::SPACE_2, theme::SPACE_2);
                    // Search opens this owner without filtering its contents. A
                    // canonical target scrolls its actual response, not the header.
                    for section in ["Session", "State and startup", "Display", "Diagnostics"] {
                        let reveal = self.pending_scroll.as_deref() == Some(section);
                        let canonical_target = ctx.data(|data| {
                            data.get_temp::<(String, String)>(egui::Id::new("control_focus_request"))
                                .is_some_and(|(owner, _)| owner == section)
                        });
                        let label = if section == "Session" {
                            crate::i18n::text(ctx, "surfaces.settings.general")
                        } else {
                            controls::localized(ctx, section)
                        };
                        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            ui.make_persistent_id(section),
                            matches!(section, "Session" | "Display"),
                        );
                        if reveal {
                            state.set_open(true);
                        }
                        // The same module-header presentation as the inspector:
                        // title on the left, modified chip and reset on the
                        // right. Settings body rows never lead with a reset.
                        let mut restore_display = false;
                        let header = ui.horizontal(|ui| {
                            let response =
                                controls::disclosure_header(ui, label.as_ref(), state.is_open());
                            if response.clicked() {
                                state.toggle(ui);
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if matches!(section, "Session" | "Display") {
                                        let changed = if section == "Session" {
                                            ui.add_enabled_ui(
                                                !self.input_transition_pending(),
                                                |ui| {
                                                    self.section_reset_controls_with_order(
                                                        ui, section, true,
                                                    )
                                                },
                                            )
                                            .inner
                                        } else {
                                            self.section_reset_controls_with_order(
                                                ui, section, true,
                                            )
                                        };
                                        if changed && section == "Display" {
                                            restore_display = true;
                                        }
                                    }
                                },
                            );
                            response
                        });
                        if restore_display {
                            let restored = display::DisplaySettings::from_json(
                                &self.gui_state.sections["display"],
                            );
                            self.viewer.settings.use_display_transform =
                                restored.use_display_transform;
                            self.viewer.settings.gray_18_canvas = restored.gray_18_canvas;
                            self.viewer.settings.white_padding = restored.white_padding;
                            self.viewer.settings.interpolation = restored.interpolation;
                        }
                        state.show_body_indented(&header.response, ui, |ui| {
                            let _scope = controls::row_modified::install(
                                ctx, section, &self.params, &self.gui_state.sections,
                            );
                            if self.input_transition_pending()
                                && !matches!(section, "Display" | "Diagnostics")
                            {
                                ui.disable();
                            }
                            match section {
                                "Session" => {
                                    ui.horizontal(|ui| {
                                        ui.label("语言 / Language");
                                        self.language_selector(ui);
                                    });
                                    self.settings_parameter_controls(ui, "Session");
                                }
                                "State and startup" => self.state_toolbar(ui),
                                "Display" => {
                                    self.display_profile_controls(ui);
                                    let transform = controls::toggle_row(ui,
                                        "use display transform",
                                        &mut self.viewer.settings.use_display_transform,
                                        "Viewer-only display conversion; does not change retained or written pixels.",
                                    );
                                    controls::locate_row(ui, "extras.display.use_display_transform", &transform);
                                    let canvas = controls::toggle_row(ui, "gray 18% canvas",
                                        &mut self.viewer.settings.gray_18_canvas, "gray 18% canvas");
                                    controls::locate_row(ui, "extras.display.gray_18_canvas", &canvas);
                                    controls::param_numeric(ui, "white padding",
                                        &mut self.viewer.settings.white_padding,
                                        "extras.display.white_padding",
                                        "Viewer-only white border, as a fraction of the image long edge.");
                                    self.viewer.interpolation_control(ui);
                                    self.sync_display_controls();
                                }
                                "Diagnostics" => self.diagnostics_panel(ui),
                                _ => unreachable!(),
                            }
                        });
                        if reveal {
                            if !canonical_target {
                                header.response.request_focus();
                                header.response.scroll_to_me(Some(egui::Align::TOP));
                            }
                            if !canonical_target || ctx.data(|data| {
                                data.get_temp::<(String, String)>(egui::Id::new("control_focus_request"))
                                    .is_none()
                            }) {
                                self.pending_scroll = None;
                            }
                        }
                        ui.add_space(theme::SPACE_MODULE);
                    }
                })).inner;
            let viewport = ui.interact(scroll.inner_rect, ui.id().with("settings-viewport"), egui::Sense::hover());
            let content_label = crate::i18n::text(ctx, "surfaces.settings.content");
            viewport.widget_info(|| egui::WidgetInfo::labeled(
                egui::WidgetType::Other, true, content_label.as_ref()));
            ctx.accesskit_node_builder(viewport.id, |node| {
                node.set_role(egui::accesskit::Role::ScrollView);
                node.set_label(content_label.as_ref());
            });
            ui.separator();
            egui::Frame::NONE.inner_margin(theme::SPACE_DIALOG as i8).show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), theme::HEIGHT_CONTROL),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        let label = crate::i18n::text(ctx, "surfaces.action.close");
                        close |= ui.button(label).clicked();
                    },
                );
            });
        });
        let title = crate::i18n::text(ctx, "surfaces.settings.title");
        modal.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Other, true, title.as_ref())
        });
        ctx.accesskit_node_builder(modal.response.id, |node| {
            node.set_role(egui::accesskit::Role::Dialog);
            node.set_label(title.as_ref());
        });
        let escape = modal.is_top_modal
            && !modal.any_popup_open
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if close || escape {
            self.settings_open = false;
        }
        if display_before
            != (
                self.viewer.settings.use_display_transform,
                self.params.settings.preview_max_size,
            )
        {
            self.refresh_viewing_artifacts();
        }
    }

    fn settings_parameter_controls(&mut self, ui: &mut egui::Ui, section: &str) {
        let scanned_medium_positive = self.scanned_medium_positive();
        let cache = self.section_labels.entry(section.to_owned()).or_default();
        let changes = controls::show(
            ui,
            &mut self.params,
            &mut self.gui_state.sections,
            section,
            self.film_medium,
            scanned_medium_positive,
            "",
            cache,
            &self.film_reference_illuminant,
            &self.data_dir,
        );
        if changes.runtime_changed {
            self.dirty = true;
            self.dirty_since = Some(Instant::now());
        }
    }
}
