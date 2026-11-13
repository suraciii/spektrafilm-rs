//! Batched evaluation of the full-source f32 pairwise tree.
//!
//! A fixed-depth frontier has one active slot per original leaf. Shorter paths
//! occupy their leftmost descendant; other slots are padding. Transport windows
//! carry a split leaf's eight accumulators, never introduce a new sum boundary.
use super::*;

const LEAF: u32 = 128;
const RECORD_BYTES: u64 = 24;

fn depth(mut count: u32) -> u32 {
    let mut depth = 0;
    while count > LEAF {
        count -= (count / 2) & !7;
        depth += 1;
    }
    depth
}

fn node(total: u32, index: u32, depth: u32) -> (u32, u32) {
    let mut first = 0;
    let mut count = total;
    for level in (0..depth).rev() {
        let right = (index >> level) & 1 != 0;
        if count <= LEAF {
            if right {
                return (first + count, 0);
            }
        } else {
            let midpoint = (count / 2) & !7;
            if right {
                first += midpoint;
                count -= midpoint;
            } else {
                count = midpoint;
            }
        }
    }
    (first, count)
}

fn lower_bound(slots: u32, mut predicate: impl FnMut(u32) -> bool) -> u32 {
    let (mut low, mut high) = (0, slots);
    while low < high {
        let middle = low + (high - low) / 2;
        if predicate(middle) {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    low
}

fn partial_records(total: u32, chunk: u32) -> u32 {
    (u64::from(chunk).div_ceil(64) * 2 + 4).min(u64::from(1u32 << depth(total))) as u32
}

pub(in crate::wgpu_backend) fn workspace_bytes(total: u32, chunk: u32) -> Result<u64, String> {
    if total == 0 || total > u32::MAX / 3 || chunk < total.min(16) {
        return Err("grain mass dimensions or transport window are not representable".into());
    }
    Ok(2 * u64::from(1u32 << depth(total)) * RECORD_BYTES
        + u64::from(partial_records(total, chunk)) * RECORD_BYTES
        + 192)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    backend: &WgpuBackend,
    commands: &mut RasterCommands,
    source: &dyn RasterStorage,
    sharpened: &dyn RasterStorage,
    source_window: &wgpu::Buffer,
    sharpen_window: &wgpu::Buffer,
    pair: &wgpu::Buffer,
    total: u32,
    chunk: u32,
) -> Result<(), String> {
    workspace_bytes(total, chunk)?;
    let tree_depth = depth(total);
    let slots = 1u32 << tree_depth;
    let bytes = u64::from(slots) * RECORD_BYTES;
    let mut current = paged_transport(backend, bytes, "grain_mass_frontier")?;
    let mut next = paged_transport(backend, bytes, "grain_mass_parent")?;
    // A leaf has at least 64 pixels, except a root which is itself a leaf.
    // Two spare frontier slots per leaf also cover an unaligned window edge.
    let records = partial_records(total, chunk);
    let partial = transport(
        backend,
        u64::from(records) * RECORD_BYTES,
        "grain_mass_partials",
    )?;
    let carry = transport(backend, 192, "grain_mass_carry")?;
    let leaf_pipe = backend.cached_pipeline(
        include_str!("../post_reduce.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    for first in (0..total).step_by(chunk as usize) {
        backend.check_cancelled()?;
        let count = chunk.min(total - first);
        let low = lower_bound(slots, |index| {
            let (start, count) = node(total, index, tree_depth);
            start + count > first
        });
        let high = lower_bound(slots, |index| {
            node(total, index, tree_depth).0 >= first + count
        });
        let length = high - low;
        if length == 0 || length > records {
            return Err("grain mass frontier exceeds the transport workspace".into());
        }
        commands.copy(
            source,
            u64::from(first) * 12,
            source_window,
            0,
            u64::from(count) * 12,
        );
        commands.copy(
            sharpened,
            u64::from(first) * 12,
            sharpen_window,
            0,
            u64::from(count) * 12,
        );
        // Finish the previous window's leaf before another leaf can publish
        // carry. This avoids a read/write race without a second carry binding
        // or increasing the supported 192-byte minimum binding capacity.
        let continuation = node(total, low, tree_depth).0 < first;
        let mut dispatch = |first_leaf: u32, leaves: u32| {
            let p = uniform(
                backend,
                &[total, tree_depth, first_leaf, leaves, first, count, low, 0],
            );
            commands.dispatch(
                &leaf_pipe,
                &[&p, source_window, sharpen_window, &carry, &partial],
                leaves * 3,
                false,
                backend,
            );
        };
        if continuation {
            dispatch(low, 1);
        }
        let bulk_first = low + u32::from(continuation);
        if bulk_first < high {
            dispatch(bulk_first, high - bulk_first);
        }
        commands.copy(
            &partial,
            0,
            &current,
            u64::from(low) * RECORD_BYTES,
            u64::from(length) * RECORD_BYTES,
        );
    }
    let merge_pipe = backend.cached_pipeline(
        include_str!("../post_reduce_merge.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let input_bytes = source_window.size().min(sharpen_window.size());
    let parents_per_window = (input_bytes / (2 * RECORD_BYTES)).max(1) as u32;
    for level in (0..tree_depth).rev() {
        let count = 1u32 << level;
        for first in (0..count).step_by(parents_per_window as usize) {
            backend.check_cancelled()?;
            let length = parents_per_window.min(count - first);
            commands.copy(
                &current,
                u64::from(first) * 2 * RECORD_BYTES,
                source_window,
                0,
                u64::from(length) * 2 * RECORD_BYTES,
            );
            let p = uniform(backend, &[total, level, first, length]);
            commands.dispatch(
                &merge_pipe,
                &[&p, source_window, sharpen_window],
                length * 6,
                false,
                backend,
            );
            commands.copy(
                sharpen_window,
                0,
                &next,
                u64::from(first) * RECORD_BYTES,
                u64::from(length) * RECORD_BYTES,
            );
        }
        std::mem::swap(&mut current, &mut next);
    }
    commands.copy(&current, 0, pair, 0, RECORD_BYTES);
    Ok(())
}

#[cfg(test)]
mod logic_tests {
    use super::*;

    #[test]
    fn padded_frontier_covers_the_original_leaves_without_changing_splits() {
        fn original(first: u32, count: u32, out: &mut Vec<(u32, u32)>) {
            if count <= LEAF {
                out.push((first, count));
            } else {
                let middle = (count / 2) & !7;
                original(first, middle, out);
                original(first + middle, count - middle, out);
            }
        }
        for n in (1..=4097).chain([60_000_000, u32::MAX / 3]) {
            let d = depth(n);
            if n <= 4097 {
                let got: Vec<_> = (0..1u32 << d)
                    .map(|i| node(n, i, d))
                    .filter(|&(_, count)| count != 0)
                    .collect();
                let mut expected = Vec::new();
                original(0, n, &mut expected);
                assert_eq!(got, expected, "n={n}");
            } else {
                let first = node(n, 0, d);
                let last = node(n, (1u32 << d) - 1, d);
                assert_eq!(first.0, 0);
                assert_eq!(last.0 + last.1, n);
            }
        }
    }
    #[test]
    fn mass_workspace_counts_owned_buffers_and_rejects_invalid_domains() {
        assert_eq!(workspace_bytes(1, 16).unwrap(), 24 * 3 + 192);
        assert_eq!(
            workspace_bytes(60_000_000, (64 << 20) / 12).unwrap(),
            25_165_824 + 4_194_432 + 192
        );
        assert!(workspace_bytes(0, 16).is_err());
        assert!(workspace_bytes(1, 0).is_err());
        assert!(workspace_bytes(128, 15).is_err());
        assert!(workspace_bytes(u32::MAX, 16).is_err());
    }

    #[test]
    fn window_frontiers_fit_partial_scratch_and_carry_only_one_leaf() {
        for total in (1..=1025).chain([4097, 1_000_000, 60_000_000, u32::MAX / 3]) {
            let d = depth(total);
            let slots = 1u32 << d;
            for chunk in [16u32, 17, 29, 130, 1024, (64 << 20) / 12] {
                let cap = (u64::from(chunk).div_ceil(64) * 2 + 4).min(u64::from(slots));
                // Probe boundaries without enumerating millions of tiny windows.
                for first in [0, chunk.min(total - 1), total / 2, total - 1] {
                    let count = chunk.min(total - first);
                    let low = lower_bound(slots, |i| {
                        let (s, n) = node(total, i, d);
                        s + n > first
                    });
                    let high = lower_bound(slots, |i| node(total, i, d).0 >= first + count);
                    assert!(high > low && u64::from(high - low) <= cap);
                    assert!(node(total, low, d).0 <= first);
                    let (s, n) = node(total, high - 1, d);
                    assert!(s + n >= first + count);
                }
            }
        }
    }
}

#[cfg(test)]
mod device_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[test]
    fn paged_mass_batches_bound_commands_and_reinitialize_after_cancellation() {
        let guard = crate::test_support::adapter();
        let mut gpu = guard.derived_explicit();
        let cancel = Arc::new(AtomicBool::new(false));
        gpu.pipe = gpu.pipe.with_cancellation(Some(cancel.clone()));
        gpu.cancellation = Some(cancel.clone());
        // Physical pages deliberately cut through an RGB pixel. Copy-only
        // storage must not require direct binding or aligned pixel pages.
        let n = 4097u32;
        let values: Vec<f32> = (0..n * 3).map(|i| (i % 31) as f32 * 0.125).collect();
        // Use custom storage to exercise RasterStorage::visit byte boundaries.
        struct Parts(Vec<(u64, wgpu::Buffer)>);
        impl RasterStorage for Parts {
            fn visit(&self, f: &mut dyn FnMut(u64, &wgpu::Buffer)) {
                for (first, buffer) in &self.0 {
                    f(*first, buffer);
                }
            }
            fn image_pages(&self) -> Vec<crate::wgpu_backend::paged::ImagePage> {
                unreachable!("mass transport uses visit")
            }
        }
        let bytes: &[u8] = bytemuck::cast_slice(&values);
        let mut parts = Vec::new();
        for (i, block) in bytes.chunks(1028).enumerate() {
            parts.push((
                (i * 1028) as u64,
                gpu.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mass_copy_only_page"),
                        contents: block,
                        usage: wgpu::BufferUsages::COPY_SRC,
                    }),
            ));
        }
        let source = Parts(parts);
        let window = transport(&gpu, 1560, "mass_input_window").unwrap();
        let sharpen_window = transport(&gpu, 1560, "mass_sharpen_window").unwrap();
        let pair = transport(&gpu, 24, "mass_result").unwrap();
        let mut commands = RasterCommands::default();
        build(
            &gpu,
            &mut commands,
            &source,
            &source,
            &window,
            &sharpen_window,
            &pair,
            n,
            130,
        )
        .unwrap();
        // Execute a real prefix, cancel, then rerun the same immutable plan.
        let prefix = commands
            .0
            .iter()
            .position(|c| {
                matches!(
                    c,
                    crate::wgpu_backend::grain::raster::RasterCommand::Dispatch { .. }
                )
            })
            .unwrap()
            + 1;
        let tail = commands.0.split_off(prefix);
        commands.submit(&gpu).unwrap();
        commands.0.extend(tail);
        cancel.store(true, Ordering::Relaxed);
        assert!(commands.submit(&gpu).is_err());
        let mut rejected = RasterCommands::default();
        assert!(
            build(
                &gpu,
                &mut rejected,
                &source,
                &source,
                &window,
                &sharpen_window,
                &pair,
                n,
                130
            )
            .is_err()
        );
        cancel.store(false, Ordering::Relaxed);
        commands.submit(&gpu).unwrap();
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mass_paged_readback"),
            size: 24,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&pair, 0, &readback, 0, 24);
        gpu.queue.submit(Some(encoder.finish()));
        let result: Vec<f32> = gpu.pipe.read_mapped(&readback, 24).unwrap();
        for channel in 0..3 {
            let expected: f64 = values
                .iter()
                .skip(channel)
                .step_by(3)
                .map(|&x| x as f64)
                .sum();
            assert!((result[channel] as f64 - expected).abs() <= expected * 1e-5);
            assert_eq!(result[channel].to_bits(), result[channel + 3].to_bits());
        }
        // A genuine 60MP reduction plan must have bulk commands, not one
        // command/uniform per global leaf. No million-node legacy plan built.
        let total = 60_000_000u32;
        let full = paged_transport(&gpu, u64::from(total) * 12, "mass_full_source").unwrap();
        let window = transport(&gpu, 64 << 20, "mass_full_window").unwrap();
        let output = transport(&gpu, 64 << 20, "mass_full_output").unwrap();
        let mut plan = RasterCommands::default();
        build(
            &gpu,
            &mut plan,
            &full,
            &full,
            &window,
            &output,
            &pair,
            total,
            ((64 << 20) / 12) as u32,
        )
        .unwrap();
        let dispatches = plan
            .0
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    crate::wgpu_backend::grain::raster::RasterCommand::Dispatch { .. }
                )
            })
            .count();
        assert_eq!(dispatches, 40);
        assert!(
            plan.0.len() < 256,
            "unbounded mass plan: {} commands",
            plan.0.len()
        );
        // Sparse exact integer mass at the full-source endpoints and both
        // sides of a transport seam exercises the actual 60MP GPU plan.
        for pixel in [0, (64 << 20) / 12 - 1, (64 << 20) / 12, total - 1] {
            let offset = u64::from(pixel) * 12;
            full.visit(&mut |first, buffer| {
                if offset >= first && offset + 12 <= first + buffer.size() {
                    gpu.queue.write_buffer(
                        buffer,
                        offset - first,
                        bytemuck::cast_slice(&[1.0f32, 2.0, 3.0]),
                    );
                }
            });
        }
        plan.submit(&gpu).unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&pair, 0, &readback, 0, 24);
        gpu.queue.submit(Some(encoder.finish()));
        let result: Vec<f32> = gpu.pipe.read_mapped(&readback, 24).unwrap();
        assert_eq!(result, [4.0, 8.0, 12.0, 4.0, 8.0, 12.0]);
    }
}
