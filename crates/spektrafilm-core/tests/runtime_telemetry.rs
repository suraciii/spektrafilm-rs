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
    params.scanner.operator = spektrafilm_core::params::ScanOperatorParams {
        exposure_ev: 0.5000000000000001,
        warmth_ev: -0.25,
        tint_ev: 0.125,
        contrast: 1.2,
    };
    let operator = params.scanner.operator;
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
    historical["configuration"]["render"]
        .as_object_mut()
        .unwrap()
        .remove("scan_operator");
    let old = spektrafilm_core::telemetry::Report::from_json(&historical.to_string()).unwrap();
    let old_render = old.configuration.render.unwrap();
    assert!(old_render.scan_operator.is_default());
    assert!(
        old_render.scan_model.readout_illuminant.is_empty(),
        "historical reports must not invent a material-category readout fact"
    );
    let decoded =
        spektrafilm_core::telemetry::Report::from_json(&report.to_json().unwrap()).unwrap();
    let actual = decoded.configuration.render.unwrap();
    assert_eq!(actual.scan_operator, operator);
    let mut invalid: serde_json::Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    invalid["configuration"]["render"]["scan_operator"]["contrast"] = 0.into();
    assert!(spektrafilm_core::telemetry::Report::from_json(&invalid.to_string()).is_err());
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
    let runtime = Runtime::new_with_backend(
        spektrafilm_core::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap(),
        spektrafilm_core::profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap(),
        params,
        &data,
        gpu.backend(),
    )
    .unwrap();
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
fn retained_input_route_reports_actual_colorspace_gpu_work() {
    use spektrafilm_backend::telemetry::{BackendSelected, Executor};
    use spektrafilm_core::telemetry::BackendRequested;

    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.workflow.route = "input".into();
    params.camera.auto_exposure = false;
    params.film_render.grain.active = false;
    params.cmyk.active = false;
    params.io.crop = false;
    params.io.upscale_factor = 1.0;
    params.io.input_color_space = "Display P3".into();
    params.io.output_color_space = "sRGB".into();
    params.io.input_cctf_decoding = true;
    params.io.output_cctf_encoding = true;
    params.io.output_gamut_compress.algorithm = "off".into();
    let reference = Runtime::from_stocks(
        "kodak_portra_400",
        "kodak_portra_endura",
        params.clone(),
        &data,
    )
    .unwrap();
    let runtime = Runtime::new_with_backend(
        spektrafilm_core::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap(),
        spektrafilm_core::profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap(),
        params,
        &data,
        gpu.backend(),
    )
    .unwrap();
    let (width, height) = (7, 5);
    let image = ImageBuf::from_data(
        width,
        height,
        (0..width * height * 3)
            .map(|i| {
                from_f64(match i {
                    0 => -0.05,
                    1 => 0.4,
                    2 => 1.2,
                    _ => 0.02 + 0.96 * ((i * 37) % 101) as f64 / 100.0,
                })
            })
            .collect(),
    );
    let expected = reference.process(image.clone(), &CpuBackend).unwrap();
    assert!(expected.data.iter().any(|sample| *sample < from_f64(0.0)));
    assert!(expected.data.iter().any(|sample| *sample > from_f64(1.0)));
    assert!(
        expected
            .data
            .iter()
            .zip(&image.data)
            .any(|(converted, source)| (*converted as f64 - *source as f64).abs() > 1e-3),
        "the fixture must exercise a nonidentity color conversion"
    );
    let unobserved = runtime.process(image.clone(), gpu.backend()).unwrap();
    let mut operation = Operation::new(OperationKind::Process, CollectionMode::Summary);
    operation.set_backend_requested(BackendRequested::Wgpu);
    operation.set_render_configuration(runtime.telemetry_configuration());
    operation.set_input_dimensions(width, height);
    let attempt = operation.attempt(1);
    let simulation = attempt
        .context()
        .scope("simulation", ObservationKind::Phase, Purpose::Image);
    let actual = runtime
        .process_observed(image, gpu.backend(), simulation.context())
        .unwrap();
    drop(simulation);
    attempt.finish(Outcome::Succeeded);
    let report = operation.finish(Outcome::Succeeded).unwrap();
    report.validate().unwrap();

    assert_eq!((actual.width, actual.height), (width, height));
    assert_eq!(
        actual.data, unobserved.data,
        "observation changed the device pixels"
    );
    assert_eq!(actual.data.len(), expected.data.len());
    for (index, (actual, expected)) in actual.data.iter().zip(&expected.data).enumerate() {
        assert!(
            (*actual as f64 - *expected as f64).abs() < 2e-5,
            "input conversion sample {index}: GPU {actual}, CPU {expected}"
        );
    }
    assert_eq!(
        report.execution.backend_selected,
        Some(BackendSelected::Wgpu)
    );
    assert_eq!(
        report.execution.adapter,
        gpu.backend().adapter_description()
    );
    assert_eq!(report.execution.path, Some(ExecutionPath::GpuResident));
    assert!(report.execution.resident_decline_reasons.is_empty());
    let stages: Vec<_> = report
        .stages
        .iter()
        .filter(|stage| stage.name == "colorspace_convert")
        .collect();
    assert_eq!(
        stages.len(),
        1,
        "the retained conversion must own one canonical stage"
    );
    let stage = stages[0];
    assert_eq!(stage.purpose, Purpose::Image);
    assert_eq!(stage.executor, Some(Executor::Gpu));
    assert!(stage.complete);
    assert!(stage.cpu_reason.is_none());
    assert_eq!(
        stage.work.dispatch_count,
        Some(3),
        "decode, matrix, and encode must execute"
    );
    assert_eq!(stage.work.submit_count, Some(3));
    assert_eq!(stage.work.image_upload_count.unwrap_or(0), 0);
    assert_eq!(stage.work.readback_count.unwrap_or(0), 0);
    assert!(
        !report
            .stages
            .iter()
            .any(|stage| stage.purpose == Purpose::Image
                && matches!(stage.executor, Some(Executor::Cpu | Executor::Mixed))),
        "the selected GPU input route must not bridge through CPU image arithmetic"
    );
    assert_eq!(report.measurements.work.image_upload_count, Some(1));
    assert_eq!(report.measurements.work.readback_count, Some(1));
    assert_eq!(report.coverage.omitted_high_level_observations, Some(0));
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
    let runtime = Runtime::new_with_backend(
        spektrafilm_core::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap(),
        spektrafilm_core::profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap(),
        params,
        &data,
        gpu.backend(),
    )
    .unwrap();
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

#[test]
fn cpu_prepared_public_runtimes_reject_selected_gpu_before_any_upload() {
    use spektrafilm_backend::telemetry::ObservationContext;
    use spektrafilm_backend::{Precision, RetainedImageMeta, SampleDomain};
    use spektrafilm_core::pipeline::RetainedRender;
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.film_render.grain.active = false;
    params.io.output_gamut_compress.algorithm = "off".into();
    let film = spektrafilm_core::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
    let print =
        spektrafilm_core::profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
    let runtimes = [
        Runtime::new(film, print, params.clone(), &data).unwrap(),
        Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", params, &data).unwrap(),
    ];
    let image = ImageBuf::from_data(2, 2, vec![from_f64(0.18); 12]);
    // Source upload is deliberately outside the measured failure operations.
    let source = gpu
        .backend()
        .retain_image(
            &image,
            &RetainedImageMeta {
                width: 2,
                height: 2,
                domain: SampleDomain::Encoded,
                precision: Precision::host(),
                space: &spektrafilm_math::colorspace::SRGB,
                pixel_pitch_um: None,
            },
        )
        .unwrap();
    for runtime in runtimes {
        let context = ObservationContext::new(CollectionMode::Summary);
        let observed = spektrafilm_backend::bind_backend(gpu.backend(), context.clone());
        let errors = [
            runtime
                .process(image.clone(), observed.as_ref())
                .unwrap_err(),
            runtime
                .process_retained(&image, observed.as_ref())
                .unwrap_err(),
            runtime
                .process_preview(image.clone(), 1, observed.as_ref())
                .unwrap_err(),
            runtime
                .process_preview_retained(&image, 1, observed.as_ref())
                .unwrap_err(),
            runtime
                .process_device_retained(&source, RetainedRender::FULL, observed.as_ref())
                .unwrap_err(),
            runtime
                .process_with_taps(
                    image.clone(),
                    observed.as_ref(),
                    Some(spektrafilm_core::params::Tap::RgbIn),
                    Some(spektrafilm_core::params::Tap::RgbPre),
                )
                .unwrap_err(),
        ];
        for error in errors {
            assert!(
                error.contains("device-prepared spectral calibration"),
                "{error}"
            );
            assert!(error.contains("Runtime::new_with_backend"), "{error}");
        }
        let totals = context.snapshot().totals;
        assert_eq!(
            totals.upload_count.unwrap_or(0),
            0,
            "CPU-prepared calibration was uploaded"
        );
        assert_eq!(
            totals.dispatch_count.unwrap_or(0),
            0,
            "failure must precede numerical work"
        );
        assert_eq!(totals.readback_count.unwrap_or(0), 0);
    }
}

#[test]
fn device_prepared_runtime_rejects_another_device_before_upload() {
    use spektrafilm_backend::telemetry::ObservationContext;
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = false;
    params.film_render.grain.active = false;
    params.io.output_gamut_compress.algorithm = "off".into();
    let runtime = {
        let first = spektrafilm_gpu::test_support::explicit_adapter();
        Runtime::new_with_backend(
            spektrafilm_core::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap(),
            spektrafilm_core::profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap(),
            params,
            &data,
            first.backend(),
        )
        .unwrap()
    };
    let selected = spektrafilm_gpu::test_support::explicit_adapter();
    let context = ObservationContext::new(CollectionMode::Summary);
    let observed = spektrafilm_backend::bind_backend(selected.backend(), context.clone());
    let image = ImageBuf::from_data(2, 2, vec![from_f64(0.18); 12]);
    let error = runtime
        .process_retained(&image, observed.as_ref())
        .unwrap_err();
    assert!(error.contains("belongs to another device"), "{error}");
    assert!(error.contains("with_params_with_backend"), "{error}");
    let totals = context.snapshot().totals;
    assert_eq!(totals.upload_count.unwrap_or(0), 0);
    assert_eq!(totals.dispatch_count.unwrap_or(0), 0);
    // The consuming migration invalidates device-specific preparation rather
    // than reusing the first device's table or uploading a host equivalent.
    let params = runtime.params().clone();
    let migrated = runtime
        .with_params_with_backend(params, selected.backend())
        .unwrap();
    assert!(
        migrated
            .process_retained(&image, selected.backend())
            .unwrap()
            .is_some()
    );
}
