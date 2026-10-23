use spektrafilm_backend::cpu_backend::CpuBackend;
use spektrafilm_backend::telemetry::{
    CollectionMode, CpuReason, ExecutionPath, ObservationKind, Outcome, Purpose,
};
use spektrafilm_core::{
    params::RuntimeParams,
    runtime::Runtime,
    telemetry::{Operation, OperationKind},
};
use spektrafilm_math::{image::ImageBuf, precision::from_f64};

#[test]
fn telemetry_scanner_configuration_roundtrips_the_neutral_scan_model() {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > scan".into();
    params.io.scan_film = true;
    params.scanner.scan_output = "negative_to_positive".into();
    let runtime =
        Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", params, &data).unwrap();
    let mut operation = Operation::new(OperationKind::Process, CollectionMode::Summary);
    operation.set_render_configuration(runtime.telemetry_configuration());
    let attempt = operation.attempt(1);
    let simulation = attempt
        .context()
        .scope("simulation", ObservationKind::Phase, Purpose::Image);
    runtime
        .process_observed(
            ImageBuf::from_data(2, 1, vec![from_f64(0.18); 6]),
            &CpuBackend,
            simulation.context(),
        )
        .unwrap();
    drop(simulation);
    attempt.finish(Outcome::Succeeded);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    let mut historical: serde_json::Value =
        serde_json::from_str(&report.to_json().unwrap()).unwrap();
    let old_model = historical["configuration"]["render"]["scan_model"]
        .as_object_mut()
        .unwrap();
    old_model.remove("readout_illuminant");
    old_model.remove("readout_rule");
    old_model.insert("lamp".into(), "D65".into());
    let old = spektrafilm_core::telemetry::Report::from_json(&historical.to_string()).unwrap();
    assert!(
        old.configuration
            .render
            .unwrap()
            .scan_model
            .readout_illuminant
            .is_empty(),
        "historical reports must not invent a material-category readout fact"
    );
    let decoded =
        spektrafilm_core::telemetry::Report::from_json(&report.to_json().unwrap()).unwrap();
    let actual = decoded.configuration.render.unwrap();
    assert_eq!(
        serde_json::to_value(actual.scan_output).unwrap(),
        "negative_to_positive"
    );
    let scan_model = actual.scan_model;
    assert_eq!(scan_model.model, "lab_scan_v1");
    assert_eq!(scan_model.sensor, "cie_1931_cmf");
    assert_eq!(scan_model.white_reference, "clear_film_base");
    assert_eq!(scan_model.output_white, "output_color_space_white");
    assert_eq!(scan_model.matrix, "identity");
    assert_eq!(scan_model.readout_illuminant, "D50");
    assert_eq!(scan_model.readout_rule, "default");
    assert_eq!(actual.conversion_style, "standard");
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
fn negative_to_positive_cpu_override_preserves_stage_reports() {
    use spektrafilm_backend::telemetry::{BackendSelected, Executor, ResidentDeclineReason};
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > scan".into();
    params.io.scan_film = true;
    params.scanner.scan_output = "negative_to_positive".into();
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
        let backend = &CpuBackend as &dyn spektrafilm_backend::ComputeBackend;
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
            report
                .stages
                .iter()
                .filter(|stage| stage.name == "standard_setup")
                .count(),
            1
        );

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
                .contains(&ResidentDeclineReason::NegativeToPositiveOutput),
            backend.is_gpu()
        );
        assert!(
            report
                .stages
                .iter()
                .any(|stage| stage.name == "negative_to_positive"
                    && stage.executor == Some(Executor::Cpu)
                    && stage.cpu_reason == Some(CpuReason::NegativeToPositiveOutput))
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
    // The supported Dye condition on a 2% crop: the readout region stays a few
    // film millimetres, so the field allocation is small while the pipeline
    // still resolves the engine, the plan and the report.
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.camera.film_format_mm = 36.0;
    params.workflow.route = "input > film > scan".to_owned();
    params.scanner.scan_output = "negative_to_positive".to_owned();
    params.io.scan_film = true;
    params.io.crop = true;
    params.io.crop_size = [0.02, 0.02];
    params.film_render.grain.engine = spektrafilm_core::params::GrainEngine::Dye;
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
        "Dye field stage missing from the report: {:?}",
        report.summary().cpu_stage_reasons
    );
}

