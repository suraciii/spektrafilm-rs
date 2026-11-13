//! LUT workflow switching, import/declaration, and the LUT look surface.
use super::state::App;
use crate::*;
use spektrafilm_core::lut_look::{LutDomainPolicy, LutLookParams, LutShaper, LutSpace};
use spektrafilm_core::params::WorkflowMode;

pub(crate) struct LutImportDraft {
    look: LutLookParams,
    /// Existing finishing values retained by **Replace cube**. A new import
    /// starts with grade and every effect disabled.
    preserved: Option<LutLookParams>,
    error: Option<String>,
    preview: Option<Vec<[u8; 3]>>,
    preview_error: Option<String>,
    preview_key: Option<(LutSpace, LutSpace, LutDomainPolicy, LutShaper, u64)>,
}

impl LutImportDraft {
    fn params(&self) -> LutLookParams {
        let mut look = self.look.clone();
        if let Some(previous) = &self.preserved {
            look.grade_active = previous.grade_active;
            look.grade = previous.grade.clone();
            look.halation = previous.halation.clone();
            look.diffusion = previous.diffusion.clone();
            look.grain = previous.grain.clone();
        }
        look
    }

    fn preview_key(&self) -> (LutSpace, LutSpace, LutDomainPolicy, LutShaper, u64) {
        (
            self.look.input_space,
            self.look.output_space,
            self.look.domain_policy,
            self.look.shaper,
            self.look.strength.to_bits(),
        )
    }

    fn refresh_preview(&mut self) {
        let key = self.preview_key();
        if self.preview_key == Some(key) {
            return;
        }
        self.preview_key = Some(key);
        match self.params().preview_srgb() {
            Ok(preview) => {
                self.preview = Some(preview);
                self.preview_error = None;
            }
            Err(error) => {
                self.preview = None;
                self.preview_error = Some(error);
            }
        }
    }
}

