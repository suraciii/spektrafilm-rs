//! Preview/Scan render dispatch and acceptance of finished render jobs.
use crate::*;

use super::state::App;

pub(crate) struct OperatorComparison {
    pub(crate) image: crate::render::InputImage,
    pub(crate) color_space: String,
    pub(crate) encoded: bool,
}

impl App {
    pub(crate) fn set_operator_compare(&mut self, comparing: bool) {
        let comparing = comparing && self.params.scanner.requires_full_frame();
        if self.operator_compare != comparing {
            self.operator_compare = comparing;
            self.operator_comparison = None;
            self.viewer.set_output_display_source(
                &self.output_color_space,
                self.output_cctf_encoding,
                self.viewer.settings.use_display_transform,
                self.gui_state.display_profile().map(Path::new),
            );
            self.viewer.invalidate_display();
            self.mark_parameters_changed();
            self.force_preview = true;
            self.dirty_since = None;
        }
    }

    pub(crate) fn accepted_frame_is_stale(&self) -> bool {
        (self.output_image.is_some() || self.output_retained.is_some())
            && self.source_render.as_ref().is_none_or(|render| {
                render.input_revision != Some(self.input_epoch)
                    || render.parameter_revision != Some(self.parameter_revision)
            })
    }

    pub(crate) fn operation_status_localized(&self, ctx: &egui::Context) -> String {
        let active_kind = self
            .render_worker
            .active
            .try_lock()
            .and_then(|active| (*active).map(|(_, kind, _)| kind));
        let Some(kind) = active_kind.or_else(|| self.render_job.as_ref().map(|job| job.kind))
        else {
            return self.status.localized(ctx);
        };
        match self.render_progress.unit() {
            Some(unit) => crate::tr!(
                ctx,
                "operations.running_stage",
                operation = kind.message().localized(ctx),
                stage = progress::stage_label_localized(ctx, unit)
            )
            .into_owned(),
            None => crate::tr!(
                ctx,
                "operations.running",
                operation = kind.message().localized(ctx)
            )
            .into_owned(),
        }
    }

    pub(crate) fn render_snapshot(&mut self, params: RuntimeParams) -> RenderSnapshot {
        RenderSnapshot::new(
            params,
            self.film_name.clone(),
            self.print_name.clone(),
            self.data_dir.clone(),
            self.gui_state.special().clone(),
        )
    }

    /// Submit the latest snapshot without waiting for active computation.
    pub(crate) fn dispatch_render(&mut self, ctx: &egui::Context) {
        let Some(image) = self.image.as_ref().cloned() else {
            return;
        };
        let kind = if self.full_scan_requested {
            RenderKind::Scan
        } else {
            RenderKind::Preview
        };
        let input_epoch = self.input_epoch;
        let mut operation = Operation::new(
            match kind {
                RenderKind::Preview => OperationKind::Preview,
                RenderKind::Scan => OperationKind::Scan,
            },
            self.diagnostics.mode,
        );
        operation.set_revisions(Some(input_epoch), Some(self.parameter_revision));
        operation.set_input_dimensions(image.width(), image.height());
        operation.set_backend_requested(diagnostics::preview_backend_requested());
        let params = match self
            .current_state()
            .and_then(|state| state.effective_runtime_params())
        {
            Ok(params) => params,
            Err(error) => {
                self.status = crate::app::Message::new("operations.render_state_error")
                    .message_arg("operation", kind.message())
                    .message_arg("detail", crate::app::Message::error(&error));
                operation.issue(IssueCategory::Configuration, IssueBoundary::Operation);
                self.diagnostics.finish(operation, Outcome::Failed);
                return;
            }
        };
        let parameter_revision = self.parameter_revision;
        let snapshot = self
            .render_snapshot(params)
            .with_operator_compare(self.operator_compare)
            .with_cancellation(Some(Arc::new(AtomicBool::new(false))));
        let request = RenderRequest {
            job: RenderJob {
                input_epoch,
                parameter_revision,
                request_id: 0,
                kind,
                animate: self.force_preview
                    || self.full_scan_requested
                    || (self.output_image.is_none() && self.output_retained.is_none()),
                backend_name: diagnostics::effective_backend_name(self.backend.as_ref(), &self.params),
                rotation_quarter_turns: self.input_rotation.rem_euclid(4) as u32,
                collection_mode: operation.mode(),
            },
            image,
            snapshot,
            backend: self.backend.clone(),
            source_metadata: self.source_metadata.clone(),
            display_enabled: self.viewer.settings.use_display_transform,
            display_profile: self.gui_state.display_profile().map(std::path::PathBuf::from),
            ctx: ctx.clone(),
            worker_wait: operation.scope("worker_wait", ObservationKind::Phase, Purpose::Image),
            operation,
        };
        let (job, replaced) = self.render_worker.submit(request);
        if let Some(operation) = replaced {
            self.diagnostics.finish(operation, Outcome::Superseded);
        }
        self.render_job = Some(job);
        self.full_scan_requested = false;
    }

