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
    /// Per-band maxima of [`band_bytes`], in the same binding order: the
    /// scratch buffers `prepare_dye` allocates and `dye_plan` validates.
    pub max_band_bytes: [u64; 5],
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

/// Names of the band scratch buffers, in binding order. The per-band bound,
/// the plan maxima and the allocations all read [`band_bytes`] in this order,
/// so a name cannot drift from its size.
pub(super) const BAND_BUFFERS: [&str; 5] = [
    "source layers",
    "cell layers",
    "cell density",
    "horizontal formation",
    "formed density",
];

/// Byte size of every band scratch buffer, in binding order: the two
/// nine-float `source`/`cell layers` tables, the cell density, the horizontal
/// formation buffer, and the formed density. This is the one rule behind the
/// per-band bound [`band_fit`] enforces, the maxima `dye_plan` validates, and
/// the buffers `prepare_dye` and the shared runner allocate.
pub(super) fn band_bytes(band: &DyeBand) -> [u64; 5] {
    [
        u64::from(band.source_cols) * u64::from(band.source_rows) * 36,
        u64::from(band.ext_cols) * u64::from(band.ext_rows) * 36,
        u64::from(band.ext_cols) * u64::from(band.ext_rows) * 12,
        u64::from(band.ext_rows) * u64::from(band.cols) * 12,
        u64::from(band.rows) * u64::from(band.cols) * 12,
    ]
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
    for (name, bytes) in BAND_BUFFERS.iter().zip(band_bytes(band)) {
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
        let mut max_band_bytes = [0u64; 5];
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
            for (max, bytes) in max_band_bytes.iter_mut().zip(band_bytes(&band)) {
                *max = (*max).max(bytes);
            }
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
            max_band_bytes,
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
        let [_, cell_bytes, _, horizontal_bytes, formed_bytes] = band_bytes(&self.band);
        let output_bytes = u64::from(self.out_width) * u64::from(self.out_height) * 12;
        let source_layers = init("dye_spatial_check_source", &self.source);
        let cell_layers = alloc("dye_spatial_check_layers", cell_bytes);
        let cells = init("dye_spatial_check_field", &self.cells);
        let horizontal = alloc("dye_spatial_check_horizontal", horizontal_bytes);
        let formed = alloc("dye_spatial_check_formed", formed_bytes);
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
mod tests;
