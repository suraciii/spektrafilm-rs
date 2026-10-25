//! A small crash-readable journal of coarse operation progress, not a report snapshot.
use super::{OperationKind, PersistenceError, ReportDestination};
use serde::Serialize;
use spektrafilm_backend::telemetry::{
    CoarseProgressReceiver, ObservationKind, Outcome, PROGRESS_QUEUE_CAPACITY,
};
use std::{
    fs::{File, OpenOptions},
    io::{self, Cursor, Seek, SeekFrom, Write},
    sync::mpsc::TryRecvError,
    time::{Duration, Instant},
};

const RECORD_BYTES: usize = 1024;
const RECORD_SLOTS: u64 = 128;
const WRITE_INTERVAL: Duration = Duration::from_millis(250);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const ACTIVE_LIMIT: usize = 32;

#[derive(Debug)]
pub struct ProgressReceiver {
    operation_id: u64,
    operation_kind: OperationKind,
    observation_root_id: u64,
    receiver: CoarseProgressReceiver,
}

impl ProgressReceiver {
    pub(super) fn new(
        operation_id: u64,
        operation_kind: OperationKind,
        observation_root_id: u64,
        receiver: CoarseProgressReceiver,
    ) -> Self {
        Self {
            operation_id,
            operation_kind,
            observation_root_id,
            receiver,
        }
    }
}

#[derive(Serialize)]
struct Record {
    schema_version: u32,
    operation_id: u64,
    observation_root_id: u64,
    operation_kind: OperationKind,
    seq: u64,
    event: &'static str,
    elapsed_seconds: f64,
    current_phase: Option<&'static str>,
    current_stage: Option<&'static str>,
    last_begin: Option<&'static str>,
    last_end: Option<&'static str>,
    outcome: Option<Outcome>,
    dropped_events: u64,
}

#[derive(Default)]
struct Active {
    entries: [Option<(u64, &'static str)>; ACTIVE_LIMIT],
}

impl Active {
    fn begin(&mut self, id: u64, name: &'static str) -> bool {
        if let Some(slot) = self.entries.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some((id, name));
            true
        } else {
            false
        }
    }

    fn end(&mut self, id: u64, name: &'static str) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|slot| *slot == Some((id, name)))
        {
            self.entries.copy_within(index + 1.., index);
            self.entries[ACTIVE_LIMIT - 1] = None;
        }
    }

    fn current(&self) -> Option<&'static str> {
        self.entries
            .iter()
            .rev()
            .find_map(|entry| entry.map(|(_, name)| name))
    }
}

/// Owns the open, exclusively created sidecar. Filesystem work belongs on a writer thread.
///
/// Each line is independently readable JSON padded to 1024 bytes. Slot zero is
/// the immutable begin record; the remaining slots wrap. Order valid records by
/// `seq`, ignoring a torn crash-write line. Only an `end` record is terminal.
/// This bounds each sidecar to 128 KiB without renaming over any user path.
pub struct ProgressJournal {
    progress: ProgressReceiver,
    file: File,
    seq: u64,
    phases: Active,
    stages: Active,
    last_begin: Option<&'static str>,
    last_end: Option<&'static str>,
    local_dropped: u64,
    changed: bool,
    disconnected: bool,
    terminal_written: bool,
    last_write: Instant,
    #[cfg(test)]
    after_drain: Option<Box<dyn FnOnce() + Send>>,
}

