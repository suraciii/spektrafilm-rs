//! Live work-unit progress for frontends.
//!
//! A frontend binds one [`ProgressSink`] to the worker thread that runs an
//! operation and reads the innermost executing unit through RAII tokens. The
//! pipeline opens a token at every stage boundary in
//! [`crate::stages::StageObservation`], so one registration reports the whole
//! render. Reporting observes unit boundaries only: it must not change
//! routing, parameters, pixels, or telemetry. A thread runs at most one
//! observed operation at a time.

use std::cell::RefCell;
use std::sync::Arc;

/// Receives progress of one operation. Methods run on the worker thread that
/// executes the operation.
pub trait ProgressSink: Send + Sync {
    /// The innermost work unit now executing on the reporting thread, or
    /// `None` when execution leaves every unit.
    fn current(&self, unit: Option<&'static str>);
}

thread_local! {
    static SINK: RefCell<Option<Arc<dyn ProgressSink>>> = const { RefCell::new(None) };
    static STACK: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// Binds a sink to the current thread. Units reported before the binding and
/// after its drop do not reach the sink.
pub struct ProgressScope;

impl ProgressScope {
    pub fn new(sink: Arc<dyn ProgressSink>) -> Self {
        SINK.with(|slot| *slot.borrow_mut() = Some(sink));
        STACK.with(|stack| stack.borrow_mut().clear());
        Self
    }
}

impl Drop for ProgressScope {
    fn drop(&mut self) {
        STACK.with(|stack| stack.borrow_mut().clear());
        SINK.with(|slot| *slot.borrow_mut() = None);
    }
}

/// Marks `unit` as executing until the returned token drops. Nested tokens
/// report the innermost unit and restore the enclosing one on drop.
pub fn report(unit: &'static str) -> ProgressToken {
    STACK.with(|stack| stack.borrow_mut().push(unit));
    emit();
    ProgressToken { unit }
}

pub struct ProgressToken {
    unit: &'static str,
}

impl Drop for ProgressToken {
    fn drop(&mut self) {
        STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.last() == Some(&self.unit) {
                stack.pop();
            }
        });
        emit();
    }
}

fn emit() {
    let unit = STACK.with(|stack| stack.borrow().last().copied());
    SINK.with(|slot| {
        if let Some(sink) = slot.borrow().as_ref() {
            sink.current(unit);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Recording(Mutex<Vec<Option<&'static str>>>);

    impl ProgressSink for Recording {
        fn current(&self, unit: Option<&'static str>) {
            self.0.lock().unwrap().push(unit);
        }
    }

    #[test]
    fn nested_units_report_the_innermost_and_restore_the_enclosing() {
        let sink = Arc::new(Recording(Mutex::new(Vec::new())));
        let scope = ProgressScope::new(sink.clone());
        let outer = report("outer");
        let inner = report("inner");
        drop(inner);
        drop(outer);
        drop(scope);
        assert_eq!(
            *sink.0.lock().unwrap(),
            vec![Some("outer"), Some("inner"), Some("outer"), None]
        );
    }

    #[test]
    fn a_thread_without_a_scope_reports_nothing() {
        let sink = Arc::new(Recording(Mutex::new(Vec::new())));
        {
            let _scope = ProgressScope::new(sink.clone());
            let _unit = report("outer");
        }
        sink.0.lock().unwrap().clear();
        let _unit = report("unobserved");
        drop(_unit);
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[test]
    fn a_stage_observation_reports_its_unit_to_the_bound_sink() {
        // The stage choke point every pipeline stage constructs, with
        // telemetry off: frontend progress must not depend on collection.
        static BACKEND: spektrafilm_backend::cpu_backend::CpuBackend =
            spektrafilm_backend::cpu_backend::CpuBackend;
        let sink = Arc::new(Recording(Mutex::new(Vec::new())));
        let _scope = ProgressScope::new(sink.clone());
        {
            let _stage = crate::stages::StageObservation::new(&BACKEND, "filming_develop");
            assert_eq!(
                sink.0.lock().unwrap().last(),
                Some(&Some("filming_develop"))
            );
        }
        assert_eq!(*sink.0.lock().unwrap(), vec![Some("filming_develop"), None]);
    }
}
