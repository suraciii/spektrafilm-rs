use super::*;
use spektrafilm_backend::telemetry::{
    AvailabilityReason, CollectionMode, Measurement, ObservationContext, ObservationKind, Purpose,
};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

mod logic_tests {
    use super::*;

    #[test]
    fn full_resolution_gui_peak_above_overlap_target_is_admitted_exclusively() {
        // Eighteen full-resolution RGB rasters are a scheduling request, not
        // an allocation/VRAM claim. No raster is allocated for this test.
        let planned_peak = 18u64 * 6342 * 9543 * 12;
        let mut state = AdmissionState::default();
        assert_eq!(
            state
                .decide(planned_peak, 0, OVERLAP_TARGET_BYTES, false, false)
                .unwrap(),
            AdmissionDecision::RetirePrior
        );
        assert_eq!(
            state
                .decide(planned_peak, 0, OVERLAP_TARGET_BYTES, false, true)
                .unwrap(),
            AdmissionDecision::Admit { exclusive: true }
        );
        state.reserve(planned_peak, true).unwrap();
        assert_eq!(
            (state.bytes, state.owners, state.exclusive),
            (planned_peak, 1, true)
        );
        state.release(planned_peak, true);
        assert_eq!((state.bytes, state.owners, state.exclusive), (0, 0, false));
    }

    #[test]
    fn finite_peak_is_not_rejected_but_footprint_arithmetic_overflow_is() {
        let mut state = AdmissionState::default();
        assert!(
            state
                .decide(u64::MAX, 1, OVERLAP_TARGET_BYTES, false, false)
                .is_err()
        );
        assert_eq!(
            state
                .decide(u64::MAX, 0, OVERLAP_TARGET_BYTES, false, false)
                .unwrap(),
            AdmissionDecision::RetirePrior
        );
        assert_eq!(
            state
                .decide(u64::MAX, 0, OVERLAP_TARGET_BYTES, false, true)
                .unwrap(),
            AdmissionDecision::Admit { exclusive: true }
        );
        state.reserve(u64::MAX, true).unwrap();
        assert_eq!(
            (state.bytes, state.owners, state.exclusive),
            (u64::MAX, 1, true)
        );
        state.release(u64::MAX, true);
        assert_eq!((state.bytes, state.owners, state.exclusive), (0, 0, false));
    }

    #[test]
    fn overlap_counts_retained_and_reserved_capacity_at_the_exact_target() {
        let mut state = AdmissionState {
            bytes: 30,
            owners: 1,
            ..Default::default()
        };
        assert_eq!(
            state.decide(50, 20, 100, false, false).unwrap(),
            AdmissionDecision::Admit { exclusive: false }
        );
        assert_eq!(
            state.decide(51, 20, 100, false, false).unwrap(),
            AdmissionDecision::Wait { exclusive: false }
        );
        state.reserve(50, false).unwrap();
        assert_eq!((state.bytes, state.owners), (80, 2));
        state.release(50, false);
        assert_eq!((state.bytes, state.owners), (30, 1));
    }

    #[test]
    fn exclusive_owners_and_waiters_prevent_overlap_bypass() {
        let mut state = AdmissionState {
            bytes: 60,
            owners: 1,
            exclusive_waiters: 1,
            ..Default::default()
        };
        assert_eq!(
            state.decide(20, 0, 100, false, false).unwrap(),
            AdmissionDecision::Wait { exclusive: false }
        );
        state.exclusive_waiters = 0;
        assert_eq!(
            state.decide(20, 0, 100, false, false).unwrap(),
            AdmissionDecision::Admit { exclusive: false }
        );
        state.exclusive = true;
        assert_eq!(
            state.decide(20, 0, 100, false, false).unwrap(),
            AdmissionDecision::Wait { exclusive: false }
        );
        assert_eq!(
            state.decide(150, 0, 100, false, true).unwrap(),
            AdmissionDecision::Wait { exclusive: true }
        );
    }

    #[test]
    fn exclusive_intent_survives_falling_footprint_and_requires_prior_retirement() {
        let mut state = AdmissionState::default();
        assert_eq!(
            state.decide(20, 0, 100, true, false).unwrap(),
            AdmissionDecision::RetirePrior
        );
        assert_eq!(
            state.decide(20, 0, 100, true, true).unwrap(),
            AdmissionDecision::Admit { exclusive: true }
        );
        state.reserve(20, true).unwrap();
        state.release(20, true);
        assert_eq!(
            state.decide(20, 0, 100, false, false).unwrap(),
            AdmissionDecision::Admit { exclusive: false }
        );
    }

