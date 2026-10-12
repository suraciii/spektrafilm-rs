use spektrafilm_core::{
    params::RuntimeParams,
    runtime::Runtime,
    telemetry::{Operation, OperationKind},
};
use spektrafilm_gpu::{
    cpu_backend::CpuBackend,
    telemetry::{CollectionMode, CpuReason, ExecutionPath, ObservationKind, Outcome, Purpose},
};
use spektrafilm_math::{image::ImageBuf, precision::from_f64};

#[test]
fn telemetry_scanner_configuration_roundtrips_actual_approximate_selection() {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > scan".into();
    params.io.scan_film = true;
    params.scanner.receiver_profile = spektrafilm_core::params::ReceiverProfile::LabScanApprox;
    params.scanner.scan_output = "positive_scan".into();
    params.scanner.finish = spektrafilm_core::params::ScannerFinish::LabScan;
    params.scanner.finish_strength = 0.5;
    let runtime =
        Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", params, &data).unwrap();
    let mut operation = Operation::new(OperationKind::Process, CollectionMode::Summary);
    operation.set_render_configuration(runtime.telemetry_configuration());
    let report = operation.finish(Outcome::Succeeded).unwrap();
    let decoded =
        spektrafilm_core::telemetry::Report::from_json(&report.to_json().unwrap()).unwrap();
    let actual = decoded.configuration.render.unwrap();
    assert_eq!(actual.receiver.profile, "lab_scan_approx");
    assert_eq!(
        actual.receiver.calibration_status,
        "approximate_uncalibrated"
    );
    assert!(actual.receiver.approximate);
    assert_eq!(actual.finish, "lab_scan");
    assert_eq!(actual.finish_strength, 0.5);
}

#[test]
fn concurrent_runtime_calls_preserve_pixels_and_operation_ownership() {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.debug.deactivate_spatial_effects = true;
    params.debug.deactivate_stochastic_effects = true;
    let runtime =
        Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", params, &data).unwrap();
    let first = ImageBuf::from_data(
        3,
        2,
        (0..18)
            .map(|index| from_f64(0.1 + index as f64 * 0.015))
            .collect(),
    );
    let second = ImageBuf::from_data(
        2,
        3,
        (0..18)
            .map(|index| from_f64(0.3 + index as f64 * 0.01))
            .collect(),
    );
    let expected_first = runtime.process(first.clone(), &CpuBackend).unwrap();
    let expected_second = runtime.process(second.clone(), &CpuBackend).unwrap();
    let run = |image: ImageBuf| {
        let mut operation = Operation::new(OperationKind::Process, CollectionMode::Summary);
        operation.set_render_configuration(runtime.telemetry_configuration());
        operation.set_input_dimensions(image.width, image.height);
        let attempt = operation.attempt(1);
        let simulation =
            attempt
                .context()
                .scope("simulation", ObservationKind::Phase, Purpose::Image);
        let output = runtime
            .process_observed(image, &CpuBackend, simulation.context())
            .unwrap();
        drop(simulation);
        attempt.finish(Outcome::Succeeded);
        let report = operation.finish(Outcome::Succeeded).unwrap();
        report.validate().unwrap();
        (output, report)
    };
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| run(first));
        let second = scope.spawn(|| run(second));
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_eq!(first.0.data, expected_first.data);
    assert_eq!(second.0.data, expected_second.data);
    assert_ne!(first.1.operation.id, second.1.operation.id);
    assert_eq!(first.1.configuration.working_dimensions, Some([3, 2]));
    assert_eq!(second.1.configuration.working_dimensions, Some([2, 3]));
    for report in [&first.1, &second.1] {
        assert_eq!(report.execution.path, Some(ExecutionPath::Cpu));
        assert!(report.execution.resident_decline_reasons.is_empty());
        assert_eq!(report.measurements.work.dispatch_count, Some(0));
        assert!(
            report
                .stages
                .iter()
                .any(|stage| stage.name == "filming_expose"
                    && stage.cpu_reason == Some(CpuReason::CpuSelected))
        );
    }
}

#[test]
fn positive_scan_cpu_override_preserves_stage_reports() {
    use spektrafilm_gpu::telemetry::{BackendSelected, Executor, ResidentDeclineReason};
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > scan".into();
    params.io.scan_film = true;
    params.scanner.scan_output = "positive_scan".into();
    params.camera.auto_exposure = false;
    params.debug.deactivate_spatial_effects = true;
    params.debug.deactivate_stochastic_effects = true;
    let runtime =
        Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", params, &data).unwrap();
    let image = ImageBuf::from_data(
        3,
        2,
        (0..18)
            .map(|index| from_f64(0.1 + index as f64 * 0.015))
            .collect(),
    );
    let expected = runtime.process(image.clone(), &CpuBackend).unwrap();
    {
        let backend = &CpuBackend as &dyn spektrafilm_gpu::ComputeBackend;
        let mut operation = Operation::new(OperationKind::Process, CollectionMode::Summary);
        operation.set_render_configuration(runtime.telemetry_configuration());
        operation.set_input_dimensions(image.width, image.height);
        let attempt = operation.attempt(1);
        let simulation =
            attempt
                .context()
                .scope("simulation", ObservationKind::Phase, Purpose::Image);
        let output = runtime
            .process_observed(image.clone(), backend, simulation.context())
            .unwrap();
        drop(simulation);
        attempt.finish(Outcome::Succeeded);
        let report = operation.finish(Outcome::Succeeded).unwrap();
        report.validate().unwrap();
        assert_eq!(output.data, expected.data);
        assert_eq!(
            report.execution.backend_selected,
            Some(if backend.is_gpu() {
                BackendSelected::Wgpu
            } else {
                BackendSelected::Cpu
            })
        );
        assert_eq!(report.execution.path, Some(ExecutionPath::Cpu));
        assert_eq!(report.measurements.work.dispatch_count, Some(0));
        assert_eq!(
            report
                .execution
                .resident_decline_reasons
                .contains(&ResidentDeclineReason::PositiveScanOutput),
            backend.is_gpu()
        );
        assert!(
            report
                .stages
                .iter()
                .any(|stage| stage.name == "positive_scan"
                    && stage.executor == Some(Executor::Cpu)
                    && stage.cpu_reason == Some(CpuReason::PositiveScanOutput))
        );
        assert!(
            report
                .stages
                .iter()
                .any(|stage| stage.name == "filming_develop")
        );
    }
}

