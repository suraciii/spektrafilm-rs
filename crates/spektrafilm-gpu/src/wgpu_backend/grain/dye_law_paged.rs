//! Bands of the Dye grain law transported through pages.
//!
//! The value-noise field is pointwise in global source coordinates, so a band
//! needs no blur halo. Its source window still includes one source cell around
//! the readout footprint for bilinear resampling. The readout grid is applied
//! after the developed density is written, so a crop, preview scale, or stage
//! split samples one film.

use super::dye_law::{LawBand, LawGeom, LawPipelines, READOUT_WGSL, band_bytes};
use super::*;
use crate::wgpu_backend::film_chain::device_image::{BufferPayload, ImagePage};
use spektrafilm_backend::dye_params::law;

/// Band working set the plan is allowed to hold at once.
const BAND_BUDGET_BYTES: u64 = 256 << 20;

struct LawPiece {
    /// Source window, in film-lattice pixels.
    window: [u32; 4],
    grid_x: u32,
    grid_y: u32,
    grid_cols: u32,
    grid_rows: u32,
    band: LawBand,
    geometry: LawGeom,
    readout: wgpu::Buffer,
}

pub(in crate::wgpu_backend) struct DyeLawState {
    source: std::sync::Arc<BufferPayload>,
    pieces: Vec<LawPiece>,
    pipes: LawPipelines,
    readout_pipe: CachedPipelineRef,
    grid: [u32; 4],
    grid_origin: [f64; 2],
    grid_pitch: [f64; 2],
    source_pitch: f64,
    pub(in crate::wgpu_backend) output_pages: Vec<ImagePage>,
    status: wgpu::Buffer,
}

impl DyeLawState {
    pub(in crate::wgpu_backend) fn status_buffer(&self) -> wgpu::Buffer {
        self.status.clone()
    }

    pub(in crate::wgpu_backend) fn output_pages(&self) -> Vec<ImagePage> {
        self.output_pages.clone()
    }

    pub(in crate::wgpu_backend) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        for piece in &self.pieces {
            backend.check_cancelled()?;
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            backend.copy_window_to(
                &mut encoder,
                &self.source,
                piece.window[0],
                piece.window[1],
                piece.window[2],
                piece.window[3],
                piece.band.buffer(),
            )?;
            backend.encode_law_band(&mut encoder, &piece.band, &piece.geometry, &self.pipes);
            backend.encode_law_readout(
                &mut encoder,
                &self.readout_pipe,
                piece.band.out_buffer(),
                &piece.readout,
                piece.window,
                [piece.grid_cols, piece.grid_rows],
                [piece.grid_x, piece.grid_y],
                [self.grid[2], self.grid[3]],
                self.source_pitch,
                self.grid_pitch,
                self.grid_origin,
            );
            let inner = u64::from(piece.grid_cols) * 3;
            let destination_sample =
                (u64::from(piece.grid_y) * u64::from(self.grid[2]) + u64::from(piece.grid_x)) * 3;
            backend.copy_samples_from(
                &mut encoder,
                &piece.readout,
                0,
                &self.output_pages,
                destination_sample,
                inner * u64::from(piece.grid_rows),
            )?;
            backend.submit_commands(Some(encoder.finish()))?;
        }
        backend.check_cancelled()?;
        Ok(())
    }
}

/// Rows of window one band may hold at this width.
pub(in crate::wgpu_backend) fn law_band_shape(
    backend: &WgpuBackend,
    source_width: u32,
    grid_width: u32,
) -> Result<u32, String> {
    let binding = crate::wgpu_backend::bounded::storage_binding_limit(backend)?;
    let budget = BAND_BUDGET_BYTES.min(binding);
    let source_width = u64::from(source_width.max(1));
    let grid_width = u64::from(grid_width.max(1));
    let per_row = band_bytes(source_width)
        .checked_add(
            source_width
                .checked_mul(12)
                .ok_or("Dye grain law: band overflow")?,
        )
        .and_then(|v| v.checked_add(grid_width.checked_mul(12)?))
        .ok_or("Dye grain law: band overflow")?;
    Ok((budget / per_row).max(1).min(u64::from(u32::MAX)) as u32)
}

