use super::configuration::{RenderConfiguration, SavingConfiguration};
use serde::{Deserialize, Serialize};
use spektrafilm_gpu::telemetry::{AvailabilityReason, CollectionMode, Outcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Preview,
    Scan,
    Save,
    Export,
    Process,
    Render,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendRequested {
    Auto,
    Cpu,
    Wgpu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnavailableField {
    pub field: String,
    pub reason: AvailabilityReason,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildIdentity {
    pub version: String,
    pub source_revision: Option<String>,
    pub version_truncated: bool,
    pub source_revision_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationIdentity {
    pub id: u64,
    pub kind: OperationKind,
    pub outcome: Outcome,
    pub duration: Option<f64>,
    pub input_revision: Option<u64>,
    pub parameter_revision: Option<u64>,
    pub observation_root_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRender {
    pub operation_id: u64,
    pub input_revision: Option<u64>,
    pub parameter_revision: Option<u64>,
    pub dimensions: [u32; 2],
    pub diagnostics_collected: bool,
    pub color_space: super::configuration::ColorSpaceRole,
    pub transfer_encoded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    F32,
    F64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Configuration {
    pub collection_mode_requested: CollectionMode,
    pub collection_mode_effective: CollectionMode,
    pub input_dimensions: Option<[u32; 2]>,
    pub working_dimensions: Option<[u32; 2]>,
    pub published_dimensions: Option<[u32; 2]>,
    pub host_precision: Option<Precision>,
    pub gpu_precision: Option<Precision>,
    pub spectral_preparation_precision: Option<Precision>,
    pub saved_attempt: Option<u64>,
    pub render: Option<RenderConfiguration>,
    pub saving: Option<SavingConfiguration>,
    pub unavailable_fields: Vec<UnavailableField>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Execution {
    pub backend_requested: Option<BackendRequested>,
    pub backend_selected: Option<spektrafilm_gpu::telemetry::BackendSelected>,
    pub path: Option<spektrafilm_gpu::telemetry::ExecutionPath>,
    pub resident_decline_reasons: Vec<spektrafilm_gpu::telemetry::ResidentDeclineReason>,
    pub adapter: Option<spektrafilm_gpu::telemetry::AdapterDescription>,
    pub unavailable_fields: Vec<UnavailableField>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurements {
    pub work: spektrafilm_gpu::telemetry::WorkCounters,
    pub gpu_timing: spektrafilm_gpu::telemetry::GpuTimingMeasurements,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub build: BuildIdentity,
    pub operation: OperationIdentity,
    pub source_render: Option<SourceRender>,
    pub configuration: Configuration,
    pub execution: Execution,
    pub measurements: Measurements,
    pub attempts: Vec<spektrafilm_gpu::telemetry::Observation>,
    pub phases: Vec<spektrafilm_gpu::telemetry::Observation>,
    pub stages: Vec<spektrafilm_gpu::telemetry::Observation>,
    pub gpu_batches: Vec<spektrafilm_gpu::telemetry::Observation>,
    pub diagnostic_issues: Vec<spektrafilm_gpu::telemetry::DiagnosticIssue>,
    pub coverage: spektrafilm_gpu::telemetry::Coverage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportSummary {
    pub operation_id: u64,
    pub kind: OperationKind,
    pub outcome: Outcome,
    pub working_dimensions: Option<[u32; 2]>,
    pub duration_seconds: Option<f64>,
    pub backend_requested: Option<BackendRequested>,
    pub backend_selected: Option<spektrafilm_gpu::telemetry::BackendSelected>,
    pub path: Option<spektrafilm_gpu::telemetry::ExecutionPath>,
    pub largest_phase: Option<(String, f64)>,
    pub resident_decline_reasons: Vec<spektrafilm_gpu::telemetry::ResidentDeclineReason>,
    pub cpu_stage_reasons: Vec<(String, spektrafilm_gpu::telemetry::CpuReason)>,
    pub software_adapter: bool,
}
