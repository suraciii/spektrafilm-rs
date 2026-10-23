//! Preview/Scan render dispatch and acceptance of finished render jobs.
use crate::*;

use super::state::App;

impl App {
    /// The one header status line: the work unit that executes now, or the
    /// last accepted operation status when no render runs. A newer queued
    /// job only names the status once its attempt starts.
    pub(crate) fn operation_status(&self) -> String {
        // Never block on the worker's start marker: fall back to the
        // submitted job while the marker is held elsewhere.
        let active_kind = self
            .render_worker
            .active
            .try_lock()
            .and_then(|active| (*active).map(|(_, kind, _)| kind));
        let kind = active_kind.or_else(|| self.render_job.as_ref().map(|job| job.kind));
        let Some(kind) = kind else {
            return self.status.clone();
        };
        match self.render_progress.unit() {
            Some(unit) => format!("{} · {}", kind.label(), progress::stage_label(unit)),
            None => format!("{} …", kind.label()),
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
        // Report the Dye field's effective path next to the selected backend.
        let backend_name = effective_backend_name(&*self.backend, &params);
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
                        self.gui_state.display_profile().map(Path::new),
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