impl App {
    /// Persistent render-mode selector. It lives above the scrollable stage
    /// list so changing modes never requires scrolling back to a stage.
    pub(crate) fn mode_selector(&mut self, ui: &mut egui::Ui) {
        let mut selected = self.params.workflow.mode;
        let label = crate::i18n::text(ui.ctx(), "surfaces.workflow.mode");
        let help = crate::i18n::text(ui.ctx(), "surfaces.workflow.mode_help");
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(label.as_ref())
                    .small()
                    .color(theme::FG_SECONDARY),
            )
            .on_hover_text(help.as_ref());
            egui::Frame::new()
                .fill(theme::SURFACE2)
                .stroke(theme::stroke(theme::BORDER))
                .corner_radius(theme::RADIUS_CARD)
                .inner_margin(2)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.visuals_mut().selection.bg_fill = theme::ACCENT;
                    ui.visuals_mut().selection.stroke = theme::stroke(theme::ACCENT);
                    let width = ((ui.available_width() - 4.0) / 3.0).max(1.0);
                    ui.horizontal(|ui| {
                        for (mode, key, help_key) in [
                            (
                                WorkflowMode::Film,
                                "surfaces.workflow.film",
                                "surfaces.workflow.film_help",
                            ),
                            (
                                WorkflowMode::Lut,
                                "surfaces.workflow.lut",
                                "surfaces.workflow.lut_help",
                            ),
                            (
                                WorkflowMode::Off,
                                "surfaces.workflow.off",
                                "surfaces.workflow.off_help",
                            ),
                        ] {
                            let mode_label = crate::i18n::text(ui.ctx(), key);
                            let mode_help = crate::i18n::text(ui.ctx(), help_key);
                            let response = ui.add_sized(
                                [width, theme::HEIGHT_CONTROL - 4.0],
                                egui::Button::new(
                                    egui::RichText::new(mode_label.as_ref()).small().color(
                                        if selected == mode {
                                            theme::CONTRAST_ON_ACCENT
                                        } else {
                                            theme::FG
                                        },
                                    ),
                                )
                                .selected(selected == mode),
                            );
                            response.clone().on_hover_text(mode_help.as_ref());
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::SelectableLabel,
                                    ui.is_enabled(),
                                    selected == mode,
                                    mode_label.as_ref(),
                                )
                            });
                            ui.ctx().accesskit_node_builder(response.id, |node| {
                                node.set_label(mode_label.as_ref());
                                node.set_description(mode_help.as_ref());
                                node.set_toggled(if selected == mode {
                                    egui::accesskit::Toggled::True
                                } else {
                                    egui::accesskit::Toggled::False
                                });
                            });
                            if response.clicked() {
                                selected = mode;
                            }
                        }
                    });
                });
        });
        if selected != self.params.workflow.mode {
            crate::controls::record_edited_path(ui.ctx(), "workflow.mode");
            self.switch_workflow(selected);
        }
    }

    pub(crate) fn switch_workflow(&mut self, mode: WorkflowMode) {
        self.pending_stage = None;
        self.pending_scroll = None;
        self.stage_jump_target = None;
        if self.params.workflow.mode == mode {
            return;
        }
        if mode == WorkflowMode::Lut {
            let usable = self
                .params
                .lut_render_params()
                .is_some_and(|look| look.load().is_ok());
            if !usable {
                self.status = crate::app::Message::new("surfaces.status.lut_import_required");
                self.begin_import_lut(false);
                return;
            }
        }
        let stored_route =
            if self.params.workflow.mode == WorkflowMode::Film && mode != WorkflowMode::Film {
                Some(self.params.workflow.route.clone())
            } else {
                self.film_route.clone()
            };
        let mut candidate = self.params.clone();
        candidate.migrate_legacy_lut_look();
        let reloaded_metadata = if mode == WorkflowMode::Film
            && (self.film_medium == crate::controls::ScanMedium::None || stored_route.is_none())
        {
            let film = match crate::profile::load_profile_by_name(&self.data_dir, &self.film_name) {
                Ok(film) => film,
                Err(error) => {
                    self.status = crate::app::Message::new("surfaces.presets.profile_load")
                        .message_arg("kind", crate::app::Message::control("film"))
                        .arg("stock", &self.film_name)
                        .message_arg("detail", crate::app::Message::profile(&error));
                    return;
                }
            };
            let print = crate::profile::load_profile_by_name(&self.data_dir, &self.print_name).ok();
            Some((film, print))
        } else {
            None
        };
        if mode == WorkflowMode::Film {
            if let Some(route) = &stored_route {
                candidate.io.scan_film = spektrafilm_core::WorkflowRoute::parse(route).scans_film();
                candidate.workflow.route = route.clone();
            } else if let Some((film, _)) = &reloaded_metadata {
                candidate.workflow.route = spektrafilm_core::presets::workflow_default(film);
                candidate.io.scan_film = candidate.workflow.route_enum().scans_film();
            }
        } else {
            candidate.workflow.route = "input".into();
            candidate.io.scan_film = false;
        }
        candidate.workflow.mode = mode;
        if let Err(error) = candidate.validate() {
            self.status =
                crate::app::Message::new("surfaces.status.workflow_error").arg("detail", error);
            return;
        }
        self.params = candidate;
        if let Some((film, print)) = reloaded_metadata {
            self.film_medium = crate::scan_medium(&film);
            self.film_reference_illuminant = film.info.reference_illuminant.clone();
            self.film_dev_times = film.data.development_time.clone();
            self.print_dev_times = print
                .map(|print| print.data.development_time)
                .unwrap_or_default();
            self.params.film_render.grain.monochrome = film.is_bw();
        }
        self.film_route = if mode == WorkflowMode::Film {
            None
        } else {
            stored_route
        };
        self.presets.state.applied = None;
        self.label_adjustment("Change workflow");
        self.invalidate_look_render();
        self.status = crate::app::Message::new(match mode {
            WorkflowMode::Film => "surfaces.status.workflow_film",
            WorkflowMode::Lut => "surfaces.status.workflow_lut",
            WorkflowMode::Off => "surfaces.status.workflow_off",
        });
    }

    /// Open the LUT import dialog. `preserve` keeps the current grade and
    /// effects for **Replace cube**; a new look starts with them disabled.
    pub(crate) fn begin_import_lut(&mut self, preserve: bool) {
        let preserved = (preserve
            && self.params.workflow.mode == WorkflowMode::Lut
            && self
                .params
                .lut
                .as_ref()
                .is_some_and(|lut| !lut.path.as_os_str().is_empty()))
        .then(|| self.params.lut_render_params())
        .flatten();
        let Some(path) = self.chooser.pick_file(
            self.file_dialog("lut")
                .set_file_name("look.cube")
                .add_filter("CUBE LUT", &["cube"]),
        ) else {
            return;
        };
        self.remember_dialog("lut", &path);
        let result = (|| -> Result<LutImportDraft, String> {
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("reading LUT {}: {error}", path.display()))?;
            let input_space = preserved
                .as_ref()
                .map_or(LutSpace::Srgb, |look| look.input_space);
            let output_space = preserved
                .as_ref()
                .map_or(LutSpace::Srgb, |look| look.output_space);
            let mut look = LutLookParams::from_cube_bytes(
                path,
                &bytes,
                input_space,
                output_space,
                LutDomainPolicy::Clamp,
            )?;
            if let Some(previous) = &preserved {
                look.strength = previous.strength;
            }
            Ok(LutImportDraft {
                look,
                preserved,
                error: None,
                preview: None,
                preview_error: None,
                preview_key: None,
            })
        })();
        match result {
            Ok(draft) => self.lut_import = Some(draft),
            Err(error) => {
                self.status = crate::app::Message::new("surfaces.status.lut_import_error")
                    .arg("detail", error);
            }
        }
    }

    pub(crate) fn lut_import_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.lut_import.take() else {
            return;
        };
        draft.refresh_preview();
        let mut open = true;
        let mut accept = false;
        let mut cancel = false;
        egui::Window::new(crate::i18n::text(ctx, "surfaces.lut.import_title"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(crate::i18n::text(ctx, "surfaces.lut.non_spectral"));
                ui.monospace(draft.look.path.display().to_string());
                if !draft.look.title.is_empty() {
                    ui.label(&draft.look.title);
                }
                ui.small(crate::i18n::text(ctx, "surfaces.lut.space_note"));
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::text(ctx, "surfaces.lut.input_space"));
                    egui::ComboBox::from_id_salt("lut-import-input")
                        .selected_text(draft.look.input_space.name())
                        .show_ui(ui, |ui| {
                            for space in [LutSpace::SrgbLinear, LutSpace::Srgb, LutSpace::DisplayP3]
                            {
                                ui.selectable_value(
                                    &mut draft.look.input_space,
                                    space,
                                    space.name(),
                                );
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::text(ctx, "surfaces.lut.output_space"));
                    egui::ComboBox::from_id_salt("lut-import-output")
                        .selected_text(draft.look.output_space.name())
                        .show_ui(ui, |ui| {
                            for space in [LutSpace::SrgbLinear, LutSpace::Srgb, LutSpace::DisplayP3]
                            {
                                ui.selectable_value(
                                    &mut draft.look.output_space,
                                    space,
                                    space.name(),
                                );
                            }
                        });
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(crate::i18n::text(ctx, "surfaces.lut.domain_policy"));
                    ui.radio_value(
                        &mut draft.look.domain_policy,
                        LutDomainPolicy::Clamp,
                        crate::i18n::text(ctx, "surfaces.lut.policy_clamp"),
                    );
                    ui.radio_value(
                        &mut draft.look.domain_policy,
                        LutDomainPolicy::Fail,
                        crate::i18n::text(ctx, "surfaces.lut.policy_fail"),
                    );
                });
                ui.small(crate::i18n::text(ctx, "surfaces.lut.policy_help"));
                ui.add(
                    egui::Slider::new(&mut draft.look.strength, 0.0..=1.0)
                        .text(crate::i18n::text(ctx, "surfaces.lut.strength")),
                );
                if draft.preserved.is_some() {
                    ui.small(crate::i18n::text(ctx, "surfaces.lut.preserve_note"));
                }
                ui.label(crate::i18n::text(ctx, "surfaces.lut.preview"));
                if let Some(preview) = &draft.preview {
                    let width = ui.available_width().clamp(80.0, 420.0);
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(width, 36.0), egui::Sense::hover());
                    let ramp_width = rect.width() / 22.0;
                    let checker_width = rect.width() / 24.0;
                    let mut left = rect.left();
                    for swatch in preview.iter().take(22) {
                        let patch = egui::Rect::from_min_size(
                            egui::pos2(left, rect.top()),
                            egui::vec2(ramp_width, 18.0),
                        );
                        ui.painter().rect_filled(
                            patch,
                            0.0,
                            egui::Color32::from_rgb(swatch[0], swatch[1], swatch[2]),
                        );
                        left += ramp_width;
                    }
                    left = rect.left();
                    for swatch in preview.iter().skip(22).take(24) {
                        let patch = egui::Rect::from_min_size(
                            egui::pos2(left, rect.top() + 18.0),
                            egui::vec2(checker_width, 18.0),
                        );
                        ui.painter().rect_filled(
                            patch,
                            0.0,
                            egui::Color32::from_rgb(swatch[0], swatch[1], swatch[2]),
                        );
                        left += checker_width;
                    }
                }
                ui.small(crate::i18n::text(ctx, "surfaces.lut.preview_note"));
                egui::CollapsingHeader::new(crate::i18n::text(ctx, "surfaces.lut.advanced"))
                    .id_salt("lut-import-advanced")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.small(format!("sha256: {}", draft.look.sha256));
                        ui.small(format!(
                            "{}: R [{:.6}, {:.6}], G [{:.6}, {:.6}], B [{:.6}, {:.6}]",
                            crate::i18n::text(ctx, "surfaces.lut.domain"),
                            draft.look.domain_min[0],
                            draft.look.domain_max[0],
                            draft.look.domain_min[1],
                            draft.look.domain_max[1],
                            draft.look.domain_min[2],
                            draft.look.domain_max[2],
                        ));
                        ui.small(format!(
                            "{}: {}",
                            crate::i18n::text(ctx, "surfaces.lut.shaper"),
                            draft.look.shaper.name()
                        ));
                    });
                if let Some(error) = &draft.preview_error {
                    ui.colored_label(theme::ERROR, error);
                }
                if let Some(error) = &draft.error {
                    ui.colored_label(theme::ERROR, error);
                }
                ui.horizontal(|ui| {
                    if ui
                        .button(crate::i18n::text(ctx, "surfaces.lut.import_action"))
                        .clicked()
                    {
                        accept = true;
                    }
                    if ui
                        .button(crate::i18n::text(ctx, "surfaces.lut.cancel"))
                        .clicked()
                    {
                        cancel = true;
                    }
                });
            });
        if accept {
            let replacing = draft.preserved.is_some();
            let params = draft.params();
            match params.load() {
                Ok(_) => {
                    if self.params.workflow.mode == WorkflowMode::Film {
                        self.film_route = Some(self.params.workflow.route.clone());
                    }
                    let (lut, effects, grade) = params.split();
                    self.params.set_lut_groups(lut, effects, grade);
                    self.params.workflow.mode = WorkflowMode::Lut;
                    self.params.workflow.route = "input".into();
                    self.params.io.scan_film = false;
                    self.presets.state.applied = None;
                    self.label_adjustment(if replacing {
                        "Replace LUT"
                    } else {
                        "Import LUT"
                    });
                    self.invalidate_look_render();
                    self.status = crate::app::Message::new(if replacing {
                        "surfaces.status.lut_replaced"
                    } else {
                        "surfaces.status.lut_imported"
                    })
                    .arg("path", draft.look.path.display());
                }
                Err(error) => {
                    draft.error = Some(error);
                    self.lut_import = Some(draft);
                }
            }
        } else if !cancel && open {
            self.lut_import = Some(draft);
        }
    }
}

