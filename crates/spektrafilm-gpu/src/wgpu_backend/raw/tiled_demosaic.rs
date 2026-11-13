//! Bounded transport around the established integer demosaic kernels.
//!
//! Bayer windows commit only cores whose complete dependency graph is present.
//! X-Trans uses the parallel Markesteijn dependency halo. Neither path invents
//! CFA phase or image borders at a storage-page boundary.
use super::paged::RawPlane;
use super::*;

#[cfg(test)]
const WRITE: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: false };

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Window {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

fn demosaic_step(limit: u64, halo: u32, cfa_rows: u32, cfa_columns: u32) -> Result<u32, String> {
    let side = ((limit / 16) as f64)
        .sqrt()
        .floor()
        .min(f64::from(u32::MAX)) as u32;
    let margin = 2 * halo + cfa_rows.max(cfa_columns) - 1;
    if side <= margin {
        return Err(format!(
            "RAW demosaic minimum dependency window exceeds {limit} bytes"
        ));
    }
    Ok(side - margin)
}

fn demosaic_window(
    image: (u32, u32),
    origin: (u32, u32),
    core: (u32, u32),
    cfa: (u32, u32),
    halo: u32,
    vng: bool,
) -> Window {
    let (x, y) = origin;
    let (rows, columns) = cfa;
    // Align the gathered origin, not the core, to the global CFA.
    let mut left = x.saturating_sub(halo) / columns * columns;
    let mut top = y.saturating_sub(halo) / rows * rows;
    let right = (x + core.0).saturating_add(halo).min(image.0);
    let bottom = (y + core.1).saturating_add(halo).min(image.1);
    // VNG's tiny-image early return describes the source, not a final partition.
    if vng {
        left = left.min(right.saturating_sub(8) / columns * columns);
        top = top.min(bottom.saturating_sub(8) / rows * rows);
    }
    Window {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

impl WgpuBackend {
    pub(super) fn raw_demosaic_paged_gpu(
        &self,
        sensor: &RawSensor,
        params: &RawGpuParams,
        plane: RawPlane,
        cfa: &RawBuffer,
        xyz: &RawBuffer,
        cbrt: &RawBuffer,
    ) -> Result<RawPlane, String> {
        self.raw_demosaic_paged_with_limit(
            sensor,
            params,
            plane,
            cfa,
            xyz,
            cbrt,
            super::super::bounded::storage_binding_limit(self)?,
        )
    }

    // Preserve the independent demosaic bindings while exposing the transport limit.
    #[allow(clippy::too_many_arguments)]
    fn raw_demosaic_paged_with_limit(
        &self,
        sensor: &RawSensor,
        params: &RawGpuParams,
        plane: RawPlane,
        cfa: &RawBuffer,
        xyz: &RawBuffer,
        cbrt: &RawBuffer,
        limit: u64,
    ) -> Result<RawPlane, String> {
        self.check_cancelled()?;
        if params.demosaic == RawDemosaic::None
            || (params.demosaic == RawDemosaic::Vng && (plane.width < 8 || plane.height < 8))
        {
            return Ok(plane);
        }
        let frame = RawFrame {
            width: plane.width,
            height: plane.height,
            cfa_rows: sensor.cfa_rows,
            cfa_columns: sensor.cfa_columns,
            black_rows: 0,
            black_columns: 0,
            green_merge: u32::from(params.green_merge),
            has_black_map: 0,
            merge_row: params.green_merge_row,
            merge_column: params.green_merge_column,
            source_width: plane.width,
            source_height: plane.height,
            channels: 4,
            fuji_width: 0,
            fuji_layout: 0,
            colors: sensor.colors,
            subtract_black_map: 0,
            mix_green: 0,
        };
        // AHD: green(2) -> RGB(1) -> homogeneity(1) -> combine(1).
        // VNG: linear(1) -> gradients/second-neighbour averages(2).
        // PPG: directional green(3) -> green-site chroma(1) -> diagonal chroma(1).
        let halo = match params.demosaic {
            RawDemosaic::BayerAhd => 5,
            RawDemosaic::Vng => 3,
            RawDemosaic::FujiPpg => 5,
            RawDemosaic::XTrans => 17,
            _ => unreachable!(),
        };
        // Direction arrays carry eight vec4 planes. Bound both storage and
        // per-submission work; each window commits only its independent core.
        let window_limit = if params.demosaic == RawDemosaic::XTrans {
            (limit / 8).min(512 * 512 * 16)
        } else {
            limit
        };
        let step = demosaic_step(window_limit, halo, sensor.cfa_rows, sensor.cfa_columns)?;
        let output = self.raw_allocate_plane(plane.width, plane.height, "raw_demosaic_pages")?;
        for y in (0..plane.height).step_by(step as usize) {
            for x in (0..plane.width).step_by(step as usize) {
                self.check_cancelled()?;
                let core_w = step.min(plane.width - x);
                let core_h = step.min(plane.height - y);
                let window = demosaic_window(
                    (plane.width, plane.height),
                    (x, y),
                    (core_w, core_h),
                    (sensor.cfa_rows, sensor.cfa_columns),
                    halo,
                    params.demosaic == RawDemosaic::Vng,
                );
                let left = window.x;
                let top = window.y;
                let image = self.raw_window_buffer(window, "raw_demosaic_window");
                let local = RawFrame {
                    width: window.width,
                    height: window.height,
                    ..frame
                };
                let frame_buf = RawBuffer::upload(
                    &self.device,
                    "raw_demosaic_frame",
                    bytemuck::bytes_of(&local),
                    wgpu::BufferUsages::UNIFORM,
                );
                let (passes, status) = self.raw_demosaic_gpu(
                    params.demosaic,
                    sensor.colors,
                    &local,
                    &frame_buf,
                    &image,
                    cfa,
                    xyz,
                    cbrt,
                )?;
                let mut encoder = self.device.create_command_encoder(&Default::default());
                plane.copy_window_to(
                    &mut encoder,
                    left,
                    top,
                    window.width,
                    window.height,
                    &image.buffer,
                )?;
                for pass in &passes {
                    pass.encode(&mut encoder, "raw_demosaic_window");
                }
                output.copy_window_from(
                    &mut encoder,
                    x,
                    y,
                    core_w,
                    core_h,
                    &image.buffer,
                    window.width,
                    x - left,
                    y - top,
                )?;
                self.raw_submit(encoder)?;
                if let Some(status) = status {
                    self.raw_xtrans_check(&status)?;
                }
            }
        }
        Ok(output)
    }

    fn raw_window_buffer(&self, window: Window, label: &str) -> RawBuffer {
        RawBuffer {
            buffer: self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: u64::from(window.width) * u64::from(window.height) * 16,
                mapped_at_creation: false,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            }),
        }
    }

    fn raw_xtrans_check(&self, metadata: &RawBuffer) -> Result<(), String> {
        let values = self
            .pipe
            .readback_buffer_cancellable::<i32>(&metadata.buffer, 147 * 4)?;
        if values[146] != 0 {
            Err("Markesteijn rejected an invalid X-Trans CFA pattern".into())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod logic_tests {
    use super::*;

    #[test]
    fn demosaic_step_requires_room_for_halo_and_cfa_alignment() {
        assert!(demosaic_step(12 * 12 * 16 - 1, 5, 2, 2).is_err());
        assert_eq!(demosaic_step(12 * 12 * 16, 5, 2, 2).unwrap(), 1);
        assert_eq!(demosaic_step(16 * 16 * 16, 3, 4, 2).unwrap(), 7);
    }

    #[test]
    fn demosaic_windows_preserve_global_cfa_phase_and_clip_halos() {
        let interior = demosaic_window((31, 23), (13, 9), (7, 7), (2, 2), 5, false);
        assert_eq!(
            (interior.x, interior.y, interior.width, interior.height),
            (8, 4, 17, 17)
        );
        let edge = demosaic_window((31, 23), (28, 21), (3, 2), (2, 2), 5, false);
        assert_eq!((edge.x, edge.y, edge.width, edge.height), (22, 16, 9, 7));
        let first = demosaic_window((31, 23), (0, 0), (7, 7), (2, 2), 5, false);
        assert_eq!(
            (first.x, first.y, first.width, first.height),
            (0, 0, 12, 12)
        );
    }

    #[test]
    fn vng_final_partition_keeps_native_eight_pixel_precondition() {
        let window = demosaic_window((15, 15), (14, 14), (1, 1), (2, 2), 3, true);
        assert_eq!(
            (window.x, window.y, window.width, window.height),
            (6, 6, 9, 9)
        );
        let ordinary = demosaic_window((15, 15), (14, 14), (1, 1), (2, 2), 3, false);
        assert_eq!(
            (ordinary.x, ordinary.y, ordinary.width, ordinary.height),
            (10, 10, 5, 5)
        );
    }

    #[test]
    fn demosaic_partition_cores_cover_image_and_windows_fit_limit() {
        let limit = 16 * 16 * 16;
        let step = demosaic_step(limit, 5, 2, 2).unwrap();
        let (width, height) = (23, 19);
        let mut covered = vec![0u8; (width * height) as usize];
        for y in (0..height).step_by(step as usize) {
            for x in (0..width).step_by(step as usize) {
                let core = (step.min(width - x), step.min(height - y));
                let window = demosaic_window((width, height), (x, y), core, (2, 2), 5, false);
                assert_eq!(window.x % 2, 0);
                assert_eq!(window.y % 2, 0);
                assert!(u64::from(window.width) * u64::from(window.height) * 16 <= limit);
                assert!(window.x <= x && window.y <= y);
                assert!(window.x + window.width >= x + core.0);
                assert!(window.y + window.height >= y + core.1);
                for row in y..y + core.1 {
                    for col in x..x + core.0 {
                        covered[(row * width + col) as usize] += 1;
                    }
                }
            }
        }
        assert!(covered.iter().all(|&count| count == 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compare(
        gpu: &WgpuBackend,
        mode: RawDemosaic,
        size: u32,
        pattern: &[u8],
        rows: u32,
        columns: u32,
        limit: u64,
    ) {
        let mut sensor = super::super::tests::synthetic_sensor(size);
        sensor.cfa = pattern.to_vec();
        sensor.cfa_rows = rows;
        sensor.cfa_columns = columns;
        sensor.colors = if pattern.contains(&3) { 4 } else { 3 };
        let mut params = super::super::tests::synthetic_params(&sensor);
        params.demosaic = mode;
        params.green_merge = false;
        let mut frame: RawFrame = bytemuck::Zeroable::zeroed();
        frame.width = size;
        frame.height = size;
        frame.cfa_rows = rows;
        frame.cfa_columns = columns;
        frame.colors = sensor.colors;
        let input: Vec<u32> = (0..size * size)
            .flat_map(|p| {
                let x = p % size;
                let y = p / size;
                let f = u32::from(pattern[((y % rows) * columns + x % columns) as usize]);
                (0..4).map(move |c| {
                    if c == f {
                        (x * 997 + y * 1731 + (x * y % 31) * 103 + f * 193) & 65535
                    } else {
                        0
                    }
                })
            })
            .collect();
        let image = RawBuffer::upload(
            &gpu.device,
            "test_full_demosaic",
            bytemuck::cast_slice(&input),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let codes: Vec<u32> = pattern.iter().map(|&c| u32::from(c)).collect();
        let cfa = RawBuffer::upload(
            &gpu.device,
            "test_cfa",
            bytemuck::cast_slice(&codes),
            wgpu::BufferUsages::STORAGE,
        );
        let xyz = RawBuffer::upload(
            &gpu.device,
            "test_xyz",
            bytemuck::cast_slice(&sensor.xyz_camera),
            wgpu::BufferUsages::STORAGE,
        );
        let cbrt = RawBuffer::zeros(&gpu.device, "test_cbrt", 65536 * 4);
        let frame_buf = RawBuffer::upload(
            &gpu.device,
            "test_frame",
            bytemuck::bytes_of(&frame),
            wgpu::BufferUsages::UNIFORM,
        );
        let setup = gpu.raw_dispatch(CBRT_SHADER, &[&cbrt], &[WRITE], 65536, "test_cbrt");
        let plane = gpu
            .raw_allocate_plane(size, size, "test_paged_demosaic")
            .unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        setup.encode(&mut encoder, "test_cbrt");
        plane
            .copy_window_from(&mut encoder, 0, 0, size, size, &image.buffer, size, 0, 0)
            .unwrap();
        let (passes, status) = gpu
            .raw_demosaic_gpu(
                mode,
                sensor.colors,
                &frame,
                &frame_buf,
                &image,
                &cfa,
                &xyz,
                &cbrt,
            )
            .unwrap();
        for pass in &passes {
            pass.encode(&mut encoder, "test_full_demosaic");
        }
        gpu.queue.submit(Some(encoder.finish()));
        if let Some(status) = status {
            gpu.raw_xtrans_check(&status).unwrap();
        }
        let tiled = gpu
            .raw_demosaic_paged_with_limit(&sensor, &params, plane, &cfa, &xyz, &cbrt, limit)
            .unwrap();
        let expected = gpu
            .pipe
            .readback_buffer_as::<u32>(&image.buffer, u64::from(size) * u64::from(size) * 16)
            .unwrap();
        let actual: Vec<u32> = tiled
            .pages
            .iter()
            .flat_map(|p| {
                gpu.pipe
                    .readback_buffer_as::<u32>(&p.buffer, u64::from(p.n_pixels) * 16)
                    .unwrap()
            })
            .collect();
        assert_eq!(actual, expected, "{mode:?}, {size}-square, limit {limit}");
    }

    #[test]
    fn bayer_demosaic_small_bindings_match_full_integer_image() {
        let gpu = crate::test_support::adapter();
        for size in [1, 7, 8, 71, 73, 77] {
            compare(
                &gpu,
                RawDemosaic::BayerAhd,
                size,
                &[0, 1, 1, 2],
                2,
                2,
                32768,
            );
            compare(&gpu, RawDemosaic::FujiPpg, size, &[1, 2, 0, 1], 2, 2, 32768);
            compare(
                &gpu,
                RawDemosaic::Vng,
                size,
                &[0, 0, 1, 2, 3, 1, 2, 3, 0, 1, 2, 3, 1, 2, 3, 0],
                4,
                4,
                32768,
            );
            compare(&gpu, RawDemosaic::None, size, &[0, 1, 1, 2], 2, 2, 32768);
        }
    }

    #[test]
    fn xtrans_cancel_between_windows_keeps_device_reusable() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        let gpu = crate::test_support::adapter();
        let mut sensor = super::super::tests::synthetic_sensor(128);
        sensor.cfa = vec![
            1, 0, 1, 1, 2, 1, 2, 1, 2, 0, 1, 0, 1, 0, 1, 1, 2, 1, 1, 2, 1, 1, 0, 1, 0, 1, 0, 2, 1,
            2, 1, 2, 1, 1, 0, 1,
        ];
        sensor.cfa_rows = 6;
        sensor.cfa_columns = 6;
        let mut params = super::super::tests::synthetic_params(&sensor);
        params.demosaic = RawDemosaic::XTrans;
        let codes: Vec<u32> = sensor.cfa.iter().map(|v| u32::from(*v)).collect();
        let cfa = RawBuffer::upload(
            &gpu.device,
            "cancel_xtrans_cfa",
            bytemuck::cast_slice(&codes),
            wgpu::BufferUsages::STORAGE,
        );
        let dummy = RawBuffer::zeros(&gpu.device, "cancel_xtrans_dummy", 16);
        let signal = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicUsize::new(0));
        gpu.pipe.test_on_readback({
            let signal = signal.clone();
            let count = count.clone();
            move || {
                count.fetch_add(1, Ordering::Relaxed);
                signal.store(true, Ordering::Relaxed);
            }
        });
        let mut operation = gpu.derived_explicit();
        operation.pipe = operation.pipe.with_cancellation(Some(signal.clone()));
        operation.cancellation = Some(signal);
        let plane = gpu
            .raw_allocate_plane(128, 128, "cancel_xtrans_source")
            .unwrap();
        assert_eq!(
            operation
                .raw_demosaic_paged_with_limit(
                    &sensor,
                    &params,
                    plane,
                    &cfa,
                    &dummy,
                    &dummy,
                    96 * 96 * 128
                )
                .err()
                .unwrap(),
            "operation cancelled"
        );
        assert_eq!(count.load(Ordering::Relaxed), 1);
        gpu.pipe.test_on_readback(|| {});
        compare(
            &gpu,
            RawDemosaic::XTrans,
            64,
            &sensor.cfa,
            6,
            6,
            96 * 96 * 128,
        );
    }

    #[test]
    fn xtrans_paged_matches_parallel_full_frame_at_seams() {
        let gpu = crate::test_support::adapter();
        let pattern = [
            1, 0, 1, 1, 2, 1, 2, 1, 2, 0, 1, 0, 1, 0, 1, 1, 2, 1, 1, 2, 1, 1, 0, 1, 0, 1, 0, 2, 1,
            2, 1, 2, 1, 1, 0, 1,
        ];
        compare(
            &gpu,
            RawDemosaic::XTrans,
            128,
            &pattern,
            6,
            6,
            96 * 96 * 128,
        );
        // Smaller windows exercise CFA alignment and clipped final cores at
        // every translation of the sensor pattern.
        for shift in 0..36 {
            let shifted: Vec<u8> = (0..36)
                .map(|i| pattern[((i / 6 + shift / 6) % 6) * 6 + (i % 6 + shift % 6) % 6])
                .collect();
            compare(&gpu, RawDemosaic::XTrans, 70, &shifted, 6, 6, 64 * 64 * 128);
        }
        // A different final partition exercises clipped halos.
        compare(
            &gpu,
            RawDemosaic::XTrans,
            130,
            &pattern,
            6,
            6,
            96 * 96 * 128,
        );
    }
}
