//! G3 and G5 of the Dye device path: exact-overlap remap of the source
//! sublayer targets onto the field cells, cell-integrated formation, and
//! fractional readout into the output grid.
//!
//! This module owns the spatial halves of
//! `specs/film-grain/v3/design.md`: `dye_layers` allocates sublayer targets
//! per source pixel (G2), the count law realizes cell masses (G4), and the
//! passes here transfer, form, and read that field (G3, G5). The host prepares
//! only bounded scalar geometry and resource bounds; every overlap, tap sum,
//! and readout weight is evaluated on the device.
//!
//! Coordinate evaluation is the one hard part. Overlap weights are
//! differences of film coordinates tens of thousands of micrometers apart,
//! and a plain f32 evaluation cancels to about `2e-3` um there, three orders
//! of magnitude above the weight precision the readout needs. Each origin and
//! pitch is therefore packed as a signed 64-bit fixed-point value in
//! `2^-42` um and the boundary differences are computed in exact two-word
//! integer arithmetic on the device (`dye_spatial_params.wgsl`), with one
//! final rounding of the small local difference to f32. Compensated float
//! residuals are deliberately not used: drivers may contract them away.
//! Every pass evaluates the same global coordinate expression for every band
//! and every crop, so banding is bitwise stable and cropping cannot move a
//! value beyond that final rounding.

use super::dye::{DyeGpuParams, DyeReadoutGrid};
use super::dye_layers::{DyeChannelTableGpu, DyeChannelTablesGpu};
use super::*;

/// Output rows per band before the cell budget applies.
const BAND_OUTPUT_ROWS: u32 = 128;
/// Largest cell or source-pixel count one band may hold. Bands are storage
/// partitions of the global field; a single output row above the budget
/// fails with an actionable error.
const MAX_BAND_CELLS: u64 = 1 << 24;
/// Fixed-point scale of the device coordinate lattice: `2^-42` um.
const FX_SCALE: f64 = 4398046511104.0;
/// Largest absolute film coordinate (in um) the lattice represents with all
/// products inside the two-word range. About 26 cm, far beyond any film.
const MAX_COORD_UM: f64 = 262144.0;
/// Largest absolute cell, source-pixel, or output index whose split-ratio
/// estimates stay below one unit of error. The plan rejects beyond it.
const MAX_INDEX: f64 = 1048576.0;
/// Window margins in cells/pixels around every exact touch range. They cover
/// the device's split-ratio estimate error (bounded far below one unit) and
/// the host/device rounding difference, so every rectangle a kernel can
/// touch stays inside its band buffers.
const WINDOW_MARGIN: i64 = 3;

/// Storage partition of the global field for one run of the passes.
#[derive(Clone, Copy, Debug)]
pub(super) struct DyeBand {
    pub output_y: u32,
    pub output_rows: u32,
    pub source_x: u32,
    pub source_y: u32,
    pub source_cols: u32,
    pub source_rows: u32,
    /// Extended halo origin in global signed cell coordinates.
    pub cell_x: i32,
    pub cell_y: i32,
    /// Inner formed cells.
    pub cols: u32,
    pub rows: u32,
    pub ext_cols: u32,
    pub ext_rows: u32,
}

/// Bounded resource plan of one Dye run.
pub(super) struct DyePlan {
    pub bands: Vec<DyeBand>,
    pub max_source_pixels: u32,
    pub max_cells: u32,
    pub max_horizontal_cells: u32,
    pub max_formed_cells: u32,
}

/// Signed 64-bit fixed-point words `(high, low)` of one film coordinate in
/// `2^-42` um. The plan validates representability before any dispatch.
fn fx_words(value_um: f64) -> [u32; 2] {
    let scaled = (value_um * FX_SCALE).round();
    let bits = scaled as i64 as u64;
    [(bits >> 32) as u32, bits as u32]
}

/// Split `numerator / denominator` into an exact integer part `q` and a
/// fractional remainder split into a 17-bit `hi` plus a `lo` term, so
/// `index * fraction` on the device keeps its error bounded by the product's
/// own magnitude instead of the index times a representation error.
fn split_ratio(numerator: f64, denominator: f64) -> (u32, f32, f32) {
    let ratio = numerator / denominator;
    let q = ratio.floor();
    let r = ratio - q;
    let hi = (r * 131072.0).round() / 131072.0;
    (q as u32, hi as f32, (r - hi) as f32)
}

/// Source window of the cells `[cell, cell + ext)` on one axis: every source
/// pixel the remap can touch, clamped to the film for the constant edge
/// extension, with the window margins.
fn source_window(cell: i64, ext: u32, h: f64, pitch: f64, extent: u32) -> (u32, u32) {
    let lo = (cell as f64 * h / pitch).floor() - WINDOW_MARGIN as f64;
    let hi = ((cell + i64::from(ext)) as f64 * h / pitch).ceil() - 1.0 + WINDOW_MARGIN as f64;
    let clamp = |v: f64| v.max(0.0).min(extent as f64 - 1.0) as u32;
    let a = clamp(lo);
    let b = clamp(hi);
    (a, b - a + 1)
}

/// Exact touch range of output rows `[row0, row1)` in global cell indices.
fn row_cell_range(p: &DyeGpuParams, row0: u32, row1: u32) -> (i64, i64) {
    let h = p.cell_um;
    let [_, y0] = p.grid.origin_um;
    let [_, row_w] = p.grid.pitch_um;
    let lo = ((y0 + f64::from(row0) * row_w) / h).floor() as i64;
    let hi = ((y0 + f64::from(row1) * row_w) / h).ceil() as i64 - 1;
    (lo, hi)
}

/// Geometry of output rows `[row0, row1)`: the inner formed rectangle with
/// the window margins, its formation halo, and the source window the remap
/// reads. Shared by the plan loop and the band-partitioning tests.
fn band_geometry(
    width: u32,
    height: u32,
    p: &DyeGpuParams,
    row0: u32,
    row1: u32,
) -> Result<DyeBand, String> {
    let h = p.cell_um;
    let pitch = p.source_pitch_um;
    let d = (p.taps.len() / 2) as i64;
    let [x0, _] = p.grid.origin_um;
    let x1 = x0 + f64::from(p.grid.width) * p.grid.pitch_um[0];
    let inner_x_lo = (x0 / h).floor() as i64 - WINDOW_MARGIN;
    let inner_x_hi = (x1 / h).ceil() as i64 - 1 + WINDOW_MARGIN;
    let (j_lo, j_hi) = row_cell_range(p, row0, row1);
    let inner_y_lo = j_lo - WINDOW_MARGIN;
    let inner_y_hi = j_hi + WINDOW_MARGIN;
    let cols = u32::try_from(inner_x_hi - inner_x_lo + 1)
        .map_err(|_| "film grain Dye: output columns exceed the device index range".to_owned())?;
    let rows = u32::try_from(inner_y_hi - inner_y_lo + 1)
        .map_err(|_| "film grain Dye: output rows exceed the device index range".to_owned())?;
    let cell_x = inner_x_lo - d;
    let cell_y = inner_y_lo - d;
    let ext_cols = cols + 2 * d as u32;
    let ext_rows = rows + 2 * d as u32;
    let (source_x, source_cols) = source_window(cell_x, ext_cols, h, pitch, width);
    let (source_y, source_rows) = source_window(cell_y, ext_rows, h, pitch, height);
    Ok(DyeBand {
        output_y: row0,
        output_rows: row1 - row0,
        source_x,
        source_y,
        source_cols,
        source_rows,
        cell_x: i32::try_from(cell_x).map_err(|_| {
            "film grain Dye: cell coordinates exceed the device signed cell range".to_owned()
        })?,
        cell_y: i32::try_from(cell_y).map_err(|_| {
            "film grain Dye: cell coordinates exceed the device signed cell range".to_owned()
        })?,
        cols,
        rows,
        ext_cols,
        ext_rows,
    })
}

/// The named bounds one band must fit before it is committed: the cell and
/// source-pixel budget and the device storage binding size. The two
/// nine-float buffers (`source layers`, `cell layers`) are the binding-count
/// limiter; the reason is returned so both the one-row guard and the
/// decomposition loop can name it.
fn band_fit(band: &DyeBand, bound: u64) -> Result<(), String> {
    let band_cells = u64::from(band.ext_cols) * u64::from(band.ext_rows);
    let band_pixels = u64::from(band.source_cols) * u64::from(band.source_rows);
    if band_cells > MAX_BAND_CELLS || band_pixels > MAX_BAND_CELLS {
        return Err(format!(
            "needs up to {band_cells} field cells and {band_pixels} source pixels, above the {MAX_BAND_CELLS} limit"
        ));
    }
    for (name, bytes) in [
        (
            "source layers",
            u64::from(band.source_cols) * u64::from(band.source_rows) * 36,
        ),
        (
            "cell layers",
            u64::from(band.ext_cols) * u64::from(band.ext_rows) * 36,
        ),
        (
            "cell density",
            u64::from(band.ext_cols) * u64::from(band.ext_rows) * 12,
        ),
        (
            "horizontal formation",
            u64::from(band.ext_rows) * u64::from(band.cols) * 12,
        ),
        (
            "formed density",
            u64::from(band.rows) * u64::from(band.cols) * 12,
        ),
    ] {
        if bytes > bound {
            return Err(format!(
                "{name} needs {bytes} bytes, above the {bound}-byte device storage limit"
            ));
        }
    }
    Ok(())
}

