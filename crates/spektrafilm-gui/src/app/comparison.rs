//! One source-local, immutable photographic reference. Its renderer and pixels
//! are deliberately independent of the editable result used by Save and Export.
use super::history::AdjustmentState;
use super::state::App;
use crate::*;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComparisonReadiness {
    Ready,
    Pending,
    Unavailable,
    Incompatible,
}

struct Reference {
    adjustments: Rc<AdjustmentState>,
    source: crate::render::InputImage,
    snapshot: RenderSnapshot,
    source_metadata: Option<ImageMetadata>,
    output: Option<Arc<ImageBuf>>,
    retained: Option<spektrafilm_backend::DeviceImage>,
    color_space: String,
    encoded: bool,
    kind: RenderKind,
    failure: Option<crate::app::Message>,
}

#[derive(Default)]
pub(crate) struct Comparison {
    reference: Option<Reference>,
    inspecting_a: bool,
    generation: u64,
    worker: Option<RenderWorker>,
    job: Option<RenderJob>,
    /// Last float source installed into the shared viewer (not saved pixels).
    presented: Option<(usize, bool)>,
    managed_viewer: bool,
    presented_size: Option<[usize; 2]>,
    select_output: bool,
}

/// `Some` means A is selected. A missing `output` must NOT fall back to B.
/// `None` means the existing B/operator-baseline presentation remains selected.
pub(crate) struct ComparisonOutput<'a> {
    pub(crate) output: Option<&'a Arc<ImageBuf>>,
    pub(crate) retained: Option<&'a spektrafilm_backend::DeviceImage>,
    pub(crate) color_space: &'a str,
    pub(crate) encoded: bool,
}

impl App {
    pub(super) fn invalidate_comparison_presentation(&mut self) {
        self.comparison.presented = None;
    }

    /// Called only after an accepted source/decode/complete-state boundary.
    /// Failed replacement must leave this method uncalled.
    pub(crate) fn clear_comparison(&mut self) {
        if let Some(worker) = &self.comparison.worker {
            worker.cancel_active();
        }
        self.comparison.reference = None;
        self.comparison.inspecting_a = false;
        self.comparison.job = None;
        self.comparison.generation = self.comparison.generation.wrapping_add(1);
        self.comparison.presented = None;
    }

    pub(crate) fn capture_comparison(&mut self, ctx: &egui::Context) -> bool {
        let Some(source) = self.image.clone() else {
            return false;
        };
        let mut adjustments = (*self.accepted_adjustment()).clone();
        let controls_match = adjustments.controls_match(self);
        // The RAW panel is a decode draft, not the interpretation of this image.
        if let Some(raw) = &self.raw_applied_settings {
            adjustments.raw_settings = raw.clone();
        }
        let params = adjustments.runtime_params(self);
        let seed = params.random_seed;
        // Project through the ordinary effective-state contract, including
        // provenance and explicit ownership, without serializing numeric text.
        let mut extras = self.gui_state.sections.clone();
        extras["special"] = adjustments.special.clone();
        extras["load_raw"] = adjustments.raw_settings.clone();
        let mut params = match state::GuiState::from_runtime(
            &params,
            &adjustments.film_name,
            &adjustments.print_name,
            &extras,
        )
        .and_then(|state| state.effective_runtime_params())
        {
            Ok(params) => params,
            Err(error) => {
                self.status = crate::app::Message::new("photo_work.compare.capture_failed")
                    .message_arg("detail", crate::app::Message::error(&error));
                return false;
            }
        };
        // random_seed is intentionally skipped by RuntimeParams serde; the
        // effective-state projection must not replace this captured execution.
        params.random_seed = seed;
        let matching = !self.accepted_frame_is_stale()
            && adjustments.params.render_preparation_state()
                == self.params.render_preparation_state()
            && controls_match
            && self.retained_kind.is_some();
        let output = matching.then(|| self.output_image.clone()).flatten();
        let retained = matching.then(|| self.output_retained.clone()).flatten();
        let kind = if output.is_some() || retained.is_some() {
            self.retained_kind.expect("matching retained kind")
        } else {
            RenderKind::Preview
        };
        let color_space = if output.is_some() || retained.is_some() {
            self.output_color_space.clone()
        } else {
            params.io.output_color_space.clone()
        };
        let encoded = if output.is_some() || retained.is_some() {
            self.output_cctf_encoding
        } else {
            params.io.output_cctf_encoding
        };
        let snapshot = RenderSnapshot::new(
            params,
            adjustments.film_name.clone(),
            adjustments.print_name.clone(),
            self.data_dir.clone(),
            adjustments.special.clone(),
        );
        self.clear_comparison();
        if !self.comparison.managed_viewer {
            self.comparison.presented_size = self.viewer.output_size;
        }
        self.comparison.managed_viewer = true;
        self.comparison.reference = Some(Reference {
            adjustments: Rc::new(adjustments),
            source,
            snapshot,
            source_metadata: self.source_metadata.clone(),
            output,
            retained,
            color_space,
            encoded,
            kind,
            failure: None,
        });
        self.select_comparison_side(true);
        if self
            .comparison
            .reference
            .as_ref()
            .is_some_and(|a| a.output.is_none() && a.retained.is_none())
        {
            self.render_comparison(ctx);
        }
        true
    }

