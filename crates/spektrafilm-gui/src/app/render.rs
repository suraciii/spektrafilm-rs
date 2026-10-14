//! Preview/Scan render dispatch and acceptance of finished render jobs.
use crate::*;

use super::state::App;

impl App {
    pub(crate) fn render_snapshot(&mut self, params: RuntimeParams) -> RenderSnapshot {
        let key = (self.input_epoch, self.parameter_revision);
        if self
            .scanner_setup
            .as_ref()
            .is_none_or(|(cached, _)| *cached != key)
        {
            self.scanner_setup = Some((key, Arc::new(OnceLock::new())));
        }
        let scanner_setup = self
            .scanner_setup
            .as_ref()
            .expect("scanner setup slot inserted")
            .1
            .clone();
        RenderSnapshot::new(
            params,
            self.film_name.clone(),
            self.print_name.clone(),
            self.data_dir.clone(),
            self.gui_state.special().clone(),
            scanner_setup,
        )
    }

    /// Submit the latest snapshot without waiting for active computation.
    pub(crate) fn dispatch_render(&mut self, ctx: &egui::Context) {
        let t_clone = Instant::now();
        let Some(image) = self.image.as_ref().cloned() else {
            return;
        };
        let input_clone_ms = t_clone.elapsed().as_secs_f32() * 1000.0;
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
        operation.set_input_dimensions(image.width, image.height);
        operation.set_backend_requested(diagnostics::preview_backend_requested());
        let params = match self
            .current_state()
            .and_then(|state| state.runtime_params())
        {
            Ok(params) => params,
            Err(error) => {
                self.status = format!("{} state error: {error:#}", kind.label());
                operation.issue(IssueCategory::Configuration, IssueBoundary::Operation);
                self.diagnostics.finish(operation, Outcome::Failed);
                return;
            }
        };
        // Report the Dye field's effective CPU route next to the selected backend.
        let backend_name = effective_backend_name(self.backend.name(), &params);
        let parameter_revision = self.parameter_revision;
        let snapshot = self
            .render_snapshot(params)
            .with_cancellation(Some(Arc::new(AtomicBool::new(false))));
        let request = RenderRequest {
            job: RenderJob {
                input_epoch,
                parameter_revision,
                request_id: 0,
                kind,
                animate: self.force_preview
                    || self.full_scan_requested
                    || self.output_image.is_none(),
                backend_name,
                collection_mode: operation.mode(),
            },
            image,
            input_clone_ms,
            snapshot,
            backend: self.backend.clone(),
            source_metadata: self.source_metadata.clone(),
            display_enabled: self.viewer.settings.use_display_transform,
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
        loop {
            let (response_job, operation, result) = match self.render_worker.rx.try_recv() {
                Ok(response) => response,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.render_job = None;
                    self.status = "Render error: worker thread vanished".into();
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
            let backend_name = response_job.backend_name;
            match result {
                Ok(r) => {
                    let handoff = operation.scope(
                        "presentation_handoff",
                        ObservationKind::Phase,
                        Purpose::Image,
                    );
                    self.source_render = Some(SourceRender::new(
                        operation.id(),
                        Some(response_job.input_epoch),
                        Some(response_job.parameter_revision),
                        r.output.width,
                        r.output.height,
                        response_job.collection_mode != CollectionMode::Off,
                        diagnostics::color_space(&r.output_color_space),
                        r.output_cctf_encoding,
                    ));
                    let display_current = r.display_enabled
                        == self.viewer.settings.use_display_transform
                        && r.display_max_size == self.params.settings.preview_max_size;
                    self.last_input_clone_ms = r.input_clone_ms;
                    self.last_scale_ms = r.scale_ms;
                    self.last_pipeline_build_ms = r.pipeline_build_ms;
                    self.last_render_ms = r.render_ms;
                    self.last_preview_ms = r.preview_ms;
                    self.last_worker_total_ms = r.worker_total_ms;
                    self.output_color_space = r.output_color_space;
                    self.output_cctf_encoding = r.output_cctf_encoding;
                    self.output_metadata = r.source_metadata;
                    self.viewer.transform_status = r.display_status;
                    self.viewer.set_output(
                        r.preview,
                        [r.output.width as usize, r.output.height as usize],
                        ctx.input(|i| i.time),
                        response_job.animate && display_current,
                    );
                    self.viewer.set_output_display_source(
                        &self.output_color_space,
                        self.output_cctf_encoding,
                        self.viewer.settings.use_display_transform,
                        self.gui_state.display_profile().map(Path::new),
                    );
                    self.status = format!(
                        "{} · {} · {} × {} ({:.1} MP)",
                        kind.label(),
                        backend_name,
                        r.output.width,
                        r.output.height,
                        r.output.pixel_count() as f64 / 1e6
                    );
                    let output = Arc::new(r.output);
                    self.viewer.seed_prepared_output(
                        output.clone(),
                        &self.output_color_space,
                        self.output_cctf_encoding,
                        r.display_enabled,
                        None,
                        r.display_max_size,
                    );
                    self.output_image = Some(output);
                    drop(handoff);
                    self.diagnostics.finish(operation, Outcome::Succeeded);
                }
                Err(msg) => {
                    eprintln!("[spektrafilm] render error: {msg}");
                    self.status = format!("Render error: {msg}");
                    self.diagnostics.finish(operation, Outcome::Failed);
                }
            }
        }
    }
}