impl ProgressJournal {
    pub fn new(
        destination: ReportDestination,
        receiver: ProgressReceiver,
    ) -> Result<Self, PersistenceError> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(destination.path()).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                PersistenceError::AlreadyExists
            } else {
                PersistenceError::Io(error)
            }
        })?;
        let mut journal = Self {
            progress: receiver,
            file,
            seq: 0,
            phases: Active::default(),
            stages: Active::default(),
            last_begin: None,
            last_end: None,
            local_dropped: 0,
            changed: false,
            disconnected: false,
            terminal_written: false,
            last_write: Instant::now(),
            #[cfg(test)]
            after_drain: None,
        };
        journal.write_record("begin", None)?;
        super::persistence::sync_parent(destination.path())?;
        journal.progress.receiver.mark_ready();
        Ok(journal)
    }

    /// Drain a bounded queue, persisting changes at most four times per second.
    /// Returns true when terminal or disconnected; disconnect alone is not success.
    pub fn poll(&mut self) -> Result<bool, PersistenceError> {
        let outcome = self.collect_progress();
        let done = outcome.is_some() || self.disconnected;
        if !self.terminal_written
            && (done
                || (self.last_write.elapsed() >= WRITE_INTERVAL
                    && (self.changed || self.last_write.elapsed() >= HEARTBEAT_INTERVAL)))
        {
            self.persist(outcome)?;
        }
        Ok(done)
    }

    /// Persist accepted progress immediately at normal shutdown or terminal handoff.
    pub fn flush(&mut self) -> Result<(), PersistenceError> {
        let outcome = self.collect_progress();
        if !self.terminal_written {
            self.persist(outcome)?;
        }
        Ok(())
    }

    fn collect_progress(&mut self) -> Option<Outcome> {
        // Terminal release publishes every accepted final scope event. If it
        // arrives during the first drain, acquire it before a second drain.
        let mut outcome = self.progress.receiver.outcome();
        self.drain();
        if outcome.is_none() {
            outcome = self.progress.receiver.outcome();
            if outcome.is_some() {
                self.drain();
            }
        }
        outcome
    }

    fn drain(&mut self) {
        for _ in 0..PROGRESS_QUEUE_CAPACITY {
            match self.progress.receiver.try_recv() {
                Ok(event) => {
                    let name = bounded_name(event.name);
                    if event.begin {
                        self.last_begin = Some(name);
                    } else {
                        self.last_end = Some(name);
                    }
                    let active = match event.kind {
                        ObservationKind::Phase => Some(&mut self.phases),
                        ObservationKind::Stage => Some(&mut self.stages),
                        _ => None,
                    };
                    if let Some(active) = active {
                        if event.begin {
                            if !active.begin(event.id, name) {
                                self.local_dropped = self.local_dropped.saturating_add(1);
                            }
                        } else {
                            active.end(event.id, name);
                        }
                    }
                    self.changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.disconnected = true;
                    break;
                }
            }
        }
        #[cfg(test)]
        if let Some(after_drain) = self.after_drain.take() {
            after_drain();
        }
    }

    fn persist(&mut self, outcome: Option<Outcome>) -> Result<(), PersistenceError> {
        self.write_record(if outcome.is_some() { "end" } else { "progress" }, outcome)?;
        self.terminal_written = outcome.is_some();
        Ok(())
    }

    fn write_record(
        &mut self,
        event: &'static str,
        outcome: Option<Outcome>,
    ) -> Result<(), PersistenceError> {
        let record = Record {
            schema_version: 1,
            operation_id: self.progress.operation_id,
            observation_root_id: self.progress.observation_root_id,
            operation_kind: self.progress.operation_kind,
            seq: self.seq,
            event,
            elapsed_seconds: self.progress.receiver.elapsed_seconds(),
            current_phase: self.phases.current(),
            current_stage: self.stages.current(),
            last_begin: self.last_begin,
            last_end: self.last_end,
            outcome,
            dropped_events: self
                .progress
                .receiver
                .dropped_events()
                .saturating_add(self.local_dropped),
        };
        let mut bytes = [b' '; RECORD_BYTES];
        let mut cursor = Cursor::new(&mut bytes[..RECORD_BYTES - 1]);
        serde_json::to_writer(&mut cursor, &record).map_err(|error| {
            PersistenceError::Io(io::Error::new(io::ErrorKind::InvalidData, error))
        })?;
        bytes[RECORD_BYTES - 1] = b'\n';
        let slot = if self.seq == 0 {
            0
        } else {
            1 + (self.seq - 1) % (RECORD_SLOTS - 1)
        };
        self.file
            .seek(SeekFrom::Start(slot * RECORD_BYTES as u64))?;
        self.file.write_all(&bytes)?;
        self.file.sync_data()?;
        self.seq = self.seq.saturating_add(1);
        self.changed = false;
        self.last_write = Instant::now();
        Ok(())
    }
}

