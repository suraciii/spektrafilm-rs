//! Vocabulary enums and bounded measurement data types for telemetry observations.
use serde::{Deserialize, Serialize};

macro_rules! vocabulary { ($name:ident { $($variant:ident),* $(,)? }) => { #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)] #[serde(rename_all="snake_case")] pub enum $name { $($variant),* } }; }
vocabulary!(CollectionMode {
    Off,
    Summary,
    GpuTiming
});
// The vocabulary macro shares derives across enums with no common default.
#[allow(clippy::derivable_impls)]
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

pub const PHASE_NAMES: &[&str] = &[
    "input_load",
    "backend_init",
    "input_prepare",
    "runtime_prepare",
    "scanner_setup_prepare",
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

// `Purpose::Telemetry` has no producer today. It is spec-reserved
// vocabulary: the enum mirrors the published observation schema, so the
// spelling is kept even while nothing emits it.
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
    DeviceResourceLimits,
    GrainV2Unsupported,
    GrainV3FieldCpu,
    FotufilmFilmCpu,
    NegativeToPositiveOutput,
    ScannerFilmPoints,
    ScannerReceiver,
});
// `ResidentDeclineReason::GrainV3FieldCpu` names the *routing* decline the
// resident raster records when the field's readout geometry (a crop or an
// upscale) does not fit it. It does not say where the field then resolves:
// `stages::grain_v3` publishes the effective field executor separately
// (`published_host_field_reason` / `effective_field_backend`), and on a device
// backend a cropped Dye request still reads its field out on the device.
vocabulary!(ResidentDeclineReason {
    WorkflowRoute,
    LangmuirChemistry,
    InputTransferDecoding,
    RequestedSpectralLut,
    ActiveOpticalDiffusion,
    FaithfulGrainDistribution,
    GrainV3FieldCpu,
    FotufilmFilmCpu,
    UnsupportedOutputGamut,
    BlurRadiusExceedsBackendSupport,
    MissingResidentFrontPass,
    BackendNoResidentSupport,
    DeviceResourceLimits,
    DiagnosticTapRoute,
    NegativeToPositiveOutput,
    ScannerFilmPoints,
    ScannerReceiver,
});
// `ReportPersistence` and `IssueBoundary::PresentationHandoff` likewise have
// no producer today; both are spec-reserved vocabulary retained so a future
// producer cannot introduce an unregistered wire spelling.
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
impl WorkCounters {
    pub fn named_fields(&self) -> impl Iterator<Item = (&'static str, &Option<u64>)> {
        [$( (stringify!($field), &self.$field), )*].into_iter()
    }

    pub fn named_value(&self, name: &str) -> Option<Option<u64>> {
        match name {
            $( stringify!($field) => Some(self.$field), )*
            _ => None,
        }
    }

    pub fn add(&mut self,other:&Self)->Vec<&'static str> {let mut overflow=Vec::new();$(let old=self.$field;self.$field=self.$field.zip(other.$field).and_then(|(a,b)|a.checked_add(b));if old.is_some()&&self.$field.is_none(){overflow.push(stringify!($field));})*overflow}
}
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
    /// None means historical coverage did not distinguish summaries from details.
    #[serde(default)]
    pub omitted_high_level_observations: Option<u64>,
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
            omitted_high_level_observations: Some(0),
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
    pub(super) fn add(&mut self, compute: &Measurement, partial: &Measurement) {
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
        self.partial_compute_pass_sum = match (self.partial_compute_pass_sum.status, partial.status)
        {
            (MeasurementStatus::Available, MeasurementStatus::Available) => Measurement::available(
                self.partial_compute_pass_sum.value.unwrap_or(0.0) + partial.value.unwrap_or(0.0),
            ),
            (MeasurementStatus::Available, _) => self.partial_compute_pass_sum.clone(),
            _ => partial.clone(),
        };
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
