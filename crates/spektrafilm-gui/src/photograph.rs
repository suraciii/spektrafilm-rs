//! Photograph-local work and explicit transitions; application preferences stay global.
use crate::app::Message;
use crate::*;
use record::{PhotographRecord, PhotographSettings};

pub(crate) mod preparation;
pub(crate) mod record;
pub(crate) mod worklist;
use preparation::PhotoPreparation;

#[cfg(test)]
mod tests;
#[derive(Default)]
pub(crate) struct PhotographWork {
    pub(crate) work_list: worklist::WorkList,
    pub(crate) baseline: Option<PhotographSettings>,
    pub(crate) record_path: Option<PathBuf>,
    pub(crate) source_digest: Option<String>,
    pub(crate) lens_model_identity: Option<String>,
    pub(crate) context: Option<egui::Context>,
    pending: Option<PhotoAction>,
    transition_error: Option<Message>,
    relocation: Option<(PathBuf, PhotographRecord)>,
    allow_close: bool,
}

pub(crate) enum PhotoAction {
    Open(PathBuf),
    Reopen(PathBuf, Option<PathBuf>),
    LoadState(PathBuf),
    SelectEntry(usize),
    RemoveEntry(usize),
    Close,
}

pub(crate) struct PhotoContext {
    pub(crate) settings: PhotographSettings,
    pub(crate) path: PathBuf,
    pub(crate) source_digest: String,
    pub(crate) raw_draft: serde_json::Value,
    pub(crate) numeric_drafts: crate::numeric::NumericDrafts,
    pub(crate) image: crate::render::InputImage,
    pub(crate) metadata: Option<ImageMetadata>,
    pub(crate) lens_info: Option<String>,
    pub(crate) lens_model_identity: Option<String>,
    pub(crate) history: crate::app::history::AdjustmentHistory,
    pub(crate) baseline: Option<PhotographSettings>,
    pub(crate) record_path: Option<PathBuf>,
    pub(crate) film_reference_illuminant: String,
    pub(crate) film_medium: controls::ScanMedium,
    pub(crate) film_dev_times: Vec<f64>,
    pub(crate) print_dev_times: Vec<f64>,
}

/// Execution and viewer preferences do not become photograph adjustments.
fn normalize_execution(params: &mut RuntimeParams) {
    let defaults = RuntimeParams::default();
    params.settings.preview_mode = defaults.settings.preview_mode;
    params.settings.preview_max_size = defaults.settings.preview_max_size;
    params.settings.use_enlarger_lut = defaults.settings.use_enlarger_lut;
    params.settings.use_scanner_lut = defaults.settings.use_scanner_lut;
    params.settings.lut_resolution = defaults.settings.lut_resolution;
    params.settings.use_fast_stats = defaults.settings.use_fast_stats;
    params.settings.neutral_print_filters_from_database =
        defaults.settings.neutral_print_filters_from_database;
    params.debug = defaults.debug;
    params.taps = defaults.taps;
}

impl App {
    pub(crate) fn accepted_source_digest(path: &Path) -> Result<String> {
        record::source_digest(path)
    }

    pub(crate) fn capture_photograph_settings(&self) -> Result<PhotographSettings> {
        let mut settings = self.accepted_photograph_settings();
        normalize_execution(&mut settings.params);
        settings.preparation = settings.params.render_preparation_state();
        Ok(settings)
    }

    pub(crate) fn photograph_has_unsaved_changes(&self) -> bool {
        if self.image.is_none() {
            return false;
        }
        let Some(b) = &self.photograph.baseline else {
            return true;
        };
        self.accepted_photograph_differs(b)
    }

    pub(crate) fn photo_preparation(
        &self,
        path: &Path,
        settings: Option<PhotographSettings>,
    ) -> Result<PhotoPreparation> {
        let mut settings = match settings {
            Some(s) => s,
            None => self.capture_photograph_settings()?,
        };
        settings
            .params
            .restore_render_preparation_state(&settings.preparation);
        let film = profile::load_profile_by_name(&self.data_dir, &settings.film)?;
        let print = profile::load_profile_by_name(&self.data_dir, &settings.print)?;
        anyhow::ensure!(
            film.is_film() && film.is_filming() && print.is_print_stock(),
            Message::new("photo.error.material_role")
        );
        settings
            .params
            .validate_detailed()
            .map_err(|error| Message::validation(&error))?;
        spektrafilm_core::validate_scan_output(&settings.params, &film)
            .map_err(anyhow::Error::msg)?;
        let mut extras = self.gui_state.sections.clone();
        extras["special"] = settings.special.clone();
        extras["load_raw"] = settings.raw.clone();
        let projected = state::GuiState::from_runtime(
            &settings.params,
            &settings.film,
            &settings.print,
            &extras,
        )?;
        projected.runtime_params()?;
        Ok(PhotoPreparation {
            path: path.to_path_buf(),
            settings,
            film,
            print,
        })
    }