/// Working set of the law: one band window, published pages, and every piece's
/// readout buffer. Readout buffers are retained until submission, so their
/// total is the full output-grid size rather than one band.
pub(in crate::wgpu_backend) fn law_workspace_bytes(
    backend: &WgpuBackend,
    source_width: u32,
    source_height: u32,
    grid_width: u32,
    grid_height: u32,
) -> Result<u64, String> {
    let rows = law_band_shape(backend, source_width, grid_width)?.min(source_height.max(1));
    let window = u64::from(source_width.max(1))
        .checked_mul(u64::from(rows))
        .ok_or("Dye grain law: band window overflow")?;
    let pages = u64::from(grid_width)
        .checked_mul(u64::from(grid_height))
        .and_then(|v| v.checked_mul(12))
        .ok_or("Dye grain law: output pages overflow")?;
    band_bytes(window)
        .checked_add(
            window
                .checked_mul(12)
                .ok_or("Dye grain law: band overflow")?,
        )
        .and_then(|v| v.checked_add(pages))
        .and_then(|v| v.checked_add(pages))
        .ok_or_else(|| "Dye grain law: workspace overflow".to_owned())
}

impl WgpuBackend {
    pub(in crate::wgpu_backend) fn prepare_dye_law(
        &self,
        width: u32,
        height: u32,
        input: &BufferPayload,
        params: &DyeGpuParams,
    ) -> Result<DyeLawState, String> {
        let rows_per_band = law_band_shape(self, width, params.grid.width)?;
        self.prepare_dye_law_with_rows(width, height, input, params, rows_per_band)
    }

