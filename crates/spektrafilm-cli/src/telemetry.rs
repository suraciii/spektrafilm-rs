use std::cell::Cell;
use std::path::Path;

use anyhow::Result;
use spektrafilm_backend::telemetry::{
    CollectionMode, ObservationContext, ObservationKind, Purpose,
};
use spektrafilm_core::telemetry::{
    BackendRequested, IssueBoundary, IssueCategory, Operation, OperationKind, Outcome,
};
use spektrafilm_math::image::ImageBuf;

/// One accepted CLI invocation. Report persistence is deliberately outside its lifetime.
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
pub(crate) fn simulation_attempt(
    invocation: &Invocation,
    index: u64,
    run: impl FnOnce(&ObservationContext) -> Result<ImageBuf>,
) -> (u64, Result<ImageBuf>) {
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
    let mut invocation = Invocation {
        operation,
        failure: Cell::new((IssueCategory::Configuration, IssueBoundary::Operation)),
    };
    // Invalid paths and aliases are command validation failures, before image work.
    let destination = diagnostics
        .map(|path| spektrafilm_core::telemetry::validate_report_destination(path, protected))
        .transpose();
    let (destination, result) = match destination {
        Ok(destination) => (destination, work(&mut invocation)),
        Err(error) => (None, Err(anyhow::Error::new(error))),
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