    #[test]
    fn overlap_overflow_waits_and_failed_reservations_leave_the_ledger_unchanged() {
        let mut state = AdmissionState {
            bytes: 1,
            owners: 1,
            ..Default::default()
        };
        assert_eq!(
            state.decide(u64::MAX, 0, u64::MAX, false, false).unwrap(),
            AdmissionDecision::Wait { exclusive: false }
        );
        assert!(state.reserve(u64::MAX, true).is_err());
        assert_eq!((state.bytes, state.owners, state.exclusive), (1, 1, false));
        state.owners = usize::MAX;
        assert!(state.reserve(1, true).is_err());
        assert_eq!(
            (state.bytes, state.owners, state.exclusive),
            (1, usize::MAX, false)
        );
    }
}

fn await_admission_waiters(resources: &DeviceResources, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut state = resources.reserved.lock();
    while state.waiters < expected {
        assert!(
            Instant::now() < deadline,
            "admission did not enter its owner-credit wait"
        );
        resources
            .released
            .wait_for(&mut state, Duration::from_millis(5));
    }
}

fn copy(backend: &WgpuBackend, source: &wgpu::Buffer, destination: &wgpu::Buffer) {
    let mut encoder = backend.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, destination, 0, source.size());
    backend.queue.submit(Some(encoder.finish()));
}

fn buffers(backend: &WgpuBackend) -> (wgpu::Buffer, wgpu::Buffer) {
    let source = backend
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("resource_test_source"),
            contents: bytemuck::cast_slice(&[17u32, 29, 43, 71]),
            usage: wgpu::BufferUsages::COPY_SRC,
        });
    let output = backend.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("resource_test_output"),
        size: source.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    (source, output)
}

#[test]
fn cancellation_keeps_credit_until_last_derivative_retires_submitted_work() {
    let gpu = crate::test_support::adapter();
    let operation = gpu.admit_operation(1024).unwrap();
    let mut backend = operation.backend(&gpu);
    let cancelled = Arc::new(AtomicBool::new(false));
    backend.cancellation = Some(cancelled.clone());
    backend.pipe = backend.pipe.with_cancellation(Some(cancelled.clone()));
    let (source, output) = buffers(&backend);
    copy(&backend, &source, &output);
    let (mapped, ready) = mpsc::channel();
    let resources = gpu.device.resources.clone();
    output
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            // Completion callbacks must be able to inspect the shared ledger; the
            // retiring thread must hold neither submission nor admission locks.
            let _pending = resources.submissions.lock().pending.len();
            let _reserved = resources.reserved.lock().bytes;
            let _ = mapped.send(result);
        });
    cancelled.store(true, Ordering::Release);
    assert!(backend.check_cancelled().is_err());
    let escaped = backend.derived_explicit();
    drop(operation);
    drop(backend);
    assert_eq!(
        gpu.device.resources.reserved.lock().bytes,
        1024,
        "an escaped derivative still owns the cancelled submission"
    );
    drop(escaped);
    assert_eq!(gpu.device.resources.reserved.lock().bytes, 0);
    ready
        .try_recv()
        .expect("cancellation must not abandon retirement")
        .unwrap();
    let values = output.slice(..).get_mapped_range();
    assert_eq!(bytemuck::cast_slice::<u8, u32>(&values), &[17, 29, 43, 71]);
    drop(values);
    output.unmap();
}

#[test]
fn error_unwind_retires_work_before_returning_credit() {
    let gpu = crate::test_support::adapter();
    let (source, output) = buffers(&gpu);
    let (completed, ready) = mpsc::channel();
    let result = (|| -> Result<(), String> {
        let operation = gpu.admit_operation(1024)?;
        let backend = operation.backend(&gpu);
        copy(&backend, &source, &output);
        output
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = completed.send(result);
            });
        Err("failure after submission".into())
    })();
    assert_eq!(result.unwrap_err(), "failure after submission");
    assert_eq!(gpu.device.resources.reserved.lock().bytes, 0);
    ready
        .try_recv()
        .expect("error must retire the submitted owner")
        .unwrap();
    output.unmap();
}

