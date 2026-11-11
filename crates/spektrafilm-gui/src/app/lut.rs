//! LUT workflow switching, import/declaration, and the LUT look surface.
use super::state::App;
use crate::*;
use spektrafilm_core::lut_formats::{LutFormat, read_lut};
use spektrafilm_core::lut_look::{LutDomainPolicy, LutLookParams, LutShaper, LutSpace, hash_file};
use spektrafilm_core::params::WorkflowMode;

pub(crate) struct LutImportDraft {
    path: PathBuf,
    sha256: String,
    domain_min: [f64; 3],
    domain_max: [f64; 3],
    shaper: LutShaper,
    input_space: LutSpace,
    output_space: LutSpace,
    policy: LutDomainPolicy,
    strength: f64,
    error: Option<String>,
    preview: Option<Vec<[u8; 3]>>,
    preview_error: Option<String>,
    preview_key: Option<(LutSpace, LutSpace, LutDomainPolicy, LutShaper, u64)>,
}

impl LutImportDraft {
    fn params(&self) -> LutLookParams {
        LutLookParams {
            path: self.path.clone(),
            sha256: self.sha256.clone(),
            input_space: self.input_space,
            output_space: self.output_space,
            domain_min: self.domain_min,
            domain_max: self.domain_max,
            domain_policy: self.policy,
            shaper: self.shaper,
            strength: self.strength,
            grade: Default::default(),
            halation: spektrafilm_core::params::HalationParams {
                active: false,
                ..Default::default()
            },
            diffusion: Default::default(),
            grain: Default::default(),
        }
    }

    fn preview_key(&self) -> (LutSpace, LutSpace, LutDomainPolicy, LutShaper, u64) {
        (
            self.input_space,
            self.output_space,
            self.policy,
            self.shaper,
            self.strength.to_bits(),
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
    pub(crate) fn switch_workflow(&mut self, mode: WorkflowMode) {
        if self.params.workflow.mode == mode {
            return;
        }
        if mode == WorkflowMode::Lut && self.params.lut_look.load().is_err() {
            self.status = crate::app::Message::new("surfaces.status.lut_import_required");
            self.begin_import_lut();
            return;
        }
        if mode == WorkflowMode::Lut {
            self.film_route = Some(self.params.workflow.route.clone());
        } else if let Some(route) = self.film_route.take() {
            self.params.io.scan_film = spektrafilm_core::WorkflowRoute::parse(&route).scans_film();
            self.params.workflow.route = route;
        }
        self.params.workflow.mode = mode;
        if mode == WorkflowMode::Lut {
            self.params.workflow.route = "input".into();
            self.params.io.scan_film = false;
        }
        self.presets.state.applied = None;
        self.label_adjustment("Change workflow");
        self.invalidate_look_render();
        self.status = crate::app::Message::new(match mode {
            WorkflowMode::Film => "surfaces.status.workflow_film",
            WorkflowMode::Lut => "surfaces.status.workflow_lut",
        });
    }

    pub(crate) fn begin_import_lut(&mut self) {
        let Some(path) = self.chooser.pick_file(
            self.file_dialog("lut")
                .set_file_name("look.cube")
                .add_filter("CUBE LUT", &["cube"]),
        ) else {
            return;
        };
        self.remember_dialog("lut", &path);
        let result = (|| -> Result<LutImportDraft, String> {
            let sha256 = hash_file(&path)?;
            let document = read_lut(LutFormat::Cube, &path).map_err(|e| e.to_string())?;
            Ok(LutImportDraft {
                path,
                sha256,
                domain_min: document.domain_min,
                domain_max: document.domain_max,
                shaper: if document.shaper.is_some() {
                    LutShaper::Cube1d
                } else {
                    LutShaper::None
                },
                input_space: LutSpace::Srgb,
                output_space: LutSpace::Srgb,
                policy: LutDomainPolicy::Fail,
                strength: 1.0,
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
                ui.monospace(draft.path.display().to_string());
                ui.small(format!("sha256: {}", draft.sha256));
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::text(ctx, "surfaces.lut.input_space"));
                    egui::ComboBox::from_id_salt("lut-import-input")
                        .selected_text(draft.input_space.name())
                        .show_ui(ui, |ui| {
                            for space in [LutSpace::SrgbLinear, LutSpace::Srgb, LutSpace::DisplayP3]
                            {
                                ui.selectable_value(&mut draft.input_space, space, space.name());
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::text(ctx, "surfaces.lut.output_space"));
                    egui::ComboBox::from_id_salt("lut-import-output")
                        .selected_text(draft.output_space.name())
                        .show_ui(ui, |ui| {
                            for space in [LutSpace::SrgbLinear, LutSpace::Srgb, LutSpace::DisplayP3]
                            {
                                ui.selectable_value(&mut draft.output_space, space, space.name());
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::text(ctx, "surfaces.lut.domain_policy"));
                    ui.selectable_value(&mut draft.policy, LutDomainPolicy::Fail, "fail");
                    ui.selectable_value(&mut draft.policy, LutDomainPolicy::Clamp, "clamp");
                });
                ui.add(
                    egui::Slider::new(&mut draft.strength, 0.0..=1.0)
                        .text(crate::i18n::text(ctx, "surfaces.lut.strength")),
                );
                ui.small(format!(
                    "{}: [{:.6}, {:.6}, {:.6}] .. [{:.6}, {:.6}, {:.6}]",
                    crate::i18n::text(ctx, "surfaces.lut.domain"),
                    draft.domain_min[0],
                    draft.domain_min[1],
                    draft.domain_min[2],
                    draft.domain_max[0],
                    draft.domain_max[1],
                    draft.domain_max[2]
                ));
                ui.small(format!(
                    "{}: {}",
                    crate::i18n::text(ctx, "surfaces.lut.shaper"),
                    draft.shaper.name()
                ));
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
            let params = draft.params();
            match params.load() {
                Ok(_) => {
                    if self.params.workflow.mode == WorkflowMode::Film {
                        self.film_route = Some(self.params.workflow.route.clone());
                    }
                    self.params.lut_look = params;
                    self.params.workflow.mode = WorkflowMode::Lut;
                    self.params.workflow.route = "input".into();
                    self.params.io.scan_film = false;
                    self.presets.state.applied = None;
                    self.label_adjustment("Import LUT");
                    self.invalidate_look_render();
                    self.status = crate::app::Message::new("surfaces.status.lut_imported")
                        .arg("path", draft.path.display());
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
        if self.params.scanner.experimental_setup {
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
        let temporary = draft.path.with_extension("cube.tmp");
        if let Err(error) = lut.write_cube(&temporary) {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        std::fs::rename(&temporary, &draft.path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            error.to_string()
        })
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
        app.params.scanner.experimental_setup = false;
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
        app.params.scanner.experimental_setup = false;
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
        app.params.scanner.experimental_setup = false;
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
}