    pub(crate) fn poll_render_job(&mut self, ctx: &egui::Context) {
        self.poll_comparison_job();
        loop {
            let (response_job, operation, result) = match self.render_worker.rx.try_recv() {
                Ok(response) => response,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.render_job = None;
                    self.status = crate::app::Message::new("operations.render_worker_vanished");
                    return;
                }
            };
            if !response_job.matches(
                self.render_job.as_ref(),
                self.input_epoch,
                self.parameter_revision,
            ) {
                if self
                    .render_job
                    .as_ref()
                    .is_some_and(|job| job.request_id == response_job.request_id)
                {
                    self.render_job = None;
                }
                self.diagnostics.finish(operation, Outcome::Superseded);
                continue;
            }
            self.render_job = None;
            let kind = response_job.kind;
            // A worker acceptance can replace disposable viewer artifacts, but
            // the next presentation must still honor an inspected reference.
            self.invalidate_comparison_presentation();
            match result {
                Ok(r) => {
                    let handoff = operation.scope(
                        "presentation_handoff",
                        ObservationKind::Phase,
                        Purpose::Image,
                    );
                    let (width, height) = if let Some(image) = r.retained.as_ref() {
                        (image.width(), image.height())
                    } else {
                        let image = r.output.as_ref().expect("successful render has output samples");
                        (image.width, image.height)
                    };
                    if r.operator_compare {
                        self.viewer.transform_status = r.display_status;
                        self.viewer.set_output(
                            r.preview, [width as usize, height as usize],
                            ctx.input(|i| i.time), false,
                        );
                        self.viewer.set_output_display_source(
                            &r.output_color_space, r.output_cctf_encoding,
                            self.viewer.settings.use_display_transform,
                            self.gui_state.display_profile().map(Path::new),
                        );
                        let comparison = if let Some(image) = r.retained {
                            self.viewer.set_retained_output(Some(image.clone()));
                            crate::render::InputImage::Device(image)
                        } else {
                            let image = Arc::new(r.output.expect("CPU comparison samples"));
                            self.viewer.seed_prepared_output(
                                image.clone(), &r.output_color_space, r.output_cctf_encoding,
                                r.display_enabled, self.gui_state.display_profile().map(Path::new),
                                r.display_max_size,
                            );
                            self.viewer.set_retained_output(None);
                            crate::render::InputImage::Host(image)
                        };
                        self.operator_comparison = Some(OperatorComparison {
                            image: comparison, color_space: r.output_color_space,
                            encoded: r.output_cctf_encoding,
                        });
                        self.status = crate::app::Message::new("operations.comparing_baseline");
                        drop(handoff);
                        self.diagnostics.finish(operation, Outcome::Succeeded);
                        continue;
                    }
                    self.source_render = Some(SourceRender::new(
                        operation.id(),
                        Some(response_job.input_epoch),
                        Some(response_job.parameter_revision),
                        width,
                        height,
                        response_job.collection_mode != CollectionMode::Off,
                        diagnostics::color_space(&r.output_color_space),
                        r.output_cctf_encoding,
                    ));
                    let display_current = r.display_enabled
                        == self.viewer.settings.use_display_transform
                        && r.display_max_size == self.params.settings.preview_max_size
                        && r.display_profile.as_deref() == self.gui_state.display_profile().map(Path::new);
                    self.output_color_space = r.output_color_space;
                    self.output_cctf_encoding = r.output_cctf_encoding;
                    self.output_metadata = r.source_metadata;
                    self.viewer.transform_status = r.display_status;
                    self.viewer.set_output(
                        r.preview,
                        [width as usize, height as usize],
                        ctx.input(|i| i.time),
                        response_job.animate && display_current,
                    );
                    self.viewer.set_output_display_source(
                        &self.output_color_space,
                        self.output_cctf_encoding,
                        self.viewer.settings.use_display_transform,
                        self.gui_state.display_profile().map(Path::new),
                    );
                    self.status = crate::app::Message::new("operations.rendered")
                        .message_arg("operation", kind.message())
                        .arg("width", width)
                        .arg("height", height)
                        .arg("megapixels", format!("{:.1}", width as f64 * height as f64 / 1e6));
                    let output = r.output.map(Arc::new);
                    if let Some(output) = output.as_ref() {
                        self.viewer.seed_prepared_output(
                            output.clone(), &self.output_color_space,
                            self.output_cctf_encoding, r.display_enabled,
                            self.gui_state.display_profile().map(Path::new), r.display_max_size,
                        );
                    }
                    self.viewer.set_retained_output(r.retained.clone());
                    self.output_retained = r.retained;
                    self.output_image = output;
                    self.output_histogram = r.histogram;
                    self.retained_kind = Some(kind);
                    drop(handoff);
                    self.diagnostics.finish(operation, Outcome::Succeeded);
                }
                Err(msg) => {
                    eprintln!("[spektrafilm] render error: {msg}");
                    self.status = crate::app::Message::new("operations.render_error")
                        .message_arg("detail", msg);
                    self.diagnostics.finish(operation, Outcome::Failed);
                }
            }
        }
    }
}
