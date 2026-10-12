use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use eframe::egui;
use parking_lot::Mutex;
use spektrafilm_core::image_io::{self};
use spektrafilm_core::telemetry::OperationKind;
use spektrafilm_core::telemetry::{IssueBoundary, IssueCategory};
use spektrafilm_core::telemetry::{Operation, Outcome};
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_gpu::telemetry::{ObservationKind, Purpose};
use spektrafilm_math::image::ImageBuf;

use super::diagnostics::BackendRequested;
use super::export::{ExportBackend, ExportOptions};
use super::{App, effective_backend_name, observed_result};

/// One in-flight export owning an immutable input, pipeline and option snapshot.
/// Cancellation discards its staged file once the current operation completes.
/// The UI alone publishes successful, uncancelled work.
pub(super) struct ExportJob {
    pub(super) rx: mpsc::Receiver<Result<ExportResult, String>>,
    pub(super) handle: Option<JoinHandle<()>>,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) started_at: Instant,
    pub(super) configuration:
        Option<Arc<Mutex<Option<spektrafilm_core::telemetry::RenderConfiguration>>>>,
    pub(super) backend: ExportBackend,
    pub(super) operation: Operation,
}

pub(super) struct ExportResult {
    pub(super) elapsed: f32,
    pub(super) filename: String,
    pub(super) backend_name: String,
    pub(super) size: [u32; 2],
    pub(super) staged: image_io::PreparedImageOutput,
}

impl App {
    /// Render an immutable snapshot of the loaded full input independently of Save.
    pub(super) fn start_export(&mut self, ctx: &egui::Context, mut options: ExportOptions) {
        if self.export_input().is_none() {
            return;
        }
        options.normalize();
        let stem = self
            .image_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("spektrafilm");
        let default_name = format!(
            "{stem}_{}_spektra_{}.{}",
            self.film_name,
            options.backend.argument(),
            options.format.extension()
        );
        let Some(chosen_path) = self
            .file_dialog("export")
            .add_filter(options.format.label(), options.format.extensions())
            .set_file_name(&default_name)
            .save_file()
        else {
            return;
        };
        self.start_export_to_path(ctx, options, &chosen_path);
    }

    pub(super) fn export_input(&mut self) -> Option<Arc<ImageBuf>> {
        if self.export_job.is_some() || self.calibration_job.is_some() {
            self.status = "Wait for the current operation before exporting.".into();
            return None;
        }
        match self.image.as_ref().cloned() {
            Some(image) => Some(image),
            None => {
                self.status = "Load an image before exporting.".into();
                None
            }
        }
    }

