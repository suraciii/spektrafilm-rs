use super::*;
use std::sync::atomic::Ordering;

struct InspectorMaterials {
    film_name: String,
    print_name: String,
    film: Arc<profile::Profile>,
    print: Option<Arc<profile::Profile>>,
}

pub(crate) fn grain_control_inactive_reason(
    ctx: &egui::Context,
    path: &str,
    params: &RuntimeParams,
    component: Option<usize>,
) -> Option<&'static str> {
    let materials = ctx.data(|data| {
        data.get_temp::<Arc<InspectorMaterials>>(egui::Id::new("inspector-material-context"))
    });
    match materials {
        Some(materials) => {
            spektrafilm_core::params::sources::grain_control_inactive_reason_with_profile(
                path,
                params,
                &materials.film,
                component,
            )
        }
        None => spektrafilm_core::params::sources::grain_control_inactive_reason(
            path, params, component,
        ),
    }
}

pub(crate) fn control_inactive_reason(
    ctx: &egui::Context,
    path: &str,
    params: &RuntimeParams,
) -> Option<String> {
    let materials = ctx.data(|data| {
        data.get_temp::<Arc<InspectorMaterials>>(egui::Id::new("inspector-material-context"))
    });
    match materials {
        Some(materials) => spektrafilm_core::params::sources::inactive_reason_with_materials(
            path,
            params,
            Some(&materials.film),
            materials.print.as_deref(),
        ),
        None => spektrafilm_core::params::sources::inactive_reason_without_profiles(path, params)
            .map(str::to_owned),
    }
}