#[test]
fn concurrent_admission_is_atomic_and_derivatives_share_one_queue_bound() {
    let gpu = crate::test_support::adapter();
    let mut resources = DeviceResources::new(gpu.device.raw.clone(), false);
    resources.overlap_target = 100;
    let resources = Arc::new(resources);
    let first = resources
        .admit(60, 0, None, &ObservationContext::default())
        .unwrap();
    let (acquired, ready) = mpsc::channel();
    let (release, released) = mpsc::channel();
    std::thread::scope(|scope| {
        let shared = resources.clone();
        let waiter = scope.spawn(move || {
            let operation = shared
                .admit(60, 0, None, &ObservationContext::default())
                .unwrap();
            acquired.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(5)).unwrap();
            drop(operation);
        });
        await_admission_waiters(&resources, 1);
        assert_eq!(resources.reserved.lock().bytes, 60);
        resources.retire_all(&ObservationContext::default());
        assert!(
            matches!(ready.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "retiring submissions must not release a live owner's credit"
        );
        drop(first);
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(resources.reserved.lock().bytes, 60);
        release.send(()).unwrap();
        waiter.join().unwrap();
    });
    assert_eq!(resources.reserved.lock().bytes, 0);

    std::thread::scope(|scope| {
        for _ in 0..2 {
            let backend = gpu.derived_explicit();
            scope.spawn(move || {
                let operation = backend.admit_operation(1024).unwrap();
                let backend = operation.backend(&backend);
                let (source, output) = buffers(&backend);
                for _ in 0..8 {
                    copy(&backend, &source, &output);
                    let submissions = backend.queue.resources.submissions.lock();
                    assert!(
                        submissions.pending.len() + submissions.submitting
                            <= MAX_PENDING_SUBMISSIONS
                    );
                }
                operation.retire_pending();
                assert_eq!(
                    backend.read_mapped_u32(&output, output.size()),
                    [17, 29, 43, 71]
                );
            });
        }
    });
    assert_eq!(gpu.device.resources.reserved.lock().bytes, 0);
}

#[test]
fn one_pending_operation_retirement_counts_waits_without_publishing_batch_ready() {
    let gpu = crate::test_support::adapter();
    for explicit_retirement in [true, false] {
        let mut backend = gpu.derived_explicit();
        let context = ObservationContext::new(CollectionMode::Summary);
        let stage = context.scope("grain_spektra_v1", ObservationKind::Stage, Purpose::Image);
        backend.context_mut(stage.context().clone());
        // Admission captures the request context, before nested batch scoping.
        let operation = backend.admit_operation(1024).unwrap();
        let (backend, batch_guard) = backend.observed_batch("resource_retirement");
        let backend = operation.backend(&backend);
        let (source, output) = buffers(&backend);
        copy(&backend, &source, &output);
        let retirement = operation.last_submission().unwrap();
        assert_eq!(context.snapshot().totals.host_wait_count, Some(0));
        if explicit_retirement {
            operation.retire_pending();
            // Already-complete retirement and repeated explicit cleanup must
            // not count a host wait when no blocking poll actually runs.
            gpu.device.resources.retire(&retirement, stage.context());
            operation.retire_pending();
        }
        drop(backend);
        drop(operation);
        assert!(retirement.is_complete());
        assert_eq!(gpu.device.resources.reserved.lock().bytes, 0);
        drop(batch_guard);
        drop(stage);
        let snapshot = context.snapshot();
        let batch = snapshot
            .observations
            .iter()
            .find(|observation| observation.kind == ObservationKind::Batch)
            .unwrap();
        let measurements = batch.batch.as_ref().unwrap();
        let incomplete = Measurement::unavailable(AvailabilityReason::Incomplete);
        assert_eq!(measurements.submit_to_map_ready, incomplete);
        assert_eq!(measurements.host_materialize, incomplete);
        assert_eq!(snapshot.totals.host_wait_count, Some(1));
        assert_eq!(batch.work.host_wait_count, Some(0));
        let request = snapshot
            .observations
            .iter()
            .find(|observation| observation.kind == ObservationKind::Stage)
            .unwrap();
        assert_eq!(request.work.host_wait_count, Some(1));
    }
}

