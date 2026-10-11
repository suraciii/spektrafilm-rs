use super::*;
use std::sync::mpsc;
use std::time::Duration;

const INPUT: [f32; 4] = [0.25, 1.0, 3.5, 12.0];
const BYTES: u64 = (INPUT.len() * std::mem::size_of::<f32>()) as u64;
const DOUBLING: &str = r#"
@group(0) @binding(0) var<storage, read_write> pixels: array<f32>;
@compute @workgroup_size(4)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < arrayLength(&pixels) { pixels[id.x] *= 2.0; }
}
"#;

// Each invocation requests a fresh adapter, which receives exactly one device request.
// Linux acceptance requires the installed software Vulkan adapter; other platforms
// follow the existing convention of skipping when no compatible adapter is present.
//
// The software Vulkan driver (llvmpipe) is not safe under concurrent device use
// in one process: parallel test threads crash with SIGSEGV (observed in ~4 of 10
// runs at four threads, 0 of 12 serial). The guard is returned first so that
// pattern bindings (dropped in reverse declaration order) destroy the backend
// before releasing DEVICE_IN_USE; every caller holds it for the whole test.
static DEVICE_IN_USE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn fresh_backend(
    features: wgpu::Features,
) -> Option<(std::sync::MutexGuard<'static, ()>, WgpuBackend)> {
    let device_guard = DEVICE_IN_USE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: if cfg!(target_os = "linux") {
            wgpu::Backends::VULKAN
        } else {
            wgpu::Backends::PRIMARY
        },
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: cfg!(target_os = "linux"),
    }));
    if cfg!(target_os = "linux") {
        assert!(
            adapter.is_some(),
            "software Vulkan adapter is required for telemetry acceptance"
        );
    }
    let adapter = adapter?;
    let supported = adapter.features();
    if cfg!(target_os = "linux") {
        assert!(
            supported.contains(features),
            "software Vulkan adapter lacks {features:?}"
        );
    }
    if !supported.contains(features) {
        return None;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("telemetry_test"),
            required_features: features,
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
        },
        None,
    ))
    .expect("fresh telemetry test device");
    let info = adapter.get_info();
    let enabled = features.contains(wgpu::Features::TIMESTAMP_QUERY);
    let reason = (!enabled).then_some(if supported.contains(wgpu::Features::TIMESTAMP_QUERY) {
        AvailabilityReason::NotEnabledOnDevice
    } else {
        AvailabilityReason::Unsupported
    });
    let description = AdapterDescription {
        api: format!("{:?}", info.backend),
        device_type: match info.device_type {
            wgpu::DeviceType::Cpu => AdapterDeviceType::Cpu,
            wgpu::DeviceType::IntegratedGpu => AdapterDeviceType::Integrated,
            wgpu::DeviceType::DiscreteGpu => AdapterDeviceType::Discrete,
            wgpu::DeviceType::VirtualGpu => AdapterDeviceType::Virtual,
            _ => AdapterDeviceType::Other,
        },
        name: info.name,
        description_truncated: false,
        shared_device: true,
        timestamp_supported: supported.contains(wgpu::Features::TIMESTAMP_QUERY),
        timestamp_enabled: enabled,
        timestamp_period_ns: if enabled {
            Measurement::available(queue.get_timestamp_period() as f64)
        } else {
            Measurement::unavailable(reason.unwrap())
        },
        timestamp_reason: reason,
    };
    Some((
        device_guard,
        WgpuBackend {
            device: ObservedDevice {
                raw: device,
                context: ObservationContext::default(),
                batch: None,
            },
            queue: ObservedQueue {
                raw: queue,
                context: ObservationContext::default(),
                batch: None,
            },
            pipeline_cache: Arc::new(PipelineCache::default()),
            name: "telemetry_test".into(),
            adapter: description,
        },
    ))
}

