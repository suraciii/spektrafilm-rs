//! Save/Export coordination for the latest accepted float output.
use crate::*;

use super::state::App;

/// Immutable accepted output and save settings, held only until journal begin is durable.
pub(crate) struct PendingSave {
    operation: spektrafilm_core::telemetry::Operation,
    path: PathBuf,
    image: Option<Arc<ImageBuf>>,
    retained: Option<spektrafilm_backend::DeviceImage>,
    metadata: Option<ImageMetadata>,
    source_color_space: String,
    source_encoded: bool,
    destination: String,
    encoded: bool,
    format: image_io::ImageFormat,
    depth: BitDepth,
    backend: Arc<dyn ComputeBackend>,
}

impl App {
    /// Save retained pixels, asking before choosing a destination if they no
    /// longer match accepted adjustments. This never rerenders.
    pub(crate) fn save_dialog(&mut self) {
        if !self.pending_saves.is_empty() {
            return;
        }
        if self.output_image.is_none() && self.output_retained.is_none() {
            self.status = crate::app::Message::new("operations.nothing_to_save");
            return;
        }
        if self.accepted_frame_is_stale() {
            self.stale_save_confirmation = true;
            return;
        }
        self.choose_retained_save_path();
    }

    /// The UI-only confirmation precedes both the chooser and publication.
    /// Canceling changes no photograph controls, retained pixels, or status.
    pub(crate) fn show_stale_save_confirmation(&mut self, ctx: &egui::Context) {
        if !self.stale_save_confirmation {
            return;
        }
        let dimensions = self
            .output_retained
            .as_ref()
            .map(|image| [image.width(), image.height()])
            .or_else(|| {
                self.output_image
                    .as_ref()
                    .map(|image| [image.width, image.height])
            });
        let Some([width, height]) = dimensions else {
            self.stale_save_confirmation = false;
            return;
        };
        let mut confirmed = false;
        let mut canceled = false;
        let response = egui::Modal::new(egui::Id::new("stale-retained-save")).show(ctx, |ui| {
            ui.set_max_width(theme::WIDTH_CONTROL_PANEL);
            ui.heading(crate::i18n::text(ui.ctx(), "result.save.title"));
            ui.label(crate::tr!(
                ui.ctx(),
                "result.save.stale_warning",
                width = width,
                height = height
            ));
            ui.label(crate::i18n::text(ui.ctx(), "result.save.export_guidance"));
            ui.horizontal_wrapped(|ui| {
                let cancel = ui.button(crate::i18n::text(ui.ctx(), "surfaces.action.cancel"));
                controls::locate_row(ui, "action.cancel_stale_save", &cancel);
                canceled = cancel.clicked();
                let continue_save = ui.button(crate::i18n::text(ui.ctx(), "result.save.continue"));
                controls::locate_row(ui, "action.confirm_stale_save", &continue_save);
                confirmed = continue_save.clicked();
            });
        });
        if canceled || response.should_close() {
            self.stale_save_confirmation = false;
        } else if confirmed {
            self.stale_save_confirmation = false;
            self.choose_retained_save_path();
        }
    }

    fn choose_retained_save_path(&mut self) {
        if !self.pending_saves.is_empty() {
            return;
        }
        let default_name = match &self.image_path {
            Some(p) => {
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("spektrafilm");
                format!("{stem}.jpg")
            }
            None => "output.jpg".into(),
        };
        let Some(path) = self.chooser.save_file(
            self.file_dialog("save_output")
                .add_filter(
                    crate::i18n::text_for(self.resolved_locale, "operations.image_filter")
                        .into_owned(),
                    &image_io::container_extensions(),
                )
                .set_file_name(&default_name),
        ) else {
            return;
        };
        self.save_to_path(&path);
    }

    /// Capture retained output directly, without asking or rerendering. Image
    /// work stays on this thread but is deferred until background diagnostics begin.
    pub(crate) fn save_to_path(&mut self, path: &Path) {
        // One deferred Save owns its captured output. Reject repeated requests
        // before allocating an operation collector or copying image metadata.
        if !self.pending_saves.is_empty() {
            return;
        }
        if self.output_image.is_none() && self.output_retained.is_none() {
            self.status = crate::app::Message::new("operations.nothing_to_save");
            return;
        }
        let mut operation = self.diagnostics.begin(OperationKind::Save);
        if let Some(source) = self.source_render.clone() {
            operation.set_source_render(source);
        }
        if let Some(source) = &self.source_render {
            operation.set_working_dimensions(source.dimensions[0], source.dimensions[1]);
        }
        self.remember_dialog("save_output", path);
        let actual_format = match image_io::ImageFormat::detect(path) {
            Ok(format) => format,
            Err(error) => {
                self.status = crate::app::Message::new("operations.save_error")
                    .message_arg("detail", crate::app::Message::image_io(&error));
                operation.issue(IssueCategory::Writing, IssueBoundary::Operation);
                self.diagnostics.finish(operation, Outcome::Failed);
                return;
            }
        };
        let destination = self.gui_state.saving_color_space();
        let encoded = self.gui_state.saving_cctf_encoding();
        let save_options = SaveOptions {
            depth: match actual_format {
                image_io::ImageFormat::Jpeg | image_io::ImageFormat::Png => BitDepth::Eight,
                image_io::ImageFormat::Tiff | image_io::ImageFormat::Exr => BitDepth::Sixteen,
            },
            color_space: destination,
            cctf_encoding: encoded,
            jpeg_quality: None,
            jpeg_subsampling: None,
            compression: None,
        };
        if operation.enabled() {
            operation.set_saving_configuration(
                spektrafilm_core::telemetry::SavingConfiguration::from_save_options(
                    actual_format,
                    &save_options,
                ),
            );
        }
        self.pending_saves.push_back(PendingSave {
            operation,
            path: path.to_path_buf(),
            image: self.output_image.clone(),
            retained: self.output_retained.clone(),
            metadata: self.output_metadata.clone(),
            source_color_space: self.output_color_space.clone(),
            source_encoded: self.output_cctf_encoding,
            destination: destination.to_owned(),
            encoded,
            format: actual_format,
            depth: save_options.depth,
            backend: Arc::clone(&self.backend),
        });
        self.run_ready_saves();
        if !self.pending_saves.is_empty()
            && let Some(ctx) = &self.photograph.context
        {
            ctx.request_repaint();
        }
    }

