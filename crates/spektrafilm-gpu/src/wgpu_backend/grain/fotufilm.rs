//! Device-resident Fotufilm formation and readout. Population fitting supplies
//! bounded coefficients; crystal deposits, clouds, capacity, tiles, prefix sums,
//! footprint statistics and image readout stay in device buffers. A global bank
//! is shared by every output window, independent of source/output page boundaries.
//! Only the four-byte execution status and explicit final materialization cross
//! back to the host. CPU `fotufilm::tiles` remains the numerical authority.

use super::*;
use spektrafilm_backend::{FotufilmGpuParams, FotufilmRecordGpuParams, FotufilmSublayerGpuParams};
use crate::wgpu_backend::film_chain::device_image::{BufferPayload, ImagePage};

/// Sample texels per bank formation axis. Both the model's coating lattice and
/// this device's scratch are `SAMPLE × SAMPLE`, as `tiles` fixes it.
const SAMPLE: u32 = 768;
/// Canonical tile side, the model's `TILE`.
const TILE: u32 = 256;
/// Bank levels per record, the model's `LEVELS`.
const LEVELS: u32 = 17;
/// Probe lattice of the pitch statistics, the model's `PROBE`.
const PROBE: u32 = 128;
/// Prefix-sum row stride of one level, the model's `(TILE + 1)²`.
const SUMS_LEVEL: u32 = (TILE + 1) * (TILE + 1);
/// Poisson count thresholds per coating cell, the model's `N_COUNTS`.
const THRESHOLDS: usize = 48;
/// Crystals a single coating cell may draw. The count law has exactly 48
/// thresholds, so no cell can draw more; the budget is the law's own bound and
/// the status flag reports a violation rather than truncating the law.
const MAX_CRYSTALS_PER_CELL: u32 = THRESHOLDS as u32;
/// Coating rows one formation band owns.
const BAND_ROWS: u32 = 16;
/// Largest formation band scratch, so a band never exceeds a buffer bound.
const MAX_BAND_SCRATCH_BYTES: u64 = 50_331_648;
/// Grid workgroup size of the 2D passes, matching their `@workgroup_size`.
const GRID_WORKGROUP: u32 = 8;

const RESAMPLE_SHADER: &str = include_str!("fotufilm_resample.wgsl");
const FORM_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("fotufilm_form.wgsl")
);
const DOWNSAMPLE_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("fotufilm_downsample.wgsl")
);
const TILE_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("fotufilm_tile.wgsl")
);
const CLOUD_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("fotufilm_cloud.wgsl")
);
const RESIDUAL_SHADER: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("fotufilm_residual.wgsl")
);
const COMPOSE_SHADER: &str = include_str!("fotufilm_compose.wgsl");

/// The conservative resample's uniform block, mirroring `ResampleParams`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ResampleUniform {
    origin_x: f64,
    origin_y: f64,
    pitch_x: f64,
    pitch_y: f64,
    source_pitch: f64,
    /// Alignment pad: the u32 group below needs 8-byte alignment.
    pad_align: u32,
    pad_align2: u32,
    source_width: u32,
    source_height: u32,
    output_width: u32,
    output_height: u32,
    output_x: u32,
    output_y: u32,
    page_first: u32,
    page_pixels: u32,
}

/// One formation band's uniform block, mirroring `FormationParams`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FormationUniform {
    cells: u32,
    record: u32,
    sublayer: u32,
    seed: u32,
    first_row: u32,
    rows: u32,
    silver: u32,
    margin: u32,
    region_w: u32,
    max_crystals: u32,
    cells_in_band: u32,
    /// Alignment pad: the f64 group below needs 8-byte alignment.
    pad_align: u32,
    /// This bank level's gross density and the record's density bounds, as the
    /// model reports them: `forming` divides by the record's own span.
    gross: f64,
    d_min: f64,
    d_max: f64,
    cell_texels: f32,
    sigma: f32,
    resolved: f32,
    edge: f32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
    pad3: u32,
}

/// One band reduction chunk's uniform block, mirroring `ReduceParams`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ReduceUniform {
    cells: u32,
    first_row: u32,
    rows: u32,
    margin: u32,
    region_w: u32,
    /// The first band row of this chunk, and how many rows it owns. A chunk
    /// never spans more than the periodic lattice, so each output row is
    /// written by exactly one work item of the chunk.
    chunk_first: i32,
    chunk_rows: u32,
    cell_texels: f32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

/// One cloud blur pass's uniform block, mirroring `BlurParams`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurUniform {
    n: u32,
    radius: i32,
    vertical: u32,
    scale: f32,
    sigma: f32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

/// The capacity law's uniform block, mirroring `CapacityParams`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CapacityUniform {
    capacity: f32,
    silver: u32,
    demand_offset: u32,
    uncovered_offset: u32,
    log_offset: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

/// The running sums' uniform block, mirroring `SumsParams`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SumsUniform {
    tile_offset: u32,
    sums_offset: u32,
    mean_offset: u32,
    pad0: u32,
}

/// The pitch statistics' uniform block, mirroring `PitchParams`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PitchUniform {
    footprint_x: f64,
    footprint_y: f64,
    record: u32,
    pad0: u32,
}

/// The residual readout's uniform block, mirroring `ResidualParams`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ResidualUniform {
    origin_x: f64,
    origin_y: f64,
    pitch_x: f64,
    pitch_y: f64,
    footprint_x: f64,
    footprint_y: f64,
    width: u32,
    height: u32,
    seed: u32,
    record: u32,
    /// Alignment pad: the f64 group below needs 8-byte alignment.
    pad_align: u32,
    pad_align2: u32,
    d_min: f64,
    d_max: f64,
    monochrome: u32,
    residual_offset: u32,
    pad0: u32,
    pad1: u32,
}

/// The colour mix's uniform block, mirroring `ComposeParams`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ComposeUniform {
    amount: f64,
    size: f64,
    own: f64,
    pad_align: f64,
    /// `layers` is a `vec4<f64>` in WGSL rather than an `array<f64, 3>`: a
    /// uniform array's element stride must be a multiple of 16, and spelling
    /// the padded lane explicitly keeps this `repr(C)` layout unambiguous.
    layers: [f64; 4],
    width: u32,
    height: u32,
    records: u32,
    monochrome: u32,
    pad_tail: [u32; 4],
}

