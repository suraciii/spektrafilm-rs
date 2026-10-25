//! Session-only background publication of completed-operation reports.
//!
//! The GUI thread only enqueues captured `Arc`s; a worker thread validates each
//! destination and writes `operation-<id>.json` as a new file. Existing files
//! are never replaced, write failures surface as one bounded status error
//! without changing the operation's outcome, and dropping the publisher drains
//! every accepted write before the worker is joined.

use spektrafilm_core::telemetry::{Report, validate_report_destination};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread::JoinHandle,
};

/// Accepted requests waiting for the worker; one additional write runs in flight.
const QUEUE_CAPACITY: usize = 20;

struct Request {
    directory: Arc<PathBuf>,
    report: Arc<Report>,
}

pub(super) struct Publisher {
    requests: Option<SyncSender<Request>>,
    errors: Receiver<String>,
    pending: Arc<AtomicUsize>,
    worker: Option<JoinHandle<()>>,
}

impl Publisher {
    pub(super) fn new() -> Result<Self, String> {
        let (requests, incoming) = sync_channel(QUEUE_CAPACITY);
        let (errors, surfaced) = sync_channel(1);
        let pending = Arc::new(AtomicUsize::new(0));
        let worker_pending = Arc::clone(&pending);
        let worker = std::thread::Builder::new()
            .name("diagnostics-publication".to_owned())
            .spawn(move || run(incoming, errors, worker_pending))
            .map_err(|error| format!("report publication worker failed to start: {error}"))?;
        Ok(Self {
            requests: Some(requests),
            errors: surfaced,
            pending,
            worker: Some(worker),
        })
    }

    /// Queue `report` for a new `operation-<id>.json` in `directory`.
    ///
    /// Never blocks the GUI thread: a full queue or stopped worker is an error
    /// string for the status boundary, and such a request is not pending.
    pub(super) fn enqueue(
        &self,
        directory: Arc<PathBuf>,
        report: Arc<Report>,
    ) -> Result<(), String> {
        self.pending.fetch_add(1, Ordering::Release);
        let request = Request { directory, report };
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

    /// Accepted writes still queued or in flight.
    pub(super) fn pending(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        // Close the queue so the worker drains accepted writes, then join it.
        drop(self.requests.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(requests: Receiver<Request>, errors: SyncSender<String>, pending: Arc<AtomicUsize>) {
    while let Ok(request) = requests.recv() {
        if let Err(error) = write_report(&request) {
            // Bounded retention: one unclaimed error is kept, later ones drop.
            let _ = errors.try_send(error);
        }
        // Release pairs with the Acquire in `pending`: a GUI thread that sees
        // zero pending writes has also observed the error reported above.
        pending.fetch_sub(1, Ordering::Release);
    }
}

fn write_report(request: &Request) -> Result<(), String> {
    let path = request
        .directory
        .join(format!("operation-{}.json", request.report.operation.id));
    validate_report_destination(&path, &[])
        .and_then(|destination| destination.write_new(&request.report))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_core::telemetry::{CollectionMode, Operation, OperationKind, Outcome};
    use std::time::{Duration, Instant};

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
            std::fs::read_dir(directory.as_path()).unwrap().count(),
            accepted
        );
    }
}
