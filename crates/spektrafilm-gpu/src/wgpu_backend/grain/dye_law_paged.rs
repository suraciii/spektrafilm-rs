//! Bands of the Dye grain law transported through pages.
//!
//! The law needs a filter halo, so a page cannot be processed on its own: each
//! band copies a window that includes the halo, runs the law, and writes back only
//! the inner rows. The noise is keyed on global coordinates, so the halo overlap
//! reproduces the same field and the bands join without a seam.

use super::*;
use crate::wgpu_backend::film_chain::device_image::{BufferPayload, ImagePage};
use super::dye_law::{
    LawBand, LawGeom, LawPipelines, LawTaps, RESAMPLE_WGSL, band_bytes, halo_um, law_taps,
};

/// Band working set the plan is allowed to hold at once.
const BAND_BUDGET_BYTES: u64 = 256 << 20;

struct LawPiece {
    band: LawBand,
    geometry: LawGeom,
    out_y: u32,
    out_rows: u32,
}

pub(in crate::wgpu_backend) struct DyeLawState {
    /// Flat copy of the full-source raster: the grid may crop or scale, and a
    /// resampling pass needs random access that pages cannot give.
    flat: wgpu::Buffer,
    flat_source: bool,
    resample: CachedPipelineRef,
    grid: [u32; 4],
    source_dims: [u32; 2],
    border: Option<std::sync::Arc<BufferPayload>>,
    pieces: Vec<LawPiece>,
    pipes: LawPipelines,
    taps: LawTaps,
    width: u32,
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
            if self.flat_source {
                backend.encode_law_resample(
                    &mut encoder,
                    &self.resample,
                    &self.flat,
                    piece.band.buffer(),
                    piece.band_cols(),
                    piece.band_rows(),
                    [0, piece.geometry.origin_y],
                    self.source_dims,
                    [self.grid[2], self.grid[3]],
                );
            } else {
                backend.copy_window_to(
                    &mut encoder,
                    self.border.as_ref().expect("1:1 keeps its pages"),
                    0,
                    piece.geometry.origin_y,
                    piece.band_cols(),
                    piece.band_rows(),
                    piece.band.buffer(),
                )?;
            }
            backend.encode_law_band(
                &mut encoder,
                &piece.band,
                &piece.geometry,
                &self.pipes,
                &self.taps,
            );
            // Write back the inner rows only; the rest of the window is halo.
            let inner = u64::from(piece.band_cols()) * 3;
            let source_sample = u64::from(piece.out_y - piece.geometry.origin_y)
                * u64::from(piece.band_cols())
                * 3;
            let destination_sample = u64::from(piece.out_y) * u64::from(self.width) * 3;
            backend.copy_samples_from(
                &mut encoder,
                piece.band.out_buffer(),
                source_sample,
                &self.output_pages,
                destination_sample,
                inner * u64::from(piece.out_rows),
            )?;
            backend.submit_commands(Some(encoder.finish()))?;
        }
        backend.check_cancelled()?;
        Ok(())
    }
}

impl LawPiece {
    fn band_cols(&self) -> u32 {
        self.band.cols()
    }
    fn band_rows(&self) -> u32 {
        self.band.rows()
    }
}

/// Rows of window one band may hold at this width, and the halo it needs.
pub(in crate::wgpu_backend) fn law_band_shape(
    backend: &WgpuBackend,
    width: u32,
    pitch_um: f64,
) -> Result<(u32, u32), String> {
    let binding = crate::wgpu_backend::bounded::storage_binding_limit(backend)?;
    let budget = BAND_BUDGET_BYTES.min(binding);
    let pixels = (budget / 68).max(64);
    let rows = (pixels / u64::from(width.max(1))).max(1);
    let halo = (halo_um() / pitch_um).ceil().max(1.0);
    if halo > 4096.0 {
        return Err(format!(
            "Dye grain law: a {pitch_um} um pitch needs a {halo} px halo"
        ));
    }
    Ok((rows.min(u64::from(u32::MAX)) as u32, halo as u32))
}

/// Working set of the law: one band window plus the published pages.
pub(in crate::wgpu_backend) fn law_workspace_bytes(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    pitch_um: f64,
) -> Result<u64, String> {
    let (rows, halo) = law_band_shape(backend, width, pitch_um)?;
    let window_rows = u64::from(rows) + 2 * u64::from(halo);
    let window = u64::from(width)
        .checked_mul(window_rows)
        .ok_or("Dye grain law: band window overflow")?;
    let pages = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|v| v.checked_mul(12))
        .ok_or("Dye grain law: output pages overflow")?;
    band_bytes(window)
        .checked_add(pages)
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
        if input.width != width || input.height != height {
            return Err(
                "Dye grain law: the source raster does not match the declared geometry".into(),
            );
        }
        let pitch_um = params.source_pitch_um;
        if !(pitch_um.is_finite() && pitch_um > 0.0) {
            return Err("Dye grain law: the source pitch must be positive".into());
        }
        let grid = &params.grid;
        let one_to_one = grid.width == width
            && grid.height == height
            && grid.origin_um == [0.0, 0.0]
            && grid.pitch_um == [pitch_um, pitch_um];
        let (rows_per_band, halo) = law_band_shape(self, grid.width, pitch_um)?;
        let output_pages = self.allocate_image_pages(grid.width, grid.height, "dye_law_output")?;
        let pipes = self.dye_law_pipelines();
        let taps = law_taps(pitch_um);
        let mut pieces = Vec::new();
        let mut y = 0u32;
        while y < grid.height {
            self.check_cancelled()?;
            let inner_rows = rows_per_band.min(grid.height - y);
            let top = y.saturating_sub(halo);
            let bottom = (y + inner_rows + halo).min(grid.height);
            let window_rows = bottom - top;
            let band = LawBand::new(self, grid.width, window_rows, "dye_law_band", None);
            pieces.push(LawPiece {
                band,
                geometry: LawGeom {
                    origin_x: 0,
                    origin_y: top,
                    pitch_um,
                    seed: params.seed,
                    expectation: params.expectation,
                },
                out_y: y,
                out_rows: inner_rows,
            });
            y += inner_rows;
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
        let flat_source = !one_to_one;
        let (flat, border) = if flat_source {
            let flat = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dye_law_source_flat"),
                size: u64::from(width) * u64::from(height) * 12,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.copy_window_to(&mut encoder, input, 0, 0, width, height, &flat)?;
            self.queue.submit(Some(encoder.finish()));
            (flat, None)
        } else {
            let flat = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("dye_law_source_flat_unused"),
                size: 12,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            (
                flat,
                Some(std::sync::Arc::new(BufferPayload::from_pages(
                    self.backend_id(),
                    input.pages.clone(),
                    width,
                    height,
                ))),
            )
        };
        let resample = self.cached_pipeline_at(
            RESAMPLE_WGSL,
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            "main",
        );
        Ok(DyeLawState {
            flat,
            flat_source,
            resample,
            grid: [
                grid.origin_um[0] as u32,
                grid.origin_um[1] as u32,
                grid.width,
                grid.height,
            ],
            source_dims: [width, height],
            border,
            pieces,
            pipes,
            taps,
            width,
            output_pages,
            status,
        })
    }
}
