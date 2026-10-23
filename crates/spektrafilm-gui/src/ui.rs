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
        self.viewer.set_output_display_source(
            &self.output_color_space,
            self.output_cctf_encoding,
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
                Ok(()) => "Saved diagnostic report.".into(),
                Err(error) => format!("Diagnostic report save error: {error}"),
            };
        }
    }

    fn state_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let save = ui.button("Save current to file");
            controls::locate_row(ui, "action.save_state", &save);
            if save.clicked() {
                self.save_state_with_dialog();
            }
            let load = ui.button("Load from file");
            controls::locate_row(ui, "action.load_state", &load);
            if load.clicked() {
                self.load_state_with_dialog();
            }
            let startup = ui.button("Save current as default").on_hover_text("Replace startup controls with the current state. Adjustment Undo does not reverse this file change.");
            controls::locate_row(ui, "action.save_startup", &startup);
            if startup.clicked() {
                let result = self
                    .current_state()
                    .and_then(|s| s.save(&state::default_path(&self.config_directory)));
                self.status = match result {
                    Ok(()) => "Replaced startup controls with current GUI state; adjustment Undo does not reverse this file change".into(),
                    Err(e) => format!("Startup save error: {e:#}"),
                };
            }
            let restore = ui.button("Restore factory default").on_hover_text("Replace the current controls and session settings with factory state and remove the saved startup default; confirmation required.");
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
        egui::Window::new("Restore factory default?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("Replaces the current controls and session settings with the factory state, and removes the saved startup default. Adjustment Undo does not reverse the startup-file change.");
                ui.horizontal(|ui| {
                    confirmed = ui.button("Restore").clicked();
                    canceled = ui.button("Cancel").clicked();
                });
            });
        if confirmed {
            self.factory_reset_confirm = false;
            let result =
                state::reset_factory(&self.config_directory).and_then(|s| self.apply_state(s));
            self.status = match result {
                Ok(()) => "Restored factory default GUI state".into(),
                Err(e) => format!("Factory reset error: {e:#}"),
            };
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
                Ok(()) => format!("Saved GUI state to {}", path.display()),
                Err(e) => format!("State save error: {e:#}"),
            };
        }
    }

    fn load_state_with_dialog(&mut self) {
        if let Some(path) = self
            .chooser
            .pick_file(self.file_dialog("state").add_filter("JSON", &["json"]))
        {
            let result = state::GuiState::load(&path).and_then(|s| self.apply_state(s));
            self.status = match result {
                Ok(()) => {
                    self.remember_dialog("state", &path);
                    match self.last_state_normalization.take() {
                        Some(detail) => {
                            format!("Loaded GUI state from {}; {detail}", path.display())
                        }
                        None => format!("Loaded GUI state from {}", path.display()),
                    }
                }
                Err(e) => format!("State load error: {e:#}"),
            };
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
        self.force_preview = true;
    }

    fn simulation_action_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let busy = self.render_job.is_some()
                || self.export_job.is_some()
                || self.calibration_job.is_some();
            let ready = self.image.is_some() && !busy;
            if ui
                .add_enabled(ready, egui::Button::new("PREVIEW"))
                .on_hover_text("Render the bounded preview with the current runtime controls, at the preview max size, and retain it as the viewer's output. Preview disables the spatial and stochastic effects covered by the preview contract, including lens blur, diffusion blur, physical grain, scanner blur, unsharp masking, and glare blur; Procedural and Fotufilm grain, halation, and diffusion filters remain active.")
                .clicked()
            {
                self.dirty = true;
                self.force_preview = true;
                self.full_scan_requested = false;
            }
            if ui.add_enabled(ready, egui::Button::new("SCAN")).on_hover_text("Render the original working resolution with the current runtime controls and retain it as the viewer's output. Spatial and stochastic effects follow their current accepted controls, including any explicit deactivation settings.").clicked() {
                self.dirty = true;
                self.force_preview = true;
                self.full_scan_requested = true;
            }
            if ui
                .add_enabled(
                    self.output_image.is_some() && !busy,
                    egui::Button::new("SAVE"),
                )
                .on_hover_text("Write the retained output to an image file without rerendering: the pixels are the latest Preview or Scan result, converted to the saving color space and transfer encoding. Export renders and writes independently.")
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
                self.status = format!("Dialog directory persistence error: {e}");
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
            "Workflow route changed",
        );
        self.invalidate_look_render();
    }

    pub(crate) fn scanner_normalization_detail(
        before: &spektrafilm_core::params::ScannerParams,
        after: &spektrafilm_core::params::ScannerParams,
        film_medium: controls::ScanMedium,
        route: &str,
    ) -> Option<String> {
        let mut normalized = Vec::new();
        if before.scan_output != after.scan_output {
            normalized.push("scan output");
        }
        let inactive = film_medium.for_route(route) == controls::ScanMedium::None;
        if normalized.is_empty() && !inactive {
            return None;
        }
        let mut detail = if normalized.is_empty() {
            "scanner selections retained".to_owned()
        } else {
            format!("normalized scanner controls: {}", normalized.join(", "))
        };
        if inactive {
            detail
                .push_str("; scanner controls inactive for this route; stored selections retained");
        }
        Some(detail)
    }

    pub(crate) fn normalization_status(
        before: &spektrafilm_core::params::ScannerParams,
        after: &spektrafilm_core::params::ScannerParams,
        film_medium: controls::ScanMedium,
        route: &str,
        cause: &str,
    ) -> String {
        Self::scanner_normalization_detail(before, after, film_medium, route)
            .map_or_else(|| cause.to_owned(), |detail| format!("{cause}; {detail}"))
    }

    pub(super) fn invalidate_look_render(&mut self) {
        self.scan_for_print_snapshot = None;
        self.mark_parameters_changed_invalidating_calibration();
    }

    pub(super) fn select_film_profile(&mut self, selected: String) {
        let film = match profile::load_profile_by_name(&self.data_dir, &selected) {
            Ok(profile) if profile.is_film() && profile.is_filming() => profile,
            Ok(_) => {
                self.status =
                    format!("Profile selection error: '{selected}' is not a film profile");
                return;
            }
            Err(error) => {
                self.status = format!("Profile selection error: {error:#}");
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
            "Film profile changed",
        );
        self.invalidate_look_render();
    }

    pub(super) fn select_print_profile(&mut self, selected: String) {
        let print = match profile::load_profile_by_name(&self.data_dir, &selected) {
            Ok(profile) if profile.is_print_stock() => profile,
            Ok(_) => {
                self.status =
                    format!("Profile selection error: '{selected}' is not a print profile");
                return;
            }
            Err(error) => {
                self.status = format!("Profile selection error: {error:#}");
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
        let t = Instant::now();
        let raw = image_io::is_raw(path);
        let mut raw_lens_info = None;
        let loaded = if raw {
            let settings = &self.gui_state.sections["load_raw"];
            let options = spektrafilm_raw::RawOptions {
                white_balance: match settings["white_balance"].as_str().unwrap_or("as_shot") {
                    "daylight" => spektrafilm_raw::WhiteBalance::Daylight,
                    "tungsten" => spektrafilm_raw::WhiteBalance::Tungsten,
                    "custom" => spektrafilm_raw::WhiteBalance::Custom,
                    _ => spektrafilm_raw::WhiteBalance::AsShot,
                },
                temperature: settings["temperature"].as_f64(),
                tint: settings["tint"].as_f64(),
                lens_correction: settings["lens_correction"].as_bool().unwrap_or(false),
            };
            spektrafilm_raw::load(path, &options)
                .map(|result| {
                    raw_lens_info = Some(result.lens_info);
                    LoadedImage {
                        image: result.image,
                        metadata: image_io::read_metadata(path),
                    }
                })
                .map_err(anyhow::Error::msg)
        } else {
            image_io::load(path).map_err(anyhow::Error::from)
        };
        match loaded {
            Ok(LoadedImage {
                image: img,
                metadata,
            }) => {
                if raw {
                    self.params.io.input_color_space = "ACES2065-1".into();
                    self.params.io.input_cctf_decoding = false;
                }
                self.source_metadata = metadata;
                self.input_rotation = 0;
                self.status = format!(
                    "Loaded {} × {} ({:.1} MP) in {:.0} ms",
                    img.width,
                    img.height,
                    (img.pixel_count() as f64 / 1e6),
                    t.elapsed().as_secs_f32() * 1000.0
                );
                self.raw_lens_info = raw_lens_info;
                self.raw_applied_settings =
                    raw.then(|| self.gui_state.sections["load_raw"].clone());
                if raw {
                    match self.raw_lens_info.as_deref() {
                        Some(info) if !info.is_empty() => self
                            .status
                            .push_str(&format!("; Lens correction applied ({info})")),
                        _ => self.status.push_str("; Lens correction not applied"),
                    }
                }
                self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
                self.input_epoch = self.input_epoch.wrapping_add(1);
                self.render_worker.cancel_active();
                self.full_scan_requested = false;
                self.image = Some(Arc::new(img));
                self.refresh_input_viewer();
                self.image_path = Some(path.to_path_buf());
                self.output_image = None;
                self.output_metadata = None;
                self.source_render = None;
                self.dirty = true;
                self.reset_adjustment_history();
            }
            Err(e) => {
                self.status = format!("Load error: {e:#}");
            }
        }
    }

    fn rotate_input_image(&mut self, clockwise: bool) {
        let Some(image) = self.image.as_ref() else {
            self.status = "Load an image before rotating.".into();
            return;
        };
        let quarter_turns = if clockwise { -1 } else { 1 };
        let rotated = image.rotated_quarter_turns(quarter_turns);
        self.image = Some(Arc::new(rotated));
        self.input_rotation = (self.input_rotation + quarter_turns).rem_euclid(4);
        self.input_epoch = self.input_epoch.wrapping_add(1);
        self.render_worker.cancel_active();
        self.calibration_epoch = self.calibration_epoch.wrapping_add(1);
        self.full_scan_requested = false;
        self.output_image = None;
        self.output_metadata = None;
        self.source_render = None;
        self.refresh_input_viewer();
        self.dirty = true;
        self.force_preview = true;
        self.status = format!("Rotated input {}°", self.input_rotation * 90);
    }

    /// Hand the current input image to the viewer: capped input raster, its
    /// display source, and the full-resolution source the viewer later prepares
    /// display rasters from. Every path that replaces `image` ends here, so a
    /// new handoff step cannot land in only one of them.
    pub(crate) fn refresh_input_viewer(&mut self) {
        let Some(image) = self.image.as_ref() else {
            return;
        };
        match display::input_display_raster(
            image,
            &self.params.io.input_color_space,
            self.params.io.input_cctf_decoding,
            self.params.settings.preview_max_size as usize,
        ) {
            Ok(raster) => {
                self.viewer
                    .set_input(raster, [image.width as usize, image.height as usize]);
                self.viewer.set_input_display_source(
                    &self.params.io.input_color_space,
                    self.params.io.input_cctf_decoding,
                );
                self.viewer
                    .seed_prepared_input(image.clone(), self.params.settings.preview_max_size);
            }
            Err(e) => self.status = format!("Viewer input error: {e}"),
        }
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
                let response = ui.button(format!("{} {title}", if open { "v" } else { ">" }));
                ui.ctx()
                    .accesskit_node_builder(response.id, |node| node.set_label(title));
                if response.clicked() && !temporary_open {
                    ui.ctx()
                        .data_mut(|data| data.insert_persisted(disclosure_id, !normal_open));
                }
                heading = Some(response);
                self.section_reset_controls(ui, section);
                self.section_preset_menu(
                    ui,
                    if section == "Output" {
                        "Simulation blend"
                    } else {
                        section
                    },
                );
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
                ui.label(egui::RichText::new(summary).color(theme::FG_MUTED).small());
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
                    egui::RichText::new(format!(
                        "Inactive controls · {reason} · stored values retained"
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
                    routes::tooltip(),
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
                    let reset_label = if film {
                        "Reset film profile to factory"
                    } else {
                        "Reset print profile to factory"
                    };
                    if ui.small_button(reset_label).clicked() {
                        let factory = state::GuiState::factory();
                        self.label_adjustment(reset_label);
                        if film {
                            self.select_film_profile(factory.film().to_owned());
                        } else {
                            self.select_print_profile(factory.paper().to_owned());
                        }
                    }
                }
                self.chemistry_dev_time(ui, film);
            }
            "Source rotation" => {
                ui.horizontal_wrapped(|ui| {
                    let ccw = ui.add_enabled(self.image.is_some(), egui::Button::new("ccw rotate")).on_hover_text("Rotate the retained source 90° counter-clockwise. Preview, Scan, and Export render this source.");
                    controls::locate_row(ui, "action.rotate_ccw", &ccw);
                    if ccw.clicked() {
                        self.label_adjustment("Rotate source counter-clockwise");
                        self.rotate_input_image_counterclockwise();
                    }
                    let cw = ui.add_enabled(self.image.is_some(), egui::Button::new("cw rotate")).on_hover_text("Rotate the retained source 90° clockwise; this is not a viewer transform.");
                    controls::locate_row(ui, "action.rotate_cw", &cw);
                    if cw.clicked() {
                        self.label_adjustment("Rotate source clockwise");
                        self.rotate_input_image_clockwise();
                    }
                    ui.label(format!("Source rotation: {}°", self.input_rotation * 90));
                });
            }
            "Import Raw" => {
                self.parameter_section_controls(ui, section);
                let raw = self.image_path.as_deref().is_some_and(image_io::is_raw);
                if raw {
                    if self.raw_applied_settings.as_ref()
                        != Some(&self.gui_state.sections["load_raw"])
                    {
                        ui.weak("RAW decode settings pending; the accepted source still uses the previous settings.");
                    } else {
                        ui.weak("RAW decode settings match the accepted source. Camera exposure is a separate simulation control.");
                    }
                } else {
                    ui.weak("RAW decoding is inactive for this source. Settings are retained for the next RAW import.");
                }
                let reprocess = ui.add_enabled(raw, egui::Button::new("Reprocess RAW"));
                controls::locate_row(ui, "action.reprocess_raw", &reprocess);
                if reprocess.clicked()
                    && let Some(path) = self.image_path.clone()
                {
                    self.load_image_from_path(&path);
                }
            }
            "Scanner" => {
                let mut scan_for_print = self.scan_for_print_snapshot.is_some();
                let response = ui.checkbox(&mut scan_for_print, "scan for print").on_hover_text("Disable print viewing glare for a print scan; toggle again to restore its previous state.");
                controls::locate_row(ui, "action.scan_for_print", &response);
                if response.changed() {
                    self.label_adjustment("Toggle scan for print");
                    self.toggle_scan_for_print();
                }
                self.parameter_section_controls(ui, section);
            }
            _ => self.parameter_section_controls(ui, section),
        }
        if controls::apply_pending_resets(ui.ctx(), &mut self.params, &mut self.gui_state.sections)
        {
            self.label_adjustment("Reset control to factory");
            if section == "Workflow" {
                self.finish_route_change();
            } else {
                self.mark_parameters_changed_invalidating_calibration();
            }
        }
    }

    /// The `modified` chip and the reset action of one section header.
    ///
    /// Every resettable section presents these on its header, whether or not
    /// it also carries a preset menu, so the reset scope the section table
    /// declares is reachable from the surface that renders the section.
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
                egui::RichText::new("modified")
                    .color(theme::WARNING)
                    .size(theme::SIZE_HINT),
            );
        }
        if section_reset::sections().any(|name| name == section) {
            let response = ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("reset")
                            .color(theme::FG_MUTED)
                            .size(theme::SIZE_HINT),
                    )
                    .corner_radius(theme::RADIUS_CARD),
                )
                .on_hover_text(format!("Restore {section} to the factory state"));
            // Name the action after its section, so one reset is
            // addressable among the sections on a page.
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_label(format!("reset {section}"));
            });
            if response.clicked() {
                self.label_adjustment(&format!("Reset {section} to factory"));
                if section == "Film material" {
                    self.select_film_profile(state::GuiState::factory().film().to_owned());
                } else if section == "Print material" {
                    self.select_print_profile(state::GuiState::factory().paper().to_owned());
                }
                section_reset::reset(&mut self.params, &mut self.gui_state.sections, section);
                if section == "Workflow" {
                    self.finish_route_change();
                } else {
                    self.mark_parameters_changed_invalidating_calibration();
                }
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
                "No controls differ from the factory baseline.".to_owned()
            } else if self.only_modified {
                format!("No modified controls match “{}”.", self.filter)
            } else {
                format!("No control matches “{}”.", self.filter)
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
                    let label = result.locator_label();
                    let response = ui.button(&label).on_hover_text(result.details());
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
                    self.settings_open = true;
                    self.settings_return_focus = Some(egui::Id::new("control-search"));
                    self.pending_scroll = Some(section.to_owned());
                }
                controls::ControlOwner::Export => {
                    self.export_return_focus = Some(egui::Id::new("control-search"));
                    self.export_dialog.open(&self.export_options);
                }
                controls::ControlOwner::Viewer => {
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
        if let Some(action) = changes.action {
            self.start_calibration(action);
        }
        if changes.preview_requested {
            self.dirty = true;
            self.force_preview = true;
        }
    }

    fn display_profile_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let select = ui.button("select ICC profile…").on_hover_text("Choose an ICC profile for the display transform applied to the viewer raster only. It never reaches retained, saved or exported pixels.");
            controls::locate_row(ui, "extras.rust.display_profile", &select);
            if select.clicked()
                && let Some(path) = self.chooser.pick_file(
                    self.file_dialog("display")
                        .add_filter("ICC profile", &["icc", "icm"]),
                )
            {
                self.gui_state.set_display_profile(path.to_str());
                self.refresh_viewing_artifacts();
                if lcms2::Profile::new_file(&path).is_err() {
                    self.viewer.transform_status =
                        "Display transform: ICC failed; using the sRGB preview; viewing sRGB preview".into();
                }
                self.status = format!("Display ICC profile: {}", path.display());
            }
            if self.gui_state.display_profile().is_some()
                && ui
                    .button("clear")
                    .on_hover_text("Stop using the selected ICC profile and fall back to the platform profile or sRGB. The retained, saved and exported pixels never used the profile, so they are unchanged.")
                    .clicked()
            {
                self.gui_state.set_display_profile(None);
                self.refresh_viewing_artifacts();
                self.status = "Display ICC profile cleared".into();
            }
        });
        if let Some(profile) = self.gui_state.display_profile() {
            ui.label(format!("Display ICC: {profile}"));
        } else {
            ui.label("Display ICC: platform profile or sRGB fallback");
        }
        ui.label(self.viewer.transform_status.clone());
        // The display transform and its profile are viewer-only: they change
        // what the viewer shows and nothing the pipeline retains or writes.
        ui.label(
            egui::RichText::new(
                "Viewer only: the display transform and its ICC profile never reach retained, saved or exported pixels.",
            )
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
            ui.label(format!(
                "Development time: {effective} min · material-defined (no selectable family)"
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
            ui.weak(format!(
                "Development time inactive: {reason}; stored selection retained."
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
        controls::row_modified::offer_reset(
            ui,
            "development time",
            Some(if film {
                "film_render.development_time"
            } else {
                "print_render.development_time"
            }),
        );
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
                        if open { "v" } else { ">" },
                        stage.label()
                    ))
                    .strong()
                    .size(theme::SIZE_TITLE),
                );
                ui.ctx().accesskit_node_builder(response.id, |node| {
                    node.set_label(format!("{} stage", stage.label()))
                });
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
        egui::Window::new("Settings")
            .id(egui::Id::new("application-settings"))
            .open(&mut open)
            .default_width(520.0)
            .max_height((ctx.screen_rect().height() - 80.0).max(160.0))
            .show(ctx, |ui| {
                close = ui.button("Close settings").clicked();
                ui.separator();
                egui::ScrollArea::vertical().id_salt("settings-scroll").max_height((ctx.screen_rect().height() - 180.0).max(100.0)).show(ui, |ui| {
                // The inspector query locates this owner; it never filters a dialog.
                for section in ["Session", "GUI parameters", "Display", "Diagnostics"] {
                    let reveal = self.pending_scroll.as_deref() == Some(section);
                    let response = egui::CollapsingHeader::new(section).default_open(false).open(reveal.then_some(true)).show(ui, |ui| {
                        let _scope = controls::row_modified::install(ui.ctx(), section, &self.params, &self.gui_state.sections);
                        match section {
                            "Session" => {
                                controls::extra_bool_tip(ui, &mut self.gui_state.sections, "simulation", "auto_preview", "auto preview", true, "Schedule a bounded Preview after accepted photographic adjustments.");
                                self.settings_parameter_controls(ui, "Session");
                            }
                            "GUI parameters" => self.state_toolbar(ui),
                            "Display" => {
                                self.display_profile_controls(ui);
                                let transform = ui.checkbox(&mut self.viewer.settings.use_display_transform, "use display transform").on_hover_text("Viewer-only display conversion; does not change retained or written pixels.");
                                controls::locate_row(ui, "extras.display.use_display_transform", &transform);
                                if ui.small_button("Reset use display transform to factory").clicked() {
                                    self.viewer.settings.use_display_transform = display::DisplaySettings::from_json(&state::GuiState::factory().sections["display"]).use_display_transform;
                                }
                                let canvas = ui.checkbox(&mut self.viewer.settings.gray_18_canvas, "gray 18% canvas");
                                controls::locate_row(ui, "extras.display.gray_18_canvas", &canvas);
                                if ui.small_button("Reset gray 18% canvas to factory").clicked() {
                                    self.viewer.settings.gray_18_canvas = display::DisplaySettings::from_json(&state::GuiState::factory().sections["display"]).gray_18_canvas;
                                }
                                controls::param_numeric(ui, "white padding", &mut self.viewer.settings.white_padding, "extras.display.white_padding", "Viewer-only white border, as a fraction of the image long edge.");
                                self.viewer.interpolation_control(ui);
                                if ui.small_button("Reset output interpolation to factory").clicked() {
                                    self.viewer.settings.interpolation = display::DisplaySettings::from_json(&state::GuiState::factory().sections["display"]).interpolation;
                                }
                                self.sync_display_controls();
                                if controls::apply_pending_resets(ui.ctx(), &mut self.params, &mut self.gui_state.sections) {
                                    let restored = display::DisplaySettings::from_json(&self.gui_state.sections["display"]);
                                    self.viewer.settings.use_display_transform = restored.use_display_transform;
                                    self.viewer.settings.gray_18_canvas = restored.gray_18_canvas;
                                    self.viewer.settings.white_padding = restored.white_padding;
                                    self.viewer.settings.interpolation = restored.interpolation;
                                    self.search_index.reset();
                                }
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
        if changes.preview_requested {
            self.dirty = true;
            self.force_preview = true;
        }
    }

    fn open_image_dialog(&mut self) {
        if let Some(path) = self.chooser.pick_file(
            self.file_dialog("load")
                .add_filter("Image", &image_io::accepted_extensions()),
        ) {
            self.remember_dialog("load", &path);
            self.load_image_from_path(&path);
        }
    }

    /// One header owns global actions, read-only context, and operation status.
    fn window_header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("window-header")
            .frame(egui::Frame::default().fill(theme::SURFACE2).inner_margin(egui::Margin::symmetric(theme::SPACE_GROUP as i8, theme::SPACE_ROW as i8)))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("spektrafilm").strong());
                    if ui.button("Open").on_hover_text("Open image — RGB and supported RAW formats use their existing import paths.").clicked() { self.open_image_dialog(); }
                    let undo_label = self.undo_label().map_or_else(|| "No adjustment to undo".to_owned(), |label| format!("Undo {label}"));
                    if ui.add_enabled(self.can_undo(), egui::Button::new("Undo")).on_hover_text(undo_label).clicked() { self.undo_adjustment(); }
                    let redo_label = self.redo_label().map_or_else(|| "No adjustment to redo".to_owned(), |label| format!("Redo {label}"));
                    if ui.add_enabled(self.can_redo(), egui::Button::new("Redo")).on_hover_text(redo_label).clicked() { self.redo_adjustment(); }
                    let settings = ui.button("Settings");
                    if settings.clicked() {
                        self.settings_return_focus = Some(settings.id);
                        self.settings_open = true;
                    }
                    if self.export_job.is_some() {
                        if ui.button("Cancel export").on_hover_text("Cancel using the export worker; an output not yet published is discarded.").clicked() { self.cancel_export(); }
                    } else {
                        let export = ui.add_enabled(self.image.is_some() && self.calibration_job.is_none() && !self.export_dialog.is_open(), egui::Button::new("Export"));
                        if export.clicked() {
                            self.export_return_focus = Some(export.id);
                            self.export_dialog.open(&self.export_options);
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let source = self.image_path.as_deref().and_then(Path::file_name).map(|name| name.to_string_lossy()).unwrap_or_else(|| "No image loaded".into());
                    ui.label(format!("Source: {source}"));
                    ui.label(format!("Film: {}", profiles::selected_display(&self.films, &self.film_name)));
                    ui.label(format!("Print: {}", profiles::selected_display(&self.papers, &self.print_name)));
                    ui.label(format!("Route: {}", self.params.workflow.route)).on_hover_text(routes::vocabulary::route_description(&self.params.workflow.route));
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(self.operation_status()).small());
                    if self.accepted_frame_is_stale() {
                        ui.label(egui::RichText::new("Retained frame is not current · controls changed").color(theme::WARNING).small());
                    }
                });
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.render_progress.bind_context(ctx);
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

        if self.dirty
            && self.image.is_some()
            && self.calibration_job.is_none()
            && !self.export_dialog.is_open()
            && (self.gui_state.auto_preview() || self.force_preview)
        {
            let now = Instant::now();
            let dirty_since = *self.dirty_since.get_or_insert(now);
            let elapsed = now.saturating_duration_since(dirty_since);
            if elapsed >= PREVIEW_DEBOUNCE {
                self.dispatch_render(ctx);
                self.force_preview = false;
                self.dirty = false;
                self.dirty_since = None;
            } else {
                ctx.request_repaint_after(PREVIEW_DEBOUNCE - elapsed);
            }
        }
        self.begin_adjustment_frame(ctx);
        self.sync_display_controls();
        controls::begin_control_frame(ctx, &self.params, &self.gui_state.sections);
        self.poll_render_job(ctx);
        self.poll_calibration_job(ctx);
        self.poll_export_job(ctx);
        let editing_text = ctx
            .memory(|memory| memory.focused())
            .is_some_and(|id| egui::text_edit::TextEditState::load(ctx, id).is_some());
        if !editing_text && !self.settings_open && !self.export_dialog.is_open() {
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
        // The export renders the retained input, so the dialog's estimate
        // reads that image's dimensions while the dialog is open.
        let export_dimensions = self
            .image
            .as_ref()
            .map(|image| [image.width, image.height])
            .unwrap_or([0, 0]);
        let export_was_open = self.export_dialog.is_open();
        if let Some(options) = self.export_dialog.show(ctx, export_dimensions) {
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
        self.preset_panel(ctx);
        // Fixed histogram, search, and stage jumps above one scrolling inspector.
        egui::SidePanel::right("controls")
            .resizable(true)
            .default_width(theme::WIDTH_CONTROL_PANEL)
            .min_width(300.0)
            .max_width((ctx.screen_rect().width() * 0.48).max(300.0))
            .show(ctx, |ui| {
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
                let output_id = self
                    .output_image
                    .as_ref()
                    .map(|o| Arc::as_ptr(o) as usize as u64)
                    .unwrap_or(0);
                self.histogram
                    .update(&self.output_image, output_id, ui.ctx());
                self.histogram.render(ui);
                ui.add_space(theme::SPACE_BLOCK);
                ui.horizontal(|ui| {
                    let clear_width = if self.filter.is_empty() { 0.0 } else { 82.0 + ui.spacing().item_spacing.x };
                    let search_width = (ui.available_width() - clear_width).max(1.0);
                    let response = ui.add_sized(
                        [search_width, ui.spacing().interact_size.y],
                        egui::TextEdit::singleline(&mut self.filter)
                            .id(egui::Id::new("control-search"))
                            .hint_text("Search controls")
                            .clip_text(true)
                            .desired_width(search_width),
                    );
                    ui.ctx().accesskit_node_builder(response.id, |node| node.set_label("Search controls"));
                    if !self.filter.is_empty() && ui.button("clear filter").on_hover_text("Clear search and restore ordinary inspector disclosure.").clicked() {
                        self.filter.clear();
                    }
                });
                ui.checkbox(&mut self.only_modified, "Only modified").on_hover_text("Show controls that differ from their factory baseline, independently of an applied preset.");
                ui.horizontal_wrapped(|ui| {
                    for stage in controls::Stage::ALL {
                        if ui.button(stage.label()).on_hover_text("Reveal and scroll to this stage in the continuous inspector.").clicked() {
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
                    egui::ScrollArea::both()
                        .id_salt(("controls-scroll", !self.filter.is_empty() || self.only_modified))
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
            self.status = format!("Diagnostic report publication error: {error}");
        }
        // Central viewer
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::TopBottomPanel::bottom("viewer-footer").show_inside(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    self.viewer.layer_controls(ui);
                    ui.checkbox(&mut self.viewer.settings.reveal, "reveal transition").on_hover_text("Viewer-only wipe between input and output; retained and written pixels are unchanged.");
                    ui.checkbox(&mut self.viewer.settings.crossfade, "crossfade transition").on_hover_text("Viewer-only transition between accepted outputs.");
                    // View: zoom presets and the fit reset. Exact percentages
                    // are a source-pixel-to-device-pixel scale, so 100% means
                    // one source pixel per device pixel.
                    for (label, percent) in [("100%", 100.0), ("200%", 200.0), ("400%", 400.0)] {
                        let response = ui.button(label).on_hover_text(format!("Show the retained image at {percent:.0}% — one source pixel per device pixel at 100%. Viewer only; retained and written pixels are unchanged."));
                        let path = match label { "100%" => "viewer.zoom.100", "200%" => "viewer.zoom.200", _ => "viewer.zoom.400" };
                        controls::locate_row(ui, path, &response);
                        if response.clicked() {
                            self.viewer.set_zoom_percent(percent);
                            ui.ctx().request_repaint();
                        }
                    }
                    let fit = ui.button("fit").on_hover_text("Fit the retained image in the viewer, leaving pan unchanged. Viewer only.");
                    controls::locate_row(ui, "viewer.fit", &fit);
                    if fit.clicked() {
                        self.viewer.fit_view();
                        ui.ctx().request_repaint();
                    }
                    let reset = ui.button("reset view").on_hover_text("Fit and recenter the viewer. Retained and written pixels are unchanged.");
                    controls::locate_row(ui, "viewer.reset", &reset);
                    if reset.clicked() {
                        self.viewer.reset_view();
                        ui.ctx().request_repaint();
                    }
                    ui.separator();
                    // Comparison: the before/after toggle and its split.
                    let compare_label = if self.compare_active {
                        "compare ON"
                    } else {
                        "compare"
                    };
                    let compare = ui.selectable_label(self.compare_active, compare_label).on_hover_text("Compare retained input and output side by side; the split is viewer-only.");
                    controls::locate_row(ui, "viewer.compare", &compare);
                    if compare.clicked() {
                        self.compare_active = !self.compare_active;
                        ui.ctx().request_repaint();
                    }
                    ui.add_space(theme::SPACE_GROUP);
                    {
                        let slider = ui.add_enabled(
                            self.compare_active,
                            egui::Slider::new(&mut self.compare_split, 0.0..=1.0)
                                .fixed_decimals(2)
                                .trailing_fill(false),
                        );
                        ui.ctx().accesskit_node_builder(slider.id, |node| node.set_label("comparison split"));
                        controls::locate_row(ui, "viewer.compare_split", &slider);
                        if slider.changed() {
                            ui.ctx().request_repaint();
                        }
                        ui.label(
                            egui::RichText::new(format!(
                                "←input {:.0}% | {:.0}%output→",
                                self.compare_split * 100.0,
                                (1.0 - self.compare_split) * 100.0
                            ))
                            .color(theme::FG_MUTED)
                            .size(theme::SIZE_HINT),
                        )
                        .on_hover_text("Where the before and after halves meet. The retained input stays left of the split and the retained output right of it; neither half reaches saved or exported pixels.");
                        if !self.compare_active {
                            ui.label(egui::RichText::new("Comparison split inactive · enable compare; stored split retained.").color(theme::FG_MUTED).small());
                        }
                    }
                    ui.separator();
                    // Read-only: the current zoom.
                    let zoom = self.viewer.zoom_percent().map_or_else(
                        || format!("{:.0}% fit", self.viewer.zoom * 100.0),
                        |percent| format!("{percent:.0}%"),
                    );
                    ui.label(
                        egui::RichText::new(format!("zoom {zoom}"))
                            .color(theme::FG_MUTED)
                            .size(theme::SIZE_HINT),
                    )
                    .on_hover_text("Read-only zoom status: the fitted zoom, or the exact source-pixel-to-device-pixel percentage. Viewer presentation only.");
                });
            });
            self.viewer
                .set_display_max_size(self.params.settings.preview_max_size);
            self.viewer.set_compare_split(if self.compare_active {
                Some(self.compare_split)
            } else {
                None
            });
            self.viewer
                .show(ui, self.image.as_ref(), self.output_image.as_ref());
            // Before/after compare divider overlay
            if self.compare_active {
                let rect = ui.max_rect();
                let painter = ui.painter_at(rect);
                let x = rect.left() + rect.width() * self.compare_split;
                // Divider line
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    egui::Stroke::new(2.0_f32, theme::ACCENT),
                );
                // Labels
                painter.text(
                    egui::pos2(x - 8.0, rect.top() + 12.0),
                    egui::Align2::RIGHT_TOP,
                    "input",
                    egui::FontId::proportional(10.0),
                    theme::ACCENT,
                );
                painter.text(
                    egui::pos2(x + 8.0, rect.top() + 12.0),
                    egui::Align2::LEFT_TOP,
                    "output",
                    egui::FontId::proportional(10.0),
                    theme::ACCENT,
                );
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
            self.load_image_from_path(&path);
        }
        self.end_adjustment_frame(ctx);
        self.settings_dialog(ctx);
        self.sync_display_controls();
        controls::end_control_frame();
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