struct Workload {
    pixels: wgpu::Buffer,
    result: wgpu::Buffer,
    pipeline: wgpu::ComputePipeline,
    bindings: wgpu::BindGroup,
    direct: bool,
}

impl Workload {
    fn new(backend: &WgpuBackend, direct: bool, input: &[f32; 4]) -> Self {
        let mut usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        if direct {
            usage |= wgpu::BufferUsages::MAP_READ;
        }
        let pixels = backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grain_encoded_input"),
                contents: bytemuck::cast_slice(input),
                usage,
            });
        let result = if direct {
            pixels.clone()
        } else {
            backend.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("image_readback"),
                size: BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        };
        let shader = backend
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("storage_doubling"),
                source: wgpu::ShaderSource::Wgsl(DOUBLING.into()),
            });
        let pipeline = backend
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("storage_doubling"),
                layout: None,
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let bindings = backend
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("storage_doubling"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pixels.as_entire_binding(),
                }],
            });
        Self {
            pixels,
            result,
            pipeline,
            bindings,
            direct,
        }
    }

    fn encode(&self, backend: &WgpuBackend, count: usize, copy_result: bool) -> ObservedEncoder {
        let mut encoder = backend.device.create_command_encoder(&Default::default());
        for _ in 0..count {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("compute_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bindings, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        if copy_result && !self.direct {
            encoder.copy_buffer_to_buffer(&self.pixels, 0, &self.result, 0, BYTES);
        }
        encoder
    }

    fn materialize(&self, backend: &WgpuBackend) -> Vec<f32> {
        let (tx, rx) = mpsc::channel();
        self.result
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        backend.device.poll(wgpu::Maintain::Wait);
        rx.recv_timeout(Duration::from_secs(10))
            .expect("image map callback")
            .expect("image map succeeded");
        let mapped = self.result.slice(..).get_mapped_range();
        let output = bytemuck::cast_slice::<u8, f32>(&mapped).to_vec();
        backend.device.materialized(BYTES);
        drop(mapped);
        self.result.unmap();
        output
    }
}

fn observed(
    backend: &mut WgpuBackend,
    mode: CollectionMode,
) -> (ObservationContext, ObservationScope) {
    let context = ObservationContext::new(mode);
    context.set_adapter(backend.adapter.clone());
    let stage = context.scope("filming_expose", ObservationKind::Stage, Purpose::Image);
    backend.context_mut(stage.context().clone());
    (context, stage)
}

fn batch(snapshot: &ObservationSnapshot) -> &Observation {
    let batches: Vec<_> = snapshot
        .observations
        .iter()
        .filter(|o| o.kind == ObservationKind::Batch)
        .collect();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].name, "compute");
    let parent = snapshot
        .observations
        .iter()
        .find(|o| o.id == batches[0].parent_id)
        .expect("retained stage parent");
    assert_eq!(parent.kind, ObservationKind::Stage);
    assert_eq!(parent.name, "filming_expose");
    batches[0]
}

fn seconds(measurement: &Measurement) -> f64 {
    assert_eq!(measurement.status, MeasurementStatus::Available);
    assert_eq!(measurement.reason, None);
    let value = measurement.value.expect("available duration");
    assert!(
        value.is_finite() && value >= 0.0,
        "invalid duration {value}"
    );
    value
}

