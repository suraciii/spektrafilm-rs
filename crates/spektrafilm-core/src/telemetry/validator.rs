use super::report::{OperationKind, Report};
use spektrafilm_gpu::telemetry::{MeasurementStatus, Observation, ObservationKind};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

const PHASE_NAMES: &[&str] = &[
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

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("unsupported schema version {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("source_render is present only for save operations")]
    UnexpectedSourceRender,
    #[error("save operation requires source_render")]
    MissingSourceRender,
    #[error("non-finite numeric measurement in {0}")]
    NonFinite(&'static str),
    #[error("duplicate observation identifier {0}")]
    DuplicateObservationId(u64),
    #[error("observation {0} has an unretained parent")]
    MissingParent(u64),
    #[error("observation {0} is outside its completed parent scope")]
    OutsideParentScope(u64),
    #[error("measurement status and value are inconsistent")]
    InvalidMeasurement,
    #[error("saved attempt is not retained")]
    MissingSavedAttempt,
    #[error("bounded text field exceeds 256 characters")]
    TextLimitExceeded,
    #[error("issue detail exceeds the retention limit")]
    IssueLimitExceeded,
    #[error("coverage totals are inconsistent")]
    InvalidCoverage,
    #[error("observation {0} has an invalid parent kind")]
    InvalidParentKind(u64),
    #[error("observation ancestry contains a cycle")]
    ObservationCycle,
    #[error("observation identifier collides with the operation root")]
    RootIdCollision,
    #[error("attempts are not applicable to this operation")]
    UnexpectedAttempts,
    #[error("report field is outside the privacy allowlist")]
    PrivacyField,
    #[error("JSON report error: {0}")]
    Json(String),
}

impl Report {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_report(self)
    }

    pub fn to_json(&self) -> Result<String, ValidationError> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|error| ValidationError::Json(error.to_string()))
    }

    pub fn from_json(json: &str) -> Result<Self, ValidationError> {
        let report: Self =
            serde_json::from_str(json).map_err(|error| ValidationError::Json(error.to_string()))?;
        report.validate()?;
        Ok(report)
    }
}