const FORM_BINDINGS: [wgpu::BufferBindingType; 6] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
    wgpu::BufferBindingType::Storage { read_only: false },
    wgpu::BufferBindingType::Storage { read_only: false },
];
const REDUCE_BINDINGS: [wgpu::BufferBindingType; 5] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
    wgpu::BufferBindingType::Storage { read_only: false },
];
const CLOUD_BINDINGS: [wgpu::BufferBindingType; 3] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
];
const CLOUD_BLUR_BINDINGS: [wgpu::BufferBindingType; 4] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
    wgpu::BufferBindingType::Storage { read_only: false },
];
const DOWNSAMPLE_BINDINGS: [wgpu::BufferBindingType; 3] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
];
const SUMS_BINDINGS: [wgpu::BufferBindingType; 4] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
    wgpu::BufferBindingType::Storage { read_only: false },
];
const PITCH_BINDINGS: [wgpu::BufferBindingType; 6] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
    wgpu::BufferBindingType::Storage { read_only: false },
    wgpu::BufferBindingType::Storage { read_only: false },
];
const RESIDUAL_BINDINGS: [wgpu::BufferBindingType; 7] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
    wgpu::BufferBindingType::Storage { read_only: true },
];
const RESAMPLE_BINDINGS: [wgpu::BufferBindingType; 4] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
    wgpu::BufferBindingType::Storage { read_only: false },
];
const COMPOSE_BINDINGS: [wgpu::BufferBindingType; 4] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
];

/// The four dye cloud terms, `(sigma scale, weight)`, the source's own table.
const CLOUD_TERMS: [(f32, f32); 4] =
    [(0.1162, 0.1096), (0.3089, 0.2167), (0.7469, 0.4030), (1.732, 0.2510)];

struct ResidentRecord {
    sums: wgpu::Buffer,
    tile_means: wgpu::Buffer,
    probe_means: wgpu::Buffer,
    correlation: wgpu::Buffer,
}

struct FormationBuffers {
    deposits: wgpu::Buffer,
    uncovered: wgpu::Buffer,
    thresholds: wgpu::Buffer,
    forming: wgpu::Buffer,
    rows: u32,
}

/// One global bank and source carrier, independently partitioned output storage.
pub(in crate::wgpu_backend) struct FotufilmPagedState {
    source: BufferPayload,
    params: FotufilmGpuParams,
    pub(in crate::wgpu_backend) width: u32,
    pub(in crate::wgpu_backend) height: u32,
    output_pages: Vec<ImagePage>,
    window_side: u32,
}

impl FotufilmPagedState {
    pub(in crate::wgpu_backend) fn output_pages(&self) -> Vec<ImagePage> { self.output_pages.clone() }

    pub(in crate::wgpu_backend) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        backend.check_cancelled()?;
        let p = &self.params;
        let status = backend.fotufilm_scratch(4, "fotufilm_status");
        let mut bank = Vec::with_capacity(p.records.len());
        for (r, record) in p.records.iter().enumerate() {
            backend.check_cancelled()?;
            bank.push(backend.form_record(record, r as u32, p, &status)?);
        }
        // Execution status is the sole bounded orchestration readback.
        backend.check_cancelled()?;
        let flags = backend.pipe.readback_buffer_as::<u32>(&status, 4);
        match flags[0] {
            0 => {}
            1 => return Err("Fotufilm formation exceeded its crystal-count bound".into()),
            _ => return Err("Fotufilm formation deposit exceeded its reserved cell region".into()),
        }
        let capacity = u64::from(self.window_side) * u64::from(self.window_side);
        let accumulated = backend.fotufilm_scratch(capacity * 24, "fotufilm_resample_accumulator");
        let resampled = backend.fotufilm_scratch(capacity * 12, "fotufilm_resampled");
        let residual = backend.fotufilm_scratch(capacity * 8 * p.records.len() as u64, "fotufilm_residual");
        let output = backend.fotufilm_scratch(capacity * 12, "fotufilm_output_window");
        for y in (0..self.height).step_by(self.window_side as usize) {
            for x in (0..self.width).step_by(self.window_side as usize) {
                backend.check_cancelled()?;
                let width = self.window_side.min(self.width - x);
                let height = self.window_side.min(self.height - y);
                let groups = (width.div_ceil(GRID_WORKGROUP), height.div_ceil(GRID_WORKGROUP));
                let mut encoder = backend.device.create_command_encoder(&Default::default());
                encoder.clear_buffer(&accumulated, 0, None);
                encoder.clear_buffer(&residual, 0, None);
                let physical_scale = p.size.clamp(0.5, 4.0);
                let mut resample = ResampleUniform {
                    origin_x: p.origin_um[0] * physical_scale, origin_y: p.origin_um[1] * physical_scale,
                    pitch_x: p.pitch_um[0] * physical_scale, pitch_y: p.pitch_um[1] * physical_scale,
                    source_pitch: p.source_pitch_um, pad_align: 0, pad_align2: 0,
                    source_width: p.source_width, source_height: p.source_height,
                    output_width: width, output_height: height,
                    output_x: x, output_y: y, page_first: 0, page_pixels: 0,
                };
                for page in &self.source.pages {
                    backend.check_cancelled()?;
                    resample.page_first = page.first_pixel;
                    resample.page_pixels = page.n_pixels;
                    let uniform = backend.uniform_buffer(bytemuck::bytes_of(&resample));
                    backend.fotufilm_encode(&mut encoder, RESAMPLE_SHADER, &RESAMPLE_BINDINGS,
                        "main", &[&uniform, &page.buffer, &accumulated, &resampled], groups);
                }
                let uniform = backend.uniform_buffer(bytemuck::bytes_of(&resample));
                backend.fotufilm_encode(&mut encoder, RESAMPLE_SHADER, &RESAMPLE_BINDINGS,
                    "finalize", &[&uniform, &self.source.pages[0].buffer, &accumulated, &resampled], groups);
                for (r, record) in bank.iter().enumerate() {
                    backend.check_cancelled()?;
                    if p.records[r].sublayers.is_empty() || (p.monochrome && r != 1) { continue; }
                    let uniform = backend.uniform_buffer(bytemuck::bytes_of(&ResidualUniform {
                        origin_x: p.origin_um[0] + f64::from(x) * p.pitch_um[0],
                        origin_y: p.origin_um[1] + f64::from(y) * p.pitch_um[1],
                        pitch_x: p.pitch_um[0], pitch_y: p.pitch_um[1],
                        footprint_x: p.footprint_um[0], footprint_y: p.footprint_um[1],
                        width, height, seed: p.seed as u32, record: r as u32,
                        pad_align: 0, pad_align2: 0, d_min: p.records[r].d_min, d_max: p.records[r].d_max,
                        monochrome: u32::from(p.monochrome), residual_offset: r as u32 * width * height,
                        pad0: 0, pad1: 0,
                    }));
                    backend.fotufilm_encode(&mut encoder, RESIDUAL_SHADER, &RESIDUAL_BINDINGS,
                        "residual", &[&uniform, &record.sums, &record.tile_means,
                        &record.correlation, &resampled, &residual, &record.probe_means], groups);
                }
                backend.check_cancelled()?;
                let uniform = backend.uniform_buffer(bytemuck::bytes_of(&ComposeUniform {
                    amount: p.amount, size: physical_scale, own: p.color.clamp(0.0, 1.0), pad_align: 0.0,
                    layers: [p.layers[0], p.layers[1], p.layers[2], 0.0],
                    width, height, records: p.records.len() as u32, monochrome: u32::from(p.monochrome),
                    pad_tail: [0; 4],
                }));
                backend.fotufilm_encode(&mut encoder, COMPOSE_SHADER, &COMPOSE_BINDINGS,
                    "main", &[&uniform, &residual, &resampled, &output], groups);
                for row in 0..height {
                    backend.check_cancelled()?;
                    backend.copy_samples_from(&mut encoder, &output, u64::from(row * width) * 3,
                        &self.output_pages, (u64::from(y + row) * u64::from(self.width) + u64::from(x)) * 3,
                        u64::from(width) * 3)?;
                }
                backend.queue.submit(Some(encoder.finish()));
            }
        }
        backend.check_cancelled()?;
        Ok(())
    }
}

