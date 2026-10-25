//! Session-local submissions. Only the shared export owner renders and publishes.
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
use std::sync::Arc;

use anyhow::Result;
use eframe::egui;
use spektrafilm_core::image_io::ImageMetadata;
use spektrafilm_core::telemetry::{OperationKind, Outcome};
#[cfg(test)]
use spektrafilm_math::image::ImageBuf;

use crate::diagnostics::BackendRequested;
use crate::export::{ExportBackend, ExportOptions};
use crate::photograph::record::Dependency;
use crate::render::RenderSnapshot;

use super::{App, Message};

/// Immutable decoded samples and accepted interpretation/rotation in the snapshot.
#[derive(Clone)]
pub(crate) struct QueueSubmission {
    pub(crate) source_name: String,
    pub(crate) image: crate::render::InputImage,
    pub(crate) rotation_quarter_turns: u32,
    pub(crate) snapshot: RenderSnapshot,
    pub(crate) metadata: Option<ImageMetadata>,
    pub(crate) options: ExportOptions,
    pub(crate) destination: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueueState {
    Waiting,
    Running,
    Completed,
    Failed,
    Canceled,
}

impl QueueState {
    fn label(self) -> &'static str {
        match self {
            Self::Waiting => "photo.queue.waiting",
            Self::Running => "photo.queue.running",
            Self::Completed => "photo.queue.completed",
            Self::Failed => "photo.queue.failed",
            Self::Canceled => "photo.queue.canceled",
        }
    }

    fn pending(self) -> bool {
        matches!(self, Self::Waiting | Self::Running)
    }
}

struct QueueEntry {
    submission: QueueSubmission,
    destination_key: PathBuf,
    replace_existing: bool,
    state: QueueState,
    outcome: Option<Message>,
    dependencies: Result<Vec<Dependency>, Message>,
}

#[derive(Default)]
pub(crate) struct ExportQueue {
    entries: Vec<QueueEntry>,
    #[cfg(test)]
    next_pause: Option<Arc<crate::export_job::ExportPause>>,
}