fn validate_report(report: &Report) -> Result<(), ValidationError> {
    if report.schema_version != 1 {
        return Err(ValidationError::UnsupportedSchemaVersion(
            report.schema_version,
        ));
    }
    validate_text(&report.build.version)?;
    if let Some(revision) = &report.build.source_revision {
        validate_text(revision)?;
    }
    validate_optional_finite(report.operation.duration, "operation.duration")?;
    if report
        .operation
        .duration
        .is_some_and(|duration| duration < 0.0)
    {
        return Err(ValidationError::InvalidMeasurement);
    }
    if (report.operation.kind == OperationKind::Save) != report.source_render.is_some() {
        return if report.source_render.is_some() {
            Err(ValidationError::UnexpectedSourceRender)
        } else {
            Err(ValidationError::MissingSourceRender)
        };
    }
    if report.operation.kind == OperationKind::Save && !report.attempts.is_empty() {
        return Err(ValidationError::UnexpectedAttempts);
    }
    if let Some(source) = &report.source_render {
        if source.operation_id == 0 || source.dimensions.contains(&0) {
            return Err(ValidationError::NonFinite("source_render"));
        }
    }
    validate_unavailable(&report.configuration.unavailable_fields)?;
    validate_unavailable(&report.execution.unavailable_fields)?;
    if let Some(render) = &report.configuration.render {
        for stock in [&render.film_stock, &render.print_stock]
            .into_iter()
            .flatten()
        {
            if stock.as_str() != "custom" && !super::configuration::is_bundled_stock(stock.as_str())
            {
                return Err(ValidationError::PrivacyField);
            }
        }
    }
    validate_execution_parts(
        report.execution.adapter.as_ref(),
        report.configuration.working_dimensions,
    )?;
    validate_measurement(&report.measurements.gpu_timing.compute_pass_sum)?;
    validate_measurement(&report.measurements.gpu_timing.partial_compute_pass_sum)?;
    if report.diagnostic_issues.len() > 64 {
        return Err(ValidationError::IssueLimitExceeded);
    }

    let observations: Vec<&Observation> = report
        .attempts
        .iter()
        .chain(&report.phases)
        .chain(&report.stages)
        .chain(&report.gpu_batches)
        .collect();
    let mut ids = HashMap::new();
    for observation in &observations {
        if observation.id == report.operation.observation_root_id {
            return Err(ValidationError::RootIdCollision);
        }
        if ids.insert(observation.id, *observation).is_some() {
            return Err(ValidationError::DuplicateObservationId(observation.id));
        }
        validate_observation_name(observation)?;
        validate_finite(observation.start_offset, "observation.start_offset")?;
        validate_finite(observation.duration, "observation.duration")?;
        if observation.start_offset < 0.0 || observation.duration < 0.0 {
            return Err(ValidationError::NonFinite("observation.duration"));
        }
        if let Some(execution) = &observation.execution {
            if observation.kind != ObservationKind::Attempt {
                return Err(ValidationError::InvalidCoverage);
            }
            validate_execution_parts(execution.adapter.as_ref(), execution.working_dimensions)?;
        }
        if observation.kind != ObservationKind::Attempt && observation.attempt_index.is_some() {
            return Err(ValidationError::InvalidCoverage);
        }
        if observation.batch.is_some()
            && !matches!(
                observation.kind,
                ObservationKind::Batch | ObservationKind::Pass
            )
        {
            return Err(ValidationError::InvalidCoverage);
        }
        if let Some(batch) = &observation.batch {
            validate_measurement(&batch.host_prepare)?;
            validate_measurement(&batch.submit_to_map_ready)?;
            validate_measurement(&batch.host_materialize)?;
            validate_measurement(&batch.compute_pass_sum)?;
            validate_measurement(&batch.partial_compute_pass_sum)?;
            validate_text(&batch.completion_boundary)?;
            if !stable_identifier(&batch.completion_boundary) {
                return Err(ValidationError::PrivacyField);
            }
        }
        if observation.pass.is_some() && observation.kind != ObservationKind::Pass {
            return Err(ValidationError::InvalidCoverage);
        }
        if let Some(pass) = &observation.pass {
            validate_measurement(&pass.duration)?;
        }
    }
    for observation in &observations {
        if observation.parent_id == report.operation.observation_root_id {
            if !matches!(
                observation.kind,
                ObservationKind::Attempt | ObservationKind::Phase
            ) {
                return Err(ValidationError::InvalidParentKind(observation.id));
            }
            if observation.complete
                && report.operation.duration.is_some_and(|duration| {
                    observation.start_offset + observation.duration > duration + 1e-9
                })
            {
                return Err(ValidationError::OutsideParentScope(observation.id));
            }
            continue;
        }
        let Some(parent) = ids.get(&observation.parent_id) else {
            return Err(ValidationError::MissingParent(observation.id));
        };
        let valid_parent_kind = match observation.kind {
            ObservationKind::Attempt => false,
            ObservationKind::Phase => matches!(
                parent.kind,
                ObservationKind::Attempt | ObservationKind::Phase
            ),
            ObservationKind::Stage => {
                matches!(parent.kind, ObservationKind::Phase | ObservationKind::Stage)
            }
            ObservationKind::Batch => {
                matches!(parent.kind, ObservationKind::Phase | ObservationKind::Stage)
            }
            ObservationKind::Pass => parent.kind == ObservationKind::Batch,
        };
        if !valid_parent_kind {
            return Err(ValidationError::InvalidParentKind(observation.id));
        }
        if parent.complete
            && observation.complete
            && (observation.start_offset < parent.start_offset
                || observation.start_offset + observation.duration
                    > parent.start_offset + parent.duration + 1e-9)
        {
            return Err(ValidationError::OutsideParentScope(observation.id));
        }
    }
    for observation in &observations {
        let mut seen = HashSet::new();
        let mut current = observation;
        while current.parent_id != report.operation.observation_root_id {
            if !seen.insert(current.id) {
                return Err(ValidationError::ObservationCycle);
            }
            let Some(parent) = ids.get(&current.parent_id) else {
                break;
            };
            current = parent;
        }
    }
    if report
        .configuration
        .saved_attempt
        .is_some_and(|id| !report.attempts.iter().any(|attempt| attempt.id == id))
    {
        return Err(ValidationError::MissingSavedAttempt);
    }

    let retained = u64::try_from(observations.len()).unwrap_or(u64::MAX);
    validate_counter_overflow(report)?;
    if report.coverage.retained_observations != retained
        || report.coverage.detail_limit == 0
        || report.coverage.retained_observations > report.coverage.detail_limit
    {
        return Err(ValidationError::InvalidCoverage);
    }
    if report.coverage.truncated != (report.coverage.omitted_observations > 0) {
        return Err(ValidationError::InvalidCoverage);
    }
    for total in report.coverage.fixed_duration_totals {
        validate_finite(total, "coverage.fixed_duration_totals")?;
        if total < 0.0 {
            return Err(ValidationError::InvalidMeasurement);
        }
    }
    if report.coverage.phase_totals.len() > PHASE_NAMES.len() {
        return Err(ValidationError::InvalidCoverage);
    }
    let mut phase_total_names = HashSet::new();
    for total in &report.coverage.phase_totals {
        validate_text(&total.name)?;
        validate_finite(total.total_duration, "coverage.phase_totals.total_duration")?;
        if total.total_duration < 0.0 {
            return Err(ValidationError::InvalidMeasurement);
        }
        if !PHASE_NAMES.contains(&total.name.as_str()) {
            return Err(ValidationError::PrivacyField);
        }
        if !phase_total_names.insert(total.name.as_str()) {
            return Err(ValidationError::InvalidCoverage);
        }
    }
    Ok(())
}

