//! Bounded transport around the established integer demosaic kernels.
//!
//! Bayer windows commit only cores whose complete dependency graph is present.
//! X-Trans instead retains native 512-square tile order and scratch state, with
//! a resumable device bounds scan before the first tile. Neither path invents
//! CFA phase or image borders at a storage-page boundary.
use super::paged::RawPlane;
use super::*;

const READ: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: true };
const WRITE: wgpu::BufferBindingType = wgpu::BufferBindingType::Storage { read_only: false };
const UNIFORM: wgpu::BufferBindingType = wgpu::BufferBindingType::Uniform;

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
        if params.demosaic == RawDemosaic::XTrans {
            return self.raw_xtrans_paged(plane, &frame, cfa, xyz, cbrt, limit, false);
        }
        // AHD: green(2) -> RGB(1) -> homogeneity(1) -> combine(1).
        // VNG: linear(1) -> gradients/second-neighbour averages(2).
        // PPG: directional green(3) -> green-site chroma(1) -> diagonal chroma(1).
        let halo = match params.demosaic {
            RawDemosaic::BayerAhd => 5,
            RawDemosaic::Vng => 3,
            RawDemosaic::FujiPpg => 5,
            _ => unreachable!(),
        };
        let step = demosaic_step(limit, halo, sensor.cfa_rows, sensor.cfa_columns)?;
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
                let (passes, _) = self.raw_demosaic_gpu(
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

    // Native tile transport needs the separate kernel bindings and sparse-path control.
    #[allow(clippy::too_many_arguments)]
    fn raw_xtrans_paged(
        &self,
        plane: RawPlane,
        frame: &RawFrame,
        cfa: &RawBuffer,
        xyz: &RawBuffer,
        cbrt: &RawBuffer,
        limit: u64,
        force_sparse: bool,
    ) -> Result<RawPlane, String> {
        if frame.width < 512 || frame.height < 512 {
            return Err("LibRaw Markesteijn requires width and height of at least 512".into());
        }
        if frame.cfa_rows != 6 || frame.cfa_columns != 6 {
            return Err("LibRaw Markesteijn requires a 6-by-6 X-Trans CFA".into());
        }
        // PATT's image offsets are at most two rows/columns; the tile reads
        // three times those offsets, so a native tile needs six source pixels.
        const HALO: u32 = 6;
        let native_bytes = u64::from((512 + 2 * HALO).min(frame.width))
            * u64::from((512 + 2 * HALO).min(frame.height))
            * 16;
        if native_bytes > limit {
            return Err(format!(
                "RAW X-Trans native 512-tile image window requires {native_bytes} bytes; limit is {limit}"
            ));
        }
        if let Some(reason) = required_buffer_exceeds_limits(
            super::demosaic::XTRANS_SCRATCH_BYTES,
            &self.device.limits(),
        ) {
            return Err(format!("RAW X-Trans native tile scratch: {reason}"));
        }
        let metadata = RawBuffer::zeros(&self.device, "raw_xtrans_metadata", 147 * 4);
        let frame_buf = RawBuffer::upload(
            &self.device,
            "raw_xtrans_frame",
            bytemuck::bytes_of(frame),
            wgpu::BufferUsages::UNIFORM,
        );
        let dummy = RawBuffer::zeros(&self.device, "raw_xtrans_metadata_dummy", 16);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        // Stage 1 contains metadata only; stage 2 also applies native global
        // source-bound checks after the (separately resumed) bounds scan.
        for stage in [1, 2] {
            let settings = self.demosaic_settings(frame.colors, stage, 0, 0);
            self.raw_dispatch(
                XTRANS_PREPARE,
                &[&frame_buf, &settings, &dummy, cfa, &metadata],
                &[UNIFORM, UNIFORM, WRITE, READ, WRITE],
                1,
                "raw_xtrans_metadata",
            )
            .encode(&mut encoder, "raw_xtrans_metadata");
        }
        self.raw_submit(encoder)?;
        self.raw_xtrans_check(&metadata)?;
        let cursor = RawBuffer::upload(
            &self.device,
            "raw_xtrans_cursor",
            bytemuck::cast_slice(&[2u32, 2, 65535, 0, 0, 0, 0, 0]),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let capacity = (limit / 16).min(u64::from(u32::MAX)) as u32;
        let mut position = [2u32, 2];
        loop {
            self.check_cancelled()?;
            // Actual allhex offsets are signed shorts, even on wide images.
            // Gather the flat interval rather than assuming spatial locality.
            let pixel = u64::from(position[1]) * u64::from(frame.width) + u64::from(position[0]);
            let first = pixel.saturating_sub(32769);
            let count = u64::from(capacity)
                .min(u64::from(frame.width) * u64::from(frame.height) - first)
                as u32;
            let window = Window {
                x: first as u32,
                y: 0,
                width: count,
                height: 1,
            };
            let image = self.raw_window_buffer(window, "raw_xtrans_bounds_window");
            let window_buf = RawBuffer::upload(
                &self.device,
                "raw_xtrans_bounds_extent",
                bytemuck::bytes_of(&window),
                wgpu::BufferUsages::UNIFORM,
            );
            let mut encoder = self.device.create_command_encoder(&Default::default());
            plane.copy_range(&mut encoder, first, count, &image.buffer, 0, false);
            self.raw_dispatch(
                XTRANS_BOUNDS,
                &[&frame_buf, &window_buf, &image, cfa, &metadata, &cursor],
                &[UNIFORM, UNIFORM, WRITE, READ, WRITE, WRITE],
                1,
                "raw_xtrans_bounds",
            )
            .encode(&mut encoder, "raw_xtrans_bounds");
            plane.copy_range(&mut encoder, first, count, &image.buffer, 0, true);
            self.raw_submit(encoder)?;
            self.raw_xtrans_check(&metadata)?;
            let control = self
                .pipe
                .readback_buffer_cancellable::<u32>(&cursor.buffer, 32)?;
            if control[5] != 0 {
                break;
            }
            if position == [control[0], control[1]] {
                return Err(
                    "RAW X-Trans bounds cursor cannot fit its native dependency window".into(),
                );
            }
            position = [control[0], control[1]];
        }
        // Scratch is intentionally retained across tiles: native padding and
        // the image's in-place overlap both participate in subsequent tiles.
        let scratch = RawBuffer::zeros(
            &self.device,
            "raw_xtrans_native_scratch",
            super::demosaic::XTRANS_SCRATCH_BYTES,
        );
        for top in (3..frame.height - 19).step_by(496) {
            for left in (3..frame.width - 19).step_by(496) {
                self.check_cancelled()?;
                let x = left.saturating_sub(HALO);
                let y = top.saturating_sub(HALO);
                let window = Window {
                    x,
                    y,
                    width: (left + 512 + HALO).min(frame.width) - x,
                    height: (top + 512 + HALO).min(frame.height) - y,
                };
                let image = self.raw_window_buffer(window, "raw_xtrans_native_window");
                let extent = RawBuffer::upload(
                    &self.device,
                    "raw_xtrans_native_extent",
                    bytemuck::bytes_of(&window),
                    wgpu::BufferUsages::UNIFORM,
                );
                let sparse = force_sparse || frame.width >= 16383;
                if sparse {
                    self.raw_xtrans_sparse_predictions(
                        &plane, frame, &metadata, cfa, &scratch, top, left, limit,
                    )?;
                }
                let settings = self.demosaic_settings(frame.colors, u32::from(sparse), top, left);
                let mut encoder = self.device.create_command_encoder(&Default::default());
                plane.copy_window_to(
                    &mut encoder,
                    x,
                    y,
                    window.width,
                    window.height,
                    &image.buffer,
                )?;
                self.raw_dispatch(
                    XTRANS_WINDOW,
                    &[
                        &frame_buf, &settings, &image, cfa, &metadata, &scratch, xyz, cbrt, &extent,
                    ],
                    &[
                        UNIFORM, UNIFORM, WRITE, READ, READ, WRITE, READ, READ, UNIFORM,
                    ],
                    1,
                    "raw_xtrans_native_tile",
                )
                .encode(&mut encoder, "raw_xtrans_native_tile");
                plane.copy_window_from(
                    &mut encoder,
                    x,
                    y,
                    window.width,
                    window.height,
                    &image.buffer,
                    window.width,
                    0,
                    0,
                )?;
                self.raw_submit(encoder)?;
            }
        }
        // The native final border reads only measured CFA channels, so these
        // writes commute; gather one-pixel halos and keep only global borders.
        self.raw_xtrans_border(plane, frame, cfa, limit)
    }

    // Tile origin, storage limit, and independent prediction bindings stay explicit.
    #[allow(clippy::too_many_arguments)]
    fn raw_xtrans_sparse_predictions(
        &self,
        plane: &RawPlane,
        frame: &RawFrame,
        metadata: &RawBuffer,
        cfa: &RawBuffer,
        scratch: &RawBuffer,
        top: u32,
        left: u32,
        limit: u64,
    ) -> Result<(), String> {
        let capacity = (limit / (16 * 16)).min(262144) as u32;
        if capacity == 0 {
            return Err("RAW X-Trans sparse prediction cannot fit one source group".into());
        }
        let frame_buf = RawBuffer::upload(
            &self.device,
            "raw_prediction_frame",
            bytemuck::bytes_of(frame),
            wgpu::BufferUsages::UNIFORM,
        );
        for first in (0..262144u32).step_by(capacity as usize) {
            self.check_cancelled()?;
            let count = capacity.min(262144 - first);
            let indices = RawBuffer::zeros(
                &self.device,
                "raw_prediction_requests",
                u64::from(count) * 16 * 4,
            );
            let values = RawBuffer::zeros(
                &self.device,
                "raw_prediction_values",
                u64::from(count) * 16 * 16,
            );
            let bounds = RawBuffer::upload(
                &self.device,
                "raw_prediction_bounds",
                bytemuck::cast_slice(&[u32::MAX, 0]),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            );
            let settings = RawBuffer::upload(
                &self.device,
                "raw_prediction_settings",
                bytemuck::cast_slice(&[top, left, first, count, 0, 0, 0, 0]),
                wgpu::BufferUsages::UNIFORM,
            );
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.raw_dispatch(
                XTRANS_PREDICTIONS,
                &[
                    &frame_buf, &settings, metadata, cfa, &indices, &bounds, &values, scratch,
                ],
                &[UNIFORM, UNIFORM, READ, READ, WRITE, WRITE, WRITE, WRITE],
                count,
                "raw_xtrans_prediction_requests",
            )
            .encode(&mut encoder, "raw_xtrans_prediction_requests");
            self.raw_submit(encoder)?;
            self.raw_gather_requests_gpu(
                &indices,
                &bounds,
                &values,
                count * 16,
                4,
                plane
                    .pages
                    .iter()
                    .map(|p| (p.first_pixel, p.n_pixels, &p.buffer)),
            )?;
            let settings = RawBuffer::upload(
                &self.device,
                "raw_prediction_settings",
                bytemuck::cast_slice(&[top, left, first, count, 1, 0, 0, 0]),
                wgpu::BufferUsages::UNIFORM,
            );
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.raw_dispatch(
                XTRANS_PREDICTIONS,
                &[
                    &frame_buf, &settings, metadata, cfa, &indices, &bounds, &values, scratch,
                ],
                &[UNIFORM, UNIFORM, READ, READ, WRITE, WRITE, WRITE, WRITE],
                count,
                "raw_xtrans_prediction_values",
            )
            .encode(&mut encoder, "raw_xtrans_prediction_values");
            self.raw_submit(encoder)?;
        }
        Ok(())
    }

    fn raw_xtrans_check(&self, metadata: &RawBuffer) -> Result<(), String> {
        let values = self
            .pipe
            .readback_buffer_cancellable::<i32>(&metadata.buffer, 147 * 4)?;
        if values[146] != 0 {
            Err("LibRaw Markesteijn rejected corrupt X-Trans pattern or source bounds".into())
        } else {
            Ok(())
        }
    }

    fn raw_xtrans_border(
        &self,
        plane: RawPlane,
        frame: &RawFrame,
        cfa: &RawBuffer,
        limit: u64,
    ) -> Result<RawPlane, String> {
        let side = ((limit / 16) as f64).sqrt().floor() as u32;
        let step = side - 2;
        for y in (0..plane.height).step_by(step as usize) {
            for x in (0..plane.width).step_by(step as usize) {
                self.check_cancelled()?;
                let w = step.min(plane.width - x);
                let h = step.min(plane.height - y);
                if x >= 8 && y >= 8 && x + w <= plane.width - 8 && y + h <= plane.height - 8 {
                    continue;
                }
                let left = x.saturating_sub(1);
                let top = y.saturating_sub(1);
                let window = Window {
                    x: left,
                    y: top,
                    width: (x + w + 1).min(plane.width) - left,
                    height: (y + h + 1).min(plane.height) - top,
                };
                let image = self.raw_window_buffer(window, "raw_xtrans_border_window");
                let extent = RawBuffer::upload(
                    &self.device,
                    "raw_xtrans_border_extent",
                    bytemuck::bytes_of(&window),
                    wgpu::BufferUsages::UNIFORM,
                );
                let frame_buf = RawBuffer::upload(
                    &self.device,
                    "raw_xtrans_border_frame",
                    bytemuck::bytes_of(frame),
                    wgpu::BufferUsages::UNIFORM,
                );
                let mut encoder = self.device.create_command_encoder(&Default::default());
                plane.copy_window_to(
                    &mut encoder,
                    left,
                    top,
                    window.width,
                    window.height,
                    &image.buffer,
                )?;
                self.raw_dispatch(
                    XTRANS_BORDER,
                    &[&frame_buf, &extent, &image, cfa],
                    &[UNIFORM, UNIFORM, WRITE, READ],
                    window.width * window.height,
                    "raw_xtrans_border",
                )
                .encode(&mut encoder, "raw_xtrans_border");
                plane.copy_window_from(
                    &mut encoder,
                    x,
                    y,
                    w,
                    h,
                    &image.buffer,
                    window.width,
                    x - left,
                    y - top,
                )?;
                self.raw_submit(encoder)?;
            }
        }
        Ok(plane)
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

const XTRANS_PREPARE: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_xtrans_prepare.wgsl")
);
const XTRANS_BOUNDS: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_xtrans_bounds_paged.wgsl")
);
const XTRANS_WINDOW: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_xtrans_window.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_xtrans.wgsl")
);
const XTRANS_BORDER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_xtrans_border_paged.wgsl")
);
const XTRANS_PREDICTIONS: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_xtrans_predictions_paged.wgsl")
);

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
        let tiled = if mode == RawDemosaic::XTrans && size == 512 {
            gpu.raw_xtrans_paged(plane, &frame, &cfa, &xyz, &cbrt, limit, true)
        } else {
            gpu.raw_demosaic_paged_with_limit(&sensor, &params, plane, &cfa, &xyz, &cbrt, limit)
        }
        .unwrap();
        let expected = gpu
            .pipe
            .readback_buffer_as::<u32>(&image.buffer, u64::from(size) * u64::from(size) * 16);
        let actual: Vec<u32> = tiled
            .pages
            .iter()
            .flat_map(|p| {
                gpu.pipe
                    .readback_buffer_as::<u32>(&p.buffer, u64::from(p.n_pixels) * 16)
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
    fn xtrans_paged_matches_native_multitile_scan_and_overlap() {
        let gpu = crate::test_support::adapter();
        let pattern = [
            1, 0, 1, 1, 2, 1, 2, 1, 2, 0, 1, 0, 1, 0, 1, 1, 2, 1, 1, 2, 1, 1, 0, 1, 0, 1, 0, 2, 1,
            2, 1, 2, 1, 1, 0, 1,
        ];
        compare(
            &gpu,
            RawDemosaic::XTrans,
            1030,
            &pattern,
            6,
            6,
            8 * 1024 * 1024,
        );
        // Same sparse preparation as wide-stride tiles, exercised without a
        // huge test sensor. Native predictions then consume stage 1 scratch.
        compare(
            &gpu,
            RawDemosaic::XTrans,
            512,
            &pattern,
            6,
            6,
            8 * 1024 * 1024,
        );
    }
}