fn section_summary(section: &str) -> Option<&'static str> {
    match section {
        "Film base" => Some("Base density: scale, tilt, and per-channel masks"),
        "Halation" => {
            Some("Highlight scatter and back-reflection; source response and RGB controls")
        }
        "Couplers" => Some("DIR couplers: inhibition and interlayer response"),
        "Convert" => {
            Some("Real negative capture: exposure, base estimation, calibration, channel swaps")
        }
        "Camera diffusion" => Some("Taking-lens diffusion: core, tail, and warmth"),
        "Print base" => Some("Base density: scale and per-channel masks"),
        "Glare" => Some("Readout stray light: percent, roughness, blur"),
        "Enlarger diffusion" => Some("Enlarger optics: core, tail, and warmth"),
        "Preflash" => Some("Linear flash exposure and independent Yellow and Magenta shifts"),
        "Film chemistry" | "Print chemistry" => {
            Some("Density curves: development time, gamma, developer exhaustion")
        }
        "Input gamut compress" => Some("Input gamut compression: algorithm, knee, hull detail"),
        "Output gamut compress" => Some("Output gamut compression: algorithm, knee"),
        _ => None,
    }
}
impl App {
    fn refresh_viewing_artifacts(&mut self) {
        self.viewer.set_input_display_source(
            &self.params.io.input_color_space,
            self.params.io.input_cctf_decoding,
        );
        let (space, encoded) = self.displayed_comparison.as_ref().map_or(
            (self.output_color_space.as_str(), self.output_cctf_encoding),
            |comparison| (comparison.color_space.as_str(), comparison.encoded),
        );
        self.viewer.set_output_display_source(
            space,
            encoded,
            self.viewer.settings.use_display_transform,
            self.gui_state.display_profile().map(Path::new),
        );
        self.viewer.invalidate_display();
    }

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
    fn factory_reset_confirmation(&mut self, ctx: &egui::Context) {
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

    pub(super) fn toggle_scan_for_print(&mut self) {
        if let Some(glare) = self.scan_for_print_snapshot.take() {
            self.params.print_render.glare.active = glare;
        } else {
            self.scan_for_print_snapshot = Some(self.params.print_render.glare.active);
            self.params.print_render.glare.active = false;
        }
        self.mark_parameters_changed();
    }

    fn simulation_action_bar(&mut self, ui: &mut egui::Ui) {
        let busy = self.render_job.is_some()
            || self.export_job.is_some()
            || self.calibration_job.is_some();
        let width = (ui.available_width() - ui.spacing().item_spacing.x) * 0.5;
        let action = |ui: &mut egui::Ui, enabled: bool, label_key, caption_key, icon, primary| {
            let label = crate::i18n::text(ui.ctx(), label_key);
            let mut text = egui::text::LayoutJob::default();
            text.append(
                &label,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(theme::SIZE_LABEL),
                    color: if primary { theme::ACCENT } else { theme::FG },
                    ..Default::default()
                },
            );
            text.append("\n", 0.0, egui::TextFormat::default());
            text.append(
                &crate::i18n::text(ui.ctx(), caption_key),
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(theme::SIZE_HINT),
                    color: theme::FG_SECONDARY,
                    ..Default::default()
                },
            );
            text.wrap.max_width = (width - 44.0).max(1.0);
            let galley = ui.fonts(|fonts| fonts.layout_job(text));
            let button = egui::Button::new("")
                .fill(if primary {
                    theme::SURFACE2
                } else {
                    theme::SURFACE3
                })
                .stroke(theme::stroke(if primary {
                    theme::ACCENT_DIM
                } else {
                    theme::BORDER
                }))
                .min_size(egui::vec2(width, 64.0));
            let response = ui.add_enabled(enabled, button);
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(response.rect.left() + 20.0, response.rect.center().y),
                egui::vec2(17.0, 17.0),
            );
            controls::paint_icon(
                ui.painter(),
                icon_rect,
                icon,
                if primary {
                    theme::ACCENT
                } else {
                    theme::FG_SECONDARY
                },
            );
            ui.painter().galley(
                egui::pos2(
                    response.rect.left() + 36.0,
                    response.rect.center().y - galley.size().y * 0.5,
                ),
                galley,
                theme::FG,
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label.as_ref())
            });
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_label(label.as_ref()));
            controls::focus_feedback(ui, &response);
            response
        };
        ui.horizontal(|ui| {
            let scan = action(
                ui,
                self.image.is_some() && !busy,
                "surfaces.action.scan",
                "surfaces.action.scan_caption",
                controls::Icon::Scan,
                true,
            );
            if scan
                .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.action.scan_help"))
                .clicked()
            {
                self.dirty = true;
                self.full_scan_requested = true;
            }
            let save = action(
                ui,
                (self.output_image.is_some() || self.output_retained.is_some())
                    && !busy
                    && self.pending_saves.is_empty(),
                "surfaces.action.save_retained",
                "surfaces.action.save_caption",
                controls::Icon::Save,
                false,
            );
            controls::locate_row(ui, "action.save_retained", &save);
            if save
                .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.action.save_help"))
                .clicked()
            {
                self.save_dialog();
            }
        });
    }

    pub(super) fn file_dialog(&self, key: &str) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new();
        match self.gui_state.dialog_dirs()[key].as_str() {
            Some(path) => dialog.set_directory(path),
            None => dialog,
        }
    }
    pub(super) fn remember_dialog(&mut self, key: &str, path: &Path) {
        if let Some(parent) = path.parent() {
            if !self.gui_state.sections["rust"].is_object() {
                self.gui_state.sections["rust"] =
                    serde_json::json!({"version":state::CURRENT_STATE_VERSION,"dialog_dirs":{}});
            }
            if !self.gui_state.sections["rust"]["dialog_dirs"].is_object() {
                self.gui_state.sections["rust"]["dialog_dirs"] = serde_json::json!({});
            }
            self.gui_state.sections["rust"]["dialog_dirs"][key] =
                serde_json::json!(parent.to_string_lossy());
            let directory = self.browser_state_path.parent().unwrap_or(Path::new("."));
            let path = directory.join("dialog_dirs.json");
            if let Err(e) = std::fs::create_dir_all(directory).and_then(|_| {
                std::fs::write(
                    path,
                    self.gui_state.sections["rust"]["dialog_dirs"].to_string(),
                )
            }) {
                self.status =
                    crate::app::Message::new("surfaces.status.directory_error").arg("error", e);
            }
        }
    }

    pub(super) fn finish_route_change(&mut self) {
        if let Some(glare) = self.scan_for_print_snapshot.take() {
            self.params.print_render.glare.active = glare;
        }
        let before = self.params.scanner.clone();
        self.params.io.scan_film = self.params.workflow.route == "input > film > scan";
        normalize_scanner_selections(&mut self.params, self.film_medium);
        self.status = Self::normalization_status(
            &before,
            &self.params.scanner,
            self.film_medium,
            &self.params.workflow.route,
            crate::app::Message::new("surfaces.status.route_changed"),
        );
        self.invalidate_look_render();
    }

    pub(crate) fn scanner_normalization_detail(
        before: &spektrafilm_core::params::ScannerParams,
        after: &spektrafilm_core::params::ScannerParams,
        film_medium: controls::ScanMedium,
        route: &str,
    ) -> Option<crate::app::Message> {
        let normalized = before.scan_output != after.scan_output;
        let inactive = film_medium.for_route(route) == controls::ScanMedium::None;
        let key = match (normalized, inactive) {
            (false, false) => return None,
            (true, false) => "surfaces.status.normalized",
            (false, true) => "surfaces.status.scanner_inactive",
            (true, true) => "surfaces.status.normalized_inactive",
        };
        Some(crate::app::Message::new(key))
    }

    pub(crate) fn normalization_status(
        before: &spektrafilm_core::params::ScannerParams,
        after: &spektrafilm_core::params::ScannerParams,
        film_medium: controls::ScanMedium,
        route: &str,
        cause: crate::app::Message,
    ) -> crate::app::Message {
        match Self::scanner_normalization_detail(before, after, film_medium, route) {
            Some(detail) => crate::app::Message::new("surfaces.status.normalization")
                .message_arg("cause", cause)
                .message_arg("detail", detail),
            None => cause,
        }
    }

    pub(super) fn invalidate_look_render(&mut self) {
        self.scan_for_print_snapshot = None;
        self.mark_parameters_changed_invalidating_calibration();
    }
    pub(crate) fn current_material_baseline(&mut self) -> Option<Arc<serde_json::Value>> {
        if let Some((data_dir, film, baseline)) = &self.material_baseline
            && data_dir == &self.data_dir
            && film == &self.film_name
        {
            return baseline.clone();
        }
        let baseline = match profile::load_profile_by_name(&self.data_dir, &self.film_name) {
            Ok(film) if film.is_film() && film.is_filming() => {
                Some(Arc::new(section_reset::material_baseline(&film)))
            }
            Ok(_) => {
                self.status = crate::app::Message::new("surfaces.status.profile_not_film")
                    .arg("name", &self.film_name);
                None
            }
            Err(error) => {
                self.status = crate::app::Message::new("surfaces.presets.profile_load")
                    .message_arg("kind", crate::app::Message::control("film"))
                    .arg("stock", &self.film_name)
                    .message_arg("detail", crate::app::Message::profile(&error));
                None
            }
        };
        self.material_baseline = Some((
            self.data_dir.clone(),
            self.film_name.clone(),
            baseline.clone(),
        ));
        baseline
    }

    pub(super) fn select_film_profile(&mut self, selected: String) {
        let film = match profile::load_profile_by_name(&self.data_dir, &selected) {
            Ok(profile) if profile.is_film() && profile.is_filming() => profile,
            Ok(_) => {
                self.status = crate::app::Message::new("surfaces.status.profile_not_film")
                    .arg("name", selected);
                return;
            }
            Err(error) => {
                self.status = crate::app::Message::new("surfaces.presets.profile_load")
                    .message_arg("kind", crate::app::Message::control("film"))
                    .arg("stock", &selected)
                    .message_arg("detail", crate::app::Message::profile(&error));
                return;
            }
        };
        spektrafilm_core::params_builder::apply_film_specifics(&mut self.params, &film);
        self.material_baseline = Some((
            self.data_dir.clone(),
            selected.clone(),
            Some(Arc::new(section_reset::material_baseline(&film))),
        ));
        self.film_name = selected;
        self.params.film_render.grain.monochrome = film.is_bw();
        self.params.film_render.development_time = None;
        self.gui_state.sections["simulation"]["film_stock"] = serde_json::json!(self.film_name);
        self.film_reference_illuminant = film.info.reference_illuminant.clone();
        self.film_dev_times = film.data.development_time.clone();
        self.film_medium = scan_medium(&film);
        let before = self.params.scanner.clone();
        self.params.workflow.route = workflow_default(&film);
        self.params.io.scan_film = film.is_positive();
        self.params.scanner.scan_output = "direct_scan".into();
        normalize_scanner_selections(&mut self.params, self.film_medium);
        self.status = Self::normalization_status(
            &before,
            &self.params.scanner,
            self.film_medium,
            &self.params.workflow.route,
            crate::app::Message::new("surfaces.status.film_changed"),
        );
        self.invalidate_look_render();
    }

    pub(super) fn select_print_profile(&mut self, selected: String) {
        let print = match profile::load_profile_by_name(&self.data_dir, &selected) {
            Ok(profile) if profile.is_print_stock() => profile,
            Ok(_) => {
                self.status = crate::app::Message::new("surfaces.status.profile_not_print")
                    .arg("name", selected);
                return;
            }
            Err(error) => {
                self.status = crate::app::Message::new("surfaces.presets.profile_load")
                    .message_arg("kind", crate::app::Message::control("print"))
                    .arg("stock", &selected)
                    .message_arg("detail", crate::app::Message::profile(&error));
                return;
            }
        };
        self.print_name = selected;
        self.params.print_render.development_time = None;
        self.gui_state.sections["simulation"]["print_paper"] = serde_json::json!(self.print_name);
        self.print_dev_times = print.data.development_time.clone();
        self.invalidate_look_render();
    }

    pub(super) fn load_image_from_path(&mut self, path: &Path) {
        if self.input_command_job.is_some() {
            self.status = crate::app::Message::new("surfaces.status.preparing_input");
            return;
        }
        let path = path.to_path_buf();
        let raw = image_io::is_raw(&path);
        let reprocess = self.image_path.as_deref() == Some(path.as_path());
        let previous_baseline = self.photograph.baseline.clone();
        let previous_record = self.photograph.record_path.clone();
        let raw_settings = self.gui_state.sections["load_raw"].clone();
        let spec = crate::input_worker::InputSpec {
            source: crate::input_worker::InputSource::File {
                path: path.clone(),
                raw: raw_settings.clone(),
            },
            rotation: 0,
            space: if raw {
                "ACES2065-1".into()
            } else {
                self.params.io.input_color_space.clone()
            },
            decode: !raw && self.params.io.input_cctf_decoding,
            max_size: Some(self.params.settings.preview_max_size),
        };
        self.queue_input(
            spec,
            crate::input_worker::InputFailure::Load,
            move |app, prepared| {
                let view = prepared.view.expect("source request prepared its view");
                if raw {
                    app.params.io.input_color_space = "ACES2065-1".into();
                    app.params.io.input_cctf_decoding = false;
                }
                app.source_metadata = prepared.metadata;
                app.input_rotation = 0;
                app.raw_lens_info = prepared.lens_info;
                app.photograph.lens_model_identity = prepared.lens_model_identity;
                let mut loaded = crate::app::Message::new(if raw {
                    "surfaces.status.loaded_raw"
                } else {
                    "surfaces.status.loaded"
                })
                .arg("width", prepared.source.width())
                .arg("height", prepared.source.height())
                .arg(
                    "mp",
                    format!("{:.1}", prepared.source.pixel_count() as f64 / 1e6),
                )
                .arg(
                    "ms",
                    format!("{:.0}", prepared.elapsed.as_secs_f64() * 1000.0),
                );
                app.raw_applied_settings = raw.then_some(raw_settings);
                if raw {
                    let lens = match app.raw_lens_info.as_deref() {
                        Some(info) if !info.is_empty() => {
                            crate::app::Message::new("surfaces.status.lens_applied")
                                .arg("info", info)
                        }
                        _ => crate::app::Message::new("surfaces.status.lens_not_applied"),
                    };
                    loaded = loaded.message_arg("lens", lens);
                }
                app.status = loaded;
                app.calibration_epoch = app.calibration_epoch.wrapping_add(1);
                app.input_epoch = app.input_epoch.wrapping_add(1);
                app.set_operator_compare(false);
                app.set_finishing_compare(false);
                app.render_worker.cancel_active();
                app.full_scan_requested = false;
                app.image = Some(prepared.source);
                app.publish_input_viewer(view.oriented, view.raster);
                app.image_path = Some(path);
                app.output_image = None;
                app.output_retained = None;
                app.output_geometry = None;
                app.analysis.selection = None;
                app.analysis.clear();
                app.output_metadata = None;
                app.source_render = None;
                app.retained_kind = None;
                app.stale_save_confirmation = false;
                app.dirty = true;
                app.reset_adjustment_history();
                if let Some(ctx) = &app.photograph.context {
                    crate::numeric::NumericDrafts::take(ctx);
                }
                app.photograph.baseline = app.capture_photograph_settings().ok();
                app.photograph.record_path = None;
                app.photograph.source_digest = prepared.digest;
                if reprocess {
                    app.photograph.baseline = previous_baseline;
                    app.photograph.record_path = previous_record;
                }
                Ok(())
            },
        );
    }

    /// Commit a deferred source rotation only after its device view is ready.
    fn rotate_input_image(&mut self, clockwise: bool) {
        let Some(source) = self.image.as_ref().cloned() else {
            self.status = crate::app::Message::new("surfaces.status.rotate_requires_image");
            return;
        };
        if self.input_transition_pending() {
            return;
        }
        let rotation = (self.input_rotation + if clockwise { -1 } else { 1 }).rem_euclid(4);
        let epoch = self.input_epoch;
        let spec = crate::input_worker::InputSpec {
            source: crate::input_worker::InputSource::Existing(source),
            rotation,
            space: self.params.io.input_color_space.clone(),
            decode: self.params.io.input_cctf_decoding,
            max_size: Some(self.params.settings.preview_max_size),
        };
        self.queue_input(
            spec,
            crate::input_worker::InputFailure::Viewer,
            move |app, prepared| {
                if app.input_epoch != epoch {
                    return Ok(());
                }
                let view = prepared.view.expect("rotation request prepared its view");
                app.input_rotation = rotation;
                app.input_epoch = app.input_epoch.wrapping_add(1);
                app.set_operator_compare(false);
                app.set_finishing_compare(false);
                app.render_worker.cancel_active();
                app.calibration_epoch = app.calibration_epoch.wrapping_add(1);
                app.full_scan_requested = false;
                app.output_image = None;
                app.output_retained = None;
                app.output_geometry = None;
                app.output_metadata = None;
                app.source_render = None;
                app.publish_input_viewer(view.oriented, view.raster);
                app.dirty = true;
                app.status = crate::app::Message::new("surfaces.status.rotated")
                    .arg("degrees", rotation * 90);
                Ok(())
            },
        );
    }

    /// Refresh presentation on the input worker without rewriting accepted samples.
    pub(crate) fn refresh_input_viewer(&mut self) {
        let Some(source) = self.image.as_ref().cloned() else {
            return;
        };
        let epoch = self.input_epoch;
        let rotation = self.input_rotation;
        let space = self.params.io.input_color_space.clone();
        let decode = self.params.io.input_cctf_decoding;
        let spec = crate::input_worker::InputSpec {
            source: crate::input_worker::InputSource::Existing(source),
            rotation,
            space: space.clone(),
            decode,
            max_size: Some(self.params.settings.preview_max_size),
        };
        self.queue_input_view(spec, move |app, prepared| {
            if app.input_epoch != epoch
                || app.input_rotation != rotation
                || app.params.io.input_color_space != space
                || app.params.io.input_cctf_decoding != decode
            {
                return Ok(());
            }
            let view = prepared.view.expect("display refresh prepared its view");
            if app.output_image.is_some() || app.output_retained.is_some() {
                app.viewer.refresh_input(
                    view.raster,
                    [
                        view.oriented.width() as usize,
                        view.oriented.height() as usize,
                    ],
                );
                app.seed_input_viewer(view.oriented);
            } else {
                app.publish_input_viewer(view.oriented, view.raster);
            }
            Ok(())
        });
    }

    pub(crate) fn publish_input_viewer(
        &mut self,
        image: crate::render::InputImage,
        raster: display::DisplayRaster,
    ) {
        self.viewer
            .set_input(raster, [image.width() as usize, image.height() as usize]);
        self.seed_input_viewer(image);
    }

    fn seed_input_viewer(&mut self, image: crate::render::InputImage) {
        let (space, encoded) = match &image {
            crate::render::InputImage::Host(_) => (
                self.params.io.input_color_space.as_str(),
                self.params.io.input_cctf_decoding,
            ),
            crate::render::InputImage::Device(source) => (
                source.space().name,
                source.domain() == spektrafilm_backend::SampleDomain::Encoded,
            ),
        };
        self.analysis.original_space = space.to_owned();
        self.analysis.original_encoded = encoded;
        self.analysis.original_rotation = self.input_rotation.rem_euclid(4) as u32;
        self.analysis.original_generation = self.analysis.original_generation.wrapping_add(1);
        self.viewer.cancel_analysis_selection();
        self.viewer.set_input_display_source(space, encoded);
        match &image {
            crate::render::InputImage::Host(source) => self
                .viewer
                .seed_prepared_input(source.clone(), self.params.settings.preview_max_size),
            crate::render::InputImage::Device(source) => self
                .viewer
                .seed_prepared_device_input(source.clone(), self.params.settings.preview_max_size),
        }
        self.oriented_input = Some(image);
    }

    fn rotate_input_image_clockwise(&mut self) {
        self.rotate_input_image(true);
    }

    fn rotate_input_image_counterclockwise(&mut self) {
        self.rotate_input_image(false);
    }
    fn parameter_section(&mut self, ui: &mut egui::Ui, section: &str) {
        if !controls::section_matches(
            section,
            &self.filter,
            &self.params,
            &self.gui_state.sections,
            self.only_modified,
        ) {
            return;
        }
        ui.push_id(("module", section), |ui| {
            let reveal = self.pending_scroll.as_deref() == Some(section);
            let disclosure_id = ui.make_persistent_id("open");
            let default_open = matches!(
                section,
                "Workflow" | "Camera" | "Film material" | "Print material" | "Output"
            );
            let normal_open = ui.ctx().data_mut(|data| {
                *data.get_persisted_mut_or_insert_with(disclosure_id, || default_open)
            });
            let temporary_open = !self.filter.is_empty() || self.only_modified;
            let open = normal_open || temporary_open || reveal;
            let frame = egui::Frame::NONE
                .fill(if open {
                    theme::SURFACE2
                } else {
                    egui::Color32::TRANSPARENT
                })
                .stroke(if open {
                    theme::stroke(theme::BORDER)
                } else {
                    egui::Stroke::NONE
                })
                .corner_radius(theme::RADIUS_CARD)
                .inner_margin(egui::Margin::symmetric(
                    theme::SPACE_MODULE as i8,
                    if open {
                        theme::SPACE_MODULE as i8
                    } else {
                        theme::SPACE_1 as i8
                    },
                ));
            frame.show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = theme::SPACE_2;
                let mut heading = None;
                ui.horizontal(|ui| {
                    let title = if section == "Glare" {
                        "Print viewing glare"
                    } else {
                        section
                    };
                    let mut response = controls::disclosure_header(ui, title, open);
                    if let Some(summary) = section_summary(section) {
                        response = response.on_hover_text(controls::localized(ui.ctx(), summary));
                    }
                    if response.clicked() && !temporary_open {
                        ui.ctx()
                            .data_mut(|data| data.insert_persisted(disclosure_id, !normal_open));
                    }
                    heading = Some(response);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.section_reset_controls_with_order(ui, section, true);
                    });
                });
                if reveal {
                    self.pending_scroll = None;
                    if !temporary_open {
                        ui.ctx()
                            .data_mut(|data| data.insert_persisted(disclosure_id, true));
                    }
                    if let Some(response) = heading {
                        response.request_focus();
                        ui.scroll_to_rect(response.rect, Some(egui::Align::TOP));
                    }
                }
                if open {
                    ui.push_id("body", |ui| self.module_body(ui, section));
                }
                if let Some(materials) = ui.ctx().data(|data| {
                    data.get_temp::<Arc<InspectorMaterials>>(egui::Id::new(
                        "inspector-material-context",
                    ))
                }) && let Some(reason) = controls::module_applicability(
                    section,
                    &self.params,
                    &materials.film,
                    materials.print.as_deref(),
                ) {
                    ui.label(
                        egui::RichText::new(crate::tr!(
                            ui.ctx(),
                            "surfaces.section.inactive",
                            reason = controls::localized(ui.ctx(), &reason)
                        ))
                        .color(theme::FG_MUTED)
                        .small(),
                    );
                }
            });
            ui.add_space(if open {
                theme::SPACE_MODULE
            } else {
                theme::SPACE_2
            });
        });
    }

    fn module_body(&mut self, ui: &mut egui::Ui, section: &str) {
        let _row_scope = controls::row_modified::install(
            ui.ctx(),
            section,
            &self.params,
            &self.gui_state.sections,
        );
        match section {
            "Workflow" => {
                if controls::choice_tip_entries(
                    ui,
                    "workflow",
                    &mut self.params.workflow.route,
                    routes::entries(),
                    crate::i18n::text(ui.ctx(), "surfaces.route.help").as_ref(),
                ) {
                    controls::record_edited_path(ui.ctx(), "workflow.route");
                    self.label_adjustment("Change workflow");
                    self.finish_route_change();
                }
            }
            "Film material" | "Print material" => {
                let film = section == "Film material";
                let path = if film {
                    "extras.simulation.film_stock"
                } else {
                    "extras.simulation.print_paper"
                };
                if controls::entry_matches(
                    path,
                    &self.filter,
                    &self.params,
                    &self.gui_state.sections,
                    self.only_modified,
                ) {
                    let mut selected = if film {
                        self.film_name.clone()
                    } else {
                        self.print_name.clone()
                    };
                    let (changed, response) = if film {
                        profile_combo(ui, "film", "film profile", &self.films, &mut selected)
                    } else {
                        profile_combo(ui, "paper", "print profile", &self.papers, &mut selected)
                    };
                    controls::locate_row(ui, path, &response);
                    if changed {
                        self.label_adjustment(if film {
                            "Change film material"
                        } else {
                            "Change print material"
                        });
                        if film {
                            self.select_film_profile(selected);
                        } else {
                            self.select_print_profile(selected);
                        }
                    }
                }
                self.chemistry_dev_time(ui, film);
            }
            "Source rotation" => {
                ui.horizontal_wrapped(|ui| {
                    let ccw = ui
                        .add_enabled(
                            self.image.is_some(),
                            egui::Button::new(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.source.rotate_ccw",
                            )),
                        )
                        .on_hover_text(crate::i18n::text(
                            ui.ctx(),
                            "surfaces.source.rotate_ccw_help",
                        ));
                    controls::locate_row(ui, "action.rotate_ccw", &ccw);
                    if ccw.clicked() {
                        self.label_adjustment("Rotate source counter-clockwise");
                        self.rotate_input_image_counterclockwise();
                    }
                    let cw = ui
                        .add_enabled(
                            self.image.is_some(),
                            egui::Button::new(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.source.rotate_cw",
                            )),
                        )
                        .on_hover_text(crate::i18n::text(
                            ui.ctx(),
                            "surfaces.source.rotate_cw_help",
                        ));
                    controls::locate_row(ui, "action.rotate_cw", &cw);
                    if cw.clicked() {
                        self.label_adjustment("Rotate source clockwise");
                        self.rotate_input_image_clockwise();
                    }
                    ui.label(crate::tr!(
                        ui.ctx(),
                        "surfaces.source.rotation",
                        degrees = self.input_rotation * 90
                    ));
                });
            }
            "Import Raw" => {
                self.parameter_section_controls(ui, section);
                let raw = self.image_path.as_deref().is_some_and(image_io::is_raw);
                if raw {
                    if self.raw_applied_settings.as_ref()
                        != Some(&self.gui_state.sections["load_raw"])
                    {
                        ui.weak(crate::i18n::text(ui.ctx(), "surfaces.raw.pending"));
                    } else {
                        ui.weak(crate::i18n::text(ui.ctx(), "surfaces.raw.current"));
                    }
                } else {
                    ui.weak(crate::i18n::text(ui.ctx(), "surfaces.raw.inactive"));
                }
                let reprocess = ui.add_enabled(
                    raw,
                    egui::Button::new(crate::i18n::text(ui.ctx(), "surfaces.raw.reprocess")),
                );
                controls::locate_row(ui, "action.reprocess_raw", &reprocess);
                if reprocess.clicked()
                    && let Some(path) = self.image_path.clone()
                {
                    self.load_image_from_path(&path);
                }
            }
            "Scanner" => {
                let mut scan_for_print = self.scan_for_print_snapshot.is_some();
                let response = ui
                    .checkbox(
                        &mut scan_for_print,
                        controls::localized(ui.ctx(), "scan for print"),
                    )
                    .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.scanner.print_help"));
                controls::locate_row(ui, "action.scan_for_print", &response);
                if response.changed() {
                    self.label_adjustment("Toggle scan for print");
                    self.toggle_scan_for_print();
                }
                self.parameter_section_controls(ui, section);
            }
            _ => self.parameter_section_controls(ui, section),
        }
    }

    /// The `modified` chip and the reset action of one section header.
    ///
    /// Every resettable section presents these on its header, so the reset scope
    /// the section table declares is reachable from the surface rendering it.
    /// `trailing` places the action after the chip in a right-aligned header.
    fn section_reset_controls_with_order(
        &mut self,
        ui: &mut egui::Ui,
        section: &str,
        trailing: bool,
    ) -> bool {
        let material_modified = match section {
            "Film material" => controls::entry_matches(
                "extras.simulation.film_stock",
                "",
                &self.params,
                &self.gui_state.sections,
                true,
            ),
            "Print material" => controls::entry_matches(
                "extras.simulation.print_paper",
                "",
                &self.params,
                &self.gui_state.sections,
                true,
            ),
            _ => false,
        };
        let baseline = self
            .current_material_baseline()
            .unwrap_or_else(|| Arc::new(section_reset::unavailable_material_baseline().clone()));
        let modified = material_modified
            || section_reset::is_modified(
                section,
                &self.params,
                &self.gui_state.sections,
                &baseline,
            );
        let resettable = section_reset::sections().any(|name| name == section);
        let mut changed = false;
        if trailing && resettable {
            changed |= self.section_reset_action(ui, section);
        }
        if modified {
            ui.label(
                egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.state.modified"))
                    .color(theme::WARNING)
                    .size(theme::SIZE_HINT),
            );
        }
        if !trailing && resettable {
            changed |= self.section_reset_action(ui, section);
        }
        changed
    }

    fn section_reset_action(&mut self, ui: &mut egui::Ui, section: &str) -> bool {
        let response = controls::module_reset(ui, section);
        if !response.clicked() {
            return false;
        }
        self.reset_parameter_section(section)
    }

    pub(crate) fn reset_parameter_section(&mut self, section: &str) -> bool {
        if !matches!(section, "Session" | "Display") {
            self.label_adjustment_message(
                crate::app::Message::new("operations.history.reset_section")
                    .message_arg("section", crate::app::Message::control(section)),
            );
        }
        if section == "Film material" {
            let factory = state::GuiState::factory();
            if self.film_name != factory.film() {
                self.select_film_profile(factory.film().to_owned());
            }
        } else if section == "Print material" {
            let factory = state::GuiState::factory();
            if self.print_name != factory.paper() {
                self.select_print_profile(factory.paper().to_owned());
            }
        }
        let baseline = self.current_material_baseline();
        if baseline.is_none() && matches!(section, "Grain" | "Halation" | "Couplers") {
            return false;
        }
        let baseline = baseline
            .unwrap_or_else(|| Arc::new(section_reset::unavailable_material_baseline().clone()));
        section_reset::reset(
            &mut self.params,
            &mut self.gui_state.sections,
            section,
            &baseline,
        );
        if section == "Workflow" {
            self.finish_route_change();
        } else if section != "Display" {
            self.mark_parameters_changed_invalidating_calibration();
        }
        self.search_index.reset();
        true
    }

    fn retain_locator_focus(ctx: &egui::Context, current: Vec<egui::Id>) {
        let key = egui::Id::new("control-locator-response-ids");
        let previous = ctx
            .data_mut(|data| data.get_temp::<Vec<egui::Id>>(key))
            .unwrap_or_default();
        if ctx
            .memory(|memory| memory.focused())
            .is_some_and(|id| previous.contains(&id) && !current.contains(&id))
        {
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("control-search")));
        }
        ctx.data_mut(|data| data.insert_temp(key, current));
    }

    fn search_results(&mut self, ui: &mut egui::Ui) {
        if self.filter.is_empty() && !self.only_modified {
            Self::retain_locator_focus(ui.ctx(), Vec::new());
            return;
        }
        let results = self.search_index.results_filtered(
            &self.filter,
            &self.params,
            &self.gui_state.sections,
            self.film_medium,
            self.parameter_revision,
            &self.data_dir,
            self.only_modified,
        );
        if results.is_empty() {
            Self::retain_locator_focus(ui.ctx(), Vec::new());
            let message = if self.filter.is_empty() {
                crate::i18n::text(ui.ctx(), "surfaces.search.none_modified").into_owned()
            } else if self.only_modified {
                crate::tr!(
                    ui.ctx(),
                    "surfaces.search.no_modified_match",
                    query = &self.filter
                )
                .into_owned()
            } else {
                crate::tr!(ui.ctx(), "surfaces.search.no_match", query = &self.filter).into_owned()
            };
            ui.label(egui::RichText::new(message).color(theme::FG_MUTED).small());
            return;
        }
        let mut destination = None;
        let mut locator_ids = Vec::with_capacity(results.len());
        egui::ScrollArea::vertical()
            .id_salt("control-locators")
            .max_height(120.0)
            .show(ui, |ui| {
                for result in results {
                    let label = result.localized_locator_label(ui.ctx());
                    let mut details = result.localized_details(ui.ctx());
                    if let Some(reason) =
                        control_inactive_reason(ui.ctx(), &result.target_path, &self.params)
                    {
                        details.push('\n');
                        details.push_str(controls::localized(ui.ctx(), &reason).as_ref());
                    }
                    let response = ui.button(&label).on_hover_text(details);
                    locator_ids.push(response.id);
                    if response.clicked() {
                        destination = Some((
                            result.owner,
                            result.section,
                            result.target_path.clone(),
                            response.id,
                        ));
                    }
                }
            });
        Self::retain_locator_focus(ui.ctx(), locator_ids);
        if let Some((owner, section, path, _invoking_id)) = destination {
            controls::request_focus(ui.ctx(), section, &path);
            match owner {
                controls::ControlOwner::Stage(stage) => {
                    self.pending_stage = Some(stage);
                    self.pending_scroll = Some(section.to_owned());
                }
                controls::ControlOwner::Settings => {
                    crate::numeric::suspend_numeric_draft(ui.ctx());
                    self.settings_open = true;
                    self.settings_return_focus = Some(egui::Id::new("control-search"));
                    self.pending_scroll = Some(section.to_owned());
                }
                controls::ControlOwner::Export => {
                    self.export_return_focus = Some(egui::Id::new("control-search"));
                    if !self.export_dialog.is_open() {
                        self.export_dialog.open(&self.export_options);
                    }
                }
                controls::ControlOwner::Viewer => {
                    self.pending_scroll = None;
                }
                controls::ControlOwner::Header | controls::ControlOwner::Result => {
                    self.pending_scroll = None;
                }
                controls::ControlOwner::WorkList => {
                    self.photograph.work_list.open = true;
                    self.pending_scroll = None;
                }
                controls::ControlOwner::Presets => {
                    if ui.ctx().screen_rect().width() > theme::WIDTH_COMPACT_WORKSPACE {
                        self.presets.state.expanded = true;
                    } else {
                        self.presets.narrow_open = true;
                    }
                    self.pending_scroll = None;
                }
            }
        }
    }

    fn parameter_section_controls(&mut self, ui: &mut egui::Ui, section: &str) {
        let scanned_medium_positive = self.scanned_medium_positive();
        let cache = self.section_labels.entry(section.to_owned()).or_default();
        let capture_reference_illuminant = &self.film_reference_illuminant;
        let data_dir = self.data_dir.clone();
        let changes = controls::show(
            ui,
            &mut self.params,
            &mut self.gui_state.sections,
            section,
            self.film_medium,
            scanned_medium_positive,
            &self.filter,
            cache,
            capture_reference_illuminant,
            &data_dir,
        );
        if changes.runtime_changed {
            self.mark_parameters_changed_invalidating_calibration();
        }
        if let Some(comparing) = changes.operator_compare {
            self.set_operator_compare(comparing);
        }
        if let Some(comparing) = changes.finishing_compare {
            self.set_finishing_compare(comparing);
        }
        if let Some(action) = changes.action {
            self.start_calibration(action);
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

    fn chemistry_dev_time(&mut self, ui: &mut egui::Ui, film: bool) {
        let path = if film {
            "film_render.development_time"
        } else {
            "print_render.development_time"
        };
        if !controls::entry_matches(
            path,
            &self.filter,
            &self.params,
            &self.gui_state.sections,
            self.only_modified,
        ) {
            return;
        }
        let times = if film {
            &self.film_dev_times
        } else {
            &self.print_dev_times
        };
        if times.len() <= 1 {
            let effective = times.first().copied().unwrap_or(1.0);
            ui.label(crate::tr!(
                ui.ctx(),
                "surfaces.chemistry.material_time",
                time = effective
            ));
            return;
        }
        let inactive = ui
            .ctx()
            .data(|data| {
                data.get_temp::<Arc<InspectorMaterials>>(egui::Id::new(
                    "inspector-material-context",
                ))
            })
            .and_then(|materials| {
                spektrafilm_core::params::sources::inactive_reason_with_materials(
                    path,
                    &self.params,
                    Some(&materials.film),
                    materials.print.as_deref(),
                )
            });
        if let Some(reason) = &inactive {
            ui.weak(crate::tr!(
                ui.ctx(),
                "surfaces.chemistry.inactive",
                reason = controls::localized(ui.ctx(), reason)
            ));
        }
        let (times, selected) = if film {
            (
                &self.film_dev_times,
                &mut self.params.film_render.development_time,
            )
        } else {
            (
                &self.print_dev_times,
                &mut self.params.print_render.development_time,
            )
        };
        let (changed, response) = ui
            .add_enabled_ui(inactive.is_none(), |ui| {
                dev_time_combo(
                    ui,
                    if film { "film-time" } else { "print-time" },
                    "development time",
                    times,
                    selected,
                )
            })
            .inner;
        if changed {
            controls::record_edited_path(
                ui.ctx(),
                if film {
                    "film_render.development_time"
                } else {
                    "print_render.development_time"
                },
            );
            self.mark_parameters_changed();
        }
        controls::locate_row(ui, path, &response);
    }

    fn controls_panel(&mut self, ui: &mut egui::Ui) {
        let input_view_before = (
            self.params.io.input_color_space.clone(),
            self.params.io.input_cctf_decoding,
        );
        let cache_id = egui::Id::new("inspector-material-context");
        let route = self.params.workflow.route_enum();
        let need_print = route.executes_print_exposure(&self.params.taps)
            || route.executes_print_development(&self.params.taps)
            || route.executes_print_readout(&self.params.taps);
        let cached = ui
            .ctx()
            .data(|data| data.get_temp::<Arc<InspectorMaterials>>(cache_id));
        let cache_current = cached.as_ref().is_some_and(|materials| {
            materials.film_name == self.film_name
                && materials.print_name == self.print_name
                && (!need_print || materials.print.is_some())
        });
        if !cache_current {
            let film = cached
                .as_ref()
                .filter(|materials| materials.film_name == self.film_name)
                .map(|materials| Arc::clone(&materials.film))
                .or_else(|| {
                    profile::load_profile_by_name(&self.data_dir, &self.film_name)
                        .ok()
                        .map(Arc::new)
                });
            let print = if need_print {
                cached
                    .as_ref()
                    .filter(|materials| materials.print_name == self.print_name)
                    .and_then(|materials| materials.print.as_ref().map(Arc::clone))
                    .or_else(|| {
                        profile::load_profile_by_name(&self.data_dir, &self.print_name)
                            .ok()
                            .map(Arc::new)
                    })
            } else {
                None
            };
            if let Some(film) = film {
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        cache_id,
                        Arc::new(InspectorMaterials {
                            film_name: self.film_name.clone(),
                            print_name: self.print_name.clone(),
                            film,
                            print,
                        }),
                    )
                });
            }
        }
        for &(stage, sections) in controls::STAGE_LAYOUT {
            let reveal_stage = self.pending_stage == Some(stage);
            let has_locator = sections
                .iter()
                .any(|section| self.pending_scroll.as_deref() == Some(*section));
            let visible = sections.iter().any(|section| {
                controls::section_matches(
                    section,
                    &self.filter,
                    &self.params,
                    &self.gui_state.sections,
                    self.only_modified,
                )
            });
            if !visible && !reveal_stage && !has_locator {
                continue;
            }
            ui.push_id(("stage", stage.label()), |ui| {
                ui.add_space(theme::SPACE_2);
                let open_id = ui.make_persistent_id("open");
                let normal_open = ui
                    .ctx()
                    .data_mut(|data| *data.get_persisted_mut_or_insert_with(open_id, || true));
                let filtered = !self.filter.is_empty() || self.only_modified;
                let open = normal_open || filtered || reveal_stage || has_locator;
                let response = ui
                    .horizontal(|ui| {
                        let number = match stage {
                            controls::Stage::Input => "01",
                            controls::Stage::Film => "02",
                            controls::Stage::Print => "03",
                            controls::Stage::Scan => "04",
                            controls::Stage::Output => "05",
                        };
                        ui.label(
                            egui::RichText::new(number)
                                .monospace()
                                .small()
                                .color(theme::ACCENT_DIM),
                        );
                        controls::stage_disclosure(ui, stage.label(), open)
                    })
                    .inner;
                let accessible_name = crate::tr!(
                    ui.ctx(),
                    "surfaces.section.stage",
                    stage = controls::localized(ui.ctx(), stage.label())
                );
                ui.ctx()
                    .accesskit_node_builder(response.id, |node| node.set_label(accessible_name));
                controls::focus_feedback(ui, &response);
                if response.clicked() && !filtered {
                    ui.ctx()
                        .data_mut(|data| data.insert_persisted(open_id, !normal_open));
                }
                if reveal_stage || has_locator {
                    ui.ctx()
                        .data_mut(|data| data.insert_persisted(open_id, true));
                    if reveal_stage && !has_locator {
                        response.request_focus();
                        ui.scroll_to_rect(response.rect, Some(egui::Align::TOP));
                    }
                    self.pending_stage = None;
                }
                if open {
                    for section in sections {
                        self.parameter_section(ui, section);
                    }
                }
                ui.add_space(theme::SPACE_MODULE);
            });
        }
        if input_view_before
            != (
                self.params.io.input_color_space.clone(),
                self.params.io.input_cctf_decoding,
            )
        {
            self.refresh_input_viewer();
            self.refresh_viewing_artifacts();
        }
    }
    fn sync_display_controls(&mut self) {
        let display = self.viewer.settings.to_json();
        let mut changed = false;
        for key in [
            "use_display_transform",
            "gray_18_canvas",
            "white_padding",
            "output_interpolation",
        ] {
            if self.gui_state.sections["display"][key] != display[key] {
                self.gui_state.sections["display"][key] = display[key].clone();
                changed = true;
            }
        }
        if changed {
            self.search_index.reset();
        }
    }

    fn settings_dialog(&mut self, ctx: &egui::Context) {
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
                    for section in ["Session", "GUI parameters", "Display", "Diagnostics"] {
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
                                "GUI parameters" => self.state_toolbar(ui),
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

    fn open_image_dialog(&mut self) {
        if let Some(path) = self.chooser.pick_file(
            self.file_dialog("load")
                .add_filter("Image", &image_io::accepted_extensions()),
        ) {
            self.remember_dialog("load", &path);
            self.request_photo_action(crate::photograph::PhotoAction::Open(path));
        }
    }

    /// Result identity is independent from the current worker's operation.
    /// In particular, a pending Scan must not relabel a retained Preview.
    pub(crate) fn retained_result_identity(&self, ctx: &egui::Context) -> String {
        let Some([width, height]) = self
            .output_retained
            .as_ref()
            .map(|image| [image.width(), image.height()])
            .or_else(|| {
                self.output_image
                    .as_ref()
                    .map(|image| [image.width, image.height])
            })
        else {
            return crate::i18n::text(ctx, "result.identity.empty").into_owned();
        };
        let operation = self.retained_kind.map_or_else(
            || crate::i18n::text(ctx, "result.identity.result").into_owned(),
            |kind| kind.message().localized(ctx),
        );
        crate::tr!(
            ctx,
            "result.identity.summary",
            operation = operation,
            width = width,
            height = height,
            currency = crate::i18n::text(
                ctx,
                if self.accepted_frame_is_stale() {
                    "result.identity.stale"
                } else {
                    "result.identity.current"
                }
            )
        )
        .into_owned()
    }

    /// One stable line describes the displayed layer and retained result.
    fn viewer_status(&self, ui: &mut egui::Ui) {
        let has_output = self.output_image.is_some() || self.output_retained.is_some();
        let original = self.viewer.settings.layer == display::ViewLayer::Input;
        let identity = if self.image.is_none() {
            crate::i18n::text(ui.ctx(), "result.identity.empty").into_owned()
        } else if original {
            crate::i18n::text(ui.ctx(), "surfaces.viewer.original_status").into_owned()
        } else if self.viewer.settings.layer == display::ViewLayer::PaperBack {
            crate::i18n::text(ui.ctx(), "surfaces.viewer.paper_status").into_owned()
        } else {
            self.retained_result_identity(ui.ctx())
        };
        let preview = !original
            && self.viewer.settings.layer == display::ViewLayer::Output
            && has_output
            && self.retained_kind == Some(RenderKind::Preview);
        ui.allocate_ui_with_layout(
            ui.available_size(),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                let help = crate::i18n::text(ui.ctx(), "surfaces.viewer.gestures_help");
                let help_button = controls::icon_button(ui, help.as_ref(), controls::Icon::Help);
                let help_id = ui.make_persistent_id("viewer-gesture-help");
                if help_button.clicked() {
                    ui.memory_mut(|memory| memory.toggle_popup(help_id));
                }
                egui::popup::popup_below_widget(
                    ui,
                    help_id,
                    &help_button,
                    egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                    |ui| {
                        ui.set_max_width(320.0);
                        let layer = crate::i18n::text(
                            ui.ctx(),
                            if original {
                                "surfaces.viewer.input"
                            } else if self.viewer.settings.layer == display::ViewLayer::PaperBack {
                                "surfaces.viewer.paper_back"
                            } else {
                                "surfaces.viewer.output"
                            },
                        );
                        ui.label(crate::tr!(
                            ui.ctx(),
                            "surfaces.viewer.gestures",
                            layer = layer,
                            zoom = format!(
                                "{:.0}",
                                self.viewer
                                    .zoom_percent()
                                    .unwrap_or(self.viewer.zoom * 100.0)
                            )
                        ));
                    },
                );
                if self.viewer_render_pending() {
                    let label = ui.label(
                        egui::RichText::new(crate::i18n::text(
                            ui.ctx(),
                            "surfaces.viewer.rendering",
                        ))
                        .small(),
                    );
                    controls::locate_row(ui, "viewer.rendering", &label);
                }
                let width = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(width, theme::HEIGHT_CONTROL),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let color = if !original && has_output && self.accepted_frame_is_stale() {
                            theme::WARNING
                        } else {
                            theme::FG_SECONDARY
                        };
                        let (dot, _) =
                            ui.allocate_exact_size(egui::vec2(6.0, 6.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot.center(), 2.5, color);
                        let identity_width = if preview {
                            ui.available_width() * 0.62
                        } else {
                            ui.available_width()
                        };
                        ui.allocate_ui_with_layout(
                            egui::vec2(identity_width, theme::HEIGHT_CONTROL),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_min_width(identity_width);
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&identity).small().color(color),
                                    )
                                    .truncate(),
                                )
                            },
                        )
                        .inner
                        .on_hover_text(&identity);
                        if preview {
                            ui.separator();
                            let caveat =
                                crate::i18n::text(ui.ctx(), "result.identity.preview_caveat");
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(caveat.as_ref())
                                        .small()
                                        .color(theme::FG_MUTED),
                                )
                                .truncate(),
                            )
                            .on_hover_text(caveat);
                        }
                    },
                );
            },
        );
    }

    /// One toolbar row owns global actions and two-line photograph context.
    fn window_header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("window-header")
            .exact_height(theme::HEIGHT_HEADER)
            .frame(
                egui::Frame::NONE
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::symmetric(theme::SPACE_4 as i8, 14)),
            )
            .show(ctx, |ui| {
                ui.allocate_ui_with_layout(
                    ui.available_size(),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = theme::SPACE_3;
                        let (mark, _) =
                            ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                        ui.painter().circle_stroke(
                            mark.center(),
                            11.5,
                            theme::stroke(theme::ACCENT_DIM),
                        );
                        controls::paint_icon(
                            ui.painter(),
                            mark,
                            controls::Icon::Aperture,
                            theme::ACCENT,
                        );
                        if ctx.screen_rect().width() > theme::WIDTH_COMPACT_WORKSPACE {
                            ui.label(
                                egui::RichText::new("SPEKTRAFILM")
                                    .strong()
                                    .size(theme::SIZE_LABEL),
                            );
                        }
                        let open_name = crate::i18n::text(ctx, "surfaces.action.open");
                        if controls::action_button(ui, open_name.as_ref(), controls::Icon::Open)
                            .on_hover_text(crate::i18n::text(ctx, "surfaces.action.open_help"))
                            .clicked()
                        {
                            self.open_image_dialog();
                        }
                        self.photograph_toolbar(ui);
                        ui.separator();
                        let settings_name = crate::i18n::text(ctx, "surfaces.settings.title");
                        let export_name = crate::i18n::text(
                            ctx,
                            if self.export_job.is_some() {
                                "surfaces.export.cancel"
                            } else {
                                "surfaces.action.export"
                            },
                        );
                        let text_action_width = |label: &str| {
                            let text = egui::WidgetText::from(label).into_galley(
                                ui,
                                Some(egui::TextWrapMode::Extend),
                                f32::INFINITY,
                                egui::TextStyle::Button,
                            );
                            text.size().x
                                + 17.0
                                + theme::SPACE_2
                                + 2.0 * ui.spacing().button_padding.x
                        };
                        let right_width = text_action_width(&settings_name)
                            + text_action_width(&export_name)
                            + 2.0 * theme::HEIGHT_CONTROL
                            + 3.0 * theme::SPACE_3;
                        let context_width =
                            (ui.available_width() - right_width - theme::SPACE_3).max(1.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(context_width, 36.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_min_width(context_width);
                                ui.spacing_mut().item_spacing.y = 0.0;
                                let source = self
                                    .image_path
                                    .as_deref()
                                    .and_then(Path::file_name)
                                    .map(|name| name.to_string_lossy())
                                    .unwrap_or_else(|| {
                                        crate::i18n::text(ctx, "surfaces.header.no_source")
                                    });
                                let source_label =
                                    crate::tr!(ctx, "surfaces.header.source", source = source);
                                let visible_source = if self.photograph_has_unsaved_changes() {
                                    format!(
                                        "{source} · {}",
                                        crate::i18n::text(ctx, "photo.record.unsaved")
                                    )
                                } else {
                                    source.into_owned()
                                };
                                let response = ui.add(
                                    egui::Label::new(egui::RichText::new(visible_source).strong())
                                        .truncate(),
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Label,
                                        true,
                                        source_label.as_ref(),
                                    )
                                });
                                ctx.accesskit_node_builder(response.id, |node| {
                                    node.set_label(source_label.as_ref())
                                });
                                response.on_hover_text(self.image_path.as_ref().map_or_else(
                                    || source_label.into_owned(),
                                    |path| path.display().to_string(),
                                ));
                                let context = format!(
                                    "{} · {} · {}",
                                    profiles::selected_display(&self.films, &self.film_name),
                                    profiles::selected_display(&self.papers, &self.print_name),
                                    controls::localized(
                                        ctx,
                                        routes::vocabulary::route_description(
                                            &self.params.workflow.route
                                        )
                                    ),
                                );
                                let status = self.status.localized(ctx);
                                let routine = status.is_empty()
                                    || matches!(
                                        &self.status,
                                        crate::app::Message::Text {
                                            key: "operations.rendered"
                                                | "operations.comparing_baseline"
                                                | "surfaces.status.loaded"
                                                | "surfaces.status.loaded_raw"
                                                | "surfaces.status.preparing_input",
                                            ..
                                        }
                                    );
                                // Material and workflow identity is persistent context;
                                // an operation outcome gets its own bounded segment
                                // instead of replacing that identity.
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = theme::SPACE_2;
                                    let available = ui.available_width();
                                    let status_width = if routine {
                                        0.0
                                    } else {
                                        (available * 0.38).min(190.0)
                                    };
                                    let context_width = (available
                                        - if routine {
                                            0.0
                                        } else {
                                            status_width + ui.spacing().item_spacing.x
                                        })
                                    .max(1.0);
                                    let context_response = ui.add_sized(
                                        [context_width, ui.available_height().max(14.0)],
                                        egui::Label::new(
                                            egui::RichText::new(&context)
                                                .small()
                                                .color(theme::FG_MUTED),
                                        )
                                        .truncate(),
                                    );
                                    context_response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Label,
                                            true,
                                            context.as_str(),
                                        )
                                    });
                                    ctx.accesskit_node_builder(context_response.id, |node| {
                                        node.set_label(context.as_str())
                                    });
                                    context_response.on_hover_text(context.as_str());
                                    if !routine {
                                        ui.separator();
                                        let status_response = ui.add_sized(
                                            [status_width, ui.available_height().max(14.0)],
                                            egui::Label::new(
                                                egui::RichText::new(status.as_str())
                                                    .small()
                                                    .color(theme::FG_MUTED),
                                            )
                                            .halign(egui::Align::Max)
                                            .truncate(),
                                        );
                                        status_response.widget_info(|| {
                                            egui::WidgetInfo::labeled(
                                                egui::WidgetType::Label,
                                                true,
                                                status.as_str(),
                                            )
                                        });
                                        ctx.accesskit_node_builder(status_response.id, |node| {
                                            node.set_label(status.as_str())
                                        });
                                        status_response.on_hover_text(status.as_str());
                                    }
                                });
                            },
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.export_job.is_some() {
                                if ui
                                    .add_enabled_ui(true, |ui| {
                                        controls::action_button(
                                            ui,
                                            export_name.as_ref(),
                                            controls::Icon::Close,
                                        )
                                    })
                                    .inner
                                    .on_hover_text(crate::i18n::text(
                                        ctx,
                                        "surfaces.export.cancel_help",
                                    ))
                                    .clicked()
                                {
                                    self.cancel_export();
                                }
                            } else {
                                let enabled = self.image.is_some()
                                    && self.calibration_job.is_none()
                                    && !self.export_dialog.is_open();
                                let export = ui
                                    .add_enabled_ui(enabled, |ui| {
                                        let visuals = ui.visuals_mut();
                                        for visual in [
                                            &mut visuals.widgets.inactive,
                                            &mut visuals.widgets.hovered,
                                            &mut visuals.widgets.active,
                                        ] {
                                            visual.weak_bg_fill = theme::ACCENT;
                                            visual.bg_fill = theme::ACCENT;
                                            visual.fg_stroke =
                                                theme::stroke(theme::CONTRAST_ON_ACCENT);
                                        }
                                        controls::action_button(
                                            ui,
                                            export_name.as_ref(),
                                            controls::Icon::Export,
                                        )
                                    })
                                    .inner;
                                if export.clicked() {
                                    self.export_return_focus = Some(export.id);
                                    self.export_dialog.open(&self.export_options);
                                }
                            }
                            let settings = controls::action_button(
                                ui,
                                settings_name.as_ref(),
                                controls::Icon::Settings,
                            );
                            if settings.is_pointer_button_down_on() {
                                crate::numeric::suspend_numeric_draft(ctx);
                            }
                            if settings.clicked() {
                                crate::numeric::suspend_numeric_draft(ctx);
                                self.settings_return_focus = Some(settings.id);
                                self.settings_open = true;
                            }
                            let redo_help = self.redo_label_localized(ctx).map_or_else(
                                || crate::i18n::text(ctx, "surfaces.history.no_redo").into_owned(),
                                |label| {
                                    crate::tr!(ctx, "surfaces.history.redo", label = label)
                                        .into_owned()
                                },
                            );
                            let redo_name = crate::i18n::text(ctx, "surfaces.action.redo");
                            if ui
                                .add_enabled_ui(
                                    self.can_redo() && !self.input_transition_pending(),
                                    |ui| {
                                        controls::icon_button(
                                            ui,
                                            redo_name.as_ref(),
                                            controls::Icon::Redo,
                                        )
                                    },
                                )
                                .inner
                                .on_hover_text(redo_help)
                                .clicked()
                            {
                                self.redo_adjustment();
                            }
                            let undo_help = self.undo_label_localized(ctx).map_or_else(
                                || crate::i18n::text(ctx, "surfaces.history.no_undo").into_owned(),
                                |label| {
                                    crate::tr!(ctx, "surfaces.history.undo", label = label)
                                        .into_owned()
                                },
                            );
                            let undo_name = crate::i18n::text(ctx, "surfaces.action.undo");
                            if ui
                                .add_enabled_ui(
                                    self.can_undo() && !self.input_transition_pending(),
                                    |ui| {
                                        controls::icon_button(
                                            ui,
                                            undo_name.as_ref(),
                                            controls::Icon::Undo,
                                        )
                                    },
                                )
                                .inner
                                .on_hover_text(undo_help)
                                .clicked()
                            {
                                self.undo_adjustment();
                            }
                        });
                    },
                );
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.photograph.context = Some(ctx.clone());
        self.bind_language(ctx);
        self.protect_photograph_close(ctx);
        if controls::release_operator_compare(ctx) {
            self.set_operator_compare(false);
        }
        if controls::release_finishing_compare(ctx) {
            self.set_finishing_compare(false);
        }
        if self.finishing_compare {
            // A route or stock change may make an already-held comparison inapplicable.
            self.set_finishing_compare(true);
        }
        // First-paint hook: tag the wgpu Metal layer's colorspace as sRGB.
        // Has to happen here (not in `App::new`) because the CAMetalLayer
        // isn't wired up at construction time.
        #[cfg(target_os = "macos")]
        if !self.metal_colorspace_tagged
            && raw_window_handle::HasWindowHandle::window_handle(frame).is_ok()
        {
            match crate::platform::tag_metal_layer_srgb(frame) {
                Ok(()) => {
                    eprintln!("[spektrafilm] CAMetalLayer.colorspace = sRGB — tagged OK");
                    self.metal_colorspace_tagged = true;
                }
                Err(e) => {
                    eprintln!("[spektrafilm] colorspace tag attempt: {e}");
                }
            }
        }
        let _ = frame;
        self.poll_input_job(ctx);

        self.begin_adjustment_frame(ctx);
        self.poll_render_job(ctx);
        self.poll_calibration_job(ctx);
        self.poll_export_job(ctx);
        self.poll_pending_saves(ctx);
        let editing_text = ctx
            .memory(|memory| memory.focused())
            .is_some_and(|id| egui::text_edit::TextEditState::load(ctx, id).is_some());
        if !editing_text
            && !self.settings_open
            && !self.export_dialog.is_open()
            && !self.stale_save_confirmation
            && !self.input_transition_pending()
        {
            if ctx.input_mut(|input| {
                input.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                )
            }) {
                self.redo_adjustment();
            } else if ctx
                .input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z))
            {
                self.undo_adjustment();
            }
        }
        // Share the renderer's exact crop and rescale rounding without
        // allocating an image just to describe its delivered dimensions.
        let export_dimensions = self
            .oriented_input
            .as_ref()
            .and_then(|image| {
                crate::export::output_dimensions([image.width(), image.height()], &self.params.io)
                    .ok()
            })
            .unwrap_or([0, 0]);
        if let Some(options) =
            self.export_dialog
                .show(ctx, export_dimensions, self.image_path.as_deref())
        {
            self.start_export(ctx, options);
        }
        self.window_header(ctx);
        self.show_stale_save_confirmation(ctx);
        self.photograph_dialogs(ctx);
        self.work_list_controls(ctx);
        self.preset_panel(ctx);
        self.sync_display_controls();
        let baseline = self
            .current_material_baseline()
            .unwrap_or_else(|| Arc::new(section_reset::unavailable_material_baseline().clone()));
        controls::begin_control_frame(ctx, &self.params, &self.gui_state.sections, baseline);
        // Fixed search and stage jumps above one scrolling inspector.
        egui::SidePanel::right("controls")
            .resizable(true)
            .default_width(theme::WIDTH_CONTROL_PANEL)
            .min_width(300.0)
            .max_width((ctx.screen_rect().width() * 0.48).max(300.0))
            .frame(
                egui::Frame::NONE
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::symmetric(
                        theme::SPACE_MODULE as i8,
                        theme::SPACE_2 as i8,
                    )),
            )
            .show(ctx, |ui| {
                if self.input_transition_pending() {
                    ui.disable();
                }
                // Panel sizing belongs to the user's resize gesture, never to a
                // long query, locator, or applicability explanation inside it.
                let inspector_rect = ui.available_rect_before_wrap();
                ui.allocate_space(inspector_rect.size());
                let mut inspector = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("inspector-content")
                        .max_rect(inspector_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                inspector.set_clip_rect(inspector_rect);
                inspector.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                let ui = &mut inspector;
                ui.add_space(theme::SPACE_BLOCK);
                ui.horizontal(|ui| {
                    let clear_width = if self.filter.is_empty() {
                        0.0
                    } else {
                        theme::HEIGHT_CONTROL + theme::SPACE_2
                    };
                    let modified_label = crate::i18n::text(ui.ctx(), "surfaces.settings.modified");
                    let label_width = egui::WidgetText::from(modified_label.as_ref())
                        .into_galley(
                            ui,
                            Some(egui::TextWrapMode::Extend),
                            f32::INFINITY,
                            egui::TextStyle::Body,
                        )
                        .size()
                        .x;
                    let toggle_width =
                        29.0 + ui.spacing().item_spacing.x + label_width + theme::SPACE_1;
                    let search_width =
                        (ui.available_width() - clear_width - toggle_width - theme::SPACE_2)
                            .max(1.0);
                    let response = ui.add_sized(
                        [search_width, ui.spacing().interact_size.y],
                        egui::TextEdit::singleline(&mut self.filter)
                            .margin(egui::vec2(theme::SPACE_2, 5.0))
                            .id(egui::Id::new("control-search"))
                            .hint_text(crate::i18n::text(ui.ctx(), "surfaces.search.label"))
                            .clip_text(true)
                            .desired_width(search_width),
                    );
                    let accessible_name = crate::i18n::text(ui.ctx(), "surfaces.search.label");
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.set_label(accessible_name)
                    });
                    if !self.filter.is_empty() {
                        let clear = crate::i18n::text(ui.ctx(), "surfaces.search.clear");
                        if controls::icon_button(ui, clear.as_ref(), controls::Icon::Close)
                            .on_hover_text(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.search.clear_help",
                            ))
                            .clicked()
                        {
                            self.filter.clear();
                        }
                    }
                    ui.allocate_ui_with_layout(
                        egui::vec2(toggle_width, theme::HEIGHT_CONTROL),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(toggle_width);
                            let label = crate::i18n::text(ui.ctx(), "surfaces.settings.modified");
                            let help = crate::i18n::text(ui.ctx(), "surfaces.search.modified_help");
                            controls::toggle_row(
                                ui,
                                label.as_ref(),
                                &mut self.only_modified,
                                help.as_ref(),
                            );
                        },
                    );
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SPACE_1;
                    let stage_width = (ui.available_width() - 4.0 * theme::SPACE_1) / 5.0;
                    for stage in controls::Stage::ALL {
                        let label = if stage == controls::Stage::Input {
                            crate::i18n::text(ui.ctx(), "surfaces.search.input_jump")
                        } else {
                            controls::localized(ui.ctx(), stage.label())
                        };
                        let response = ui
                            .add_sized(
                                [stage_width, theme::HEIGHT_CONTROL],
                                egui::Button::new(egui::RichText::new(label).small()).frame(false),
                            )
                            .on_hover_text(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.search.stage_help",
                            ));
                        controls::focus_feedback(ui, &response);
                        let accessible_name = controls::localized(ui.ctx(), stage.label());
                        ui.ctx().accesskit_node_builder(response.id, |node| {
                            node.set_label(accessible_name.as_ref())
                        });
                        if response.clicked() {
                            self.pending_stage = Some(stage);
                        }
                    }
                });
                ui.separator();
                self.search_results(ui);
                controls::install_view_filter(ui.ctx(), self.only_modified);
                ui.add_space(theme::SPACE_ROW);
                // Reserve the action bar's height below the scrollable
                // controls, so the bar stays inside the panel.
                let scroll_height =
                    (ui.available_height() - theme::HEIGHT_ACTION_BAR_RESERVE).max(1.0);
                ui.allocate_ui(egui::vec2(ui.available_width(), scroll_height), |ui| {
                    let scroll = egui::ScrollArea::vertical()
                        .id_salt((
                            "controls-scroll",
                            !self.filter.is_empty() || self.only_modified,
                        ))
                        .max_width(ui.available_width())
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            self.controls_panel(ui);
                            ui.unique_id()
                        });
                    let content_label = crate::i18n::text(ctx, "surfaces.search.content");
                    ctx.accesskit_node_builder(scroll.inner, |node| {
                        node.set_role(egui::accesskit::Role::ScrollView);
                        node.set_label(content_label.as_ref());
                        node.set_bounds(egui::accesskit::Rect {
                            x0: f64::from(scroll.inner_rect.left()),
                            y0: f64::from(scroll.inner_rect.top()),
                            x1: f64::from(scroll.inner_rect.right()),
                            y1: f64::from(scroll.inner_rect.bottom()),
                        });
                    });
                    controls::install_view_filter(ui.ctx(), false);
                });
                ui.separator();
                self.simulation_action_bar(ui);
            });
        self.persist_browser_state(false);
        self.persist_analysis_preferences(false);
        self.persist_clipping_preferences(false);
        if self.browser_state_changed_at.is_some()
            || self.analysis_preferences_changed_at.is_some()
            || self.clipping_preferences_changed_at.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(300));
        }
        if self.diagnostics.pending_publications() > 0 {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if let Some(error) = self.diagnostics.take_publication_error() {
            self.status =
                crate::app::Message::new("surfaces.status.publication_error").arg("error", error);
        }
        // Central viewer
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                // Reserve both fixed strips before allocating any of the shelf.
                let status_rect = egui::TopBottomPanel::top("viewer-status")
                    .exact_height(theme::HEIGHT_VIEWER_STATUS)
                    .frame(
                        egui::Frame::NONE
                            .fill(theme::BG)
                            .inner_margin(egui::Margin::symmetric(theme::SPACE_3 as i8, 3)),
                    )
                    .show_inside(ui, |ui| ui.available_rect_before_wrap())
                    .inner;
                let available = (ui.available_height() - theme::HEIGHT_VIEWER_FOOTER).max(0.0);
                let capacity = (available - theme::HEIGHT_PHOTOGRAPH_MIN)
                    .clamp(0.0, theme::HEIGHT_ANALYSIS_MAX);
                let space_available = capacity >= theme::HEIGHT_ANALYSIS_MIN;
                self.viewer.clipping_suspended = self.operator_compare
                    || self.finishing_compare
                    || self.displayed_comparison.is_some();
                let clipping_source_available = match self.viewer.settings.layer {
                    crate::display::ViewLayer::Input => self.oriented_input.is_some(),
                    crate::display::ViewLayer::Output => {
                        self.output_image.is_some() || self.output_retained.is_some()
                    }
                    crate::display::ViewLayer::PaperBack => false,
                };
                let clipping_modal_open = self.settings_open
                    || self.export_dialog.is_open()
                    || self.factory_reset_confirm
                    || self.stale_save_confirmation
                    || self.photograph_confirmation_open();
                let footer_id = egui::TopBottomPanel::bottom("viewer-footer")
                    .exact_height(theme::HEIGHT_VIEWER_FOOTER)
                    .frame(
                        egui::Frame::NONE
                            .fill(theme::SURFACE)
                            .inner_margin(egui::Margin::symmetric(theme::SPACE_3 as i8, 9)),
                    )
                    .show_inside(ui, |ui| {
                        self.viewer.footer_controls(
                            ui,
                            &mut self.analysis.visible,
                            space_available,
                            clipping_source_available,
                            clipping_modal_open,
                        )
                    })
                    .inner;

                // Identify only controls removed with the shelf, not inspector focus.
                let shelf_rect_id = egui::Id::new("analysis-shelf-previous-rect");
                let viewer_controls_id = egui::Id::new("analysis-viewer-controls-previous-rect");
                let focused_removed_control = |ctx: &egui::Context| {
                    let focused = ctx.memory(|memory| memory.focused());
                    let rect = focused.and_then(|id| ctx.read_response(id)).map(|r| r.rect);
                    let (shelf, viewer_controls) = ctx.data(|data| {
                        (
                            data.get_temp::<egui::Rect>(shelf_rect_id),
                            data.get_temp::<egui::Rect>(viewer_controls_id),
                        )
                    });
                    rect.is_some_and(|rect| {
                        shelf.is_some_and(|area| area.contains_rect(rect))
                            || viewer_controls.is_some_and(|area| area.contains_rect(rect))
                    })
                };
                let expanded = self.analysis.visible && space_available;
                if self.analysis.expanded && !expanded {
                    if focused_removed_control(ctx) {
                        ctx.memory_mut(|memory| memory.request_focus(footer_id));
                    }
                    self.suspend_image_analysis();
                }
                self.analysis.expanded = expanded;
                if expanded {
                    let height = self
                        .analysis
                        .preferences
                        .shelf_height
                        .clamp(theme::HEIGHT_ANALYSIS_MIN, theme::HEIGHT_ANALYSIS_MAX)
                        .min(capacity);
                    let shelf = egui::TopBottomPanel::bottom("image-analysis-workspace")
                        .exact_height(height)
                        .show_separator_line(false)
                        .frame(egui::Frame::NONE.fill(theme::SURFACE))
                        .show_inside(ui, |ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 6.0),
                                egui::Sense::hover(),
                            );
                            let divider = ui
                                .interact(
                                    rect,
                                    egui::Id::new("analysis-shelf-divider"),
                                    egui::Sense::click_and_drag(),
                                )
                                .on_hover_cursor(egui::CursorIcon::ResizeVertical);
                            let label = crate::i18n::text(ui.ctx(), "surfaces.analysis.resize");
                            divider.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    label.as_ref(),
                                )
                            });
                            ui.painter().hline(
                                rect.x_range(),
                                rect.center().y,
                                theme::stroke(theme::FG_MUTED),
                            );
                            crate::controls::focus_feedback(ui, &divider);
                            if divider.clicked() || divider.drag_started() {
                                divider.request_focus();
                            }
                            let drag_id = divider.id.with("height-at-press");
                            if divider.drag_started()
                                && let Some(origin) = ui.input(|input| input.pointer.press_origin())
                            {
                                ctx.data_mut(|data| {
                                    data.insert_temp(
                                        drag_id,
                                        (self.analysis.preferences.shelf_height, height, origin.y),
                                    )
                                });
                            }
                            if (divider.dragged() || divider.drag_stopped())
                                && let Some(position) = divider.interact_pointer_pos()
                                && let Some((requested, displayed, origin)) =
                                    ctx.data(|data| data.get_temp::<(f32, f32, f32)>(drag_id))
                            {
                                let delta = origin - position.y;
                                let resized = displayed + delta;
                                self.analysis.preferences.shelf_height = if delta >= 0.0 {
                                    requested.max(resized)
                                } else {
                                    resized
                                }
                                .clamp(theme::HEIGHT_ANALYSIS_MIN, theme::HEIGHT_ANALYSIS_MAX);
                                ctx.request_repaint();
                            }
                            if divider.drag_stopped() {
                                ctx.data_mut(|data| data.remove::<(f32, f32, f32)>(drag_id));
                            }
                            if divider.has_focus() {
                                ui.memory_mut(|memory| {
                                    memory.set_focus_lock_filter(
                                        divider.id,
                                        egui::EventFilter {
                                            vertical_arrows: true,
                                            ..Default::default()
                                        },
                                    )
                                });
                                let (up, down, reset) = ui.input(|input| {
                                    (
                                        input.key_pressed(egui::Key::ArrowUp),
                                        input.key_pressed(egui::Key::ArrowDown),
                                        input.key_pressed(egui::Key::Home),
                                    )
                                });
                                let delta = (up as i32 - down as i32) as f32 * 8.0;
                                if delta != 0.0 {
                                    let resized = height + delta;
                                    self.analysis.preferences.shelf_height = if delta > 0.0 {
                                        self.analysis.preferences.shelf_height.max(resized)
                                    } else {
                                        resized
                                    }
                                    .clamp(theme::HEIGHT_ANALYSIS_MIN, theme::HEIGHT_ANALYSIS_MAX);
                                    ctx.request_repaint();
                                }
                                if reset {
                                    self.analysis.preferences.shelf_height =
                                        theme::HEIGHT_ANALYSIS_DEFAULT;
                                    ctx.request_repaint();
                                }
                            }
                            if divider.double_clicked() {
                                self.analysis.preferences.shelf_height =
                                    theme::HEIGHT_ANALYSIS_DEFAULT;
                                ctx.request_repaint();
                            }
                            self.show_image_analysis(ui);
                        });
                    ctx.data_mut(|data| data.insert_temp(shelf_rect_id, shelf.response.rect));
                    if !self.analysis.visible {
                        if focused_removed_control(ctx) {
                            ctx.memory_mut(|memory| memory.request_focus(footer_id));
                        }
                        self.analysis.expanded = false;
                        self.suspend_image_analysis();
                        ctx.request_repaint();
                    }
                }
                let viewer_rect = ui.available_rect_before_wrap();
                let viewer_controls_rect = egui::Rect::from_min_size(
                    viewer_rect.min + egui::Vec2::splat(theme::SPACE_2),
                    egui::vec2(viewer_rect.width().max(1.0), theme::HEIGHT_CONTROL * 2.0),
                );
                ctx.data_mut(|data| data.insert_temp(viewer_controls_id, viewer_controls_rect));
                // Populate status after the viewer accepts composition and transition work.

                self.viewer
                    .set_display_max_size(self.params.settings.preview_max_size);
                let displayed_output = self.displayed_comparison.as_ref().map_or_else(
                    || self.output_image.clone(),
                    |comparison| comparison.image.host_image().cloned(),
                );
                self.viewer
                    .show(ui, self.oriented_input.as_ref(), displayed_output.as_ref());
                let mut status_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("viewer-status-content")
                        .max_rect(status_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                status_ui.set_clip_rect(status_rect);
                self.viewer_status(&mut status_ui);
            });
        // Accept drag-and-dropped image files.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(path) = dropped.into_iter().next() {
            self.request_photo_action(crate::photograph::PhotoAction::Open(path));
        }
        self.end_adjustment_frame(ctx);
        self.settings_dialog(ctx);
        self.factory_reset_confirmation(ctx);
        self.sync_display_controls();
        controls::end_control_frame();
        for (open, return_focus) in [
            (self.settings_open, &mut self.settings_return_focus),
            (self.export_dialog.is_open(), &mut self.export_return_focus),
        ] {
            if !open && return_focus.is_some() {
                if ctx.memory(|memory| memory.top_modal_layer().is_none()) {
                    ctx.memory_mut(|memory| memory.request_focus(return_focus.take().unwrap()));
                } else {
                    ctx.request_repaint();
                }
            }
        }
        // Accepted edits must schedule their wakeup in this frame, including
        // the last input event before the native event loop becomes idle.
        if self.dirty
            && self.image.is_some()
            && !self.input_transition_pending()
            && self.calibration_job.is_none()
            && !self.export_dialog.is_open()
            && !self.stale_save_confirmation
        {
            let now = Instant::now();
            let dirty_since = *self.dirty_since.get_or_insert(now);
            let elapsed = now.saturating_duration_since(dirty_since);
            if elapsed >= PREVIEW_DEBOUNCE {
                self.dispatch_render(ctx);

                self.dirty = false;
                self.dirty_since = None;
            } else {
                ctx.request_repaint_after(PREVIEW_DEBOUNCE - elapsed);
            }
        }
    }

    /// Cancel pending export publication and drain its worker before exiting.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_browser_state(true);
        self.persist_analysis_preferences(true);
        self.persist_clipping_preferences(true);
        self.cancel_pending_saves();
        if let Some(mut job) = self.export_job.take() {
            job.cancel.store(true, Ordering::SeqCst);
            #[cfg(test)]
            if let Some(pause) = &job.pause {
                pause.notify_cancelled();
                pause.release();
            }
            if let Some(h) = job.handle.take() {
                let _ = h.join();
            }
            if let Some(configuration) = job
                .configuration
                .as_ref()
                .and_then(|slot| slot.lock().take())
            {
                job.operation.set_render_configuration(configuration);
            }
            let _ = job.rx.try_recv();
            job.operation
                .issue(IssueCategory::Cancelled, IssueBoundary::Operation);
            self.diagnostics.finish(job.operation, Outcome::Cancelled);
        }
        if let Some(mut operation) = self.render_worker.shutdown() {
            operation.issue(IssueCategory::Cancelled, IssueBoundary::Operation);
            self.diagnostics.finish(operation, Outcome::Cancelled);
        }
        while let Ok((_, mut operation, _)) = self.render_worker.rx.try_recv() {
            operation.issue(IssueCategory::Cancelled, IssueBoundary::Operation);
            self.diagnostics.finish(operation, Outcome::Cancelled);
        }
        if let Some(mut job) = self.calibration_job.take()
            && let Some(handle) = job.handle.take()
        {
            let _ = handle.join();
        }
        self.diagnostics.shutdown();
    }
}