    pub(crate) fn poll_pending_saves(&mut self, ctx: &egui::Context) {
        self.run_ready_saves();
        if !self.pending_saves.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn run_ready_saves(&mut self) {
        while self
            .pending_saves
            .front()
            .is_some_and(|save| save.operation.progress_ready())
        {
            let save = self.pending_saves.pop_front().expect("ready save exists");
            self.write_retained_save(save);
        }
    }

    pub(crate) fn cancel_pending_saves(&mut self) {
        while let Some(mut save) = self.pending_saves.pop_front() {
            save.operation
                .issue(IssueCategory::Cancelled, IssueBoundary::Operation);
            self.diagnostics.finish(save.operation, Outcome::Cancelled);
        }
    }

    fn write_retained_save(&mut self, save: PendingSave) {
        let PendingSave {
            mut operation,
            path,
            image,
            retained,
            metadata,
            source_color_space,
            source_encoded,
            destination,
            encoded,
            format: actual_format,
            depth,
            backend,
        } = save;
        // Readiness was checked nonblockingly above; this records a diagnostic
        // failure if begin failed, and never waits on filesystem work here.
        operation.wait_for_progress();
        let t = Instant::now();
        let save_options = SaveOptions {
            depth,
            color_space: &destination,
            cctf_encoding: encoded,
            jpeg_quality: None,
            jpeg_subsampling: None,
            compression: None,
        };
        let saving_convert =
            operation.scope("saving_convert", ObservationKind::Phase, Purpose::Image);
        let kind = match image_io::output_sample_kind(actual_format, save_options.depth) {
            Ok(kind) => kind,
            Err(error) => {
                self.status = crate::app::Message::new("operations.save_error")
                    .message_arg("detail", crate::app::Message::image_io(&error));
                saving_convert
                    .context()
                    .issue(IssueCategory::Writing, IssueBoundary::SavingConvert);
                saving_convert.finish(Outcome::Failed);
                self.diagnostics.finish(operation, Outcome::Failed);
                return;
            }
        };
        let pixels = match if let Some(retained) = retained.as_ref() {
            image_io::convert_retained_samples_with_backend(
                retained,
                &destination,
                encoded,
                kind,
                backend.as_ref(),
            )
        } else {
            image_io::convert_samples_with_backend(
                image.as_ref().expect("accepted CPU output"),
                &source_color_space,
                source_encoded,
                &destination,
                encoded,
                kind,
                backend.as_ref(),
            )
        } {
            Ok(image) => image,
            Err(error) => {
                self.status = crate::app::Message::new("operations.save_error")
                    .message_arg("detail", crate::app::Message::image_io(&error));
                saving_convert
                    .context()
                    .issue(IssueCategory::Writing, IssueBoundary::SavingConvert);
                saving_convert.finish(Outcome::Failed);
                self.diagnostics.finish(operation, Outcome::Failed);
                return;
            }
        };
        drop(saving_convert);
        let file_write = operation.scope("file_write", ObservationKind::Phase, Purpose::Image);
        let result = pixels
            .prepare_observed(
                &path,
                save_options,
                metadata.as_ref(),
                true,
                file_write.context(),
            )
            .and_then(|staged| staged.publish(true));
        let result = observed_result(
            file_write,
            result,
            IssueCategory::Writing,
            IssueBoundary::FileWrite,
        );
        match result {
            Ok(report) => {
                let saved = crate::app::Message::new("operations.saved")
                    .arg(
                        "file",
                        path.file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("(file)"),
                    )
                    .arg(
                        "milliseconds",
                        format!("{:.0}", t.elapsed().as_secs_f32() * 1000.0),
                    );
                self.status = if let Some(warning) = report.metadata_warning {
                    crate::app::Message::new("operations.metadata_warning")
                        .message_arg("operation", saved)
                        .message_arg("detail", crate::app::Message::bridge_error(&warning))
                } else {
                    saved
                };
                operation.set_published_dimensions(pixels.width(), pixels.height());
                self.diagnostics.finish(operation, Outcome::Succeeded);
            }
            Err(e) => {
                self.status = crate::app::Message::new("operations.save_error")
                    .message_arg("detail", crate::app::Message::image_io(&e));
                self.diagnostics.finish(operation, Outcome::Failed);
            }
        }
    }
}
