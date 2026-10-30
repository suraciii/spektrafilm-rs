//! Vocabulary enums and bounded measurement data types for telemetry observations.
use serde::{Deserialize, Serialize};

macro_rules! vocabulary { ($(#[$enum_attr:meta])* $name:ident { $( $(#[$attr:meta])* $variant:ident),* $(,)? }) => { #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)] $(#[$enum_attr])* #[serde(rename_all="snake_case")] pub enum $name { $( $(#[$attr])* $variant),* } }; }
// These finite vocabularies are also the aggregate keys; shader labels never
// become report keys.
macro_rules! named_vocabulary {
    ($(#[$enum_attr:meta])* $name:ident { $($(#[$attr:meta])* $variant:ident => $wire:literal),+ $(,)? }) => {
        vocabulary!($(#[$enum_attr])* $name { $($(#[$attr])* $variant),+ });
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
        }
    };
}
named_vocabulary!(#[derive(Default)] GpuStage {
    #[default] Other => "other", SpectralPrepare => "spectral_prepare", GamutPrepare => "gamut_prepare",
    Input => "input", Metering => "metering", Geometry => "geometry", FilmExpose => "film_expose",
    Halation => "halation", FilmDevelop => "film_develop", Dir => "dir", GrainSpektra => "grain_spektra",
    GrainProcedural => "grain_procedural", DyeFormation => "dye_formation", DyeReadout => "dye_readout",
    FotufilmFormation => "fotufilm_formation", FotufilmStatistics => "fotufilm_statistics",
    FotufilmReadout => "fotufilm_readout", PrintExpose => "print_expose", PrintDevelop => "print_develop",
    Glare => "glare", Scan => "scan", NegativeToPositive => "negative_to_positive", Unsharp => "unsharp",
    Gamut => "gamut", Output => "output", Saving => "saving", Display => "display"
});
impl GpuStage {
    pub fn from_scope_name(name: &str) -> Self {
        match name {
            "spectral_prepare" | "enlarger_lut" => Self::SpectralPrepare,
            "gamut_prepare" | "input_gamut_prepare" => Self::GamutPrepare,
            "input"
            | "input_prepare"
            | "input_decode"
            | "convert_inverse_solver"
            | "front_transform"
            | "highlight_boost"
            | "camera_diffusion"
            | "camera_lens_blur" => Self::Input,
            "metering" => Self::Metering,
            "geometry" => Self::Geometry,
            "film_expose" | "filming_expose" | "log_exposure" => Self::FilmExpose,
            "halation" => Self::Halation,
            "film_develop" | "filming_develop" | "film_density" => Self::FilmDevelop,
            "dir" | "dir_couplers" => Self::Dir,
            "grain_spektra" | "grain_spektra_sampler" => Self::GrainSpektra,
            "grain_procedural" | "grain_procedural_cpu" => Self::GrainProcedural,
            "dye_formation" => Self::DyeFormation,
            "dye_readout" => Self::DyeReadout,
            "fotufilm_formation" => Self::FotufilmFormation,
            "fotufilm_statistics" => Self::FotufilmStatistics,
            "fotufilm_readout" => Self::FotufilmReadout,
            "print_expose" | "print_lut_roundtrip" | "print_spectral" | "enlarger_diffusion" => {
                Self::PrintExpose
            }
            "print_develop" | "print_density" => Self::PrintDevelop,
            "glare" => Self::Glare,
            "scan" | "scanning" | "scanner_density" | "scanner_lut" | "scan_spectral"
            | "scanner_lens_blur" => Self::Scan,
            "negative_to_positive" => Self::NegativeToPositive,
            "unsharp" | "unsharp_mask" => Self::Unsharp,
            "gamut" | "output_gamut" => Self::Gamut,
            "output" | "output_transfer" | "cmyk" | "finishing" => Self::Output,
            "saving" | "saving_convert" => Self::Saving,
            "display" | "display_prepare" => Self::Display,
            _ => Self::Other,
        }
    }
}
named_vocabulary!(GpuPassKind {
    Other => "other", Pointwise => "pointwise", Gather => "gather", Scatter => "scatter", Copy => "copy",
    Filter => "filter", Reduction => "reduction", PrefixSum => "prefix_sum", Spectral => "spectral",
    Formation => "formation", Readout => "readout"
});
named_vocabulary!(HostTimingKind {
    ResourceCreate => "resource_create", Upload => "upload", CommandEncoding => "command_encoding",
    QueueSubmit => "queue_submit", SubmissionWait => "submission_wait", ReadbackWait => "readback_wait"
});
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

/// Allowlisted host stage scope names used by observations and streamed totals.
pub const STAGE_NAMES: &[&str] = &[
    "metering",
    "geometry",
    "color_reference",
    "spectral_prepare",
    "film_chain",
    "filming_expose",
    "filming_develop",
    "converting",
    "grain_spektra",
    "grain_spektra_sampler",
    "grain_procedural",
    "grain_procedural_cpu",
    "grain_dye_field",
    "grain_fotufilm",
    "printing",
    "scanning",
    "post_scan",
    "cmyk",
    "output_mix",
    "negative_to_positive",
    "negative_reconstruction",
    "negative_to_positive_tone",
    "standard_setup",
    "input_transfer",
    "colorspace_convert",
    "output_transfer",
    "rgb_to_raw",
    "highlight_boost",
    "optical_diffusion",
    "scanner_lut",
    "enlarger_lut_prepare",
    "enlarger_lut_interpolate",
    "glare",
    "finishing",
    "output_gamut",
    "pipeline_create",
    "raw_sensor",
    "saving_convert",
    "retained_histogram",
    "viewer_resample",
    "viewer_display_transform",
    "viewer_full_source",
    "gamut_prepare",
    "input_gamut_prepare",
    "fotufilm_formation",
    "fotufilm_statistics",
    "fotufilm_readout",
    "input_decode",
    "convert_inverse_solver",
    "front_transform",
    "camera_diffusion",
    "camera_lens_blur",
    "halation",
    "log_exposure",
    "film_density",
    "dir_couplers",
    "grain_dye",
    "enlarger_lut",
    "print_lut_roundtrip",
    "print_spectral",
    "enlarger_diffusion",
    "print_density",
    "scanner_density",
    "scan_spectral",
    "scanner_lens_blur",
    "unsharp_mask",
];

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
    QueryCapacity,
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
    #[serde(alias = "grain_v1_sampler")]
    SpektraGrainSampler,
    UnsupportedOutputGamut,
    BlurRadiusExceedsBackendSupport,
    DeviceResourceLimits,
    #[serde(alias = "grain_v2_unsupported")]
    ProceduralGrainUnsupported,
    #[serde(alias = "grain_v3_field_cpu")]
    DyeGrainFieldCpu,
    FotufilmFilmCpu,
    NegativeToPositiveOutput,
    ScannerFilmPoints,
    ScannerReceiver,
});
// `ResidentDeclineReason::DyeGrainFieldCpu` names the *routing* decline the
// resident raster records when the field's readout geometry (a crop or an
// upscale) does not fit it. It does not say where the field then resolves:
// `stages::grain_dye` publishes the effective field executor separately
// (`published_host_field_reason` / `effective_field_backend`), and on a device
// backend a cropped Dye request still reads its field out on the device.
vocabulary!(ResidentDeclineReason {
    WorkflowRoute,
    LangmuirChemistry,
    InputTransferDecoding,
    RequestedSpectralLut,
    ActiveOpticalDiffusion,
    FaithfulGrainDistribution,
    #[serde(alias = "grain_v3_field_cpu")]
    DyeGrainFieldCpu,
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
// Historical telemetry names are input spellings only. This keeps saved
// schema-one reports readable while new reports use engine names.
fn deserialize_observation_name<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let name = String::deserialize(deserializer)?;
    Ok(match name.as_str() {
        "grain_v1" => "grain_spektra".into(),
        "grain_v1_sampler" => "grain_spektra_sampler".into(),
        "grain_v2" => "grain_procedural".into(),
        "grain_v2_cpu" => "grain_procedural_cpu".into(),
        "grain_v3_field" => "grain_dye_field".into(),
        _ => name,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub id: u64,
    pub parent_id: u64,
    #[serde(deserialize_with = "deserialize_observation_name")]
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
    #[serde(default)]
    pub host_timings: Option<Vec<HostTimingTotal>>,
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
    #[serde(default)]
    pub stage_totals: Vec<PhaseTotal>,
    /// Host interval totals were discarded after an arithmetic or input overflow.
    #[serde(default)]
    pub host_timing_overflow: bool,
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
            stage_totals: Vec::new(),
            host_timing_overflow: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostTimingTotal {
    pub kind: HostTimingKind,
    pub count: u64,
    pub total_duration: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpuTimingGroup {
    pub stage: GpuStage,
    pub kind: GpuPassKind,
    pub purpose: Purpose,
    pub executed_pass_count: u64,
    pub timed_pass_count: u64,
    pub valid_pass_count: u64,
    pub compute_pass_sum: Measurement,
    pub partial_compute_pass_sum: Measurement,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GpuTimingMeasurements {
    pub compute_pass_sum: Measurement,
    pub partial_compute_pass_sum: Measurement,
    #[serde(default)]
    pub groups: Vec<GpuTimingGroup>,
}
impl GpuTimingMeasurements {
    pub(super) fn add(&mut self, compute: &Measurement, partial: &Measurement) {
        if self.compute_pass_sum.reason == Some(AvailabilityReason::CounterOverflow)
            || self.partial_compute_pass_sum.reason == Some(AvailabilityReason::CounterOverflow)
        {
            return;
        }
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
        if [&self.compute_pass_sum, &self.partial_compute_pass_sum]
            .iter()
            .any(|m| {
                m.status == MeasurementStatus::Available
                    && m.value.is_none_or(|v| !v.is_finite() || v < 0.0)
            })
        {
            self.compute_pass_sum = Measurement::unavailable(AvailabilityReason::CounterOverflow);
            self.partial_compute_pass_sum =
                Measurement::unavailable(AvailabilityReason::CounterOverflow);
        }
    }
}
impl Default for GpuTimingMeasurements {
    fn default() -> Self {
        Self {
            compute_pass_sum: Measurement::unavailable(AvailabilityReason::NotCollected),
            partial_compute_pass_sum: Measurement::unavailable(AvailabilityReason::NotCollected),
            groups: Vec::new(),
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
    #[serde(default)]
    pub host_timings: Option<Vec<HostTimingTotal>>,
    pub coverage: Coverage,
    pub execution: ExecutionFacts,
    pub issues: Vec<DiagnosticIssue>,
}