fn validate_counter_overflow(report: &Report) -> Result<(), ValidationError> {
    if report.coverage.counter_overflow_fields.len() > 64 {
        return Err(ValidationError::InvalidCoverage);
    }
    let mut names = HashSet::new();
    for field in &report.coverage.counter_overflow_fields {
        let Some(is_none) = counter_is_none(&report.measurements.work, field) else {
            return Err(ValidationError::PrivacyField);
        };
        if !is_none || !names.insert(field.as_str()) {
            return Err(ValidationError::InvalidCoverage);
        }
    }
    let counters = &report.measurements.work;
    let fields = [
        ("dispatch_count", counters.dispatch_count),
        ("submit_count", counters.submit_count),
        ("command_buffer_count", counters.command_buffer_count),
        ("host_wait_count", counters.host_wait_count),
        ("upload_count", counters.upload_count),
        ("upload_bytes", counters.upload_bytes),
        ("image_upload_count", counters.image_upload_count),
        ("image_upload_bytes", counters.image_upload_bytes),
        (
            "spectral_lut_upload_count",
            counters.spectral_lut_upload_count,
        ),
        (
            "spectral_lut_upload_bytes",
            counters.spectral_lut_upload_bytes,
        ),
        ("uniform_upload_count", counters.uniform_upload_count),
        ("uniform_upload_bytes", counters.uniform_upload_bytes),
        ("scratch_upload_count", counters.scratch_upload_count),
        ("scratch_upload_bytes", counters.scratch_upload_bytes),
        ("readback_count", counters.readback_count),
        ("readback_bytes", counters.readback_bytes),
        ("staging_copy_count", counters.staging_copy_count),
        ("staging_copy_bytes", counters.staging_copy_bytes),
        ("device_copy_count", counters.device_copy_count),
        ("device_copy_bytes", counters.device_copy_bytes),
        ("buffer_create_count", counters.buffer_create_count),
        ("buffer_create_bytes", counters.buffer_create_bytes),
        (
            "pipeline_cache_hit_count",
            counters.pipeline_cache_hit_count,
        ),
        (
            "pipeline_cache_miss_count",
            counters.pipeline_cache_miss_count,
        ),
        (
            "telemetry_buffer_create_count",
            counters.telemetry_buffer_create_count,
        ),
        (
            "telemetry_buffer_create_bytes",
            counters.telemetry_buffer_create_bytes,
        ),
        ("telemetry_resolve_count", counters.telemetry_resolve_count),
        ("telemetry_copy_count", counters.telemetry_copy_count),
        ("telemetry_copy_bytes", counters.telemetry_copy_bytes),
        ("executed_pass_count", counters.executed_pass_count),
        ("timed_pass_count", counters.timed_pass_count),
        ("valid_pass_count", counters.valid_pass_count),
        ("omitted_pass_count", counters.omitted_pass_count),
    ];
    for (field, value) in fields {
        if value.is_none() && !names.contains(field) {
            return Err(ValidationError::InvalidCoverage);
        }
    }
    Ok(())
}