#[test]
fn cancelled_unused_submission_slot_returns_credit_without_a_device_wait() {
    let gpu = crate::test_support::adapter();
    let mut backend = gpu.derived_explicit();
    let context = ObservationContext::new(CollectionMode::Summary);
    backend.context_mut(context.clone());
    let operation = backend.admit_operation(1024).unwrap();
    let owned = operation.backend(&backend);
    let (source, output) = buffers(&owned);
    let mut encoder = owned.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&source, 0, &output, 0, source.size());
    let command = encoder.finish();
    drop(owned);
    drop(operation);
    assert_eq!(gpu.device.resources.reserved.lock().bytes, 1024);
    let cancellation = Arc::new(AtomicBool::new(true));
    let result = backend
        .queue
        .submit_cancellable(Some(command), Some(&cancellation));
    assert_eq!(result.err().as_deref(), Some("operation cancelled"));
    assert_eq!(gpu.device.resources.reserved.lock().bytes, 0);
    let submissions = gpu.device.resources.submissions.lock();
    assert_eq!(submissions.submitting, 0);
    assert!(submissions.pending.is_empty());
    let counters = context.snapshot().totals;
    assert_eq!(counters.host_wait_count, Some(0));
    assert_eq!(counters.submit_count, Some(0));
    assert_eq!(counters.command_buffer_count, Some(0));
}

#[test]
fn exclusive_admission_attributes_prior_retirement_to_the_request_context() {
    let gpu = crate::test_support::adapter();
    let mut resources = DeviceResources::new(gpu.device.raw.clone(), false);
    resources.overlap_target = 1;
    let resources = Arc::new(resources);
    let prior = ObservationContext::new(CollectionMode::Summary);
    let request = ObservationContext::new(CollectionMode::Summary);
    let command = gpu
        .device
        .raw
        .create_command_encoder(&Default::default())
        .finish();
    let retirement = resources
        .submit(&gpu.queue.raw, Some(command), None, None, &prior)
        .unwrap();
    let operation = resources.admit(2, 0, None, &request).unwrap();
    assert!(operation.state.exclusive);
    assert!(retirement.is_complete());
    assert_eq!(prior.snapshot().totals.host_wait_count, Some(0));
    assert_eq!(request.snapshot().totals.host_wait_count, Some(1));
    drop(operation);
    assert_eq!(request.snapshot().totals.host_wait_count, Some(1));
}

#[test]
fn native_allocation_lifetime_and_owner_inventory_are_not_cumulative_telemetry() {
    let gpu = crate::test_support::adapter();
    let Some(before) = gpu.device.resources.native_buffer_bytes() else {
        return;
    };
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("native_lifetime"),
        size: 4096,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let alias = buffer.clone();
    assert_eq!(unique_buffer_bytes([&buffer, &alias]).unwrap(), 4096);
    let allocated = gpu.device.resources.native_buffer_bytes().unwrap();
    assert!(allocated >= before + 4096);
    let layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
    let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    drop(buffer);
    drop(alias);
    gpu.device.raw.poll(wgpu::Maintain::Wait);
    assert_eq!(
        gpu.device.resources.native_buffer_bytes(),
        Some(allocated),
        "opaque bindgroup ownership must remain in native live allocation bytes"
    );
    drop(group);
    gpu.device.raw.poll(wgpu::Maintain::Wait);
    assert_eq!(gpu.device.resources.native_buffer_bytes(), Some(before));
}

#[test]
fn encoded_command_keeps_its_originating_reservation_through_foreign_queue_submit() {
    let gpu = crate::test_support::adapter();
    let operation = gpu.admit_operation(1024).unwrap();
    let backend = operation.backend(&gpu);
    let (source, output) = buffers(&backend);
    let mut encoder = backend.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&source, 0, &output, 0, source.size());
    drop(backend);
    drop(operation);
    assert_eq!(
        gpu.device.resources.reserved.lock().bytes,
        1024,
        "unsubmitted command encoders still own their allocation credit"
    );
    let command = encoder.finish();
    assert_eq!(gpu.device.resources.reserved.lock().bytes, 1024);
    // Submit through a different derivative from the command's origin. Its
    // originating operation must nevertheless acquire the retirement index.
    gpu.queue.submit(Some(command));
    assert_eq!(gpu.device.resources.reserved.lock().bytes, 0);
    assert!(gpu.device.resources.submissions.lock().pending.is_empty());
    assert_eq!(
        gpu.read_mapped_u32(&output, output.size()),
        [17, 29, 43, 71]
    );
}

