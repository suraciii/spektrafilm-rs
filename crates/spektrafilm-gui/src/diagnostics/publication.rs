//! Session-only background publication of live progress and terminal reports.
//!
//! The GUI only enqueues captured destinations and receivers. Image workers wait
//! for the immutable begin record to be synced; synchronous Saves defer without
//! blocking the GUI. One writer polls at most 20 journals every 250 ms and flushes
//! each terminal journal before its report. Existing files are never replaced,
//! and errors stay separate from image outcomes. Shutdown drains accepted requests
//! and flushes unfinished journals.

use spektrafilm_core::telemetry::{
    ProgressJournal, ProgressReceiver, Report, validate_report_destination,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Accepted requests waiting for the worker; one additional request runs in flight.
const QUEUE_CAPACITY: usize = 20;
const MAX_ACTIVE_JOURNALS: usize = 20;
const POLL_INTERVAL: Duration = Duration::from_millis(250);

enum Request {
    Begin {
        directory: Arc<PathBuf>,
        operation_id: u64,
        receiver: ProgressReceiver,
    },
    Report {
        directory: Arc<PathBuf>,
        report: Arc<Report>,
        #[cfg(test)]
        before_write: Option<Box<dyn FnOnce() + Send>>,
    },
    #[cfg(test)]
    Pause {
        entered: SyncSender<()>,
        release: Receiver<()>,
    },
}

pub(super) struct Publisher {
    requests: Option<SyncSender<Request>>,
    errors: Receiver<String>,
    pending: Arc<AtomicUsize>,
    active_progress: Arc<AtomicUsize>,
    worker: Option<JoinHandle<()>>,
}

impl Publisher {
    pub(super) fn new() -> Result<Self, String> {
        let (requests, incoming) = sync_channel(QUEUE_CAPACITY);
        let (errors, surfaced) = sync_channel(1);
        let pending = Arc::new(AtomicUsize::new(0));
        let worker_pending = Arc::clone(&pending);
        let active_progress = Arc::new(AtomicUsize::new(0));
        let worker_progress = Arc::clone(&active_progress);
        let worker = std::thread::Builder::new()
            .name("diagnostics-publication".to_owned())
            .spawn(move || run(incoming, errors, worker_pending, worker_progress))
            .map_err(|error| format!("report publication worker failed to start: {error}"))?;
        Ok(Self {
            requests: Some(requests),
            errors: surfaced,
            pending,
            active_progress,
            worker: Some(worker),
        })
    }

    /// Reserve a bounded live-journal slot before enqueueing its captured sink.
    pub(super) fn begin(
        &self,
        directory: Arc<PathBuf>,
        operation_id: u64,
        receiver: ProgressReceiver,
    ) -> Result<(), String> {
        let mut active = self.active_progress.load(Ordering::Acquire);
        loop {
            if active >= MAX_ACTIVE_JOURNALS {
                return Err("progress journal limit reached".to_owned());
            }
            match self.active_progress.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => active = current,
            }
        }
        let result = self.enqueue_request(Request::Begin {
            directory,
            operation_id,
            receiver,
        });
        if result.is_err() {
            self.active_progress.fetch_sub(1, Ordering::Release);
        }
        result
    }

    /// Queue a new `operation-<id>.json` without blocking the GUI thread.
    pub(super) fn enqueue(
        &self,
        directory: Arc<PathBuf>,
        report: Arc<Report>,
    ) -> Result<(), String> {
        self.enqueue_request(Request::Report {
            directory,
            report,
            #[cfg(test)]
            before_write: None,
        })
    }

    fn enqueue_request(&self, request: Request) -> Result<(), String> {
        self.pending.fetch_add(1, Ordering::Release);
        let failure = match self.requests.as_ref() {
            Some(requests) => match requests.try_send(request) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(_)) => "report publication queue is full",
                Err(TrySendError::Disconnected(_)) => "report publication worker stopped",
            },
            None => "report publication worker stopped",
        };
        self.pending.fetch_sub(1, Ordering::Release);
        Err(failure.to_owned())
    }

    /// The first unclaimed publication error, retained one at a time.
    pub(super) fn take_error(&self) -> Option<String> {
        self.errors.try_recv().ok()
    }

    /// Accepted report writes and live journals still queued or in flight.
    pub(super) fn pending(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn pause_for_test(&self) -> (Receiver<()>, SyncSender<()>) {
        let (entered, ready) = sync_channel(1);
        let (release, released) = sync_channel(1);
        self.requests
            .as_ref()
            .unwrap()
            .try_send(Request::Pause {
                entered,
                release: released,
            })
            .unwrap_or_else(|_| panic!("test publication pause was not accepted"));
        (ready, release)
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        // Closing the queue lets the worker drain requests, then flush live journals.
        drop(self.requests.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(
    requests: Receiver<Request>,
    errors: SyncSender<String>,
    pending: Arc<AtomicUsize>,
    active_progress: Arc<AtomicUsize>,
) {
    let mut journals = Vec::with_capacity(MAX_ACTIVE_JOURNALS);
    let mut next_poll = Instant::now() + POLL_INTERVAL;
    loop {
        match requests.recv_timeout(next_poll.saturating_duration_since(Instant::now())) {
            Ok(Request::Begin {
                directory,
                operation_id,
                receiver,
            }) => {
                let path = directory.join(format!("operation-{operation_id}.json"));
                let journal = validate_report_destination(&path, &[])
                    .and_then(|destination| destination.progress_destination(&[]))
                    .and_then(|destination| ProgressJournal::new(destination, receiver));
                match journal {
                    Ok(journal) => journals.push((operation_id, journal)),
                    Err(error) => {
                        let _ = errors.try_send(error.to_string());
                        active_progress.fetch_sub(1, Ordering::Release);
                        pending.fetch_sub(1, Ordering::Release);
                    }
                }
            }
            Ok(Request::Report {
                directory,
                report,
                #[cfg(test)]
                before_write,
            }) => {
                if let Some(index) = journals
                    .iter()
                    .position(|(id, _)| *id == report.operation.id)
                {
                    let (_, mut journal) = journals.swap_remove(index);
                    if let Err(error) = journal.flush() {
                        let _ = errors.try_send(error.to_string());
                    }
                    active_progress.fetch_sub(1, Ordering::Release);
                    pending.fetch_sub(1, Ordering::Release);
                }
                #[cfg(test)]
                if let Some(before_write) = before_write {
                    before_write();
                }
                let path = directory.join(format!("operation-{}.json", report.operation.id));
                if let Err(error) = validate_report_destination(&path, &[])
                    .and_then(|destination| destination.write_new(&report))
                {
                    let _ = errors.try_send(error.to_string());
                }
                // Acquire readers of zero pending also observe the preceding error.
                pending.fetch_sub(1, Ordering::Release);
            }
            #[cfg(test)]
            Ok(Request::Pause { entered, release }) => {
                let _ = entered.send(());
                let _ = release.recv();
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        // Check after requests too, so queued writes cannot starve progress.
        if Instant::now() >= next_poll {
            let mut index = 0;
            while index < journals.len() {
                let finished = match journals[index].1.poll() {
                    Ok(finished) => finished,
                    Err(error) => {
                        let _ = errors.try_send(error.to_string());
                        true
                    }
                };
                if finished {
                    journals.swap_remove(index);
                    active_progress.fetch_sub(1, Ordering::Release);
                    pending.fetch_sub(1, Ordering::Release);
                } else {
                    index += 1;
                }
            }
            next_poll = Instant::now() + POLL_INTERVAL;
        }
    }
    for (_, mut journal) in journals {
        if let Err(error) = journal.flush() {
            let _ = errors.try_send(error.to_string());
        }
        active_progress.fetch_sub(1, Ordering::Release);
        pending.fetch_sub(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_core::telemetry::{
        CollectionMode, ObservationKind, Operation, OperationKind, Outcome, Purpose,
    };
    use std::path::Path;

    fn terminal_report(kind: OperationKind, outcome: Outcome) -> Arc<Report> {
        Arc::new(
            Operation::new(kind, CollectionMode::Summary)
                .finish(outcome)
                .expect("summary operation yields a terminal report"),
        )
    }

    fn wait_for_idle(publisher: &Publisher) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while publisher.pending() > 0 {
            assert!(
                Instant::now() < deadline,
                "publication writes did not drain"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn begin(publisher: &Publisher, directory: &Path, kind: OperationKind) -> Operation {
        let mut operation = Operation::new(kind, CollectionMode::Summary);
        publisher
            .begin(
                Arc::new(directory.to_path_buf()),
                operation.id(),
                operation.progress().unwrap(),
            )
            .unwrap();
        operation.wait_for_progress();
        operation
    }

    fn progress_path(directory: &Path, operation_id: u64) -> PathBuf {
        directory.join(format!("operation-{operation_id}.json.progress.jsonl"))
    }

    fn records(path: &Path) -> Vec<serde_json::Value> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn wait_for_record(path: &Path, predicate: impl Fn(&serde_json::Value) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if records(path).iter().any(&predicate) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "live progress did not reach disk"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn image_worker_gate_waits_for_real_begin_or_reported_begin_failure() {
        for collision in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let publisher = Publisher::new().unwrap();
            let (paused, release) = publisher.pause_for_test();
            paused.recv_timeout(Duration::from_secs(30)).unwrap();
            let mut operation = Operation::new(OperationKind::Preview, CollectionMode::Summary);
            let id = operation.id();
            let path = progress_path(directory.path(), id);
            if collision {
                std::fs::write(&path, b"existing progress").unwrap();
            }
            publisher
                .begin(
                    Arc::new(directory.path().to_path_buf()),
                    id,
                    operation.progress().unwrap(),
                )
                .unwrap();
            assert!(!operation.progress_ready());
            let (completed, ready) = sync_channel(1);
            let worker_path = path.clone();
            let worker = std::thread::spawn(move || {
                operation.wait_for_progress();
                assert!(operation.progress_ready());
                if collision {
                    assert_eq!(std::fs::read(&worker_path).unwrap(), b"existing progress");
                } else {
                    assert!(
                        records(&worker_path)
                            .iter()
                            .any(|record| record["event"] == "begin")
                    );
                }
                completed.send(operation).unwrap();
            });
            assert!(matches!(
                ready.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
            if !collision {
                assert!(!path.exists());
            }
            release.send(()).unwrap();
            let operation = ready.recv_timeout(Duration::from_secs(30)).unwrap();
            worker.join().unwrap();
            let report = Arc::new(operation.finish(Outcome::Succeeded).unwrap());
            publisher
                .enqueue(Arc::new(directory.path().to_path_buf()), report)
                .unwrap();
            wait_for_idle(&publisher);
            assert_eq!(publisher.take_error().is_some(), collision);
            let report_path = directory.path().join(format!("operation-{id}.json"));
            let report = Report::from_json(&std::fs::read_to_string(report_path).unwrap()).unwrap();
            assert_eq!(report.operation.outcome, Outcome::Succeeded);
        }
    }

    #[test]
    fn terminal_journal_is_flushed_before_final_report_publication() {
        let directory = tempfile::tempdir().unwrap();
        let publisher = Publisher::new().unwrap();
        let (paused, release) = publisher.pause_for_test();
        paused.recv_timeout(Duration::from_secs(30)).unwrap();
        let mut operation = Operation::new(OperationKind::Preview, CollectionMode::Summary);
        let id = operation.id();
        publisher
            .begin(
                Arc::new(directory.path().to_path_buf()),
                id,
                operation.progress().unwrap(),
            )
            .unwrap();
        // Immediate validation failure can finish before Begin is handled. FIFO
        // delivery must still sync its terminal journal before publishing JSON.
        let report = Arc::new(operation.finish(Outcome::Failed).unwrap());
        let sidecar = progress_path(directory.path(), id);
        let final_path = directory.path().join(format!("operation-{id}.json"));
        let inspected_final_path = final_path.clone();
        let (inspected, inspection) = sync_channel(1);
        publisher
            .enqueue_request(Request::Report {
                directory: Arc::new(directory.path().to_path_buf()),
                report,
                before_write: Some(Box::new(move || {
                    assert!(!inspected_final_path.exists());
                    assert!(records(&sidecar).iter().any(|record| {
                        record["event"] == "end" && record["outcome"] == "failed"
                    }));
                    inspected.send(()).unwrap();
                })),
            })
            .unwrap();
        release.send(()).unwrap();
        inspection.recv_timeout(Duration::from_secs(30)).unwrap();
        wait_for_idle(&publisher);
        assert_eq!(publisher.active_progress.load(Ordering::Acquire), 0);
        assert!(publisher.take_error().is_none());
        let report = Report::from_json(&std::fs::read_to_string(final_path).unwrap()).unwrap();
        assert_eq!(report.operation.outcome, Outcome::Failed);
    }

    #[test]
    fn progress_reaches_disk_before_operation_completion() {
        let directory = tempfile::tempdir().unwrap();
        let publisher = Publisher::new().unwrap();
        let operation = begin(&publisher, directory.path(), OperationKind::Preview);
        let path = progress_path(directory.path(), operation.id());
        let phase = operation.scope("simulation", ObservationKind::Phase, Purpose::Image);
        let stage = phase
            .context()
            .scope("metering", ObservationKind::Stage, Purpose::Image);
        wait_for_record(&path, |record| {
            record["event"] == "progress" && record["current_stage"] == "metering"
        });
        assert!(
            !directory
                .path()
                .join(format!("operation-{}.json", operation.id()))
                .exists()
        );
        let live = records(&path);
        assert!(live.iter().any(|record| record["event"] == "begin"));
        assert!(!live.iter().any(|record| record["event"] == "end"));
        stage.finish(Outcome::Succeeded);
        phase.finish(Outcome::Succeeded);
        let report = Arc::new(operation.finish(Outcome::Succeeded).unwrap());
        publisher
            .enqueue(Arc::new(directory.path().to_path_buf()), report)
            .unwrap();
        wait_for_idle(&publisher);
        assert!(publisher.take_error().is_none());
        assert!(
            records(&path)
                .iter()
                .any(|record| { record["event"] == "end" && record["outcome"] == "succeeded" })
        );
    }

    #[test]
    fn concurrent_cancelled_and_failed_operations_remain_isolated() {
        let directory = tempfile::tempdir().unwrap();
        let publisher = Publisher::new().unwrap();
        let preview = begin(&publisher, directory.path(), OperationKind::Preview);
        let export = begin(&publisher, directory.path(), OperationKind::Export);
        let preview_id = preview.id();
        let export_id = export.id();
        let preview_phase = preview.scope("simulation", ObservationKind::Phase, Purpose::Image);
        let export_phase = export.scope("file_write", ObservationKind::Phase, Purpose::Image);
        let preview_path = progress_path(directory.path(), preview_id);
        let export_path = progress_path(directory.path(), export_id);
        wait_for_record(&preview_path, |record| {
            record["current_phase"] == "simulation"
        });
        wait_for_record(&export_path, |record| {
            record["current_phase"] == "file_write"
        });
        preview_phase.finish(Outcome::Cancelled);
        let preview = Arc::new(preview.finish(Outcome::Cancelled).unwrap());
        publisher
            .enqueue(Arc::new(directory.path().to_path_buf()), preview)
            .unwrap();
        wait_for_record(&preview_path, |record| record["event"] == "end");
        assert!(
            !records(&export_path)
                .iter()
                .any(|record| record["event"] == "end")
        );
        export_phase.finish(Outcome::Failed);
        let export = Arc::new(export.finish(Outcome::Failed).unwrap());
        publisher
            .enqueue(Arc::new(directory.path().to_path_buf()), export)
            .unwrap();
        wait_for_idle(&publisher);
        let mut roots = Vec::new();
        for (id, path, kind, outcome) in [
            (preview_id, preview_path, "preview", "cancelled"),
            (export_id, export_path, "export", "failed"),
        ] {
            let written = records(&path);
            assert!(!written.is_empty());
            let root = written[0]["observation_root_id"].as_u64().unwrap();
            roots.push(root);
            assert!(written.iter().all(|record| {
                record["operation_id"] == id
                    && record["observation_root_id"] == root
                    && record["operation_kind"] == kind
            }));
            assert!(
                written
                    .iter()
                    .any(|record| { record["event"] == "end" && record["outcome"] == outcome })
            );
            let report = Report::from_json(
                &std::fs::read_to_string(directory.path().join(format!("operation-{id}.json")))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(report.operation.outcome).unwrap(),
                outcome
            );
        }
        assert_ne!(roots[0], roots[1]);
        assert!(publisher.take_error().is_none());
    }

    #[test]
    fn shutdown_flushes_live_journals_and_drains_terminal_reports() {
        let directory = tempfile::tempdir().unwrap();
        let publisher = Publisher::new().unwrap();
        let unfinished = begin(&publisher, directory.path(), OperationKind::Scan);
        let unfinished_path = progress_path(directory.path(), unfinished.id());
        unfinished
            .scope("simulation", ObservationKind::Phase, Purpose::Image)
            .finish(Outcome::Succeeded);
        let completed = begin(&publisher, directory.path(), OperationKind::Export);
        let completed_id = completed.id();
        let report = Arc::new(completed.finish(Outcome::Cancelled).unwrap());
        publisher
            .enqueue(Arc::new(directory.path().to_path_buf()), report)
            .unwrap();
        let pending = Arc::clone(&publisher.pending);
        let active = Arc::clone(&publisher.active_progress);
        drop(publisher);
        assert_eq!(pending.load(Ordering::Acquire), 0);
        assert_eq!(active.load(Ordering::Acquire), 0);
        let unfinished_records = records(&unfinished_path);
        assert!(
            unfinished_records
                .iter()
                .any(|record| record["event"] == "progress")
        );
        assert!(
            !unfinished_records
                .iter()
                .any(|record| record["event"] == "end")
        );
        assert!(
            records(&progress_path(directory.path(), completed_id))
                .iter()
                .any(|record| { record["event"] == "end" && record["outcome"] == "cancelled" })
        );
        let report_path = directory
            .path()
            .join(format!("operation-{completed_id}.json"));
        let report = Report::from_json(&std::fs::read_to_string(report_path).unwrap()).unwrap();
        assert_eq!(report.operation.outcome, Outcome::Cancelled);
        drop(unfinished);
    }

    #[test]
    fn active_journals_are_bounded_independently_of_the_request_queue() {
        let directory = tempfile::tempdir().unwrap();
        let publisher = Publisher::new().unwrap();
        let mut operations = Vec::new();
        for _ in 0..MAX_ACTIVE_JOURNALS {
            operations.push(begin(&publisher, directory.path(), OperationKind::Preview));
        }
        assert_eq!(
            publisher.active_progress.load(Ordering::Acquire),
            MAX_ACTIVE_JOURNALS
        );
        let mut rejected = Operation::new(OperationKind::Preview, CollectionMode::Summary);
        assert_eq!(
            publisher
                .begin(
                    Arc::new(directory.path().to_path_buf()),
                    rejected.id(),
                    rejected.progress().unwrap()
                )
                .unwrap_err(),
            "progress journal limit reached",
        );
        drop(publisher);
        assert!(!progress_path(directory.path(), rejected.id()).exists());
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            MAX_ACTIVE_JOURNALS
        );
        assert!(operations.iter().all(|operation| {
            records(&progress_path(directory.path(), operation.id()))
                .iter()
                .any(|record| record["event"] == "begin")
        }));
    }

    fn assert_progress_collision(directory: &Path, mut operation: Operation) {
        let publisher = Publisher::new().unwrap();
        publisher
            .begin(
                Arc::new(directory.to_path_buf()),
                operation.id(),
                operation.progress().unwrap(),
            )
            .unwrap();
        wait_for_idle(&publisher);
        assert!(publisher.take_error().is_some());
        assert!(publisher.take_error().is_none());
        // A journal collision does not poison terminal-report publication.
        let id = operation.id();
        let report = Arc::new(operation.finish(Outcome::Succeeded).unwrap());
        publisher
            .enqueue(Arc::new(directory.to_path_buf()), report)
            .unwrap();
        wait_for_idle(&publisher);
        assert!(publisher.take_error().is_none());
        let report = Report::from_json(
            &std::fs::read_to_string(directory.join(format!("operation-{id}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(report.operation.outcome, Outcome::Succeeded);
    }

    #[test]
    fn existing_progress_file_is_preserved_without_failing_the_operation() {
        let directory = tempfile::tempdir().unwrap();
        let operation = Operation::new(OperationKind::Export, CollectionMode::Summary);
        let path = progress_path(directory.path(), operation.id());
        std::fs::write(&path, b"existing progress").unwrap();
        assert_progress_collision(directory.path(), operation);
        assert_eq!(std::fs::read(&path).unwrap(), b"existing progress");
    }

    #[cfg(unix)]
    #[test]
    fn progress_symlink_is_rejected_without_touching_its_target() {
        let directory = tempfile::tempdir().unwrap();
        for dangling in [false, true] {
            let operation = Operation::new(OperationKind::Export, CollectionMode::Summary);
            let path = progress_path(directory.path(), operation.id());
            let target = directory.path().join(format!("target-{}", operation.id()));
            if !dangling {
                std::fs::write(&target, b"protected target").unwrap();
            }
            std::os::unix::fs::symlink(&target, &path).unwrap();
            assert_progress_collision(directory.path(), operation);
            assert!(
                std::fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            if dangling {
                assert!(!target.exists());
            } else {
                assert_eq!(std::fs::read(&target).unwrap(), b"protected target");
            }
        }
    }

    #[test]
    fn writes_decodable_terminal_report_with_matching_identity() {
        let directory = tempfile::tempdir().unwrap();
        let publisher = Publisher::new().unwrap();
        let report = terminal_report(OperationKind::Preview, Outcome::Succeeded);
        let id = report.operation.id;
        publisher
            .enqueue(Arc::new(directory.path().to_path_buf()), report)
            .unwrap();
        wait_for_idle(&publisher);
        assert_eq!(publisher.pending(), 0);
        assert!(publisher.take_error().is_none());
        drop(publisher);
        let path = directory.path().join(format!("operation-{id}.json"));
        let written = Report::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(written.operation.id, id);
        assert_eq!(written.operation.kind, OperationKind::Preview);
        assert_eq!(written.operation.outcome, Outcome::Succeeded);
    }

    #[test]
    fn duplicate_destination_is_preserved_and_surfaces_error() {
        let directory = tempfile::tempdir().unwrap();
        let report = terminal_report(OperationKind::Export, Outcome::Failed);
        let id = report.operation.id;
        let path = directory.path().join(format!("operation-{id}.json"));
        std::fs::write(&path, "existing report").unwrap();
        let publisher = Publisher::new().unwrap();
        publisher
            .enqueue(Arc::new(directory.path().to_path_buf()), report)
            .unwrap();
        wait_for_idle(&publisher);
        assert!(publisher.take_error().is_some());
        assert!(publisher.take_error().is_none());
        drop(publisher);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "existing report");
    }

    #[test]
    fn pending_is_bounded_and_drop_drains_accepted_writes() {
        let directory = tempfile::tempdir().unwrap();
        let directory = Arc::new(directory.path().to_path_buf());
        let publisher = Publisher::new().unwrap();
        let mut accepted = 0usize;
        for _ in 0..4 * QUEUE_CAPACITY {
            let report = terminal_report(OperationKind::Render, Outcome::Succeeded);
            let queued = publisher.enqueue(Arc::clone(&directory), report);
            assert!(publisher.pending() <= QUEUE_CAPACITY + 1);
            match queued {
                Ok(()) => accepted += 1,
                Err(error) => assert_eq!(error, "report publication queue is full"),
            }
        }
        // An empty queue admits the first `QUEUE_CAPACITY` requests regardless
        // of worker scheduling.
        assert!(accepted >= QUEUE_CAPACITY);
        // Dropping joins the worker, so every accepted write is complete.
        drop(publisher);
        assert_eq!(
            std::fs::read_dir(directory.as_path())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path
                    .extension()
                    .is_some_and(|extension| extension == "json"))
                .count(),
            accepted
        );
    }
}