#[test]
fn active_v3_field_reports_its_cpu_stage_reason() {
    // The supported V3 condition on a 2% crop: the readout region stays a few
    // film millimetres, so the field allocation is small while the pipeline
    // still resolves the engine, the plan and the report.
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.camera.film_format_mm = 36.0;
    params.workflow.route = "input > film > scan".to_owned();
    params.scanner.scan_output = "positive_scan".to_owned();
    params.io.scan_film = true;
    params.io.crop = true;
    params.io.crop_size = [0.02, 0.02];
    params.film_render.grain.engine = spektrafilm_core::params::GrainEngine::V3;
    params.film_render.grain.v3_dye_support_um = 8.0;
    let runtime =
        Runtime::from_stocks("kodak_portra_800", "kodak_portra_endura", params, &data).unwrap();
    let (width, height) = (768u32, 512u32);
    let image = ImageBuf::from_data(
        width,
        height,
        (0..(width * height * 3) as usize)
            .map(|index| from_f64(0.2 + (index % 97) as f64 * 0.004))
            .collect(),
    );
    let mut operation = Operation::new(OperationKind::Process, CollectionMode::Summary);
    operation.set_render_configuration(runtime.telemetry_configuration());
    operation.set_input_dimensions(width, height);
    let attempt = operation.attempt(1);
    let simulation = attempt
        .context()
        .scope("simulation", ObservationKind::Phase, Purpose::Image);
    let output = runtime
        .process_observed(image, &CpuBackend, simulation.context())
        .unwrap();
    drop(simulation);
    attempt.finish(Outcome::Succeeded);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    assert!(output.width < 64 && output.height < 64, "crop geometry");
    assert!(
        report
            .summary()
            .cpu_stage_reasons
            .contains(&("grain_v3_field".to_owned(), CpuReason::GrainV3FieldCpu)),
        "V3 field stage missing from the report: {:?}",
        report.summary().cpu_stage_reasons
    );
}

#[test]
fn slide_reference_batches_are_calibration_not_image_work() {
    let Some(backend) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
        eprintln!("WGPU adapter unavailable; calibration batch accounting not exercised");
        return;
    };
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > scan".into();
    params.io.scan_film = true;
    params.camera.auto_exposure = false;
    params.film_render.grain.active = false;
    params.debug.deactivate_spatial_effects = true;
    params.debug.deactivate_stochastic_effects = true;
    params.settings.use_scanner_lut = false;
    params.scanner.black_point = [0.04; 3];
    params.scanner.white_point = [0.96; 3];
    let runtime =
        Runtime::from_stocks("fujifilm_provia_100f", "kodak_portra_endura", params, &data).unwrap();
    let mut operation = Operation::new(OperationKind::Process, CollectionMode::Summary);
    operation.set_render_configuration(runtime.telemetry_configuration());
    operation.set_input_dimensions(3, 2);
    let attempt = operation.attempt(1);
    let simulation = attempt
        .context()
        .scope("simulation", ObservationKind::Phase, Purpose::Image);
    runtime
        .process_observed(
            ImageBuf::from_data(3, 2, vec![from_f64(0.18); 18]),
            &backend,
            simulation.context(),
        )
        .unwrap();
    drop(simulation);
    attempt.finish(Outcome::Succeeded);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    let calibration = report
        .stages
        .iter()
        .find(|stage| stage.name == "scanner_film_points" && stage.purpose == Purpose::Calibration)
        .expect("slide reference calibration scope");
    let references: Vec<_> = report
        .gpu_batches
        .iter()
        .filter(|batch| batch.parent_id == calibration.id)
        .collect();
    assert_eq!(
        references.len(),
        1,
        "one two-reference spectral capture batch"
    );
    assert_eq!(references[0].purpose, Purpose::Calibration);
    assert_eq!(references[0].work.dispatch_count, Some(1));
    assert_eq!(references[0].work.readback_count, Some(1));
    assert!(
        report
            .gpu_batches
            .iter()
            .any(|batch| batch.name == references[0].name
                && batch.purpose == Purpose::Image
                && batch.parent_id != calibration.id),
        "bulk spectral capture retains image attribution"
    );
}
