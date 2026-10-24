use super::*;
use std::sync::atomic::Ordering;

struct InspectorMaterials {
    film_name: String,
    print_name: String,
    film: profile::Profile,
    print: profile::Profile,
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
        let (space, encoded) = self.operator_comparison.as_ref().map_or(
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
        let save_report = self.diagnostics.show(ui, render, queued, export);
        if save_report
            && let Some(path) = self.chooser.save_file(
                rfd::FileDialog::new()
                    .add_filter("JSON report", &["json"])
                    .set_file_name("spektrafilm-report.json"),
            )
        {
            self.status = match self.diagnostics.save_selected(&path) {
                Ok(()) => crate::app::Message::new("surfaces.status.report_saved"),
                Err(error) => {
                    crate::app::Message::new("surfaces.status.report_error").arg("error", error)
                }
            };
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
    /// default, so it states both consequences before it applies. The window is
    /// transient state on `App`: nothing is persisted, and the header status
    /// remains the one surface that reports the outcome.
    fn factory_reset_confirmation(&mut self, ctx: &egui::Context) {
        if !self.factory_reset_confirm {
            return;
        }
        let mut confirmed = false;
        let mut canceled = false;
        egui::Window::new(crate::i18n::text(ctx, "surfaces.state.restore_title"))
            .id(egui::Id::new("Restore factory default?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
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
        } else if canceled {
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
        ui.horizontal(|ui| {
            let busy = self.render_job.is_some()
                || self.export_job.is_some()
                || self.calibration_job.is_some();
            let ready = self.image.is_some() && !busy;
            if ui
                .add_enabled(
                    ready,
                    egui::Button::new(crate::i18n::text(ui.ctx(), "surfaces.action.scan"))
                        .min_size(egui::vec2(112.0, 38.0)),
                )
                .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.action.scan_help"))
                .clicked()
            {
                self.dirty = true;

                self.full_scan_requested = true;
            }
            let save = ui
                .add_enabled(
                    (self.output_image.is_some() || self.output_retained.is_some()) && !busy,
                    egui::Button::new(crate::i18n::text(ui.ctx(), "surfaces.action.save_retained"))
                        .min_size(egui::vec2(112.0, 38.0)),
                )
                .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.action.save_help"));
            controls::locate_row(ui, "action.save_retained", &save);
            if save.clicked() {
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
                self.gui_state.sections["rust"] = serde_json::json!({"version":1,"dialog_dirs":{}});
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
        self.film_name = selected;
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
                app.render_worker.cancel_active();
                app.full_scan_requested = false;
                app.image = Some(prepared.source);
                app.publish_input_viewer(view.oriented, view.raster);
                app.image_path = Some(path);
                app.output_image = None;
                app.output_retained = None;
                app.output_histogram = None;
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
                app.render_worker.cancel_active();
                app.calibration_epoch = app.calibration_epoch.wrapping_add(1);
                app.full_scan_requested = false;
                app.output_image = None;
                app.output_retained = None;
                app.output_histogram = None;
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
            let mut heading = None;
            ui.horizontal_wrapped(|ui| {
                let title = if section == "Glare" {
                    "Print viewing glare"
                } else {
                    section
                };
                let response = controls::disclosure_header(ui, title, open);
                if response.clicked() && !temporary_open {
                    ui.ctx()
                        .data_mut(|data| data.insert_persisted(disclosure_id, !normal_open));
                }
                heading = Some(response);
                self.section_reset_controls(ui, section);
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
                ui.indent("body", |ui| self.module_body(ui, section));
            } else if let Some(summary) = section_summary(section) {
                ui.label(
                    egui::RichText::new(controls::localized(ui.ctx(), summary))
                        .color(theme::FG_MUTED)
                        .small(),
                );
            }
            if let Some(materials) = ui.ctx().data(|data| {
                data.get_temp::<Arc<InspectorMaterials>>(egui::Id::new(
                    "inspector-material-context",
                ))
            }) && let Some(reason) = controls::module_applicability(
                section,
                &self.params,
                &materials.film,
                &materials.print,
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
            ui.add_space(theme::SPACE_ROW);
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
                    let changed = if film {
                        profile_combo(ui, "film", "film profile", &self.films, &mut selected)
                    } else {
                        profile_combo(ui, "paper", "print profile", &self.papers, &mut selected)
                    };
                    if let Some(response) =
                        ui.ctx()
                            .read_response(ui.make_persistent_id(egui::Id::new(if film {
                                "film"
                            } else {
                                "paper"
                            })))
                    {
                        controls::locate_row(ui, path, &response);
                    }
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
    fn section_reset_controls(&mut self, ui: &mut egui::Ui, section: &str) -> bool {
        let mut changed = false;
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
        if material_modified
            || section_reset::is_modified(section, &self.params, &self.gui_state.sections)
        {
            ui.label(
                egui::RichText::new(crate::i18n::text(ui.ctx(), "surfaces.state.modified"))
                    .color(theme::WARNING)
                    .size(theme::SIZE_HINT),
            );
        }
        if section_reset::sections().any(|name| name == section) {
            let response = controls::module_reset(ui, section);
            if response.clicked() {
                if !matches!(section, "Session" | "Display") {
                    self.label_adjustment_message(
                        crate::app::Message::new("operations.history.reset_section")
                            .message_arg("section", crate::app::Message::control(section)),
                    );
                }
                if section == "Film material" {
                    self.select_film_profile(state::GuiState::factory().film().to_owned());
                } else if section == "Print material" {
                    self.select_print_profile(state::GuiState::factory().paper().to_owned());
                }
                section_reset::reset(&mut self.params, &mut self.gui_state.sections, section);
                if section == "Workflow" {
                    self.finish_route_change();
                } else if section != "Display" {
                    self.mark_parameters_changed_invalidating_calibration();
                }
                self.search_index.reset();
                changed = true;
            }
        }
        changed
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
                    let response = ui
                        .button(&label)
                        .on_hover_text(result.localized_details(ui.ctx()));
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
                    self.presets.state.expanded = true;
                    self.pending_scroll = None;
                }
            }
        }
    }

    fn parameter_section_controls(&mut self, ui: &mut egui::Ui, section: &str) {
        let cache = self.section_labels.entry(section.to_owned()).or_default();
        let capture_reference_illuminant = &self.film_reference_illuminant;
        let data_dir = self.data_dir.clone();
        let changes = controls::show(
            ui,
            &mut self.params,
            &mut self.gui_state.sections,
            section,
            self.film_medium,
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
                spektrafilm_core::params::sources::inactive_reason(
                    path,
                    &self.params,
                    &materials.film,
                    &materials.print,
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
        let changed = ui
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
        if let Some(response) =
            ui.ctx()
                .read_response(ui.make_persistent_id(egui::Id::new(if film {
                    "film-time"
                } else {
                    "print-time"
                })))
        {
            controls::locate_row(ui, path, &response);
        }
    }

    fn controls_panel(&mut self, ui: &mut egui::Ui) {
        let input_view_before = (
            self.params.io.input_color_space.clone(),
            self.params.io.input_cctf_decoding,
        );
        let cache_id = egui::Id::new("inspector-material-context");
        let cache_current = ui
            .ctx()
            .data(|data| data.get_temp::<Arc<InspectorMaterials>>(cache_id))
            .is_some_and(|materials| {
                materials.film_name == self.film_name && materials.print_name == self.print_name
            });
        if !cache_current
            && let (Ok(film), Ok(print)) = (
                profile::load_profile_by_name(&self.data_dir, &self.film_name),
                profile::load_profile_by_name(&self.data_dir, &self.print_name),
            )
        {
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
                let open_id = ui.make_persistent_id("open");
                let normal_open = ui
                    .ctx()
                    .data_mut(|data| *data.get_persisted_mut_or_insert_with(open_id, || true));
                let filtered = !self.filter.is_empty() || self.only_modified;
                let open = normal_open || filtered || reveal_stage || has_locator;
                let response = ui.button(
                    egui::RichText::new(format!(
                        "{} {}",
                        if open { "⌄" } else { "›" },
                        controls::localized(ui.ctx(), stage.label())
                    ))
                    .strong()
                    .size(theme::SIZE_STAGE),
                );
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
                ui.separator();
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
            return;
        }
        let mut open = true;
        let mut close = false;
        let display_before = (
            self.viewer.settings.use_display_transform,
            self.params.settings.preview_max_size,
        );
        egui::Window::new(crate::i18n::text(ctx, "surfaces.settings.title"))
            .id(egui::Id::new("application-settings"))
            .open(&mut open)
            .default_width(520.0)
            .max_height((ctx.screen_rect().height() - 80.0).max(160.0))
            .show(ctx, |ui| {
                close = ui.button(crate::i18n::text(ui.ctx(), "surfaces.settings.close")).clicked();
                ui.separator();
                egui::ScrollArea::vertical().id_salt("settings-scroll").max_height((ctx.screen_rect().height() - 180.0).max(100.0)).show(ui, |ui| {
                // The inspector query locates this owner; it never filters a dialog.
                for section in ["Session", "GUI parameters", "Display", "Diagnostics"] {
                    let reveal = self.pending_scroll.as_deref() == Some(section);
                    let response = egui::CollapsingHeader::new(if section == "Session" { crate::i18n::text(ui.ctx(), "surfaces.settings.general") } else { controls::localized(ui.ctx(), section) }).id_salt(section).default_open(false).open(reveal.then_some(true)).show(ui, |ui| {
                        let _scope = controls::row_modified::install(ui.ctx(), section, &self.params, &self.gui_state.sections);
                        if self.input_transition_pending() && !matches!(section, "Display" | "Diagnostics") { ui.disable(); }
                        if matches!(section, "Session" | "Display") && self.section_reset_controls(ui, section) && section == "Display" {
                            let restored = display::DisplaySettings::from_json(&self.gui_state.sections["display"]);
                            self.viewer.settings.use_display_transform = restored.use_display_transform;
                            self.viewer.settings.gray_18_canvas = restored.gray_18_canvas;
                            self.viewer.settings.white_padding = restored.white_padding;
                            self.viewer.settings.interpolation = restored.interpolation;
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
                                let transform = ui.checkbox(&mut self.viewer.settings.use_display_transform, controls::localized(ui.ctx(), "use display transform")).on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.display.transform_help"));
                                controls::locate_row(ui, "extras.display.use_display_transform", &transform);
                                let canvas = ui.checkbox(&mut self.viewer.settings.gray_18_canvas, controls::localized(ui.ctx(), "gray 18% canvas"));
                                controls::locate_row(ui, "extras.display.gray_18_canvas", &canvas);
                                controls::param_numeric(ui, "white padding", &mut self.viewer.settings.white_padding, "extras.display.white_padding", "Viewer-only white border, as a fraction of the image long edge.");
                                self.viewer.interpolation_control(ui);
                                self.sync_display_controls();
                            }
                            "Diagnostics" => self.diagnostics_panel(ui),
                            _ => unreachable!(),
                        }
                    });
                    if self.pending_scroll.as_deref() == Some(section) {
                        if ui.ctx().data(|data| data.get_temp::<(String, String)>(egui::Id::new("control_focus_request"))).is_some() {
                            response.header_response.request_focus();
                        }
                        ui.scroll_to_rect(response.header_response.rect, Some(egui::Align::TOP));
                        self.pending_scroll = None;
                    }
                }
                });
            });
        if !open
            || close
            || ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.settings_open = false;
            if let Some(id) = self.settings_return_focus.take() {
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
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
        let cache = self.section_labels.entry(section.to_owned()).or_default();
        let changes = controls::show(
            ui,
            &mut self.params,
            &mut self.gui_state.sections,
            section,
            self.film_medium,
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

    /// One header owns global actions, read-only context, and operation status.
    fn window_header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("window-header")
            .frame(egui::Frame::default().fill(theme::SURFACE2).inner_margin(
                egui::Margin::symmetric(theme::SPACE_GROUP as i8, theme::SPACE_ROW as i8),
            ))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new("spektrafilm")
                            .strong()
                            .size(theme::SIZE_TITLE),
                    );
                    self.photograph_toolbar(ui);
                    ui.add_space(theme::SPACE_GROUP);
                    if ui
                        .button(crate::i18n::text(ui.ctx(), "surfaces.action.open"))
                        .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.action.open_help"))
                        .clicked()
                    {
                        self.open_image_dialog();
                    }
                    ui.horizontal(|ui| {
                        let undo_label = self.undo_label_localized(ui.ctx()).map_or_else(
                            || crate::i18n::text(ui.ctx(), "surfaces.history.no_undo").into_owned(),
                            |label| {
                                crate::tr!(ui.ctx(), "surfaces.history.undo", label = label)
                                    .into_owned()
                            },
                        );
                        if ui
                            .add_enabled(
                                self.can_undo() && !self.input_transition_pending(),
                                egui::Button::new(crate::i18n::text(
                                    ui.ctx(),
                                    "surfaces.action.undo",
                                )),
                            )
                            .on_hover_text(undo_label)
                            .clicked()
                        {
                            self.undo_adjustment();
                        }
                        let redo_label = self.redo_label_localized(ui.ctx()).map_or_else(
                            || crate::i18n::text(ui.ctx(), "surfaces.history.no_redo").into_owned(),
                            |label| {
                                crate::tr!(ui.ctx(), "surfaces.history.redo", label = label)
                                    .into_owned()
                            },
                        );
                        if ui
                            .add_enabled(
                                self.can_redo() && !self.input_transition_pending(),
                                egui::Button::new(crate::i18n::text(
                                    ui.ctx(),
                                    "surfaces.action.redo",
                                )),
                            )
                            .on_hover_text(redo_label)
                            .clicked()
                        {
                            self.redo_adjustment();
                        }
                    });
                    ui.add_space(theme::SPACE_GROUP);
                    let settings =
                        ui.button(crate::i18n::text(ui.ctx(), "surfaces.settings.title"));
                    if settings.is_pointer_button_down_on() {
                        crate::numeric::suspend_numeric_draft(ui.ctx());
                    }
                    if settings.clicked() {
                        crate::numeric::suspend_numeric_draft(ui.ctx());
                        self.settings_return_focus = Some(settings.id);
                        self.settings_open = true;
                    }
                    if self.export_job.is_some() {
                        if ui
                            .button(crate::i18n::text(ui.ctx(), "surfaces.export.cancel"))
                            .on_hover_text(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.export.cancel_help",
                            ))
                            .clicked()
                        {
                            self.cancel_export();
                        }
                    } else {
                        let export = ui.add_enabled(
                            self.image.is_some()
                                && self.calibration_job.is_none()
                                && !self.export_dialog.is_open(),
                            egui::Button::new(
                                egui::RichText::new(crate::i18n::text(
                                    ui.ctx(),
                                    "surfaces.action.export",
                                ))
                                .color(theme::CONTRAST_ON_ACCENT),
                            )
                            .fill(theme::ACCENT),
                        );
                        if export.clicked() {
                            self.export_return_focus = Some(export.id);
                            self.export_dialog.open(&self.export_options);
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let source = self
                        .image_path
                        .as_deref()
                        .and_then(Path::file_name)
                        .map(|name| name.to_string_lossy())
                        .unwrap_or_else(|| {
                            crate::i18n::text(ui.ctx(), "surfaces.header.no_source")
                        });
                    ui.label(crate::tr!(
                        ui.ctx(),
                        "surfaces.header.source",
                        source = source
                    ));
                    if self.photograph_has_unsaved_changes() {
                        ui.label(crate::i18n::text(ui.ctx(), "photo.record.unsaved"));
                    }
                    ui.separator();
                    ui.label(crate::tr!(
                        ui.ctx(),
                        "surfaces.header.film",
                        name = profiles::selected_display(&self.films, &self.film_name)
                    ));
                    ui.label(crate::tr!(
                        ui.ctx(),
                        "surfaces.header.print",
                        name = profiles::selected_display(&self.papers, &self.print_name)
                    ));
                    ui.separator();
                    ui.label(crate::tr!(
                        ui.ctx(),
                        "surfaces.header.route",
                        route = &self.params.workflow.route
                    ))
                    .on_hover_text(controls::localized(
                        ui.ctx(),
                        routes::vocabulary::route_description(&self.params.workflow.route),
                    ));
                });
                ui.horizontal_wrapped(|ui| {
                    let color = if self.accepted_frame_is_stale() {
                        theme::WARNING
                    } else {
                        theme::FG_SECONDARY
                    };
                    ui.label(
                        egui::RichText::new(self.retained_result_identity(ui.ctx()))
                            .color(color)
                            .size(theme::SIZE_UNIT),
                    );
                    if (self.output_image.is_some() || self.output_retained.is_some())
                        && self.retained_kind == Some(RenderKind::Preview)
                    {
                        ui.label(
                            egui::RichText::new(crate::i18n::text(
                                ui.ctx(),
                                "result.identity.preview_caveat",
                            ))
                            .small()
                            .color(theme::FG_SECONDARY),
                        );
                    }
                });
                ui.label(egui::RichText::new(self.status.localized(ui.ctx())).small());
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
        let export_was_open = self.export_dialog.is_open();
        if let Some(options) =
            self.export_dialog
                .show(ctx, export_dimensions, self.image_path.as_deref())
        {
            self.start_export(ctx, options);
        }
        if export_was_open
            && !self.export_dialog.is_open()
            && let Some(id) = self.export_return_focus.take()
        {
            ctx.memory_mut(|memory| memory.request_focus(id));
        }
        self.window_header(ctx);
        self.factory_reset_confirmation(ctx);
        self.show_stale_save_confirmation(ctx);
        self.photograph_dialogs(ctx);
        self.work_list_controls(ctx);
        self.preset_panel(ctx);
        self.sync_display_controls();
        controls::begin_control_frame(ctx, &self.params, &self.gui_state.sections);
        // Fixed histogram, search, and stage jumps above one scrolling inspector.
        egui::SidePanel::right("controls")
            .resizable(true)
            .default_width(theme::WIDTH_CONTROL_PANEL)
            .min_width(300.0)
            .max_width((ctx.screen_rect().width() * 0.48).max(300.0))
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
                if let Some(data) = self.output_histogram.as_ref() {
                    let output_id = self
                        .source_render
                        .as_ref()
                        .expect("accepted GPU output has source identity")
                        .operation_id;
                    self.histogram.update_retained(data, output_id);
                } else {
                    let output_id = self
                        .output_image
                        .as_ref()
                        .map(|o| Arc::as_ptr(o) as usize as u64)
                        .unwrap_or(0);
                    self.histogram
                        .update(&self.output_image, output_id, ui.ctx());
                }
                self.histogram.render(ui);
                ui.add_space(theme::SPACE_BLOCK);
                ui.horizontal(|ui| {
                    let clear_width = if self.filter.is_empty() {
                        0.0
                    } else {
                        82.0 + ui.spacing().item_spacing.x
                    };
                    let search_width = (ui.available_width() - clear_width).max(1.0);
                    let response = ui.add_sized(
                        [search_width, ui.spacing().interact_size.y],
                        egui::TextEdit::singleline(&mut self.filter)
                            .id(egui::Id::new("control-search"))
                            .hint_text(crate::i18n::text(ui.ctx(), "surfaces.search.label"))
                            .clip_text(true)
                            .desired_width(search_width),
                    );
                    let accessible_name = crate::i18n::text(ui.ctx(), "surfaces.search.label");
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.set_label(accessible_name)
                    });
                    if !self.filter.is_empty()
                        && ui
                            .button(crate::i18n::text(ui.ctx(), "surfaces.search.clear"))
                            .on_hover_text(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.search.clear_help",
                            ))
                            .clicked()
                    {
                        self.filter.clear();
                    }
                });
                ui.checkbox(
                    &mut self.only_modified,
                    crate::i18n::text(ui.ctx(), "surfaces.settings.modified"),
                )
                .on_hover_text(crate::i18n::text(ui.ctx(), "surfaces.search.modified_help"));
                ui.horizontal_wrapped(|ui| {
                    for stage in controls::Stage::ALL {
                        if ui
                            .button(controls::localized(ui.ctx(), stage.label()))
                            .on_hover_text(crate::i18n::text(
                                ui.ctx(),
                                "surfaces.search.stage_help",
                            ))
                            .clicked()
                        {
                            self.filter.clear();
                            self.only_modified = false;
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
                    egui::ScrollArea::vertical()
                        .id_salt((
                            "controls-scroll",
                            !self.filter.is_empty() || self.only_modified,
                        ))
                        .max_width(ui.available_width())
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.controls_panel(ui));
                    controls::install_view_filter(ui.ctx(), false);
                });
                ui.separator();
                self.simulation_action_bar(ui);
            });
        self.persist_browser_state(false);
        if self.browser_state_changed_at.is_some() {
            ctx.request_repaint_after(Duration::from_millis(300));
        }
        if let Some(error) = self.diagnostics.take_publication_error() {
            self.status =
                crate::app::Message::new("surfaces.status.publication_error").arg("error", error);
        }
        // Central viewer
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::TopBottomPanel::bottom("viewer-footer").show_inside(ui, |ui| {
                self.viewer.footer_controls(ui);
            });
            self.viewer
                .set_display_max_size(self.params.settings.preview_max_size);
            let displayed_output = self.operator_comparison.as_ref().map_or_else(
                || self.output_image.clone(),
                |comparison| comparison.image.host_image().cloned(),
            );
            let viewer_rect = ui.available_rect_before_wrap();
            self.viewer
                .show(ui, self.oriented_input.as_ref(), displayed_output.as_ref());
            if self.viewer_render_pending() {
                let text = crate::i18n::text(ui.ctx(), "surfaces.viewer.rendering");
                let font = egui::FontId::proportional(theme::SIZE_HINT);
                let galley = ui
                    .painter()
                    .layout_no_wrap(text.into_owned(), font, theme::FG);
                let inset = theme::SPACE_2;
                let size = galley.size() + egui::vec2(inset * 2.0, inset);
                let rect = egui::Rect::from_min_size(
                    viewer_rect.right_top() + egui::vec2(-size.x - inset, inset),
                    size,
                );
                ui.painter_at(viewer_rect)
                    .rect_filled(rect, theme::RADIUS_CARD, theme::SURFACE2);
                let label = ui.put(
                    rect.shrink2(egui::vec2(inset, inset * 0.5)),
                    egui::Label::new(galley).sense(egui::Sense::hover()),
                );
                controls::locate_row(ui, "viewer.rendering", &label);
            }
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
        self.sync_display_controls();
        controls::end_control_frame();
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
        self.diagnostics.clear();
    }
}