/// Resolve existing ancestors, including symlinks, while accepting a missing
/// destination. Doing this at submission also freezes relative paths against cwd.
fn normalized_destination(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
        match std::fs::canonicalize(&normalized) {
            Ok(existing) => normalized = existing,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(normalized)
}

impl App {
    /// Validate the complete batch before accepting any entry. An explicit
    /// replacement decision is retained through publication, including retries.
    pub(crate) fn enqueue_exports(
        &mut self,
        ctx: &egui::Context,
        submissions: Vec<QueueSubmission>,
        replace_existing: bool,
    ) -> Result<()> {
        let mut destinations: HashSet<PathBuf> = self
            .export_queue
            .entries
            .iter()
            .filter(|entry| entry.state.pending())
            .map(|entry| entry.destination_key.clone())
            .collect();
        if let Some(job) = &self.export_job {
            destinations.insert(normalized_destination(&job.destination)?);
        }
        let mut accepted = Vec::with_capacity(submissions.len());
        for mut submission in submissions {
            submission.options.normalize();
            let destination = submission
                .options
                .format
                .output_path(&submission.destination);
            let filename = destination.file_name().ok_or_else(|| {
                anyhow::Error::new(Message::new("photo.queue.invalid_destination"))
            })?;
            // Canonicalize the parent, not the final filename: publication replaces
            // the selected directory entry, never follows a final symlink.
            let parent = normalized_destination(destination.parent().unwrap_or(Path::new(".")))?;
            submission.destination = parent.join(filename);
            let key = normalized_destination(&submission.destination)?;
            if !destinations.insert(key.clone()) {
                return Err(anyhow::Error::new(
                    Message::new("photo.queue.duplicate_destination")
                        .arg("path", submission.destination.display()),
                ));
            }
            if !replace_existing && submission.destination.try_exists()? {
                return Err(anyhow::Error::new(
                    Message::new("photo.queue.replace_required")
                        .arg("path", submission.destination.display()),
                ));
            }
            let dependencies = submission
                .snapshot
                .export_dependencies()
                .map_err(|error| Message::error(&error));
            accepted.push(QueueEntry {
                submission,
                destination_key: key,
                replace_existing,
                state: QueueState::Waiting,
                outcome: None,
                dependencies,
            });
        }
        self.export_queue.entries.extend(accepted);
        self.start_next_queued_export(ctx);
        ctx.request_repaint();
        Ok(())
    }

    pub(crate) fn start_next_queued_export(&mut self, ctx: &egui::Context) {
        if self.export_job.is_some() || self.calibration_job.is_some() {
            return;
        }
        let Some(index) = self
            .export_queue
            .entries
            .iter()
            .position(|entry| entry.state == QueueState::Waiting)
        else {
            return;
        };
        let entry = &mut self.export_queue.entries[index];
        entry.state = QueueState::Running;
        let submission = entry.submission.clone();
        let replace_existing = entry.replace_existing;
        let dependencies = entry.dependencies.clone();
        let mut operation = self.diagnostics.begin(OperationKind::Export);
        // Queued work belongs to the submission, not the currently active source.
        operation.set_input_dimensions(submission.image.width(), submission.image.height());
        operation.set_backend_requested(match submission.options.backend {
            ExportBackend::Cpu => BackendRequested::Cpu,
            ExportBackend::Gpu => BackendRequested::Wgpu,
        });
        if operation.enabled() {
            operation.set_saving_configuration(
                spektrafilm_core::telemetry::SavingConfiguration::from_save_options(
                    submission.options.image_format(),
                    &submission.options.save_options(),
                ),
            );
        }
        #[cfg(test)]
        let pause = self.export_queue.next_pause.take();
        self.launch_export(
            ctx,
            submission,
            replace_existing,
            Some((index, dependencies)),
            operation,
            #[cfg(test)]
            pause,
        );
    }

    pub(crate) fn finish_queued_export(
        &mut self,
        index: Option<usize>,
        outcome: Outcome,
        message: Message,
    ) {
        let Some(index) = index else {
            return;
        };
        let entry = &mut self.export_queue.entries[index];
        entry.state = match outcome {
            Outcome::Succeeded => QueueState::Completed,
            Outcome::Cancelled => QueueState::Canceled,
            _ => QueueState::Failed,
        };
        entry.outcome = Some(message);
    }

    pub(crate) fn cancel_queued_export(&mut self, index: usize) {
        let Some(entry) = self.export_queue.entries.get_mut(index) else {
            return;
        };
        match entry.state {
            QueueState::Waiting => {
                entry.state = QueueState::Canceled;
                entry.outcome = Some(Message::new("operations.export_cancelled"));
            }
            QueueState::Running => self.cancel_export(),
            _ => {}
        }
    }

    pub(crate) fn cancel_remaining_exports(&mut self) {
        for entry in &mut self.export_queue.entries {
            if entry.state == QueueState::Waiting {
                entry.state = QueueState::Canceled;
                entry.outcome = Some(Message::new("operations.export_cancelled"));
            }
        }
        if self
            .export_job
            .as_ref()
            .is_some_and(|job| job.queue_entry.is_some())
        {
            self.cancel_export();
        }
    }

    /// Repeat the failed submission, never read current source/controls/options.
    pub(crate) fn retry_queued_export(&mut self, ctx: &egui::Context, index: usize) -> Result<()> {
        let Some(entry) = self.export_queue.entries.get(index) else {
            return Ok(());
        };
        if entry.state != QueueState::Failed {
            return Ok(());
        }
        let key = normalized_destination(&entry.submission.destination)?;
        let duplicate = self
            .export_queue
            .entries
            .iter()
            .any(|other| other.state.pending() && other.destination_key == key);
        let interactive_duplicate = self
            .export_job
            .as_ref()
            .map(|job| normalized_destination(&job.destination))
            .transpose()?
            .is_some_and(|destination| destination == key);
        if duplicate || interactive_duplicate {
            return Err(anyhow::Error::new(
                Message::new("photo.queue.duplicate_destination")
                    .arg("path", entry.submission.destination.display()),
            ));
        }
        let entry = &mut self.export_queue.entries[index];
        entry.destination_key = key;
        entry.state = QueueState::Waiting;
        entry.outcome = None;
        self.start_next_queued_export(ctx);
        ctx.request_repaint();
        Ok(())
    }

    pub(crate) fn export_queue_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading(crate::i18n::text(ui.ctx(), "photo.queue.title"));
        ui.label(crate::i18n::text(ui.ctx(), "photo.queue.snapshot_help"));
        if self.export_queue.entries.is_empty() {
            ui.label(crate::i18n::text(ui.ctx(), "photo.queue.empty"));
            return;
        }
        let remaining = self
            .export_queue
            .entries
            .iter()
            .any(|entry| entry.state.pending());
        let cancel_all = ui.add_enabled(
            remaining,
            egui::Button::new(crate::i18n::text(ui.ctx(), "photo.queue.cancel_remaining")),
        );
        crate::controls::locate_row(ui, "photo.queue.cancel_remaining", &cancel_all);
        if cancel_all.clicked() {
            self.cancel_remaining_exports();
        }
        let mut cancel = None;
        let mut retry = None;
        for (index, entry) in self.export_queue.entries.iter().enumerate() {
            ui.push_id(("export-queue", index), |ui| {
                ui.group(|ui| {
                    ui.label(&entry.submission.source_name);
                    ui.label(entry.submission.destination.display().to_string());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(crate::i18n::text(ui.ctx(), entry.state.label()));
                        if entry.state.pending() {
                            let response =
                                ui.button(crate::i18n::text(ui.ctx(), "photo.queue.cancel"));
                            crate::controls::locate_row(ui, "photo.queue.cancel", &response);
                            if response.clicked() {
                                cancel = Some(index);
                            }
                        }
                        if entry.state == QueueState::Failed {
                            let response = ui
                                .button(crate::i18n::text(ui.ctx(), "photo.queue.retry_snapshot"));
                            crate::controls::locate_row(ui, "photo.queue.retry", &response);
                            if response.clicked() {
                                retry = Some(index);
                            }
                        }
                    });
                    if let Some(outcome) = &entry.outcome {
                        ui.label(outcome.localized(ui.ctx()));
                    }
                });
            });
        }
        if let Some(index) = cancel {
            self.cancel_queued_export(index);
        }
        if let Some(index) = retry
            && let Err(error) = self.retry_queued_export(ui.ctx(), index)
        {
            self.status = Message::error(&error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_core::image_io;
    use spektrafilm_core::params::RuntimeParams;
    use std::time::{Duration, Instant};

    fn app() -> (App, tempfile::TempDir, egui::Context) {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state.json");
        crate::state::GuiState::factory().save(&state).unwrap();
        let mut app = App::new_with_browser_path(
            Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
            None,
            Some(state),
            None,
            directory.path().join("browser.json"),
            crate::headless::data_dir(),
        );
        app.film_name = "kodak_portra_400".into();
        app.print_name = "kodak_portra_endura".into();
        app.params = RuntimeParams::default();
        app.params.workflow.route = "input > film > scan".into();
        app.params.io.scan_film = true;
        app.params.scanner.scan_output = "negative_to_positive".into();
        app.params.film_render.grain.active = false;
        app.params.camera.auto_exposure = false;
        app.params.print_render.glare.active = false;
        app.image = Some(crate::render::InputImage::Host(Arc::new(
            ImageBuf::from_data(
                24,
                8,
                (0..24 * 8 * 3)
                    .map(|index| {
                        spektrafilm_math::precision::from_f64(if index / 3 % 24 < 5 {
                            0.01
                        } else {
                            0.7
                        })
                    })
                    .collect(),
            ),
        )));
        (app, directory, egui::Context::default())
    }

    fn submission(app: &App, destination: PathBuf) -> QueueSubmission {
        QueueSubmission {
            source_name: "accepted source".into(),
            image: app.image.as_ref().unwrap().clone(),
            rotation_quarter_turns: app.input_rotation.rem_euclid(4) as u32,
            snapshot: RenderSnapshot::new(
                app.params.clone(),
                app.film_name.clone(),
                app.print_name.clone(),
                app.data_dir.clone(),
                serde_json::json!({}),
            ),
            metadata: None,
            options: app.export_options.clone(),
            destination,
        }
    }

    fn drain(app: &mut App, ctx: &egui::Context) {
        let deadline = Instant::now() + crate::headless::WAIT_DEADLINE;
        while app.export_job.is_some()
            || app
                .export_queue
                .entries
                .iter()
                .any(|entry| entry.state.pending())
        {
            assert!(Instant::now() < deadline, "real export queue did not drain");
            app.poll_export_job(ctx);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn submission_resolves_destinations_atomically_and_requires_replacement_choice() {
        let (mut app, directory, ctx) = app();
        let path = directory.path().join("duplicate.png");
        let first = submission(&app, path.clone());
        let second = submission(&app, directory.path().join("./duplicate.other"));
        assert!(
            app.enqueue_exports(&ctx, vec![first, second], false)
                .is_err()
        );
        assert!(app.export_queue.entries.is_empty());
        assert!(app.export_job.is_none());
        std::fs::write(&path, b"existing file").unwrap();
        let item = submission(&app, path.clone());
        assert!(
            app.enqueue_exports(&ctx, vec![item.clone()], false)
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"existing file");
        app.enqueue_exports(&ctx, vec![item], true).unwrap();
        drain(&mut app, &ctx);
        assert_eq!(app.export_queue.entries[0].state, QueueState::Completed);
        assert!(image_io::load(&path).is_ok());
    }

    #[test]
    fn canceled_and_failed_entries_do_not_stop_other_real_exports() {
        let (mut app, directory, ctx) = app();
        let canceled = directory.path().join("canceled.png");
        let waiting = directory.path().join("waiting.png");
        let failed = directory.path().join("failed.png");
        let completed = directory.path().join("completed.png");
        std::fs::write(&canceled, b"keep running destination").unwrap();
        std::fs::write(&waiting, b"keep waiting destination").unwrap();
        std::fs::write(&failed, b"keep failed destination").unwrap();
        let mut bad = submission(&app, failed.clone());
        bad.snapshot = RenderSnapshot::new(
            app.params.clone(),
            "missing-material".into(),
            app.print_name.clone(),
            app.data_dir.clone(),
            serde_json::json!({}),
        );
        let items = vec![
            submission(&app, canceled.clone()),
            bad,
            submission(&app, waiting.clone()),
            submission(&app, completed.clone()),
        ];
        let (staged, ready) = std::sync::mpsc::channel();
        let pause = Arc::new(crate::export_job::ExportPause {
            staged,
            released: (parking_lot::Mutex::new(false), parking_lot::Condvar::new()),
            cancelled: (parking_lot::Mutex::new(false), parking_lot::Condvar::new()),
        });
        app.export_queue.next_pause = Some(pause.clone());
        app.enqueue_exports(&ctx, items, true).unwrap();
        ready
            .recv_timeout(crate::headless::WAIT_DEADLINE)
            .expect("real queue writer did not stage output");
        app.cancel_queued_export(0);
        app.cancel_queued_export(2);
        pause.release();
        drain(&mut app, &ctx);
        assert_eq!(
            app.export_queue
                .entries
                .iter()
                .map(|entry| entry.state)
                .collect::<Vec<_>>(),
            vec![
                QueueState::Canceled,
                QueueState::Failed,
                QueueState::Canceled,
                QueueState::Completed
            ]
        );
        assert_eq!(
            std::fs::read(&canceled).unwrap(),
            b"keep running destination"
        );
        assert_eq!(
            std::fs::read(&waiting).unwrap(),
            b"keep waiting destination"
        );
        assert_eq!(std::fs::read(&failed).unwrap(), b"keep failed destination");
        assert!(image_io::load(&completed).is_ok());
    }

    #[test]
    fn publication_refuses_a_file_created_after_nonreplacement_submission() {
        let (mut app, directory, ctx) = app();
        let destination = directory.path().join("raced.png");
        let item = submission(&app, destination.clone());
        app.enqueue_exports(&ctx, vec![item], false).unwrap();
        let deadline = Instant::now() + crate::headless::WAIT_DEADLINE;
        while !app
            .export_job
            .as_ref()
            .unwrap()
            .handle
            .as_ref()
            .unwrap()
            .is_finished()
        {
            assert!(
                Instant::now() < deadline,
                "real writer did not finish staging"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::write(&destination, b"concurrent user file").unwrap();
        drain(&mut app, &ctx);
        assert_eq!(app.export_queue.entries[0].state, QueueState::Failed);
        assert_eq!(std::fs::read(destination).unwrap(), b"concurrent user file");
    }

    #[test]
    fn retry_renders_exact_failed_submission_after_live_source_and_settings_change() {
        let (mut app, directory, ctx) = app();
        let missing_parent = directory.path().join("not-created-yet");
        let destination = missing_parent.join("retry.png");
        let item = submission(&app, destination.clone());
        let mut expected = item.clone();
        expected.destination = directory.path().join("reference.png");
        let submitted_params = serde_json::to_value(item.snapshot.params()).unwrap();
        let submitted_options = item.options.clone();
        let submitted_image = item.image.clone();
        app.enqueue_exports(&ctx, vec![item], false).unwrap();
        drain(&mut app, &ctx);
        assert_eq!(app.export_queue.entries[0].state, QueueState::Failed);
        app.image = Some(crate::render::InputImage::Host(Arc::new(ImageBuf::new(
            2, 2,
        ))));
        app.params.camera.exposure_compensation_ev += 3.0;
        app.film_name = "unrelated-live-film".into();
        app.export_options.jpeg_quality = 23;
        std::fs::create_dir(&missing_parent).unwrap();
        app.retry_queued_export(&ctx, 0).unwrap();
        app.enqueue_exports(&ctx, vec![expected], false).unwrap();
        drain(&mut app, &ctx);
        assert_eq!(app.export_queue.entries[0].state, QueueState::Completed);
        let captured = &app.export_queue.entries[0].submission;
        assert!(captured.image.same_source(&submitted_image));
        assert_eq!(
            serde_json::to_value(captured.snapshot.params()).unwrap(),
            submitted_params
        );
        assert_eq!(captured.options, submitted_options);
        assert_eq!(
            image_io::load(&destination).unwrap().image.data,
            image_io::load(&directory.path().join("reference.png"))
                .unwrap()
                .image
                .data
        );
        assert_eq!(app.image.as_ref().unwrap().width(), 2);
        assert_eq!(app.film_name, "unrelated-live-film");
        assert_eq!(app.export_options.jpeg_quality, 23);
    }

    #[test]
    fn interactive_and_queued_exports_serialize_and_cancel_remaining_keeps_completed_files() {
        let (mut app, directory, ctx) = app();
        let interactive = directory.path().join("interactive.png");
        let first = directory.path().join("first.png");
        let remaining = directory.path().join("remaining.png");
        app.start_export_to_path(&ctx, app.export_options.clone(), &interactive);
        let items = vec![
            submission(&app, first.clone()),
            submission(&app, remaining.clone()),
        ];
        app.enqueue_exports(&ctx, items, false).unwrap();
        assert_eq!(app.export_queue.entries[0].state, QueueState::Waiting);
        assert!(app.export_job.as_ref().unwrap().queue_entry.is_none());
        let deadline = Instant::now() + crate::headless::WAIT_DEADLINE;
        while app.export_queue.entries[0].state != QueueState::Completed {
            assert!(
                Instant::now() < deadline,
                "real queue did not complete first export"
            );
            app.poll_export_job(&ctx);
            std::thread::sleep(Duration::from_millis(10));
        }
        app.cancel_remaining_exports();
        drain(&mut app, &ctx);
        assert!(image_io::load(&interactive).is_ok());
        assert!(image_io::load(&first).is_ok());
        assert_eq!(app.export_queue.entries[1].state, QueueState::Canceled);
        assert!(!remaining.exists());
    }

    #[cfg(unix)]
    #[test]
    fn changed_material_fails_only_its_submission_and_retry_keeps_original_identity() {
        let (mut app, directory, ctx) = app();
        let data = directory.path().join("isolated-data");
        std::fs::create_dir(&data).unwrap();
        for entry in std::fs::read_dir(&app.data_dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() != "profiles" {
                std::os::unix::fs::symlink(entry.path(), data.join(entry.file_name())).unwrap();
            }
        }
        let profiles = data.join("profiles");
        std::fs::create_dir(&profiles).unwrap();
        let selected = format!("{}.json", app.film_name);
        for entry in std::fs::read_dir(app.data_dir.join("profiles")).unwrap() {
            let entry = entry.unwrap();
            let target = profiles.join(entry.file_name());
            if entry.file_name() == selected.as_str() {
                std::fs::copy(entry.path(), &target).unwrap();
            } else {
                std::os::unix::fs::symlink(entry.path(), target).unwrap();
            }
        }
        let material = profiles.join(selected);
        let original = std::fs::read(&material).unwrap();
        let affected_path = directory.path().join("changed-material.png");
        let unrelated_path = directory.path().join("unrelated.png");
        let mut affected = submission(&app, affected_path.clone());
        affected.snapshot = RenderSnapshot::new(
            app.params.clone(),
            app.film_name.clone(),
            app.print_name.clone(),
            data,
            serde_json::json!({}),
        );
        let items = vec![
            submission(&app, directory.path().join("blocker.png")),
            affected,
            submission(&app, unrelated_path.clone()),
        ];
        let (staged, ready) = std::sync::mpsc::channel();
        let pause = Arc::new(crate::export_job::ExportPause {
            staged,
            released: (parking_lot::Mutex::new(false), parking_lot::Condvar::new()),
            cancelled: (parking_lot::Mutex::new(false), parking_lot::Condvar::new()),
        });
        app.export_queue.next_pause = Some(pause.clone());
        app.enqueue_exports(&ctx, items, false).unwrap();
        ready.recv_timeout(crate::headless::WAIT_DEADLINE).unwrap();
        let mut changed = original.clone();
        changed.push(b' ');
        std::fs::write(&material, changed).unwrap();
        pause.release();
        drain(&mut app, &ctx);
        assert_eq!(app.export_queue.entries[1].state, QueueState::Failed);
        assert!(!affected_path.exists());
        assert!(image_io::load(&unrelated_path).is_ok());
        app.retry_queued_export(&ctx, 1).unwrap();
        drain(&mut app, &ctx);
        assert_eq!(app.export_queue.entries[1].state, QueueState::Failed);
        assert!(!affected_path.exists());
        std::fs::write(&material, original).unwrap();
        app.retry_queued_export(&ctx, 1).unwrap();
        drain(&mut app, &ctx);
        assert_eq!(app.export_queue.entries[1].state, QueueState::Completed);
        assert!(image_io::load(&affected_path).is_ok());
    }
}