    /// Submit A on a separate instance of the existing worker: never cancel or
    /// replace a queued/running B, never write output_image or source_render.
    pub(crate) fn render_comparison(&mut self, ctx: &egui::Context) {
        let Some(reference) = self.comparison.reference.as_mut() else {
            return;
        };
        if self.comparison.job.is_some() {
            return;
        }
        reference.failure = None;
        let mut operation = Operation::new(OperationKind::Preview, self.diagnostics.mode);
        operation.set_revisions(
            Some(self.comparison.generation),
            Some(self.comparison.generation),
        );
        operation.set_input_dimensions(reference.source.width(), reference.source.height());
        operation.set_backend_requested(diagnostics::preview_backend_requested());
        let request = RenderRequest {
            job: RenderJob {
                input_epoch: self.comparison.generation,
                parameter_revision: self.comparison.generation,
                request_id: 0,
                kind: reference.kind,
                animate: false,
                rotation_quarter_turns: reference.adjustments.rotation.rem_euclid(4) as u32,
                collection_mode: operation.mode(),
            },
            image: reference.source.clone(),
            snapshot: reference
                .snapshot
                .clone()
                .with_cancellation(Some(Arc::new(AtomicBool::new(false)))),
            backend: self.backend.clone(),
            source_metadata: reference.source_metadata.clone(),
            display_enabled: self.viewer.settings.use_display_transform,
            display_profile: self.gui_state.display_profile().map(PathBuf::from),
            ctx: ctx.clone(),
            worker_wait: operation.scope("worker_wait", ObservationKind::Phase, Purpose::Image),
            operation,
        };
        let worker = self.comparison.worker.get_or_insert_with(|| {
            RenderWorker::new(Arc::new(progress::RenderProgress::default()))
        });
        let (job, replaced) = worker.submit(request);
        if let Some(operation) = replaced {
            self.diagnostics.finish(operation, Outcome::Superseded);
        }
        self.comparison.job = Some(job);
    }