impl DyePlan {
    /// Validate the resolved geometry and partition the output into bounded
    /// bands. Axes and resource bounds are decided before any band vector or
    /// per-band uniform exists; a band that would exceed the budget holds
    /// fewer output rows and only a single row above the budget fails.
    pub(super) fn new(
        width: u32,
        height: u32,
        p: &DyeGpuParams,
        limits: &wgpu::Limits,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("film grain Dye: source dimensions must be positive".to_owned());
        }
        let tap_count = p.taps.len();
        if tap_count == 0 || tap_count.is_multiple_of(2) {
            return Err("film grain Dye: formation taps must be an odd positive count".to_owned());
        }
        if p.taps.iter().any(|tap| !tap.is_finite()) {
            return Err("film grain Dye: formation taps must be finite".to_owned());
        }
        let d = (tap_count / 2) as i64;
        let h = p.cell_um;
        let pitch = p.source_pitch_um;
        if !h.is_finite() || h <= 0.0 {
            return Err(format!(
                "film grain Dye: the field pitch must be finite and > 0 (got {h})"
            ));
        }
        if !pitch.is_finite() || pitch <= 0.0 {
            return Err(format!(
                "film grain Dye: the source pitch must be finite and > 0 (got {pitch})"
            ));
        }
        let [x0, y0] = p.grid.origin_um;
        let [col_w, row_w] = p.grid.pitch_um;
        if !x0.is_finite() || !y0.is_finite() {
            return Err("film grain Dye: readout origin must be finite and readout pitch finite and positive".to_owned());
        }
        if !col_w.is_finite() || !row_w.is_finite() || col_w <= 0.0 || row_w <= 0.0 {
            return Err("film grain Dye: readout origin must be finite and readout pitch finite and positive".to_owned());
        }
        let out_width = p.grid.width;
        let out_height = p.grid.height;
        if out_width == 0 || out_height == 0 {
            return Err("film grain Dye: output dimensions must be positive".to_owned());
        }
        let x1 = x0 + f64::from(out_width) * col_w;
        let y1 = y0 + f64::from(out_height) * row_w;
        if x0 + col_w == x0 || y0 + row_w == y0 || x1 - col_w == x1 || y1 - row_w == y1 {
            return Err("film grain Dye: output footprints are below film-coordinate precision; reduce the output dimensions".to_owned());
        }
        // The count law packs signed 32-bit cell coordinates into its stream
        // address, including the formation halo.
        let address_limit = i32::MAX as f64 - d as f64 - 1.0;
        if [x0, x1, y0, y1]
            .iter()
            .any(|v| !v.is_finite() || (*v / h).abs() > address_limit)
            || !(f64::from(width) * pitch).is_finite()
            || !(f64::from(height) * pitch).is_finite()
        {
            return Err("film grain Dye: film coordinates and support halo exceed the signed 32-bit cell address range; reduce camera.film_format_mm or move the requested region closer to the film origin".to_owned());
        }
        // Device lattice and exact index arithmetic. Beyond these the fixed
        // point products or the split-ratio estimates leave the representable
        // range, and refusing is the honest answer.
        let halo = (d + WINDOW_MARGIN) as f64 * h;
        let reach = |v: f64| v.abs() + halo;
        let extent = [
            reach(x0),
            reach(x1),
            reach(y0),
            reach(y1),
            f64::from(width) * pitch + pitch,
            f64::from(height) * pitch + pitch,
        ]
        .into_iter()
        .fold(0.0_f64, f64::max);
        if extent >= MAX_COORD_UM {
            return Err("film grain Dye: film coordinates and support halo exceed the device fixed-point coordinate range (2^18 um); reduce camera.film_format_mm or move the requested region closer to the film origin".to_owned());
        }
        if extent / h >= MAX_INDEX
            || extent / pitch >= MAX_INDEX
            || f64::from(out_width) >= MAX_INDEX
            || f64::from(out_height) >= MAX_INDEX
        {
            return Err("film grain Dye: a cell, source-pixel, or output index exceeds the device's exact index range (2^20); increase the field pitch or reduce the film extent".to_owned());
        }
        // The storage binding bound applies per band, so a device limit must
        // reduce the band height instead of failing a geometry that smaller
        // bands could serve.
        let bound = limits
            .max_buffer_size
            .min(u64::from(limits.max_storage_buffer_binding_size));
        // One output row must fit before any band storage is sized.
        let one_row = band_geometry(width, height, p, 0, 1)?;
        if let Err(reason) = band_fit(&one_row, bound) {
            return Err(format!(
                "film grain Dye: one output row {reason}; reduce the requested film region or increase the output size"
            ));
        }

        let mut bands = Vec::new();
        let mut max_source_pixels = 0u32;
        let mut max_cells = 0u32;
        let mut max_horizontal_cells = 0u32;
        let mut max_formed_cells = 0u32;
        // Coarse output pitches read many cells per row; halve the band
        // until its cells, source pixels, and storage buffers fit and keep
        // that height.
        let mut band_rows = BAND_OUTPUT_ROWS;
        let mut row0 = 0u32;
        while row0 < out_height {
            let band = loop {
                let row1 = row0.saturating_add(band_rows).min(out_height);
                let band = band_geometry(width, height, p, row0, row1)?;
                match band_fit(&band, bound) {
                    Ok(()) => break band,
                    Err(reason) => {
                        if band_rows == 1 {
                            return Err(format!(
                                "film grain Dye: one output row {reason}; reduce the requested \
                                 film region or increase the output size"
                            ));
                        }
                        band_rows /= 2;
                    }
                }
            };
            max_source_pixels = max_source_pixels.max(band.source_cols * band.source_rows);
            max_cells = max_cells.max(band.ext_cols * band.ext_rows);
            max_horizontal_cells = max_horizontal_cells.max(band.ext_rows * band.cols);
            max_formed_cells = max_formed_cells.max(band.rows * band.cols);
            row0 += band.output_rows;
            bands.push(band);
        }

        // The output buffer is not bandable and stays whole.
        let output_bytes = u64::from(out_width) * u64::from(out_height) * 12;
        if output_bytes > bound {
            return Err(format!(
                "film grain Dye: output needs {output_bytes} bytes; device storage limit is {bound} bytes"
            ));
        }
        Ok(Self {
            bands,
            max_source_pixels,
            max_cells,
            max_horizontal_cells,
            max_formed_cells,
        })
    }
}

/// Device uniform of the spatial passes. Members are `vec4`-aligned in WGSL,
/// so each Rust member is one 16-byte group and the padding is structural.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct DyeSpatialUniforms {
    origin: [u32; 4],
    footprint: [u32; 4],
    pitches: [u32; 4],
    dims: [u32; 4],
    source_rect: [u32; 4],
    cell_rect: [u32; 4],
    layout: [u32; 4],
    ratios: [u32; 4],
    phase: [u32; 4],
    fracs: [f32; 4],
    fracs2: [f32; 4],
    widths: [f32; 4],
    mins: [f32; 4],
}

fn spatial_uniform(
    p: &DyeGpuParams,
    band: &DyeBand,
    width: u32,
    height: u32,
) -> DyeSpatialUniforms {
    let h = p.cell_um;
    let pitch = p.source_pitch_um;
    let [x0, y0] = p.grid.origin_um;
    let [col_w, row_w] = p.grid.pitch_um;
    let (q_rp, rp_hi, rp_lo) = split_ratio(h, pitch);
    let (q_x, rx_hi, rx_lo) = split_ratio(col_w, h);
    let (q_y, ry_hi, ry_lo) = split_ratio(row_w, h);
    let ox = x0 / h;
    let oy = y0 / h;
    let ox_int = ox.floor();
    let oy_int = oy.floor();
    let x0_words = fx_words(x0);
    let y0_words = fx_words(y0);
    let col_words = fx_words(col_w);
    let row_words = fx_words(row_w);
    let cell_words = fx_words(h);
    let source_words = fx_words(pitch);
    DyeSpatialUniforms {
        origin: [x0_words[0], x0_words[1], y0_words[0], y0_words[1]],
        footprint: [col_words[0], col_words[1], row_words[0], row_words[1]],
        pitches: [
            cell_words[0],
            cell_words[1],
            source_words[0],
            source_words[1],
        ],
        dims: [width, height, p.grid.width, band.output_rows],
        source_rect: [
            band.source_x,
            band.source_y,
            band.source_cols,
            band.source_rows,
        ],
        cell_rect: [
            band.cell_x as u32,
            band.cell_y as u32,
            band.ext_cols,
            band.ext_rows,
        ],
        layout: [
            ((p.taps.len() - 1) / 2) as u32,
            band.cols,
            band.rows,
            band.output_y,
        ],
        ratios: [q_rp, q_x, q_y, 0],
        phase: [ox_int as i64 as u32, oy_int as i64 as u32, 0, 0],
        fracs: [rp_hi, rp_lo, (ox - ox_int) as f32, (oy - oy_int) as f32],
        fracs2: [rx_hi, rx_lo, ry_hi, ry_lo],
        widths: [col_w as f32, row_w as f32, h as f32, pitch as f32],
        mins: [
            p.tables.tables[0].min,
            p.tables.tables[1].min,
            p.tables.tables[2].min,
            0.0,
        ],
    }
}

