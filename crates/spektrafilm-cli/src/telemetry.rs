use std::cell::Cell;
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::Result;
use spektrafilm_backend::telemetry::{
    CollectionMode, ObservationContext, ObservationKind, Purpose,
};
use spektrafilm_core::telemetry::{
    BackendRequested, IssueBoundary, IssueCategory, Operation, OperationKind, Outcome,
    ProgressJournal,
};

/// One accepted CLI invocation. Final report persistence follows its terminal boundary.
pub(crate) struct Invocation {
    pub operation: Operation,
    failure: Cell<(IssueCategory, IssueBoundary)>,
}

impl Invocation {
    pub fn boundary(&self, category: IssueCategory, boundary: IssueBoundary) {
        self.failure.set((category, boundary));
    }

    pub fn phase<T>(
        &self,
        name: &'static str,
        category: IssueCategory,
        boundary: IssueBoundary,
        parent: &ObservationContext,
        work: impl FnOnce(&ObservationContext) -> Result<T>,
    ) -> Result<T> {
        self.boundary(category, boundary);
        let scope = parent.scope(name, ObservationKind::Phase, Purpose::Image);
        let span = scope.context().span().map(|span| span.enter());
        let result = work(scope.context());
        drop(span);
        scope.finish(if result.is_ok() {
            Outcome::Succeeded
        } else {
            Outcome::Failed
        });
        result
    }
}

/// One instrumented simulation attempt: the `simulation` phase around `run`,
/// the attempt's identity, and its terminal outcome. Both render paths share
/// this helper so attempt numbering and outcome policy cannot drift apart.
pub(crate) fn simulation_attempt<T>(
    invocation: &Invocation,
    index: u64,
    run: impl FnOnce(&ObservationContext) -> Result<T>,
) -> (u64, Result<T>) {
    let attempt = invocation.operation.attempt(index);
    let attempt_id = attempt.context().id();
    let result = invocation.phase(
        "simulation",
        IssueCategory::Simulation,
        IssueBoundary::Simulation,
        attempt.context(),
        run,
    );
    attempt.finish(if result.is_ok() {
        Outcome::Succeeded
    } else {
        Outcome::Failed
    });
    (attempt_id, result)
}

/// Owns the journal outside image work, including its final unthrottled flush.
struct ProgressWriter {
    shutdown: SyncSender<()>,
    worker: Option<JoinHandle<()>>,
}

impl ProgressWriter {
    fn start(mut journal: ProgressJournal) -> Option<Self> {
        let (shutdown, requests) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("diagnostics-progress".into())
            .spawn(move || {
                loop {
                    match requests.recv_timeout(Duration::from_millis(250)) {
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => match journal.poll() {
                            Ok(true) => break,
                            Ok(false) => {}
                            Err(error) => {
                                eprintln!("Diagnostics progress could not be saved: {error}");
                                break;
                            }
                        },
                    }
                }
                if let Err(error) = journal.flush() {
                    eprintln!("Diagnostics progress could not be saved: {error}");
                }
            }) {
            Ok(worker) => Some(Self {
                shutdown,
                worker: Some(worker),
            }),
            Err(error) => {
                eprintln!("Diagnostics progress writer could not be started: {error}");
                None
            }
        }
    }
}

impl Drop for ProgressWriter {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            eprintln!("Diagnostics progress writer failed before shutdown");
        }
    }
}

pub(crate) fn run<T>(
    kind: OperationKind,
    diagnostics: Option<&Path>,
    gpu_timings: bool,
    protected: &[&Path],
    backend: BackendRequested,
    work: impl FnOnce(&mut Invocation) -> Result<T>,
) -> Result<T> {
    let mode = if diagnostics.is_none() {
        CollectionMode::Off
    } else if gpu_timings {
        CollectionMode::GpuTiming
    } else {
        CollectionMode::Summary
    };
    let mut operation = Operation::new(kind, mode);
    operation.set_backend_requested(backend);
    // Validate both paths before image work. Journal I/O remains diagnostic-only.
    let destinations = diagnostics
        .map(|path| {
            let destination =
                spektrafilm_core::telemetry::validate_report_destination(path, protected)?;
            let progress = match destination.progress_destination(protected) {
                Ok(progress) => Some(progress),
                Err(error @ spektrafilm_core::telemetry::PersistenceError::Io(_)) => {
                    eprintln!("Diagnostics progress could not be saved: {error}");
                    None
                }
                Err(error) => return Err(error),
            };
            Ok((destination, progress))
        })
        .transpose();
    let (destination, progress_destination, validation_error) = match destinations {
        Ok(Some((destination, progress))) => (Some(destination), progress, None),
        Ok(None) => (None, None, None),
        Err(error) => (None, None, Some(error)),
    };
    // Attach before work can clone any observation contexts.
    let progress_writer = progress_destination.and_then(|destination| {
        let receiver = operation.progress()?;
        match ProgressJournal::new(destination, receiver) {
            Ok(journal) => ProgressWriter::start(journal),
            Err(error) => {
                eprintln!("Diagnostics progress could not be saved: {error}");
                None
            }
        }
    });
    let mut invocation = Invocation {
        operation,
        failure: Cell::new((IssueCategory::Configuration, IssueBoundary::Operation)),
    };
    let result = match validation_error {
        None => work(&mut invocation),
        Some(error) => Err(anyhow::Error::new(error)),
    };
    if result.is_err() {
        let (category, boundary) = invocation.failure.get();
        invocation.operation.issue(category, boundary);
    }
    let report = invocation.operation.finish(if result.is_ok() {
        Outcome::Succeeded
    } else {
        Outcome::Failed
    });
    // finish emits the terminal event; join drains it before the final report.
    drop(progress_writer);
    if let (Some(destination), Some(report)) = (destination, report) {
        match destination.write_new(&report) {
            Ok(()) => eprintln!(
                "Diagnostics report saved: {}",
                diagnostics.unwrap().display()
            ),
            Err(error) => eprintln!("Diagnostics report could not be saved: {error}"),
        }
    }
    result
}

