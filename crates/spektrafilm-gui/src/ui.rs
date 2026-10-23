use super::*;
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum GuiTab {
    #[default]
    Main,
    Film,
    Print,
    Advanced,
    Config,
}

impl GuiTab {
    const ALL: [Self; 5] = [
        Self::Main,
        Self::Film,
        Self::Print,
        Self::Advanced,
        Self::Config,
    ];

    /// One-line description of what the page holds.
    pub(super) fn description(self) -> &'static str {
        match self {
            Self::Main => "Import, workflow, and film stock",
            Self::Film => "Film negative response and material controls",
            Self::Print => "Print response and finishing controls",
            Self::Advanced => "Scanner, gamut, and experimental controls",
            Self::Config => "GUI and display settings",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Main => "MAIN",
            Self::Film => "FILM",
            Self::Print => "PRINT",
            Self::Advanced => "ADVANCED",
            Self::Config => "CONFIG",
        }
    }
}
fn section_summary(section: &str) -> Option<&'static str> {
    match section {
        "Film base" => Some("Base density: scale, tilt, and per-channel masks"),
        "Halation" => Some("Highlight glow: halation, in-emulsion scatter, boost"),
        "Couplers" => Some("DIR couplers: inhibition and interlayer response"),
        "Convert" => Some("Convert-film calibration: base, matrix, controller actions"),
        "Camera diffusion" => Some("Lens diffusion: core, tail, and warmth"),
        "Print base" => Some("Base density: scale and per-channel masks"),
        "Glare" => Some("Print viewing glare: percent, roughness, blur"),
        "Enlarger diffusion" => Some("Enlarger diffusion: core, tail, and warmth"),
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
            if ui.button("Save current to file").clicked() {
                self.save_state_with_dialog();
            }
            if ui.button("Load from file").clicked() {
                self.load_state_with_dialog();
            }
            if ui
                .button("Save current as default")
                .on_hover_text("Replace the startup GUI state with the current controls. The current session keeps its settings.")
                .clicked()
            {
                let result = self
                    .current_state()
                    .and_then(|s| s.save(&state::default_path(&self.config_directory)));
                self.status = match result {
                    Ok(()) => "Saved current GUI state as startup default".into(),
                    Err(e) => format!("Startup save error: {e:#}"),
                };
            }
            if ui
                .button("Restore factory default")
                .on_hover_text(
                    "Replace the current controls and session settings with the factory state and remove the saved startup default.",
                )
                .clicked()
            {
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
                ui.label("Replaces the current controls and session settings with the factory state, and removes the saved startup default.");
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
        ui.horizontal_wrapped(|ui| {
            controls::extra_bool_tip(ui, &mut self.gui_state.sections, "simulation", "auto_preview", "auto preview", true, "trigger the preview after every change of gui parameters, use mouse scrollwheel on parameters field, read preview tooltip for details");
            let mut scan_for_print = self.scan_for_print_snapshot.is_some();
            if ui
                .checkbox(&mut scan_for_print, "scan for print")
                .on_hover_text(
                    "Disables print viewing glare for the print scan, as a scan of the \
                     paper record has no viewing enclosure. Toggle it again to restore the \
                     previous value.",
                )
                .changed()
            {
                self.toggle_scan_for_print();
            }
        });
        // The workflow selector's own row: its selected text is long, and a
        // checkbox sharing the row would be pushed past the panel.
        ui.horizontal(|ui| {
            if controls::choice_tip_entries(
                ui,
                "workflow",
                &mut self.params.workflow.route,
                routes::entries(),
                routes::tooltip(),
            ) {
                self.finish_route_change();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut self.params.cmyk.active, "CMYK ink style")
                .on_hover_text("Apply the CMYK ink style appearance after the selected workflow. The result remains three-channel RGB; it is not a press proof or a separation.")
                .changed()
            {
                self.mark_parameters_changed();
            }
        });
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
        if let Some(error) = self.last_export_error.clone() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(error).color(theme::ERROR).small());
                if ui.button("Dismiss").clicked() {
                    self.last_export_error = None;
                }
            });
        }
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
    fn refresh_input_viewer(&mut self) {
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
        let reveal = self.pending_scroll.as_deref() == Some(section);
        if reveal {
            self.pending_scroll = None;
        }
        // Sections that own their own header rather than rendering one inside
        // the controls, with the state the header opens in.
        let collapsible = match section {
            "Film base" => Some(("Base", false)),
            "Grain" => Some(("Grain", true)),
            "Halation" => Some(("Halation", false)),
            "Couplers" => Some(("Couplers", false)),
            "Convert" => Some(("Convert", false)),
            "Camera" => Some(("Camera", true)),
            "Camera diffusion" => Some(("Diffusion", false)),
            "Enlarger diffusion" => Some(("Diffusion", false)),
            "Enlarger" => Some(("Enlarger", true)),
            "Scanner" => Some(("Scanner", false)),
            "Output" => Some(("Output", true)),
            _ => None,
        };
        if !self.filter.is_empty() {
            // Search mode: reveal the matching controls of a section that has
            // any, and drop a section that has none.
            if !self.section_has_filter_match(section) {
                return;
            }
            if let Some((label, _)) = collapsible {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(label)
                            .color(theme::FG)
                            .size(theme::SIZE_TITLE)
                            .strong(),
                    );
                    self.section_reset_controls(ui, section);
                });
            }
            self.parameter_section_controls(ui, section);
            if reveal {
                ui.scroll_to_rect(ui.min_rect(), Some(egui::Align::Center));
            }
            return;
        }
        if let Some((label, default_open)) = collapsible {
            let mut label_clicked = false;
            let mut header = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                ui.make_persistent_id(("section", section)),
                default_open || reveal,
            )
            .show_header(ui, |ui| {
                label_clicked = ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new(label)
                                .color(theme::FG)
                                .size(theme::SIZE_TITLE)
                                .strong(),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .clicked();
                self.section_reset_controls(ui, section);
            });
            if reveal {
                header.set_open(true);
            }
            if label_clicked {
                header.toggle();
            }
            let closed = !header.is_open();
            header.body(|ui| self.parameter_section_controls(ui, section));
            if closed && let Some(summary) = section_summary(section) {
                ui.label(egui::RichText::new(summary).color(theme::FG_MUTED).small());
            }
            if reveal {
                ui.scroll_to_rect(ui.min_rect(), Some(egui::Align::Center));
            }
        } else {
            // A section without its own header renders one inside the controls,
            // so its reset scope rides on a row above them.
            if section_reset::sections().any(|name| name == section) {
                ui.horizontal(|ui| {
                    self.section_reset_controls(ui, section);
                });
            }
            self.parameter_section_controls(ui, section);
            if reveal {
                ui.scroll_to_rect(ui.min_rect(), Some(egui::Align::Center));
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
        if section_reset::is_modified(section, &self.params, &self.gui_state.sections) {
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
                section_reset::reset(&mut self.params, &mut self.gui_state.sections, section);
                self.mark_parameters_changed_invalidating_calibration();
                changed = true;
            }
        }
        changed
    }

    /// Whether `section` rendered a control whose label matches the search
    /// filter.
    fn section_has_filter_match(&self, section: &str) -> bool {
        controls::cached_labels_match(&self.filter, self.section_labels.get(section))
    }

    fn search_results(&mut self, ui: &mut egui::Ui) {
        if self.filter.is_empty() {
            return;
        }
        let extras = self.gui_state.sections.clone();
        let params = self.params.clone();
        let data_dir = self.data_dir.clone();
        let results = self.search_index.results(
            &self.filter,
            &params,
            &extras,
            self.film_medium,
            self.parameter_revision,
            &data_dir,
        );
        if results.is_empty() {
            ui.label(
                egui::RichText::new(format!("No control matches “{}”.", self.filter))
                    .color(theme::FG_MUTED)
                    .small(),
            );
            return;
        }
        if !results.iter().any(|result| result.page == self.gui_tab) {
            ui.label(
                egui::RichText::new(format!("No control on {} matches.", self.gui_tab.label()))
                    .color(theme::FG_MUTED)
                    .small(),
            );
        }
        let mut destination = None;
        for result in results {
            if result.page == self.gui_tab {
                continue;
            }
            let summary = result.summary();
            let response = ui
                .button(format!(
                    "{} · {}: {}",
                    result.page.label(),
                    result.section,
                    summary
                ))
                .on_hover_text(summary);
            let result_label = format!("{} · {}", result.page.label(), result.section);
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_label(result_label);
            });
            if response.clicked() {
                destination = Some((result.page, result.section));
            }
        }
        if let Some((page, section)) = destination {
            self.gui_tab = page;
            self.pending_scroll = Some(section.to_owned());
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
        if changes.preview_requested {
            self.dirty = true;
            self.force_preview = true;
        }
        if changes.raw_reload
            && let Some(path) = self.image_path.clone()
        {
            self.load_image_from_path(&path);
        }
    }

    fn import_section(&mut self, ui: &mut egui::Ui, raw: bool) {
        ui.collapsing(if raw { "Import Raw" } else { "Import RGB" }, |ui| {
            let label = if raw {
                "Select RAW file…"
            } else {
                "Select RGB image…"
            };
            if ui
                .button(label)
                .on_hover_text(if raw {
                    "Choose and load a camera RAW file through the RAW path. The raw white balance, temperature, tint and lens correction settings apply to the loaded input before the workflow renders it."
                } else {
                    "Choose and load a standard raster image. The image becomes the retained input the workflow renders."
                })
                .clicked()
                && let Some(path) = self.chooser.pick_file(
                    self.file_dialog("load").add_filter("Image", &image_io::accepted_extensions()),
                )
            {
                self.remember_dialog("load", &path);
                self.load_image_from_path(&path);
            }
            if raw {
                self.parameter_section(ui, "Import Raw");
            }
            if let Some(p) = &self.image_path {
                ui.label(egui::RichText::new(p.file_name().and_then(|s| s.to_str()).unwrap_or("" )).small());
            }
            ui.add_space(theme::SPACE_BLOCK);
        });
    }

    fn display_profile_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("select ICC profile…").on_hover_text("Choose an ICC profile for the display transform applied to the viewer raster only. It never reaches the retained, saved or exported pixels, which use their own saving color space and transfer encoding.").clicked()
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

    fn export_actions(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Export options")
            .default_open(false)
            .show(ui, |ui| {
                if self.export_job.is_some() {
                    if ui.button("Cancel")
                        .on_hover_text("Cancel export; the current operation finishes before its output is discarded.")
                        .clicked()
                    {
                        self.cancel_export();
                    }
                } else {
                    let enabled = self.image.is_some()
                        && self.calibration_job.is_none()
                        && !self.export_dialog.is_open();
                    if ui.add_enabled(enabled, egui::Button::new("Open export settings…"))
                        .on_hover_text("Open the independent export settings: format, bit depth, compression, saving color space and saving transfer encoding. These settings belong to Export only and do not change Save, which writes the retained output with the current simulation saving settings.")
                        .clicked()
                    {
                        self.export_dialog.open(&self.export_options);
                    }
                }
            });
    }
    fn chemistry_section(&mut self, ui: &mut egui::Ui, film: bool) {
        let section = if film {
            "Film chemistry"
        } else {
            "Print chemistry"
        };
        if !self.filter.is_empty() {
            // Search mode: no collapsing header, and matching parts only.
            let development_time = controls::label_matches(&self.filter, "development time");
            if !development_time && !self.section_has_filter_match(section) {
                return;
            }
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Chemistry")
                        .color(theme::FG)
                        .size(theme::SIZE_TITLE)
                        .strong(),
                );
                self.section_reset_controls(ui, section);
            });
            if development_time {
                self.chemistry_dev_time(ui, film);
            }
            self.parameter_section(ui, section);
            return;
        }
        let mut label_clicked = false;
        let mut header = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.make_persistent_id(("chemistry", film)),
            false,
        )
        .show_header(ui, |ui| {
            label_clicked = ui
                .add(egui::Label::new("Chemistry").sense(egui::Sense::click()))
                .clicked();
            self.section_reset_controls(ui, section);
        });
        if label_clicked {
            header.toggle();
        }
        let closed = !header.is_open();
        header.body(|ui| {
            self.chemistry_dev_time(ui, film);
            self.parameter_section(ui, section);
        });
        if closed {
            ui.label(
                egui::RichText::new(
                    "Density curves: development time, gamma, developer exhaustion",
                )
                .color(theme::FG_MUTED)
                .small(),
            );
        }
    }

    fn chemistry_dev_time(&mut self, ui: &mut egui::Ui, film: bool) {
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
        if dev_time_combo(
            ui,
            if film { "film-time" } else { "print-time" },
            "development time",
            times,
            selected,
        ) {
            self.mark_parameters_changed();
        }
    }

    fn controls_panel(&mut self, ui: &mut egui::Ui) {
        let input_view_before = (
            self.params.io.input_color_space.clone(),
            self.params.io.input_cctf_decoding,
            self.params.settings.preview_max_size,
        );
        match self.gui_tab {
            GuiTab::Main => {
                self.import_section(ui, false);
                self.import_section(ui, true);
                for section in ["Crop and upscale", "Input", "Camera"] {
                    self.parameter_section(ui, section);
                }
                ui.collapsing("Profiles", |ui| {
                    if self.params.workflow.route == "input" {
                        ui.weak(
                            "The input route ignores these profiles: it only colour-manages the \
                             input to the output space.",
                        );
                    }
                    if profile_combo(ui, "film", "film profile", &self.films, &mut self.film_name) {
                        let selected = self.film_name.clone();
                        self.params.film_render.development_time = None;
                        self.select_film_profile(selected);
                    }
                    if profile_combo(
                        ui,
                        "paper",
                        "print profile",
                        &self.papers,
                        &mut self.print_name,
                    ) {
                        let selected = self.print_name.clone();
                        self.params.print_render.development_time = None;
                        self.select_print_profile(selected);
                    }
                });
                for section in ["Enlarger", "Scanner"] {
                    self.parameter_section(ui, section);
                }
                self.parameter_section(ui, "Output");
                self.export_actions(ui);
            }
            GuiTab::Film => {
                self.chemistry_section(ui, true);
                for section in [
                    "Film base",
                    "Halation",
                    "Couplers",
                    "Grain",
                    "Camera diffusion",
                    "Convert",
                ] {
                    self.parameter_section(ui, section);
                }
            }
            GuiTab::Print => {
                self.chemistry_section(ui, false);
                for section in ["Print base", "Glare", "Enlarger diffusion"] {
                    self.parameter_section(ui, section);
                }
                self.parameter_section(ui, "CMYK ink style");
            }
            GuiTab::Advanced => {
                for section in [
                    "Spectral upsampling",
                    "Input gamut compress",
                    "Output gamut compress",
                ] {
                    self.parameter_section(ui, section);
                }
            }
            GuiTab::Config => {
                self.diagnostics_panel(ui);
                ui.collapsing("GUI parameters", |ui| {
                    self.state_toolbar(ui);
                });
                ui.collapsing("Display", |ui| {
                    let display_transform_before = self.viewer.settings.use_display_transform;
                    self.display_profile_controls(ui);
                    self.viewer.controls(ui);
                    self.parameter_section(ui, "Display");
                    self.viewer.interpolation_control(ui);
                    if display_transform_before != self.viewer.settings.use_display_transform {
                        self.refresh_viewing_artifacts();
                    }
                });
            }
        }
        if input_view_before
            != (
                self.params.io.input_color_space.clone(),
                self.params.io.input_cctf_decoding,
                self.params.settings.preview_max_size,
            )
        {
            self.refresh_viewing_artifacts();
        }
    }

    /// The window header spans the full width above the three regions. It
    /// shows the film profile, print profile, and workflow route as
    /// read-only text, and it is the only surface that shows the
    /// operation status.
    ///
    /// The exact route string stays visible: it identifies the physical
    /// sequence and is required for diagnostics. The description beside it
    /// is a direct reading of that same sequence, taken from the shared
    /// presentation vocabulary, so it cannot describe a route the selector
    /// does not offer.
    fn window_header(&self, ctx: &egui::Context) {
        let film = profiles::selected_display(&self.films, &self.film_name);
        let paper = profiles::selected_display(&self.papers, &self.print_name);
        egui::TopBottomPanel::top("window-header")
            .frame(egui::Frame::default().fill(theme::SURFACE2).inner_margin(
                egui::Margin::symmetric(theme::SPACE_GROUP as i8, theme::SPACE_ROW as i8),
            ))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("spektrafilm").strong());
                    ui.separator();
                    ui.label(format!("Film: {film}"));
                    ui.separator();
                    ui.label(format!("Print: {paper}"));
                    ui.separator();
                    ui.label(format!("Route: {}", self.params.workflow.route));
                    // Direct physical reading of the same route, not a second
                    // route: the exact serialized string stays on the row, and
                    // the description beside it comes from the shared
                    // vocabulary, so it cannot name a route the selector does
                    // not offer. An unknown route shows no description rather
                    // than a wrong one.
                    let description =
                        routes::vocabulary::route_description(&self.params.workflow.route);
                    if !description.is_empty() {
                        // Truncated rather than wrapped: the header stays one
                        // row, so the pinned viewer canvas does not move.
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(description)
                                    .color(theme::FG_MUTED)
                                    .small(),
                            )
                            .wrap_mode(egui::TextWrapMode::Truncate),
                        )
                        .on_hover_text(format!(
                            "{description} The exact route beside it is the serialized value and is required for diagnostics."
                        ));
                    }
                    ui.separator();
                    ui.label(egui::RichText::new(self.operation_status()).small());
                });
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.render_progress.bind_context(ctx);
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
        self.poll_render_job(ctx);
        self.poll_calibration_job(ctx);
        self.poll_export_job(ctx);
        // The export renders the retained input, so the dialog's estimate
        // reads that image's dimensions while the dialog is open.
        let export_dimensions = self
            .image
            .as_ref()
            .map(|image| [image.width, image.height])
            .unwrap_or([0, 0]);
        if let Some(options) = self.export_dialog.show(ctx, export_dimensions) {
            self.start_export(ctx, options);
        }
        self.window_header(ctx);
        self.factory_reset_confirmation(ctx);
        self.preset_panel(ctx);
        // Right control panel: histogram at top, tabs, scrollable controls, action bar
        egui::SidePanel::right("controls")
            .resizable(false)
            .exact_width(theme::WIDTH_CONTROL_PANEL)
            .show(ctx, |ui| {
                let output_id = self
                    .output_image
                    .as_ref()
                    .map(|o| Arc::as_ptr(o) as usize as u64)
                    .unwrap_or(0);
                self.histogram
                    .update(&self.output_image, output_id, ui.ctx());
                self.histogram.render(ui);
                ui.add_space(theme::SPACE_BLOCK);
                ui.horizontal_wrapped(|ui| {
                    // One segmented control: every page shares a frame, so the
                    // current page reads as a position in a row rather than a
                    // lone highlighted button. Segments take their natural
                    // width, which fits the panel, so no label is clipped.
                    for tab in GuiTab::ALL {
                        let selected = self.gui_tab == tab;
                        if ui
                            .add(
                                egui::Button::new(tab.label())
                                    .selected(selected)
                                    .corner_radius(theme::RADIUS_CARD),
                            )
                            .on_hover_text(tab.description())
                            .clicked()
                        {
                            self.gui_tab = tab;
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let label = ui.label("Filter controls");
                    ui.text_edit_singleline(&mut self.filter)
                        .labelled_by(label.id);
                    // Clearing the filter restores the unfiltered controls; it
                    // is transient state and touches neither parameters nor
                    // persisted sections. The action appears only while a
                    // filter is active, so it never suggests an empty filter.
                    if !self.filter.is_empty() {
                        let response = ui.button("clear filter").on_hover_text(
                            "Clear the control filter and show every control of this page.",
                        );
                        if response.clicked() {
                            self.filter.clear();
                        }
                    }
                });
                self.search_results(ui);
                ui.add_space(theme::SPACE_ROW);
                // Reserve the action bar's height below the scrollable
                // controls, so the bar stays inside the panel.
                let scroll_height =
                    (ui.available_height() - theme::HEIGHT_ACTION_BAR_RESERVE).max(1.0);
                ui.allocate_ui(egui::vec2(ui.available_width(), scroll_height), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("controls-scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.controls_panel(ui));
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
                    // Orientation: rotate the retained input. This is an
                    // input change, not a viewer decoration: Preview, Scan and
                    // Export render the rotated image.
                    if ui
                        .button("ccw rotate")
                        .on_hover_text("Rotate the retained input 90° counter-clockwise. Rotation clears the retained output and schedules a new preview; Preview, Scan, and Export render the rotated input. Save writes only a retained output and cannot rerender it.")
                        .clicked()
                    {
                        self.rotate_input_image_counterclockwise();
                        ui.ctx().request_repaint();
                    }
                    if ui
                        .button("cw rotate")
                        .on_hover_text("Rotate the retained input 90° clockwise. Rotation clears the retained output and schedules a new preview; Preview, Scan, and Export render the rotated input. Save writes only a retained output and cannot rerender it.")
                        .clicked()
                    {
                        self.rotate_input_image_clockwise();
                        ui.ctx().request_repaint();
                    }
                    ui.separator();
                    // View: zoom presets and the fit reset. Exact percentages
                    // are a source-pixel-to-device-pixel scale, so 100% means
                    // one source pixel per device pixel.
                    for (label, percent) in [("100%", 100.0), ("200%", 200.0), ("400%", 400.0)] {
                        if ui
                            .button(label)
                            .on_hover_text(format!(
                                "Show the retained image at {percent:.0}% — one source pixel per device pixel at 100%. Viewer presentation only; retained, saved and exported pixels are unchanged."
                            ))
                            .clicked()
                        {
                            self.viewer.set_zoom_percent(percent);
                            ui.ctx().request_repaint();
                        }
                    }
                    if ui
                        .button("fit")
                        .on_hover_text("Return to the zoom that fits the retained image in the viewer window, leaving the pan where it is. Viewer presentation only; retained, saved and exported pixels are unchanged.")
                        .clicked()
                    {
                        self.viewer.fit_view();
                        ui.ctx().request_repaint();
                    }
                    if ui
                        .button("reset view")
                        .on_hover_text("Return to the fitted zoom and recenter the pan. Viewer presentation only; retained, saved and exported pixels are unchanged.")
                        .clicked()
                    {
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
                    if ui
                        .selectable_label(self.compare_active, compare_label)
                        .on_hover_text("Compare the retained input with the retained output side by side. The split moves where the before and after halves meet; both halves are viewer composition only and change neither retained, saved nor exported pixels.")
                        .clicked()
                    {
                        self.compare_active = !self.compare_active;
                        ui.ctx().request_repaint();
                    }
                    if self.compare_active {
                        ui.add_space(theme::SPACE_GROUP);
                        let slider = ui.add(
                            egui::Slider::new(&mut self.compare_split, 0.0..=1.0)
                                .fixed_decimals(2)
                                .trailing_fill(false),
                        );
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
            self.viewer.layer_controls(ui);
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