impl WgpuBackend {
    pub(in crate::wgpu_backend) fn prepare_fotufilm_paged(
        &self, width: u32, height: u32, input: &BufferPayload, params: &FotufilmGpuParams,
    ) -> Result<FotufilmPagedState, String> {
        self.check_cancelled()?;
        if width == 0 || height == 0 || input.width != width || input.height != height
            || params.source_width != width || params.source_height != height
            || params.width == 0 || params.height == 0 || input.pages.is_empty() {
            return Err("Fotufilm source/output geometry does not match its retained raster".into());
        }
        if params.records.len() != 3 {
            return Err("Fotufilm requires three indexed records (including empty records)".into());
        }
        let limit = super::super::bounded::storage_binding_limit(self)?;
        if u64::from(LEVELS * SUMS_LEVEL) * 4 > limit || u64::from(SAMPLE * SAMPLE) * 4 > limit {
            return Err("Fotufilm fixed bank lattices exceed the device storage binding limit".into());
        }
        let mut side = 256u32;
        while u64::from(side) * u64::from(side) * 24 > limit && side > 1 { side /= 2; }
        if u64::from(side) * u64::from(side) * 24 > limit {
            return Err("Fotufilm cannot bind one output sample on this device".into());
        }
        Ok(FotufilmPagedState {
            source: BufferPayload::from_pages(self.backend_id(), input.pages.clone(), width, height),
            params: params.clone(), width: params.width, height: params.height,
            output_pages: self.allocate_image_pages(params.width, params.height, "fotufilm_output")?,
            window_side: side,
        })
    }

    pub(in crate::wgpu_backend) fn fotufilm_grain_device(
        &self, img: &ImageBuf, params: &FotufilmGpuParams,
    ) -> Option<Result<ImageBuf, String>> { Some(self.fotufilm_grain_inner(img, params)) }

