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

    fn label(self) -> &'static str {
        match self {
            Self::Main => "MAIN",
            Self::Film => "FILM",
            Self::Print => "PRINT",
            Self::Advanced => "ADVANCED",
            Self::Config => "CONFIG",
        }
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
            if ui.button("Save current as default").clicked() {
                let result = self
                    .current_state()
                    .and_then(|s| s.save(&state::default_path(&self.config_directory)));
                self.status = match result {
                    Ok(()) => "Saved current GUI state as startup default".into(),
                    Err(e) => format!("Startup save error: {e:#}"),
                };
            }
            if ui.button("Restore factory default").clicked() {
                let result =
                    state::reset_factory(&self.config_directory).and_then(|s| self.apply_state(s));
                self.status = match result {
                    Ok(()) => "Restored factory default GUI state".into(),
                    Err(e) => format!("Factory reset error: {e:#}"),
                };
            }
        });
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
                    format!("Loaded GUI state from {}", path.display())
                }
                Err(e) => format!("State load error: {e:#}"),
            };
        }
    }

    pub(super) fn toggle_scan_for_print(&mut self) {
        if let Some((white, black, glare)) = self.scan_for_print_snapshot.take() {
            self.params.scanner.white_correction = white;
            self.params.scanner.black_correction = black;
            self.params.print_render.glare.active = glare;
        } else {
            if !self
                .film_medium
                .for_route(&self.params.workflow.route)
                .correction_eligible()
            {
                return;
            }
            self.scan_for_print_snapshot = Some((
                self.params.scanner.white_correction,
                self.params.scanner.black_correction,
                self.params.print_render.glare.active,
            ));
            self.params.scanner.white_correction = true;
            self.params.scanner.black_correction = true;
            self.params.print_render.glare.active = false;
        }
        self.mark_parameters_changed();
        self.force_preview = true;
    }

    fn simulation_action_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            controls::extra_bool_tip(ui, &mut self.gui_state.sections, "simulation", "auto_preview", "auto preview", true, "trigger the preview after every change of gui parameters, use mouse scrollwheel on parameters field, read preview tooltip for details");
            if self.film_medium.for_route(&self.params.workflow.route).correction_eligible() {
                let mut scan_for_print = self.scan_for_print_snapshot.is_some();
                if ui.checkbox(&mut scan_for_print, "white and black correction").on_hover_text("White and black correction of the scanner are active, and glare is deactivated.").changed() {
                    self.toggle_scan_for_print();
                }
            }
        });
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
            if ui
                .checkbox(
                    &mut self.params.magazine_print_color.active,
                    "Magazine finish",
                )
                .on_hover_text("Apply a magazine finish appearance after the selected workflow.")
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
                .on_hover_text("run the simulation on a small preview and deactivates grain, halation, blurs, unsharp mask (diffusion filters are active)")
                .clicked()
            {
                self.dirty = true;
                self.force_preview = true;
                self.full_scan_requested = false;
            }
            if ui.add_enabled(ready, egui::Button::new("SCAN")).on_hover_text("Run the full simulation on the full-resolution input").clicked() {
                self.dirty = true;
                self.force_preview = true;
                self.full_scan_requested = true;
            }
            if ui
                .add_enabled(
                    self.output_image.is_some() && !busy,
                    egui::Button::new("SAVE"),
                )
                .on_hover_text("Save the current output layer to an image file")
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
        if let Some((white, black, glare)) = self.scan_for_print_snapshot.take() {
            self.params.scanner.white_correction = white;
            self.params.scanner.black_correction = black;
            self.params.print_render.glare.active = glare;
        }
        self.params.io.scan_film = self.params.workflow.route == "input > film > scan";
        normalize_scanner_selections(&mut self.params, self.film_medium);
        self.invalidate_look_render();
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
        self.film_dev_times = film.data.development_time.clone();
        self.film_medium = scan_medium(&film);
        self.params.workflow.route = workflow_default(&film);
        self.params.io.scan_film = film.is_positive();
        self.params.scanner.scan_output = "direct_scan".into();
        normalize_scanner_selections(&mut self.params, self.film_medium);
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
        let preset_section = match section {
            "Film base" => Some((Section::Base, "Base", false)),
            "Grain" => Some((Section::Grain, "Grain", true)),
            "Halation" => Some((Section::Halation, "Halation", false)),
            "Couplers" => Some((Section::Couplers, "Couplers", false)),
            "Convert" => Some((Section::Convert, "Convert", false)),
            "Camera" => Some((Section::Camera, "Camera", true)),
            "Camera diffusion" => Some((Section::CameraFilter, "Diffusion", false)),
            "Enlarger diffusion" => Some((Section::EnlargerFilter, "Diffusion", false)),
            "Enlarger" => Some((Section::Enlarger, "Enlarger", true)),
            "Scanner" => Some((Section::Scanner, "Scanner", false)),
            "Output" => Some((Section::OutputMix, "Output", true)),
            _ => None,
        };
        if !self.filter.is_empty() {
            // Search mode: reveal the matching controls of a section that has
            // any, and drop a section that has none.
            if !self.section_has_filter_match(section) {
                return;
            }
            if let Some((kind, label, _)) = preset_section {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label).strong());
                    self.section_preset_menu(ui, kind);
                });
            }
            self.parameter_section_controls(ui, section);
            return;
        }
        if let Some((kind, label, default_open)) = preset_section {
            let mut label_clicked = false;
            let mut header = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                ui.make_persistent_id(("preset-section", section)),
                default_open,
            )
            .show_header(ui, |ui| {
                label_clicked = ui
                    .add(egui::Label::new(label).sense(egui::Sense::click()))
                    .clicked();
                self.section_preset_menu(ui, kind);
            });
            if label_clicked {
                header.toggle();
            }
            header.body(|ui| self.parameter_section_controls(ui, section));
        } else {
            self.parameter_section_controls(ui, section);
        }
    }

    /// Whether `section` rendered a control whose label matches the search
    /// filter.
    fn section_has_filter_match(&self, section: &str) -> bool {
        controls::cached_labels_match(&self.filter, self.section_labels.get(section))
    }

    fn parameter_section_controls(&mut self, ui: &mut egui::Ui, section: &str) {
        let cache = self.section_labels.entry(section.to_owned()).or_default();
        let changes = controls::show(
            ui,
            &mut self.params,
            &mut self.gui_state.sections,
            section,
            self.film_medium,
            &self.filter,
            cache,
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
        if changes.raw_reload
            && let Some(path) = self.image_path.clone()
        {
            self.load_image_from_path(&path);
        }
        if section == "Scanner" && self.params.scanner.scan_output == "negative_to_positive" {
            match self.params.scanner.auto_setup {
                ScannerAutoSetup::Off => {
                    ui.label("Scanner setup: disabled");
                }
                ScannerAutoSetup::PerFrame if self.image.is_none() => {
                    ui.label("Scanner setup: awaiting input");
                }
                ScannerAutoSetup::PerFrame => {
                    let key = (self.input_epoch, self.parameter_revision);
                    match self
                        .scanner_setup
                        .as_ref()
                        .filter(|(cached, _)| *cached == key)
                        .and_then(|(_, setup)| setup.get())
                    {
                        None => {
                            ui.label("Scanner setup: pending for current input revision");
                        }
                        Some(Ok(_)) => {
                            ui.label("Scanner setup: derived for current input revision");
                        }
                        Some(Err(reason)) => {
                            ui.label(format!("Scanner setup: failed — {reason}"));
                        }
                    }
                }
            }
        }
    }

    fn import_section(&mut self, ui: &mut egui::Ui, raw: bool) {
        ui.collapsing(if raw { "Import Raw" } else { "Import RGB" }, |ui| {
            if ui.button("select file").on_hover_text(if raw {
                "Load and process a raw file with the selected white balance and lens correction settings."
            } else {
                "Select an input image"
            }).clicked()
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
                ui.label(egui::RichText::new(p.file_name().and_then(|s| s.to_str()).unwrap_or("")).small());
            }
            ui.add_space(4.0);
        });
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
                    if ui.add_enabled(enabled, egui::Button::new("Export…"))
                        .on_hover_text("Choose settings and render the full image independently of Save.")
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
                ui.label(egui::RichText::new("Chemistry").strong());
                self.section_preset_menu(ui, if film { Section::Film } else { Section::Print });
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
            self.section_preset_menu(ui, if film { Section::Film } else { Section::Print });
        });
        if label_clicked {
            header.toggle();
        }
        header.body(|ui| {
            self.chemistry_dev_time(ui, film);
            self.parameter_section(ui, section);
        });
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
                            "Finished RGB ignores these profiles: it only colour-manages the \
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
                self.parameter_section(ui, "Magazine finish color");
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
    fn window_header(&self, ctx: &egui::Context) {
        let film = profiles::selected_display(&self.films, &self.film_name);
        let paper = profiles::selected_display(&self.papers, &self.print_name);
        egui::TopBottomPanel::top("window-header")
            .frame(
                egui::Frame::default()
                    .fill(theme::SURFACE2)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("spektrafilm").strong());
                    ui.separator();
                    ui.label(format!("Film: {film}"));
                    ui.separator();
                    ui.label(format!("Print: {paper}"));
                    ui.separator();
                    ui.label(format!("Route: {}", self.params.workflow.route));
                    ui.separator();
                    ui.label(egui::RichText::new(&self.status).small());
                });
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
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
        self.preset_panel(ctx);
        // Right control panel: histogram at top, tabs, scrollable controls, action bar
        egui::SidePanel::right("controls")
            .resizable(false)
            .exact_width(320.0)
            .show(ctx, |ui| {
                let output_id = self
                    .output_image
                    .as_ref()
                    .map(|o| Arc::as_ptr(o) as usize as u64)
                    .unwrap_or(0);
                self.histogram
                    .update(&self.output_image, output_id, ui.ctx());
                self.histogram.render(ui);
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for tab in GuiTab::ALL {
                        ui.selectable_value(&mut self.gui_tab, tab, tab.label());
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let label = ui.label("Filter controls");
                    ui.text_edit_singleline(&mut self.filter)
                        .labelled_by(label.id);
                });
                ui.add_space(2.0);
                let scroll_height = (ui.available_height() - 95.0).max(1.0);
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
                    if ui.button("ccw rotate").clicked() {
                        self.rotate_input_image_counterclockwise();
                        ui.ctx().request_repaint();
                    }
                    if ui.button("cw rotate").clicked() {
                        self.rotate_input_image_clockwise();
                        ui.ctx().request_repaint();
                    }
                    for (label, percent) in [("100%", 100.0), ("200%", 200.0), ("400%", 400.0)] {
                        if ui.button(label).clicked() {
                            self.viewer.set_zoom_percent(percent);
                            ui.ctx().request_repaint();
                        }
                    }
                    if ui.button("reset view").clicked() {
                        self.viewer.reset_view();
                        ui.ctx().request_repaint();
                    }
                    // Before/after compare slider
                    let compare_label = if self.compare_active {
                        "compare ON"
                    } else {
                        "compare"
                    };
                    if ui
                        .selectable_label(self.compare_active, compare_label)
                        .on_hover_text("Split-screen before/after comparison")
                        .clicked()
                    {
                        self.compare_active = !self.compare_active;
                        ui.ctx().request_repaint();
                    }
                    if self.compare_active {
                        ui.add_space(8.0);
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
                            .size(9.0),
                        );
                    }
                    let zoom = self.viewer.zoom_percent().map_or_else(
                        || format!("{:.0}% fit", self.viewer.zoom * 100.0),
                        |percent| format!("{percent:.0}%"),
                    );
                    ui.label(format!("zoom {zoom}"));
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