    pub(crate) fn validate_photo_context(&self, context: &PhotoContext) -> Result<()> {
        context
            .settings
            .params
            .validate_detailed()
            .map_err(|error| Message::validation(&error))?;
        anyhow::ensure!(
            !context.settings.film.is_empty() && !context.settings.print.is_empty(),
            Message::new("photo.error.material_role")
        );
        Ok(())
    }

    pub(crate) fn take_photo_context(&mut self) -> Result<PhotoContext> {
        let settings = self.capture_photograph_settings()?;
        let path = self
            .image_path
            .clone()
            .ok_or_else(|| Message::new("photo.error.source_required"))?;
        let image = self
            .image
            .clone()
            .ok_or_else(|| Message::new("photo.error.source_required"))?;
        let source_digest = self
            .photograph
            .source_digest
            .clone()
            .ok_or_else(|| Message::new("photo.error.source_changed"))?;
        Ok(PhotoContext {
            settings,
            path,
            image,
            metadata: self.source_metadata.clone(),
            lens_info: self.raw_lens_info.clone(),
            lens_model_identity: self.photograph.lens_model_identity.clone(),
            history: std::mem::take(&mut self.adjustment_history),
            baseline: self.photograph.baseline.clone(),
            record_path: self.photograph.record_path.clone(),
            source_digest,
            raw_draft: self.gui_state.sections["load_raw"].clone(),
            numeric_drafts: self
                .photograph
                .context
                .as_ref()
                .map(crate::numeric::NumericDrafts::take)
                .unwrap_or_default(),
            film_reference_illuminant: self.film_reference_illuminant.clone(),
            film_medium: self.film_medium,
            film_dev_times: self.film_dev_times.clone(),
            print_dev_times: self.print_dev_times.clone(),
        })
    }

    pub(crate) fn install_photo_context(
        &mut self,
        context: PhotoContext,
        view: crate::input_worker::InputView,
    ) {
        let PhotoContext {
            mut settings,
            path,
            image,
            metadata,
            lens_info,
            lens_model_identity,
            history,
            baseline,
            record_path,
            source_digest,
            raw_draft,
            numeric_drafts,
            film_reference_illuminant,
            film_medium,
            film_dev_times,
            print_dev_times,
        } = context;
        if let Some(ctx) = &self.photograph.context {
            numeric_drafts.restore(ctx);
        }
        let execution = &self.params.settings;
        settings.params.settings.preview_max_size = execution.preview_max_size;
        settings.params.settings.preview_mode = execution.preview_mode;
        settings.params.settings.use_enlarger_lut = execution.use_enlarger_lut;
        settings.params.settings.use_scanner_lut = execution.use_scanner_lut;
        settings.params.settings.lut_resolution = execution.lut_resolution;
        settings.params.settings.use_fast_stats = execution.use_fast_stats;
        settings.params.settings.neutral_print_filters_from_database =
            execution.neutral_print_filters_from_database;
        settings.params.debug = self.params.debug.clone();
        settings.params.taps = self.params.taps.clone();
        self.params = settings.params;
        self.params
            .restore_render_preparation_state(&settings.preparation);
        self.film_name = settings.film;
        self.print_name = settings.print;
        self.gui_state.sections["special"] = settings.special;
        self.gui_state.sections["load_raw"] = raw_draft;
        self.gui_state.sections["simulation"]["film_stock"] = self.film_name.clone().into();
        self.gui_state.sections["simulation"]["print_paper"] = self.print_name.clone().into();
        self.raw_applied_settings = image_io::is_raw(&path).then_some(settings.raw);
        self.presets.state.applied = settings.applied;
        self.film_reference_illuminant = film_reference_illuminant;
        self.film_medium = film_medium;
        self.film_dev_times = film_dev_times;
        self.print_dev_times = print_dev_times;
        self.input_rotation = settings.rotation;
        self.image_path = Some(path);
        self.image = Some(image);
        self.source_metadata = metadata;
        self.raw_lens_info = lens_info;
        self.photograph.lens_model_identity = lens_model_identity;
        self.adjustment_history = history;
        self.photograph.baseline = baseline;
        self.photograph.record_path = record_path;
        self.photograph.source_digest = Some(source_digest);
        self.input_epoch = self.input_epoch.wrapping_add(1);
        self.mark_parameters_changed_invalidating_calibration();
        self.render_worker.cancel_active();
        self.render_worker.invalidate_runtime_cache();
        self.render_job = None;
        self.output_image = None;
        self.output_retained = None;
        self.output_histogram = None;
        self.viewer.set_retained_output(None);
        self.output_metadata = None;
        self.source_render = None;
        self.retained_kind = None;
        self.stale_save_confirmation = false;
        self.scan_for_print_snapshot = None;
        self.operator_compare = false;
        self.operator_comparison = None;
        self.clear_comparison();
        self.force_preview = true;
        self.full_scan_requested = false;
        self.dirty_since = Some(Instant::now());
        self.search_index.reset();
        self.publish_input_viewer(view.oriented, view.raster);
    }