    /// Render to the native chooser's confirmed path, preserving overwrite checks.
    pub(super) fn start_export_to_path(
        &mut self,
        ctx: &egui::Context,
        mut options: ExportOptions,
        chosen_path: &Path,
    ) {
        let Some(image) = self.export_input() else {
            return;
        };
        options.normalize();
        let mut operation = Operation::new(OperationKind::Export, self.diagnostics.mode);
        operation.set_revisions(Some(self.input_epoch), Some(self.parameter_revision));
        operation.set_input_dimensions(image.width, image.height);
        operation.set_backend_requested(match options.backend {
            ExportBackend::Cpu => BackendRequested::Cpu,
            ExportBackend::Gpu => BackendRequested::Wgpu,
        });
        if operation.enabled() {
            operation.set_saving_configuration(
                spektrafilm_core::telemetry::SavingConfiguration::from_save_options(
                    options.image_format(),
                    &options.save_options(),
                ),
            );
        }
        let out_path = options.format.output_path(chosen_path);
        if out_path != chosen_path && out_path.exists() {
            self.status = format!(
                "Export destination already exists: {}. Choose that exact filename to confirm replacement.",
                out_path.display()
            );
            operation.issue(IssueCategory::Publication, IssueBoundary::Operation);
            self.diagnostics.finish(operation, Outcome::Failed);
            return;
        }
        let params = match self
            .current_state()
            .and_then(|state| state.runtime_params())
        {
            Ok(params) => params,
            Err(error) => {
                self.status = format!("Export state error: {error:#}");
                operation.issue(IssueCategory::Configuration, IssueBoundary::Operation);
                self.diagnostics.finish(operation, Outcome::Failed);
                return;
            }
        };
        let snapshot = self.render_snapshot(params);
        self.remember_dialog("export", &out_path);
        self.export_options = options.clone();
        let metadata = self.source_metadata.clone();
        let export_backend = options.backend;
        let selected_backend = self.backend.clone();
        let (tx, rx) = mpsc::channel();
        let ctx_for_worker = ctx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_worker = Arc::clone(&cancel);
        let started_at = Instant::now();
        let configuration = operation.enabled().then(|| Arc::new(Mutex::new(None)));
        let worker_configuration = configuration.clone();
        let attempt = operation.attempt(1);
        if operation.enabled() {
            operation.set_saved_attempt(attempt.id());
        }
        let observation = attempt.context().clone();
        let worker_wait = operation.scope("worker_wait", ObservationKind::Phase, Purpose::Image);
        let handle = std::thread::Builder::new()
            .name("spektrafilm-export".into())
            .spawn(move || {
                drop(worker_wait);
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<ExportResult> {
                        if cancel_for_worker.load(Ordering::SeqCst) {
                            anyhow::bail!("cancelled");
                        }
                        let setup = snapshot
                            .prepare_scanner_setup(&image, &observation)
                            .map_err(anyhow::Error::msg)?;
                        let mut pipeline =
                            super::build_runtime_observed(&snapshot, false, &observation)
                                .map_err(anyhow::Error::msg)?;
                        if let Some(setup) = setup {
                            pipeline = pipeline.with_scanner_setup(setup);
                        }
                        if let Some(slot) = &worker_configuration {
                            *slot.lock() = Some(pipeline.telemetry_configuration());
                        }
                        if cancel_for_worker.load(Ordering::SeqCst) {
                            anyhow::bail!("cancelled");
                        }
                        let backend_init = observation.scope(
                            "backend_init",
                            ObservationKind::Phase,
                            Purpose::Image,
                        );
                        let backend_result: Result<Arc<dyn ComputeBackend>> = match export_backend {
                            ExportBackend::Cpu => {
                                Ok(Arc::new(spektrafilm_gpu::cpu_backend::CpuBackend))
                            }
                            ExportBackend::Gpu if selected_backend.is_gpu() => Ok(selected_backend),
                            ExportBackend::Gpu => spektrafilm_gpu::wgpu_backend::WgpuBackend::new()
                                .map(|backend| Arc::new(backend) as Arc<dyn ComputeBackend>)
                                .context("GPU export requested but no WGPU adapter is available"),
                        };
                        let backend = observed_result(
                            backend_init,
                            backend_result,
                            IssueCategory::Backend,
                            IssueBoundary::BackendInit,
                        )?;
                        let input_prepare = observation.scope(
                            "input_prepare",
                            ObservationKind::Phase,
                            Purpose::Image,
                        );
                        let working_image = (*image).clone();
                        drop(input_prepare);
                        let simulation =
                            observation.scope("simulation", ObservationKind::Phase, Purpose::Image);
                        let processed = pipeline
                            .process_observed(working_image, backend.as_ref(), simulation.context())
                            .map_err(anyhow::Error::msg);
                        let output = observed_result(
                            simulation,
                            processed,
                            IssueCategory::Simulation,
                            IssueBoundary::Simulation,
                        )?;
                        if cancel_for_worker.load(Ordering::SeqCst) {
                            anyhow::bail!("cancelled");
                        }
                        let saving_convert = observation.scope(
                            "saving_convert",
                            ObservationKind::Phase,
                            Purpose::Image,
                        );
                        let converted = image_io::convert_image(
                            &output,
                            &pipeline.params().io.output_color_space,
                            pipeline.params().io.output_cctf_encoding,
                            &options.saving_color_space,
                            options.saving_cctf_encoding,
                        );
                        let converted = observed_result(
                            saving_convert,
                            converted,
                            IssueCategory::Writing,
                            IssueBoundary::SavingConvert,
                        )?;
                        let file_write =
                            observation.scope("file_write", ObservationKind::Phase, Purpose::Image);
                        let staged = image_io::prepare_save_observed(
                            &out_path,
                            &converted,
                            options.save_options(),
                            metadata.as_ref(),
                            file_write.context(),
                        );
                        let staged = observed_result(
                            file_write,
                            staged,
                            IssueCategory::Writing,
                            IssueBoundary::FileWrite,
                        )?;
                        if cancel_for_worker.load(Ordering::SeqCst) {
                            anyhow::bail!("cancelled");
                        }
                        Ok(ExportResult {
                            elapsed: started_at.elapsed().as_secs_f32(),
                            filename: out_path
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("(file)")
                                .to_owned(),
                            backend_name: effective_backend_name(backend.name(), pipeline.params()),
                            size: [output.width, output.height],
                            staged,
                        })
                    },
                ))
                .unwrap_or_else(|panic| Err(anyhow::anyhow!(super::panic_message(&panic))));
                attempt.finish(if res.is_ok() {
                    Outcome::Succeeded
                } else if cancel_for_worker.load(Ordering::SeqCst) {
                    Outcome::Cancelled
                } else {
                    Outcome::Failed
                });
                let msg = res.map_err(|e| format!("{e:#}"));
                let _ = tx.send(msg);
                ctx_for_worker.request_repaint();
            })
            .expect("OS thread spawn");
        self.status = format!("Exporting with {}…", export_backend.label());
        self.export_job = Some(ExportJob {
            rx,
            handle: Some(handle),
            cancel,
            started_at,
            backend: export_backend,
            operation,
            configuration,
        });
    }

    /// Cancellation discards the staged file after the current pipeline operation.
    pub(super) fn cancel_export(&mut self) {
        let Some(job) = self.export_job.as_ref() else {
            return;
        };
        if !job.cancel.swap(true, Ordering::SeqCst) {
            self.status = "Cancelling export…".into();
        }
    }

    /// Called once per `update()`. If the export is still in-flight,
    /// refreshes the status with elapsed time and requests a repaint a
    /// second from now (so the timer ticks without us busy-looping).
    /// If the job finished, drains the result into the status bar and
    /// joins the worker thread.
    pub(super) fn poll_export_job(&mut self, ctx: &egui::Context) {
        let Some(job) = self.export_job.as_mut() else {
            return;
        };
        let msg = match job.rx.try_recv() {
            Ok(m) => m,
            Err(mpsc::TryRecvError::Empty) => {
                let secs = job.started_at.elapsed().as_secs();
                self.status = if job.cancel.load(Ordering::SeqCst) {
                    format!("Cancelling export… ({secs}s)")
                } else {
                    format!("Exporting with {}… ({secs}s)", job.backend.label())
                };
                ctx.request_repaint_after(Duration::from_secs(1));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.status = "Export error: worker thread vanished".into();
                let mut job = self.export_job.take().expect("export job exists");
                if let Some(handle) = job.handle.take() {
                    let _ = handle.join();
                }
                if let Some(configuration) = job
                    .configuration
                    .as_ref()
                    .and_then(|slot| slot.lock().take())
                {
                    job.operation.set_render_configuration(configuration);
                }
                job.operation
                    .issue(IssueCategory::Backend, IssueBoundary::Operation);
                self.diagnostics.finish(job.operation, Outcome::Failed);
                return;
            }
        };
        let mut job = self.export_job.take().expect("export job exists");
        if let Some(h) = job.handle.take() {
            let _ = h.join();
        }
        let cancelled = job.cancel.load(Ordering::SeqCst);
        let mut operation = job.operation;
        if let Some(configuration) = job
            .configuration
            .as_ref()
            .and_then(|slot| slot.lock().take())
        {
            operation.set_render_configuration(configuration);
        }
        let mut outcome = if cancelled {
            Outcome::Cancelled
        } else {
            Outcome::Failed
        };
        self.status = match msg {
            Ok(_) if cancelled => "Export cancelled.".into(),
            Ok(result) => {
                let publication =
                    operation.scope("publication", ObservationKind::Phase, Purpose::Image);
                let published = result.staged.publish(true);
                let published = observed_result(
                    publication,
                    published,
                    IssueCategory::Publication,
                    IssueBoundary::Publication,
                );
                match published {
                    Ok(report) => {
                        outcome = Outcome::Succeeded;
                        operation.set_published_dimensions(result.size[0], result.size[1]);
                        let mut status = format!(
                            "Exported ({}) {} · {} × {} in {:.1} s",
                            result.backend_name,
                            result.filename,
                            result.size[0],
                            result.size[1],
                            result.elapsed
                        );
                        if let Some(warning) = report.metadata_warning {
                            status.push_str(&format!(" — Metadata warning: {warning}"));
                        }
                        status
                    }
                    Err(e) => format!("Export error: {e:#}"),
                }
            }
            Err(_) if cancelled => "Export cancelled.".into(),
            Err(e) => format!("Export error: {e}"),
        };
        if cancelled {
            operation.issue(IssueCategory::Cancelled, IssueBoundary::Operation);
        }
        self.diagnostics.finish(operation, outcome);
    }
}