fn counter_is_none(
    counters: &spektrafilm_gpu::telemetry::WorkCounters,
    field: &str,
) -> Option<bool> {
    Some(match field {
        "dispatch_count" => counters.dispatch_count.is_none(),
        "submit_count" => counters.submit_count.is_none(),
        "command_buffer_count" => counters.command_buffer_count.is_none(),
        "host_wait_count" => counters.host_wait_count.is_none(),
        "upload_count" => counters.upload_count.is_none(),
        "upload_bytes" => counters.upload_bytes.is_none(),
        "image_upload_count" => counters.image_upload_count.is_none(),
        "image_upload_bytes" => counters.image_upload_bytes.is_none(),
        "spectral_lut_upload_count" => counters.spectral_lut_upload_count.is_none(),
        "spectral_lut_upload_bytes" => counters.spectral_lut_upload_bytes.is_none(),
        "uniform_upload_count" => counters.uniform_upload_count.is_none(),
        "uniform_upload_bytes" => counters.uniform_upload_bytes.is_none(),
        "scratch_upload_count" => counters.scratch_upload_count.is_none(),
        "scratch_upload_bytes" => counters.scratch_upload_bytes.is_none(),
        "readback_count" => counters.readback_count.is_none(),
        "readback_bytes" => counters.readback_bytes.is_none(),
        "staging_copy_count" => counters.staging_copy_count.is_none(),
        "staging_copy_bytes" => counters.staging_copy_bytes.is_none(),
        "device_copy_count" => counters.device_copy_count.is_none(),
        "device_copy_bytes" => counters.device_copy_bytes.is_none(),
        "buffer_create_count" => counters.buffer_create_count.is_none(),
        "buffer_create_bytes" => counters.buffer_create_bytes.is_none(),
        "pipeline_cache_hit_count" => counters.pipeline_cache_hit_count.is_none(),
        "pipeline_cache_miss_count" => counters.pipeline_cache_miss_count.is_none(),
        "telemetry_buffer_create_count" => counters.telemetry_buffer_create_count.is_none(),
        "telemetry_buffer_create_bytes" => counters.telemetry_buffer_create_bytes.is_none(),
        "telemetry_resolve_count" => counters.telemetry_resolve_count.is_none(),
        "telemetry_copy_count" => counters.telemetry_copy_count.is_none(),
        "telemetry_copy_bytes" => counters.telemetry_copy_bytes.is_none(),
        "executed_pass_count" => counters.executed_pass_count.is_none(),
        "timed_pass_count" => counters.timed_pass_count.is_none(),
        "valid_pass_count" => counters.valid_pass_count.is_none(),
        "omitted_pass_count" => counters.omitted_pass_count.is_none(),
        _ => return None,
    })
}