pub(crate) fn requested_backend(explicit: Option<crate::Backend>) -> BackendRequested {
    match explicit {
        Some(crate::Backend::Cpu) => BackendRequested::Cpu,
        Some(crate::Backend::Gpu) => BackendRequested::Wgpu,
        None => match std::env::var("SPEKTRAFILM_BACKEND")
            .ok()
            .map(|value| value.to_ascii_lowercase())
            .as_deref()
        {
            Some("cpu") => BackendRequested::Cpu,
            Some("wgpu") => BackendRequested::Wgpu,
            _ => BackendRequested::Auto,
        },
    }
}

pub(crate) fn observe_backend(
    backend: &dyn spektrafilm_backend::ComputeBackend,
    context: &ObservationContext,
) {
    if !context.enabled() {
        return;
    }
    if !backend.is_gpu() {
        context.set_backend_selected(spektrafilm_backend::telemetry::BackendSelected::Cpu);
    }
    drop(spektrafilm_backend::bind_backend(backend, context.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::fs;
    use std::time::Instant;

    fn records(path: &Path) -> Vec<Value> {
        fs::read(path)
            .unwrap()
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice(line).ok())
            .collect()
    }

    #[test]
    fn progress_is_readable_while_work_is_running_and_finishes_before_return() {
        let directory = tempfile::tempdir().unwrap();
        let report = directory.path().join("report.json");
        let progress = directory.path().join("report.json.progress.jsonl");
        let result = run(
            OperationKind::Process,
            Some(&report),
            false,
            &[],
            BackendRequested::Cpu,
            |invocation| {
                assert!(!report.exists());
                let initial = records(&progress);
                assert_eq!(initial[0]["event"], "begin");
                assert_eq!(initial[0]["operation_id"], invocation.operation.id());
                let context = invocation.operation.context();
                invocation.phase(
                    "input_load",
                    IssueCategory::Input,
                    IssueBoundary::Operation,
                    &context,
                    |_| {
                        let deadline = Instant::now() + Duration::from_secs(5);
                        loop {
                            let observed = records(&progress);
                            if observed.iter().any(|record| {
                                record["event"] == "progress"
                                    && record["current_phase"] == "input_load"
                            }) {
                                break;
                            }
                            assert!(Instant::now() < deadline, "no live phase snapshot");
                            thread::sleep(Duration::from_millis(10));
                        }
                        assert!(!report.exists());
                        Ok(())
                    },
                )
            },
        );
        result.unwrap();
        let observed = records(&progress);
        assert!(
            observed
                .iter()
                .any(|record| { record["event"] == "end" && record["outcome"] == "succeeded" })
        );
        let terminal: Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        assert_eq!(terminal["operation"]["outcome"], "succeeded");
        assert_eq!(observed[0]["operation_id"], terminal["operation"]["id"]);
    }

    #[test]
    fn immediate_failure_flushes_terminal_progress_without_waiting_for_poll() {
        let directory = tempfile::tempdir().unwrap();
        let report = directory.path().join("failure.json");
        let progress = directory.path().join("failure.json.progress.jsonl");
        let result = run::<()>(
            OperationKind::Render,
            Some(&report),
            false,
            &[],
            BackendRequested::Cpu,
            |invocation| {
                assert_eq!(records(&progress)[0]["event"], "begin");
                invocation.boundary(IssueCategory::Input, IssueBoundary::Operation);
                anyhow::bail!("input could not be loaded")
            },
        );
        assert_eq!(result.unwrap_err().to_string(), "input could not be loaded");
        assert!(
            records(&progress)
                .iter()
                .any(|record| { record["event"] == "end" && record["outcome"] == "failed" })
        );
        let terminal: Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        assert_eq!(terminal["operation"]["outcome"], "failed");
    }

    #[test]
    fn disabled_diagnostics_do_not_create_files_or_attach_progress() {
        let directory = tempfile::tempdir().unwrap();
        run(
            OperationKind::Process,
            None,
            false,
            &[],
            BackendRequested::Cpu,
            |invocation| {
                assert!(!invocation.operation.enabled());
                assert!(invocation.operation.progress().is_none());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
