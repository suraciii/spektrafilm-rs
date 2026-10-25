mod configuration;
mod operation;
mod persistence;
mod progress;
mod report;
mod validator;

pub use configuration::{
    BitDepth, ColorSpaceRole, Compression, GamutAlgorithm, GrainConfiguration, GrainEngine,
    GrainMode, OutputFormat, RenderConfiguration, RgbToRawAlgorithm, SavingConfiguration,
    ScanModelConfiguration, SpatialConfiguration, StockId, WorkflowRoute,
};
pub use operation::Operation;
pub use persistence::{
    PersistenceError, ReportDestination, paths_alias, validate_report_destination,
};
pub use progress::{ProgressJournal, ProgressReceiver};
pub use report::{
    BackendRequested, BuildIdentity, Configuration, Execution, Measurements, OperationIdentity,
    OperationKind, Precision, Report, ReportSummary, SourceRender, UnavailableField,
};
pub use spektrafilm_backend::telemetry::{
    AdapterDescription, AdapterDeviceType, AvailabilityReason, BackendSelected, BatchMeasurements,
    CollectionMode, Coverage, CpuReason, DiagnosticIssue, ExecutionFacts, ExecutionPath, Executor,
    GpuTimingMeasurements, IssueBoundary, IssueCategory, Measurement, MeasurementStatus,
    Observation, ObservationContext, ObservationKind, ObservationScope, ObservationSnapshot,
    Outcome, PassMeasurement, PhaseTotal, Purpose, ResidentDeclineReason, UploadCategory,
    WorkCounters,
};
pub use validator::ValidationError;