    fn fotufilm_grain_inner(&self, img: &ImageBuf, params: &FotufilmGpuParams) -> Result<ImageBuf, String> {
        self.check_cancelled()?;
        let pages = self.allocate_image_pages(img.width, img.height, "fotufilm_source")?;
        for page in &pages {
            self.check_cancelled()?;
            let first = page.first_pixel as usize * 3;
            let end = first + page.n_pixels as usize * 3;
            let samples = scalars_to_f32(&img.data[first..end]);
            self.queue.write_buffer(&page.buffer, 0, bytemuck::cast_slice(samples.as_ref()));
        }
        let input = BufferPayload::from_pages(self.backend_id(), pages, img.width, img.height);
        let state = self.prepare_fotufilm_paged(img.width, img.height, &input, params)?;
        state.submit(self)?;
        let mut data = Vec::with_capacity(params.width as usize * params.height as usize * 3);
        for page in &state.output_pages {
            self.check_cancelled()?;
            data.extend(f32_to_scalars(self.pipe.readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12)));
        }
        self.check_cancelled()?;
        Ok(ImageBuf::from_data(params.width, params.height, data))
    }

    fn form_record(&self, record: &FotufilmRecordGpuParams, r: u32, params: &FotufilmGpuParams,
        status: &wgpu::Buffer) -> Result<ResidentRecord, String> {
        self.check_cancelled()?;
        let bank = ResidentRecord {
            sums: self.fotufilm_scratch(u64::from(LEVELS * SUMS_LEVEL) * 4, "fotufilm_bank_sums"),
            tile_means: self.fotufilm_scratch(u64::from(LEVELS) * 8, "fotufilm_tile_means"),
            probe_means: self.fotufilm_scratch(u64::from(LEVELS) * 8, "fotufilm_probe_means"),
            correlation: self.fotufilm_scratch(u64::from(LEVELS - 1) * 8, "fotufilm_correlation"),
        };
        if record.sublayers.is_empty() || (params.monochrome && r != 1) { return Ok(bank); }
        let bytes = u64::from(SAMPLE * SAMPLE) * 4;
        let deposits = self.fotufilm_scratch(bytes, "fotufilm_deposits");
        let uncovered = self.fotufilm_scratch(bytes, "fotufilm_uncovered");
        let demand = self.fotufilm_scratch(bytes, "fotufilm_cloud_sum");
        let log_light = self.fotufilm_scratch(bytes, "fotufilm_log_light");
        let tile = self.fotufilm_scratch(u64::from(TILE * TILE) * 4, "fotufilm_tile");
        let cloud_scratch = [
            self.fotufilm_scratch(bytes, "fotufilm_cloud_horizontal"),
            self.fotufilm_scratch(bytes, "fotufilm_cloud_blurred"),
            self.fotufilm_scratch(bytes, "fotufilm_cloud_coarse"),
        ];
        // Reserve the largest actual, untruncated Gaussian support of this
        // record. Its tap buffer is reused across sublayers and bank levels.
        let mut max_radius = 1u64;
        for layer in &record.sublayers {
            let sigma = layer.sigma_mm as f32 / (0.001 / 3.0);
            for (s, _) in CLOUD_TERMS {
                let spread = s * sigma;
                let mut factor = (spread / 1.6).floor().max(1.0) as u32;
                while factor > 1 && !SAMPLE.is_multiple_of(factor) { factor -= 1; }
                let f = factor as f32;
                let blur_sigma = if factor == 1 { spread } else { (spread * spread - f * f / 3.0).sqrt() / f };
                max_radius = max_radius.max((3.5 * blur_sigma).ceil().max(1.0) as u64);
            }
        }
        let cloud_kernel = self.fotufilm_scratch((2 * max_radius + 1) * 4, "fotufilm_cloud_kernel");
        let mut formation = Vec::with_capacity(record.sublayers.len());
        for (b, layer) in record.sublayers.iter().enumerate() {
            let cell_texels = SAMPLE as f32 / layer.cells as f32;
            let margin = layer.reach_texels.ceil() as u32;
            let region = (cell_texels.ceil() as u32 + 2 * margin).max(1);
            let per_row = u64::from(layer.cells) * u64::from(region) * u64::from(region) * 4;
            let limit = super::super::bounded::storage_binding_limit(self)?.min(MAX_BAND_SCRATCH_BYTES);
            let rows = BAND_ROWS.min((limit / per_row) as u32);
            if rows == 0 { return Err(format!("Fotufilm record {r} sublayer {b} cannot bind one formation row")); }
            formation.push(FormationBuffers {
                deposits: self.fotufilm_scratch(per_row * u64::from(rows), "fotufilm_band_deposits"),
                uncovered: self.fotufilm_scratch(per_row * u64::from(rows), "fotufilm_band_uncovered"),
                thresholds: self.storage_ro_buffer(bytemuck::cast_slice(&layer.thresholds)),
                forming: self.storage_ro_buffer(bytemuck::cast_slice(&layer.forming)), rows,
            });
        }
        let span = (record.d_max - record.d_min).max(1e-6);
        for level in 0..LEVELS {
            self.check_cancelled()?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            encoder.clear_buffer(&log_light, 0, None);
            self.queue.submit(Some(encoder.finish()));
            let gross = record.d_min + span * f64::from(level) / f64::from(LEVELS - 1);
            for (b, layer) in record.sublayers.iter().enumerate() {
                self.check_cancelled()?;
                self.form_sublayer(layer, r, b as u32, gross, record, params.formation_seed,
                    &deposits, &uncovered, status, &formation[b])?;
                if !layer.silver { self.cloud_sublayer(layer, &deposits, &demand, &cloud_scratch, &cloud_kernel)?; }
                let uniform = self.uniform_buffer(bytemuck::bytes_of(&CapacityUniform {
                    capacity: layer.capacity as f32, silver: u32::from(layer.silver),
                    demand_offset: 0, uncovered_offset: 0, log_offset: 0, pad0: 0, pad1: 0, pad2: 0,
                }));
                let mut encoder = self.device.create_command_encoder(&Default::default());
                self.fotufilm_encode(&mut encoder, TILE_SHADER, &COMPOSE_BINDINGS, "transmittance",
                    &[&uniform, if layer.silver { &deposits } else { &demand }, &uncovered, &log_light],
                    (SAMPLE.div_ceil(GRID_WORKGROUP), SAMPLE.div_ceil(GRID_WORKGROUP)));
                self.queue.submit(Some(encoder.finish()));
            }
            self.check_cancelled()?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            let uniform = self.uniform_buffer(bytemuck::cast_slice(&[0u32; 4]));
            self.fotufilm_encode(&mut encoder, DOWNSAMPLE_SHADER, &DOWNSAMPLE_BINDINGS, "downsample",
                &[&uniform, &log_light, &tile], (TILE.div_ceil(GRID_WORKGROUP), TILE.div_ceil(GRID_WORKGROUP)));
            let uniform = self.uniform_buffer(bytemuck::bytes_of(&SumsUniform {
                tile_offset: 0, sums_offset: level * SUMS_LEVEL, mean_offset: level, pad0: 0,
            }));
            self.fotufilm_encode(&mut encoder, TILE_SHADER, &SUMS_BINDINGS, "running_sums",
                &[&uniform, &tile, &bank.sums, &bank.tile_means], (1, 1));
            self.queue.submit(Some(encoder.finish()));
        }
        self.check_cancelled()?;
        let density = self.fotufilm_scratch(u64::from(PROBE * PROBE * LEVELS) * 8, "fotufilm_probe_density");
        let uniform = self.uniform_buffer(bytemuck::bytes_of(&PitchUniform {
            footprint_x: params.footprint_um[0], footprint_y: params.footprint_um[1], record: r, pad0: 0,
        }));
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let buffers = [&uniform, &bank.sums, &bank.tile_means, &density, &bank.correlation, &bank.probe_means];
        self.fotufilm_encode(&mut encoder, RESIDUAL_SHADER, &PITCH_BINDINGS, "pitch", &buffers,
            (PROBE.div_ceil(GRID_WORKGROUP), PROBE.div_ceil(GRID_WORKGROUP)));
        self.check_cancelled()?;
        self.fotufilm_encode(&mut encoder, RESIDUAL_SHADER, &PITCH_BINDINGS, "correlate", &buffers, (LEVELS, 1));
        self.queue.submit(Some(encoder.finish()));
        Ok(bank)
    }

    #[allow(clippy::too_many_arguments)]
    fn form_sublayer(&self, layer: &FotufilmSublayerGpuParams, r: u32, b: u32, gross: f64,
        record: &FotufilmRecordGpuParams, seed: u32, deposits: &wgpu::Buffer, uncovered: &wgpu::Buffer,
        status: &wgpu::Buffer, buffers: &FormationBuffers) -> Result<(), String> {
        let cells = layer.cells;
        let cell_texels = SAMPLE as f32 / cells as f32;
        let sigma = layer.sigma_mm as f32 / (0.001 / 3.0);
        let margin = layer.reach_texels.ceil() as u32;
        let region_w = (cell_texels.ceil() as u32 + 2 * margin).max(1);
        let band_rows = buffers.rows;
        let band_deposits = &buffers.deposits;
        let band_uncovered = &buffers.uncovered;
        let thresholds = &buffers.thresholds;
        let forming = &buffers.forming;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.clear_buffer(deposits, 0, None);
        encoder.clear_buffer(uncovered, 0, None);
        self.queue.submit(Some(encoder.finish()));
        for first_row in (0..cells).step_by(band_rows as usize) {
            self.check_cancelled()?;
            let rows = (cells - first_row).min(band_rows);
            let uniform = self.uniform_buffer(bytemuck::bytes_of(&FormationUniform {
                cells, record: r, sublayer: b, seed, first_row, rows, silver: u32::from(layer.silver),
                margin, region_w, max_crystals: MAX_CRYSTALS_PER_CELL, cells_in_band: rows * cells,
                pad_align: 0, gross, d_min: record.d_min, d_max: record.d_max, cell_texels, sigma,
                resolved: ((sigma - 0.5) / 1.5).clamp(0.0, 1.0), edge: layer.edge as f32,
                pad0: 0, pad1: 0, pad2: 0, pad3: 0,
            }));
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.fotufilm_encode(&mut encoder, FORM_SHADER, &FORM_BINDINGS, "main",
                &[&uniform, thresholds, forming, band_deposits, band_uncovered, status],
                ((rows * cells).div_ceil(64), 1));
            let first = (first_row as f32 * cell_texels).floor() as i32 - margin as i32;
            let end = ((first_row + rows - 1) as f32 * cell_texels).floor() as i32
                - margin as i32 + region_w as i32;
            let mut chunk_first = first;
            while chunk_first < end {
                self.check_cancelled()?;
                let chunk_rows = (end - chunk_first).min(SAMPLE as i32) as u32;
                let uniform = self.uniform_buffer(bytemuck::bytes_of(&ReduceUniform {
                    cells, first_row, rows, margin, region_w, chunk_first, chunk_rows,
                    cell_texels, pad0: 0, pad1: 0, pad2: 0,
                }));
                self.fotufilm_encode(&mut encoder, FORM_SHADER, &REDUCE_BINDINGS, "reduce",
                    &[&uniform, band_deposits, band_uncovered, deposits, uncovered],
                    (chunk_rows.div_ceil(64), 1));
                chunk_first += chunk_rows as i32;
            }
            self.queue.submit(Some(encoder.finish()));
        }
        Ok(())
    }

    fn cloud_sublayer(&self, layer: &FotufilmSublayerGpuParams,
        deposits: &wgpu::Buffer, demand: &wgpu::Buffer, scratch: &[wgpu::Buffer; 3], kernel: &wgpu::Buffer) -> Result<(), String> {
        let [horizontal, blurred, coarse] = scratch;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.clear_buffer(demand, 0, None);
        let sigma = layer.sigma_mm as f32 / (0.001 / 3.0);
        for (s, weight) in CLOUD_TERMS {
            self.check_cancelled()?;
            let spread = s * sigma;
            let mut factor = (spread / 1.6).floor().max(1.0) as u32;
            while factor > 1 && !SAMPLE.is_multiple_of(factor) { factor -= 1; }
            let scale = weight * 2.0 * std::f32::consts::PI * spread * spread;
            let n = SAMPLE / factor;
            let mut p = BlurUniform { n, radius: 0, vertical: 0, scale: 1.0,
                sigma: spread, pad0: factor, pad1: 0, pad2: 0 };
            if factor > 1 {
                let uniform = self.uniform_buffer(bytemuck::bytes_of(&p));
                self.fotufilm_encode(&mut encoder, CLOUD_SHADER, &CLOUD_BINDINGS, "tent",
                    &[&uniform, deposits, horizontal], (n.div_ceil(8), SAMPLE.div_ceil(8)));
                p.vertical = 1;
                let uniform = self.uniform_buffer(bytemuck::bytes_of(&p));
                self.fotufilm_encode(&mut encoder, CLOUD_SHADER, &CLOUD_BINDINGS, "tent",
                    &[&uniform, horizontal, coarse], (n.div_ceil(8), n.div_ceil(8)));
                let f = factor as f32;
                p.sigma = (spread * spread - f * f / 3.0).sqrt() / f;
            }
            p.radius = (3.5 * p.sigma).ceil().max(1.0) as i32;
            p.vertical = 0;
            let uniform = self.uniform_buffer(bytemuck::bytes_of(&p));
            self.fotufilm_encode(&mut encoder, CLOUD_SHADER, &CLOUD_BLUR_BINDINGS, "prepare_kernel",
                &[&uniform, deposits, horizontal, kernel], (1, 1));
            self.fotufilm_encode(&mut encoder, CLOUD_SHADER, &CLOUD_BLUR_BINDINGS, "main",
                &[&uniform, if factor == 1 { deposits } else { coarse }, horizontal, kernel],
                (n.div_ceil(8), n.div_ceil(8)));
            p.vertical = 1;
            p.scale = if factor == 1 { scale } else { 1.0 };
            let uniform = self.uniform_buffer(bytemuck::bytes_of(&p));
            self.fotufilm_encode(&mut encoder, CLOUD_SHADER, &CLOUD_BLUR_BINDINGS, "main",
                &[&uniform, horizontal, blurred, kernel], (n.div_ceil(8), n.div_ceil(8)));
            p.scale = scale;
            let uniform = self.uniform_buffer(bytemuck::bytes_of(&p));
            self.fotufilm_encode(&mut encoder, CLOUD_SHADER, &CLOUD_BINDINGS,
                if factor == 1 { "accumulate" } else { "upsample_add" },
                &[&uniform, blurred, demand], (SAMPLE.div_ceil(8), SAMPLE.div_ceil(8)));
        }
        self.queue.submit(Some(encoder.finish()));
        self.check_cancelled()?;
        Ok(())
    }

    fn fotufilm_scratch(&self, bytes: u64, label: &str) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label), size: bytes, mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn uniform_buffer(&self, contents: &[u8]) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fotufilm_uniform"), contents, usage: wgpu::BufferUsages::UNIFORM,
        })
    }

    fn storage_ro_buffer(&self, contents: &[u8]) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fotufilm_coefficients"), contents, usage: wgpu::BufferUsages::STORAGE,
        })
    }

    fn fotufilm_encode(&self, encoder: &mut ObservedEncoder, shader: &'static str,
        bindings: &[wgpu::BufferBindingType], entry_point: &'static str,
        buffers: &[&wgpu::Buffer], groups: (u32, u32)) {
        let (cached, _) = self.pipeline_cache.get_or_compile_at(&self.device.raw, shader, bindings, entry_point);
        let entries: Vec<_> = buffers.iter().enumerate().map(|(i, buffer)| wgpu::BindGroupEntry {
            binding: i as u32, resource: buffer.as_entire_binding(),
        }).collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fotufilm_bind_group"), layout: &cached.layout, entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(entry_point), timestamp_writes: None,
        });
        pass.set_pipeline(&cached.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(groups.0, groups.1, 1);
    }
}