struct SpatialPass {
    pipeline: CachedPipelineRef,
    group: wgpu::BindGroup,
    items: u32,
}

/// The G3 and G5 passes of one band.
pub(super) struct DyeSpatialState {
    remap: SpatialPass,
    form_h: SpatialPass,
    form_v: SpatialPass,
    readout: SpatialPass,
}

fn spatial_group(
    backend: &WgpuBackend,
    label: &str,
    pipeline: &CachedPipelineRef,
    buffers: &[&wgpu::Buffer],
) -> wgpu::BindGroup {
    let entries: Vec<_> = buffers
        .iter()
        .enumerate()
        .map(|(i, buffer)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: buffer.as_entire_binding(),
        })
        .collect();
    backend
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &pipeline.layout,
            entries: &entries,
        })
}

/// Shader sources of the four spatial passes. Production always uses
/// `PRODUCTION_SHADERS`; the regression tests substitute known-bad controls
/// so a broken kernel law is executed on the device and rejected by the same
/// independent oracle that accepts the production passes.
struct SpatialShaders {
    remap: &'static str,
    form_h: &'static str,
    form_v: &'static str,
    readout: &'static str,
}

const PRODUCTION_SHADERS: SpatialShaders = SpatialShaders {
    remap: concat!(
        include_str!("dye_spatial_params.wgsl"),
        include_str!("dye_spatial_remap.wgsl")
    ),
    form_h: concat!(
        include_str!("dye_spatial_params.wgsl"),
        include_str!("dye_spatial_form_h.wgsl")
    ),
    form_v: concat!(
        include_str!("dye_spatial_params.wgsl"),
        include_str!("dye_spatial_form_v.wgsl")
    ),
    readout: concat!(
        include_str!("dye_spatial_params.wgsl"),
        include_str!("dye_spatial_readout.wgsl")
    ),
};