    pub(crate) fn pick_work_files(&mut self) -> Option<Vec<PathBuf>> {
        self.chooser.pick_files(
            self.file_dialog("load")
                .add_filter("Image", &image_io::accepted_extensions()),
        )
    }

    pub(crate) fn pick_work_output_directory(&mut self) -> Option<PathBuf> {
        self.chooser.pick_folder(self.file_dialog("export"))
    }

    fn open_photograph(&mut self, path: &Path) -> Result<()> {
        let mut settings = self.capture_photograph_settings()?;
        settings.rotation = 0;
        settings.raw = self.gui_state.sections["load_raw"].clone();
        if image_io::is_raw(path) {
            settings.params.io.input_color_space = "ACES2065-1".into();
            settings.params.io.input_cctf_decoding = false;
        }
        let preparation = self.photo_preparation(path, Some(settings))?;
        self.queue_photograph(preparation, None)
    }

    fn queue_photograph(
        &mut self,
        preparation: PhotoPreparation,
        record: Option<(PathBuf, PhotographRecord)>,
    ) -> Result<()> {
        let outgoing = self.stage_outgoing_work()?;
        let mut inputs = vec![preparation.input(Some(self.params.settings.preview_max_size))];
        if let Some(preparation) = &outgoing {
            inputs.push(preparation.input(None));
        }
        let data_dir = self.data_dir.clone();
        let source = preparation.path.clone();
        let validation_record = record.as_ref().map(|(_, record)| record.clone());
        self.queue_input_command(
            inputs,
            Some(Box::new(move |_, prepared| {
                if let Some(record) = validation_record {
                    record.validate(Some(&source), &data_dir)?;
                    record
                        .validate_accepted_lens_model(prepared[0].lens_model_identity.as_deref())?;
                    anyhow::ensure!(
                        prepared[0].digest.as_deref() == Some(record.source.sha256.as_str()),
                        Message::new("photo.error.source_changed")
                    );
                }
                Ok(())
            })),
            move |app, mut prepared| {
                let loaded = prepared.remove(0);
                let (mut context, view) = preparation.finish(loaded);
                app.validate_photo_context(&context)?;
                let restored = outgoing.map(|outgoing| outgoing.finish(prepared.remove(0)).0);
                app.retain_prepared_work_context(restored)?;
                let (key, path) = if let Some((path, _)) = record {
                    context.record_path = Some(path.clone());
                    app.photograph.relocation = None;
                    ("photo.record.reopened", path)
                } else {
                    ("photo.opened", context.path.clone())
                };
                app.install_photo_context(
                    context,
                    view.expect("photograph request prepared its view"),
                );
                app.photograph.work_list.detach_active();
                app.status = Message::new(key).arg("path", path.display());
                Ok(())
            },
        )
    }

    pub(crate) fn request_photo_action(&mut self, action: PhotoAction) {
        if self.input_transition_pending() {
            self.status = Message::new("surfaces.status.preparing_input");
            return;
        }
        if self.photograph.pending.is_some() {
            return;
        }
        if self.photograph_has_unsaved_changes() || self.affected_unsaved_work(&action).is_some() {
            self.photograph.pending = Some(action);
            self.photograph.transition_error = None;
        } else {
            self.perform_photo_action(action);
        }
    }

