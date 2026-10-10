use spektrafilm_core::telemetry::{
    AvailabilityReason, CollectionMode, ObservationKind, Operation, OperationKind, Outcome,
    PersistenceError, Purpose, Report, validate_report_destination,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEST_PATH: AtomicU64 = AtomicU64::new(1);

fn test_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "spektrafilm-telemetry-test-{}-{}",
        std::process::id(),
        NEXT_TEST_PATH.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn collected_report() -> Report {
    let mut operation = Operation::new(OperationKind::Preview, CollectionMode::Summary);
    operation.set_backend_requested(spektrafilm_core::telemetry::BackendRequested::Cpu);
    let phase = operation.scope("simulation", ObservationKind::Phase, Purpose::Image);
    let stage = phase
        .context()
        .scope("grain_v1", ObservationKind::Stage, Purpose::Image);
    stage.finish(Outcome::Succeeded);
    phase.finish(Outcome::Succeeded);
    operation.finish(Outcome::Succeeded).unwrap()
}

#[test]
fn disabled_operation_preserves_identity_without_report_state() {
    let operation = Operation::new(OperationKind::Preview, CollectionMode::Off);
    assert_ne!(operation.id(), 0);
    assert!(!operation.context().enabled());
    assert!(operation.finish(Outcome::Cancelled).is_none());
}

#[test]
fn report_round_trip_preserves_u64_above_json_float_precision() {
    let mut report = collected_report();
    let large = (1_u64 << 53) + 1;
    report.measurements.work.dispatch_count = Some(large);
    let json = report.to_json().unwrap();
    let decoded = Report::from_json(&json).unwrap();
    assert_eq!(decoded.measurements.work.dispatch_count, Some(large));
    decoded.validate().unwrap();
}

#[test]
fn validator_rejects_nonfinite_and_missing_parent() {
    let mut report = collected_report();
    report.operation.duration = Some(f64::NAN);
    assert!(report.validate().is_err());

    let mut report = collected_report();
    report.operation.observation_root_id =
        report.operation.observation_root_id.saturating_add(1000);
    report.attempts.clear();
    assert!(report.validate().is_err());
}

#[test]
fn failed_before_configuration_keeps_availability_reasons() {
    let operation = Operation::new(OperationKind::Render, CollectionMode::Summary);
    let report = operation.finish(Outcome::Failed).unwrap();
    report.validate().unwrap();
    assert!(report.configuration.working_dimensions.is_none());
    assert!(report.configuration.unavailable_fields.iter().any(|field| {
        field.field == "working_dimensions" && field.reason == AvailabilityReason::NotReached
    }));
    assert!(report.configuration.unavailable_fields.iter().any(|field| {
        field.field == "render" && field.reason == AvailabilityReason::NotReached
    }));
    assert!(report.configuration.unavailable_fields.iter().any(|field| {
        field.field == "gpu_precision" && field.reason == AvailabilityReason::NotReached
    }));
    assert!(report.configuration.unavailable_fields.iter().any(|field| {
        field.field == "spectral_preparation_precision"
            && field.reason == AvailabilityReason::NotReached
    }));
    assert!(report.execution.unavailable_fields.iter().any(|field| {
        field.field == "backend_requested" && field.reason == AvailabilityReason::NotReached
    }));
}

#[test]
fn failed_attempt_keeps_index_outcome_and_parent() {
    let operation = Operation::new(OperationKind::Render, CollectionMode::Summary);
    let attempt = operation.attempt(2);
    let attempt_id = attempt.id();
    let simulation = attempt
        .context()
        .scope("simulation", ObservationKind::Phase, Purpose::Image);
    simulation.finish(Outcome::Failed);
    attempt.finish(Outcome::Failed);
    let report = operation.finish(Outcome::Failed).unwrap();
    report.validate().unwrap();
    let attempt = report
        .attempts
        .iter()
        .find(|item| item.id == attempt_id)
        .unwrap();
    assert_eq!(attempt.attempt_index, Some(2));
    assert_eq!(attempt.outcome, Some(Outcome::Failed));
    let simulation = report
        .phases
        .iter()
        .find(|item| item.parent_id == attempt_id)
        .unwrap();
    assert_eq!(simulation.outcome, Some(Outcome::Failed));
    assert!(!simulation.complete);
}

#[test]
fn validator_rejects_completed_child_outside_completed_parent() {
    let mut report = collected_report();
    let parent_id = report.phases[0].id;
    let parent_end = report.phases[0].start_offset + report.phases[0].duration;
    let stage = report
        .stages
        .iter_mut()
        .find(|stage| stage.parent_id == parent_id)
        .unwrap();
    stage.start_offset = parent_end + 1.0;
    assert!(report.validate().is_err());
}
#[test]
fn validator_rejects_cycles_root_collisions_and_operation_overrun() {
    let mut report = collected_report();
    let stage_id = report.stages[0].id;
    report.stages[0].parent_id = stage_id;
    assert!(matches!(
        report.validate().unwrap_err(),
        spektrafilm_core::telemetry::ValidationError::ObservationCycle
    ));

    let mut report = collected_report();
    report.stages[0].id = report.operation.observation_root_id;
    assert!(matches!(
        report.validate().unwrap_err(),
        spektrafilm_core::telemetry::ValidationError::RootIdCollision
    ));

    let mut report = collected_report();
    report.operation.duration = Some(0.0);
    assert!(matches!(
        report.validate().unwrap_err(),
        spektrafilm_core::telemetry::ValidationError::OutsideParentScope(_)
    ));
}

#[test]
fn detail_truncation_keeps_exact_totals_and_valid_ancestry() {
    let mut operation = Operation::new(OperationKind::Process, CollectionMode::Summary);
    operation.set_backend_requested(spektrafilm_core::telemetry::BackendRequested::Cpu);
    for _ in 0..1100 {
        let scope = operation.scope("simulation", ObservationKind::Phase, Purpose::Image);
        let mut work = spektrafilm_core::telemetry::WorkCounters::default();
        work.dispatch_count = Some(1);
        scope.context().record_work(&work);
        scope.finish(Outcome::Succeeded);
    }
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    assert_eq!(report.coverage.retained_observations, 1024);
    assert_eq!(report.coverage.omitted_observations, 76);
    assert!(report.coverage.truncated);
    assert_eq!(report.measurements.work.dispatch_count, Some(1100));
    let simulation_total = report
        .coverage
        .phase_totals
        .iter()
        .find(|total| total.name == "simulation")
        .unwrap();
    assert_eq!(simulation_total.count, 1100);
    assert_eq!(
        report
            .summary()
            .largest_phase
            .as_ref()
            .map(|(name, _)| name.as_str()),
        Some("simulation")
    );
}

#[test]
fn truncated_saved_attempt_has_explicit_availability_reason() {
    let mut operation = Operation::new(OperationKind::Process, CollectionMode::Summary);
    for index in 1..=1025 {
        let attempt = operation.attempt(index);
        if index == 1025 {
            operation.set_saved_attempt(attempt.id());
        }
        attempt.finish(Outcome::Succeeded);
    }
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    assert_eq!(report.configuration.saved_attempt, None);
    assert!(report.configuration.unavailable_fields.iter().any(|field| {
        field.field == "saved_attempt"
            && field.reason == spektrafilm_core::telemetry::AvailabilityReason::DetailLimit
    }));
    assert_eq!(report.coverage.omitted_observations, 1);
}

#[test]
fn counter_overflow_uses_stable_field_identity_without_wrapping() {
    let mut report = collected_report();
    report.measurements.work.dispatch_count = None;
    report
        .coverage
        .counter_overflow_fields
        .push("dispatch_count".to_owned());
    report.validate().unwrap();

    let mut report = collected_report();
    report
        .coverage
        .counter_overflow_fields
        .push("upload_bytes".to_owned());
    assert!(report.validate().is_err());
}

#[test]
fn checked_counter_sum_overflow_is_reported_without_wrapping() {
    let operation = Operation::new(OperationKind::Process, CollectionMode::Summary);
    let phase = operation.scope("simulation", ObservationKind::Phase, Purpose::Image);
    let mut work = spektrafilm_core::telemetry::WorkCounters::default();
    work.dispatch_count = Some(u64::MAX);
    phase.context().record_work(&work);
    phase.context().record_work(&work);
    phase.finish(Outcome::Succeeded);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    assert_eq!(report.measurements.work.dispatch_count, None);
    assert_eq!(
        report.coverage.counter_overflow_fields,
        vec!["dispatch_count".to_owned()]
    );
    report.validate().unwrap();
}

#[test]
fn repeated_attempts_keep_distinct_route_facts() {
    let operation = Operation::new(OperationKind::Render, CollectionMode::Summary);
    let first = operation.attempt(1);
    first
        .context()
        .set_path(spektrafilm_core::telemetry::ExecutionPath::Cpu);
    first.finish(Outcome::Succeeded);
    let second = operation.attempt(2);
    second
        .context()
        .set_path(spektrafilm_core::telemetry::ExecutionPath::GpuResident);
    second.finish(Outcome::Succeeded);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    assert_eq!(
        report.execution.path,
        Some(spektrafilm_core::telemetry::ExecutionPath::Mixed)
    );
    let paths: Vec<_> = report
        .attempts
        .iter()
        .map(|attempt| {
            attempt
                .execution
                .as_ref()
                .and_then(|execution| execution.path)
        })
        .collect();
    assert_eq!(
        paths,
        vec![
            Some(spektrafilm_core::telemetry::ExecutionPath::Cpu),
            Some(spektrafilm_core::telemetry::ExecutionPath::GpuResident),
        ]
    );
}

#[test]
fn cpu_gpu_timing_is_explicitly_unavailable_not_zero() {
    let operation = Operation::new(OperationKind::Process, CollectionMode::GpuTiming);
    operation
        .context()
        .set_backend_selected(spektrafilm_core::telemetry::BackendSelected::Cpu);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    assert_eq!(
        report.configuration.collection_mode_effective,
        CollectionMode::Summary
    );
    assert_eq!(
        report.measurements.gpu_timing.compute_pass_sum.status,
        spektrafilm_core::telemetry::MeasurementStatus::Unavailable
    );
    assert_eq!(
        report.measurements.gpu_timing.compute_pass_sum.reason,
        Some(spektrafilm_core::telemetry::AvailabilityReason::Unsupported)
    );
    assert!(
        report
            .measurements
            .gpu_timing
            .compute_pass_sum
            .value
            .is_none()
    );
}

#[test]
fn gpu_timing_measurements_total_actual_batch_values() {
    use spektrafilm_core::telemetry::{BatchMeasurements, Measurement};
    let operation = Operation::new(OperationKind::Render, CollectionMode::GpuTiming);
    let phase = operation.scope("simulation", ObservationKind::Phase, Purpose::Image);
    let batch = phase
        .context()
        .scope("grain_v2", ObservationKind::Batch, Purpose::Image);
    let measurement = BatchMeasurements {
        host_prepare: Measurement::unavailable(AvailabilityReason::NotReached),
        submit_to_map_ready: Measurement::unavailable(AvailabilityReason::NotReached),
        host_materialize: Measurement::unavailable(AvailabilityReason::NotReached),
        completion_boundary: "queue_submission".to_owned(),
        compute_pass_sum: Measurement::available(2.0),
        partial_compute_pass_sum: Measurement::available(0.5),
    };
    batch.context().record_batch(measurement.clone());
    batch.context().record_batch(measurement);
    batch.finish(Outcome::Succeeded);
    phase.finish(Outcome::Succeeded);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    assert_eq!(
        report.measurements.gpu_timing.compute_pass_sum.value,
        Some(4.0)
    );
    assert_eq!(
        report
            .measurements
            .gpu_timing
            .partial_compute_pass_sum
            .value,
        Some(1.0)
    );
}

#[test]
fn cpu_execution_reports_available_zero_gpu_work() {
    let operation = Operation::new(OperationKind::Process, CollectionMode::Summary);
    let context = operation.context();
    context.set_backend_selected(spektrafilm_core::telemetry::BackendSelected::Cpu);
    context.set_path(spektrafilm_core::telemetry::ExecutionPath::Cpu);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    assert_eq!(report.measurements.work.dispatch_count, Some(0));
    assert_eq!(report.measurements.work.upload_bytes, Some(0));
    assert_eq!(
        report.execution.path,
        Some(spektrafilm_core::telemetry::ExecutionPath::Cpu)
    );
    assert_eq!(report.configuration.gpu_precision, None);
    assert!(report.configuration.unavailable_fields.iter().any(|field| {
        field.field == "gpu_precision" && field.reason == AvailabilityReason::NotApplicable
    }));
}

#[test]
fn resident_route_facts_remain_available_outside_observation_detail() {
    let operation = Operation::new(OperationKind::Render, CollectionMode::Summary);
    let context = operation.context();
    context.set_backend_selected(spektrafilm_core::telemetry::BackendSelected::Wgpu);
    context.set_path(spektrafilm_core::telemetry::ExecutionPath::GpuResident);
    context.set_working_dimensions(1024, 768);
    let phase = operation.scope("simulation", ObservationKind::Phase, Purpose::Image);
    let stage = phase
        .context()
        .scope("grain_v2", ObservationKind::Stage, Purpose::Image);
    stage
        .context()
        .record_executor(spektrafilm_core::telemetry::Executor::Gpu, None);
    stage.finish(Outcome::Succeeded);
    phase.finish(Outcome::Succeeded);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    assert_eq!(
        report.execution.backend_selected,
        Some(spektrafilm_core::telemetry::BackendSelected::Wgpu)
    );
    assert_eq!(
        report.execution.path,
        Some(spektrafilm_core::telemetry::ExecutionPath::GpuResident)
    );
    assert_eq!(report.configuration.working_dimensions, Some([1024, 768]));
    assert_eq!(
        report.stages[0].executor,
        Some(spektrafilm_core::telemetry::Executor::Gpu)
    );
}

#[test]
fn mixed_grain_v1_stage_reports_cpu_reason() {
    let operation = Operation::new(OperationKind::Render, CollectionMode::Summary);
    operation
        .context()
        .set_path(spektrafilm_core::telemetry::ExecutionPath::PerStage);
    let phase = operation.scope("simulation", ObservationKind::Phase, Purpose::Image);
    let stage = phase
        .context()
        .scope("grain_v1", ObservationKind::Stage, Purpose::Image);
    stage.context().record_executor(
        spektrafilm_core::telemetry::Executor::Cpu,
        Some(spektrafilm_core::telemetry::CpuReason::GrainV1Sampler),
    );
    stage
        .context()
        .record_executor(spektrafilm_core::telemetry::Executor::Gpu, None);
    stage.finish(Outcome::Succeeded);
    phase.finish(Outcome::Succeeded);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    assert_eq!(
        report.stages[0].executor,
        Some(spektrafilm_core::telemetry::Executor::Mixed)
    );
    assert_eq!(
        report.summary().cpu_stage_reasons,
        vec![(
            "grain_v1".to_owned(),
            spektrafilm_core::telemetry::CpuReason::GrainV1Sampler,
        )]
    );
}

#[test]
fn color_space_roles_recognize_frontend_names_without_leaking_custom_values() {
    use spektrafilm_core::telemetry::ColorSpaceRole;
    assert_eq!(
        ColorSpaceRole::parse("Display P3"),
        ColorSpaceRole::DisplayP3
    );
    assert_eq!(
        ColorSpaceRole::parse("ACES2065-1"),
        ColorSpaceRole::Aces2065
    );
    assert_eq!(
        ColorSpaceRole::parse("ITU-R BT.2020"),
        ColorSpaceRole::Rec2020
    );
    assert_eq!(
        ColorSpaceRole::parse("Adobe RGB (1998)"),
        ColorSpaceRole::AdobeRgb
    );
    assert_eq!(
        ColorSpaceRole::parse("ProPhoto RGB"),
        ColorSpaceRole::ProphotoRgb
    );
    assert_eq!(
        ColorSpaceRole::parse("/private/profile.icc"),
        ColorSpaceRole::Custom
    );
}

#[test]
fn diagnostic_issues_are_bounded_with_exact_omissions() {
    let mut operation = Operation::new(OperationKind::Save, CollectionMode::Summary);
    operation.set_source_render(spektrafilm_core::telemetry::SourceRender::new(
        1,
        None,
        None,
        1,
        1,
        true,
        spektrafilm_core::telemetry::ColorSpaceRole::Srgb,
        true,
    ));
    for _ in 0..70 {
        operation.issue(
            spektrafilm_core::telemetry::IssueCategory::MetadataWarning,
            spektrafilm_core::telemetry::IssueBoundary::Metadata,
        );
    }
    let report = operation.finish(Outcome::Succeeded).unwrap();
    assert_eq!(report.diagnostic_issues.len(), 64);
    assert_eq!(report.coverage.omitted_issues, 6);
    report.validate().unwrap();
}

#[test]
fn save_report_keeps_small_source_render_without_render_configuration() {
    let mut operation = Operation::new(OperationKind::Save, CollectionMode::Summary);
    operation.set_source_render(spektrafilm_core::telemetry::SourceRender::new(
        77,
        Some(3),
        Some(9),
        800,
        600,
        false,
        spektrafilm_core::telemetry::ColorSpaceRole::Srgb,
        true,
    ));
    operation.set_saving_configuration(spektrafilm_core::telemetry::SavingConfiguration::new(
        spektrafilm_core::telemetry::OutputFormat::Png,
        spektrafilm_core::telemetry::BitDepth::U16,
        spektrafilm_core::telemetry::Compression::Deflate,
        spektrafilm_core::telemetry::ColorSpaceRole::Srgb,
        true,
    ));
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    let source = report.source_render.unwrap();
    assert_eq!(source.operation_id, 77);
    assert!(!source.diagnostics_collected);
    assert!(report.configuration.render.is_none());
    assert!(report.configuration.unavailable_fields.iter().any(|field| {
        field.field == "spectral_preparation_precision"
            && field.reason == AvailabilityReason::NotApplicable
    }));
    assert!(report.configuration.unavailable_fields.iter().any(|field| {
        field.field == "render"
            && field.reason == AvailabilityReason::SourceConfigurationNotRetained
    }));
    for backend in ["backend_requested", "backend_selected"] {
        assert!(report.execution.unavailable_fields.iter().any(|field| {
            field.field == backend && field.reason == AvailabilityReason::NotApplicable
        }));
    }
}

#[test]
fn report_destination_rejects_protected_symlink_and_hardlink_aliases() {
    let directory = test_dir();
    let input = directory.join("input.png");
    fs::write(&input, b"image").unwrap();
    let hardlink = directory.join("input-hardlink.png");
    fs::hard_link(&input, &hardlink).unwrap();
    assert!(matches!(
        validate_report_destination(&hardlink, &[&input]).unwrap_err(),
        PersistenceError::ProtectedAlias
    ));

    let output = directory.join("output.png");
    let lexical_alias = directory.join("./output.png");
    assert!(matches!(
        validate_report_destination(&lexical_alias, &[&output]).unwrap_err(),
        PersistenceError::ProtectedAlias
    ));

    #[cfg(unix)]
    {
        let symlink = directory.join("input-link.png");
        std::os::unix::fs::symlink(&input, &symlink).unwrap();
        assert!(matches!(
            validate_report_destination(&symlink, &[&input]).unwrap_err(),
            PersistenceError::InvalidDestination
        ));
    }
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn report_destination_rejects_dangling_protected_raw_output_symlinks() {
    let directory = test_dir();
    let report = directory.join("report.json");
    let relative_raw = directory.join("relative-raw.bin");
    let absolute_raw = directory.join("absolute-raw.bin");
    let chained_raw = directory.join("chained-raw.bin");
    std::os::unix::fs::symlink("report.json", &relative_raw).unwrap();
    std::os::unix::fs::symlink(&report, &absolute_raw).unwrap();
    std::os::unix::fs::symlink("absolute-raw.bin", &chained_raw).unwrap();

    for raw_output in [&relative_raw, &absolute_raw, &chained_raw] {
        assert!(matches!(
            validate_report_destination(&report, &[raw_output]),
            Err(PersistenceError::ProtectedAlias)
        ));
        assert!(!report.exists());
        assert!(
            fs::symlink_metadata(raw_output)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    let ordinary_raw = directory.join("ordinary-raw.bin");
    assert_eq!(
        validate_report_destination(&report, &[&ordinary_raw])
            .unwrap()
            .path(),
        report.as_path()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[cfg(unix)]
#[test]
fn report_destination_rejects_protected_symlink_cycles() {
    let directory = test_dir();
    let report = directory.join("report.json");
    let raw_output = directory.join("raw.bin");
    let intermediate = directory.join("intermediate.bin");
    std::os::unix::fs::symlink("intermediate.bin", &raw_output).unwrap();
    std::os::unix::fs::symlink("raw.bin", &intermediate).unwrap();
    assert!(matches!(
        validate_report_destination(&report, &[&raw_output]),
        Err(PersistenceError::Io(_))
    ));
    assert!(!report.exists());
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn bare_relative_report_destination_resolves_current_directory_parent() {
    let path = PathBuf::from(format!(
        "spektrafilm-telemetry-destination-{}-{}.json",
        std::process::id(),
        NEXT_TEST_PATH.fetch_add(1, Ordering::Relaxed)
    ));
    let destination = validate_report_destination(&path, &[]).unwrap();
    assert_eq!(destination.path(), path.as_path());
}

#[test]
fn report_publication_is_atomic_no_clobber_and_confirmed_overwrite_is_explicit() {
    let directory = test_dir();
    let destination_path = directory.join("report.json");
    let destination = validate_report_destination(&destination_path, &[]).unwrap();
    let report = collected_report();
    destination.write_new(&report).unwrap();
    let first = fs::read_to_string(&destination_path).unwrap();
    assert!(first.contains("\"schema_version\": 1"));

    let destination = validate_report_destination(&destination_path, &[]).unwrap();
    assert!(matches!(
        destination.write_new(&report).unwrap_err(),
        PersistenceError::AlreadyExists
    ));
    assert_eq!(fs::read_to_string(&destination_path).unwrap(), first);

    let mut replacement = collected_report();
    replacement.operation.id = 999;
    destination.write_confirmed(&replacement).unwrap();
    let second = fs::read_to_string(&destination_path).unwrap();
    assert_ne!(first, second);
    assert!(second.contains("\"id\": 999"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn validated_runtime_vocabulary_retains_exact_telemetry_roles() {
    use spektrafilm_core::telemetry::{GamutAlgorithm, RgbToRawAlgorithm, WorkflowRoute};
    for route in [
        "input",
        "input > film > scan",
        "input > film > print > scan",
        "input > convert-film > print > scan",
        "input > convert-film > scan-minus-base",
        "input > convert-film > scan",
    ] {
        assert_ne!(WorkflowRoute::parse(route), WorkflowRoute::Custom);
    }
    assert_eq!(
        WorkflowRoute::parse("input > film > print > scan"),
        WorkflowRoute::FilmPrintScan
    );
    for method in [
        "hanatos2025",
        "mallett2019",
        "arctic2026alpha02",
        "arctic2026beta04",
        "gauss-lasers",
        "jakob2019",
        "otsu2018",
    ] {
        assert_ne!(RgbToRawAlgorithm::parse(method), RgbToRawAlgorithm::Custom);
    }
    assert_eq!(
        RgbToRawAlgorithm::parse("hanatos2025"),
        RgbToRawAlgorithm::Hanatos2025
    );
    for gamut in ["off", "aces_rgc", "oklch", "oklrab", "jzazbz", "cam16ucs"] {
        assert_ne!(GamutAlgorithm::parse(gamut), GamutAlgorithm::Custom);
    }
}

#[test]
fn saving_configuration_matches_tiff_default_and_exr_half_storage() {
    use spektrafilm_core::{image_io, telemetry};
    let options = image_io::SaveOptions {
        depth: image_io::BitDepth::Sixteen,
        color_space: "sRGB",
        cctf_encoding: true,
        jpeg_quality: None,
        jpeg_subsampling: None,
        compression: None,
    };
    let tiff =
        telemetry::SavingConfiguration::from_save_options(image_io::ImageFormat::Tiff, &options);
    assert_eq!(tiff.compression, telemetry::Compression::Zip);
    assert_eq!(tiff.depth, telemetry::BitDepth::U16);
    let exr =
        telemetry::SavingConfiguration::from_save_options(image_io::ImageFormat::Exr, &options);
    assert_eq!(exr.depth, telemetry::BitDepth::F16);
}

#[test]
fn grain_configuration_describes_effective_sampler_branches() {
    use spektrafilm_core::{params, profile, telemetry::RenderConfiguration};
    let profile: profile::Profile = serde_json::from_value(serde_json::json!({
        "metadata": {}, "info": {}, "data": {}
    }))
    .unwrap();
    let mut params = params::RuntimeParams::default();
    params.settings.use_fast_stats = true;
    assert!(
        RenderConfiguration::from_runtime(&params, &profile, &profile)
            .grain
            .v1_fast_statistics_sampler
    );
    params.film_render.grain.sublayers_active = false;
    assert!(
        !RenderConfiguration::from_runtime(&params, &profile, &profile)
            .grain
            .v1_fast_statistics_sampler
    );
    params.film_render.grain.engine = params::GrainEngine::V2;
    params.settings.rgb_to_raw_method = "mallett2019".to_owned();
    let grain = RenderConfiguration::from_runtime(&params, &profile, &profile).grain;
    assert!(!grain.active);
    assert!(!grain.v1_fast_statistics_sampler);
    params.film_render.grain.engine = params::GrainEngine::V1;
    params.film_render.grain.active = false;
    params.film_render.grain.sublayers_active = true;
    assert!(
        !RenderConfiguration::from_runtime(&params, &profile, &profile)
            .grain
            .v1_fast_statistics_sampler
    );
}

#[test]
fn v3_grain_configuration_reports_engine_and_field_mode() {
    use spektrafilm_core::{params, profile, telemetry};
    let profile: profile::Profile = serde_json::from_value(serde_json::json!({
        "metadata": {}, "info": {}, "data": {}
    }))
    .unwrap();
    let mut params = params::RuntimeParams::default();
    params.film_render.grain.engine = params::GrainEngine::V3;
    params.film_render.grain.v3_dye_support_um = 8.0;
    params.settings.use_fast_stats = true;
    params.film_render.grain.sublayers_active = true;
    let grain = telemetry::RenderConfiguration::from_runtime(&params, &profile, &profile).grain;
    assert!(grain.active);
    assert_eq!(grain.engine, telemetry::GrainEngine::V3);
    assert_eq!(grain.mode, telemetry::GrainMode::DyeField);
    assert!(!grain.v1_fast_statistics_sampler);
}

#[cfg(unix)]
#[test]
fn report_destination_preserves_symlink_parent_traversal_before_canonicalizing() {
    let directory = test_dir();
    let local = directory.join("local");
    let target = directory.join("target");
    fs::create_dir_all(&local).unwrap();
    fs::create_dir_all(target.join("sub")).unwrap();
    std::os::unix::fs::symlink(target.join("sub"), local.join("link")).unwrap();
    let protected = target.join("report.json");
    let destination = local.join("link/../report.json");
    assert!(matches!(
        validate_report_destination(&destination, &[&protected]),
        Err(PersistenceError::ProtectedAlias)
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn negative_durations_and_nonpositive_timestamp_periods_are_rejected() {
    use spektrafilm_core::telemetry::{AdapterDescription, AdapterDeviceType, Measurement};
    let mut report = collected_report();
    report.operation.duration = Some(-1.0);
    assert!(report.validate().is_err());
    let mut report = collected_report();
    report.coverage.fixed_duration_totals[0] = -1.0;
    assert!(report.validate().is_err());
    let mut report = collected_report();
    report.coverage.phase_totals[0].total_duration = -1.0;
    assert!(report.validate().is_err());
    let mut report = collected_report();
    report.measurements.gpu_timing.compute_pass_sum = Measurement::available(-1.0);
    assert!(report.validate().is_err());
    for period in [-1.0, 0.0] {
        let mut report = collected_report();
        report.execution.adapter = Some(AdapterDescription {
            api: "vulkan".to_owned(),
            device_type: AdapterDeviceType::Cpu,
            name: "software".to_owned(),
            description_truncated: false,
            shared_device: true,
            timestamp_supported: true,
            timestamp_enabled: true,
            timestamp_period_ns: Measurement::available(period),
            timestamp_reason: None,
        });
        assert!(report.validate().is_err());
    }
}