fn validate_observation_name(observation: &Observation) -> Result<(), ValidationError> {
    const STAGES: &[&str] = &[
        "metering",
        "geometry",
        "color_reference",
        "spectral_prepare",
        "film_chain",
        "filming_expose",
        "filming_develop",
        "converting",
        "grain_v1",
        "grain_v1_sampler",
        "grain_v2",
        "grain_v2_cpu",
        "printing",
        "scanning",
        "post_scan",
        "magazine_print_color",
        "positive_scan",
        "input_transfer",
        "output_transfer",
        "rgb_to_raw",
        "highlight_boost",
        "optical_diffusion",
        "scanner_lut",
        "enlarger_lut_prepare",
        "enlarger_lut_interpolate",
        "glare",
        "output_gamut",
        "pipeline_create",
    ];
    let valid = match observation.kind {
        ObservationKind::Attempt => observation.name == "attempt",
        ObservationKind::Phase => PHASE_NAMES.contains(&observation.name.as_str()),
        ObservationKind::Stage => STAGES.contains(&observation.name.as_str()),
        ObservationKind::Batch | ObservationKind::Pass => stable_identifier(&observation.name),
    };
    if valid {
        Ok(())
    } else {
        Err(ValidationError::PrivacyField)
    }
}

fn stable_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

fn validate_unavailable(fields: &[super::report::UnavailableField]) -> Result<(), ValidationError> {
    let mut names = HashSet::new();
    for field in fields {
        if !stable_identifier(&field.field) {
            return Err(ValidationError::PrivacyField);
        }
        if !names.insert(field.field.as_str()) {
            return Err(ValidationError::InvalidCoverage);
        }
    }
    Ok(())
}

fn validate_execution_parts(
    adapter: Option<&spektrafilm_gpu::telemetry::AdapterDescription>,
    dimensions: Option<[u32; 2]>,
) -> Result<(), ValidationError> {
    if let Some(adapter) = adapter {
        validate_text(&adapter.api)?;
        validate_text(&adapter.name)?;
        validate_measurement(&adapter.timestamp_period_ns)?;
        if adapter
            .timestamp_period_ns
            .value
            .is_some_and(|period| period <= 0.0)
        {
            return Err(ValidationError::InvalidMeasurement);
        }
    }
    if dimensions.is_some_and(|dimensions| dimensions.contains(&0)) {
        return Err(ValidationError::InvalidCoverage);
    }
    Ok(())
}

fn validate_measurement(
    measurement: &spektrafilm_gpu::telemetry::Measurement,
) -> Result<(), ValidationError> {
    match measurement.status {
        MeasurementStatus::Available => {
            let Some(value) = measurement.value else {
                return Err(ValidationError::InvalidMeasurement);
            };
            if !value.is_finite() || value < 0.0 || measurement.reason.is_some() {
                return Err(ValidationError::InvalidMeasurement);
            }
        }
        MeasurementStatus::Unavailable => {
            if measurement.value.is_some() || measurement.reason.is_none() {
                return Err(ValidationError::InvalidMeasurement);
            }
        }
        MeasurementStatus::NotApplicable => {
            if measurement.value.is_some() || measurement.reason.is_none() {
                return Err(ValidationError::InvalidMeasurement);
            }
        }
    }
    Ok(())
}

fn validate_finite(value: f64, field: &'static str) -> Result<(), ValidationError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ValidationError::NonFinite(field))
    }
}

fn validate_optional_finite(
    value: Option<f64>,
    field: &'static str,
) -> Result<(), ValidationError> {
    if let Some(value) = value {
        validate_finite(value, field)?;
    }
    Ok(())
}

fn validate_text(value: &str) -> Result<(), ValidationError> {
    if value.chars().count() <= 256 {
        Ok(())
    } else {
        Err(ValidationError::TextLimitExceeded)
    }
}
