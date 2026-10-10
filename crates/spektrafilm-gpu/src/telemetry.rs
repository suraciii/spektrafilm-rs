//! Operation-local, bounded observations shared by simulation and compute backends.
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

macro_rules! vocabulary { ($name:ident { $($variant:ident),* $(,)? }) => { #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all="snake_case")] pub enum $name { $($variant),* } }; }
vocabulary!(CollectionMode {
    Off,
    Summary,
    GpuTiming
});
impl Default for CollectionMode {
    fn default() -> Self {
        Self::Off
    }
}
vocabulary!(ObservationKind {
    Attempt,
    Phase,
    Stage,
    Batch,
    Pass
});
vocabulary!(Purpose {
    Image,
    Calibration,
    Telemetry
});
vocabulary!(Outcome {
    Succeeded,
    Failed,
    Cancelled,
    Superseded
});
vocabulary!(AvailabilityReason {
    NotCollected,
    NotReached,
    NotApplicable,
    Unsupported,
    FeatureRequestFailed,
    NotEnabledOnDevice,
    QueryFailed,
    NotReady,
    InvalidTimestamp,
    DetailLimit,
    Incomplete,
    CounterOverflow,
    SourceConfigurationNotRetained
});
vocabulary!(BackendSelected { Cpu, Wgpu });
vocabulary!(ExecutionPath {
    Cpu,
    GpuResident,
    PerStage,
    Mixed
});
vocabulary!(Executor { Cpu, Gpu, Mixed });
vocabulary!(CpuReason {
    CpuSelected,
    BackendDefault,
    Unsupported,
    InputTransferDecoding,
    OpticalDiffusion,
    Mallett,
    SpectralLut,
    FaithfulGrainDistribution,
    GrainV1Sampler,
    UnsupportedOutputGamut,
    BlurRadiusExceedsBackendSupport,
    GrainV2Unsupported
});
vocabulary!(ResidentDeclineReason {
    WorkflowRoute,
    LangmuirChemistry,
    InputTransferDecoding,
    RequestedSpectralLut,
    ActiveOpticalDiffusion,
    FaithfulGrainDistribution,
    UnsupportedOutputGamut,
    BlurRadiusExceedsBackendSupport,
    MissingResidentFrontPass,
    MallettExecutionParity,
    BackendNoResidentSupport,
    DiagnosticTapRoute
});
vocabulary!(IssueCategory {
    Input,
    Configuration,
    Backend,
    Simulation,
    Writing,
    Publication,
    Cancelled,
    MetadataWarning,
    CollectionUnavailable,
    ReportPersistence
});
vocabulary!(IssueBoundary {
    InputLoad,
    BackendInit,
    InputPrepare,
    RuntimePrepare,
    Simulation,
    DisplayPrepare,
    PresentationHandoff,
    SavingConvert,
    FileWrite,
    Metadata,
    Publication,
    ResultPrepare,
    Operation
});
vocabulary!(AdapterDeviceType {
    Cpu,
    Integrated,
    Discrete,
    Virtual,
    Other
});
vocabulary!(UploadCategory {
    Image,
    SpectralLut,
    Uniform,
    Scratch
});

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub status: MeasurementStatus,
    pub value: Option<f64>,
    pub reason: Option<AvailabilityReason>,
}
vocabulary!(MeasurementStatus {
    Available,
    Unavailable,
    NotApplicable
});
impl Measurement {
    pub fn available(value: f64) -> Self {
        Self {
            status: MeasurementStatus::Available,
            value: Some(value),
            reason: None,
        }
    }
    pub fn unavailable(reason: AvailabilityReason) -> Self {
        Self {
            status: MeasurementStatus::Unavailable,
            value: None,
            reason: Some(reason),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdapterDescription {
    pub api: String,
    pub device_type: AdapterDeviceType,
    pub name: String,
    pub description_truncated: bool,
    pub shared_device: bool,
    pub timestamp_supported: bool,
    pub timestamp_enabled: bool,
    pub timestamp_period_ns: Measurement,
    pub timestamp_reason: Option<AvailabilityReason>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionFacts {
    pub backend_selected: Option<BackendSelected>,
    pub path: Option<ExecutionPath>,
    pub effective_mode: Option<CollectionMode>,
    pub resident_decline_reasons: Vec<ResidentDeclineReason>,
    pub adapter: Option<AdapterDescription>,
    pub working_dimensions: Option<[u32; 2]>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticIssue {
    pub category: IssueCategory,
    pub boundary: IssueBoundary,
}

macro_rules! counters { ($($field:ident),* $(,)?) => {
#[derive(Clone,Debug,PartialEq,Serialize,Deserialize)] pub struct WorkCounters { $(pub $field:Option<u64>,)* }
impl Default for WorkCounters { fn default()->Self {Self{$($field:Some(0),)*}} }
impl WorkCounters { pub fn add(&mut self,other:&Self)->Vec<&'static str> {let mut overflow=Vec::new();$(let old=self.$field;self.$field=self.$field.zip(other.$field).and_then(|(a,b)|a.checked_add(b));if old.is_some()&&self.$field.is_none(){overflow.push(stringify!($field));})*overflow} }
}; }
counters!(
    dispatch_count,
    submit_count,
    command_buffer_count,
    host_wait_count,
    upload_count,
    upload_bytes,
    image_upload_count,
    image_upload_bytes,
    spectral_lut_upload_count,
    spectral_lut_upload_bytes,
    uniform_upload_count,
    uniform_upload_bytes,
    scratch_upload_count,
    scratch_upload_bytes,
    readback_count,
    readback_bytes,
    staging_copy_count,
    staging_copy_bytes,
    device_copy_count,
    device_copy_bytes,
    buffer_create_count,
    buffer_create_bytes,
    pipeline_cache_hit_count,
    pipeline_cache_miss_count,
    telemetry_buffer_create_count,
    telemetry_buffer_create_bytes,
    telemetry_resolve_count,
    telemetry_copy_count,
    telemetry_copy_bytes,
    executed_pass_count,
    timed_pass_count,
    valid_pass_count,
    omitted_pass_count
);
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchMeasurements {
    pub host_prepare: Measurement,
    pub submit_to_map_ready: Measurement,
    pub host_materialize: Measurement,
    pub completion_boundary: String,
    pub compute_pass_sum: Measurement,
    pub partial_compute_pass_sum: Measurement,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassMeasurement {
    pub occurrence: u64,
    pub duration: Measurement,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub id: u64,
    pub parent_id: u64,
    pub name: String,
    pub kind: ObservationKind,
    pub purpose: Purpose,
    pub start_offset: f64,
    pub duration: f64,
    pub complete: bool,
    pub outcome: Option<Outcome>,
    pub attempt_index: Option<u64>,
    pub executor: Option<Executor>,
    pub cpu_reason: Option<CpuReason>,
    pub work: WorkCounters,
    pub batch: Option<BatchMeasurements>,
    pub pass: Option<PassMeasurement>,
    pub execution: Option<ExecutionFacts>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhaseTotal {
    pub name: String,
    pub total_duration: f64,
    pub count: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Coverage {
    pub detail_limit: u64,
    pub byte_limit: u64,
    pub retained_observations: u64,
    pub omitted_observations: u64,
    pub truncated: bool,
    pub retained_bytes: u64,
    pub omitted_issues: u64,
    pub counter_overflow_fields: Vec<String>,
    pub fixed_duration_totals: [f64; 5],
    pub phase_totals: Vec<PhaseTotal>,
}
impl Default for Coverage {
    fn default() -> Self {
        Self {
            detail_limit: 1024,
            byte_limit: 1048576,
            retained_observations: 0,
            omitted_observations: 0,
            truncated: false,
            retained_bytes: 0,
            omitted_issues: 0,
            counter_overflow_fields: Vec::new(),
            fixed_duration_totals: [0.0; 5],
            phase_totals: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpuTimingMeasurements {
    pub compute_pass_sum: Measurement,
    pub partial_compute_pass_sum: Measurement,
}
impl GpuTimingMeasurements {
    fn add(&mut self, compute: &Measurement, partial: &Measurement) {
        let incomplete = Measurement::unavailable(AvailabilityReason::Incomplete);
        let compute = if compute.status != MeasurementStatus::Available
            && partial.status == MeasurementStatus::Available
        {
            &incomplete
        } else {
            compute
        };
        match (self.compute_pass_sum.status, compute.status) {
            (MeasurementStatus::Available, MeasurementStatus::Available) => {
                self.compute_pass_sum = Measurement::available(
                    self.compute_pass_sum.value.unwrap_or(0.0) + compute.value.unwrap_or(0.0),
                );
            }
            (MeasurementStatus::Available, _) => {
                self.compute_pass_sum = Measurement::unavailable(AvailabilityReason::Incomplete);
            }
            (MeasurementStatus::Unavailable, _)
                if self.compute_pass_sum.reason == Some(AvailabilityReason::Incomplete) => {}
            (MeasurementStatus::Unavailable, MeasurementStatus::Available)
                if self.compute_pass_sum.reason != Some(AvailabilityReason::NotCollected) =>
            {
                self.compute_pass_sum = Measurement::unavailable(AvailabilityReason::Incomplete);
            }
            (_, MeasurementStatus::Available) => self.compute_pass_sum = compute.clone(),
            (_, MeasurementStatus::Unavailable) => self.compute_pass_sum = compute.clone(),
            _ => {}
        }
        if partial.status == MeasurementStatus::Available {
            self.partial_compute_pass_sum = match self.partial_compute_pass_sum.status {
                MeasurementStatus::Available => Measurement::available(
                    self.partial_compute_pass_sum.value.unwrap_or(0.0)
                        + partial.value.unwrap_or(0.0),
                ),
                _ => partial.clone(),
            };
        }
    }
}
impl Default for GpuTimingMeasurements {
    fn default() -> Self {
        Self {
            compute_pass_sum: Measurement::unavailable(AvailabilityReason::NotCollected),
            partial_compute_pass_sum: Measurement::unavailable(AvailabilityReason::NotCollected),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservationSnapshot {
    pub root_id: u64,
    pub duration: f64,
    pub outcome: Option<Outcome>,
    pub observations: Vec<Observation>,
    pub totals: WorkCounters,
    pub gpu_timing: GpuTimingMeasurements,
    pub coverage: Coverage,
    pub execution: ExecutionFacts,
    pub issues: Vec<DiagnosticIssue>,
}
struct Collector {
    start: Instant,
    mode: CollectionMode,
    next: u64,
    snapshot: ObservationSnapshot,
}
// Root facts, bounded issues, fixed phase totals and overflow field names reserve
// enough space independently of detailed observation admission.
const ROOT_BYTES: u64 = 16 * 1024;

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
    meta: Option<ScopeMeta>,
    span: Option<tracing::Span>,
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
                snapshot: ObservationSnapshot {
                    root_id,
                    duration: 0.0,
                    outcome: None,
                    observations: Vec::with_capacity(1024),
                    totals: WorkCounters::default(),
                    gpu_timing: GpuTimingMeasurements::default(),
                    coverage: Coverage {
                        retained_bytes: ROOT_BYTES
                            + (1024 * std::mem::size_of::<Observation>()) as u64,
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
            meta: Some(ScopeMeta {
                name: "operation",
                kind: ObservationKind::Attempt,
            }),
            span: Some(
                tracing::debug_span!(target:"spektrafilm::telemetry", parent: None, "operation", observation_root_id=root_id),
            ),
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
    pub fn purpose(&self) -> Purpose {
        self.purpose.unwrap_or(Purpose::Image)
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
        let id = c.next;
        c.next = c.next.saturating_add(1);
        let bytes = name.len() as u64;
        let retain = self.retained
            && c.snapshot.coverage.retained_observations < 1024
            && c.snapshot.coverage.retained_bytes + bytes <= 1048576;
        let start = Instant::now();
        let offset = c.start.elapsed().as_secs_f64();
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
                execution: if kind == ObservationKind::Attempt {
                    Some(ExecutionFacts::default())
                } else {
                    None
                },
            });
            c.snapshot.coverage.retained_observations += 1;
            c.snapshot.coverage.retained_bytes += bytes;
        } else {
            c.snapshot.coverage.omitted_observations =
                c.snapshot.coverage.omitted_observations.saturating_add(1);
            c.snapshot.coverage.truncated = true;
        }
        drop(c);
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
                meta: Some(ScopeMeta { name, kind }),
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
                (1024 - c.snapshot.coverage.retained_observations) as u32
            }
        })
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
            c.snapshot.duration = c.start.elapsed().as_secs_f64();
            c.snapshot.outcome = Some(outcome);
            tracing::debug!(target:"spektrafilm::telemetry",parent:self.span.as_ref().and_then(|s|s.id()),event="operation_finish",observation_root_id=self.root,outcome=?outcome,duration=c.snapshot.duration);
        }
    }
    fn mutate(&self, f: impl FnOnce(&mut ObservationSnapshot)) {
        if let Some(c) = &self.collector {
            let _entered = self.span.as_ref().map(|span| span.enter());
            f(&mut c.lock().snapshot)
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
    pub fn record_batch(&self, mut batch: BatchMeasurements) {
        if !self.enabled() {
            return;
        }
        let _entered = self.span.as_ref().map(|span| span.enter());
        // Completion boundaries are backend vocabulary, never free-form diagnostics.
        bounded_description(&mut batch.completion_boundary);
        tracing::debug!(target:"spektrafilm::telemetry",parent:self.span.as_ref().and_then(|s|s.id()),event="batch",observation_root_id=self.root,observation_id=self.id,host_prepare_status=?batch.host_prepare.status,host_prepare=batch.host_prepare.value,submit_to_map_ready_status=?batch.submit_to_map_ready.status,submit_to_map_ready=batch.submit_to_map_ready.value,host_materialize_status=?batch.host_materialize.status,host_materialize=batch.host_materialize.value,compute_pass_sum_status=?batch.compute_pass_sum.status,compute_pass_sum=batch.compute_pass_sum.value,partial_compute_pass_sum_status=?batch.partial_compute_pass_sum.status,partial_compute_pass_sum=batch.partial_compute_pass_sum.value,completion_boundary=batch.completion_boundary.as_str());
        self.mutate(|s| {
            s.gpu_timing
                .add(&batch.compute_pass_sum, &batch.partial_compute_pass_sum);
            if let Some(o) = s.observations.iter_mut().find(|o| o.id == self.id) {
                let old_bytes = observation_bytes(o);
                let added = batch.completion_boundary.capacity() as u64;
                let removed = o
                    .batch
                    .as_ref()
                    .map_or(0, |b| b.completion_boundary.capacity() as u64);
                if s.coverage.retained_bytes + added - removed <= s.coverage.byte_limit {
                    o.batch = Some(batch);
                    s.coverage.retained_bytes =
                        s.coverage.retained_bytes - old_bytes + observation_bytes(o);
                } else {
                    s.coverage.truncated = true;
                }
            }
        });
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
            if let Some(meta) = meta {
                if meta.kind == ObservationKind::Phase {
                    const NAMES: &[&str] = &[
                        "input_load",
                        "backend_init",
                        "input_prepare",
                        "runtime_prepare",
                        "simulation",
                        "display_prepare",
                        "presentation_handoff",
                        "saving_convert",
                        "file_write",
                        "metadata",
                        "publication",
                        "result_prepare",
                        "worker_wait",
                    ];
                    if let Some(i) = NAMES.iter().position(|&n| n == meta.name) {
                        while s.coverage.phase_totals.len() <= i {
                            s.coverage.phase_totals.push(PhaseTotal {
                                name: NAMES[s.coverage.phase_totals.len()].into(),
                                total_duration: 0.0,
                                count: 0,
                            });
                        }
                        let t = &mut s.coverage.phase_totals[i];
                        t.total_duration += duration;
                        t.count = t.count.saturating_add(1);
                    }
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
    }
}

#[cfg(test)]
mod collector_edges {
    use super::*;

    #[test]
    fn omitted_descendants_keep_exact_ancestor_work() {
        let root = ObservationContext::new(CollectionMode::Summary);
        let stage = root.scope("scanning", ObservationKind::Stage, Purpose::Image);
        for _ in 1..1024 {
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
    fn incomplete_gpu_sum_is_sticky_with_valid_subset() {
        let mut timing = GpuTimingMeasurements::default();
        timing.add(
            &Measurement::unavailable(AvailabilityReason::QueryFailed),
            &Measurement::available(0.5),
        );
        timing.add(&Measurement::available(1.0), &Measurement::available(1.0));
        assert_eq!(
            timing.compute_pass_sum.reason,
            Some(AvailabilityReason::Incomplete)
        );
        assert_eq!(timing.partial_compute_pass_sum.value, Some(1.5));
    }

    #[test]
    fn cpu_gpu_timing_preserves_unsupported() {
        let root = ObservationContext::new(CollectionMode::GpuTiming);
        root.set_backend_selected(BackendSelected::Cpu);
        assert_eq!(
            root.snapshot().gpu_timing.compute_pass_sum.reason,
            Some(AvailabilityReason::Unsupported)
        );
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