#[test]
fn timestamps_match_pass_sum_and_preserve_real_compute_pixels() {
    let Some((_device, mut backend)) = fresh_backend(wgpu::Features::TIMESTAMP_QUERY) else {
        return;
    };
    let off = Workload::new(&backend, false, &INPUT);
    backend
        .queue
        .submit([off.encode(&backend, 1, true).finish()]);
    let baseline = off.materialize(&backend);
    assert_eq!(baseline, INPUT.map(|pixel| pixel * 2.0));

    let (context, stage) = observed(&mut backend, CollectionMode::GpuTiming);
    let (scoped, guard) = backend.observed_batch("compute");
    let on = Workload::new(&scoped, false, &INPUT);
    scoped.queue.submit([on.encode(&scoped, 1, true).finish()]);
    assert_eq!(on.materialize(&scoped), baseline);
    drop(guard);
    drop(stage);
    let snapshot = context.snapshot();
    let batch = batch(&snapshot);
    let measurements = batch.batch.as_ref().expect("batch measurements");
    assert_eq!(measurements.completion_boundary, "image_map_ready");
    seconds(&measurements.host_prepare);
    seconds(&measurements.submit_to_map_ready);
    seconds(&measurements.host_materialize);
    let passes: Vec<_> = snapshot
        .observations
        .iter()
        .filter(|o| o.kind == ObservationKind::Pass)
        .collect();
    assert_eq!(passes.len(), 1);
    assert_eq!(passes[0].parent_id, batch.id);
    assert_eq!(passes[0].purpose, Purpose::Image);
    let pass = passes[0].pass.as_ref().expect("timestamp pass");
    assert_eq!(pass.occurrence, 0);
    let duration = seconds(&pass.duration);
    assert_eq!(seconds(&measurements.compute_pass_sum), duration);
    assert_eq!(seconds(&measurements.partial_compute_pass_sum), duration);
    assert_eq!(seconds(&snapshot.gpu_timing.compute_pass_sum), duration);
    let counters = &snapshot.totals;
    assert_eq!(counters.dispatch_count, Some(1));
    assert_eq!(counters.executed_pass_count, Some(1));
    assert_eq!(counters.timed_pass_count, Some(1));
    assert_eq!(counters.valid_pass_count, Some(1));
    assert_eq!(counters.omitted_pass_count, Some(0));
    assert_eq!(counters.submit_count, Some(1));
    assert_eq!(counters.host_wait_count, Some(1));
    assert_eq!(counters.telemetry_buffer_create_count, Some(2));
    assert_eq!(counters.telemetry_resolve_count, Some(1));
    assert_eq!(counters.telemetry_copy_count, Some(1));
    assert_eq!(counters.telemetry_copy_bytes, Some(16));
    assert_eq!(counters.staging_copy_bytes, Some(BYTES));
    assert_eq!(counters.readback_bytes, Some(BYTES));
}

#[test]
fn staging_and_direct_mapping_count_the_same_logical_result_once() {
    for direct in [false, true] {
        let features = if direct {
            wgpu::Features::MAPPABLE_PRIMARY_BUFFERS
        } else {
            wgpu::Features::empty()
        };
        let Some((_device, mut backend)) = fresh_backend(features) else {
            continue;
        };
        let (context, stage) = observed(&mut backend, CollectionMode::Summary);
        let (scoped, guard) = backend.observed_batch("compute");
        let work = Workload::new(&scoped, direct, &INPUT);
        scoped
            .queue
            .submit([work.encode(&scoped, 1, true).finish()]);
        assert_eq!(work.materialize(&scoped), INPUT.map(|pixel| pixel * 2.0));
        drop(guard);
        drop(stage);
        let snapshot = context.snapshot();
        assert_eq!(snapshot.totals.image_upload_count, Some(1));
        assert_eq!(snapshot.totals.image_upload_bytes, Some(BYTES));
        assert_eq!(snapshot.totals.readback_count, Some(1));
        assert_eq!(snapshot.totals.readback_bytes, Some(BYTES));
        assert_eq!(snapshot.totals.staging_copy_count, Some(u64::from(!direct)));
        assert_eq!(
            snapshot.totals.staging_copy_bytes,
            Some(if direct { 0 } else { BYTES })
        );
        assert_eq!(snapshot.totals.device_copy_count, Some(0));
        assert_eq!(snapshot.totals.telemetry_buffer_create_count, Some(0));
        assert_eq!(snapshot.totals.host_wait_count, Some(1));
        assert_eq!(
            batch(&snapshot).batch.as_ref().unwrap().compute_pass_sum,
            Measurement::unavailable(AvailabilityReason::NotCollected)
        );
    }
}

