//! Save/Export coordination for the latest accepted float output.
use crate::*;

use super::state::App;

impl App {
    /// Save the latest accepted float output using current saving color settings.
    /// The typed extension selects the format; this never rerenders.
    pub(crate) fn save_dialog(&mut self) {
        if self.output_image.is_none() {
            self.status = "Nothing to save yet — load an image first.".into();
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
                .add_filter("Image", &image_io::container_extensions())
                .set_file_name(&default_name),
        ) else {
            return;
        };
        self.save_to_path(&path);
    }

    /// Save the retained output; `path` is the native chooser's confirmed result.
    pub(crate) fn save_to_path(&mut self, path: &Path) {
        if self.output_image.is_none() {
            self.status = "Nothing to save yet — load an image first.".into();
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
                self.status = format!("Save error: {error}");
                operation.issue(IssueCategory::Writing, IssueBoundary::Operation);
                self.diagnostics.finish(operation, Outcome::Failed);
                return;
            }
        };
        let out = self
            .output_image
            .as_ref()
            .expect("output checked before dialog");
        let t = Instant::now();
<<<<<<< HEAD
        let destination = self.gui_state.sections["simulation"]["saving_color_space"]
            .as_str()
            .unwrap_or("sRGB");
        let encoded = self.gui_state.sections["simulation"]["saving_cctf_encoding"]
            .as_bool()
            .unwrap_or(true);
=======
        let destination = self.gui_state.saving_color_space();
        let encoded = self.gui_state.saving_cctf_encoding();
>>>>>>> origin/main-0.3.4
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
        let converted = match image_io::convert_image(
            out,
            &self.output_color_space,
            self.output_cctf_encoding,
            destination,
            encoded,
        ) {
            Ok(image) => image,
            Err(error) => {
                self.status = format!("Save error: {error}");
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
        let result = image_io::save_rendered_output_observed(
            path,
            &converted,
            save_options,
            self.output_metadata.as_ref(),
            file_write.context(),
        );
        let result = observed_result(
            file_write,
            result,
            IssueCategory::Writing,
            IssueBoundary::FileWrite,
        );
        match result {
            Ok(report) => {
                self.status = format!(
                    "Saved {} in {:.0} ms",
                    path.file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("(file)"),
                    t.elapsed().as_secs_f32() * 1000.0
                );
                if let Some(warning) = report.metadata_warning {
                    self.status
                        .push_str(&format!(" — Metadata warning: {warning}"));
                }
                operation.set_published_dimensions(converted.width, converted.height);
                self.diagnostics.finish(operation, Outcome::Succeeded);
            }
            Err(e) => {
                self.status = format!("Save error: {e:#}");
                self.diagnostics.finish(operation, Outcome::Failed);
            }
        }
    }
}
