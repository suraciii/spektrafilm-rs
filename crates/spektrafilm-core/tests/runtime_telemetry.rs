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