pub(crate) struct LutExportDraft {
    path: PathBuf,
    resolution: usize,
    input_space: LutSpace,
    output_space: LutSpace,
    error: Option<String>,
}

impl App {
    pub(crate) fn begin_export_lut(&mut self) {
        if self.params.workflow.mode != WorkflowMode::Film {
            self.status = crate::app::Message::new("surfaces.status.lut_export_film_only");
            return;
        }
        let Some(path) = self.chooser.save_file(
            self.file_dialog("lut")
                .set_file_name("film-look.cube")
                .add_filter("CUBE LUT", &["cube"]),
        ) else {
            return;
        };
        self.remember_dialog("lut", &path);
        self.lut_export = Some(LutExportDraft {
            path,
            resolution: 33,
            input_space: LutSpace::Srgb,
            output_space: LutSpace::Srgb,
            error: None,
        });
    }

    pub(crate) fn lut_export_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.lut_export.take() else {
            return;
        };
        let mut open = true;
        let mut accept = false;
        let mut cancel = false;
        egui::Window::new(crate::i18n::text(ctx, "surfaces.lut.export_title"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(crate::i18n::text(ctx, "surfaces.lut.export_external"));
                ui.small(crate::i18n::text(ctx, "surfaces.lut.export_pointwise"));
                ui.monospace(draft.path.display().to_string());
                ui.add(
                    egui::Slider::new(&mut draft.resolution, 2..=65)
                        .text(crate::i18n::text(ctx, "surfaces.lut.export_resolution")),
                );
                for (label, field) in [
                    ("surfaces.lut.input_space", &mut draft.input_space),
                    ("surfaces.lut.output_space", &mut draft.output_space),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::text(ctx, label));
                        egui::ComboBox::from_id_salt(("lut-export", label))
                            .selected_text(field.name())
                            .show_ui(ui, |ui| {
                                for space in
                                    [LutSpace::SrgbLinear, LutSpace::Srgb, LutSpace::DisplayP3]
                                {
                                    ui.selectable_value(field, space, space.name());
                                }
                            });
                    });
                }
                if let Some(error) = &draft.error {
                    ui.colored_label(theme::ERROR, error);
                }
                ui.horizontal(|ui| {
                    if ui
                        .button(crate::i18n::text(ctx, "surfaces.lut.export_action"))
                        .clicked()
                    {
                        accept = true;
                    }
                    if ui
                        .button(crate::i18n::text(ctx, "surfaces.lut.cancel"))
                        .clicked()
                    {
                        cancel = true;
                    }
                });
            });
        if accept {
            match self.export_lut_to(&draft) {
                Ok(()) => {
                    self.status = crate::app::Message::new("surfaces.status.lut_exported")
                        .arg("path", draft.path.display())
                        .arg("input", draft.input_space.name())
                        .arg("output", draft.output_space.name())
                        .arg("neutralized", "diffusion, halation, grain");
                }
                Err(error) => {
                    draft.error = Some(error);
                    self.lut_export = Some(draft);
                }
            }
        } else if !cancel && open {
            self.lut_export = Some(draft);
        }
    }

    fn export_lut_to(&self, draft: &LutExportDraft) -> Result<(), String> {
        if self.params.workflow.mode != WorkflowMode::Film {
            return Err("Export LUT is available in the film workflow only".into());
        }
        if self.params.output_mix.amount != 1.0 {
            return Err("output_mix.amount must be exactly 1 for a simple LUT export".into());
        }
        if false {
            return Err(
                "an active frame-dependent negative setup cannot be baked into a pointwise LUT"
                    .into(),
            );
        }
        let mut params = self.params.clone();
        params.debug.deactivate_spatial_effects = true;
        params.debug.deactivate_stochastic_effects = true;
        params.output_mix.amount = 1.0;
        let spec: spektrafilm_core::lut_baker::BundleSpec =
            serde_json::from_value(serde_json::json!({
                "film_profile": self.film_name,
                "print_profiles": [self.print_name],
                "input_color_space": draft.input_space.name(),
                "output_color_space": draft.output_space.name(),
                "topology": "1lut",
                "resolution": draft.resolution,
            }))
            .map_err(|error| error.to_string())?;
        let backend = spektrafilm_backend::cpu_backend::CpuBackend;
        let bundle = spektrafilm_core::lut_baker::BundleBuilder::with_params(spec, params)
            .build(&self.data_dir, &backend)?;
        let (_, lut) = bundle
            .luts
            .first()
            .ok_or_else(|| "the LUT baker returned no LUT".to_owned())?;
        let parent = draft
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let temporary = tempfile::Builder::new()
            .prefix(".lut-export-")
            .suffix(".tmp")
            .tempfile_in(parent)
            .map_err(|error| error.to_string())?
            .into_temp_path();
        lut.write_cube(temporary.as_ref())?;
        temporary
            .persist(&draft.path)
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        let directory = tempfile::tempdir().unwrap();
        let state_path = directory.path().join("state.json");
        crate::state::GuiState::factory().save(&state_path).unwrap();
        App::new_with_browser_path(
            std::sync::Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
            None,
            Some(state_path),
            None,
            directory.path().join("browser.json"),
            crate::headless::data_dir(),
        )
    }

    #[test]
    fn exported_lut_renders_through_the_lut_runtime() {
        let mut app = app();
        app.params.workflow.mode = WorkflowMode::Film;
        app.params.workflow.route = "input > film > print > scan".into();
        app.params.output_mix.amount = 1.0;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("look.cube");
        let draft = LutExportDraft {
            path: path.clone(),
            resolution: 2,
            input_space: LutSpace::Srgb,
            output_space: LutSpace::Srgb,
            error: None,
        };
        app.export_lut_to(&draft).unwrap();
        let document = spektrafilm_core::lut_formats::read_lut(
            spektrafilm_core::lut_formats::LutFormat::Cube,
            &path,
        )
        .unwrap();
        let mut params = app.params.clone();
        params.workflow.mode = WorkflowMode::Lut;
        params.workflow.route = "input".into();
        params.io.scan_film = false;
        params.lut_look = LutLookParams {
            path: path.clone(),
            sha256: spektrafilm_core::lut_look::hash_file(&path).unwrap(),
            input_space: LutSpace::Srgb,
            output_space: LutSpace::Srgb,
            domain_min: document.domain_min,
            domain_max: document.domain_max,
            domain_policy: LutDomainPolicy::Clamp,
            shaper: if document.shaper.is_some() {
                LutShaper::Cube1d
            } else {
                LutShaper::None
            },
            strength: 1.0,
            ..LutLookParams::default()
        };
        let film =
            spektrafilm_core::profile::load_profile_by_name(&app.data_dir, &app.film_name).unwrap();
        let print = spektrafilm_core::profile::load_profile_by_name(&app.data_dir, &app.print_name)
            .unwrap();
        let runtime =
            spektrafilm_core::runtime::Runtime::new(film, print, params, &app.data_dir).unwrap();
        let image =
            spektrafilm_math::image::ImageBuf::from_data(2, 1, vec![0.2, 0.4, 0.6, 0.8, 0.6, 0.3]);
        let rendered = runtime
            .process(image, &spektrafilm_backend::cpu_backend::CpuBackend)
            .unwrap();
        assert_eq!((rendered.width, rendered.height), (2, 1));
        assert!(rendered.data.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn export_accepts_linear_srgb_transport_in_both_directions() {
        let mut app = app();
        app.params.workflow.mode = WorkflowMode::Film;
        app.params.workflow.route = "input > film > print > scan".into();
        app.params.output_mix.amount = 1.0;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("linear.cube");
        let draft = LutExportDraft {
            path: path.clone(),
            resolution: 2,
            input_space: LutSpace::SrgbLinear,
            output_space: LutSpace::SrgbLinear,
            error: None,
        };
        app.export_lut_to(&draft).unwrap();
        assert!(path.is_file());
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("LUT_3D_SIZE 2"));
    }

    #[test]
    fn export_writes_one_cube_and_leaves_workflow_unchanged() {
        let mut app = app();
        app.params.workflow.mode = WorkflowMode::Film;
        app.params.workflow.route = "input > film > print > scan".into();
        app.params.output_mix.amount = 1.0;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("out.cube");
        let before = app.params.clone();
        let draft = LutExportDraft {
            path: path.clone(),
            resolution: 2,
            input_space: LutSpace::Srgb,
            output_space: LutSpace::Srgb,
            error: None,
        };
        app.export_lut_to(&draft).unwrap();
        assert!(path.is_file());
        assert_eq!(app.params, before);
        assert_eq!(app.params.workflow.mode, WorkflowMode::Film);
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("LUT_3D_SIZE 2"));
    }

    const IDENTITY_CUBE: &[u8] =
        b"TITLE \"identity\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n\
        0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";

    fn draft(preserved: Option<LutLookParams>) -> LutImportDraft {
        let look = LutLookParams::from_cube_bytes(
            PathBuf::from("/tmp/look.cube"),
            IDENTITY_CUBE,
            LutSpace::Srgb,
            LutSpace::Srgb,
            LutDomainPolicy::Clamp,
        )
        .unwrap();
        LutImportDraft {
            look,
            preserved,
            error: None,
            preview: None,
            preview_error: None,
            preview_key: None,
        }
    }

    #[test]
    fn a_new_import_disables_effects_and_uses_clamp() {
        let params = draft(None).params();
        assert_eq!(params.domain_policy, LutDomainPolicy::Clamp);
        assert_eq!(params.strength, 1.0);
        assert!(!params.grade_active);
        assert!(!params.diffusion.active);
        assert!(!params.halation.active);
        assert!(!params.grain.active);
    }

    #[test]
    fn replacing_a_cube_preserves_the_finishing_look() {
        let mut previous = draft(None).params();
        previous.strength = 0.4;
        previous.grade_active = true;
        previous.grade.contrast = 0.3;
        previous.diffusion.active = true;
        previous.halation.active = true;
        previous.grain.active = true;
        previous.grain.amount = 0.2;

        let mut replacement = draft(Some(previous.clone()));
        replacement.look.strength = previous.strength;
        let params = replacement.params();
        assert_eq!(params.strength, previous.strength);
        assert_eq!(params.grade_active, previous.grade_active);
        assert_eq!(params.grade.contrast, previous.grade.contrast);
        assert_eq!(params.diffusion.active, previous.diffusion.active);
        assert_eq!(params.halation.active, previous.halation.active);
        assert_eq!(params.grain.active, previous.grain.active);
        assert_eq!(params.grain.amount, previous.grain.amount);
    }
}