    fn perform_photo_action(&mut self, action: PhotoAction) {
        let result = match action {
            PhotoAction::Open(path) => self.open_photograph(&path),
            PhotoAction::Reopen(path, source) => {
                self.reopen_photograph_record(&path, source.as_deref())
            }
            PhotoAction::LoadState(path) => state::GuiState::load(&path).and_then(|state| {
                self.apply_state_with_source(state, move |app| {
                    app.remember_dialog("state", &path);
                    app.status = if let Some(detail) = &app.last_state_normalization {
                        Message::new("surfaces.status.state_loaded_detail")
                            .arg("path", path.display())
                            .message_arg("detail", detail.clone())
                    } else {
                        Message::new("surfaces.status.state_loaded").arg("path", path.display())
                    };
                })
            }),
            PhotoAction::SelectEntry(index) => self.perform_work_select(index),
            PhotoAction::RemoveEntry(index) => self.perform_work_remove(index),
            PhotoAction::Close => {
                self.photograph.allow_close = true;
                Ok(())
            }
        };
        if let Err(error) = result {
            self.status =
                Message::new("photo.error.action").message_arg("detail", Message::error(&error));
        }
    }

    pub(crate) fn save_photograph_record_to(&mut self, path: &Path) -> Result<()> {
        let source = self
            .image_path
            .as_deref()
            .ok_or_else(|| Message::new("photo.error.source_required"))?;
        anyhow::ensure!(
            self.photograph.source_digest.as_deref()
                == Some(Self::accepted_source_digest(source)?.as_str()),
            Message::new("photo.error.source_changed")
        );
        let settings = self.capture_photograph_settings()?;
        let record = PhotographRecord::capture(source, settings.clone(), &self.data_dir)?;
        anyhow::ensure!(
            Some(record.source.sha256.as_str()) == self.photograph.source_digest.as_deref(),
            Message::new("photo.error.source_changed")
        );
        record.validate_accepted_lens_model(self.photograph.lens_model_identity.as_deref())?;
        record.save(path)?;
        self.photograph.baseline = Some(settings);
        self.photograph.record_path = Some(path.to_path_buf());
        self.status = Message::new("photo.record.saved").arg("path", path.display());
        Ok(())
    }

    fn save_photograph_record_dialog(&mut self) -> Result<bool> {
        let path = if let Some(path) = self.photograph.record_path.clone() {
            path
        } else {
            let filename = self
                .image_path
                .as_ref()
                .and_then(|p| p.file_stem())
                .and_then(|s| s.to_str())
                .unwrap_or("photograph");
            let Some(path) = self.chooser.save_file(
                self.file_dialog("photo_record")
                    .add_filter("JSON", &["json"])
                    .set_file_name(format!("{filename}.spektra-photo.json")),
            ) else {
                return Ok(false);
            };
            path
        };
        self.save_photograph_record_to(&path)?;
        Ok(true)
    }

    pub(crate) fn reopen_photograph_record(
        &mut self,
        path: &Path,
        source_override: Option<&Path>,
    ) -> Result<()> {
        let record = PhotographRecord::load(path)?;
        let source = match record.validate(source_override, &self.data_dir) {
            Ok(source) => source,
            Err(error) => {
                self.photograph.relocation = Some((path.to_path_buf(), record));
                return Err(error);
            }
        };
        let preparation = self.photo_preparation(&source, Some(record.settings.clone()))?;
        self.queue_photograph(preparation, Some((path.to_path_buf(), record)))
    }

    pub(crate) fn photograph_toolbar(&mut self, ui: &mut egui::Ui) {
        let save = ui
            .add_enabled(
                self.image.is_some(),
                egui::Button::new(crate::i18n::text(ui.ctx(), "photo.record.save")),
            )
            .on_hover_text(crate::i18n::text(ui.ctx(), "photo.record.accepted_only"));
        controls::locate_row(ui, "action.save_photo_record", &save);
        if save.clicked()
            && let Err(error) = self.save_photograph_record_dialog()
        {
            self.status =
                Message::new("photo.error.action").message_arg("detail", Message::error(&error));
        }
        let reopen = ui
            .button(crate::i18n::text(ui.ctx(), "photo.record.reopen"))
            .on_hover_text(crate::i18n::text(ui.ctx(), "photo.record.dependencies"));
        controls::locate_row(ui, "action.reopen_photo_record", &reopen);
        if reopen.clicked()
            && let Some(path) = self.chooser.pick_file(
                self.file_dialog("photo_record")
                    .add_filter("JSON", &["json"]),
            )
        {
            self.request_photo_action(PhotoAction::Reopen(path, None));
        }
        let list = ui.button(crate::i18n::text(ui.ctx(), "photo.work_list"));
        controls::locate_row(ui, "action.work_list", &list);
        if list.clicked() {
            self.photograph.work_list.open = true;
        }
    }