#[test]
fn portable_owner_inventory_deduplicates_physical_buffers_and_keeps_only_weak_owners() {
    let gpu = crate::test_support::adapter();
    let resources = Arc::new(DeviceResources::new(gpu.device.raw.clone(), false));
    assert_eq!(resources.native_buffer_bytes(), None);
    let page = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("portable_carrier"),
        size: 12,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let image = Arc::new(BufferPayload::from_buffer(1, page.clone(), 1, 1));
    let alias = Arc::new(BufferPayload::from_buffer(1, page.clone(), 1, 1));
    resources.register_image(&image);
    resources.register_image(&image);
    resources.register_image(&alias);
    let scan = Arc::new(PreparedScan {
        exposure: image.pages.clone(),
        setup: page.clone(),
        status: page.clone(),
    });
    resources.register_scan(&scan);
    assert_eq!(
        resources.retained_owner_bytes().unwrap(),
        12,
        "views, cloned carriers, and prepared cache aliases name one allocation"
    );
    let mut small = DeviceResources::new(gpu.device.raw.clone(), false);
    small.overlap_target = 20;
    let small = Arc::new(small);
    small.register_image(&image);
    let oversized = small
        .admit(9, 0, None, &ObservationContext::default())
        .unwrap();
    assert!(
        small.reserved.lock().exclusive,
        "retained-heavy work must serialize, not be rejected at the overlap target"
    );
    drop(oversized);
    let ordinary = small
        .admit(8, 0, None, &ObservationContext::default())
        .unwrap();
    assert!(!small.reserved.lock().exclusive);
    drop(ordinary);
    drop(image);
    drop(alias);
    drop(scan);
    assert_eq!(
        resources.retained_owner_bytes().unwrap(),
        0,
        "registrations must not prolong an application's carrier lifetime"
    );
    assert!(resources.retained.lock().images.is_empty());
    assert!(resources.retained.lock().scans.is_empty());
    assert!(
        small
            .admit(20, 0, None, &ObservationContext::default())
            .is_ok()
    );
}

#[test]
fn portable_admission_counts_readbacks_after_their_image_owner_drops() {
    let gpu = crate::test_support::adapter();
    let input = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("portable_pool_input"),
            contents: bytemuck::cast_slice(&[1.0f32; 12]),
            usage: wgpu::BufferUsages::COPY_SRC,
        });
    assert_eq!(gpu.pipe.readback_buffer(&input, 48), vec![1.0f32; 12]);
    drop(input);
    gpu.device.raw.poll(wgpu::Maintain::Wait);
    let cached = gpu.pipe.retained_cache_bytes().unwrap();
    assert!(cached >= 48);
    let mut resources = DeviceResources::new(gpu.device.raw.clone(), false);
    resources.overlap_target = cached + 11;
    let resources = Arc::new(resources);
    let mut backend = gpu.derived_explicit();
    backend.device.resources = resources.clone();
    backend.queue.resources = resources;
    backend.pipe = backend
        .pipe
        .with_observed(backend.device.clone(), backend.queue.clone());
    let operation = backend.admit_operation(12).unwrap();
    assert!(
        operation.state.exclusive,
        "pooled readback capacity must trigger exclusive scheduling above the target"
    );
    drop(operation);
    let operation = backend.admit_operation(11).unwrap();
    assert!(!operation.state.exclusive);
}

