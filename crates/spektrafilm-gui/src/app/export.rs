//! Save/Export coordination for the latest accepted float output.
use crate::*;

use super::state::App;

impl App {
    /// Save retained pixels, asking before choosing a destination if they no
    /// longer match accepted adjustments. This never rerenders.
    pub(crate) fn save_dialog(&mut self) {
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

    /// Save retained output directly. Consumers own any UI confirmation; this
    /// raw pixel-writing boundary deliberately does not ask or rerender.
    pub(crate) fn save_to_path(&mut self, path: &Path) {
        if self.output_image.is_none() && self.output_retained.is_none() {
            self.status = crate::app::Message::new("operations.nothing_to_save");
            return;
        }
        let mut operation = Operation::new(OperationKind::Save, self.diagnostics.mode);
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
        let t = Instant::now();
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
        let pixels = match if let Some(retained) = self.output_retained.as_ref() {
            image_io::convert_retained_samples_with_backend(
                retained,
                destination,
                encoded,
                kind,
                self.backend.as_ref(),
            )
        } else {
            image_io::convert_samples_with_backend(
                self.output_image.as_ref().expect("accepted CPU output"),
                &self.output_color_space,
                self.output_cctf_encoding,
                destination,
                encoded,
                kind,
                self.backend.as_ref(),
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
                path,
                save_options,
                self.output_metadata.as_ref(),
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
