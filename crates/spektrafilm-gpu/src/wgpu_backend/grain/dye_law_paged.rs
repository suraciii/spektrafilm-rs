//! Bands of the Dye grain law transported through pages.
//!
//! The law lives on the film lattice, so a band is a window of the *source* raster
//! with the halo its filter needs; the readout grid is applied afterwards, when the
//! developed density is written out. That ordering is what makes a crop, a preview
//! scale, or a render split into stages agree with a single full-frame pass.
//!
//! The noise is keyed on global source coordinates, so overlapping halos reproduce
//! the same field and the bands join without a seam.

use super::dye_law::{
    LawBand, LawGeom, LawPipelines, LawTaps, READOUT_WGSL, band_bytes, halo_um, law_taps,
};
use super::*;
use crate::wgpu_backend::film_chain::device_image::{BufferPayload, ImagePage};

/// Band working set the plan is allowed to hold at once.
const BAND_BUDGET_BYTES: u64 = 256 << 20;

struct LawPiece {
    /// Source window, in film-lattice pixels.
    window: [u32; 4],
    /// Grid rows this piece publishes.
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
    taps: LawTaps,
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
            backend.encode_law_band(
                &mut encoder,
                &piece.band,
                &piece.geometry,
                &self.pipes,
                &self.taps,
            );
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
        .checked_add(window * 12)
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
        if input.width != width || input.height != height {
            return Err(
                "Dye grain law: the source raster does not match the declared geometry".into(),
            );
        }
        let source_pitch = params.source_pitch_um;
        if !(source_pitch.is_finite() && source_pitch > 0.0) {
            return Err("Dye grain law: the source pitch must be positive".into());
        }
        let grid = &params.grid;
        let (rows_per_band, halo) = law_band_shape(self, width, source_pitch)?;
        let output_pages = self.allocate_image_pages(grid.width, grid.height, "dye_law_output")?;
        let pipes = self.dye_law_pipelines();
        let taps = law_taps(source_pitch);
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
            // Source rows this grid band needs, plus the filter halo.
            let top_film = grid.origin_um[1] + y as f64 * grid.pitch_um[1];
            let bottom_film = grid.origin_um[1] + (y + grid_rows) as f64 * grid.pitch_um[1];
            let left_film = grid.origin_um[0];
            let right_film = grid.origin_um[0] + grid.width as f64 * grid.pitch_um[0];
            let to_px = |um: f64| um / source_pitch;
            let window_y = (to_px(top_film).floor().max(0.0) as u32).saturating_sub(halo);
            let window_bottom = ((to_px(bottom_film).ceil() as u32) + halo).min(height);
            let window_x = (to_px(left_film).floor().max(0.0) as u32).saturating_sub(halo);
            let window_right = ((to_px(right_film).ceil() as u32) + halo).min(width);
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
                    pitch_um: source_pitch,
                    seed: params.seed,
                    expectation: params.expectation,
                    monochrome: params.monochrome,
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
            taps,
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
