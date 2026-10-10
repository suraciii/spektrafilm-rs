use std::cell::Cell;
use std::path::Path;

use anyhow::Result;
use spektrafilm_core::telemetry::{
    BackendRequested, IssueBoundary, IssueCategory, Operation, OperationKind, Outcome,
};
use spektrafilm_gpu::telemetry::{CollectionMode, ObservationContext, ObservationKind, Purpose};

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
    backend: &dyn spektrafilm_gpu::ComputeBackend,
    context: &ObservationContext,
) {
    if !context.enabled() {
        return;
    }
    if !backend.is_gpu() {
        context.set_backend_selected(spektrafilm_gpu::telemetry::BackendSelected::Cpu);
    }
    drop(spektrafilm_gpu::bind_backend(backend, context.clone()));
}