    pub(super) fn poll_comparison_job(&mut self) {
        loop {
            let Some(worker) = self.comparison.worker.as_ref() else {
                return;
            };
            let (job, operation, result) = match worker.rx.try_recv() {
                Ok(response) => response,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.comparison.job = None;
                    if let Some(reference) = self.comparison.reference.as_mut() {
                        reference.failure = Some(crate::app::Message::new(
                            "operations.render_worker_vanished",
                        ));
                    }
                    self.comparison.worker = None;
                    return;
                }
            };
            if !job.matches(
                self.comparison.job.as_ref(),
                self.comparison.generation,
                self.comparison.generation,
            ) {
                self.diagnostics.finish(operation, Outcome::Superseded);
                continue;
            }
            self.comparison.job = None;
            let Some(reference) = self.comparison.reference.as_mut() else {
                self.diagnostics.finish(operation, Outcome::Superseded);
                continue;
            };
            match result {
                Ok(rendered) => {
                    reference.output = rendered.output.map(Arc::new);
                    reference.retained = rendered.retained;
                    reference.color_space = rendered.output_color_space;
                    reference.encoded = rendered.output_cctf_encoding;
                    reference.kind = job.kind;
                    reference.failure = None;
                    self.comparison.presented = None;
                    self.diagnostics.finish(operation, Outcome::Succeeded);
                }
                Err(error) => {
                    reference.failure = Some(error);
                    self.diagnostics.finish(operation, Outcome::Failed);
                }
            }
        }
    }

    pub(crate) fn comparison_geometry_compatible(&self) -> bool {
        let Some(a) = &self.comparison.reference else {
            return false;
        };
        let Some(source) = &self.image else {
            return false;
        };
        let original = &a.adjustments.params.io;
        let current = &self.params.io;
        a.source.width() == source.width()
            && a.source.height() == source.height()
            && a.adjustments.rotation == self.input_rotation
            && original.crop == current.crop
            && (!original.crop
                || (original.crop_center == current.crop_center
                    && original.crop_size == current.crop_size))
            && original.upscale_factor == current.upscale_factor
    }

    fn comparison_readiness(&self) -> ComparisonReadiness {
        let Some(a) = &self.comparison.reference else {
            return ComparisonReadiness::Unavailable;
        };
        if !self.comparison_geometry_compatible() {
            ComparisonReadiness::Incompatible
        } else if a.output.is_some() || a.retained.is_some() {
            ComparisonReadiness::Ready
        } else if self.comparison.job.is_some() {
            ComparisonReadiness::Pending
        } else {
            ComparisonReadiness::Unavailable
        }
    }

    pub(crate) fn comparison_output(&self) -> Option<ComparisonOutput<'_>> {
        if !self.comparison.inspecting_a
            || self.operator_compare
            || !self.comparison_geometry_compatible()
        {
            return None;
        }
        let a = self.comparison.reference.as_ref()?;
        Some(ComparisonOutput {
            output: a.output.as_ref(),
            retained: a.retained.as_ref(),
            color_space: &a.color_space,
            encoded: a.encoded,
        })
    }

    /// Only disposable viewer artifacts change here. Zoom/pan and transform
    /// preferences stay on the same Viewer for both photographs.
    pub(crate) fn prepare_comparison_viewer(&mut self) {
        if self.comparison.reference.is_none() && !self.comparison.managed_viewer {
            return;
        }
        let selected = self.comparison_output();
        let inspecting = selected.is_some();
        let (image, retained, space, encoded) = if let Some(a) = selected {
            (a.output, a.retained, a.color_space, a.encoded)
        } else if let Some(operator) = &self.operator_comparison {
            let retained = match &operator.image {
                crate::render::InputImage::Device(image) => Some(image),
                crate::render::InputImage::Host(_) => None,
            };
            (
                operator.image.host_image(),
                retained,
                operator.color_space.as_str(),
                operator.encoded,
            )
        } else {
            (
                self.output_image.as_ref(),
                self.output_retained.as_ref(),
                self.output_color_space.as_str(),
                self.output_cctf_encoding,
            )
        };
        let identity = image
            .map(|image| Arc::as_ptr(image) as usize)
            .or_else(|| {
                retained.and_then(|image| {
                    image
                        .payload()
                        .map(|payload| Arc::as_ptr(payload) as *const () as usize)
                })
            })
            .unwrap_or(0);
        let key = (identity, inspecting);
        if self.comparison.presented == Some(key) && !self.comparison.select_output {
            return;
        }
        let new_size = retained
            .map(|image| [image.width() as usize, image.height() as usize])
            .or_else(|| image.map(|image| [image.width as usize, image.height as usize]));
        let retained = retained.cloned();
        let space = space.to_owned();
        self.comparison.presented = Some(key);
        let selecting_output = self.comparison.select_output;
        if inspecting
            || selecting_output
            || self.viewer.settings.layer == display::ViewLayer::Output
        {
            let old_size = if self.viewer.settings.layer == display::ViewLayer::Output {
                self.comparison.presented_size.or(self.viewer.output_size)
            } else {
                self.viewer.input_size
            };
            if let (Some(old), Some(new), Some(percent)) =
                (old_size, new_size, self.viewer.zoom_percent)
            {
                // Keep the same photograph coordinate under each screen point,
                // not the same number of pixels from differently sized renders.
                // World scale is percent / pixel_world; compensating percent
                // preserves scale and therefore the existing screen-space pan.
                let source_size = self.viewer.input_size.unwrap_or(old);
                let long = source_size[0].max(source_size[1]).max(1) as f32;
                let bounds = egui::vec2(source_size[0] as f32 / long, source_size[1] as f32 / long);
                let pixel_world = |size: [usize; 2]| {
                    (bounds.x / size[0].max(1) as f32).min(bounds.y / size[1].max(1) as f32)
                };
                self.viewer.zoom_percent = Some(percent * pixel_world(new) / pixel_world(old));
            }
        }
        self.comparison.presented_size = new_size
            .or(self.comparison.presented_size)
            .or(self.viewer.output_size);
        self.comparison.select_output = false;
        self.comparison.managed_viewer = self.comparison.reference.is_some();
        if !self.comparison.managed_viewer {
            self.comparison.presented_size = None;
        }
        self.viewer.set_output_display_source(
            &space,
            encoded,
            self.viewer.settings.use_display_transform,
            self.gui_state.display_profile().map(Path::new),
        );
        // Clear unrelated disposable rasters too: a pending A must never show B.
        self.viewer.output = None;
        self.viewer.output_size = new_size;
        self.viewer.set_retained_output(retained);
        self.viewer.invalidate_display();
        if inspecting || selecting_output {
            self.viewer.settings.layer = display::ViewLayer::Output;
        }
    }

    fn select_comparison_side(&mut self, inspecting_a: bool) {
        self.comparison.inspecting_a = inspecting_a;
        self.comparison.select_output = true;
    }

    pub(crate) fn adopt_comparison(&mut self) -> bool {
        let Some(a) = &self.comparison.reference else {
            return false;
        };
        let target = a.adjustments.clone();
        let changed = self.adopt_adjustment(target);
        self.select_comparison_side(false);
        self.comparison.presented = None;
        changed
    }

    pub(crate) fn comparison_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(crate::i18n::text(ui.ctx(), "photo_work.compare.title"))
                    .strong()
                    .color(crate::theme::FG),
            );
            let capture = ui
                .add_enabled(
                    self.image.is_some(),
                    egui::Button::new(crate::i18n::text(ui.ctx(), "photo_work.compare.capture")),
                )
                .on_hover_text(crate::i18n::text(
                    ui.ctx(),
                    "photo_work.compare.capture_help",
                ));
            crate::controls::locate_row(ui, "photo_work.compare.capture", &capture);
            if capture.clicked() {
                self.capture_comparison(ui.ctx());
            }
            if self.comparison.reference.is_none() {
                return;
            }
            let compatible = self.comparison_geometry_compatible();
            let inspecting = self.comparison_output().is_some();
            let reference = ui.add_enabled(
                compatible && !self.operator_compare,
                egui::SelectableLabel::new(
                    inspecting,
                    crate::i18n::text(ui.ctx(), "photo_work.compare.a"),
                ),
            );
            crate::controls::locate_row(ui, "photo_work.compare.a", &reference);
            if reference.clicked() {
                self.select_comparison_side(true);
            }
            let current = ui.selectable_label(
                !inspecting,
                crate::i18n::text(ui.ctx(), "photo_work.compare.b"),
            );
            crate::controls::locate_row(ui, "photo_work.compare.b", &current);
            if current.clicked() {
                self.select_comparison_side(false);
            }
            let adopt = ui.button(crate::i18n::text(ui.ctx(), "photo_work.compare.adopt"));
            crate::controls::locate_row(ui, "photo_work.compare.adopt", &adopt);
            if adopt.clicked() {
                numeric::detach_suspended_adjustment(ui.ctx());
                self.adopt_comparison();
            }
            let clear = ui.button(crate::i18n::text(ui.ctx(), "photo_work.compare.clear"));
            crate::controls::locate_row(ui, "photo_work.compare.clear", &clear);
            if clear.clicked() {
                self.clear_comparison();
            }
        });
        let Some(a) = self.comparison.reference.as_ref() else {
            return;
        };
        let readiness = self.comparison_readiness();
        ui.label(crate::tr!(
            ui.ctx(),
            "photo_work.compare.reference_status",
            quality = a.kind.message().localized(ui.ctx()),
            dimensions = a
                .retained
                .as_ref()
                .map(|image| [image.width(), image.height()])
                .or_else(|| a.output.as_ref().map(|image| [image.width, image.height]))
                .map_or_else(
                    || crate::i18n::text(ui.ctx(), "photo_work.compare.dimensions_pending")
                        .into_owned(),
                    |[width, height]| format!("{width} × {height}")
                ),
            readiness = crate::i18n::text(
                ui.ctx(),
                match readiness {
                    ComparisonReadiness::Ready => "photo_work.compare.ready",
                    ComparisonReadiness::Pending => "photo_work.compare.pending",
                    ComparisonReadiness::Unavailable => "photo_work.compare.unavailable",
                    ComparisonReadiness::Incompatible => "photo_work.compare.geometry_mismatch",
                }
            )
        ));
        ui.label(crate::tr!(
            ui.ctx(),
            "photo_work.compare.current_status",
            quality = self.retained_kind.map_or_else(
                || crate::i18n::text(ui.ctx(), "photo_work.compare.unavailable").into_owned(),
                |kind| kind.message().localized(ui.ctx())
            ),
            dimensions = self
                .output_retained
                .as_ref()
                .map(|image| [image.width(), image.height()])
                .or_else(|| self
                    .output_image
                    .as_ref()
                    .map(|image| [image.width, image.height]))
                .map_or_else(
                    || crate::i18n::text(ui.ctx(), "photo_work.compare.dimensions_pending")
                        .into_owned(),
                    |[width, height]| format!("{width} × {height}")
                ),
            readiness = crate::i18n::text(
                ui.ctx(),
                if self.output_image.is_none() && self.output_retained.is_none() {
                    "photo_work.compare.unavailable"
                } else if self.accepted_frame_is_stale() {
                    "photo_work.compare.stale"
                } else {
                    "photo_work.compare.ready"
                }
            )
        ));
        if (a.output.is_some() || a.retained.is_some())
            && (self.output_image.is_some() || self.output_retained.is_some())
            && (self.retained_kind != Some(a.kind)
                || self
                    .output_retained
                    .as_ref()
                    .map(|image| [image.width(), image.height()])
                    .or_else(|| {
                        self.output_image
                            .as_ref()
                            .map(|image| [image.width, image.height])
                    })
                    != a.retained
                        .as_ref()
                        .map(|image| [image.width(), image.height()])
                        .or_else(|| a.output.as_ref().map(|image| [image.width, image.height])))
        {
            ui.label(crate::i18n::text(
                ui.ctx(),
                "photo_work.compare.unequal_quality",
            ));
        }
        if let Some(error) = &a.failure {
            ui.label(error.localized(ui.ctx()));
        }
        if a.output.is_none() && a.retained.is_none() && self.comparison.job.is_none() {
            let render = ui.button(crate::i18n::text(ui.ctx(), "photo_work.compare.render"));
            crate::controls::locate_row(ui, "photo_work.compare.render", &render);
            if render.clicked() {
                self.render_comparison(ui.ctx());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (App, tempfile::TempDir, egui::Context) {
        let directory = tempfile::tempdir().unwrap();
        let state_path = directory.path().join("state.json");
        state::GuiState::factory().save(&state_path).unwrap();
        let mut app = App::new_with_browser_path(
            Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
            None,
            Some(state_path),
            None,
            directory.path().join("browser.json"),
            crate::headless::data_dir(),
        );
        app.params.workflow.route = "input".into();
        app.params.io.crop = false;
        app.params.io.upscale_factor = 1.0;
        app.params.debug.deactivate_stochastic_effects = true;
        app.params.debug.deactivate_spatial_effects = true;
        app.params.camera.auto_exposure = false;
        app.params.settings.preview_max_size = 64;
        app.params.random_seed = 4123;
        app.image = Some(crate::render::InputImage::Host(Arc::new(
            ImageBuf::from_data(8, 4, vec![0.2; 8 * 4 * 3]),
        )));
        app.reset_adjustment_history();
        (app, directory, egui::Context::default())
    }

    fn retained(app: &mut App, value: f32, matching: bool) -> Arc<ImageBuf> {
        let image = Arc::new(ImageBuf::from_data(
            8,
            4,
            vec![value as spektrafilm_math::precision::Scalar; 8 * 4 * 3],
        ));
        app.output_image = Some(image.clone());
        app.retained_kind = Some(RenderKind::Scan);
        app.source_render = Some(SourceRender::new(
            Operation::new(OperationKind::Scan, CollectionMode::Off).id(),
            Some(app.input_epoch),
            Some(if matching {
                app.parameter_revision
            } else {
                app.parameter_revision.wrapping_add(1)
            }),
            8,
            4,
            false,
            diagnostics::color_space("sRGB"),
            true,
        ));
        image
    }

    fn edit(app: &mut App, ctx: &egui::Context, exposure: f32) {
        app.begin_adjustment_frame(ctx);
        app.params.camera.exposure_compensation_ev = exposure;
        controls::record_edited_path(ctx, "camera.exposure_compensation_ev");
        app.end_adjustment_frame(ctx);
    }

    fn wait_for_a(app: &mut App) {
        // Like production renders, even the input route prepares the selected
        // spectral method; tiny pixels do not eliminate cold LUT calibration.
        // Share the GUI suite's parallel-runner budget rather than asserting
        // an unrelated twenty-second performance limit.
        let deadline = Instant::now() + crate::headless::WAIT_DEADLINE;
        while app.comparison.job.is_some() {
            app.poll_comparison_job();
            assert!(
                Instant::now() < deadline,
                "reference render timed out: generation={}, active={:?}, queued={:?}",
                app.comparison.generation,
                app.comparison
                    .worker
                    .as_ref()
                    .and_then(|worker| worker.active.try_lock().map(|active| *active)),
                app.comparison.worker.as_ref().and_then(|worker| worker
                    .state
                    .0
                    .try_lock()
                    .map(|queued| queued.is_some()))
            );
            crate::headless::wait_tick();
        }
        let reference = app.comparison.reference.as_ref().unwrap();
        assert!(
            reference.failure.is_none(),
            "{:?}",
            reference.failure.as_ref().map(ToString::to_string)
        );
        assert!(reference.output.is_some());
    }

    #[test]
    fn capture_edit_inspect_adopt_and_undo_preserve_pixels_and_ownership() {
        let (mut app, _directory, ctx) = app();
        let original = app.params.camera.exposure_compensation_ev;
        app.presets.state.applied = Some(presets::AppliedPreset {
            group: spektrafilm_core::presets::PresetGroup::User,
            name: "reference removed from library after capture".into(),
        });
        app.reset_adjustment_history();
        let preset = app.presets.state.applied.clone();
        let ownership = app.params.render_preparation_state();
        let pixels = retained(&mut app, 0.4, true);
        assert!(app.capture_comparison(&ctx));
        assert!(Arc::ptr_eq(
            app.comparison_output().unwrap().output.unwrap(),
            &pixels
        ));
        assert!(
            app.comparison.worker.is_none(),
            "matching capture must share retained pixels"
        );
        assert!(!app.can_undo());
        edit(&mut app, &ctx, original + 1.0);
        let edited_ownership = app.params.render_preparation_state();
        app.presets.state.applied = None;
        app.viewer.pan = egui::vec2(17.0, -9.0);
        app.viewer.zoom_percent = Some(100.0);
        let revision = app.parameter_revision;
        app.prepare_comparison_viewer();
        assert_eq!(app.parameter_revision, revision);
        assert_eq!(app.params.camera.exposure_compensation_ev, original + 1.0);
        assert_eq!(app.viewer.pan, egui::vec2(17.0, -9.0));
        assert_eq!(app.viewer.zoom_percent, Some(100.0));
        assert!(Arc::ptr_eq(app.output_image.as_ref().unwrap(), &pixels));
        assert!(app.adopt_comparison());
        assert_eq!(app.params.camera.exposure_compensation_ev, original);
        assert_eq!(app.params.render_preparation_state(), ownership);
        assert_eq!(app.presets.state.applied, preset);
        assert!(app.undo_adjustment());
        assert_eq!(app.params.camera.exposure_compensation_ev, original + 1.0);
        assert_eq!(app.params.render_preparation_state(), edited_ownership);
        assert!(
            app.undo_adjustment(),
            "adoption adds exactly one entry after the edit"
        );
        assert_eq!(app.params.camera.exposure_compensation_ev, original);
        assert!(!app.can_undo());
    }

    #[test]
    fn stale_capture_really_renders_a_without_replacing_save_pixels_or_inflight_b() {
        let (mut app, _directory, ctx) = app();
        let saved = retained(&mut app, 0.95, false);
        // Deterministic barrier: B cannot begin simulation until A finishes.
        let active = app.render_worker.active.clone();
        let guard = active.lock();
        app.dispatch_render(&ctx);
        let b_job = app.render_job.clone().unwrap();
        let revision = app.parameter_revision;
        assert!(app.capture_comparison(&ctx));
        assert_eq!(app.comparison_readiness(), ComparisonReadiness::Pending);
        assert!(app.comparison_output().unwrap().output.is_none());
        app.prepare_comparison_viewer();
        assert!(
            app.viewer.output.is_none(),
            "pending A cannot display retained B"
        );
        wait_for_a(&mut app);
        let a = app.comparison_output().unwrap();
        assert_eq!(app.comparison_readiness(), ComparisonReadiness::Ready);
        assert_ne!(
            a.output.unwrap().data,
            saved.data,
            "A must contain freshly rendered accepted adjustments"
        );
        assert!(Arc::ptr_eq(app.output_image.as_ref().unwrap(), &saved));
        assert_eq!(
            app.render_job.as_ref().unwrap().request_id,
            b_job.request_id
        );
        assert_eq!(app.parameter_revision, revision);
        assert!(!app.can_undo());
        drop(guard);
    }

    #[test]
    fn numeric_and_raw_drafts_are_not_captured_and_seed_is_frozen() {
        let (mut app, _directory, ctx) = app();
        let accepted = app.params.camera.exposure_compensation_ev;
        app.raw_applied_settings = Some(serde_json::json!({"white_balance":"as_shot"}));
        app.gui_state.sections["load_raw"]["temperature"] = 1234.into();
        // Simulate a candidate not yet admitted by end_adjustment_frame.
        app.params.camera.exposure_compensation_ev = accepted + 3.0;
        assert!(app.capture_comparison(&ctx));
        let a = app.comparison.reference.as_ref().unwrap();
        assert_eq!(
            a.adjustments.params.camera.exposure_compensation_ev,
            accepted
        );
        assert_eq!(
            a.adjustments.raw_settings,
            app.raw_applied_settings.clone().unwrap()
        );
        assert_eq!(a.snapshot.params().random_seed, 4123);
        assert_eq!(
            a.snapshot.params().camera.exposure_compensation_ev,
            accepted
        );
        app.params.random_seed = 9999;
        assert_eq!(
            app.comparison
                .reference
                .as_ref()
                .unwrap()
                .snapshot
                .params()
                .random_seed,
            4123
        );
        wait_for_a(&mut app);
    }

    #[test]
    fn geometry_mismatch_disables_inspection_without_destroying_reference() {
        let (mut app, _directory, ctx) = app();
        retained(&mut app, 0.5, true);
        app.capture_comparison(&ctx);
        app.begin_adjustment_frame(&ctx);
        app.params.io.crop = true;
        app.end_adjustment_frame(&ctx);
        assert!(!app.comparison_geometry_compatible());
        assert!(app.comparison_output().is_none());
        assert!(app.comparison.reference.is_some());
        assert!(app.adopt_comparison());
        assert!(!app.params.io.crop);
        // Baseline/input comparison remains a distinct presentation.
        app.comparison.inspecting_a = true;
        app.operator_compare = true;
        assert!(app.comparison_output().is_none());
    }

    #[test]
    fn source_reset_hook_fences_delayed_reference_and_failed_replacement_preserves_it() {
        let (mut app, _directory, ctx) = app();
        app.capture_comparison(&ctx);
        let generation = app.comparison.generation;
        assert!(app.comparison.reference.is_some());
        app.image_path = Some(_directory.path().join("missing-source.png"));
        let state = app.current_state().unwrap();
        assert!(app.apply_state(state).is_err());
        assert!(
            app.comparison.reference.is_some(),
            "failed replacement keeps A"
        );
        app.clear_comparison();
        assert_ne!(app.comparison.generation, generation);
        assert!(app.comparison.reference.is_none());
        assert!(app.comparison.job.is_none());
        app.poll_comparison_job();
        assert!(app.comparison.reference.is_none());
        assert!(app.comparison_output().is_none());
    }
    #[test]
    fn unequal_resolution_alternation_preserves_photograph_coordinates_at_exact_zoom() {
        let (mut app, _directory, ctx) = app();
        let a_pixels = retained(&mut app, 0.4, true);
        app.retained_kind = Some(RenderKind::Preview);
        app.viewer.input_size = Some([80, 40]);
        app.viewer.output_size = Some([8, 4]);
        app.viewer.settings.layer = display::ViewLayer::Output;
        app.viewer.zoom_percent = Some(100.0);
        app.viewer.pan = egui::vec2(19.0, -7.0);
        assert!(app.capture_comparison(&ctx));
        app.prepare_comparison_viewer();
        let b_pixels = Arc::new(ImageBuf::from_data(80, 40, vec![0.7; 80 * 40 * 3]));
        app.output_image = Some(b_pixels.clone());
        app.retained_kind = Some(RenderKind::Scan);
        let scale = |size: [usize; 2], percent: f32| percent * size[0] as f32;
        let a_scale = scale([8, 4], app.viewer.zoom_percent.unwrap());
        app.select_comparison_side(false);
        app.prepare_comparison_viewer();
        assert_eq!(app.viewer.output_size, Some([80, 40]));
        assert_eq!(app.viewer.zoom_percent, Some(10.0));
        assert_eq!(scale([80, 40], app.viewer.zoom_percent.unwrap()), a_scale);
        assert_eq!(app.viewer.pan, egui::vec2(19.0, -7.0));
        for _ in 0..3 {
            app.select_comparison_side(true);
            app.prepare_comparison_viewer();
            assert_eq!(app.viewer.zoom_percent, Some(100.0));
            assert_eq!(app.viewer.output_size, Some([8, 4]));
            assert!(Arc::ptr_eq(
                app.comparison_output().unwrap().output.unwrap(),
                &a_pixels
            ));
            app.select_comparison_side(false);
            app.prepare_comparison_viewer();
            assert_eq!(app.viewer.zoom_percent, Some(10.0));
            assert_eq!(app.viewer.pan, egui::vec2(19.0, -7.0));
        }
        assert!(Arc::ptr_eq(app.output_image.as_ref().unwrap(), &b_pixels));
    }

    #[test]
    fn explicit_ab_navigation_reveals_output_after_input_layer_navigation() {
        let (mut app, _directory, ctx) = app();
        retained(&mut app, 0.4, true);
        app.viewer.input_size = Some([8, 4]);
        app.capture_comparison(&ctx);
        app.prepare_comparison_viewer();
        app.viewer.settings.layer = display::ViewLayer::Input;
        app.select_comparison_side(false);
        app.prepare_comparison_viewer();
        assert!(app.viewer.settings.layer == display::ViewLayer::Output);
        assert_eq!(app.viewer.output_size, Some([8, 4]));
        assert!(app.comparison_output().is_none());
        // Re-selecting an already chosen side must also leave Input, even
        // though neither selected image identity nor readiness changed.
        app.viewer.settings.layer = display::ViewLayer::Input;
        app.select_comparison_side(false);
        app.prepare_comparison_viewer();
        assert!(app.viewer.settings.layer == display::ViewLayer::Output);
        app.viewer.settings.layer = display::ViewLayer::Input;
        app.select_comparison_side(true);
        app.prepare_comparison_viewer();
        assert!(app.viewer.settings.layer == display::ViewLayer::Output);
        assert!(app.comparison_output().unwrap().output.is_some());
    }
    #[test]
    fn recapture_after_clear_uses_current_ordinary_render_geometry() {
        let (mut app, _directory, ctx) = app();
        retained(&mut app, 0.4, true);
        app.retained_kind = Some(RenderKind::Preview);
        app.viewer.input_size = Some([80, 40]);
        app.viewer.output_size = Some([8, 4]);
        app.viewer.settings.layer = display::ViewLayer::Output;
        app.viewer.zoom_percent = Some(100.0);
        app.viewer.pan = egui::vec2(19.0, -7.0);
        app.capture_comparison(&ctx);
        app.prepare_comparison_viewer();
        app.clear_comparison();
        app.prepare_comparison_viewer();
        assert!(!app.comparison.managed_viewer);
        assert_eq!(app.comparison.presented_size, None);
        // An ordinary worker accepts a larger Scan after A has relinquished
        // viewer ownership; its actual pixel zoom is now the starting view.
        let scan = Arc::new(ImageBuf::from_data(80, 40, vec![0.7; 80 * 40 * 3]));
        app.output_image = Some(scan.clone());
        app.retained_kind = Some(RenderKind::Scan);
        app.viewer.output_size = Some([80, 40]);
        app.invalidate_comparison_presentation();
        app.prepare_comparison_viewer();
        assert_eq!(app.viewer.zoom_percent, Some(100.0));
        app.capture_comparison(&ctx);
        app.prepare_comparison_viewer();
        assert_eq!(app.viewer.output_size, Some([80, 40]));
        assert_eq!(
            app.viewer.zoom_percent,
            Some(100.0),
            "recapture must not rescale an unchanged Scan using the old Preview"
        );
        assert_eq!(app.viewer.pan, egui::vec2(19.0, -7.0));
        assert!(Arc::ptr_eq(
            app.comparison_output().unwrap().output.unwrap(),
            &scan
        ));
    }
    #[test]
    fn gpu_selected_pending_and_failed_reference_publish_empty_canvas_then_restore_b() {
        use egui_kittest::{Harness, kittest::Queryable};

        fn composed(h: &mut Harness<'_, App>) -> crate::display::ViewerObservation {
            let deadline = Instant::now() + crate::headless::WAIT_DEADLINE;
            loop {
                h.input_mut().time = Some(100.0);
                h.step();
                let view = h.state().viewer.observation();
                if view.generation == view.accepted_generation && view.pixels.is_some() {
                    return view;
                }
                assert!(
                    Instant::now() < deadline,
                    "GPU comparison composition timed out"
                );
                crate::headless::wait_tick();
            }
        }
        fn center_color(view: &crate::display::ViewerObservation) -> egui::Color32 {
            let image = view.pixels.as_ref().unwrap();
            let p = view.image_rect.center();
            let x = ((p.x - view.viewport.min.x) / view.viewport.width() * image.size[0] as f32)
                as usize;
            let y = ((p.y - view.viewport.min.y) / view.viewport.height() * image.size[1] as f32)
                as usize;
            image.pixels[y * image.size[0] + x]
        }
        fn assert_empty_image(view: &crate::display::ViewerObservation) {
            let image = view.pixels.as_ref().unwrap();
            let interior = view.image_rect.shrink(2.0);
            let mut count = 0;
            for y in 0..image.size[1] {
                for x in 0..image.size[0] {
                    let p = egui::pos2(
                        view.viewport.min.x
                            + (x as f32 + 0.5) * view.viewport.width() / image.size[0] as f32,
                        view.viewport.min.y
                            + (y as f32 + 0.5) * view.viewport.height() / image.size[1] as f32,
                    );
                    if interior.contains(p) {
                        assert_eq!(image.pixels[y * image.size[0] + x], egui::Color32::WHITE);
                        count += 1;
                    }
                }
            }
            assert!(
                count > 0,
                "the empty selected image must occupy visible canvas pixels"
            );
        }

        let device = spektrafilm_gpu::test_support::explicit_adapter();
        let backend: Arc<dyn ComputeBackend> = Arc::new(device.derived_explicit());
        let (mut app, directory, _) = app();
        app.backend = backend.clone();
        app.viewer = crate::display::Viewer::with_backend(backend.clone());
        app.viewer.settings.reveal = false;
        app.viewer.settings.crossfade = false;
        app.viewer.settings.use_display_transform = false;
        app.refresh_input_viewer();
        let samples = ImageBuf::from_data(8, 4, vec![0.2; 8 * 4 * 3]);
        let retained = backend
            .retain_image(
                &samples,
                &spektrafilm_backend::RetainedImageMeta {
                    width: 8,
                    height: 4,
                    domain: spektrafilm_backend::SampleDomain::Encoded,
                    precision: spektrafilm_backend::Precision::host(),
                    space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
                    pixel_pitch_um: None,
                },
            )
            .unwrap();
        let saved_samples = retained.to_host(backend.as_ref()).unwrap().data;
        app.output_retained = Some(retained.clone());
        app.output_color_space = "sRGB".into();
        app.output_cctf_encoding = true;
        app.viewer
            .set_output_display_source("sRGB", true, false, None);
        app.retained_kind = Some(RenderKind::Scan);
        app.viewer.set_retained_output(Some(retained.clone()));
        app.viewer.output_size = Some([8, 4]);
        app.viewer.settings.layer = crate::display::ViewLayer::Output;
        app.dirty = false;
        app.force_preview = false;
        let mut h = Harness::builder()
            .with_size(egui::vec2(1500.0, 1400.0))
            .build_eframe(move |cc| {
                crate::theme::apply(&cc.egui_ctx);
                app
            });
        let b = composed(&mut h);
        let b_color = center_color(&b);
        assert_eq!(b_color, egui::Color32::from_gray(51));
        // Freeze the real reference worker before it can render. A's captured
        // missing material directory will then fail through the normal worker.
        h.state_mut().comparison.worker = Some(RenderWorker::new(Arc::new(
            progress::RenderProgress::default(),
        )));
        let active = h.state().comparison.worker.as_ref().unwrap().active.clone();
        let guard = active.lock();
        let real_data = h.state().data_dir.clone();
        h.state_mut().data_dir = directory.path().join("missing-reference-models");
        let ctx = h.ctx.clone();
        assert!(h.state_mut().capture_comparison(&ctx));
        h.state_mut().data_dir = real_data;
        assert_eq!(
            h.state().comparison_readiness(),
            ComparisonReadiness::Pending
        );
        let pending = composed(&mut h);
        assert_empty_image(&pending);
        assert_ne!(center_color(&pending), b_color);
        assert!(h.state().output_image.is_none());
        drop(guard);
        let deadline = Instant::now() + crate::headless::WAIT_DEADLINE;
        while h.state().comparison.job.is_some() {
            h.step();
            assert!(Instant::now() < deadline, "reference failure timed out");
            crate::headless::wait_tick();
        }
        assert!(
            h.state()
                .comparison
                .reference
                .as_ref()
                .unwrap()
                .failure
                .is_some()
        );
        assert_eq!(
            h.state().comparison_readiness(),
            ComparisonReadiness::Unavailable
        );
        let failed = composed(&mut h);
        assert_empty_image(&failed);
        h.get_by_label("B · current").simulate_click();
        h.run_steps(3);
        let restored = composed(&mut h);
        assert_eq!(center_color(&restored), b_color);
        assert!(h.state().output_image.is_none());
        assert!(Arc::ptr_eq(
            h.state()
                .output_retained
                .as_ref()
                .unwrap()
                .payload()
                .unwrap(),
            retained.payload().unwrap()
        ));
        assert_eq!(
            retained.to_host(backend.as_ref()).unwrap().data,
            saved_samples
        );
        eframe::App::on_exit(h.state_mut(), None);
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;

    #[test]
    fn failed_reference_render_keeps_live_parameters_and_retained_save_pixels() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = App::new_with_browser_path(
            Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
            None,
            None,
            None,
            directory.path().join("browser.json"),
            crate::headless::data_dir(),
        );
        app.image = Some(crate::render::InputImage::Host(Arc::new(
            ImageBuf::from_data(2, 1, vec![0.2; 6]),
        )));
        app.params.workflow.route = "input".into();
        app.reset_adjustment_history();
        let retained = Arc::new(ImageBuf::from_data(2, 1, vec![0.7; 6]));
        app.output_image = Some(retained.clone());
        let params = app.params.clone();
        let ctx = egui::Context::default();
        app.capture_comparison(&ctx);
        // Cancel initial attempt, then rerender the same captured controls with
        // an unavailable model directory to exercise the real failure route.
        app.clear_comparison();
        let data_dir = app.data_dir.clone();
        app.data_dir = directory.path().join("missing-model-data");
        assert!(app.capture_comparison(&ctx));
        app.data_dir = data_dir;
        let deadline = Instant::now() + crate::headless::WAIT_DEADLINE;
        while app.comparison.job.is_some() {
            app.poll_comparison_job();
            assert!(Instant::now() < deadline);
            crate::headless::wait_tick();
        }
        assert!(app.comparison.reference.as_ref().unwrap().failure.is_some());
        assert_eq!(app.comparison_readiness(), ComparisonReadiness::Unavailable);
        assert!(app.comparison_output().unwrap().output.is_none());
        assert_eq!(app.params, params);
        assert!(Arc::ptr_eq(app.output_image.as_ref().unwrap(), &retained));
        assert!(!app.can_undo());
    }
}
