mod configuration;
mod operation;
mod persistence;
mod report;
mod validator;

pub use configuration::{
    BitDepth, ColorSpaceRole, Compression, GamutAlgorithm, GrainConfiguration, GrainEngine,
    GrainMode, OutputFormat, RenderConfiguration, RgbToRawAlgorithm, SavingConfiguration,
    SpatialConfiguration, StockId, WorkflowRoute,
};
pub use operation::Operation;
pub use persistence::{PersistenceError, ReportDestination, validate_report_destination};
pub use report::{
    BackendRequested, BuildIdentity, Configuration, Execution, Measurements, OperationIdentity,
    OperationKind, Precision, Report, ReportSummary, SourceRender, UnavailableField,
};
pub use spektrafilm_gpu::telemetry::{
    AdapterDescription, AdapterDeviceType, AvailabilityReason, BackendSelected, BatchMeasurements,
    CollectionMode, Coverage, CpuReason, DiagnosticIssue, ExecutionFacts, ExecutionPath, Executor,
    GpuTimingMeasurements, IssueBoundary, IssueCategory, Measurement, MeasurementStatus,
    Observation, ObservationContext, ObservationKind, ObservationScope, ObservationSnapshot,
    Outcome, PassMeasurement, PhaseTotal, Purpose, ResidentDeclineReason, UploadCategory,
    WorkCounters,
};
pub use validator::ValidationError;