    /// Build a field state with an explicit band height. The public entry point
    /// derives this from the device binding limit; tests use it to prove that
    /// band partitioning does not change the realization.
    fn prepare_dye_law_with_rows(
        &self,
        width: u32,
        height: u32,
        input: &BufferPayload,
        params: &DyeGpuParams,
        rows_per_band: u32,
    ) -> Result<DyeLawState, String> {
        if input.width != width || input.height != height {
            return Err(
                "Dye grain law: the source raster does not match the declared geometry".into(),
            );
        }
        let source_pitch = params.source_pitch_um;
        if !(source_pitch.is_finite() && source_pitch > 0.0) {
            return Err("Dye grain law: the source pitch must be positive".into());
        }
        if !(params.length_scale.is_finite() && params.length_scale > 0.0) {
            return Err("Dye grain law: the correlation-length scale must be positive".into());
        }
        let grid = &params.grid;
        let base_px = law::base_cell_px(params.length_scale, source_pitch);
        let output_pages = self.allocate_image_pages(grid.width, grid.height, "dye_law_output")?;
        let pipes = self.dye_law_pipelines();
        let readout_pipe = self.cached_pipeline_at(
            READOUT_WGSL,
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            "main",
        );
        let mut pieces = Vec::new();
        let mut y = 0u32;
        while y < grid.height {
            self.check_cancelled()?;
            let grid_rows = rows_per_band.min(grid.height - y);
            let top_film = grid.origin_um[1] + y as f64 * grid.pitch_um[1];
            let bottom_film = grid.origin_um[1] + (y + grid_rows) as f64 * grid.pitch_um[1];
            let left_film = grid.origin_um[0];
            let right_film = grid.origin_um[0] + grid.width as f64 * grid.pitch_um[0];
            let to_px = |um: f64| um / source_pitch;
            // A readout window entirely past a source edge still clamps to the
            // last source sample, exactly as the readout shader does. The extra
            // source cell on each side is the bilinear readout support, not a
            // field blur.
            let window_y = (to_px(top_film).floor().max(0.0) as u32)
                .saturating_sub(1)
                .min(height.saturating_sub(1));
            let window_bottom = (to_px(bottom_film).ceil() as u32)
                .saturating_add(1)
                .min(height);
            let window_x = (to_px(left_film).floor().max(0.0) as u32)
                .saturating_sub(1)
                .min(width.saturating_sub(1));
            let window_right = (to_px(right_film).ceil() as u32)
                .saturating_add(1)
                .min(width);
            let window_w = window_right.saturating_sub(window_x).max(1);
            let window_h = window_bottom.saturating_sub(window_y).max(1);
            let band = LawBand::new(self, window_w, window_h, "dye_law_band", None);
            let readout = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dye_law_readout_band"),
                size: u64::from(grid.width) * u64::from(grid_rows) * 12,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            pieces.push(LawPiece {
                window: [window_x, window_y, window_w, window_h],
                grid_x: 0,
                grid_y: y,
                grid_cols: grid.width,
                grid_rows,
                band,
                geometry: LawGeom {
                    origin_x: window_x,
                    origin_y: window_y,
                    seed: params.seed,
                    expectation: params.expectation,
                    monochrome: params.monochrome,
                    white: params.white,
                    event_area_um2: params.event_area_um2,
                    pitch_um: source_pitch,
                    base_px,
                },
                readout,
            });
            y += grid_rows;
        }
        let status = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_law_status"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        status.unmap();
        Ok(DyeLawState {
            source: std::sync::Arc::new(BufferPayload::from_pages(
                self.backend_id(),
                input.pages.clone(),
                width,
                height,
            )),
            pieces,
            pipes,
            readout_pipe,
            grid: [grid.width, grid.height, grid.width, grid.height],
            grid_origin: grid.origin_um,
            grid_pitch: grid.pitch_um,
            source_pitch,
            output_pages,
            status,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_helpers::{f32_to_scalars, scalars_to_f32};
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::{ZERO, from_f64, to_f64};

    fn upload(backend: &WgpuBackend, image: &ImageBuf) -> BufferPayload {
        let pages = backend
            .allocate_image_pages(image.width, image.height, "dye_band_probe")
            .unwrap();
        let values = scalars_to_f32(&image.data);
        for page in &pages {
            let first = page.first_pixel as usize * 3;
            let end = first + page.n_pixels as usize * 3;
            backend
                .queue
                .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&values[first..end]));
        }
        BufferPayload::from_pages(backend.backend_id(), pages, image.width, image.height)
    }

    fn materialize(
        backend: &WgpuBackend,
        pages: &[ImagePage],
        width: u32,
        height: u32,
    ) -> ImageBuf {
        let mut values = vec![ZERO; width as usize * height as usize * 3];
        for page in pages {
            let readback: Vec<f32> = backend
                .pipe
                .readback_buffer_cancellable(&page.buffer, u64::from(page.n_pixels) * 12)
                .unwrap();
            let readback = f32_to_scalars(readback);
            let first = page.first_pixel as usize * 3;
            values[first..first + readback.len()].copy_from_slice(&readback);
        }
        ImageBuf::from_data(width, height, values)
    }

    fn run(backend: &WgpuBackend, image: &ImageBuf, params: &DyeGpuParams, rows: u32) -> ImageBuf {
        let source = upload(backend, image);
        let state = backend
            .prepare_dye_law_with_rows(image.width, image.height, &source, params, rows)
            .unwrap();
        state.submit(backend).unwrap();
        materialize(
            backend,
            &state.output_pages(),
            params.grid.width,
            params.grid.height,
        )
    }

    fn params() -> DyeGpuParams {
        DyeGpuParams {
            source_pitch_um: 5.0,
            grid: spektrafilm_backend::dye_params::DyeReadoutGrid {
                origin_um: [0.0, 0.0],
                pitch_um: [5.0, 5.0],
                width: 64,
                height: 64,
            },
            seed: 7,
            expectation: false,
            monochrome: false,
            white: law::WHITE,
            event_area_um2: law::EVENT_AREA_UM2,
            length_scale: 1.0,
        }
    }

    #[test]
    fn coarse_readout_with_a_nonzero_window_origin_uses_global_source_cells() {
        let gpu = crate::test_support::adapter();
        let source = ImageBuf::from_data(
            64,
            64,
            (0..64 * 64 * 3)
                .map(|i| from_f64(0.3 + (i % 7) as f64 * 0.02))
                .collect(),
        );
        let mut p = params();
        p.grid = spektrafilm_backend::dye_params::DyeReadoutGrid {
            origin_um: [10.0, 10.0],
            pitch_um: [10.0, 10.0],
            width: 8,
            height: 8,
        };
        p.expectation = true;
        let out = run(&gpu, &source, &p, 3);
        let source_pitch = p.source_pitch_um;
        let origin = p.grid.origin_um;
        let pitch = p.grid.pitch_um;
        let mut worst = 0.0f64;
        for y in 0..p.grid.height {
            for x in 0..p.grid.width {
                let first_x = (origin[0] + x as f64 * pitch[0]) / source_pitch;
                let first_y = (origin[1] + y as f64 * pitch[1]) / source_pitch;
                let mut acc = [0.0f64; 3];
                for sy in 0..2 {
                    for sx in 0..2 {
                        let ix = (first_x.floor() as u32 + sx).min(63);
                        let iy = (first_y.floor() as u32 + sy).min(63);
                        for (c, acc_c) in acc.iter_mut().enumerate() {
                            *acc_c += spektrafilm_math::precision::to_f64(
                                source.data[(iy as usize * 64 + ix as usize) * 3 + c],
                            );
                        }
                    }
                }
                for (c, acc_c) in acc.iter().enumerate() {
                    let expected = *acc_c / 4.0;
                    let got = to_f64(out.data[((y * p.grid.width + x) * 3) as usize + c]);
                    worst = worst.max((expected - got).abs());
                }
            }
        }
        assert!(
            worst < 1e-5,
            "readout disagrees with global source cells: {worst}"
        );
    }

    #[test]
    fn sub_lattice_readout_with_a_nonzero_window_origin_uses_global_source_cells() {
        let gpu = crate::test_support::adapter();
        let source = ImageBuf::from_data(
            64,
            64,
            (0..64 * 64 * 3)
                .map(|i| from_f64(0.3 + (i % 7) as f64 * 0.02))
                .collect(),
        );
        let mut p = params();
        p.grid = spektrafilm_backend::dye_params::DyeReadoutGrid {
            origin_um: [10.0, 10.0],
            pitch_um: [2.5, 2.5],
            width: 8,
            height: 8,
        };
        p.expectation = true;
        let out = run(&gpu, &source, &p, 3);
        let source_pitch = p.source_pitch_um;
        let origin = p.grid.origin_um;
        let pitch = p.grid.pitch_um;
        let mut worst = 0.0f64;
        for y in 0..p.grid.height {
            for x in 0..p.grid.width {
                let first_x = (origin[0] + x as f64 * pitch[0]) / source_pitch;
                let last_x = first_x + pitch[0] / source_pitch;
                let first_y = (origin[1] + y as f64 * pitch[1]) / source_pitch;
                let last_y = first_y + pitch[1] / source_pitch;
                let sx = ((first_x + last_x) * 0.5 - 0.5).clamp(0.0, 63.0);
                let sy = ((first_y + last_y) * 0.5 - 0.5).clamp(0.0, 63.0);
                let x0 = sx.floor() as u32;
                let y0 = sy.floor() as u32;
                let x1 = (x0 + 1).min(63);
                let y1 = (y0 + 1).min(63);
                let wx = sx - x0 as f64;
                let wy = sy - y0 as f64;
                for c in 0..3 {
                    let p00 = spektrafilm_math::precision::to_f64(
                        source.data[((y0 * 64 + x0) * 3) as usize + c],
                    );
                    let p10 = spektrafilm_math::precision::to_f64(
                        source.data[((y0 * 64 + x1) * 3) as usize + c],
                    );
                    let p01 = spektrafilm_math::precision::to_f64(
                        source.data[((y1 * 64 + x0) * 3) as usize + c],
                    );
                    let p11 = spektrafilm_math::precision::to_f64(
                        source.data[((y1 * 64 + x1) * 3) as usize + c],
                    );
                    let expected = p00
                        + (p10 - p00) * wx
                        + ((p01 + (p11 - p01) * wx) - (p00 + (p10 - p00) * wx)) * wy;
                    let got = to_f64(out.data[((y * p.grid.width + x) * 3) as usize + c]);
                    worst = worst.max((expected - got).abs());
                }
            }
        }
        assert!(
            worst < 1e-5,
            "readout disagrees with global source cells: {worst}"
        );
    }

    #[test]
    fn band_split_preserves_the_device_field_and_count() {
        let gpu = crate::test_support::adapter();
        let image = ImageBuf::from_data(
            64,
            64,
            (0..64 * 64 * 3)
                .map(|i| from_f64(0.3 + (i % 7) as f64 * 0.02))
                .collect(),
        );
        let p = params();
        let whole = run(&gpu, &image, &p, 64);
        let split = run(&gpu, &image, &p, 7);
        let mut worst = 0.0f64;
        let mut at = 0;
        for (i, (a, b)) in whole.data.iter().zip(split.data.iter()).enumerate() {
            let d = (to_f64(*a) - to_f64(*b)).abs();
            if d > worst {
                worst = d;
                at = i;
            }
        }
        eprintln!("band split worst {worst:.3e} at sample {at}");
        assert!(worst <= 1e-5, "band split changed the field: {worst:.3e}");
    }
}
