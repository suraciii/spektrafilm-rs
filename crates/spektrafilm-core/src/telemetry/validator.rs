use super::report::{OperationKind, Report};
use spektrafilm_backend::telemetry::{
    GpuStage, HostTimingKind, HostTimingTotal, MeasurementStatus, Observation, ObservationKind,
    PHASE_NAMES, STAGE_NAMES,
};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

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
    #[error("invalid effective scalar-record configuration")]
    InvalidRecordConfiguration,
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
    if let Some(source) = &report.source_render
        && (source.operation_id == 0 || source.dimensions.contains(&0))
    {
        return Err(ValidationError::NonFinite("source_render"));
    }
    validate_unavailable(&report.configuration.unavailable_fields)?;
    validate_unavailable(&report.execution.unavailable_fields)?;
    if let Some(render) = &report.configuration.render {
        validate_finite(
            render.output_mix_amount,
            "configuration.render.output_mix_amount",
        )?;
        if !(0.0..=1.0).contains(&render.output_mix_amount) {
            return Err(ValidationError::InvalidMeasurement);
        }
        for (value, field) in [
            (
                render.scan_operator.exposure_ev,
                "configuration.render.scan_operator.exposure_ev",
            ),
            (
                render.scan_operator.warmth_ev,
                "configuration.render.scan_operator.warmth_ev",
            ),
            (
                render.scan_operator.tint_ev,
                "configuration.render.scan_operator.tint_ev",
            ),
            (
                render.scan_operator.contrast,
                "configuration.render.scan_operator.contrast",
            ),
        ] {
            validate_finite(value, field)?;
        }
        if render.scan_operator.contrast <= 0.0 {
            return Err(ValidationError::InvalidMeasurement);
        }
        if let Some(record) = &render.grain.record {
            use super::configuration::{GrainEngine, GrainMode, ScanOutput, WorkflowRoute};
            for (value, field) in [
                (
                    record.event_contribution_d_um2,
                    "configuration.render.grain.record.event_contribution_d_um2",
                ),
                (
                    record.support_um,
                    "configuration.render.grain.record.support_um",
                ),
                (record.cell_um, "configuration.render.grain.record.cell_um"),
            ] {
                validate_finite(value, field)?;
                if value <= 0.0 {
                    return Err(ValidationError::InvalidRecordConfiguration);
                }
            }
            if !render.grain.active
                || render.grain.engine != GrainEngine::Dye
                || render.grain.mode != GrainMode::DyeField
                || render.workflow_route != WorkflowRoute::FilmScan
                || render.scan_output != ScanOutput::DirectScan
                || render.enlarger_lut_requested
                || render.scanner_lut_requested
                || record.cell_um > record.support_um / 2.0
            {
                return Err(ValidationError::InvalidRecordConfiguration);
            }
        }
        validate_text(&render.model_version)?;
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
    validate_host_timings(report.measurements.host_timings.as_deref())?;
    if report.coverage.host_timing_overflow
        && (!report.coverage.truncated
            || report
                .measurements
                .host_timings
                .as_ref()
                .is_some_and(|totals| !totals.is_empty()))
    {
        return Err(ValidationError::InvalidCoverage);
    }
    let timing = &report.measurements.gpu_timing;
    let mut group_keys = HashSet::new();
    for group in &timing.groups {
        if !group_keys.insert((group.stage, group.kind, group.purpose))
            || group.executed_pass_count == 0
            || group.timed_pass_count > group.executed_pass_count
            || group.valid_pass_count > group.timed_pass_count
        {
            return Err(ValidationError::InvalidCoverage);
        }
        validate_measurement(&group.compute_pass_sum)?;
        validate_measurement(&group.partial_compute_pass_sum)?;
        let complete = group.valid_pass_count == group.executed_pass_count;
        if group.compute_pass_sum.status == MeasurementStatus::Available {
            if !complete || group.compute_pass_sum != group.partial_compute_pass_sum {
                return Err(ValidationError::InvalidCoverage);
            }
        } else if complete
            && group.compute_pass_sum.reason
                != Some(spektrafilm_backend::telemetry::AvailabilityReason::CounterOverflow)
        {
            return Err(ValidationError::InvalidCoverage);
        }
        if group.partial_compute_pass_sum.status == MeasurementStatus::Available
            && group.valid_pass_count == 0
        {
            return Err(ValidationError::InvalidCoverage);
        }
        if group.valid_pass_count > 0
            && group.partial_compute_pass_sum.status != MeasurementStatus::Available
            && group.partial_compute_pass_sum.reason
                != Some(spektrafilm_backend::telemetry::AvailabilityReason::CounterOverflow)
        {
            return Err(ValidationError::InvalidCoverage);
        }
    }
    if !timing.groups.is_empty() {
        let all_complete = timing
            .groups
            .iter()
            .all(|g| g.compute_pass_sum.status == MeasurementStatus::Available);
        let overflow = timing.compute_pass_sum.reason
            == Some(spektrafilm_backend::telemetry::AvailabilityReason::CounterOverflow);
        if (timing.compute_pass_sum.status == MeasurementStatus::Available) != all_complete
            && !overflow
        {
            return Err(ValidationError::InvalidCoverage);
        }
        let sum = timing
            .groups
            .iter()
            .filter_map(|g| g.partial_compute_pass_sum.value)
            .sum::<f64>();
        let any_valid = timing.groups.iter().any(|g| g.valid_pass_count > 0);
        if any_valid
            && timing.partial_compute_pass_sum.value.is_none()
            && timing.partial_compute_pass_sum.reason
                != Some(spektrafilm_backend::telemetry::AvailabilityReason::CounterOverflow)
        {
            return Err(ValidationError::InvalidCoverage);
        }
        if let Some(value) = timing.partial_compute_pass_sum.value
            && (!any_valid
                || !sum.is_finite()
                || (value - sum).abs() > 1e-12 * value.abs().max(sum.abs()).max(1.0))
        {
            return Err(ValidationError::InvalidCoverage);
        }
        if all_complete && !overflow && timing.compute_pass_sum != timing.partial_compute_pass_sum {
            return Err(ValidationError::InvalidCoverage);
        }
    }
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
        validate_host_timings(observation.host_timings.as_deref())?;
        if report.coverage.host_timing_overflow
            && observation
                .host_timings
                .as_ref()
                .is_some_and(|totals| !totals.is_empty())
        {
            return Err(ValidationError::InvalidCoverage);
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
                matches!(
                    parent.kind,
                    ObservationKind::Phase | ObservationKind::Stage | ObservationKind::Batch
                )
            }
            ObservationKind::Batch => {
                matches!(parent.kind, ObservationKind::Phase | ObservationKind::Stage)
            }
            ObservationKind::Pass => parent.kind == ObservationKind::Batch,
        };
        if !valid_parent_kind {
            return Err(ValidationError::InvalidParentKind(observation.id));
        }
        // Pass intervals use the device clock; a host batch can close before retirement.
        if observation.kind != ObservationKind::Pass
            && parent.complete
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
        || report
            .coverage
            .omitted_high_level_observations
            .is_some_and(|count| count > report.coverage.omitted_observations)
    {
        return Err(ValidationError::InvalidCoverage);
    }
    if report.coverage.omitted_observations > 0 && !report.coverage.truncated {
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
    let mut stage_names = HashSet::new();
    for total in &report.coverage.stage_totals {
        validate_finite(total.total_duration, "coverage.stage_totals.total_duration")?;
        if total.total_duration < 0.0 || total.count == 0 {
            return Err(ValidationError::InvalidMeasurement);
        }
        if !STAGE_NAMES.contains(&total.name.as_str())
            && !GpuStage::ALL
                .iter()
                .any(|stage| stage.as_str() == total.name)
        {
            return Err(ValidationError::PrivacyField);
        }
        if !stage_names.insert(total.name.as_str()) {
            return Err(ValidationError::InvalidCoverage);
        }
    }
    Ok(())
}
fn validate_host_timings(totals: Option<&[HostTimingTotal]>) -> Result<(), ValidationError> {
    let Some(totals) = totals else { return Ok(()) };
    if totals.len() > HostTimingKind::ALL.len() {
        return Err(ValidationError::InvalidCoverage);
    }
    let mut kinds = HashSet::new();
    for total in totals {
        validate_finite(total.total_duration, "host_timings.total_duration")?;
        if total.count == 0 || total.total_duration < 0.0 || !kinds.insert(total.kind) {
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
        let Some(value) = report.measurements.work.named_value(field) else {
            return Err(ValidationError::PrivacyField);
        };
        if value.is_some() || !names.insert(field.as_str()) {
            return Err(ValidationError::InvalidCoverage);
        }
    }
    for (field, value) in report.measurements.work.named_fields() {
        if value.is_none() && !names.contains(field) {
            return Err(ValidationError::InvalidCoverage);
        }
    }
    Ok(())
}

fn validate_observation_name(observation: &Observation) -> Result<(), ValidationError> {
    let valid = match observation.kind {
        ObservationKind::Attempt => observation.name == "attempt",
        ObservationKind::Phase => PHASE_NAMES.contains(&observation.name.as_str()),
        ObservationKind::Stage => {
            STAGE_NAMES.contains(&observation.name.as_str())
                || GpuStage::ALL
                    .iter()
                    .any(|stage| stage.as_str() == observation.name)
        }
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
    adapter: Option<&spektrafilm_backend::telemetry::AdapterDescription>,
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
    measurement: &spektrafilm_backend::telemetry::Measurement,
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
