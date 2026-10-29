//! Observation collection infrastructure: scopes, collectors, and recorders.
use super::types::*;
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

struct Collector {
    start: Instant,
    mode: CollectionMode,
    next: u64,
    snapshot: ObservationSnapshot,
    retained_summaries: u64,
    detail_bytes: u64,
    detail_byte_limit: u64,
    group_state: Vec<GroupState>,
}
#[derive(Default)]
struct GroupState {
    completed: u64,
    failure: Option<AvailabilityReason>,
    overflow: bool,
}
// Root facts, bounded issues, fixed phase totals and overflow field names reserve
// enough space independently of detailed observation admission.
const MAX_GROUPS: usize = GpuStage::ALL.len() * GpuPassKind::ALL.len() * 3;
const MAX_STAGE_TOTALS: usize = STAGE_NAMES.len() + GpuStage::ALL.len();
const ROOT_BYTES: u64 = 16 * 1024
    + (MAX_GROUPS * (std::mem::size_of::<GpuTimingGroup>() + std::mem::size_of::<GroupState>()))
        as u64
    + ((OBSERVATION_LIMIT + 1) * HostTimingKind::ALL.len() * std::mem::size_of::<HostTimingTotal>())
        as u64
    + (MAX_STAGE_TOTALS * (std::mem::size_of::<PhaseTotal>() + 64)) as u64;
// Reserve space for per-observation inclusive host totals and the complete finite
// GPU key space before admitting details, within the existing one MiB budget.
const OBSERVATION_LIMIT: usize = 512;
const SUMMARY_LIMIT: u64 = 256;
const INITIAL_BYTES: u64 =
    ROOT_BYTES + (OBSERVATION_LIMIT * std::mem::size_of::<Observation>()) as u64;
// Leave half the dynamic storage for late attempts, phases, and stages.
const DETAIL_BYTE_LIMIT: u64 = (1048576 - INITIAL_BYTES) / 2;

fn is_summary(kind: ObservationKind) -> bool {
    matches!(
        kind,
        ObservationKind::Attempt | ObservationKind::Phase | ObservationKind::Stage
    )
}

fn execution_bytes(facts: &ExecutionFacts) -> u64 {
    (facts.resident_decline_reasons.capacity() * std::mem::size_of::<ResidentDeclineReason>()
        + facts.adapter.as_ref().map_or(0, |adapter| {
            adapter.api.capacity() + adapter.name.capacity()
        })) as u64
}

fn observation_bytes(observation: &Observation) -> u64 {
    observation.name.capacity() as u64
        + observation.execution.as_ref().map_or(0, execution_bytes)
        + observation
            .batch
            .as_ref()
            .map_or(0, |batch| batch.completion_boundary.capacity() as u64)
}
fn add_host_timing(totals: &mut Vec<HostTimingTotal>, kind: HostTimingKind, seconds: f64) -> bool {
    if let Some(total) = totals.iter_mut().find(|total| total.kind == kind) {
        let Some(count) = total.count.checked_add(1) else {
            return false;
        };
        let duration = total.total_duration + seconds;
        if !duration.is_finite() {
            return false;
        }
        total.count = count;
        total.total_duration = duration;
    } else {
        if totals.capacity() == 0 {
            totals.reserve_exact(HostTimingKind::ALL.len());
        }
        totals.push(HostTimingTotal {
            kind,
            count: 1,
            total_duration: seconds,
        });
    }
    true
}

fn refresh_gpu_totals(collector: &mut Collector) {
    let unavailable = if collector.mode != CollectionMode::GpuTiming {
        Some(AvailabilityReason::NotCollected)
    } else {
        collector
            .snapshot
            .execution
            .adapter
            .as_ref()
            .and_then(|adapter| adapter.timestamp_reason)
    };
    let timing = &mut collector.snapshot.gpu_timing;
    let mut partial = 0.0;
    let mut valid = false;
    let mut complete = true;
    let mut reason = None;
    let mut has_failure = false;
    let mut group_overflow = false;
    for (group, state) in timing.groups.iter_mut().zip(&collector.group_state) {
        let pending = state.completed < group.timed_pass_count;
        let group_reason = if state.overflow {
            AvailabilityReason::CounterOverflow
        } else if let Some(reason) = unavailable {
            reason
        } else if let Some(reason) = state.failure {
            reason
        } else if group.executed_pass_count > group.timed_pass_count {
            AvailabilityReason::Incomplete
        } else if pending {
            AvailabilityReason::NotReady
        } else {
            AvailabilityReason::Incomplete
        };
        if group.valid_pass_count == 0 && !state.overflow {
            group.partial_compute_pass_sum = Measurement::unavailable(group_reason);
        }
        let full = !state.overflow
            && group.executed_pass_count > 0
            && group.valid_pass_count == group.executed_pass_count;
        group.compute_pass_sum = if full {
            group.partial_compute_pass_sum.clone()
        } else {
            Measurement::unavailable(group_reason)
        };
        if let Some(value) = group.partial_compute_pass_sum.value {
            partial += value;
            valid = true;
        }
        complete &= full;
        group_overflow |= state.overflow;
        let group_has_failure = state.failure == Some(group_reason);
        if !full
            && (reason.is_none()
                || (!has_failure && group_has_failure)
                || (!has_failure
                    && reason == Some(AvailabilityReason::NotReady)
                    && group_reason != AvailabilityReason::NotReady))
        {
            reason = Some(group_reason);
            has_failure = group_has_failure;
        }
    }
    let overflow = group_overflow || !partial.is_finite();
    timing.partial_compute_pass_sum = if overflow {
        Measurement::unavailable(AvailabilityReason::CounterOverflow)
    } else if valid {
        Measurement::available(partial)
    } else {
        Measurement::unavailable(reason.unwrap_or(AvailabilityReason::NotCollected))
    };
    timing.compute_pass_sum = if overflow {
        Measurement::unavailable(AvailabilityReason::CounterOverflow)
    } else if complete && valid {
        Measurement::available(partial)
    } else if valid {
        Measurement::unavailable(AvailabilityReason::Incomplete)
    } else {
        Measurement::unavailable(reason.unwrap_or(AvailabilityReason::NotCollected))
    };
}