#[test]
fn oversized_owners_block_both_normal_and_oversized_admission_until_owner_release() {
    let gpu = crate::test_support::adapter();
    let mut resources = DeviceResources::new(gpu.device.raw.clone(), false);
    resources.overlap_target = 100;
    let resources = Arc::new(resources);
    let first = resources
        .admit(150, 0, None, &ObservationContext::default())
        .unwrap();
    let (large_acquired, large_ready) = mpsc::channel();
    let (release_large, large_released) = mpsc::channel();
    let (normal_acquired, normal_ready) = mpsc::channel();
    std::thread::scope(|scope| {
        let shared = resources.clone();
        let large = scope.spawn(move || {
            let operation = shared
                .admit(200, 0, None, &ObservationContext::default())
                .unwrap();
            large_acquired.send(()).unwrap();
            large_released.recv_timeout(Duration::from_secs(5)).unwrap();
            drop(operation);
        });
        await_admission_waiters(&resources, 1);
        let shared = resources.clone();
        let normal = scope.spawn(move || {
            let operation = shared
                .admit(20, 0, None, &ObservationContext::default())
                .unwrap();
            normal_acquired.send(()).unwrap();
            drop(operation);
        });
        await_admission_waiters(&resources, 2);
        resources.retire_all(&ObservationContext::default());
        assert!(matches!(
            large_ready.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert!(matches!(
            normal_ready.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        drop(first);
        large_ready.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(resources.reserved.lock().bytes, 200);
        assert_eq!(resources.reserved.lock().owners, 1);
        assert!(matches!(
            normal_ready.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        release_large.send(()).unwrap();
        normal_ready.recv_timeout(Duration::from_secs(5)).unwrap();
        large.join().unwrap();
        normal.join().unwrap();
    });
    assert_eq!(resources.reserved.lock().owners, 0);
    assert!(!resources.reserved.lock().exclusive);
}

#[test]
fn cancelled_admission_waits_drop_priority_without_releasing_another_owner() {
    let gpu = crate::test_support::adapter();
    let mut resources = DeviceResources::new(gpu.device.raw.clone(), false);
    resources.overlap_target = 100;
    let resources = Arc::new(resources);
    for requested in [60, 150] {
        let first = resources
            .admit(60, 0, None, &ObservationContext::default())
            .unwrap();
        let cancellation = Arc::new(AtomicBool::new(false));
        let (finished, ready) = mpsc::channel();
        std::thread::scope(|scope| {
            let shared = resources.clone();
            let token = cancellation.clone();
            let waiter = scope.spawn(move || {
                let result =
                    shared.admit(requested, 0, Some(&token), &ObservationContext::default());
                finished.send(result.err()).unwrap();
            });
            await_admission_waiters(&resources, 1);
            cancellation.store(true, Ordering::Release);
            assert_eq!(
                ready
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .as_deref(),
                Some("operation cancelled")
            );
            waiter.join().unwrap();
        });
        let state = resources.reserved.lock();
        assert_eq!(state.bytes, 60);
        assert_eq!(state.owners, 1);
        assert_eq!(state.waiters, 0);
        assert_eq!(state.exclusive_waiters, 0);
        drop(state);
        let ordinary = resources
            .admit(20, 0, None, &ObservationContext::default())
            .unwrap();
        assert_eq!(resources.reserved.lock().bytes, 80);
        drop(ordinary);
        drop(first);
    }
    assert_eq!(resources.reserved.lock().owners, 0);
}

#[test]
fn nested_admission_reuses_the_declared_parent_peak_without_waiting_on_itself() {
    let gpu = crate::test_support::adapter();
    let operation = gpu.admit_operation(1024).unwrap();
    let backend = operation.backend(&gpu);
    let nested = backend.admit_operation(1024).unwrap();
    assert!(Arc::ptr_eq(&operation.state, &nested.state));
    assert_eq!(gpu.device.resources.reserved.lock().owners, 1);
    assert_eq!(
        backend.admit_operation(1025).err().as_deref(),
        Some("nested GPU operation exceeds its declared parent peak")
    );
    drop(nested);
    drop(operation);
    assert_eq!(gpu.device.resources.reserved.lock().owners, 1);
    drop(backend);
    assert_eq!(gpu.device.resources.reserved.lock().owners, 0);
}

#[test]
fn last_submission_marker_survives_pruning_and_reports_exact_retirement_identity() {
    let gpu = crate::test_support::adapter();
    let operation = gpu.admit_operation(1024).unwrap();
    let backend = operation.backend(&gpu);
    let (source, output) = buffers(&backend);
    assert!(operation.last_submission().is_none());
    copy(&backend, &source, &output);
    let first = operation.last_submission().unwrap();
    assert!(!first.is_complete());
    copy(&backend, &source, &output);
    let second = operation.last_submission().unwrap();
    assert!(!first.same_submission(&second));
    copy(&backend, &source, &output);
    let third = operation.last_submission().unwrap();
    assert!(
        first.is_complete(),
        "third submission must retire the oldest shared queue owner"
    );
    assert!(!third.same_submission(&second));
    assert!(operation.state.pending.lock().pending.len() <= MAX_PENDING_SUBMISSIONS);
    operation.retire_pending();
    assert!(third.is_complete());
    assert!(operation.state.pending.lock().pending.is_empty());
    assert!(
        operation.last_submission().unwrap().same_submission(&third),
        "pruning retired pending entries must not erase the last-submission marker"
    );
}
