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
                if !reprocess {
                    app.multiple_exposure = Default::default();
                } else if app.multiple_exposure.active() {
                    app.multiple_exposure.images.clear();
                    app.multiple_exposure.thumbnails.clear();
                    app.multiple_exposure.needs_reload = true;
                }
                app.publish_input_viewer(view.oriented, view.raster);
                app.image_path = Some(path);
                app.output_image = None;
                app.output_retained = None;
                app.lut_clipped_fraction = None;
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

    pub(crate) fn add_multiple_exposure_shots(&mut self) {
        if self.input_transition_pending() {
            self.status = crate::app::Message::new("surfaces.status.preparing_input");
            return;
        }
        if self.image.is_none() {
            self.status = crate::app::Message::new("multiple_exposure.primary_required");
            return;
        }
        if self.multiple_exposure.settings.total_shots() >= 8 {
            self.status = crate::app::Message::new("multiple_exposure.limit");
            return;
        }
        let Some(paths) = self.chooser.pick_files(
            self.file_dialog("load")
                .add_filter("Image", &image_io::accepted_extensions()),
        ) else {
            return;
        };
        self.queue_multiple_exposure_sources(paths, true);
        self.multiple_exposure.open = true;
    }

    pub(crate) fn replace_multiple_exposure_source(&mut self, index: usize) {
        if self.input_transition_pending() {
            self.status = crate::app::Message::new("surfaces.status.preparing_input");
            return;
        }
        let Some(shot) = self.multiple_exposure.settings.shots.get(index) else {
            return;
        };
        let old_path = shot.path.clone();
        let Some(path) = self.chooser.pick_file(
            self.file_dialog("load")
                .add_filter("Image", &image_io::accepted_extensions()),
        ) else {
            return;
        };
        let primary_raw = self.image_path.as_deref().is_some_and(image_io::is_raw);
        if image_io::is_raw(&path) != primary_raw {
            self.multiple_exposure
                .set_error("RAW and non-RAW shots cannot share one stack".into());
            return;
        }
        if self.multiple_exposure.settings.method
            == crate::multiple_exposure::MultipleExposureMethod::Rgb
            && primary_raw
        {
            self.multiple_exposure
                .set_error("RGB multiple exposure accepts raster input only".into());
            return;
        }
        let raw = self.gui_state.sections["load_raw"].clone();
        let spec = crate::input_worker::InputSpec {
            source: crate::input_worker::InputSource::File {
                path: path.clone(),
                raw,
            },
            rotation: 0,
            space: if primary_raw {
                "ACES2065-1".into()
            } else {
                self.params.io.input_color_space.clone()
            },
            decode: !primary_raw && self.params.io.input_cctf_decoding,
            max_size: None,
        };
        let revision = self.parameter_revision;
        self.queue_input_batch(
            vec![spec],
            crate::input_worker::InputFailure::Load,
            move |app, mut prepared| {
                anyhow::ensure!(
                    app.parameter_revision == revision,
                    crate::app::Message::new("operations.operation_cancelled")
                );
                let primary = app.image.as_ref().ok_or_else(|| {
                    crate::app::Message::new("multiple_exposure.primary_required")
                })?;
                let prepared = prepared.remove(0);
                anyhow::ensure!(
                    prepared.source.width() == primary.width()
                        && prepared.source.height() == primary.height(),
                    crate::app::Message::new("multiple_exposure.size_mismatch")
                );
                let digest = prepared
                    .digest
                    .ok_or_else(|| crate::app::Message::new("multiple_exposure.identity"))?;
                let path = std::fs::canonicalize(&path).map_err(|error| anyhow::anyhow!(error))?;
                anyhow::ensure!(
                    app.multiple_exposure
                        .settings
                        .shots
                        .iter()
                        .enumerate()
                        .all(|(candidate, shot)| candidate == index || shot.path != path),
                    crate::app::Message::new("multiple_exposure.identity")
                );
                let mut settings = app.multiple_exposure.settings.clone();
                settings.shots[index].path = path.clone();
                settings.shots[index].sha256 = digest;
                settings.validate().map_err(|detail| {
                    crate::app::Message::new("multiple_exposure.invalid")
                        .context(anyhow::anyhow!(detail))
                })?;
                let count = settings.total_shots();
                app.multiple_exposure
                    .images
                    .retain(|(candidate, _)| candidate != &old_path);
                app.multiple_exposure.images.push((path, prepared.source));
                app.multiple_exposure.settings = settings;
                app.multiple_exposure.selected = index + 1;
                app.multiple_exposure.clear_error();
                app.mark_parameters_changed();
                app.render_worker.cancel_active();
                app.source_render = None;
                app.output_image = None;
                app.output_retained = None;
                app.output_geometry = None;
                app.status =
                    crate::app::Message::new("multiple_exposure.status_loaded").arg("count", count);
                app.label_adjustment_message(crate::app::Message::new(
                    "multiple_exposure.history_replace",
                ));
                Ok(())
            },
        );
    }

    pub(crate) fn reload_multiple_exposure_sources(&mut self) {
        if self.input_transition_pending() {
            return;
        }
        let paths = self
            .multiple_exposure
            .settings
            .shots
            .iter()
            .map(|shot| shot.path.clone())
            .collect::<Vec<_>>();
        if paths.is_empty() {
            self.multiple_exposure.needs_reload = false;
            return;
        }
        self.multiple_exposure.needs_reload = false;
        self.queue_multiple_exposure_sources(paths, false);
    }

    fn queue_multiple_exposure_sources(&mut self, paths: Vec<PathBuf>, append: bool) {
        if self.image.is_none() {
            self.status = crate::app::Message::new("multiple_exposure.primary_required");
            return;
        }
        let primary_raw = self.image_path.as_deref().is_some_and(image_io::is_raw);
        if self.multiple_exposure.settings.method
            == crate::multiple_exposure::MultipleExposureMethod::Rgb
            && primary_raw
        {
            self.multiple_exposure
                .set_error("RGB multiple exposure accepts raster input only".into());
            return;
        }
        let mut unique = Vec::new();
        for path in paths {
            if unique.contains(&path) {
                continue;
            }
            if image_io::is_raw(&path) != primary_raw {
                self.multiple_exposure
                    .set_error("RAW and non-RAW shots cannot share one stack".into());
                return;
            }
            unique.push(path);
        }
        if append && self.multiple_exposure.settings.total_shots() + unique.len() > 8 {
            self.status = crate::app::Message::new("multiple_exposure.limit");
            return;
        }
        let raw = self.gui_state.sections["load_raw"].clone();
        let space = if primary_raw {
            "ACES2065-1".to_owned()
        } else {
            self.params.io.input_color_space.clone()
        };
        let decode = !primary_raw && self.params.io.input_cctf_decoding;
        let max_size = Some(self.params.settings.preview_max_size);
        let specs = unique
            .iter()
            .map(|path| crate::input_worker::InputSpec {
                source: crate::input_worker::InputSource::File {
                    path: path.clone(),
                    raw: raw.clone(),
                },
                rotation: 0,
                space: space.clone(),
                decode,
                max_size,
            })
            .collect::<Vec<_>>();
        let expected = if append {
            None
        } else {
            Some(
                self.multiple_exposure
                    .settings
                    .shots
                    .iter()
                    .map(|shot| shot.sha256.clone())
                    .collect::<Vec<_>>(),
            )
        };
        let revision = self.parameter_revision;
        self.queue_input_batch(
            specs,
            crate::input_worker::InputFailure::Load,
            move |app, prepared| {
                anyhow::ensure!(
                    app.parameter_revision == revision,
                    crate::app::Message::new("operations.operation_cancelled")
                );
                let primary = app.image.as_ref().ok_or_else(|| {
                    crate::app::Message::new("multiple_exposure.primary_required")
                })?;
                let (width, height) = (primary.width(), primary.height());
                let mut settings = app.multiple_exposure.settings.clone();
                let previous = settings.shots.clone();
                let mut images = if append {
                    app.multiple_exposure.images.clone()
                } else {
                    Vec::new()
                };
                if !append {
                    settings.shots.clear();
                }
                for (path, prepared) in unique.iter().zip(prepared) {
                    anyhow::ensure!(
                        prepared.source.width() == width && prepared.source.height() == height,
                        crate::app::Message::new("multiple_exposure.size_mismatch")
                    );
                    let digest = prepared
                        .digest
                        .ok_or_else(|| crate::app::Message::new("multiple_exposure.identity"))?;
                    let path =
                        std::fs::canonicalize(path).map_err(|error| anyhow::anyhow!(error))?;
                    if let Some(index) = settings.shot_index(&path) {
                        anyhow::ensure!(
                            settings.shots[index].sha256 == digest,
                            crate::app::Message::new("multiple_exposure.identity")
                        );
                        continue;
                    }
                    settings
                        .shots
                        .push(crate::multiple_exposure::MultipleExposureShot {
                            path: path.clone(),
                            sha256: digest,
                            exposure_ev: previous
                                .iter()
                                .find(|shot| shot.path == path)
                                .map_or(0.0, |shot| shot.exposure_ev),
                        });
                    images.push((path, prepared.source));
                }
                if let Some(expected) = expected.as_ref() {
                    anyhow::ensure!(
                        settings
                            .shots
                            .iter()
                            .map(|shot| shot.sha256.as_str())
                            .eq(expected.iter().map(String::as_str)),
                        crate::app::Message::new("multiple_exposure.identity")
                    );
                }
                settings.validate().map_err(|detail| {
                    crate::app::Message::new("multiple_exposure.invalid")
                        .context(anyhow::anyhow!(detail))
                })?;
                let count = settings.total_shots();
                let selected = if append && !unique.is_empty() {
                    settings.shots.len()
                } else {
                    app.multiple_exposure.selected.min(settings.shots.len())
                };
                app.multiple_exposure.settings = settings;
                app.multiple_exposure.images = images;
                app.multiple_exposure.selected = selected;
                app.multiple_exposure.needs_reload = false;
                app.multiple_exposure.clear_error();
                app.mark_parameters_changed();
                app.render_worker.cancel_active();
                app.source_render = None;
                app.output_image = None;
                app.output_retained = None;
                app.output_geometry = None;
                app.status =
                    crate::app::Message::new("multiple_exposure.status_loaded").arg("count", count);
                if append {
                    app.label_adjustment_message(crate::app::Message::new(
                        "multiple_exposure.history_add",
                    ));
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
                app.lut_clipped_fraction = None;
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
        self.analysis.clear_viewer_selection();
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