#[test]
fn fresh_baseline_device_reports_disabled_timestamps_without_changing_image_work() {
    let Some((_device, mut backend)) = fresh_backend(wgpu::Features::empty()) else {
        return;
    };
    assert!(
        !backend
            .device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
    );
    let expected_reason = backend.adapter.timestamp_reason.unwrap();
    let (context, stage) = observed(&mut backend, CollectionMode::GpuTiming);
    let (scoped, guard) = backend.observed_batch("compute");
    let work = Workload::new(&scoped, false, &INPUT);
    scoped
        .queue
        .submit([work.encode(&scoped, 1, true).finish()]);
    assert_eq!(work.materialize(&scoped), INPUT.map(|pixel| pixel * 2.0));
    drop(guard);
    drop(stage);
    let snapshot = context.snapshot();
    assert_eq!(
        batch(&snapshot).batch.as_ref().unwrap().compute_pass_sum,
        Measurement::unavailable(expected_reason)
    );
    let pass = snapshot
        .observations
        .iter()
        .find(|o| o.kind == ObservationKind::Pass)
        .unwrap();
    assert_eq!(
        pass.pass.as_ref().unwrap().duration,
        Measurement::unavailable(expected_reason)
    );
    assert_eq!(snapshot.totals.executed_pass_count, Some(1));
    assert_eq!(snapshot.totals.timed_pass_count, Some(0));
    assert_eq!(snapshot.totals.telemetry_buffer_create_count, Some(0));
    assert_eq!(snapshot.totals.telemetry_resolve_count, Some(0));
    assert_eq!(snapshot.totals.telemetry_copy_bytes, Some(0));
    assert_eq!(snapshot.totals.readback_count, Some(1));
}

#[test]
fn failed_and_pending_query_callbacks_preserve_completed_image() {
    for reason in [
        AvailabilityReason::QueryFailed,
        AvailabilityReason::NotReady,
    ] {
        let Some((_device, mut backend)) = fresh_backend(wgpu::Features::TIMESTAMP_QUERY) else {
            return;
        };
        let (context, stage) = observed(&mut backend, CollectionMode::GpuTiming);
        let (scoped, guard) = backend.observed_batch("compute");
        let work = Workload::new(&scoped, false, &INPUT);
        scoped
            .queue
            .submit([work.encode(&scoped, 1, true).finish()]);
        let image = work.materialize(&scoped);
        let (tx, rx) = mpsc::channel();
        if reason == AvailabilityReason::QueryFailed {
            tx.send(Err(wgpu::BufferAsyncError)).unwrap();
        }
        scoped
            .device
            .batch
            .as_ref()
            .unwrap()
            .lock()
            .queries
            .as_mut()
            .unwrap()
            .receiver = Some(rx);
        // Keeping tx alive makes NotReady an empty live callback channel, not a disconnection.
        drop(guard);
        drop(tx);
        drop(stage);
        assert_eq!(image, INPUT.map(|pixel| pixel * 2.0));
        let snapshot = context.snapshot();
        let measurements = batch(&snapshot).batch.as_ref().unwrap();
        assert_eq!(
            measurements.compute_pass_sum,
            Measurement::unavailable(reason)
        );
        assert_eq!(
            measurements.partial_compute_pass_sum,
            Measurement::unavailable(reason)
        );
        assert_eq!(
            snapshot.gpu_timing.partial_compute_pass_sum,
            Measurement::unavailable(reason)
        );
        assert_eq!(
            snapshot.gpu_timing.compute_pass_sum,
            Measurement::unavailable(reason)
        );
        let pass = snapshot
            .observations
            .iter()
            .find(|o| o.kind == ObservationKind::Pass)
            .unwrap();
        assert_eq!(
            pass.pass
                .as_ref()
                .expect("unavailable pass observation")
                .duration,
            Measurement::unavailable(reason)
        );
        assert_eq!(snapshot.totals.valid_pass_count, Some(0));
        assert_eq!(snapshot.totals.readback_count, Some(1));
        assert_eq!(snapshot.totals.host_wait_count, Some(1));
        assert_eq!(snapshot.totals.submit_count, Some(1));
    }
}

