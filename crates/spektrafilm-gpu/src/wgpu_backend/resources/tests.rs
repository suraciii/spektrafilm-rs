use super::*;
use spektrafilm_backend::telemetry::{
    AvailabilityReason, CollectionMode, Measurement, ObservationContext, ObservationKind, Purpose,
};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

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
    let first = resources.admit(60, 0, None).unwrap();
    let (acquired, ready) = mpsc::channel();
    let (release, released) = mpsc::channel();
    std::thread::scope(|scope| {
        let shared = resources.clone();
        let waiter = scope.spawn(move || {
            let operation = shared.admit(60, 0, None).unwrap();
            acquired.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(5)).unwrap();
            drop(operation);
        });
        await_admission_waiters(&resources, 1);
        assert_eq!(resources.reserved.lock().bytes, 60);
        resources.retire_all();
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
fn common_queue_retires_without_publishing_batch_ready() {
    let gpu = crate::test_support::adapter();
    let mut backend = gpu.derived_explicit();
    let context = ObservationContext::new(CollectionMode::Summary);
    let stage = context.scope("grain_spektra_v1", ObservationKind::Stage, Purpose::Image);
    backend.context_mut(stage.context().clone());
    let (backend, batch_guard) = backend.observed_batch("resource_retirement");
    let operation = backend.admit_operation(1024).unwrap();
    let backend = operation.backend(&backend);
    let (source, output) = buffers(&backend);
    for _ in 0..3 {
        copy(&backend, &source, &output);
    }
    assert!(backend.queue.resources.submissions.lock().pending.len() <= MAX_PENDING_SUBMISSIONS);
    operation.retire_pending();
    drop(backend);
    drop(operation);
    drop(batch_guard);
    drop(stage);
    let snapshot = context.snapshot();
    let measurements = snapshot
        .observations
        .iter()
        .find(|observation| observation.kind == ObservationKind::Batch)
        .unwrap()
        .batch
        .as_ref()
        .unwrap();
    let incomplete = Measurement::unavailable(AvailabilityReason::Incomplete);
    assert_eq!(measurements.submit_to_map_ready, incomplete);
    assert_eq!(measurements.host_materialize, incomplete);
    assert!(snapshot.totals.host_wait_count.unwrap_or(0) > 0);
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
    let oversized = small.admit(9, 0, None).unwrap();
    assert!(
        small.reserved.lock().exclusive,
        "retained-heavy work must serialize, not be rejected at the overlap target"
    );
    drop(oversized);
    let ordinary = small.admit(8, 0, None).unwrap();
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
    assert!(small.admit(20, 0, None).is_ok());
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
fn full_resolution_gui_peak_above_overlap_target_is_admitted_exclusively() {
    let gpu = crate::test_support::adapter();
    let resources = Arc::new(DeviceResources::new(gpu.device.raw.clone(), false));
    // Current GUI-default layered Spektra alone can retain eighteen RGB
    // rasters. This is admission coverage, not an allocation/VRAM claim.
    let planned_peak = 18u64 * 6342 * 9543 * 12;
    assert!(planned_peak > OVERLAP_TARGET_BYTES);
    let operation = resources.admit(planned_peak, 0, None).unwrap();
    assert_eq!(operation.reserved_bytes(), planned_peak);
    assert!(operation.state.exclusive);
    assert_eq!(resources.reserved.lock().owners, 1);
    drop(operation);
    assert_eq!(resources.reserved.lock().bytes, 0);
}

#[test]
fn oversized_owners_block_both_normal_and_oversized_admission_until_owner_release() {
    let gpu = crate::test_support::adapter();
    let mut resources = DeviceResources::new(gpu.device.raw.clone(), false);
    resources.overlap_target = 100;
    let resources = Arc::new(resources);
    let first = resources.admit(150, 0, None).unwrap();
    let (large_acquired, large_ready) = mpsc::channel();
    let (release_large, large_released) = mpsc::channel();
    let (normal_acquired, normal_ready) = mpsc::channel();
    std::thread::scope(|scope| {
        let shared = resources.clone();
        let large = scope.spawn(move || {
            let operation = shared.admit(200, 0, None).unwrap();
            large_acquired.send(()).unwrap();
            large_released.recv_timeout(Duration::from_secs(5)).unwrap();
            drop(operation);
        });
        await_admission_waiters(&resources, 1);
        let shared = resources.clone();
        let normal = scope.spawn(move || {
            let operation = shared.admit(20, 0, None).unwrap();
            normal_acquired.send(()).unwrap();
            drop(operation);
        });
        await_admission_waiters(&resources, 2);
        resources.retire_all();
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
        let first = resources.admit(60, 0, None).unwrap();
        let cancellation = Arc::new(AtomicBool::new(false));
        let (finished, ready) = mpsc::channel();
        std::thread::scope(|scope| {
            let shared = resources.clone();
            let token = cancellation.clone();
            let waiter = scope.spawn(move || {
                let result = shared.admit(requested, 0, Some(&token));
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
        let ordinary = resources.admit(20, 0, None).unwrap();
        assert_eq!(resources.reserved.lock().bytes, 80);
        drop(ordinary);
        drop(first);
    }
    assert_eq!(resources.reserved.lock().owners, 0);
}

#[test]
fn finite_peak_is_not_rejected_but_footprint_arithmetic_overflow_is() {
    let gpu = crate::test_support::adapter();
    let resources = Arc::new(DeviceResources::new(gpu.device.raw.clone(), false));
    let result = resources.admit(u64::MAX, 1, None);
    assert_eq!(
        result.err().as_deref(),
        Some("GPU operation planned footprint overflows")
    );
    let operation = resources.admit(u64::MAX, 0, None).unwrap();
    assert!(operation.state.exclusive);
    assert_eq!(resources.reserved.lock().bytes, u64::MAX);
    drop(operation);
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