#[test]
fn active_v3_field_reports_its_device_stage() {
    use spektrafilm_backend::ComputeBackend as _;
    // The delivered raster (no crop, no upscale) resolves the Dye field as
    // the resident device pass: the render stays on the resident chain and
    // records no host field stage. The host field and its published reason
    // are held by `active_v3_field_reports_its_cpu_stage_reason` (a host
    // backend) and the mass-law case in the resident tests.
    let gpu = spektrafilm_gpu::test_support::adapter();
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.camera.film_format_mm = 8.0;
    params.workflow.route = "input > film > scan".to_owned();
    params.io.scan_film = true;
    params.io.output_gamut_compress.algorithm = "off".into();
    params.film_render.grain.engine = spektrafilm_core::params::GrainEngine::Dye;
    params.film_render.grain.v3_dye_support_um = 8.0;
    let runtime =
        Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", params, &data).unwrap();
    let (width, height) = (32u32, 32u32);
    let image = ImageBuf::from_data(
        width,
        height,
        (0..(width * height * 3) as usize)
            .map(|index| from_f64(0.05 + 0.9 * (index % 97) as f64 / 97.0))
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
        .process_observed(image, gpu.backend(), simulation.context())
        .unwrap();
    drop(simulation);
    attempt.finish(Outcome::Succeeded);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    let summary = report.summary();
    assert_eq!(
        summary.path,
        Some(ExecutionPath::GpuResident),
        "the device Dye field must keep the resident chain on {}: {:?}",
        gpu.name(),
        summary.path
    );
    assert!(
        !summary
            .cpu_stage_reasons
            .iter()
            .any(|(stage, _)| stage == "grain_v3_field"),
        "the device Dye field recorded a host stage on {}: {:?}",
        gpu.name(),
        summary.cpu_stage_reasons
    );
    assert!(
        summary.resident_decline_reasons.is_empty(),
        "the Dye field must not decline the resident route: {:?}",
        summary.resident_decline_reasons
    );
    assert_eq!((output.width, output.height), (width, height));
}

#[test]
fn gpu_timing_collection_survives_a_host_fence() {
    // A host fence splits the resident chain into several submissions inside
    // one batch. The timestamp readback is mapped at the batch boundary, so
    // the submission that follows the fence must not see a mapped buffer
    // (wgpu rejects that submit with a validation error), and the collected
    // passes stay complete.
    let gpu = spektrafilm_gpu::test_support::adapter();
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.workflow.route = "input > film > scan".to_owned();
    params.io.scan_film = true;
    params.io.output_gamut_compress.algorithm = "off".into();
    // The default Spektra engine runs as the assembled density fence.
    let runtime =
        Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", params, &data).unwrap();
    let (width, height) = (32u32, 32u32);
    let image = ImageBuf::from_data(
        width,
        height,
        (0..(width * height * 3) as usize)
            .map(|index| from_f64(0.05 + 0.9 * (index % 97) as f64 / 97.0))
            .collect(),
    );
    let mut operation = Operation::new(OperationKind::Process, CollectionMode::GpuTiming);
    operation.set_render_configuration(runtime.telemetry_configuration());
    operation.set_input_dimensions(width, height);
    let attempt = operation.attempt(1);
    let simulation = attempt
        .context()
        .scope("simulation", ObservationKind::Phase, Purpose::Image);
    let output = runtime
        .process_observed(image, gpu.backend(), simulation.context())
        .unwrap();
    drop(simulation);
    attempt.finish(Outcome::Succeeded);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();
    assert_eq!(
        report.summary().path,
        Some(ExecutionPath::GpuResident),
        "the fenced chain must stay resident"
    );
    let chain = report
        .gpu_batches
        .iter()
        .find(|batch| batch.name == "film_chain")
        .expect("the resident chain records its batch");
    assert!(
        chain.work.submit_count > Some(1),
        "the fence must split the chain into several submissions: {:?}",
        chain.work.submit_count
    );
    // Timed passes require device timestamp queries. llvmpipe exposes them, so
    // every pass is timed there; Metal on the macOS runner does not, so the
    // same fence yields `timed_pass_count == 0` against 39 executed passes.
    // Branch on the adapter's own claim so both devices assert the strict
    // invariant that applies to them, instead of an equality that only holds
    // on one. `valid <= executed` is checked unconditionally because a pass
    // cannot be valid without having run on either device.
    let timestamps = report
        .execution
        .adapter
        .as_ref()
        .map(|adapter| adapter.timestamp_supported)
        .unwrap_or(false);
    let executed = chain.work.executed_pass_count.unwrap_or(0);
    let timed = chain.work.timed_pass_count.unwrap_or(0);
    let valid = chain.work.valid_pass_count.unwrap_or(0);
    assert!(
        timed <= executed,
        "timed {timed} exceeds executed {executed}"
    );
    assert!(
        valid <= executed,
        "valid {valid} exceeds executed {executed}"
    );
    if timestamps {
        assert_eq!(timed, executed, "every executed pass must be timed");
        assert_eq!(valid, executed, "every executed pass must be valid");
    } else {
        assert_eq!(timed, 0, "no pass may be timed without timestamp support");
    }
    assert_eq!((output.width, output.height), (width, height));
}

#[test]
fn active_fotufilm_engine_reports_its_cpu_stage_reason() {
    // The grain engine resolves on the CPU from the developed film density, so
    // a report must name the stage and the CPU reason that declined the
    // resident route. The crop keeps the readout region a few film
    // millimetres while the pipeline still resolves the engine and the plan.
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.camera.film_format_mm = 36.0;
    params.workflow.route = "input > film > scan".to_owned();
    params.scanner.scan_output = "negative_to_positive".to_owned();
    params.io.scan_film = true;
    params.io.crop = true;
    params.io.crop_size = [0.02, 0.02];
    params.film_render.grain.active = true;
    params.film_render.grain.engine = spektrafilm_core::params::GrainEngine::Fotufilm;
    params.film_render.grain.fotufilm.amount = 1.0;
    let runtime =
        Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", params, &data).unwrap();
    let (width, height) = (512u32, 384u32);
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
            .contains(&("grain_fotufilm".to_owned(), CpuReason::FotufilmFilmCpu)),
        "Fotufilm stage missing from the report: {:?}",
        report.summary().cpu_stage_reasons
    );
}