#[test]
fn discarded_and_finished_unsubmitted_commands_contribute_no_executed_work() {
    let Some((_device, mut backend)) = fresh_backend(wgpu::Features::TIMESTAMP_QUERY) else {
        return;
    };
    let (context, stage) = observed(&mut backend, CollectionMode::GpuTiming);
    let (scoped, guard) = backend.observed_batch("compute");
    let work = Workload::new(&scoped, false, &INPUT);
    let scratch = scoped.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("discarded_copy"),
        size: BYTES,
        usage: wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut discarded = work.encode(&scoped, 2, true);
    discarded.copy_buffer_to_buffer(&work.pixels, 0, &scratch, 0, BYTES);
    drop(discarded);
    let mut unsubmitted = work.encode(&scoped, 3, true);
    unsubmitted.copy_buffer_to_buffer(&work.pixels, 0, &scratch, 0, BYTES);
    drop(unsubmitted.finish());
    drop(guard);
    drop(stage);
    let snapshot = context.snapshot();
    assert!(
        snapshot
            .observations
            .iter()
            .all(|o| o.kind != ObservationKind::Pass)
    );
    assert_eq!(snapshot.coverage.omitted_observations, 0);
    let counters = &snapshot.totals;
    assert_eq!(counters.dispatch_count, Some(0));
    assert_eq!(counters.executed_pass_count, Some(0));
    assert_eq!(counters.timed_pass_count, Some(0));
    assert_eq!(counters.valid_pass_count, Some(0));
    assert_eq!(counters.submit_count, Some(0));
    assert_eq!(counters.command_buffer_count, Some(0));
    assert_eq!(counters.staging_copy_count, Some(0));
    assert_eq!(counters.staging_copy_bytes, Some(0));
    assert_eq!(counters.device_copy_count, Some(0));
    assert_eq!(counters.device_copy_bytes, Some(0));
    assert_eq!(counters.telemetry_resolve_count, Some(0));
    assert_eq!(counters.telemetry_copy_count, Some(0));
    assert_eq!(counters.telemetry_copy_bytes, Some(0));
    assert_eq!(counters.readback_count, Some(0));
    assert_eq!(counters.host_wait_count, Some(0));
    assert_eq!(
        batch(&snapshot).batch.as_ref().unwrap().compute_pass_sum,
        Measurement::unavailable(AvailabilityReason::Incomplete)
    );
}

#[test]
fn discarded_detail_reservations_leave_submitted_passes_and_pixels_intact() {
    let Some((_device, mut backend)) = fresh_backend(wgpu::Features::TIMESTAMP_QUERY) else {
        return;
    };
    let (context, stage) = observed(&mut backend, CollectionMode::GpuTiming);
    let (scoped, guard) = backend.observed_batch("compute");
    let work = Workload::new(&scoped, false, &INPUT);
    drop(work.encode(&scoped, 1030, false));
    drop(work.encode(&scoped, 1030, false).finish());
    assert!(
        context
            .snapshot()
            .observations
            .iter()
            .all(|o| o.kind != ObservationKind::Pass)
    );
    let discarded = work.encode(&scoped, 1, false).finish();
    let first = work.encode(&scoped, 1, false);
    let second = work.encode(&scoped, 1, true).finish();
    drop(discarded);
    scoped.queue.submit([first.finish(), second]);
    assert_eq!(work.materialize(&scoped), INPUT.map(|pixel| pixel * 4.0));
    drop(guard);
    drop(stage);
    let snapshot = context.snapshot();
    let parent = batch(&snapshot);
    let passes: Vec<_> = snapshot
        .observations
        .iter()
        .filter(|o| o.kind == ObservationKind::Pass)
        .collect();
    assert_eq!(passes.len(), 2);
    for (index, pass) in passes.iter().enumerate() {
        assert_eq!(pass.parent_id, parent.id);
        assert_eq!(pass.pass.as_ref().unwrap().occurrence, index as u64);
        seconds(&pass.pass.as_ref().unwrap().duration);
    }
    assert_eq!(snapshot.coverage.omitted_observations, 0);
    assert_eq!(snapshot.totals.executed_pass_count, Some(2));
    assert_eq!(snapshot.totals.timed_pass_count, Some(2));
    assert_eq!(snapshot.totals.valid_pass_count, Some(2));
}