#[allow(clippy::too_many_arguments)]
fn build_spatial_with(
    backend: &WgpuBackend,
    p: &DyeGpuParams,
    band: &DyeBand,
    width: u32,
    height: u32,
    source_layers: &wgpu::Buffer,
    cell_layers: &wgpu::Buffer,
    cells: &wgpu::Buffer,
    horizontal: &wgpu::Buffer,
    formed: &wgpu::Buffer,
    output: &wgpu::Buffer,
    shaders: &SpatialShaders,
) -> DyeSpatialState {
    let uniform = spatial_uniform(p, band, width, height);
    let params = backend.pipe.static_buffer(
        "dye_spatial_params",
        bytemuck::bytes_of(&uniform),
        wgpu::BufferUsages::UNIFORM,
    );
    let taps: Vec<f32> = p.taps.iter().map(|&tap| tap as f32).collect();
    let taps = backend.pipe.static_buffer(
        "dye_spatial_taps",
        bytemuck::cast_slice(&taps),
        wgpu::BufferUsages::STORAGE,
    );
    let uniform_ty = wgpu::BufferBindingType::Uniform;
    let storage = wgpu::BufferBindingType::Storage { read_only: true };
    let writable = wgpu::BufferBindingType::Storage { read_only: false };
    let remap = backend.cached_pipeline(shaders.remap, &[uniform_ty, storage, writable]);
    let form_h = backend.cached_pipeline(shaders.form_h, &[uniform_ty, storage, writable, storage]);
    let form_v = backend.cached_pipeline(shaders.form_v, &[uniform_ty, storage, writable, storage]);
    let readout = backend.cached_pipeline(shaders.readout, &[uniform_ty, storage, writable]);
    DyeSpatialState {
        remap: SpatialPass {
            group: spatial_group(
                backend,
                "dye_spatial_remap",
                &remap,
                &[&params, source_layers, cell_layers],
            ),
            pipeline: remap,
            items: band.ext_cols * band.ext_rows,
        },
        form_h: SpatialPass {
            group: spatial_group(
                backend,
                "dye_spatial_form_h",
                &form_h,
                &[&params, cells, horizontal, &taps],
            ),
            pipeline: form_h,
            items: band.ext_rows * band.cols,
        },
        form_v: SpatialPass {
            group: spatial_group(
                backend,
                "dye_spatial_form_v",
                &form_v,
                &[&params, horizontal, formed, &taps],
            ),
            pipeline: form_v,
            items: band.rows * band.cols,
        },
        readout: SpatialPass {
            group: spatial_group(
                backend,
                "dye_spatial_readout",
                &readout,
                &[&params, formed, output],
            ),
            pipeline: readout,
            items: band.output_rows * p.grid.width,
        },
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_spatial(
    backend: &WgpuBackend,
    p: &DyeGpuParams,
    band: &DyeBand,
    width: u32,
    height: u32,
    source_layers: &wgpu::Buffer,
    cell_layers: &wgpu::Buffer,
    cells: &wgpu::Buffer,
    horizontal: &wgpu::Buffer,
    formed: &wgpu::Buffer,
    output: &wgpu::Buffer,
) -> DyeSpatialState {
    build_spatial_with(
        backend,
        p,
        band,
        width,
        height,
        source_layers,
        cell_layers,
        cells,
        horizontal,
        formed,
        output,
        &PRODUCTION_SHADERS,
    )
}

impl DyeSpatialState {
    fn encode_pass(&self, encoder: &mut ObservedEncoder, label: &'static str, pass: &SpatialPass) {
        let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        compute.set_pipeline(&pass.pipeline.pipeline);
        compute.set_bind_group(0, &pass.group, &[]);
        dispatch_linear(&mut compute, pass.items);
    }

    /// G3: source sublayer targets -> extended cell targets.
    pub(super) fn encode_remap(&self, encoder: &mut ObservedEncoder) {
        self.encode_pass(encoder, "dye_remap", &self.remap);
    }

    /// G5: horizontal and vertical formation, then exact readout.
    pub(super) fn encode_formation_readout(&self, encoder: &mut ObservedEncoder) {
        self.encode_pass(encoder, "dye_form_h", &self.form_h);
        self.encode_pass(encoder, "dye_form_v", &self.form_v);
        self.encode_pass(encoder, "dye_readout", &self.readout);
    }
}

/// One bounded synthetic case of the spatial passes. The self-check and the
/// regression tests share this runner, so a device run is exercised the same
/// way in production certification and in tests.
#[derive(Clone)]
struct SpatialCase {
    src_w: u32,
    src_h: u32,
    /// Nine values per window pixel, channel-major `[channel][layer]`.
    source: Vec<f32>,
    /// Three values per extended cell, the field the formation reads.
    cells: Vec<f32>,
    band: DyeBand,
    cell_um: f64,
    source_pitch_um: f64,
    taps: Vec<f64>,
    mins: [f64; 3],
    origin_um: [f64; 2],
    pitch_um: [f64; 2],
    out_width: u32,
    out_height: u32,
}

impl SpatialCase {
    fn params(&self) -> DyeGpuParams {
        DyeGpuParams {
            tables: DyeChannelTablesGpu {
                tables: std::array::from_fn(|c| DyeChannelTableGpu {
                    min: self.mins[c] as f32,
                    ..Default::default()
                }),
                curves: Vec::new(),
            },
            cell_um: self.cell_um,
            taps: self.taps.clone(),
            source_pitch_um: self.source_pitch_um,
            grid: DyeReadoutGrid {
                origin_um: self.origin_um,
                pitch_um: self.pitch_um,
                width: self.out_width,
                height: self.out_height,
            },
            seed: 0,
            channel_correlation: 0.0,
            expectation: true,
        }
    }

    /// Run G3 and G5 on the device; returns the nine-value cell targets and
    /// the three-value output pixels of the full output grid.
    fn run(&self, backend: &WgpuBackend) -> Result<(Vec<f32>, Vec<f32>), String> {
        self.run_shaders(backend, &PRODUCTION_SHADERS)
    }

    /// The same device run with substituted shader sources, for the
    /// known-bad kernel controls.
    fn run_shaders(
        &self,
        backend: &WgpuBackend,
        shaders: &SpatialShaders,
    ) -> Result<(Vec<f32>, Vec<f32>), String> {
        let params = self.params();
        let init = |label: &'static str, data: &[f32]| {
            backend
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::cast_slice(data),
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let alloc = |label: &'static str, bytes: u64| {
            backend.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let readback = |label: &'static str, bytes: u64| {
            backend.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let cell_bytes = u64::from(self.band.ext_cols) * u64::from(self.band.ext_rows) * 36;
        let output_bytes = u64::from(self.out_width) * u64::from(self.out_height) * 12;
        let source_layers = init("dye_spatial_check_source", &self.source);
        let cell_layers = alloc("dye_spatial_check_layers", cell_bytes);
        let cells = init("dye_spatial_check_field", &self.cells);
        let horizontal = alloc(
            "dye_spatial_check_horizontal",
            u64::from(self.band.ext_rows) * u64::from(self.band.cols) * 12,
        );
        let formed = alloc(
            "dye_spatial_check_formed",
            u64::from(self.band.rows) * u64::from(self.band.cols) * 12,
        );
        let output = alloc("dye_spatial_check_output", output_bytes);
        let state = build_spatial_with(
            backend,
            &params,
            &self.band,
            self.src_w,
            self.src_h,
            &source_layers,
            &cell_layers,
            &cells,
            &horizontal,
            &formed,
            &output,
            shaders,
        );
        let cell_readback = readback("dye_spatial_check_layers_readback", cell_bytes);
        let output_readback = readback("dye_spatial_check_output_readback", output_bytes);
        let mut encoder = backend.device.create_command_encoder(&Default::default());
        state.encode_remap(&mut encoder);
        state.encode_formation_readout(&mut encoder);
        encoder.copy_buffer_to_buffer(&cell_layers, 0, &cell_readback, 0, cell_bytes);
        encoder.copy_buffer_to_buffer(&output, 0, &output_readback, 0, output_bytes);
        backend.queue.submit(Some(encoder.finish()));
        Ok((
            backend.read_mapped_f32(&cell_readback, cell_bytes),
            backend.read_mapped_f32(&output_readback, output_bytes),
        ))
    }
}

impl WgpuBackend {
    /// Once-per-device self-check of the spatial kernels against closed-form
    /// values. It builds its own bounded buffers and runs `build_spatial`
    /// directly, so it cannot recurse through `prepare_dye` or `dye_field`.
    pub(super) fn certify_dye_spatial(&self) -> Result<(), String> {
        // G3 with 2 um source pixels and 4 um cells: every overlap is an
        // exact multiple of one half per axis, so the expected cell targets
        // are exact in f32 and any overlap or normalization error fails.
        // The band must cover the complete configured output: three rows of
        // three pixels.
        let band = DyeBand {
            output_y: 0,
            output_rows: 3,
            source_x: 0,
            source_y: 0,
            source_cols: 3,
            source_rows: 3,
            cell_x: -1,
            cell_y: -1,
            cols: 3,
            rows: 3,
            ext_cols: 3,
            ext_rows: 3,
        };
        let source: Vec<f32> = (1..=81).map(|value| value as f32).collect();
        let cells: Vec<f32> = (0..9).flat_map(|_| [1.0_f32, 2.0, 3.0]).collect();
        // Constant per channel over the field: identity formation and a
        // one-to-one readout must return it unchanged apart from the base.
        let mins = [0.5, 0.25, 0.125];
        let case = SpatialCase {
            src_w: 3,
            src_h: 3,
            source: source.clone(),
            cells,
            band,
            cell_um: 4.0,
            source_pitch_um: 2.0,
            taps: vec![1.0],
            mins,
            origin_um: [-4.0, -4.0],
            pitch_um: [4.0, 4.0],
            out_width: 3,
            out_height: 3,
        };
        let (layers, output) = case.run(self)?;
        // Closed form per axis: cell -1 keeps the left edge pixel, cell 0
        // averages pixels 0 and 1, cell 1 keeps the right edge pixel.
        let axis = [[1.0_f32, 0.0, 0.0], [0.5, 0.5, 0.0], [0.0, 0.0, 1.0]];
        for cy in 0..3usize {
            for cx in 0..3usize {
                for channel in 0..3 {
                    for layer in 0..3 {
                        let mut expected = 0.0_f32;
                        for (y, wy) in axis[cy].iter().enumerate() {
                            for (x, wx) in axis[cx].iter().enumerate() {
                                expected += wy * wx * source[(y * 3 + x) * 9 + channel * 3 + layer];
                            }
                        }
                        let actual = layers[(cy * 3 + cx) * 9 + channel * 3 + layer];
                        if (actual - expected).abs() > 2e-6 {
                            return Err(format!(
                                "film grain Dye: device remap self-check failed at cell {cx},{cy} channel {channel} layer {layer}: {actual} != {expected}"
                            ));
                        }
                    }
                }
            }
        }
        for pixel in 0..9usize {
            for channel in 0..3 {
                let expected = [1.0_f32, 2.0, 3.0][channel] - mins[channel] as f32;
                let actual = output[pixel * 3 + channel];
                if (actual - expected).abs() > 2e-6 {
                    return Err(format!(
                        "film grain Dye: device readout self-check failed at pixel {pixel} channel {channel}: {actual} != {expected}"
                    ));
                }
            }
        }

        // Formation with cell-integrated taps: one hot cell must produce the
        // outer product of the taps, and a one-to-one readout must preserve
        // it after the base subtraction.
        let taps = vec![0.25, 0.5, 0.25];
        let band = DyeBand {
            output_y: 0,
            output_rows: 3,
            source_x: 0,
            source_y: 0,
            source_cols: 1,
            source_rows: 1,
            cell_x: 0,
            cell_y: 0,
            cols: 3,
            rows: 3,
            ext_cols: 5,
            ext_rows: 5,
        };
        let mut field = vec![0.0_f32; 5 * 5 * 3];
        let hot = (2 * 5 + 2) * 3;
        field[hot] = 1.0;
        field[hot + 1] = 2.0;
        field[hot + 2] = 4.0;
        let case = SpatialCase {
            src_w: 1,
            src_h: 1,
            source: vec![0.0; 9],
            cells: field,
            band,
            cell_um: 1.0,
            source_pitch_um: 1.0,
            taps,
            mins,
            origin_um: [1.0, 1.0],
            pitch_um: [1.0, 1.0],
            out_width: 3,
            out_height: 3,
        };
        let (_, output) = case.run(self)?;
        let amplitude = [1.0_f32, 2.0, 4.0];
        for py in 0..3usize {
            for px in 0..3usize {
                for channel in 0..3 {
                    let expected = amplitude[channel] * case.taps[px] as f32 * case.taps[py] as f32
                        - mins[channel] as f32;
                    let actual = output[(py * 3 + px) * 3 + channel];
                    if (actual - expected).abs() > 2e-6 {
                        return Err(format!(
                            "film grain Dye: device formation self-check failed at pixel {px},{py} channel {channel}: {actual} != {expected}"
                        ));
                    }
                }
            }
        }

        // Subcell readout: a 1.5 cell footprint over constant cells must
        // keep the constant and the exact two-thirds/one-third split.
        let band = DyeBand {
            output_y: 0,
            output_rows: 2,
            source_x: 0,
            source_y: 0,
            source_cols: 1,
            source_rows: 1,
            cell_x: 1,
            cell_y: 1,
            cols: 3,
            rows: 3,
            ext_cols: 3,
            ext_rows: 3,
        };
        let cells: Vec<f32> = (0..9).flat_map(|_| [1.0_f32, 2.0, 3.0]).collect();
        let case = SpatialCase {
            src_w: 1,
            src_h: 1,
            source: vec![0.0; 9],
            cells,
            band,
            cell_um: 1.0,
            source_pitch_um: 1.0,
            taps: vec![1.0],
            mins,
            origin_um: [1.0, 1.0],
            pitch_um: [1.0, 1.5],
            out_width: 3,
            out_height: 2,
        };
        let (_, output) = case.run(self)?;
        for pixel in 0..6usize {
            for channel in 0..3 {
                let expected = [1.0_f32, 2.0, 3.0][channel] - mins[channel] as f32;
                let actual = output[pixel * 3 + channel];
                if (actual - expected).abs() > 2e-6 {
                    return Err(format!(
                        "film grain Dye: device fractional readout self-check failed at pixel {pixel} channel {channel}: {actual} != {expected}"
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::wgpu_backend::observation::TEST_DEVICE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// KNOWN-BAD G3: the remap pass compiled with un-normalized overlaps.
    const RAW_REMAP_SHADERS: SpatialShaders = SpatialShaders {
        remap: concat!(
            include_str!("dye_spatial_params.wgsl"),
            include_str!("dye_spatial_remap_control.wgsl")
        ),
        form_h: concat!(
            include_str!("dye_spatial_params.wgsl"),
            include_str!("dye_spatial_form_h.wgsl")
        ),
        form_v: concat!(
            include_str!("dye_spatial_params.wgsl"),
            include_str!("dye_spatial_form_v.wgsl")
        ),
        readout: concat!(
            include_str!("dye_spatial_params.wgsl"),
            include_str!("dye_spatial_readout.wgsl")
        ),
    };

    /// KNOWN-BAD G5: the readout pass compiled with the cell width as the
    /// overlap normalization instead of the footprint width.
    const CELL_NORM_SHADERS: SpatialShaders = SpatialShaders {
        remap: concat!(
            include_str!("dye_spatial_params.wgsl"),
            include_str!("dye_spatial_remap.wgsl")
        ),
        form_h: concat!(
            include_str!("dye_spatial_params.wgsl"),
            include_str!("dye_spatial_form_h.wgsl")
        ),
        form_v: concat!(
            include_str!("dye_spatial_params.wgsl"),
            include_str!("dye_spatial_form_v.wgsl")
        ),
        readout: concat!(
            include_str!("dye_spatial_params.wgsl"),
            include_str!("dye_spatial_readout_control.wgsl")
        ),
    };

    /// KNOWN-BAD G5: the formation passes compiled with the tap offset
    /// flipped in both axes.
    const FLIPPED_TAPS_SHADERS: SpatialShaders = SpatialShaders {
        remap: concat!(
            include_str!("dye_spatial_params.wgsl"),
            include_str!("dye_spatial_remap.wgsl")
        ),
        form_h: concat!(
            include_str!("dye_spatial_params.wgsl"),
            include_str!("dye_spatial_form_h_control.wgsl")
        ),
        form_v: concat!(
            include_str!("dye_spatial_params.wgsl"),
            include_str!("dye_spatial_form_v_control.wgsl")
        ),
        readout: concat!(
            include_str!("dye_spatial_params.wgsl"),
            include_str!("dye_spatial_readout.wgsl")
        ),
    };

    // ── f64 oracle mirroring `crates/spektrafilm-model/src/grain/v3.rs` ──

    /// Overlap weights of one cell interval with the source pixel grid,
    /// normalized to sum to one. Pixels outside the film keep the nearest
    /// edge pixel (constant edge extension).
    fn axis_stencil(lo: f64, hi: f64, pitch: f64, extent: usize) -> Vec<(usize, f64)> {
        let width = hi - lo;
        let end = extent as f64 * pitch;
        let mut out = Vec::new();
        if lo < 0.0 {
            out.push((0, (hi.min(0.0) - lo) / width));
        }
        let a = lo.max(0.0);
        let b = hi.min(end);
        if b > a {
            let first = ((a / pitch).floor() as usize).min(extent - 1);
            let past = ((b / pitch).ceil() as usize).clamp(1, extent);
            for pixel in first..past {
                let overlap = b.min((pixel + 1) as f64 * pitch) - a.max(pixel as f64 * pitch);
                if overlap > 0.0 {
                    out.push((pixel, overlap / width));
                }
            }
        }
        if hi > end {
            out.push((extent - 1, (hi - end.max(lo)) / width));
        }
        out
    }

    /// Cells and normalized weights of one output footprint.
    fn readout_stencil(lo: f64, hi: f64, cell: f64) -> Vec<(i64, f64)> {
        let width = hi - lo;
        let first = (lo / cell).floor() as i64;
        let last = (hi / cell).ceil() as i64 - 1;
        let mut out = Vec::new();
        for i in first..=last {
            let overlap = hi.min((i + 1) as f64 * cell) - lo.max(i as f64 * cell);
            if overlap > 0.0 {
                out.push((i, overlap / width));
            }
        }
        out
    }

    /// The normalization invariant every stencil must satisfy; a known-broken
    /// weight law must fail here, on the same code path as the real one.
    fn checked_sum(weights: &[f64], what: &str) -> Result<(), String> {
        let sum: f64 = weights.iter().sum();
        if (sum - 1.0).abs() > 1e-9 {
            return Err(format!(
                "film grain Dye: {what} weights sum to {sum}, expected 1"
            ));
        }
        Ok(())
    }

    #[derive(Clone, Copy, PartialEq)]
    enum RemapNorm {
        /// Correct law: overlap divided by the cell width.
        Cell,
        /// Known-broken control: overlap without normalization.
        Raw,
    }

    fn oracle_remap(case: &SpatialCase) -> Result<Vec<f32>, String> {
        oracle_remap_with(case, RemapNorm::Cell, true)
    }

    fn oracle_remap_with(
        case: &SpatialCase,
        norm: RemapNorm,
        validate: bool,
    ) -> Result<Vec<f32>, String> {
        let h = case.cell_um;
        let pitch = case.source_pitch_um;
        let band = case.band;
        let scale = match norm {
            RemapNorm::Cell => 1.0,
            RemapNorm::Raw => h,
        };
        let mut out = vec![0.0_f32; (band.ext_cols * band.ext_rows * 9) as usize];
        for cy in 0..band.ext_rows {
            for cx in 0..band.ext_cols {
                let i = i64::from(band.cell_x) + i64::from(cx);
                let j = i64::from(band.cell_y) + i64::from(cy);
                let xs = axis_stencil(i as f64 * h, (i + 1) as f64 * h, pitch, case.src_w as usize);
                let ys = axis_stencil(j as f64 * h, (j + 1) as f64 * h, pitch, case.src_h as usize);
                if validate {
                    checked_sum(
                        &xs.iter().map(|(_, w)| w * scale).collect::<Vec<_>>(),
                        "remap x",
                    )?;
                    checked_sum(
                        &ys.iter().map(|(_, w)| w * scale).collect::<Vec<_>>(),
                        "remap y",
                    )?;
                }
                let mut acc = [0.0_f64; 9];
                for &(py, wy) in &ys {
                    for &(px, wx) in &xs {
                        let sx = px as i64 - i64::from(band.source_x);
                        let sy = py as i64 - i64::from(band.source_y);
                        if sx < 0
                            || sy < 0
                            || sx >= i64::from(band.source_cols)
                            || sy >= i64::from(band.source_rows)
                        {
                            return Err(
                                "film grain Dye: remap window does not cover the touched source \
                                 pixels"
                                    .to_owned(),
                            );
                        }
                        let base = ((sy * i64::from(band.source_cols) + sx) * 9) as usize;
                        let weight = wx * wy * scale * scale;
                        for (k, value) in acc.iter_mut().enumerate() {
                            *value += weight * f64::from(case.source[base + k]);
                        }
                    }
                }
                let dst = ((cy * band.ext_cols + cx) * 9) as usize;
                for k in 0..9 {
                    out[dst + k] = acc[k] as f32;
                }
            }
        }
        Ok(out)
    }

    #[derive(Clone, Copy, PartialEq)]
    enum ReadoutNorm {
        /// Correct law: overlap divided by the footprint width.
        Footprint,
        /// Known-broken control: overlap divided by the cell width.
        Cell,
    }

    #[derive(Clone, Copy, PartialEq)]
    enum TapLaw {
        /// Correct law: tap `t` carries offset `d - t` from the cell.
        Integrated,
        /// Known-broken control: tap `t` carries offset `+t`.
        Flipped,
    }

    fn oracle_output(case: &SpatialCase) -> Result<Vec<f32>, String> {
        oracle_output_with(case, ReadoutNorm::Footprint, TapLaw::Integrated, true)
    }

    fn oracle_output_with(
        case: &SpatialCase,
        norm: ReadoutNorm,
        tap_law: TapLaw,
        validate: bool,
    ) -> Result<Vec<f32>, String> {
        let taps_sum: f64 = case.taps.iter().sum();
        if validate && (taps_sum - 1.0).abs() > 1e-9 {
            return Err(format!(
                "film grain Dye: formation taps sum to {taps_sum}, expected 1"
            ));
        }
        let h = case.cell_um;
        let d = (case.taps.len() - 1) / 2;
        let tap_offset = |t: usize| match tap_law {
            TapLaw::Integrated => 2 * d - t,
            TapLaw::Flipped => t,
        };
        let band = case.band;
        let ext_cols = band.ext_cols as usize;
        let cols = band.cols as usize;
        let ext_rows = band.ext_rows as usize;
        let rows = band.rows as usize;
        let mut hbuf = vec![0.0_f64; ext_rows * cols * 3];
        let mut fbuf = vec![0.0_f64; rows * cols * 3];
        for c in 0..3 {
            for r in 0..ext_rows {
                for ci in 0..cols {
                    let mut acc = 0.0;
                    for t in 0..=2 * d {
                        acc += case.taps[t]
                            * f64::from(case.cells[(r * ext_cols + (ci + tap_offset(t))) * 3 + c]);
                    }
                    hbuf[(r * cols + ci) * 3 + c] = acc;
                }
            }
            for k in 0..rows {
                for ci in 0..cols {
                    let mut acc = 0.0;
                    for t in 0..=2 * d {
                        acc += case.taps[t] * hbuf[((k + tap_offset(t)) * cols + ci) * 3 + c];
                    }
                    fbuf[(k * cols + ci) * 3 + c] = acc;
                }
            }
        }
        let [x0, y0] = case.origin_um;
        let [col_w, row_w] = case.pitch_um;
        let inner_x = i64::from(band.cell_x) + d as i64;
        let inner_y = i64::from(band.cell_y) + d as i64;
        let mut out = vec![0.0_f32; (case.out_width * case.out_height * 3) as usize];
        for py in 0..case.out_height {
            let ky = f64::from(band.output_y + py);
            let ys = readout_stencil(y0 + ky * row_w, y0 + (ky + 1.0) * row_w, h);
            for px in 0..case.out_width {
                let kx = f64::from(px);
                let xs = readout_stencil(x0 + kx * col_w, x0 + (kx + 1.0) * col_w, h);
                let (scale_x, scale_y) = match norm {
                    ReadoutNorm::Footprint => (1.0, 1.0),
                    ReadoutNorm::Cell => (col_w / h, row_w / h),
                };
                if validate {
                    checked_sum(
                        &ys.iter().map(|(_, w)| w * scale_y).collect::<Vec<_>>(),
                        "readout y",
                    )?;
                    checked_sum(
                        &xs.iter().map(|(_, w)| w * scale_x).collect::<Vec<_>>(),
                        "readout x",
                    )?;
                }
                let mut acc = [0.0_f64; 3];
                for &(cj, wy) in &ys {
                    for &(ci, wx) in &xs {
                        let lx = ci - inner_x;
                        let ly = cj - inner_y;
                        if lx < 0 || ly < 0 || lx >= cols as i64 || ly >= rows as i64 {
                            return Err(
                                "film grain Dye: readout window does not cover the formed cells"
                                    .to_owned(),
                            );
                        }
                        let weight = wx * wy * scale_x * scale_y;
                        let base = ((ly * cols as i64 + lx) * 3) as usize;
                        for c in 0..3 {
                            acc[c] += weight * fbuf[base + c];
                        }
                    }
                }
                let dst = ((py * case.out_width + px) * 3) as usize;
                for c in 0..3 {
                    out[dst + c] = (acc[c] - case.mins[c]) as f32;
                }
            }
        }
        Ok(out)
    }

    // ── case construction ──

    struct Geometry {
        src_w: u32,
        src_h: u32,
        h: f64,
        pitch: f64,
        origin: [f64; 2],
        out_pitch: [f64; 2],
        out: (u32, u32),
        taps: Vec<f64>,
        mins: [f64; 3],
    }

    fn params_of(g: &Geometry) -> DyeGpuParams {
        DyeGpuParams {
            tables: DyeChannelTablesGpu {
                tables: std::array::from_fn(|c| DyeChannelTableGpu {
                    min: g.mins[c] as f32,
                    ..Default::default()
                }),
                curves: Vec::new(),
            },
            cell_um: g.h,
            taps: g.taps.clone(),
            source_pitch_um: g.pitch,
            grid: DyeReadoutGrid {
                origin_um: g.origin,
                pitch_um: g.out_pitch,
                width: g.out.0,
                height: g.out.1,
            },
            seed: 0,
            channel_correlation: 0.0,
            expectation: true,
        }
    }

    fn source_pattern(width: u32, height: u32) -> Vec<f32> {
        (0..width * height * 9)
            .map(|k| (((k * 37 + 11) % 23) as f32) * 0.1 - 1.0)
            .collect()
    }

    fn field(i: i32, j: i32) -> [f32; 3] {
        let hash = |c: i32| {
            let v = i.wrapping_mul(73856093) ^ j.wrapping_mul(19349663) ^ c.wrapping_mul(83492791);
            ((v >> 7) & 255) as f32 / 32.0 - 2.0
        };
        [hash(0), hash(1), hash(2)]
    }

    fn case_for_rows(g: &Geometry, row0: u32, row1: u32) -> SpatialCase {
        let params = params_of(g);
        let band = band_geometry(g.src_w, g.src_h, &params, row0, row1).expect("band geometry");
        let source_full = source_pattern(g.src_w, g.src_h);
        let mut source = Vec::with_capacity((band.source_cols * band.source_rows * 9) as usize);
        for sy in 0..band.source_rows {
            for sx in 0..band.source_cols {
                let base = (((band.source_y + sy) * g.src_w + band.source_x + sx) * 9) as usize;
                source.extend_from_slice(&source_full[base..base + 9]);
            }
        }
        let mut cells = Vec::with_capacity((band.ext_cols * band.ext_rows * 3) as usize);
        for cy in 0..band.ext_rows {
            for cx in 0..band.ext_cols {
                cells.extend(field(band.cell_x + cx as i32, band.cell_y + cy as i32));
            }
        }
        SpatialCase {
            src_w: g.src_w,
            src_h: g.src_h,
            source,
            cells,
            band,
            cell_um: g.h,
            source_pitch_um: g.pitch,
            taps: g.taps.clone(),
            mins: g.mins,
            origin_um: g.origin,
            pitch_um: g.out_pitch,
            out_width: g.out.0,
            out_height: g.out.1,
        }
    }

    fn max_error(a: &[f32], b: &[f32]) -> f64 {
        a.iter()
            .zip(b.iter())
            .map(|(x, y)| (f64::from(*x) - f64::from(*y)).abs())
            .fold(0.0, f64::max)
    }

    // ── placed boundary sweeps ──

    #[test]
    fn remap_matches_the_oracle_at_placed_boundaries() {
        let _guard = device_lock();
        let backend = crate::test_support::adapter();
        let mut maximum_error = 0.0_f64;
        let mut constant_error = 0.0_f64;
        for exponent in [0, 1, 5, 10, 15, 20] {
            for sign in [-1.0, 1.0] {
                let phase = sign * 2.0_f64.powi(-exponent);
                for (h, pitch) in [(1.75, 1.13), (0.9, 2.7), (2.5, 2.5)] {
                    let g = Geometry {
                        src_w: 17,
                        src_h: 13,
                        h,
                        pitch,
                        origin: [3.0 * h + phase, -2.0 * h + phase],
                        out_pitch: [2.9, 1.6],
                        out: (9, 7),
                        taps: vec![1.0],
                        mins: [0.125, 0.25, 0.5],
                    };
                    let case = case_for_rows(&g, 0, g.out.1);
                    let (layers, _) = case.run(&backend).expect("device run");
                    let expected = oracle_remap(&case).expect("oracle");
                    maximum_error = maximum_error.max(max_error(&layers, &expected));
                    // A constant source must stay constant through the remap.
                    let mut constant_case = case.clone();
                    for value in constant_case.source.iter_mut() {
                        *value = 0.75;
                    }
                    let (constant, _) = constant_case.run(&backend).expect("constant run");
                    let constant_expected = oracle_remap(&constant_case).expect("constant oracle");
                    constant_error = constant_error.max(max_error(&constant, &constant_expected));
                    for value in &constant {
                        constant_error = constant_error.max((f64::from(*value) - 0.75).abs());
                    }
                }
            }
        }
        eprintln!(
            "G3 placed boundaries +/-2^-0..2^-20, three ratios: max error {maximum_error:.3e}, \
             constant error {constant_error:.3e} on {}",
            backend.name
        );
        assert!(
            maximum_error < 1e-5,
            "G3 boundary sweep error {maximum_error}"
        );
        assert!(
            constant_error < 1e-5,
            "G3 constant preservation error {constant_error}"
        );
    }

    #[test]
    fn readout_matches_the_oracle_at_placed_boundaries() {
        let _guard = device_lock();
        let backend = crate::test_support::adapter();
        let mut maximum_error = 0.0_f64;
        for exponent in [0, 1, 5, 10, 15, 20] {
            for sign in [-1.0, 1.0] {
                let phase = sign * 2.0_f64.powi(-exponent);
                for out_pitch in [[2.9, 1.6], [1.0, 1.0], [5.357, 0.7]] {
                    let g = Geometry {
                        src_w: 17,
                        src_h: 13,
                        h: 1.75,
                        pitch: 1.13,
                        origin: [0.4 * 1.75 + phase, -1.3 * 1.75 + phase],
                        out_pitch,
                        out: (9, 7),
                        taps: vec![0.05, 0.11, 0.18, 0.32, 0.18, 0.11, 0.05],
                        mins: [0.125, 0.25, 0.5],
                    };
                    let case = case_for_rows(&g, 0, g.out.1);
                    let (_, output) = case.run(&backend).expect("device run");
                    let expected = oracle_output(&case).expect("oracle");
                    maximum_error = maximum_error.max(max_error(&output, &expected));
                }
            }
        }
        eprintln!(
            "G5 readout placed boundaries +/-2^-0..2^-20, subcell and coarse footprints: max \
             error {maximum_error:.3e} on {}",
            backend.name
        );
        assert!(
            maximum_error < 1e-5,
            "readout boundary sweep error {maximum_error}"
        );
    }

    // ── tap law and edges ──

    #[test]
    fn formation_uses_the_cell_integrated_tap_law() {
        let _guard = device_lock();
        let backend = crate::test_support::adapter();
        let g = Geometry {
            src_w: 3,
            src_h: 3,
            h: 1.0,
            pitch: 1.0,
            origin: [0.0, 0.0],
            out_pitch: [1.0, 1.0],
            out: (5, 5),
            taps: vec![0.05, 0.11, 0.18, 0.32, 0.18, 0.11, 0.05],
            mins: [0.0; 3],
        };
        let mut case = case_for_rows(&g, 0, g.out.1);
        let d = (case.taps.len() - 1) / 2;
        let lx = 2 + d;
        let ly = 2 + d;
        let hot = (ly * case.band.ext_cols as usize + lx) * 3;
        case.cells[hot] = 1.0;
        case.cells[hot + 1] = -0.5;
        case.cells[hot + 2] = 0.25;
        let (_, output) = case.run(&backend).expect("device run");
        let expected = oracle_output(&case).expect("oracle");
        let error = max_error(&output, &expected);
        eprintln!(
            "G5 formation impulse response of the integrated taps: max error {error:.3e} on {}",
            backend.name
        );
        assert!(error < 1e-5, "formation tap law error {error}");

        // Known-broken control: a point-sampled Gaussian substitute of a
        // different width must fail the same oracle comparison.
        let mut wrong = case.clone();
        wrong.taps = vec![0.02, 0.06, 0.14, 0.56, 0.14, 0.06, 0.02];
        let (_, wrong_output) = wrong.run(&backend).expect("control run");
        let difference = max_error(&wrong_output, &expected);
        assert!(
            difference > 1e-5,
            "a substituted tap law reproduced the oracle: {difference}"
        );
    }

    #[test]
    fn edge_extension_keeps_the_film_boundary() {
        let _guard = device_lock();
        let backend = crate::test_support::adapter();
        let g = Geometry {
            src_w: 8,
            src_h: 5,
            h: 3.0,
            pitch: 1.0,
            origin: [-2.0, -2.0],
            out_pitch: [2.0, 2.0],
            out: (7, 5),
            taps: vec![1.0],
            mins: [0.0; 3],
        };
        let mut maximum_error = 0.0_f64;
        for corner in [(0u32, 0u32), (7, 4)] {
            let mut case = case_for_rows(&g, 0, g.out.1);
            for value in case.source.iter_mut() {
                *value = 0.0;
            }
            let base = ((corner.1 * g.src_w + corner.0) * 9) as usize;
            for k in 0..9 {
                case.source[base + k] = 1.0 + k as f32 * 0.125;
            }
            let (layers, _) = case.run(&backend).expect("device run");
            let expected = oracle_remap(&case).expect("oracle");
            maximum_error = maximum_error.max(max_error(&layers, &expected));
        }
        eprintln!(
            "G3 edge impulses with constant extension: max error {maximum_error:.3e} on {}",
            backend.name
        );
        assert!(maximum_error < 1e-5, "edge extension error {maximum_error}");
    }

    // ── large global coordinates ──

    #[test]
    fn large_global_indices_match_the_oracle() {
        let _guard = device_lock();
        let backend = crate::test_support::adapter();
        let mut remap_error = 0.0_f64;
        let mut readout_error = 0.0_f64;
        for phase in [0.0, 2.0_f64.powi(-12), -2.0_f64.powi(-12)] {
            let g = Geometry {
                src_w: 41,
                src_h: 29,
                h: 1.75,
                pitch: 0.91,
                origin: [15000.0 * 1.75 + phase, -12000.0 * 1.75 - phase],
                out_pitch: [5.357, 2.4],
                out: (24, 6),
                taps: vec![0.05, 0.11, 0.18, 0.32, 0.18, 0.11, 0.05],
                mins: [0.125, 0.25, 0.5],
            };
            let case = case_for_rows(&g, 0, g.out.1);
            let (layers, output) = case.run(&backend).expect("device run");
            remap_error =
                remap_error.max(max_error(&layers, &oracle_remap(&case).expect("oracle")));
            readout_error =
                readout_error.max(max_error(&output, &oracle_output(&case).expect("oracle")));
        }
        eprintln!(
            "G3+G5 large global indices (~15k cells, fractional footprints): remap \
             {remap_error:.3e}, readout {readout_error:.3e} on {}",
            backend.name
        );
        assert!(
            remap_error < 1e-5,
            "large-coordinate remap error {remap_error}"
        );
        assert!(
            readout_error < 1e-5,
            "large-coordinate readout error {readout_error}"
        );
    }

    // ── banding and cropping ──

    #[test]
    fn band_partitioning_preserves_every_value_bitwise() {
        let _guard = device_lock();
        let backend = crate::test_support::adapter();
        let g = Geometry {
            src_w: 23,
            src_h: 19,
            h: 1.75,
            pitch: 1.13,
            origin: [-7.3, 2.1],
            out_pitch: [2.9, 1.6],
            out: (8, 300),
            taps: vec![0.05, 0.11, 0.18, 0.32, 0.18, 0.11, 0.05],
            mins: [0.125, 0.25, 0.5],
        };
        let whole = case_for_rows(&g, 0, 300);
        let (whole_layers, whole_output) = whole.run(&backend).expect("whole run");
        for (row0, row1) in [(0u32, 100u32), (100, 220), (220, 300)] {
            let part = case_for_rows(&g, row0, row1);
            let (layers, output) = part.run(&backend).expect("band run");
            let start = (row0 * g.out.0 * 3) as usize;
            let end = (row1 * g.out.0 * 3) as usize;
            assert_eq!(
                &output[start..end],
                &whole_output[start..end],
                "band rows {row0}..{row1} differ from the whole run"
            );
            for cy in 0..part.band.ext_rows {
                for cx in 0..part.band.ext_cols {
                    let i = part.band.cell_x + cx as i32;
                    let j = part.band.cell_y + cy as i32;
                    let whole_cx = i - whole.band.cell_x;
                    let whole_cy = j - whole.band.cell_y;
                    if whole_cx >= 0
                        && whole_cy >= 0
                        && whole_cx < whole.band.ext_cols as i32
                        && whole_cy < whole.band.ext_rows as i32
                    {
                        let a = ((cy * part.band.ext_cols + cx) * 9) as usize;
                        let b = ((whole_cy as u32 * whole.band.ext_cols + whole_cx as u32) * 9)
                            as usize;
                        assert_eq!(
                            &layers[a..a + 9],
                            &whole_layers[b..b + 9],
                            "cell {i},{j} differs between bandings"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn crop_reads_keep_global_footprints() {
        let _guard = device_lock();
        let backend = crate::test_support::adapter();
        let g = Geometry {
            src_w: 23,
            src_h: 19,
            h: 1.75,
            pitch: 1.13,
            origin: [-7.3, 2.1],
            out_pitch: [2.9, 1.6],
            out: (9, 7),
            taps: vec![0.05, 0.11, 0.18, 0.32, 0.18, 0.11, 0.05],
            mins: [0.125, 0.25, 0.5],
        };
        let full = case_for_rows(&g, 0, g.out.1);
        let (_, full_output) = full.run(&backend).expect("full run");
        let (full_width, _) = g.out;
        let crop = Geometry {
            origin: [
                g.origin[0] + 3.0 * g.out_pitch[0],
                g.origin[1] + 2.0 * g.out_pitch[1],
            ],
            out: (6, 5),
            ..g
        };
        let crop_case = case_for_rows(&crop, 0, crop.out.1);
        let (_, crop_output) = crop_case.run(&backend).expect("crop run");
        let mut error = 0.0_f64;
        for py in 0..5u32 {
            for px in 0..6u32 {
                for c in 0..3u32 {
                    let a = (((py + 2) * full_width + (px + 3)) * 3 + c) as usize;
                    let b = ((py * crop.out.0 + px) * 3 + c) as usize;
                    error =
                        error.max((f64::from(full_output[a]) - f64::from(crop_output[b])).abs());
                }
            }
        }
        eprintln!(
            "G5 crop keeps global footprints: max difference {error:.3e} on {}",
            backend.name
        );
        assert!(error < 1e-5, "crop difference {error}");
    }

    // ── bounded resource planning ──

    #[test]
    fn resource_bounds_reduce_the_band_and_refuse_impossible_rows() {
        // Host-only: `DyePlan::new` must decide the whole bounded
        // decomposition before any device buffer exists, so an artificial
        // storage limit reduces the band height and a geometry whose single
        // row cannot fit fails with the named one-row error instead of
        // failing after allocation.
        let g = Geometry {
            src_w: 23,
            src_h: 19,
            h: 1.75,
            pitch: 1.13,
            origin: [-7.3, 2.1],
            out_pitch: [2.9, 1.6],
            out: (64, 200),
            taps: vec![0.05, 0.11, 0.18, 0.32, 0.18, 0.11, 0.05],
            mins: [0.0; 3],
        };
        let params = params_of(&g);
        let largest_buffer = |band: &DyeBand| {
            [
                u64::from(band.source_cols) * u64::from(band.source_rows) * 36,
                u64::from(band.ext_cols) * u64::from(band.ext_rows) * 36,
                u64::from(band.ext_cols) * u64::from(band.ext_rows) * 12,
                u64::from(band.ext_rows) * u64::from(band.cols) * 12,
                u64::from(band.rows) * u64::from(band.cols) * 12,
            ]
            .into_iter()
            .fold(0, u64::max)
        };
        let one = band_geometry(g.src_w, g.src_h, &params, 0, 1).expect("one row");
        let full =
            band_geometry(g.src_w, g.src_h, &params, 0, BAND_OUTPUT_ROWS).expect("full band");
        // A device that admits a few rows of band storage, but not a full
        // 128-row band, must still serve the geometry.
        let bound = (largest_buffer(&one) + largest_buffer(&full)) / 2;
        assert!(band_fit(&one, bound).is_ok());
        assert!(band_fit(&full, bound).is_err());
        let limits = wgpu::Limits {
            max_buffer_size: bound,
            max_storage_buffer_binding_size: bound as u32,
            ..Default::default()
        };
        let plan = DyePlan::new(g.src_w, g.src_h, &params, &limits).expect("reduced plan");
        assert!(
            plan.bands
                .iter()
                .any(|band| band.output_rows < BAND_OUTPUT_ROWS),
            "the storage limit did not reduce the band height"
        );
        let tall = DyePlan::new(g.src_w, g.src_h, &params, &wgpu::Limits::default())
            .expect("unbounded plan");
        assert!(
            tall.bands[0].output_rows > plan.bands[0].output_rows,
            "bounded planning did not shrink the band below the default plan"
        );
        for band in &plan.bands {
            band_fit(band, bound).expect("planned band exceeds the storage limit");
        }
        assert_eq!(
            plan.bands.iter().map(|band| band.output_rows).sum::<u32>(),
            g.out.1,
            "the bands must cover the complete output"
        );

        // A device that cannot hold even one row of this geometry must fail
        // with the named one-row error, before any allocation.
        let impossible = largest_buffer(&one) - 1;
        let tiny = wgpu::Limits {
            max_buffer_size: impossible,
            max_storage_buffer_binding_size: impossible as u32,
            ..Default::default()
        };
        let error = DyePlan::new(g.src_w, g.src_h, &params, &tiny)
            .err()
            .expect("one row should not fit the artificial device");
        assert!(
            error.contains("one output row"),
            "unnamed one-row failure: {error}"
        );
    }

    // ── known-broken controls ──
    //
    // Each control executes an actually altered kernel on the device and is
    // rejected by the same independent oracle comparison that admits the
    // production pass; the CPU copy of the broken law only pins down which
    // law the control realizes.

    #[test]
    fn oracle_rejects_wrong_overlap_normalization() {
        let _guard = device_lock();
        let backend = crate::test_support::adapter();
        let g = Geometry {
            src_w: 9,
            src_h: 7,
            h: 1.75,
            pitch: 1.13,
            origin: [0.3, -1.1],
            out_pitch: [2.9, 1.6],
            out: (5, 4),
            taps: vec![1.0],
            mins: [0.0; 3],
        };
        let case = case_for_rows(&g, 0, g.out.1);
        let expected = oracle_remap(&case).expect("oracle");
        let (layers, _) = case.run(&backend).expect("device run");
        let difference = max_error(&layers, &expected);
        assert!(
            difference < 1e-5,
            "production G3 diverged from the oracle: {difference}"
        );
        // Known-bad kernel control: the G3 shader compiled with raw,
        // un-normalized overlaps runs on the device and must be rejected by
        // the same oracle comparison.
        let (wrong, _) = case
            .run_shaders(&backend, &RAW_REMAP_SHADERS)
            .expect("control run");
        let difference = max_error(&wrong, &expected);
        assert!(
            difference > 1e-5,
            "un-normalized overlaps reproduced the oracle: {difference}"
        );
        // The control realizes exactly the modeled raw-overlap law; its
        // larger-magnitude sums leave f32 room above the frozen tolerance.
        let modeled = oracle_remap_with(&case, RemapNorm::Raw, false).expect("control values");
        let difference = max_error(&wrong, &modeled);
        assert!(
            difference < 1e-4,
            "the control kernel diverged from the raw-overlap law: {difference}"
        );
        // The same invariant check that admits the correct law rejects the
        // un-normalized weights.
        assert!(oracle_remap_with(&case, RemapNorm::Raw, true).is_err());
    }

    #[test]
    fn oracle_rejects_wrong_readout_width() {
        let _guard = device_lock();
        let backend = crate::test_support::adapter();
        let g = Geometry {
            src_w: 9,
            src_h: 7,
            h: 1.75,
            pitch: 1.13,
            origin: [0.3, -1.1],
            out_pitch: [2.9, 1.6],
            out: (5, 4),
            taps: vec![0.05, 0.11, 0.18, 0.32, 0.18, 0.11, 0.05],
            mins: [0.125, 0.25, 0.5],
        };
        let case = case_for_rows(&g, 0, g.out.1);
        let expected = oracle_output(&case).expect("oracle");
        let (_, output) = case.run(&backend).expect("device run");
        let difference = max_error(&output, &expected);
        assert!(
            difference < 1e-5,
            "production G5 diverged from the oracle: {difference}"
        );
        // Known-bad kernel control: the G5 readout shader compiled with the
        // cell width as the overlap normalization runs on the device and
        // must be rejected by the same oracle comparison.
        let (_, wrong) = case
            .run_shaders(&backend, &CELL_NORM_SHADERS)
            .expect("control run");
        let difference = max_error(&wrong, &expected);
        assert!(
            difference > 1e-5,
            "wrong readout width reproduced the oracle: {difference}"
        );
        let modeled = oracle_output_with(&case, ReadoutNorm::Cell, TapLaw::Integrated, false)
            .expect("control values");
        let difference = max_error(&wrong, &modeled);
        assert!(
            difference < 1e-4,
            "the control kernel diverged from the cell-width law: {difference}"
        );
        assert!(oracle_output_with(&case, ReadoutNorm::Cell, TapLaw::Integrated, true).is_err());
    }

    #[test]
    fn oracle_rejects_wrong_tap_offset() {
        let _guard = device_lock();
        let backend = crate::test_support::adapter();
        // Asymmetric taps: a flipped tap offset is invisible for a symmetric
        // tap law, so the control case must not be symmetric.
        let g = Geometry {
            src_w: 9,
            src_h: 7,
            h: 1.75,
            pitch: 1.13,
            origin: [0.3, -1.1],
            out_pitch: [2.9, 1.6],
            out: (5, 4),
            taps: vec![0.02, 0.06, 0.12, 0.30, 0.20, 0.16, 0.14],
            mins: [0.125, 0.25, 0.5],
        };
        let case = case_for_rows(&g, 0, g.out.1);
        let expected = oracle_output(&case).expect("oracle");
        let (_, output) = case.run(&backend).expect("device run");
        let difference = max_error(&output, &expected);
        assert!(
            difference < 1e-5,
            "production G5 formation diverged from the oracle: {difference}"
        );
        // Known-bad kernel control: the formation shaders compiled with the
        // tap offset flipped in both axes run on the device and must be
        // rejected by the same oracle comparison.
        let (_, wrong) = case
            .run_shaders(&backend, &FLIPPED_TAPS_SHADERS)
            .expect("control run");
        let difference = max_error(&wrong, &expected);
        assert!(
            difference > 1e-5,
            "flipped tap offset reproduced the oracle: {difference}"
        );
        let modeled = oracle_output_with(&case, ReadoutNorm::Footprint, TapLaw::Flipped, false)
            .expect("control values");
        let difference = max_error(&wrong, &modeled);
        assert!(
            difference < 1e-4,
            "the control kernel diverged from the flipped tap law: {difference}"
        );
    }

    // ── runtime self-check ──

    #[test]
    fn certify_dye_spatial_passes_on_this_device() {
        let _guard = device_lock();
        let backend = crate::test_support::adapter();
        backend.certify_dye_spatial().expect("spatial self-check");
        eprintln!("G3+G5 runtime self-check passed on {}", backend.name);
    }
}