fn bounded_name(name: &'static str) -> &'static str {
    name.char_indices()
        .nth(64)
        .map_or(name, |(end, _)| &name[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::{CollectionMode, Operation, Purpose, validate_report_destination};
    use serde_json::Value;
    use std::path::Path;

    fn records(path: &Path) -> Vec<Value> {
        let text = std::fs::read_to_string(path).unwrap();
        let mut records: Vec<Value> = text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        records.sort_by_key(|record| record["seq"].as_u64().unwrap());
        records
    }

    fn journal(path: &Path, operation: &mut Operation) -> ProgressJournal {
        ProgressJournal::new(
            validate_report_destination(path, &[]).unwrap(),
            operation.progress().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn live_phase_and_chain_stage_are_readable_before_completion_and_flush_terminal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("operation.json.progress.jsonl");
        let mut operation = Operation::new(OperationKind::Export, CollectionMode::Summary);
        let id = operation.id();
        let mut journal = journal(&path, &mut operation);
        assert_eq!(records(&path)[0]["event"], "begin");
        let phase = operation.scope("simulation", ObservationKind::Phase, Purpose::Image);
        phase.context().chain_progress("grain_blur", true);
        journal.last_write = Instant::now() - WRITE_INTERVAL;
        assert!(!journal.poll().unwrap());
        let live = records(&path).pop().unwrap();
        assert_eq!(live["operation_id"], id);
        assert_eq!(live["current_phase"], "simulation");
        assert_eq!(live["current_stage"], "grain_blur");
        assert!(live["outcome"].is_null());
        assert!(!directory.path().join("operation.json").exists());
        phase.context().chain_progress("grain_blur", false);
        phase.finish(Outcome::Cancelled);
        let _ = operation.finish(Outcome::Cancelled);
        journal.flush().unwrap();
        let terminal = records(&path).pop().unwrap();
        assert_eq!(terminal["event"], "end");
        assert_eq!(terminal["outcome"], "cancelled");
        assert!(terminal["current_stage"].is_null());
        assert!(terminal["current_phase"].is_null());
    }

    #[test]
    fn journal_wraps_without_losing_begin_or_terminal_and_bounds_disk_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bounded.progress.jsonl");
        let mut operation = Operation::new(OperationKind::Process, CollectionMode::Summary);
        let mut journal = journal(&path, &mut operation);
        for _ in 0..RECORD_SLOTS * 2 {
            let phase = operation.scope("simulation", ObservationKind::Phase, Purpose::Image);
            journal.flush().unwrap();
            phase.finish(Outcome::Succeeded);
        }
        let _ = operation.finish(Outcome::Succeeded);
        journal.flush().unwrap();
        let written = records(&path);
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            RECORD_SLOTS * RECORD_BYTES as u64
        );
        assert_eq!(written.len(), RECORD_SLOTS as usize);
        assert_eq!(written[0]["seq"], 0);
        assert_eq!(written[0]["event"], "begin");
        assert_eq!(written.last().unwrap()["outcome"], "succeeded");
        assert!(written[1]["seq"].as_u64().unwrap() > 1);
    }

    #[test]
    fn full_event_queue_cannot_lose_failed_terminal_and_reports_omissions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("failure.progress.jsonl");
        let mut operation = Operation::new(OperationKind::Render, CollectionMode::Summary);
        let mut journal = journal(&path, &mut operation);
        let context = operation.context();
        for _ in 0..PROGRESS_QUEUE_CAPACITY * 2 {
            context.chain_progress("grain_blur", true);
            context.chain_progress("grain_blur", false);
        }
        let _ = operation.finish(Outcome::Failed);
        assert!(journal.poll().unwrap());
        let terminal = records(&path).pop().unwrap();
        assert_eq!(terminal["outcome"], "failed");
        assert!(terminal["dropped_events"].as_u64().unwrap() > 0);
    }

    #[test]
    fn dropped_unfinished_operation_remains_honestly_incomplete() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("interrupted.progress.jsonl");
        let mut operation = Operation::new(OperationKind::Scan, CollectionMode::Summary);
        let mut journal = journal(&path, &mut operation);
        let phase = operation.scope("simulation", ObservationKind::Phase, Purpose::Image);
        phase.context().chain_progress("grain_blur", true);
        journal.flush().unwrap();
        drop(phase);
        drop(operation);
        assert!(journal.poll().unwrap());
        let written = records(&path);
        assert!(written.iter().all(|record| record["event"] != "end"));
        assert!(written.iter().all(|record| record["outcome"].is_null()));
        assert!(
            written
                .iter()
                .any(|record| record["current_stage"] == "grain_blur")
        );
    }

    #[test]
    fn worker_acceptance_waits_for_durable_begin_without_blocking_frontend() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("gated.progress.jsonl");
        let mut operation = Operation::new(OperationKind::Preview, CollectionMode::Summary);
        let receiver = operation.progress().unwrap();
        assert!(!operation.progress_ready());
        let (entered, entering) = std::sync::mpsc::sync_channel(1);
        let (finished, completion) = std::sync::mpsc::sync_channel(1);
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            entered.send(()).unwrap();
            operation.wait_for_progress();
            // This is the image-worker boundary, not a frontend file read.
            assert_eq!(records(&worker_path)[0]["event"], "begin");
            finished
                .send(operation.finish(Outcome::Succeeded).unwrap())
                .unwrap();
        });
        entering.recv().unwrap();
        assert!(matches!(
            completion.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        assert!(!path.exists());
        let mut journal =
            ProgressJournal::new(validate_report_destination(&path, &[]).unwrap(), receiver)
                .unwrap();
        let report = completion.recv_timeout(Duration::from_secs(30)).unwrap();
        worker.join().unwrap();
        journal.flush().unwrap();
        assert_eq!(records(&path).last().unwrap()["outcome"], "succeeded");
        assert!(report.diagnostic_issues.is_empty());
    }

    #[test]
    fn failed_begin_releases_worker_with_separate_diagnostic_and_preserves_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("collision.progress.jsonl");
        std::fs::write(&path, b"protected existing data").unwrap();
        let mut operation = Operation::new(OperationKind::Export, CollectionMode::Summary);
        let receiver = operation.progress().unwrap();
        let worker = std::thread::spawn(move || {
            operation.wait_for_progress();
            operation.finish(Outcome::Succeeded).unwrap()
        });
        assert!(matches!(
            ProgressJournal::new(validate_report_destination(&path, &[]).unwrap(), receiver,),
            Err(PersistenceError::AlreadyExists)
        ));
        let report = worker.join().unwrap();
        assert_eq!(report.operation.outcome, Outcome::Succeeded);
        assert!(report.diagnostic_issues.iter().any(|issue| {
            issue.category == crate::telemetry::IssueCategory::CollectionUnavailable
                && issue.boundary == crate::telemetry::IssueBoundary::Operation
        }));
        assert_eq!(std::fs::read(&path).unwrap(), b"protected existing data");
    }

    #[test]
    fn final_scope_events_arriving_after_first_drain_are_included_in_poll_and_flush() {
        let directory = tempfile::tempdir().unwrap();
        for flush in [false, true] {
            let path = directory.path().join(if flush {
                "flush.progress.jsonl"
            } else {
                "poll.progress.jsonl"
            });
            let mut operation = Operation::new(OperationKind::Export, CollectionMode::Summary);
            let mut journal = journal(&path, &mut operation);
            let phase = operation.scope("simulation", ObservationKind::Phase, Purpose::Image);
            let stage = phase
                .context()
                .scope("metering", ObservationKind::Stage, Purpose::Image);
            journal.flush().unwrap();
            assert_eq!(records(&path).last().unwrap()["current_stage"], "metering");
            // Force the producer's exact race: both scope ends and root terminal
            // become visible only after the writer has drained an empty queue.
            journal.after_drain = Some(Box::new(move || {
                stage.finish(Outcome::Succeeded);
                phase.finish(Outcome::Succeeded);
                let _ = operation.finish(Outcome::Succeeded);
            }));
            if flush {
                journal.flush().unwrap();
            } else {
                assert!(journal.poll().unwrap());
            }
            let terminal = records(&path).pop().unwrap();
            assert_eq!(terminal["event"], "end");
            assert_eq!(terminal["outcome"], "succeeded");
            assert!(terminal["current_stage"].is_null());
            assert!(terminal["current_phase"].is_null());
            assert_eq!(terminal["last_end"], "simulation");
            assert_eq!(terminal["dropped_events"], 0);
        }
    }
}