#[test]
fn exhausted_details_report_detail_limit_on_timestamp_enabled_device() {
    let Some((_device, mut backend)) = fresh_backend(wgpu::Features::TIMESTAMP_QUERY) else {
        return;
    };
    let (context, stage) = observed(&mut backend, CollectionMode::GpuTiming);
    for _ in 0..1023 {
        drop(
            stage
                .context()
                .scope("filled", ObservationKind::Stage, Purpose::Image),
        );
    }
    let (scoped, guard) = backend.observed_batch("compute");
    let work = Workload::new(&scoped, false, &INPUT);
    scoped
        .queue
        .submit([work.encode(&scoped, 1, true).finish()]);
    assert_eq!(work.materialize(&scoped), INPUT.map(|pixel| pixel * 2.0));
    drop(guard);
    drop(stage);
    let snapshot = context.snapshot();
    assert_eq!(
        snapshot.gpu_timing.compute_pass_sum,
        Measurement::unavailable(AvailabilityReason::DetailLimit)
    );
    assert_eq!(
        snapshot.gpu_timing.partial_compute_pass_sum,
        Measurement::unavailable(AvailabilityReason::DetailLimit)
    );
    assert_eq!(snapshot.totals.executed_pass_count, Some(1));
    assert_eq!(snapshot.totals.timed_pass_count, Some(0));
    assert_eq!(snapshot.totals.valid_pass_count, Some(0));
}

#[test]
fn one_submission_of_multiple_command_buffers_has_exact_work_sum() {
    let Some((_device, mut backend)) = fresh_backend(wgpu::Features::TIMESTAMP_QUERY) else {
        return;
    };
    let (context, stage) = observed(&mut backend, CollectionMode::GpuTiming);
    let (scoped, guard) = backend.observed_batch("compute");
    let work = Workload::new(&scoped, false, &INPUT);
    let scratch = scoped.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("scratch_copy"),
        size: BYTES,
        usage: wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut first = work.encode(&scoped, 1, false);
    first.copy_buffer_to_buffer(&work.pixels, 0, &scratch, 0, BYTES);
    let second = work.encode(&scoped, 2, true);
    scoped.queue.submit([first.finish(), second.finish()]);
    assert_eq!(work.materialize(&scoped), INPUT.map(|pixel| pixel * 8.0));
    drop(guard);
    drop(stage);
    let snapshot = context.snapshot();
    let counters = &snapshot.totals;
    assert_eq!(counters.submit_count, Some(1));
    assert_eq!(counters.command_buffer_count, Some(2));
    assert_eq!(counters.dispatch_count, Some(3));
    assert_eq!(counters.executed_pass_count, Some(3));
    assert_eq!(counters.timed_pass_count, Some(3));
    assert_eq!(counters.valid_pass_count, Some(3));
    assert_eq!(counters.omitted_pass_count, Some(0));
    assert_eq!(counters.device_copy_count, Some(1));
    assert_eq!(counters.device_copy_bytes, Some(BYTES));
    assert_eq!(counters.staging_copy_count, Some(1));
    assert_eq!(counters.staging_copy_bytes, Some(BYTES));
    assert_eq!(counters.readback_count, Some(1));
    assert_eq!(counters.telemetry_resolve_count, Some(2));
    assert_eq!(counters.telemetry_copy_count, Some(2));
    assert_eq!(counters.telemetry_copy_bytes, Some(16 + 48));
    let parent = batch(&snapshot);
    let passes: Vec<_> = snapshot
        .observations
        .iter()
        .filter(|o| o.kind == ObservationKind::Pass)
        .collect();
    assert_eq!(passes.len(), 3);
    let mut sum = 0.0;
    for (index, observation) in passes.iter().enumerate() {
        assert_eq!(observation.parent_id, parent.id);
        let pass = observation.pass.as_ref().unwrap();
        assert_eq!(pass.occurrence, index as u64);
        sum += seconds(&pass.duration);
    }
    assert_eq!(
        seconds(&parent.batch.as_ref().unwrap().compute_pass_sum),
        sum
    );
}