    pub(crate) fn photograph_dialogs(&mut self, ctx: &egui::Context) {
        if self.photograph.pending.is_some() {
            let mut decision = None;
            let inactive = self
                .photograph
                .pending
                .as_ref()
                .and_then(|action| self.affected_unsaved_work(action));
            egui::Modal::new(egui::Id::new("photograph-unsaved-transition")).show(ctx, |ui| {
                if self.input_transition_pending() {
                    ui.disable();
                }
                ui.heading(crate::i18n::text(ctx, "photo.unsaved.title"));
                ui.label(crate::i18n::text(ctx, "photo.record.accepted_only"));
                if let Some(path) = inactive
                    .and_then(|index| self.work_entry_path(index))
                    .or(self.image_path.as_deref())
                {
                    ui.label(path.display().to_string());
                }
                if let Some(error) = &self.photograph.transition_error {
                    ui.colored_label(ui.visuals().error_fg_color, error.localized(ctx));
                }
                ui.horizontal(|ui| {
                    for (key, value) in [
                        ("photo.unsaved.save", 0),
                        ("photo.unsaved.discard", 1),
                        ("photo.unsaved.cancel", 2),
                    ] {
                        if ui.button(crate::i18n::text(ctx, key)).clicked() {
                            decision = Some(value);
                        }
                    }
                });
            });
            if !self.input_transition_pending()
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                decision = Some(2);
            }
            match decision {
                Some(0) => match if let Some(index) = inactive {
                    self.save_inactive_record_dialog(index)
                } else {
                    self.save_photograph_record_dialog()
                } {
                    Ok(true) => {
                        if let Some(action) = self.photograph.pending.take() {
                            self.request_photo_action(action);
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        self.photograph.transition_error = Some(Message::error(&error));
                    }
                },
                Some(1) => {
                    if let Some(index) = inactive {
                        match self.discard_inactive_work_then(index, |app| {
                            if let Some(action) = app.photograph.pending.take() {
                                app.request_photo_action(action);
                            }
                            Ok(())
                        }) {
                            Ok(()) => {}
                            Err(error) => {
                                self.photograph.transition_error = Some(Message::error(&error));
                            }
                        }
                    } else if matches!(self.photograph.pending, Some(PhotoAction::Close)) {
                        self.photograph.baseline = self.capture_photograph_settings().ok();
                        if let Some(action) = self.photograph.pending.take() {
                            self.request_photo_action(action);
                        }
                    } else {
                        self.photograph.work_list.discard_outgoing = true;
                        if let Some(action) = self.photograph.pending.take() {
                            self.perform_photo_action(action);
                        }
                        self.photograph.work_list.discard_outgoing = false;
                    }
                }
                Some(2) => {
                    self.photograph.pending = None;
                    self.photograph.transition_error = None;
                }
                _ => {}
            }
        }
        if self.photograph.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.photograph.pending.is_none() {
            let mut relocate = false;
            let mut dismiss = false;
            if self.photograph.relocation.is_some() {
                egui::Window::new(crate::i18n::text(ctx, "photo.relocate.title"))
                    .id(egui::Id::new("photo-relocate-source"))
                    .collapsible(false)
                    .show(ctx, |ui| {
                        ui.label(crate::i18n::text(ctx, "photo.relocate.help"));
                        if ui
                            .button(crate::i18n::text(ctx, "photo.relocate.choose"))
                            .clicked()
                        {
                            relocate = true;
                        }
                        if ui
                            .button(crate::i18n::text(ctx, "photo.unsaved.cancel"))
                            .clicked()
                        {
                            dismiss = true;
                        }
                    });
            }
            if dismiss {
                self.photograph.relocation = None;
            }
            if relocate
                && let Some(source) = self.chooser.pick_file(
                    self.file_dialog("load")
                        .add_filter("Image", &image_io::accepted_extensions()),
                )
                && let Some((path, _)) = self.photograph.relocation.take()
            {
                self.request_photo_action(PhotoAction::Reopen(path, Some(source)));
            }
        }
    }

    pub(crate) fn protect_photograph_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.viewport().close_requested())
            && !self.photograph.allow_close
            && (self.photograph_has_unsaved_changes()
                || self.affected_unsaved_work(&PhotoAction::Close).is_some())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_photo_action(PhotoAction::Close);
        }
    }
}
