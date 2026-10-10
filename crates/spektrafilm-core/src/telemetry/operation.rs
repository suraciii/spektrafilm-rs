use super::{
    configuration::{RenderConfiguration, SavingConfiguration},
    report::{
        BackendRequested, BuildIdentity, Configuration, Execution, Measurements, OperationIdentity,
        OperationKind, Precision, Report, ReportSummary, SourceRender, UnavailableField,
    },
};
use spektrafilm_gpu::telemetry::{
    AvailabilityReason, CollectionMode, IssueBoundary, IssueCategory, ObservationContext,
    ObservationKind, ObservationScope, Outcome, Purpose,
};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OPERATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct Operation {
    id: u64,
    kind: OperationKind,
    mode: CollectionMode,
    context: ObservationContext,
    enabled: bool,
    backend_requested: Option<BackendRequested>,
    input_dimensions: Option<[u32; 2]>,
    working_dimensions: Option<[u32; 2]>,
    published_dimensions: Option<[u32; 2]>,
    revisions: (Option<u64>, Option<u64>),
    render: Option<RenderConfiguration>,
    saving: Option<SavingConfiguration>,
    source_render: Option<SourceRender>,
    saved_attempt: Option<u64>,
}

impl Operation {
    pub fn new(kind: OperationKind, mode: CollectionMode) -> Self {
        let context = if mode == CollectionMode::Off {
            ObservationContext::default()
        } else {
            ObservationContext::new(mode)
        };
        let id = NEXT_OPERATION_ID.fetch_add(1, Ordering::Relaxed);
        if mode != CollectionMode::Off {
            tracing::debug!(target: "spektrafilm::telemetry", event = "operation_start", operation_id = id, observation_root_id = context.root_id(), kind = ?kind, collection_mode = ?mode);
        }
        Self {
            id,
            kind,
            mode,
            enabled: mode != CollectionMode::Off,
            context,
            backend_requested: None,
            input_dimensions: None,
            working_dimensions: None,
            published_dimensions: None,
            revisions: (None, None),
            render: None,
            saving: None,
            source_render: None,
            saved_attempt: None,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn mode(&self) -> CollectionMode {
        self.mode
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn context(&self) -> ObservationContext {
        self.context.clone()
    }

    pub fn scope(
        &self,
        name: &'static str,
        kind: ObservationKind,
        purpose: Purpose,
    ) -> ObservationScope {
        self.context.scope(name, kind, purpose)
    }

    pub fn attempt(&self, index: u64) -> ObservationScope {
        self.context.attempt(index)
    }

    pub fn set_backend_requested(&mut self, backend: BackendRequested) {
        self.backend_requested = Some(backend);
    }
    pub fn set_input_dimensions(&mut self, width: u32, height: u32) {
        self.input_dimensions = Some([width, height]);
    }
    pub fn set_working_dimensions(&mut self, width: u32, height: u32) {
        self.working_dimensions = Some([width, height]);
    }
    pub fn set_published_dimensions(&mut self, width: u32, height: u32) {
        self.published_dimensions = Some([width, height]);
    }
    pub fn set_revisions(&mut self, input: Option<u64>, parameters: Option<u64>) {
        self.revisions = (input, parameters);
    }
    pub fn set_render_configuration(&mut self, configuration: RenderConfiguration) {
        self.render = Some(configuration);
    }
    pub fn set_saving_configuration(&mut self, configuration: SavingConfiguration) {
        self.saving = Some(configuration);
    }
    pub fn set_source_render(&mut self, source: SourceRender) {
        self.source_render = Some(source);
    }
    pub fn set_saved_attempt(&mut self, id: u64) {
        self.saved_attempt = Some(id);
    }

    pub fn issue(&mut self, category: IssueCategory, boundary: IssueBoundary) {
        self.context.issue(category, boundary);
    }

    pub fn finish(self, outcome: Outcome) -> Option<Report> {
        if !self.enabled {
            return None;
        }
        self.context.finish(outcome);
        tracing::debug!(target: "spektrafilm::telemetry", event = "operation_terminal", operation_id = self.id, observation_root_id = self.context.root_id(), kind = ?self.kind, outcome = ?outcome);
        Some(self.report(outcome))
    }

    fn report(self, outcome: Outcome) -> Report {
        let mut snapshot = self.context.snapshot();
        merge_counter_overflow_fields(&mut snapshot);
        let mut attempts = Vec::new();
        let mut phases = Vec::new();
        let mut stages = Vec::new();
        let mut batches = Vec::new();
        for observation in snapshot.observations {
            match observation.kind {
                ObservationKind::Attempt => attempts.push(observation),
                ObservationKind::Phase => phases.push(observation),
                ObservationKind::Stage => stages.push(observation),
                ObservationKind::Batch | ObservationKind::Pass => batches.push(observation),
            }
        }
        let mut unavailable_fields = Vec::new();
        let requires_render = matches!(
            self.kind,
            OperationKind::Preview
                | OperationKind::Scan
                | OperationKind::Export
                | OperationKind::Process
                | OperationKind::Render
        );
        if (requires_render || self.kind == OperationKind::Save) && self.render.is_none() {
            unavailable_fields.push(UnavailableField {
                field: "render".to_owned(),
                reason: if self.kind == OperationKind::Save {
                    AvailabilityReason::SourceConfigurationNotRetained
                } else {
                    AvailabilityReason::NotReached
                },
            });
        }
        if requires_render && self.input_dimensions.is_none() {
            unavailable_fields.push(UnavailableField {
                field: "input_dimensions".to_owned(),
                reason: AvailabilityReason::NotReached,
            });
        }
        if self.kind == OperationKind::Save && self.saving.is_none() {
            unavailable_fields.push(UnavailableField {
                field: "saving".to_owned(),
                reason: AvailabilityReason::NotReached,
            });
        }
        let mut execution_unavailable = Vec::new();
        if self.backend_requested.is_none() {
            execution_unavailable.push(UnavailableField {
                field: "backend_requested".to_owned(),
                reason: if self.kind == OperationKind::Save {
                    AvailabilityReason::NotApplicable
                } else {
                    AvailabilityReason::NotReached
                },
            });
        }
        if snapshot.execution.backend_selected.is_none() {
            execution_unavailable.push(UnavailableField {
                field: "backend_selected".to_owned(),
                reason: if self.kind == OperationKind::Save {
                    AvailabilityReason::NotApplicable
                } else {
                    AvailabilityReason::NotReached
                },
            });
        }
        if snapshot.execution.path.is_none() {
            execution_unavailable.push(UnavailableField {
                field: "path".to_owned(),
                reason: if self.kind == OperationKind::Save {
                    AvailabilityReason::NotApplicable
                } else {
                    AvailabilityReason::NotReached
                },
            });
        }
        let gpu_work_observed = snapshot
            .totals
            .dispatch_count
            .is_some_and(|dispatches| dispatches > 0);
        if !gpu_work_observed {
            unavailable_fields.push(UnavailableField {
                field: "gpu_precision".to_owned(),
                reason: if snapshot.totals.dispatch_count.is_none() {
                    AvailabilityReason::CounterOverflow
                } else if self.kind == OperationKind::Save
                    || snapshot.execution.backend_selected
                        == Some(spektrafilm_gpu::telemetry::BackendSelected::Cpu)
                {
                    AvailabilityReason::NotApplicable
                } else {
                    AvailabilityReason::NotReached
                },
            });
        }
        let spectral_preparation_observed =
            self.kind != OperationKind::Save && self.render.is_some();
        if !spectral_preparation_observed {
            unavailable_fields.push(UnavailableField {
                field: "spectral_preparation_precision".to_owned(),
                reason: if self.kind == OperationKind::Save {
                    AvailabilityReason::NotApplicable
                } else {
                    AvailabilityReason::NotReached
                },
            });
        }
        let working_dimensions = self
            .working_dimensions
            .or(snapshot.execution.working_dimensions);
        if working_dimensions.is_none() {
            unavailable_fields.push(UnavailableField {
                field: "working_dimensions".to_owned(),
                reason: AvailabilityReason::NotReached,
            });
        }
        let saved_attempt_retained = self
            .saved_attempt
            .is_some_and(|attempt| attempts.iter().any(|observation| observation.id == attempt));
        let saved_attempt = self.saved_attempt.filter(|_| saved_attempt_retained);
        let mut diagnostic_issues = snapshot.issues;
        if self.saved_attempt.is_some() && !saved_attempt_retained {
            unavailable_fields.push(UnavailableField {
                field: "saved_attempt".to_owned(),
                reason: AvailabilityReason::DetailLimit,
            });
            if diagnostic_issues.len() < 64 {
                diagnostic_issues.push(spektrafilm_gpu::telemetry::DiagnosticIssue {
                    category: IssueCategory::CollectionUnavailable,
                    boundary: IssueBoundary::Operation,
                });
            } else {
                snapshot.coverage.omitted_issues =
                    snapshot.coverage.omitted_issues.saturating_add(1);
            }
        }
        let (version, version_truncated) = bounded_report_text(env!("CARGO_PKG_VERSION"));
        let (source_revision, source_revision_truncated) =
            option_env!("SPEKTRAFILM_SOURCE_REVISION")
                .map(bounded_report_text)
                .map(|(value, truncated)| (Some(value), truncated))
                .unwrap_or((None, false));
        let effective_mode = if self.mode == CollectionMode::GpuTiming {
            match snapshot.execution.adapter.as_ref() {
                Some(adapter) if adapter.timestamp_enabled => CollectionMode::GpuTiming,
                _ => CollectionMode::Summary,
            }
        } else {
            snapshot.execution.effective_mode.unwrap_or(self.mode)
        };
        Report {
            schema_version: 1,
            build: BuildIdentity {
                version,
                source_revision,
                version_truncated,
                source_revision_truncated,
            },
            operation: OperationIdentity {
                id: self.id,
                kind: self.kind,
                outcome,
                duration: Some(snapshot.duration),
                input_revision: self.revisions.0,
                parameter_revision: self.revisions.1,
                observation_root_id: self.context.root_id(),
            },
            source_render: self.source_render,
            configuration: Configuration {
                collection_mode_requested: self.mode,
                collection_mode_effective: effective_mode,
                input_dimensions: self.input_dimensions,
                working_dimensions,
                published_dimensions: self.published_dimensions,
                host_precision: Some(if cfg!(feature = "precision-f64") {
                    Precision::F64
                } else {
                    Precision::F32
                }),
                gpu_precision: if gpu_work_observed {
                    Some(Precision::F32)
                } else {
                    None
                },
                spectral_preparation_precision: if spectral_preparation_observed {
                    Some(Precision::F64)
                } else {
                    None
                },
                saved_attempt,
                render: self.render,
                saving: self.saving,
                unavailable_fields,
            },
            execution: Execution {
                backend_requested: self.backend_requested,
                backend_selected: snapshot.execution.backend_selected,
                path: snapshot.execution.path,
                resident_decline_reasons: snapshot.execution.resident_decline_reasons,
                adapter: snapshot.execution.adapter,
                unavailable_fields: execution_unavailable,
            },
            measurements: Measurements {
                work: snapshot.totals,
                gpu_timing: snapshot.gpu_timing,
            },
            attempts,
            phases,
            stages,
            gpu_batches: batches,
            diagnostic_issues,
            coverage: snapshot.coverage,
        }
    }
}

fn merge_counter_overflow_fields(snapshot: &mut spektrafilm_gpu::telemetry::ObservationSnapshot) {
    macro_rules! merge_fields {
        ($($field:ident),* $(,)?) => {{
            $(
                if snapshot.totals.$field.is_none()
                    && !snapshot
                        .coverage
                        .counter_overflow_fields
                        .iter()
                        .any(|field| field == stringify!($field))
                {
                    snapshot
                        .coverage
                        .counter_overflow_fields
                        .push(stringify!($field).to_owned());
                }
            )*
        }};
    }
    merge_fields!(
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
        omitted_pass_count,
    );
}

fn bounded_report_text(value: &str) -> (String, bool) {
    let mut text = String::new();
    let mut truncated = false;
    for (index, character) in value.chars().enumerate() {
        if index < 256 {
            text.push(character);
        } else {
            truncated = true;
            break;
        }
    }
    (text, truncated)
}

impl Report {
    pub fn summary(&self) -> ReportSummary {
        let largest_phase = self
            .coverage
            .phase_totals
            .iter()
            .filter(|phase| phase.total_duration.is_finite())
            .map(|phase| (phase.name.clone(), phase.total_duration))
            .max_by(|left, right| left.1.total_cmp(&right.1));
        let cpu_stage_reasons = self
            .stages
            .iter()
            .filter_map(|stage| {
                stage
                    .cpu_reason
                    .filter(|_| {
                        matches!(
                            stage.executor,
                            Some(spektrafilm_gpu::telemetry::Executor::Cpu)
                                | Some(spektrafilm_gpu::telemetry::Executor::Mixed)
                        )
                    })
                    .map(|reason| (stage.name.clone(), reason))
            })
            .collect();
        ReportSummary {
            operation_id: self.operation.id,
            kind: self.operation.kind,
            outcome: self.operation.outcome,
            working_dimensions: self.configuration.working_dimensions,
            duration_seconds: self.operation.duration,
            backend_requested: self.execution.backend_requested,
            backend_selected: self.execution.backend_selected,
            path: self.execution.path,
            largest_phase,
            resident_decline_reasons: self.execution.resident_decline_reasons.clone(),
            cpu_stage_reasons,
            software_adapter: self.execution.adapter.as_ref().is_some_and(|adapter| {
                adapter.device_type == spektrafilm_gpu::telemetry::AdapterDeviceType::Cpu
            }),
        }
    }
}

impl SourceRender {
    pub fn new(
        operation_id: u64,
        input_revision: Option<u64>,
        parameter_revision: Option<u64>,
        width: u32,
        height: u32,
        diagnostics_collected: bool,
        color_space: super::configuration::ColorSpaceRole,
        transfer_encoded: bool,
    ) -> Self {
        Self {
            operation_id,
            input_revision,
            parameter_revision,
            dimensions: [width, height],
            diagnostics_collected,
            color_space,
            transfer_encoded,
        }
    }
}