#[test]
fn more_than_1024_submitted_passes_bound_detail_and_keep_exact_counters() {
    const PASSES: usize = 1030;
    let Some((_device, mut backend)) = fresh_backend(wgpu::Features::TIMESTAMP_QUERY) else {
        return;
    };
    let (context, stage) = observed(&mut backend, CollectionMode::GpuTiming);
    let (scoped, guard) = backend.observed_batch("compute");
    let work = Workload::new(&scoped, false, &[0.0; 4]);
    scoped
        .queue
        .submit([work.encode(&scoped, PASSES, true).finish()]);
    assert_eq!(work.materialize(&scoped), [0.0; 4]);
    drop(guard);
    drop(stage);
    let snapshot = context.snapshot();
    assert_eq!(snapshot.observations.len(), 1024);
    assert_eq!(snapshot.coverage.retained_observations, 1024);
    assert_eq!(
        snapshot.coverage.omitted_observations,
        (PASSES + 2 - 1024) as u64
    );
    assert!(snapshot.coverage.truncated);
    assert!(snapshot.coverage.retained_bytes <= snapshot.coverage.byte_limit);
    let counters = &snapshot.totals;
    assert_eq!(counters.executed_pass_count, Some(PASSES as u64));
    assert_eq!(counters.dispatch_count, Some(PASSES as u64));
    assert_eq!(counters.timed_pass_count, Some(1022));
    assert_eq!(counters.valid_pass_count, Some(1022));
    assert_eq!(counters.omitted_pass_count, Some((PASSES - 1022) as u64));
    assert_eq!(counters.command_buffer_count, Some(1));
    assert_eq!(counters.submit_count, Some(1));
    assert_eq!(counters.host_wait_count, Some(1));
    assert_eq!(counters.readback_count, Some(1));
    assert_eq!(counters.telemetry_copy_bytes, Some(1022 * 16));
    let parent = batch(&snapshot);
    let passes: Vec<_> = snapshot
        .observations
        .iter()
        .filter(|o| o.kind == ObservationKind::Pass)
        .collect();
    assert_eq!(passes.len(), 1022);
    for (index, observation) in passes.iter().enumerate() {
        assert_eq!(observation.parent_id, parent.id);
        let pass = observation.pass.as_ref().unwrap();
        assert_eq!(pass.occurrence, index as u64);
        seconds(&pass.duration);
    }
    let measurements = parent.batch.as_ref().unwrap();
    assert_eq!(
        measurements.compute_pass_sum,
        Measurement::unavailable(AvailabilityReason::Incomplete)
    );
    let sum: f64 = passes
        .iter()
        .map(|o| seconds(&o.pass.as_ref().unwrap().duration))
        .sum();
    assert_eq!(seconds(&measurements.partial_compute_pass_sum), sum);
}