/// A negative colour stock's fitted population, formed from the same synthetic
/// curves the stage tests use.
///
/// The population solver is the model's own (`Population::prepare`), so the
/// reference side of the comparison is the fitted population the stage submits
/// to the device — never a second derivation of the fitting law. The model is
/// a dev-dependency, so this helper is test-only.
#[cfg(test)]
fn population() -> spektrafilm_model::grain::fotufilm::Population {
    use spektrafilm_model::grain::fotufilm::{Curve, Metadata, Population, PopulationInput};
    let log_exposure: Vec<f64> = vec![-2.0, -0.5, 1.0, 2.5];
    let density: Vec<f64> = vec![0.0, 0.7, 1.9, 2.2];
    let curves: [Curve; 3] = std::array::from_fn(|record| Curve {
        log_exposure: log_exposure.clone(),
        density: density.iter().map(|d| d + record as f64 * 0.05).collect(),
        d_min: 0.0,
        d_max: 2.2 + 2.0 * 0.05,
    });
    let metadata = Metadata {
        sigma48: [0.009 * 0.7, 0.009, 0.009 * 1.35],
        anchor_net_density: [1.0; 3],
        cloud_radius_mm: 0.005,
        layer_size_ratio: [1.0; 3],
        radius_span: 6.0,
        sublayer_shares: [0.25; 4],
        coating_density_scale: 1.0,
        fog_density: 0.03,
        reversal: false,
        silver: false,
        monochrome: false,
    };
    Population::prepare(
        PopulationInput {
            curves,
            reference_curves: None,
            metadata: metadata.clone(),
            reference_metadata: None,
        },
        &|| Ok(()),
    )
    .expect("population")
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::precision::{from_f64, to_f64};
    use spektrafilm_model::grain::fotufilm::DeviceRecord;

    /// The stage's own `FotufilmGpuParams` for a source of `source`'s extent
    /// read onto a `readout`-sized lattice, which differs from the source.
    fn device_params(
        records: &[DeviceRecord],
        source: (u32, u32),
        readout: (u32, u32),
    ) -> FotufilmGpuParams {
        FotufilmGpuParams {
            formation_seed: spektrafilm_model::grain::fotufilm::formation_seed(),
            records: records
                .iter()
                .map(|record| FotufilmRecordGpuParams {
                    sublayers: record
                        .sublayers
                        .iter()
                        .map(|sublayer| FotufilmSublayerGpuParams {
                            cells: sublayer.cells,
                            silver: sublayer.silver,
                            sigma_mm: sublayer.sigma_mm,
                            capacity: sublayer.capacity,
                            edge: sublayer.edge,
                            forming: sublayer.forming.clone(),
                            thresholds: sublayer.thresholds.clone(),
                            reach_texels: sublayer.reach_texels,
                            occupied_area: sublayer.occupied_area,
                        })
                        .collect(),
                    d_min: record.d_min,
                    d_max: record.d_max,
                })
                .collect(),
            monochrome: false,
            source_pitch_um: 10.0,
            origin_um: [0.0; 2],
            pitch_um: [10.0, 10.0],
            width: readout.0,
            height: readout.1,
            source_width: source.0,
            source_height: source.1,
            footprint_um: [10.0, 10.0],
            seed: 12345,
            amount: 1.0,
            size: 1.0,
            softness: 1.0,
            color: 1.0,
            layers: [1.0, 1.0, 1.0],
        }
    }

    /// The developed source the stage hands the device: a mid-density ramp
    /// with per-channel variation, at `size`.
    fn source_image(size: (u32, u32)) -> ImageBuf {
        let (width, height) = size;
        let mut data = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                let u = x as f64 / (width - 1).max(1) as f64;
                let v = y as f64 / (height - 1).max(1) as f64;
                data.push(from_f64(0.2 + 0.6 * u));
                data.push(from_f64(0.15 + 0.7 * v));
                data.push(from_f64(0.3 + 0.4 * u * v));
            }
        }
        ImageBuf::from_data(width, height, data)
    }

    /// The host readout the stage would compute for the same request.
    ///
    /// The reference is the model's own bank driven exactly as the stage
    /// drives it: the fitted population prepares a bank, the developed source
    /// is conservatively resampled onto the readout lattice with the same
    /// geometry, and `Bank::apply` reads the residual out. Nothing here
    /// re-derives the physics — the model evaluates every rule.
    fn host_reference(
        bank: &spektrafilm_model::grain::fotufilm::Bank,
        size: (u32, u32),
    ) -> ImageBuf {
        use spektrafilm_model::grain::fotufilm::{Geometry, Look};
        let input = source_image(size);
        let geometry = Geometry {
            source_pitch_mm: [0.01; 2],
            origin_mm: [0.0; 2],
            output_pitch_mm: [0.01; 2],
            footprint_mm: [0.01; 2],
        };
        bank.apply(&input, geometry, Look::default(), 12345, &|| Ok(()))
            .expect("host readout")
    }

    /// Run one device-vs-host comparison at `size`, reporting the mean and
    /// maximum absolute deviation over every sample of the readout.
    ///
    /// The readout lattice matches the source's extent, so the conservative
    /// resample is the identity on both sides and the comparison isolates the
    /// grain readout itself.
    ///
    /// `test_support::adapter` acquires [`DEVICE_LOCK`](crate::test_support::DEVICE_LOCK)
    /// before creating the device and holds it for the returned backend's whole
    /// lifetime, so the device never outlives the guard that makes using it
    /// safe; this helper takes no lock of its own.
    fn compare(
        bank: &spektrafilm_model::grain::fotufilm::Bank,
        records: &[DeviceRecord],
        size: (u32, u32),
    ) -> (f64, f64, usize) {
        let backend = crate::test_support::adapter();
        let image = source_image(size);
        let params = device_params(records, size, size);
        let device = backend
            .fotufilm_grain_inner(&image, &params)
            .expect("the device readout completes");
        let expected = host_reference(bank, size);
        assert_eq!(device.width, expected.width);
        assert_eq!(device.height, expected.height);
        let mut mean = 0.0f64;
        let mut max = 0.0f64;
        for (index, want) in expected.data.iter().enumerate() {
            let got = to_f64(device.data[index]);
            let delta = (got - to_f64(*want)).abs();
            assert!(delta.is_finite(), "nonfinite device readout at sample {index}");
            mean += delta;
            max = max.max(delta);
        }
        mean /= expected.data.len() as f64;
        (mean, max, expected.data.len())
    }

    #[test]
    fn device_multi_sublayer_readout_matches_the_host_reference() {
        let population = population();
        let records = population.device_records();
        let bank = population.build_bank(&|| Ok(())).expect("bank");
        for (width, height) in [(32, 24), (400, 300), (1600, 1200)] {
            let (mean, max, samples) = compare(&bank, &records, (width, height));
            println!(
                "fotufilm device vs host at {width}x{height}: \
                 mean {mean:.6e}, max {max:.6e} over {samples} samples"
            );
            assert!(
                max < 5e-2,
                "device readout at {width}x{height} deviates from the host by \
                 {max:.6e} (mean {mean:.6e})"
            );
        }
    }

    #[test]
    fn prepared_cloud_kernel_matches_reference_blur() {
        let backend = crate::test_support::adapter();
        let n = 16u32;
        let source: Vec<f32> = (0..n * n).map(|i| ((i * 37 + i / n * 11) % 101) as f32 / 101.0).collect();
        let input = backend.storage_ro_buffer(bytemuck::cast_slice(&source));
        let horizontal = backend.fotufilm_scratch(u64::from(n * n) * 4, "cloud_test_horizontal");
        let output = backend.fotufilm_scratch(u64::from(n * n) * 4, "cloud_test_output");
        for sigma in [0.35f32, 1.6, 3.19, 7.75] {
            // Literal CPU tiles::kernel/blur reference loops: f32 exp, ascending
            // normalization sum, ascending x taps, then ascending y taps.
            let radius = (3.5 * sigma).ceil().max(1.0) as i32;
            let mut taps: Vec<f32> = (-radius..=radius)
                .map(|t| (-(t * t) as f32 / (2.0 * sigma * sigma)).exp()).collect();
            let sum = taps.iter().sum::<f32>();
            for tap in &mut taps { *tap /= sum; }
            let mut across = vec![0.0f32; source.len()];
            let mut expected = vec![0.0f32; source.len()];
            for y in 0..n as i32 {
                for x in 0..n as i32 {
                    for (t, weight) in taps.iter().enumerate() {
                        across[(y * n as i32 + x) as usize] += weight
                            * source[(y * n as i32 + (x + t as i32 - radius).rem_euclid(n as i32)) as usize];
                    }
                }
            }
            for y in 0..n as i32 {
                for x in 0..n as i32 {
                    for (t, weight) in taps.iter().enumerate() {
                        expected[(y * n as i32 + x) as usize] += weight
                            * across[((y + t as i32 - radius).rem_euclid(n as i32) * n as i32 + x) as usize];
                    }
                }
            }
            let kernel = backend.fotufilm_scratch(taps.len() as u64 * 4, "cloud_test_kernel");
            let mut p = BlurUniform { n, radius, vertical: 0, scale: 1.0, sigma, pad0: 0, pad1: 0, pad2: 0 };
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            let uniform = backend.uniform_buffer(bytemuck::bytes_of(&p));
            backend.fotufilm_encode(&mut encoder, CLOUD_SHADER, &CLOUD_BLUR_BINDINGS, "prepare_kernel",
                &[&uniform, &input, &horizontal, &kernel], (1, 1));
            backend.fotufilm_encode(&mut encoder, CLOUD_SHADER, &CLOUD_BLUR_BINDINGS, "main",
                &[&uniform, &input, &horizontal, &kernel], (n.div_ceil(8), n.div_ceil(8)));
            p.vertical = 1;
            let uniform = backend.uniform_buffer(bytemuck::bytes_of(&p));
            backend.fotufilm_encode(&mut encoder, CLOUD_SHADER, &CLOUD_BLUR_BINDINGS, "main",
                &[&uniform, &horizontal, &output, &kernel], (n.div_ceil(8), n.div_ceil(8)));
            backend.queue.submit(Some(encoder.finish()));
            let actual_taps = backend.pipe.readback_buffer(&kernel, taps.len() as u64 * 4);
            let actual = backend.pipe.readback_buffer(&output, u64::from(n * n) * 4);
            let tap_max = actual_taps.iter().zip(&taps).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            let max = actual.iter().zip(&expected).map(|(a, b)| {
                assert!(a.is_finite());
                (a - b).abs()
            }).fold(0.0f32, f32::max);
            println!("Fotufilm prepared cloud kernel sigma {sigma}: tap max {tap_max:.6e}, blur max {max:.6e}");
            assert!(tap_max < 1e-7, "kernel sigma {sigma}: {tap_max}");
            assert!(max < 1e-6, "blur sigma {sigma}: {max}");
        }
    }

    #[test]
    fn conservative_resample_preserves_constant_density_far_outside_source() {
        let backend = crate::test_support::adapter();
        for source_side in [1, 2] {
            let input = ImageBuf::from_data(source_side, source_side,
                (0..source_side * source_side).flat_map(|_| [0.2, 0.7, 1.3].map(from_f64)).collect());
            let mut params = device_params(&[], (source_side, source_side), (37, 29));
            // Empty records disable grain without changing resample execution.
            params.records = vec![FotufilmRecordGpuParams { sublayers: Vec::new(), d_min: 0.0, d_max: 2.0 }; 3];
            params.size = 1.7;
            params.origin_um = [-103.25 / params.size, 506.75 / params.size];
            params.pitch_um = [7.5 / params.size, 12.5 / params.size];
            let output = backend.fotufilm_grain_inner(&input, &params).expect("constant edge resample");
            for (index, sample) in output.data.iter().enumerate() {
                let expected = [0.2, 0.7, 1.3][index % 3];
                assert!((to_f64(*sample) - expected).abs() < 1e-6,
                    "source side {source_side}, sample {index}: {} instead of {expected}", to_f64(*sample));
            }
        }
    }

    #[test]
    fn fractional_geometry_and_paged_readout_match_the_cpu_bank() {
        use spektrafilm_model::grain::fotufilm::{Geometry, Look};
        let population = population();
        let bank = population.build_bank(&|| Ok(())).expect("CPU bank");
        let records = population.device_records();
        let backend = crate::test_support::adapter();
        // A 4x4 source contains four constant quadrants separated at 40um.
        // Their transition is interior, so its exact area average is analytical
        // even with the reference's virtual edge-cell merging convention.
        let quadrants: [[f64; 3]; 4] = [[0.2, 0.4, 0.7], [1.1, 0.8, 0.3], [0.6, 1.3, 0.9], [1.4, 0.2, 1.0]];
        let input = ImageBuf::from_data(4, 4, (0..4).flat_map(|y| (0..4).flat_map(move |x| {
            quadrants[usize::from(y >= 2) * 2 + usize::from(x >= 2)].map(from_f64)
        })).collect());
        let origin: [f64; 2] = [-3.25, 6.75];
        let pitch: [f64; 2] = [7.5, 12.5];
        let look = Look { amount: 0.8, size: 1.7, softness: 1.3, color: 0.65, layers: [0.9, 1.1, 0.7] };
        let (width, height) = (37, 29);
        let mut averaged = ImageBuf::new(width, height);
        for y in 0..height {
            let top = origin[1] + f64::from(y) * pitch[1];
            let wy = ((40.0 - top) / pitch[1]).clamp(0.0, 1.0);
            for x in 0..width {
                let left = origin[0] + f64::from(x) * pitch[0];
                let wx = ((40.0 - left) / pitch[0]).clamp(0.0, 1.0);
                for (c, &upper_left) in quadrants[0].iter().enumerate() {
                    averaged.data[((y * width + x) * 3) as usize + c] = from_f64(
                        wx * wy * upper_left + (1.0 - wx) * wy * quadrants[1][c]
                        + wx * (1.0 - wy) * quadrants[2][c] + (1.0 - wx) * (1.0 - wy) * quadrants[3][c]);
                }
            }
        }
        let expected = bank.apply(&averaged, Geometry {
            source_pitch_mm: [0.02; 2], origin_mm: origin.map(|v| v / 1000.0),
            output_pitch_mm: pitch.map(|v| v / 1000.0), footprint_mm: pitch.map(|v| v / 1000.0),
        }, look, 12345, &|| Ok(())).expect("CPU physical readout");
        let mut params = device_params(&records, (4, 4), (width, height));
        params.source_pitch_um = 20.0;
        params.origin_um = origin.map(|v| v / look.size);
        params.pitch_um = pitch.map(|v| v / look.size);
        params.footprint_um = params.pitch_um.map(|v| v * look.softness);
        params.amount = look.amount;
        params.size = look.size;
        params.softness = look.softness;
        params.color = look.color;
        params.layers = look.layers;
        let contiguous = backend.fotufilm_grain_inner(&input, &params).expect("contiguous readout");
        let mut pages = Vec::new();
        for pixel in 0..16 {
            let buffer = backend.fotufilm_scratch(12, "fotufilm_test_source_page");
            let samples = scalars_to_f32(&input.data[pixel * 3..pixel * 3 + 3]);
            backend.queue.write_buffer(&buffer, 0, bytemuck::cast_slice(samples.as_ref()));
            pages.push(ImagePage { first_pixel: pixel as u32, n_pixels: 1, buffer });
        }
        let payload = BufferPayload::from_pages(backend.backend_id(), pages, 4, 4);
        let mut split = backend.prepare_fotufilm_paged(4, 4, &payload, &params).expect("paged preparation");
        split.window_side = 17;
        split.output_pages = (0..width * height).step_by(19).map(|first| {
            let count = (width * height - first).min(19);
            ImagePage { first_pixel: first, n_pixels: count,
                buffer: backend.fotufilm_scratch(u64::from(count) * 12, "fotufilm_test_output_page") }
        }).collect();
        split.submit(&backend).expect("split readout");
        let mut paged = Vec::new();
        for page in &split.output_pages {
            paged.extend(backend.pipe.readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12));
        }
        let mut max = 0.0f64;
        let mut mean = 0.0f64;
        let mut page_max = 0.0f64;
        for (i, want) in expected.data.iter().enumerate() {
            let delta = (to_f64(contiguous.data[i]) - to_f64(*want)).abs();
            assert!(delta.is_finite(), "nonfinite device readout at sample {i}");
            max = max.max(delta);
            mean += delta;
            page_max = page_max.max((f64::from(paged[i]) - to_f64(contiguous.data[i])).abs());
        }
        mean /= expected.data.len() as f64;
        println!("fotufilm fractional geometry: mean {mean:.6e}, max {max:.6e}; page/window split max {page_max:.6e}");
        assert!(max < 5e-2, "fractional geometry max {max:.6e}, mean {mean:.6e}");
        assert!(page_max < 1e-6, "page/window partition changed the physical field by {page_max:.6e}");
    }

}