fn bounded_description(value: &mut String) -> bool {
    if let Some((end, _)) = value.char_indices().nth(256) {
        value.truncate(end);
        value.shrink_to_fit();
        true
    } else {
        value.shrink_to_fit();
        false
    }
}
#[derive(Clone)]
struct ScopeMeta {
    name: &'static str,
    kind: ObservationKind,
}
#[derive(Clone, Default)]
pub struct ObservationContext {
    collector: Option<Arc<Mutex<Collector>>>,
    id: u64,
    retained_id: u64,
    root: u64,
    retained: bool,
    purpose: Option<Purpose>,
    gpu_stage: GpuStage,
    explicit_gpu_stage: bool,
    meta: Option<ScopeMeta>,
    span: Option<tracing::Span>,
    progress: Option<super::progress::ProgressSender>,
}
impl std::fmt::Debug for ObservationContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservationContext")
            .field("id", &self.id)
            .field("root", &self.root)
            .field("enabled", &self.enabled())
            .field("retained", &self.retained)
            .field("purpose", &self.purpose)
            .finish()
    }
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
impl ObservationContext {
    pub fn new(mode: CollectionMode) -> Self {
        if mode == CollectionMode::Off {
            return Self::default();
        }
        let root_id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            collector: Some(Arc::new(Mutex::new(Collector {
                start: Instant::now(),
                mode,
                next: root_id + 1,
                retained_summaries: 0,
                detail_bytes: 0,
                detail_byte_limit: DETAIL_BYTE_LIMIT,
                group_state: Vec::new(),
                snapshot: ObservationSnapshot {
                    root_id,
                    duration: 0.0,
                    outcome: None,
                    observations: Vec::with_capacity(OBSERVATION_LIMIT),
                    totals: WorkCounters::default(),
                    gpu_timing: GpuTimingMeasurements::default(),
                    host_timings: Some(Vec::new()),
                    coverage: Coverage {
                        detail_limit: OBSERVATION_LIMIT as u64,
                        retained_bytes: INITIAL_BYTES,
                        ..Coverage::default()
                    },
                    execution: ExecutionFacts {
                        backend_selected: None,
                        path: None,
                        effective_mode: Some(mode),
                        resident_decline_reasons: Vec::new(),
                        adapter: None,
                        working_dimensions: None,
                    },
                    issues: Vec::new(),
                },
            }))),
            id: root_id,
            retained_id: root_id,
            root: root_id,
            retained: true,
            purpose: Some(Purpose::Image),
            gpu_stage: GpuStage::Other,
            explicit_gpu_stage: false,
            meta: Some(ScopeMeta {
                name: "operation",
                kind: ObservationKind::Attempt,
            }),
            span: Some(
                tracing::debug_span!(target:"spektrafilm::telemetry", parent: None, "operation", observation_root_id=root_id),
            ),
            progress: None,
        }
    }
    pub fn enabled(&self) -> bool {
        self.collector.is_some()
    }
    pub fn retained(&self) -> bool {
        self.retained
    }
    pub fn span(&self) -> Option<&tracing::Span> {
        self.span.as_ref()
    }
    pub fn adapter_reason(&self) -> Option<AvailabilityReason> {
        self.collector.as_ref().and_then(|c| {
            c.lock()
                .snapshot
                .execution
                .adapter
                .as_ref()
                .and_then(|a| a.timestamp_reason)
        })
    }
    pub fn set_effective_mode(&self, mode: CollectionMode) {
        self.mutate(|s| s.execution.effective_mode = Some(mode));
    }
    pub fn mode(&self) -> CollectionMode {
        self.collector
            .as_ref()
            .map_or(CollectionMode::Off, |c| c.lock().mode)
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn root_id(&self) -> u64 {
        self.root
    }
    /// Attach before cloning the root context for worker dispatch.
    pub fn progress(&mut self) -> Option<super::CoarseProgressReceiver> {
        if self.progress.is_some() {
            return None;
        }
        let start = self.collector.as_ref()?.lock().start;
        let (sender, receiver) = super::progress::channel(start);
        self.progress = Some(sender);
        Some(receiver)
    }

    /// Nonblocking readiness query for frontend deferral; no filesystem access.
    pub fn progress_ready(&self) -> bool {
        self.progress
            .as_ref()
            .is_none_or(|progress| progress.is_ready())
    }

    /// Image-worker acceptance gate. Never call from a GUI frame callback.
    pub fn wait_for_progress(&self) {
        if let Some(progress) = &self.progress
            && !progress.wait_ready()
        {
            self.issue(
                IssueCategory::CollectionUnavailable,
                IssueBoundary::Operation,
            );
        }
    }

    /// Return false on cancellation, without treating it as collection failure.
    /// True admits image work after durable begin or a separate diagnostic failure.
    /// Uses 10 ms timed waits to recheck cancellation/shutdown tokens.
    pub fn wait_for_progress_or_cancel(&self, mut cancelled: impl FnMut() -> bool) -> bool {
        let Some(progress) = &self.progress else {
            return !cancelled();
        };
        let Some(ready) = progress.wait_ready_or_cancel(&mut cancelled) else {
            return false;
        };
        if !ready {
            self.issue(
                IssueCategory::CollectionUnavailable,
                IssueBoundary::Operation,
            );
        }
        true
    }

    /// Coarse chain boundaries are progress only, not nested Stage observations.
    pub fn chain_progress(&self, name: &'static str, begin: bool) {
        self.emit_progress(name, ObservationKind::Stage, self.id, begin);
    }

    fn emit_progress(&self, name: &'static str, kind: ObservationKind, id: u64, begin: bool) {
        if is_summary(kind)
            && let Some(progress) = &self.progress
        {
            progress.send(super::ProgressEvent {
                id,
                name,
                kind,
                begin,
            });
        }
    }
    pub fn purpose(&self) -> Purpose {
        self.purpose.unwrap_or(Purpose::Image)
    }
    pub fn with_gpu_stage(&self, stage: GpuStage) -> Self {
        let mut context = self.clone();
        context.gpu_stage = stage;
        context.explicit_gpu_stage = true;
        context
    }
    pub fn gpu_stage(&self) -> GpuStage {
        self.gpu_stage
    }
    pub fn scope(
        &self,
        name: &'static str,
        kind: ObservationKind,
        purpose: Purpose,
    ) -> ObservationScope {
        self.make_scope(name, kind, purpose, None)
    }
    pub fn attempt(&self, index: u64) -> ObservationScope {
        self.make_scope(
            "attempt",
            ObservationKind::Attempt,
            self.purpose(),
            Some(index),
        )
    }
    fn make_scope(
        &self,
        name: &'static str,
        kind: ObservationKind,
        purpose: Purpose,
        index: Option<u64>,
    ) -> ObservationScope {
        let Some(c) = &self.collector else {
            return ObservationScope::default();
        };
        let mut c = c.lock();
        if c.snapshot.outcome.is_some() {
            return ObservationScope::default();
        }
        let id = c.next;
        c.next = c.next.saturating_add(1);
        let bytes = name.len() as u64;
        let summary = is_summary(kind);
        let slots_available = if summary {
            c.retained_summaries < SUMMARY_LIMIT
        } else {
            c.snapshot.coverage.retained_observations - c.retained_summaries
                < OBSERVATION_LIMIT as u64 - SUMMARY_LIMIT
        };
        let retain = self.retained
            && slots_available
            && c.snapshot.coverage.retained_bytes + bytes <= c.snapshot.coverage.byte_limit
            && (summary || c.detail_bytes + bytes <= c.detail_byte_limit);
        let start = Instant::now();
        let offset = start.duration_since(c.start).as_secs_f64();
        if retain {
            c.snapshot.observations.push(Observation {
                id,
                parent_id: self.id,
                name: name.into(),
                kind,
                purpose,
                start_offset: offset,
                duration: 0.0,
                complete: false,
                outcome: None,
                attempt_index: index,
                executor: None,
                cpu_reason: None,
                work: WorkCounters::default(),
                batch: None,
                pass: None,
                host_timings: Some(Vec::new()),
                execution: if kind == ObservationKind::Attempt {
                    Some(ExecutionFacts::default())
                } else {
                    None
                },
            });
            c.snapshot.coverage.retained_observations += 1;
            c.snapshot.coverage.retained_bytes += bytes;
            if summary {
                c.retained_summaries += 1;
            } else {
                c.detail_bytes += bytes;
            }
        } else {
            c.snapshot.coverage.omitted_observations =
                c.snapshot.coverage.omitted_observations.saturating_add(1);
            c.snapshot.coverage.truncated = true;
            if summary {
                c.snapshot.coverage.omitted_high_level_observations = c
                    .snapshot
                    .coverage
                    .omitted_high_level_observations
                    .and_then(|count| count.checked_add(1));
            }
        }
        drop(c);
        self.emit_progress(name, kind, id, true);
        let _entered = self.span.as_ref().map(|span| span.enter());
        tracing::debug!(target:"spektrafilm::telemetry",parent:self.span.as_ref().and_then(|s|s.id()),event="observation_start",observation_root_id=self.root,observation_id=id,parent_id=self.id,name,kind=?kind,purpose=?purpose,start_offset=offset,attempt_index=index);
        let parent_span_id = self.span.as_ref().and_then(|s| s.id());
        ObservationScope {
            context: Self {
                collector: self.collector.clone(),
                id,
                retained_id: if retain { id } else { self.retained_id },
                root: self.root,
                retained: retain,
                purpose: Some(purpose),
                gpu_stage: if self.explicit_gpu_stage {
                    self.gpu_stage
                } else {
                    match GpuStage::from_scope_name(name) {
                        GpuStage::Other => self.gpu_stage,
                        stage => stage,
                    }
                },
                explicit_gpu_stage: self.explicit_gpu_stage,
                meta: Some(ScopeMeta { name, kind }),
                progress: self.progress.clone(),
                span: Some(
                    tracing::debug_span!(target:"spektrafilm::telemetry", parent: parent_span_id, "observation", observation_root_id=self.root, observation_id=id, parent_id=self.id, name, kind=?kind),
                ),
            },
            start: if kind == ObservationKind::Pass {
                None
            } else {
                Some(start)
            },
            kind: Some(kind),
            complete: true,
            outcome: None,
        }
    }
    pub fn detail_remaining(&self) -> u32 {
        self.collector.as_ref().map_or(0, |c| {
            let c = c.lock();
            if !self.retained {
                0
            } else {
                let slots = OBSERVATION_LIMIT as u64
                    - SUMMARY_LIMIT
                    - (c.snapshot.coverage.retained_observations - c.retained_summaries);
                let bytes = (c.detail_byte_limit - c.detail_bytes)
                    .min(c.snapshot.coverage.byte_limit - c.snapshot.coverage.retained_bytes);
                // Every pass name consumes at least one byte. Admission charges
                // the actual name length as well as checking the slot budget.
                slots.min(bytes) as u32
            }
        })
    }
    pub fn record_omitted_details(&self, count: u64) {
        if count > 0 {
            self.mutate(|s| {
                s.coverage.omitted_observations =
                    s.coverage.omitted_observations.saturating_add(count);
                s.coverage.truncated = true;
            });
        }
    }
    pub fn snapshot(&self) -> ObservationSnapshot {
        let mut s = self
            .collector
            .as_ref()
            .expect("disabled context has no snapshot")
            .lock()
            .snapshot
            .clone();
        if s.outcome.is_none() {
            s.duration = self
                .collector
                .as_ref()
                .unwrap()
                .lock()
                .start
                .elapsed()
                .as_secs_f64()
        }
        s
    }
    pub fn finish(&self, outcome: Outcome) {
        if let Some(c) = &self.collector {
            let _entered = self.span.as_ref().map(|span| span.enter());
            let mut c = c.lock();
            if c.snapshot.outcome.is_some() {
                return;
            }
            c.snapshot.duration = c.start.elapsed().as_secs_f64();
            c.snapshot.outcome = Some(outcome);
            tracing::debug!(target:"spektrafilm::telemetry",parent:self.span.as_ref().and_then(|s|s.id()),event="operation_finish",observation_root_id=self.root,outcome=?outcome,duration=c.snapshot.duration);
        }
        if let Some(progress) = &self.progress {
            progress.finish(outcome);
        }
    }
    fn mutate(&self, f: impl FnOnce(&mut ObservationSnapshot)) {
        if let Some(c) = &self.collector {
            let _entered = self.span.as_ref().map(|span| span.enter());
            let mut c = c.lock();
            if c.snapshot.outcome.is_none() {
                f(&mut c.snapshot);
            }
        }
    }
    pub fn set_backend_selected(&self, value: BackendSelected) {
        let _entered = self.span.as_ref().map(|span| span.enter());
        self.mutate(|s| s.execution.backend_selected = Some(value));
        self.attempt_fact(|f| f.backend_selected = Some(value));
        if value == BackendSelected::Cpu && self.mode() == CollectionMode::GpuTiming {
            self.mutate(|s| {
                if s.gpu_timing.compute_pass_sum.reason == Some(AvailabilityReason::NotCollected) {
                    s.gpu_timing.compute_pass_sum =
                        Measurement::unavailable(AvailabilityReason::Unsupported);
                    s.gpu_timing.partial_compute_pass_sum =
                        Measurement::unavailable(AvailabilityReason::Unsupported);
                }
            });
        }
        if self.enabled() {
            tracing::debug!(target:"spektrafilm::telemetry",parent:self.span.as_ref().and_then(|s|s.id()),event="route",observation_root_id=self.root,observation_id=self.id,backend_selected=?value)
        }
    }
    pub fn set_path(&self, value: ExecutionPath) {
        let _entered = self.span.as_ref().map(|span| span.enter());
        self.mutate(|s| {
            s.execution.path = Some(match s.execution.path {
                Some(old) if old != value => ExecutionPath::Mixed,
                _ => value,
            })
        });
        self.attempt_fact(|f| {
            f.path = Some(match f.path {
                Some(old) if old != value => ExecutionPath::Mixed,
                _ => value,
            })
        });
        if self.enabled() {
            tracing::debug!(target:"spektrafilm::telemetry",parent:self.span.as_ref().and_then(|s|s.id()),event="route",observation_root_id=self.root,observation_id=self.id,execution_path=?value)
        }
    }
    pub fn decline_resident(&self, value: ResidentDeclineReason) {
        let _entered = self.span.as_ref().map(|span| span.enter());
        self.mutate(|s| {
            if !s.execution.resident_decline_reasons.contains(&value) {
                s.execution.resident_decline_reasons.push(value)
            }
        });
        self.attempt_fact(|f| {
            if !f.resident_decline_reasons.contains(&value) {
                f.resident_decline_reasons.push(value)
            }
        });
        if self.enabled() {
            tracing::debug!(target:"spektrafilm::telemetry",parent:self.span.as_ref().and_then(|s|s.id()),event="route",observation_root_id=self.root,observation_id=self.id,resident_decline_reason=?value)
        }
    }
    pub fn set_working_dimensions(&self, width: u32, height: u32) {
        let _entered = self.span.as_ref().map(|span| span.enter());
        self.mutate(|s| s.execution.working_dimensions = Some([width, height]));
        self.attempt_fact(|f| f.working_dimensions = Some([width, height]));
        if self.enabled() {
            tracing::debug!(target:"spektrafilm::telemetry",parent:self.span.as_ref().and_then(|s|s.id()),event="route",observation_root_id=self.root,observation_id=self.id,working_width=width,working_height=height)
        }
    }
    pub fn set_adapter(&self, mut value: AdapterDescription) {
        if !self.enabled() {
            return;
        }
        let _entered = self.span.as_ref().map(|span| span.enter());
        value.description_truncated |= bounded_description(&mut value.api);
        value.description_truncated |= bounded_description(&mut value.name);
        self.mutate(|s| s.execution.adapter = Some(value.clone()));
        self.attempt_fact(|f| f.adapter = Some(value.clone()));
        tracing::debug!(target:"spektrafilm::telemetry",parent:self.span.as_ref().and_then(|s|s.id()),event="route",observation_root_id=self.root,observation_id=self.id,adapter_type=?value.device_type,api=value.api.as_str(),timestamp_status=?value.timestamp_period_ns.status,timestamp_period_ns=value.timestamp_period_ns.value);
    }
    fn attempt_fact(&self, f: impl FnOnce(&mut ExecutionFacts)) {
        if let Some(c) = &self.collector {
            let mut c = c.lock();
            if c.snapshot.outcome.is_some() {
                return;
            }
            let mut id = self.retained_id;
            loop {
                let Some(o) = c.snapshot.observations.iter_mut().find(|o| o.id == id) else {
                    return;
                };
                if let Some(execution) = &mut o.execution {
                    let old_bytes = execution_bytes(execution);
                    let mut updated = execution.clone();
                    f(&mut updated);
                    let bytes =
                        c.snapshot.coverage.retained_bytes - old_bytes + execution_bytes(&updated);
                    if bytes <= c.snapshot.coverage.byte_limit {
                        let o = c
                            .snapshot
                            .observations
                            .iter_mut()
                            .find(|o| o.id == id)
                            .unwrap();
                        o.execution = Some(updated);
                        c.snapshot.coverage.retained_bytes = bytes;
                    } else {
                        c.snapshot.coverage.truncated = true;
                    }
                    return;
                }
                id = o.parent_id;
                if id == 0 {
                    return;
                }
            }
        }
    }
    pub fn issue(&self, category: IssueCategory, boundary: IssueBoundary) {
        self.mutate(|s| {
            if s.issues.len() < 64 {
                s.issues.push(DiagnosticIssue { category, boundary })
            } else {
                s.coverage.omitted_issues = s.coverage.omitted_issues.saturating_add(1)
            }
        })
    }
    pub fn record_executor(&self, value: Executor, reason: Option<CpuReason>) {
        self.mutate(|s| {
            let mut id = self.retained_id;
            while id != s.root_id {
                let Some(o) = s.observations.iter_mut().find(|o| o.id == id) else {
                    break;
                };
                o.executor = Some(match o.executor {
                    Some(old) if old != value => Executor::Mixed,
                    _ => value,
                });
                if reason.is_some() && o.kind == ObservationKind::Stage {
                    o.cpu_reason = reason;
                }
                id = o.parent_id;
            }
        })
    }
    pub fn record_work(&self, work: &WorkCounters) {
        let _entered = self.span.as_ref().map(|span| span.enter());
        self.mutate(|s| {
            let mut overflow = s.totals.add(work);
            let mut id = self.retained_id;
            while id != s.root_id {
                let Some(o) = s.observations.iter_mut().find(|o| o.id == id) else {
                    break;
                };
                overflow.extend(o.work.add(work));
                id = o.parent_id;
            }
            let coverage = &mut s.coverage;
            for field in overflow {
                if !coverage.counter_overflow_fields.iter().any(|f| f == field) {
                    coverage.counter_overflow_fields.push(field.into());
                }
            }
        });
        if self.enabled() {
            tracing::debug!(target:"spektrafilm::telemetry",parent:self.span.as_ref().and_then(|s|s.id()),event="work",observation_root_id=self.root,observation_id=self.id,dispatch_count=work.dispatch_count,submit_count=work.submit_count,command_buffer_count=work.command_buffer_count,host_wait_count=work.host_wait_count,upload_count=work.upload_count,upload_bytes=work.upload_bytes,readback_count=work.readback_count,readback_bytes=work.readback_bytes,executed_pass_count=work.executed_pass_count,timed_pass_count=work.timed_pass_count,valid_pass_count=work.valid_pass_count,omitted_pass_count=work.omitted_pass_count);
        }
    }
    /// Records inclusive host intervals. Categories may overlap and are not exclusive CPU time.
    pub fn record_host_timing(&self, kind: HostTimingKind, seconds: f64) {
        let Some(c) = &self.collector else { return };
        let mut c = c.lock();
        if c.snapshot.outcome.is_some() || c.snapshot.coverage.host_timing_overflow {
            return;
        }
        let s = &mut c.snapshot;
        let mut ok = seconds.is_finite() && seconds >= 0.0;
        if ok {
            ok = add_host_timing(s.host_timings.as_mut().unwrap(), kind, seconds);
            let mut id = self.retained_id;
            while id != s.root_id {
                let Some(o) = s.observations.iter_mut().find(|o| o.id == id) else {
                    break;
                };
                ok &= add_host_timing(o.host_timings.as_mut().unwrap(), kind, seconds);
                id = o.parent_id;
            }
        }
        if !ok {
            s.coverage.truncated = true;
            s.coverage.host_timing_overflow = true;
            s.host_timings.as_mut().unwrap().clear();
            for observation in &mut s.observations {
                observation.host_timings.as_mut().unwrap().clear();
            }
        }
    }

    /// Registers only submitted work; callers separately record the work counters.
    pub fn record_gpu_pass_submission(
        &self,
        stage: GpuStage,
        kind: GpuPassKind,
        executed: u64,
        timed: u64,
    ) {
        let Some(c) = &self.collector else { return };
        let mut c = c.lock();
        if c.snapshot.outcome.is_some() || executed == 0 {
            return;
        }
        let purpose = self.purpose();
        let index = match c
            .snapshot
            .gpu_timing
            .groups
            .iter()
            .position(|g| g.stage == stage && g.kind == kind && g.purpose == purpose)
        {
            Some(index) => index,
            None => {
                if c.group_state.is_empty() {
                    c.group_state.reserve_exact(MAX_GROUPS);
                    c.snapshot.gpu_timing.groups.reserve_exact(MAX_GROUPS);
                }
                c.group_state.push(GroupState::default());
                c.snapshot.gpu_timing.groups.push(GpuTimingGroup {
                    stage,
                    kind,
                    purpose,
                    executed_pass_count: 0,
                    timed_pass_count: 0,
                    valid_pass_count: 0,
                    compute_pass_sum: Measurement::unavailable(AvailabilityReason::NotReady),
                    partial_compute_pass_sum: Measurement::unavailable(
                        AvailabilityReason::NotReady,
                    ),
                });
                c.group_state.len() - 1
            }
        };
        let group = &mut c.snapshot.gpu_timing.groups[index];
        let next_executed = group.executed_pass_count.checked_add(executed);
        let next_timed = group.timed_pass_count.checked_add(timed);
        group.executed_pass_count = next_executed.unwrap_or(u64::MAX);
        group.timed_pass_count = next_timed
            .unwrap_or(u64::MAX)
            .min(group.executed_pass_count);
        if next_executed.is_none() || next_timed.is_none() || timed > executed {
            c.group_state[index].overflow = true;
        }
        refresh_gpu_totals(&mut c);
    }

    /// Records why submitted passes could not receive queries, without completing a timed pass.
    pub fn record_gpu_pass_unavailable(
        &self,
        stage: GpuStage,
        kind: GpuPassKind,
        reason: AvailabilityReason,
    ) {
        let Some(c) = &self.collector else { return };
        let mut c = c.lock();
        if c.snapshot.outcome.is_some() {
            return;
        }
        let Some(index) = c
            .snapshot
            .gpu_timing
            .groups
            .iter()
            .position(|g| g.stage == stage && g.kind == kind && g.purpose == self.purpose())
        else {
            return;
        };
        c.group_state[index].failure = Some(reason);
        refresh_gpu_totals(&mut c);
    }

    /// Completes one registered timed pass, updating inclusive valid-pass counters once.
    pub fn record_gpu_pass_duration(
        &self,
        stage: GpuStage,
        kind: GpuPassKind,
        duration: Measurement,
    ) {
        let Some(c) = &self.collector else { return };
        let mut c = c.lock();
        if c.snapshot.outcome.is_some() {
            return;
        }
        let Some(index) = c
            .snapshot
            .gpu_timing
            .groups
            .iter()
            .position(|g| g.stage == stage && g.kind == kind && g.purpose == self.purpose())
        else {
            return;
        };
        if c.group_state[index].completed >= c.snapshot.gpu_timing.groups[index].timed_pass_count {
            return;
        }
        c.group_state[index].completed += 1;
        let value = duration.value.filter(|v| {
            v.is_finite()
                && *v >= 0.0
                && duration.status == MeasurementStatus::Available
                && duration.reason.is_none()
        });
        if let Some(value) = value {
            let was_overflow = c.group_state[index].overflow;
            let group = &mut c.snapshot.gpu_timing.groups[index];
            let sum = group.partial_compute_pass_sum.value.unwrap_or(0.0) + value;
            group.valid_pass_count += 1;
            if !was_overflow && sum.is_finite() {
                group.partial_compute_pass_sum = Measurement::available(sum);
            } else {
                group.partial_compute_pass_sum =
                    Measurement::unavailable(AvailabilityReason::CounterOverflow);
                c.group_state[index].overflow = true;
            }
            let work = WorkCounters {
                valid_pass_count: Some(1),
                ..Default::default()
            };
            let s = &mut c.snapshot;
            let mut overflow = s.totals.add(&work);
            let mut id = self.retained_id;
            while id != s.root_id {
                let Some(o) = s.observations.iter_mut().find(|o| o.id == id) else {
                    break;
                };
                overflow.extend(o.work.add(&work));
                id = o.parent_id;
            }
            for field in overflow {
                if !s
                    .coverage
                    .counter_overflow_fields
                    .iter()
                    .any(|f| f == field)
                {
                    s.coverage.counter_overflow_fields.push(field.into());
                }
            }
        } else {
            c.group_state[index].failure = Some(
                duration
                    .reason
                    .unwrap_or(AvailabilityReason::InvalidTimestamp),
            );
        }
        refresh_gpu_totals(&mut c);
    }
    pub fn record_batch(&self, mut batch: BatchMeasurements) {
        let Some(c) = &self.collector else {
            return;
        };
        let _entered = self.span.as_ref().map(|span| span.enter());
        // Completion boundaries are backend vocabulary, never free-form diagnostics.
        bounded_description(&mut batch.completion_boundary);
        tracing::debug!(target:"spektrafilm::telemetry",parent:self.span.as_ref().and_then(|s|s.id()),event="batch",observation_root_id=self.root,observation_id=self.id,host_prepare_status=?batch.host_prepare.status,host_prepare=batch.host_prepare.value,submit_to_map_ready_status=?batch.submit_to_map_ready.status,submit_to_map_ready=batch.submit_to_map_ready.value,host_materialize_status=?batch.host_materialize.status,host_materialize=batch.host_materialize.value,compute_pass_sum_status=?batch.compute_pass_sum.status,compute_pass_sum=batch.compute_pass_sum.value,partial_compute_pass_sum_status=?batch.partial_compute_pass_sum.status,partial_compute_pass_sum=batch.partial_compute_pass_sum.value,completion_boundary=batch.completion_boundary.as_str());
        let mut c = c.lock();
        if c.snapshot.outcome.is_some() {
            return;
        }
        if c.group_state.is_empty() {
            c.snapshot
                .gpu_timing
                .add(&batch.compute_pass_sum, &batch.partial_compute_pass_sum);
        }
        if let Some(index) = c.snapshot.observations.iter().position(|o| o.id == self.id) {
            let o = &c.snapshot.observations[index];
            let old_bytes = observation_bytes(o);
            let added = batch.completion_boundary.capacity() as u64;
            let removed = o
                .batch
                .as_ref()
                .map_or(0, |b| b.completion_boundary.capacity() as u64);
            let summary = is_summary(o.kind);
            if c.snapshot.coverage.retained_bytes + added - removed
                <= c.snapshot.coverage.byte_limit
                && (summary || c.detail_bytes + added - removed <= c.detail_byte_limit)
            {
                let o = &mut c.snapshot.observations[index];
                o.batch = Some(batch);
                let new_bytes = observation_bytes(o);
                c.snapshot.coverage.retained_bytes =
                    c.snapshot.coverage.retained_bytes - old_bytes + new_bytes;
                if !summary {
                    c.detail_bytes = c.detail_bytes + added - removed;
                }
            } else {
                c.snapshot.coverage.truncated = true;
            }
        }
    }
    pub fn record_pass(&self, occurrence: u64, duration: Measurement) {
        let _entered = self.span.as_ref().map(|span| span.enter());
        self.mutate(|s| {
            if let Some(o) = s.observations.iter_mut().find(|o| o.id == self.id) {
                o.duration = duration.value.unwrap_or(0.0);
                o.complete = duration.status == MeasurementStatus::Available;
                o.pass = Some(PassMeasurement {
                    occurrence,
                    duration: duration.clone(),
                })
            }
        });
        if self.enabled() {
            tracing::debug!(target:"spektrafilm::telemetry",parent:self.span.as_ref().and_then(|s|s.id()),event="pass",observation_root_id=self.root,observation_id=self.id,occurrence,duration_status=?duration.status,duration=duration.value)
        }
    }
}
#[derive(Default)]
pub struct ObservationScope {
    context: ObservationContext,
    start: Option<Instant>,
    kind: Option<ObservationKind>,
    complete: bool,
    outcome: Option<Outcome>,
}
impl ObservationScope {
    pub fn context(&self) -> &ObservationContext {
        &self.context
    }
    pub fn id(&self) -> u64 {
        self.context.id()
    }
    pub fn set_complete(&mut self, value: bool) {
        self.complete = value
    }
    pub fn set_outcome(&mut self, value: Outcome) {
        self.outcome = Some(value);
        if value != Outcome::Succeeded {
            self.complete = false
        }
    }
    pub fn finish(mut self, value: Outcome) {
        self.set_outcome(value)
    }
}
impl Drop for ObservationScope {
    fn drop(&mut self) {
        let Some(kind) = self.kind else { return };
        let _entered = self.context.span().map(|span| span.enter());
        self.complete &= !std::thread::panicking();
        let duration = self
            .start
            .map_or(0.0, |start| start.elapsed().as_secs_f64());
        self.context.mutate(|s| {
            s.coverage.fixed_duration_totals[kind as usize] += duration;
            let meta = self.context.meta.clone();
            if let Some(meta) = meta
                && meta.kind == ObservationKind::Phase
                && let Some(i) = PHASE_NAMES.iter().position(|&n| n == meta.name)
            {
                while s.coverage.phase_totals.len() <= i {
                    s.coverage.phase_totals.push(PhaseTotal {
                        name: PHASE_NAMES[s.coverage.phase_totals.len()].into(),
                        total_duration: 0.0,
                        count: 0,
                    });
                }
                let t = &mut s.coverage.phase_totals[i];
                t.total_duration += duration;
                t.count = t.count.saturating_add(1);
            }
            if kind == ObservationKind::Stage {
                let name = self.context.meta.as_ref().map_or("other", |meta| {
                    if STAGE_NAMES.contains(&meta.name)
                        || GpuStage::ALL
                            .iter()
                            .any(|stage| stage.as_str() == meta.name)
                    {
                        meta.name
                    } else {
                        "other"
                    }
                });
                if let Some(total) = s
                    .coverage
                    .stage_totals
                    .iter_mut()
                    .find(|total| total.name == name)
                {
                    let sum = total.total_duration + duration;
                    if sum.is_finite() {
                        total.total_duration = sum;
                        total.count = total.count.saturating_add(1);
                    } else {
                        s.coverage.truncated = true;
                    }
                } else {
                    if s.coverage.stage_totals.capacity() == 0 {
                        s.coverage.stage_totals.reserve_exact(MAX_STAGE_TOTALS);
                    }
                    s.coverage.stage_totals.push(PhaseTotal {
                        name: name.into(),
                        total_duration: duration,
                        count: 1,
                    });
                }
            }
            let Some(o) = s.observations.iter_mut().find(|o| o.id == self.context.id) else {
                return;
            };
            if kind != ObservationKind::Pass {
                o.duration = duration;
                o.complete = self.complete;
            }
            o.outcome = self.outcome;
        });
        if let Some(meta) = &self.context.meta {
            self.context
                .emit_progress(meta.name, kind, self.context.id, false);
        }
        if self.context.enabled() {
            tracing::debug!(target:"spektrafilm::telemetry",parent:self.context.span().and_then(|s|s.id()),event="observation_finish",observation_root_id=self.context.root_id(),observation_id=self.context.id(),kind=?kind,outcome=?self.outcome,complete=self.complete,duration)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn batch(compute: Measurement, partial: Measurement) -> BatchMeasurements {
        BatchMeasurements {
            host_prepare: Measurement::available(0.1),
            submit_to_map_ready: Measurement::available(0.2),
            host_materialize: Measurement::available(0.3),
            completion_boundary: "test".into(),
            compute_pass_sum: compute,
            partial_compute_pass_sum: partial,
        }
    }
    #[test]
    fn streamed_timings_survive_detail_truncation_and_ignore_late_completion() {
        let root = ObservationContext::new(CollectionMode::GpuTiming);
        let phase = root.scope("simulation", ObservationKind::Phase, Purpose::Image);
        let stage = phase
            .context()
            .scope("grain_spektra", ObservationKind::Stage, Purpose::Image);
        for _ in 0..OBSERVATION_LIMIT as u64 - SUMMARY_LIMIT {
            drop(
                stage
                    .context()
                    .scope("batch", ObservationKind::Batch, Purpose::Image),
            );
        }
        let omitted = stage
            .context()
            .scope("batch", ObservationKind::Batch, Purpose::Image);
        assert!(!omitted.context().retained());
        let ctx = omitted.context();
        ctx.record_host_timing(HostTimingKind::Upload, 0.25);
        ctx.record_gpu_pass_submission(GpuStage::GrainSpektra, GpuPassKind::Gather, 3, 2);
        ctx.record_gpu_pass_duration(
            GpuStage::GrainSpektra,
            GpuPassKind::Gather,
            Measurement::available(0.125),
        );
        let pending = root.snapshot();
        assert_eq!(
            pending.gpu_timing.groups[0].compute_pass_sum.reason,
            Some(AvailabilityReason::Incomplete)
        );
        assert_eq!(
            pending.gpu_timing.groups[0].partial_compute_pass_sum.value,
            Some(0.125)
        );
        assert_eq!(pending.totals.valid_pass_count, Some(1));
        for observation in &pending.observations[..2] {
            assert_eq!(observation.work.valid_pass_count, Some(1));
            assert_eq!(
                observation.host_timings.as_ref().unwrap()[0].total_duration,
                0.25
            );
        }
        ctx.record_gpu_pass_duration(
            GpuStage::GrainSpektra,
            GpuPassKind::Gather,
            Measurement::unavailable(AvailabilityReason::QueryFailed),
        );
        root.finish(Outcome::Succeeded);
        let published = root.snapshot();
        ctx.record_gpu_pass_duration(
            GpuStage::GrainSpektra,
            GpuPassKind::Gather,
            Measurement::available(1.0),
        );
        assert_eq!(root.snapshot().gpu_timing, published.gpu_timing);
        assert_eq!(root.snapshot().totals, published.totals);
        assert!(published.coverage.retained_bytes <= published.coverage.byte_limit);
    }

    #[test]
    fn pending_failure_and_concurrent_completions_are_truthful() {
        let root = ObservationContext::new(CollectionMode::GpuTiming);
        root.record_gpu_pass_submission(GpuStage::Scan, GpuPassKind::Filter, 64, 64);
        assert_eq!(
            root.snapshot().gpu_timing.compute_pass_sum.reason,
            Some(AvailabilityReason::NotReady)
        );
        std::thread::scope(|threads| {
            for _ in 0..8 {
                let context = root.clone();
                threads.spawn(move || {
                    for _ in 0..8 {
                        context.record_gpu_pass_duration(
                            GpuStage::Scan,
                            GpuPassKind::Filter,
                            Measurement::available(0.5),
                        );
                    }
                });
            }
        });
        let snapshot = root.snapshot();
        assert_eq!(snapshot.gpu_timing.compute_pass_sum.value, Some(32.0));
        assert_eq!(snapshot.gpu_timing.groups[0].valid_pass_count, 64);
        assert_eq!(snapshot.totals.valid_pass_count, Some(64));
        root.record_batch(batch(
            Measurement::available(32.0),
            Measurement::available(32.0),
        ));
        assert_eq!(
            root.snapshot().gpu_timing.compute_pass_sum.value,
            Some(32.0)
        );
        let other = ObservationContext::new(CollectionMode::GpuTiming);
        other.record_gpu_pass_submission(GpuStage::Scan, GpuPassKind::Filter, 1, 1);
        other.record_gpu_pass_duration(
            GpuStage::Scan,
            GpuPassKind::Filter,
            Measurement::unavailable(AvailabilityReason::QueryFailed),
        );
        assert_eq!(
            other.snapshot().gpu_timing.groups[0]
                .compute_pass_sum
                .reason,
            Some(AvailabilityReason::QueryFailed)
        );
    }

    #[test]
    fn known_gpu_failure_wins_over_pending_queries() {
        for failure in [
            AvailabilityReason::QueryFailed,
            AvailabilityReason::InvalidTimestamp,
        ] {
            for pending_group_first in [false, true] {
                let root = ObservationContext::new(CollectionMode::GpuTiming);
                if pending_group_first {
                    root.record_gpu_pass_submission(GpuStage::Input, GpuPassKind::Copy, 1, 1);
                }
                root.record_gpu_pass_submission(GpuStage::Scan, GpuPassKind::Filter, 2, 2);
                if !pending_group_first {
                    root.record_gpu_pass_submission(GpuStage::Input, GpuPassKind::Copy, 1, 1);
                }
                root.record_gpu_pass_duration(
                    GpuStage::Scan,
                    GpuPassKind::Filter,
                    Measurement::unavailable(failure),
                );
                let timing = root.snapshot().gpu_timing;
                let failed = timing
                    .groups
                    .iter()
                    .find(|group| group.stage == GpuStage::Scan)
                    .unwrap();
                assert_eq!(failed.compute_pass_sum.reason, Some(failure));
                assert_eq!(failed.partial_compute_pass_sum.reason, Some(failure));
                assert_eq!(timing.compute_pass_sum.reason, Some(failure));
                assert_eq!(timing.partial_compute_pass_sum.reason, Some(failure));

                root.record_gpu_pass_duration(
                    GpuStage::Input,
                    GpuPassKind::Copy,
                    Measurement::available(0.25),
                );
                let timing = root.snapshot().gpu_timing;
                assert_eq!(
                    timing.compute_pass_sum.reason,
                    Some(AvailabilityReason::Incomplete)
                );
                assert_eq!(timing.partial_compute_pass_sum.value, Some(0.25));
                let failed = timing
                    .groups
                    .iter()
                    .find(|group| group.stage == GpuStage::Scan)
                    .unwrap();
                assert_eq!(failed.compute_pass_sum.reason, Some(failure));
            }
        }
    }

    #[test]
    fn stage_totals_include_omitted_scopes_and_explicit_context_inherits() {
        let root = ObservationContext::new(CollectionMode::Summary);
        let phase = root.scope("simulation", ObservationKind::Phase, Purpose::Image);
        for _ in 0..(SUMMARY_LIMIT + 5) {
            let stage = phase.context().with_gpu_stage(GpuStage::Scan).scope(
                "scan",
                ObservationKind::Stage,
                Purpose::Image,
            );
            assert_eq!(stage.context().gpu_stage(), GpuStage::Scan);
            let child = stage
                .context()
                .scope("metering", ObservationKind::Stage, Purpose::Image);
            assert_eq!(child.context().gpu_stage(), GpuStage::Scan);
        }
        let snapshot = root.snapshot();
        let total = snapshot
            .coverage
            .stage_totals
            .iter()
            .find(|t| t.name == "scan")
            .unwrap();
        assert_eq!(total.count, SUMMARY_LIMIT + 5);
        let metering = snapshot
            .coverage
            .stage_totals
            .iter()
            .find(|t| t.name == "metering")
            .unwrap();
        assert_eq!(metering.count, SUMMARY_LIMIT + 5);
        assert!(snapshot.coverage.omitted_observations > 0);
    }
    #[test]
    fn aggregate_overflow_is_not_a_valid_duration_and_batches_do_not_double_count() {
        let root = ObservationContext::new(CollectionMode::GpuTiming);
        root.record_host_timing(HostTimingKind::Upload, f64::MAX);
        root.record_host_timing(HostTimingKind::Upload, f64::MAX);
        root.record_host_timing(HostTimingKind::Upload, 1.0);
        let snapshot = root.snapshot();
        assert!(snapshot.coverage.host_timing_overflow);
        assert!(snapshot.coverage.truncated);
        assert!(snapshot.host_timings.unwrap().is_empty());

        root.record_gpu_pass_submission(GpuStage::Scan, GpuPassKind::Filter, 2, 2);
        root.record_gpu_pass_duration(
            GpuStage::Scan,
            GpuPassKind::Filter,
            Measurement::available(f64::MAX),
        );
        root.record_gpu_pass_duration(
            GpuStage::Scan,
            GpuPassKind::Filter,
            Measurement::available(f64::MAX),
        );
        root.record_batch(batch(
            Measurement::available(1.0),
            Measurement::available(1.0),
        ));
        let snapshot = root.snapshot();
        assert_eq!(
            snapshot.gpu_timing.compute_pass_sum.reason,
            Some(AvailabilityReason::CounterOverflow)
        );
        assert_eq!(
            snapshot.gpu_timing.partial_compute_pass_sum.reason,
            Some(AvailabilityReason::CounterOverflow)
        );
        assert_eq!(snapshot.gpu_timing.groups[0].valid_pass_count, 2);
    }

    #[test]
    fn disabled_context_has_no_span_or_allocation() {
        let context = ObservationContext::new(CollectionMode::Off);
        assert!(!context.enabled());
        assert!(context.span().is_none());
    }

    #[test]
    fn gpu_timing_keeps_partial_sum_and_invalidates_full_sum() {
        let context = ObservationContext::new(CollectionMode::GpuTiming);
        assert!(context.span().is_some());
        context.record_batch(batch(
            Measurement::available(1.0),
            Measurement::available(1.0),
        ));
        context.record_batch(batch(
            Measurement::unavailable(AvailabilityReason::QueryFailed),
            Measurement::available(0.5),
        ));
        let snapshot = context.snapshot();
        assert_eq!(
            snapshot.gpu_timing.compute_pass_sum.reason,
            Some(AvailabilityReason::Incomplete)
        );
        assert_eq!(
            snapshot.gpu_timing.partial_compute_pass_sum.value,
            Some(1.5)
        );
    }

    #[test]
    fn observation_detail_is_bounded() {
        let context = ObservationContext::new(CollectionMode::Summary);
        for _ in 0..1100 {
            let _scope = context.scope("stage", ObservationKind::Stage, Purpose::Image);
        }
        let snapshot = context.snapshot();
        assert!(snapshot.coverage.retained_observations <= 1024);
        assert!(snapshot.coverage.truncated);
        assert!(snapshot.coverage.omitted_observations > 0);
        assert_eq!(
            snapshot.coverage.omitted_high_level_observations,
            Some(1100 - SUMMARY_LIMIT)
        );
    }
}

#[cfg(test)]
mod collector_edges {
    use super::*;

    #[test]
    fn omitted_descendants_keep_exact_ancestor_work() {
        let root = ObservationContext::new(CollectionMode::Summary);
        let stage = root.scope("scanning", ObservationKind::Stage, Purpose::Image);
        for _ in 0..OBSERVATION_LIMIT as u64 - SUMMARY_LIMIT {
            drop(
                stage
                    .context()
                    .scope("batch", ObservationKind::Batch, Purpose::Image),
            );
        }
        let omitted = stage
            .context()
            .scope("omitted", ObservationKind::Batch, Purpose::Image);
        assert!(!omitted.context().retained());
        let child = omitted
            .context()
            .scope("child", ObservationKind::Pass, Purpose::Image);
        assert!(!child.context().retained());
        child.context().record_work(&WorkCounters {
            dispatch_count: Some(7),
            ..WorkCounters::default()
        });
        let snapshot = root.snapshot();
        assert_eq!(snapshot.totals.dispatch_count, Some(7));
        assert_eq!(snapshot.observations[0].work.dispatch_count, Some(7));
        assert_eq!(snapshot.coverage.omitted_observations, 2);
        assert!(snapshot.coverage.retained_bytes <= snapshot.coverage.byte_limit);
    }

    #[test]
    fn batch_byte_pressure_cannot_consume_late_stage_storage() {
        let root = ObservationContext::new(CollectionMode::Summary);
        let phase = root.scope("simulation", ObservationKind::Phase, Purpose::Image);
        for _ in 0..600 {
            let batch = phase
                .context()
                .scope("compute", ObservationKind::Batch, Purpose::Image);
            batch.context().record_batch(BatchMeasurements {
                host_prepare: Measurement::available(0.0),
                submit_to_map_ready: Measurement::available(0.0),
                host_materialize: Measurement::available(0.0),
                completion_boundary: "界".repeat(256),
                compute_pass_sum: Measurement::unavailable(AvailabilityReason::NotCollected),
                partial_compute_pass_sum: Measurement::unavailable(
                    AvailabilityReason::NotCollected,
                ),
            });
        }
        let late = phase
            .context()
            .scope("grain_spektra", ObservationKind::Stage, Purpose::Image);
        late.context()
            .record_executor(Executor::Cpu, Some(CpuReason::SpektraGrainSampler));
        assert!(late.context().retained());
        drop(late);
        drop(phase);
        let snapshot = root.snapshot();
        assert!(snapshot.coverage.truncated);
        assert!(snapshot.coverage.omitted_observations > 0);
        assert_eq!(snapshot.coverage.omitted_high_level_observations, Some(0));
        assert!(snapshot.coverage.retained_bytes <= snapshot.coverage.byte_limit);
        let stage = snapshot
            .observations
            .iter()
            .find(|o| o.name == "grain_spektra")
            .unwrap();
        assert_eq!(stage.executor, Some(Executor::Cpu));
        assert_eq!(stage.cpu_reason, Some(CpuReason::SpektraGrainSampler));
    }

    #[test]
    fn unavailable_batches_preserve_root_partial_reasons() {
        for reason in [
            AvailabilityReason::QueryFailed,
            AvailabilityReason::NotReady,
            AvailabilityReason::Unsupported,
            AvailabilityReason::DetailLimit,
        ] {
            let root = ObservationContext::new(CollectionMode::GpuTiming);
            let batch = root.scope("compute", ObservationKind::Batch, Purpose::Image);
            batch.context().record_batch(BatchMeasurements {
                host_prepare: Measurement::available(0.0),
                submit_to_map_ready: Measurement::available(0.0),
                host_materialize: Measurement::available(0.0),
                completion_boundary: "image_map_ready".into(),
                compute_pass_sum: Measurement::unavailable(reason),
                partial_compute_pass_sum: Measurement::unavailable(reason),
            });
            drop(batch);
            assert_eq!(
                root.snapshot().gpu_timing.partial_compute_pass_sum,
                Measurement::unavailable(reason)
            );
        }
    }

    #[test]
    fn pass_completion_uses_device_measurement() {
        let root = ObservationContext::new(CollectionMode::GpuTiming);
        let pass = root.scope("pass", ObservationKind::Pass, Purpose::Image);
        pass.context().record_pass(1, Measurement::available(0.25));
        drop(pass);
        let snapshot = root.snapshot();
        assert_eq!(snapshot.observations[0].duration, 0.25);
        assert!(snapshot.observations[0].complete);
    }
}
