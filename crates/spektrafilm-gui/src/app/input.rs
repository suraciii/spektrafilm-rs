//! Accepted-source replacement, rotation and viewer publication.
use super::state::App;
use crate::*;

impl App {
    pub(crate) fn load_image_from_path(&mut self, path: &Path) {
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
                app.set_finishing_compare(false);
                app.render_worker.cancel_active();
                app.full_scan_requested = false;
                app.image = Some(prepared.source);
                app.publish_input_viewer(view.oriented, view.raster);
                app.image_path = Some(path);
                app.output_image = None;
                app.output_retained = None;
                app.output_geometry = None;
                app.analysis.selection = None;
                app.analysis.clear();
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
                app.set_finishing_compare(false);
                app.render_worker.cancel_active();
                app.calibration_epoch = app.calibration_epoch.wrapping_add(1);
                app.full_scan_requested = false;
                app.output_image = None;
                app.output_retained = None;
                app.output_geometry = None;
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
        self.analysis.original_space = space.to_owned();
        self.analysis.original_encoded = encoded;
        self.analysis.original_rotation = self.input_rotation.rem_euclid(4) as u32;
        self.analysis.original_generation = self.analysis.original_generation.wrapping_add(1);
        self.viewer.cancel_analysis_selection();
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

    pub(crate) fn rotate_input_image_clockwise(&mut self) {
        self.rotate_input_image(true);
    }

    pub(crate) fn rotate_input_image_counterclockwise(&mut self) {
        self.rotate_input_image(false);
    }
}
