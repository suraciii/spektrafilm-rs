//! Film-coordinate conditional dye-mass grain (Dye).
//!
//! The model realizes independent conditional Poisson dye masses on a global
//! film-cell grid, forms the complete realization with a normalized truncated
//! Gaussian, and reads the formed field through rectangular output
//! footprints. `specs/film-grain/v3/design.md` owns the contract; this module
//! implements it and `specs/film-grain/v3/spec.md` owns the supported
//! conditions.

use rayon::prelude::*;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};
use spektrafilm_math::stats::{FastStatsRng, cell_stream_index, poisson_exact};

mod experiment_mass;
use experiment_mass::MassLaw;

type CellResponse<'a> = &'a (dyn Fn(&ImageBuf) -> Result<ImageBuf, String> + Sync);

/// Production field pitch as a fraction of the support sigma (`h = sigma/2`).
const CELLS_PER_SIGMA: f64 = 2.0;
/// Gaussian truncation half-width in sigmas.
const SUPPORT_SIGMAS: f64 = 4.0;
/// Output rows per readout band before the cell budget applies.
const BAND_OUTPUT_ROWS: usize = 128;
/// Largest cell or source-pixel count a single band may hold. Bands are
/// storage partitions of the global field, so a band that would exceed it
/// holds fewer output rows; the field is never coarsened. A single output
/// row above the budget fails with an actionable error.
const MAX_BAND_CELLS: u64 = 1 << 24;

/// First stream tag reserved for the per-layer shared counts.
///
/// Tags `0..=8` are the per-`(channel, layer)` streams (`c * 3 + l`) and stay
/// as they are, so adding the shared streams cannot move any existing
/// sequence. Tags `9..=11` are the shared stream of layers 0..=2. The shared
/// stream is per layer, not one stream for all three: all three layers of one
/// cell share the same cell address, so a single shared tag would hand them
/// the same uniform sequence and correlate three independently coated
/// emulsions.
const SHARED_TAG_BASE: u64 = 9;

/// Rectangular readout footprints in global film coordinates. Cropping
/// changes the origin and dimensions, while retaining the full-frame pitch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReadoutGrid {
    pub origin_um: [f64; 2],
    pub pitch_um: [f64; 2],
    pub width: u32,
    pub height: u32,
}

/// Resolved Dye inputs. Plain data — no profile or runtime types.
#[derive(Debug, Clone)]
pub struct GrainV3Params {
    /// Gaussian sigma of the dye support in micrometers.
    pub support_um: f64,
    /// Field-cell pitch in micrometers. `None` selects the production value
    /// `support_um / 2`; numerical certification may pass a finer pitch. It
    /// must not be coarser than `support_um / 2`.
    pub cell_um: Option<f64>,
    /// Statistical event area in square micrometers before channel and layer
    /// scaling. It controls count variance, never the support width.
    pub event_area_um2: f64,
    /// Fraction of a layer's count rate shared by its active channels. The
    /// structural domain is `[0, 1)`; the accepted range stops at the measured
    /// inter-record coherence ceiling, so validation rejects a value above it.
    /// It sets the channel correlation without changing any channel's marginal
    /// count law.
    pub channel_correlation: f64,
    /// Per-channel event-area scale (the stock granularity ratio squared).
    pub channel_scale: [f64; 3],
    /// Per-layer event-area scale.
    pub particle_scale_sublayers: [f64; 3],
    /// Per-channel base density offset.
    pub density_min: [f64; 3],
    /// Per-channel grain uniformity in `[0, 1]`.
    pub uniformity: [f64; 3],
    /// Raw per-layer density maxima `[layer][channel]`.
    pub layer_max: [[f64; 3]; 3],
    /// Normalized composite density curves: the x axis of the layer split.
    pub composite_curves: Vec<[f64; 3]>,
    /// Raw per-layer density curves `[exposure][layer][channel]`.
    pub layer_curves: Vec<[[f64; 3]; 3]>,
    /// Positive films flip the composite lookup axis.
    pub positive_film: bool,
    /// Full 64-bit seed; cell addresses derive from it.
    pub seed: u64,
    /// Replace every Poisson count with its conditional mean (diagnostic).
    pub expectation: bool,
}

/// Realize the conditional dye field and read the global film `grid`.
///
/// `target` is the full-source developed CMY density and `pitch_um` its pixel
/// pitch on the full film. The returned image holds the readout density with
/// `density_min` subtracted once.
pub fn apply(
    target: &ImageBuf,
    pitch_um: f64,
    grid: ReadoutGrid,
    params: &GrainV3Params,
) -> Result<ImageBuf, String> {
    apply_with_budget(target, pitch_um, grid, params, MAX_BAND_CELLS)
}

/// Experimental optical readout of the same formed field. The route owner
/// converts each bounded CMY band (base offset already subtracted) into a
/// linear response. Exact cell overlaps average that response, before any
/// nonlinear development or output encoding. Exactness is relative to the
/// piecewise-constant field; it does not recover within-cell fluctuations.
pub fn apply_with_response(
    target: &ImageBuf,
    pitch_um: f64,
    grid: ReadoutGrid,
    params: &GrainV3Params,
    response: impl Fn(&ImageBuf) -> Result<ImageBuf, String> + Sync,
) -> Result<ImageBuf, String> {
    // Existing spectral backends may materialize wavelength tensors. Bound
    // the band rather than allocate them for a full film canvas.
    apply_impl(target, pitch_um, grid, params, 1 << 20, Some(&response))
}

fn apply_with_budget(
    target: &ImageBuf,
    pitch_um: f64,
    grid: ReadoutGrid,
    params: &GrainV3Params,
    max_band_cells: u64,
) -> Result<ImageBuf, String> {
    apply_impl(target, pitch_um, grid, params, max_band_cells, None)
}

fn apply_impl(
    target: &ImageBuf,
    pitch_um: f64,
    grid: ReadoutGrid,
    params: &GrainV3Params,
    max_band_cells: u64,
    response: Option<CellResponse<'_>>,
) -> Result<ImageBuf, String> {
    let model = Model::new(params, pitch_um)?;
    let src_w = target.width as usize;
    let src_h = target.height as usize;
    if src_w == 0 || src_h == 0 {
        return Err("film grain Dye: source dimensions must be positive".to_owned());
    }
    let out_width = grid.width;
    let out_height = grid.height;
    if grid.origin_um.iter().any(|v| !v.is_finite())
        || grid.pitch_um.iter().any(|v| !v.is_finite() || *v <= 0.0)
    {
        return Err(
            "film grain Dye: readout origin must be finite and readout pitch finite and positive"
                .to_owned(),
        );
    }
    if out_width == 0 || out_height == 0 {
        return Err("film grain Dye: output dimensions must be positive".to_owned());
    }
    check_target_range(target, &model)?;

    let h = model.cell_um;
    // Requested region in film coordinates.
    let [x0, y0] = grid.origin_um;
    let x1 = x0 + out_width as f64 * grid.pitch_um[0];
    let y1 = y0 + out_height as f64 * grid.pitch_um[1];

    let d = model.radius;
    let dr = d as usize;
    // Cells the readout footprints can touch, in global cell indices.
    // Packing two signed 32-bit coordinates gives every supported cell a
    // distinct 64-bit stream address, including the formation halo.
    let address_limit = i32::MAX as f64 - d as f64 - 1.0;
    if [x0, x1, y0, y1]
        .iter()
        .any(|v| !v.is_finite() || (*v / h).abs() > address_limit)
        || !(src_w as f64 * pitch_um).is_finite()
        || !(src_h as f64 * pitch_um).is_finite()
    {
        return Err("film grain Dye: film coordinates and support halo exceed the signed 32-bit cell address range; reduce camera.film_format_mm or move the requested region closer to the film origin".to_owned());
    }
    let inner_i0 = (x0 / h).floor() as i64;
    let inner_i1 = (x1 / h).ceil() as i64 - 1;
    let cols = (inner_i1 - inner_i0 + 1) as usize;
    let ext_cols = cols + 2 * d as usize;
    let [col_w, row_w] = grid.pitch_um;

    if col_w <= 0.0
        || row_w <= 0.0
        || x0 + col_w == x0
        || y0 + row_w == y0
        || x1 - col_w == x1
        || y1 - row_w == y1
    {
        return Err("film grain Dye: output footprints are below film-coordinate precision; reduce the output dimensions".to_owned());
    }
    // Every output row must fit before any readout/source stencils or output
    // storage are built. Ceil(width/h)+1 bounds cell coverage at any phase.
    let row_cells = (row_w / h).ceil() as u64 + 1 + 2 * d as u64;
    let band_cells = row_cells.saturating_mul(ext_cols as u64);
    let source_cols =
        (src_w as u64).min(((ext_cols as f64 * h / pitch_um).ceil() as u64).saturating_add(1));
    let source_rows =
        (src_h as u64).min(((row_cells as f64 * h / pitch_um).ceil() as u64).saturating_add(1));
    let band_pixels = source_cols * source_rows;
    if band_cells > max_band_cells || band_pixels > max_band_cells {
        return Err(format!(
            "film grain Dye: one output row needs up to {band_cells} field cells and {band_pixels} source pixels, above the {max_band_cells} limit; reduce the requested film region or increase the output size"
        ));
    }
    let output_bytes = (out_width as u64)
        .checked_mul(out_height as u64)
        .and_then(|n| {
            n.checked_mul(3 * std::mem::size_of::<spektrafilm_math::precision::Scalar>() as u64)
        });
    if output_bytes.is_none_or(|bytes| bytes > isize::MAX as u64) {
        return Err("film grain Dye: output storage exceeds the addressable allocation size; reduce the output dimensions".to_owned());
    }
    if out_width as u64 + out_height as u64 > MAX_BAND_CELLS {
        return Err("film grain Dye: output-axis stencil storage exceeds the allocation budget; reduce the output dimensions".to_owned());
    }
    // Readout stencils repeat along the other axis, so build each once.
    let x_read: Vec<Vec<(i64, f64)>> = (0..out_width)
        .map(|px| readout_stencil(x0 + px as f64 * col_w, x0 + (px + 1) as f64 * col_w, h))
        .collect();
    let y_read: Vec<Vec<(i64, f64)>> = (0..out_height)
        .map(|py| readout_stencil(y0 + py as f64 * row_w, y0 + (py + 1) as f64 * row_w, h))
        .collect();
    // Cell-to-source overlap weights depend only on the axis index.
    let x_stencils: Vec<Vec<(usize, f64)>> = (inner_i0 - d..=inner_i1 + d)
        .map(|i| axis_stencil(i as f64 * h, (i + 1) as f64 * h, pitch_um, src_w))
        .collect();
    let edge = |index: i64, extent: usize| index.clamp(0, extent as i64 - 1) as usize;
    // Source pixels the band's cells can read. Edge targets extend constantly
    // outside the film bounds, so this range stays inside the source.
    let sx_lo = edge(((inner_i0 - d) as f64 * h / pitch_um).floor() as i64, src_w);
    let sx_hi = edge(
        ((((inner_i1 + d + 1) as f64 * h) / pitch_um).ceil() as i64).saturating_sub(1),
        src_w,
    );

    let cell_area = h * h;
    let out_w = out_width as usize;
    let mut out = ImageBuf::new(out_width, out_height);
    let mut hbuf = Vec::new();
    let mut fbuf = Vec::new();
    let mut py = 0usize;
    // Coarse output pitches read many cells per row; halve the band until
    // its cells and source pixels fit, and keep that height for later bands.
    let mut band_rows = BAND_OUTPUT_ROWS;
    while py < out_height as usize {
        let (band_end, j_lo, j_hi, ext_j0, ext_j1, sy_lo, sy_hi) = loop {
            let band_end = (py + band_rows).min(out_height as usize);
            let j_lo = y_read[py].first().expect("non-empty stencil").0;
            let j_hi = y_read[band_end - 1].last().expect("non-empty stencil").0;
            let ext_j0 = j_lo - d;
            let ext_j1 = j_hi + d;
            let sy_lo = edge((ext_j0 as f64 * h / pitch_um).floor() as i64, src_h);
            let sy_hi = edge(
                ((((ext_j1 + 1) as f64 * h) / pitch_um).ceil() as i64).saturating_sub(1),
                src_h,
            );
            let band_cells = ((ext_j1 - ext_j0 + 1) as u64).saturating_mul(ext_cols as u64);
            let band_pixels = ((sy_hi - sy_lo + 1) as u64) * ((sx_hi - sx_lo + 1) as u64);
            if band_cells <= max_band_cells && band_pixels <= max_band_cells {
                break (band_end, j_lo, j_hi, ext_j0, ext_j1, sy_lo, sy_hi);
            }
            if band_rows == 1 {
                return Err(format!(
                    "film grain Dye: one output row needs {band_cells} field cells and \
                     {band_pixels} source pixels, above the {max_band_cells} limit; reduce the \
                     requested film region or increase the output size"
                ));
            }
            band_rows /= 2;
        };
        let ext_rows = (ext_j1 - ext_j0 + 1) as usize;
        let rows = (j_hi - j_lo + 1) as usize;
        let y_stencils: Vec<Vec<(usize, f64)>> = (ext_j0..=ext_j1)
            .map(|j| axis_stencil(j as f64 * h, (j + 1) as f64 * h, pitch_um, src_h))
            .collect();
        let px_rows = sy_hi - sy_lo + 1;
        let px_cols = sx_hi - sx_lo + 1;

        // Sublayer targets are allocated on the source grid, once per source
        // pixel, and only then remapped conservatively to field cells.
        let mut planes: [Vec<[f64; 3]>; 3] = [
            vec![[0.0; 3]; px_rows * px_cols],
            vec![[0.0; 3]; px_rows * px_cols],
            vec![[0.0; 3]; px_rows * px_cols],
        ];
        {
            let [p0, p1, p2] = &mut planes;
            p0.par_chunks_mut(px_cols)
                .zip(p1.par_chunks_mut(px_cols))
                .zip(p2.par_chunks_mut(px_cols))
                .enumerate()
                .for_each(|(r, ((row0, row1), row2))| {
                    let row = (sy_lo + r) * src_w;
                    for px in 0..px_cols {
                        let index = (row + sx_lo + px) * 3;
                        let target = [
                            to_f64(target.data[index]),
                            to_f64(target.data[index + 1]),
                            to_f64(target.data[index + 2]),
                        ];
                        let layers = model.layer_targets(target);
                        row0[px] = layers[0];
                        row1[px] = layers[1];
                        row2[px] = layers[2];
                    }
                });
        }

        // Cell densities per channel over the extended band.
        let mut src = [
            vec![0.0f64; ext_rows * ext_cols],
            vec![0.0f64; ext_rows * ext_cols],
            vec![0.0f64; ext_rows * ext_cols],
        ];
        {
            let [s0, s1, s2] = &mut src;
            s0.par_chunks_mut(ext_cols)
                .zip(s1.par_chunks_mut(ext_cols))
                .zip(s2.par_chunks_mut(ext_cols))
                .enumerate()
                .try_for_each(|(r, ((row0, row1), row2))| -> Result<(), String> {
                    let j = ext_j0 + r as i64;
                    let yst = &y_stencils[r];
                    for ci in 0..ext_cols {
                        let i = inner_i0 - d + ci as i64;
                        let cell =
                            remap_layers(&planes, &x_stencils[ci], yst, (sx_lo, sy_lo), px_cols);
                        let add = model.accumulate(i, j, cell, cell_area)?;
                        row0[ci] = add[0];
                        row1[ci] = add[1];
                        row2[ci] = add[2];
                    }
                    Ok(())
                })?;
        }

        hbuf.resize(ext_rows * cols, 0.0);
        fbuf.resize(rows * cols, 0.0);
        let mut formed = response.map(|_| ImageBuf::new(cols as u32, rows as u32));
        for (c, s) in src.iter().enumerate() {
            horizontal_pass(s, &mut hbuf, ext_rows, ext_cols, cols, dr, &model.taps);
            vertical_pass(&hbuf, &mut fbuf, ext_rows, cols, rows, dr, &model.taps);
            let min = model.channels[c].min;
            if let Some(formed) = &mut formed {
                for (pixel, &density) in formed.data.as_chunks_mut::<3>().0.iter_mut().zip(&fbuf) {
                    pixel[c] = from_f64(density - min);
                }
                continue;
            }
            let band = &mut out.data[py * out_w * 3..band_end * out_w * 3];
            band.par_chunks_mut(out_w * 3)
                .enumerate()
                .for_each(|(r, data)| {
                    let yst = &y_read[py + r];
                    for (px, xst) in x_read.iter().enumerate() {
                        let mut acc = 0.0f64;
                        for &(cj, wy) in yst {
                            let fr = (cj - j_lo) as usize * cols;
                            for &(ci, wx) in xst {
                                acc += wy * wx * fbuf[fr + (ci - inner_i0) as usize];
                            }
                        }
                        data[px * 3 + c] = from_f64(acc - min);
                    }
                });
        }
        if let (Some(response), Some(formed)) = (response, formed) {
            let linear = response(&formed)?;
            if linear.width != formed.width
                || linear.height != formed.height
                || linear.data.len() != formed.data.len()
                || linear.data.iter().any(|&value| !to_f64(value).is_finite())
            {
                return Err("film grain Dye: cell response must preserve band dimensions and return finite linear values".to_owned());
            }
            out.data[py * out_w * 3..band_end * out_w * 3]
                .par_chunks_mut(out_w * 3)
                .enumerate()
                .for_each(|(r, data)| {
                    for (px, xst) in x_read.iter().enumerate() {
                        let mut sum = [0.0; 3];
                        for &(cj, wy) in &y_read[py + r] {
                            for &(ci, wx) in xst {
                                let index =
                                    ((cj - j_lo) as usize * cols + (ci - inner_i0) as usize) * 3;
                                for (c, value) in sum.iter_mut().enumerate() {
                                    *value += wy * wx * to_f64(linear.data[index + c]);
                                }
                            }
                        }
                        for c in 0..3 {
                            data[px * 3 + c] = from_f64(sum[c]);
                        }
                    }
                });
        }
        py = band_end;
    }
    Ok(out)
}

/// Named rejection of a device field request for the selected mass law.
///
/// The device realizes the fixed-mass conditional Poisson count law. The
/// experimental random-mark laws of `SPEKTRAFILM_GRAIN_V3_MASS_LAW` are
/// host-only diagnostics, so a device request for one fails by name instead
/// of silently rendering with the production law. Expectation mode and the
/// field-pitch override are part of the production law and stay device-side.
pub fn check_device_mass_law() -> Result<(), String> {
    check_mass_law(MassLaw::from_env()?)
}

fn check_mass_law(law: MassLaw) -> Result<(), String> {
    let name = match law {
        MassLaw::Fixed => return Ok(()),
        MassLaw::TwoClass => "two_class",
        MassLaw::Gamma4 => "gamma4",
    };
    Err(format!(
        "film grain Dye: the experimental {name} mass law (SPEKTRAFILM_GRAIN_V3_MASS_LAW) has \
         no device field path; unset it or render on the CPU backend"
    ))
}

/// Derived model state: taps, per-channel capacity/curve tables.
/// The Dye field model: the channel tables derived from a resolved
/// [`GrainV3Params`], plus the seed and mass law the realization uses.
///
/// [`apply`] owns the whole CPU realization. It is public because the device
/// path has to evaluate the same stages against the same tables instead of
/// re-deriving them, and a second derivation would be a second
/// implementation of the conditioning.
pub struct Model {
    /// One-dimensional cell-integrated formation taps for offsets
    /// `-radius..=radius`.
    taps: Vec<f64>,
    radius: i64,
    cell_um: f64,
    channels: [Channel; 3],
    seed: u64,
    expectation: bool,
    channel_correlation: f64,
    mass_law: MassLaw,
}

/// Per-channel layer state and composite-axis lookup.
struct Channel {
    /// `M_lc = m_lc + f_lc * density_min_c`.
    cap: [f64; 3],
    /// `sum_l M_lc`.
    cap_total: f64,
    /// `f_lc = m_lc / sum_l m_lc`.
    frac: [f64; 3],
    /// `a_lc`, the effective event area in square micrometers.
    area: [f64; 3],
    /// `density_min_c`.
    min: f64,
    /// Uniformity control of this channel.
    uniformity: f64,
    /// Composite-axis knots with the positive-film sign applied.
    knots: Vec<f64>,
    /// Inverse knot spacing (`0` for duplicate knots).
    inv_dx: Vec<f64>,
    /// Raw layer curves at the knots, per layer.
    layers: [Vec<f64>; 3],
}

impl Channel {
    /// Bracket lookup with the composite-to-layer convention of
    /// `spektrafilm_model::density_curves::interp_density_cmy_layers`:
    /// endpoint clamping plus `searchsorted(side='right') - 1`.
    fn bracket(&self, x: f64) -> (usize, f64) {
        let k = self.knots.len();
        if x <= self.knots[0] {
            (0, 0.0)
        } else if x >= self.knots[k - 1] {
            (k - 2, 1.0)
        } else {
            let idx = self.knots.partition_point(|&v| v <= x);
            let low = if idx > 0 { idx - 1 } else { 0 };
            (low, (x - self.knots[low]) * self.inv_dx[low])
        }
    }
}

/// One channel's lookup tables and capacities, as plain data.
///
/// A faithful view of [`Channel`] for consumers that have to evaluate the same
/// stages elsewhere: the device path packs these into its own layout instead
/// of deriving them again, which would be a second implementation of the
/// conditioning. The curves are borrowed — the consumer packs them into its
/// own buffers, so an owned view would clone every knot twice for nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelTable<'a> {
    /// `M_lc = m_lc + f_lc * density_min_c`.
    pub cap: [f64; 3],
    /// `sum_l M_lc`.
    pub cap_total: f64,
    /// `f_lc = m_lc / sum_l m_lc`.
    pub frac: [f64; 3],
    /// `a_lc`, the effective event area in square micrometers.
    pub area: [f64; 3],
    /// `density_min_c`.
    pub min: f64,
    /// Uniformity control of this channel.
    pub uniformity: f64,
    /// Composite-axis knots with the positive-film sign applied.
    pub knots: &'a [f64],
    /// Inverse knot spacing (`0` for duplicate knots).
    pub inv_dx: &'a [f64],
    /// Raw layer curves at the knots, per layer.
    pub layers: [&'a [f64]; 3],
}

impl Model {
    /// The three channel tables, in channel order.
    pub fn channel_tables(&self) -> [ChannelTable<'_>; 3] {
        std::array::from_fn(|c| {
            let channel = &self.channels[c];
            ChannelTable {
                cap: channel.cap,
                cap_total: channel.cap_total,
                frac: channel.frac,
                area: channel.area,
                min: channel.min,
                uniformity: channel.uniformity,
                knots: &channel.knots,
                inv_dx: &channel.inv_dx,
                layers: [&channel.layers[0], &channel.layers[1], &channel.layers[2]],
            }
        })
    }

    /// Field-cell pitch in micrometers the realization uses.
    pub fn cell_um(&self) -> f64 {
        self.cell_um
    }

    /// One-dimensional cell-integrated formation taps for offsets
    /// `-radius..=radius`.
    pub fn taps(&self) -> &[f64] {
        &self.taps
    }

    /// Build the model from resolved parameters and the source pitch.
    pub fn new(params: &GrainV3Params, pitch_um: f64) -> Result<Self, String> {
        if !params.support_um.is_finite() || params.support_um <= 0.0 {
            return Err(format!(
                "film grain Dye: v3_dye_support_um must be finite and > 0 (got {})",
                params.support_um
            ));
        }
        if !pitch_um.is_finite() || pitch_um <= 0.0 {
            return Err(format!(
                "film grain Dye: the film pitch must be finite and > 0 (got {pitch_um})"
            ));
        }
        let production_cell = params.support_um / CELLS_PER_SIGMA;
        let cell_um = params.cell_um.unwrap_or(production_cell);
        if !cell_um.is_finite() || cell_um <= 0.0 || cell_um > production_cell {
            return Err(format!(
                "film grain Dye: the field pitch must be finite and at most {} um (got {cell_um})",
                production_cell
            ));
        }
        let radius_value = (SUPPORT_SIGMAS * (params.support_um / cell_um)).ceil() + 1.0;
        // Even the smallest output footprint needs the full square halo.
        if !radius_value.is_finite() || (2.0 * radius_value + 1.0).powi(2) > MAX_BAND_CELLS as f64 {
            return Err("film grain Dye: support halo exceeds the single-row field-cell budget; increase the field pitch".to_owned());
        }
        let cell_area = cell_um * cell_um;
        if !cell_area.is_finite() || cell_area <= 0.0 {
            return Err("film grain Dye: squared field pitch must be finite and > 0; reduce the dye support or increase the field pitch".to_owned());
        }
        let mut channels = Vec::with_capacity(3);
        for c in 0..3 {
            channels.push(Channel::new(
                params,
                c,
                params.layer_max.iter().map(|row| row[c]).collect(),
                params.composite_curves.iter().map(|row| row[c]).collect(),
                std::array::from_fn(|l| params.layer_curves.iter().map(|row| row[l][c]).collect()),
            )?);
        }
        let channel_correlation = params.channel_correlation;
        if !channel_correlation.is_finite() || !(0.0..1.0).contains(&channel_correlation) {
            return Err(
                "film grain Dye: channel correlation must be finite and in [0, 1)".to_owned(),
            );
        }
        let mass_law = MassLaw::from_env()?;
        for (c, channel) in channels.iter().enumerate() {
            channel.check_sampling_bounds(c, cell_area)?;
            channel.check_sampling_bounds(c, cell_area * mass_law.rate_multiplier())?;
        }
        Ok(Self {
            taps: support_taps(params.support_um, cell_um),
            radius: radius_value as i64,
            cell_um,
            channels: channels
                .try_into()
                .map_err(|_| "film grain Dye: channel state".to_owned())?,
            seed: params.seed,
            expectation: params.expectation,
            channel_correlation,
            mass_law,
        })
    }

    /// Absolute sublayer targets `t_lc` of one source pixel.
    ///
    /// Implements `design.md#sublayer-target-allocation`: bracket lookup on
    /// the normalized composite axis, weights `v_lc + b_lc`, then iterative
    /// weighted capacity allocation of `T_c = D_c + density_min_c`.
    /// Realized absolute cell density per channel for one cell target.
    pub fn layer_targets(&self, target: [f64; 3]) -> [[f64; 3]; 3] {
        let mut out = [[0.0f64; 3]; 3];
        for (c, ch) in self.channels.iter().enumerate() {
            let total = (target[c] + ch.min).clamp(0.0, ch.cap_total);
            let (low, t) = ch.bracket(target[c]);
            let mut weights = [0.0f64; 3];
            for (l, weight) in weights.iter_mut().enumerate() {
                if ch.cap[l] <= 0.0 {
                    continue;
                }
                let value = ch.layers[l][low] + t * (ch.layers[l][low + 1] - ch.layers[l][low]);
                *weight = (value + ch.frac[l] * ch.min).max(0.0);
            }
            out[c] = allocate(total, &ch.cap, &weights);
        }
        out
    }

    /// Realized (or expected) absolute cell density per channel.
    ///
    /// `i`/`j` are global signed cell coordinates: they alone select the
    /// random stream, so a crop or a tile boundary cannot change a cell's
    /// counts.
    ///
    /// Each layer splits its per-channel rate `lambda_c` into a shared part
    /// `corr * min_c(lambda_c)` and an independent remainder
    /// `lambda_c - shared`. The shared part is drawn once per layer, so the
    /// channels of one layer correlate while every channel keeps its marginal
    /// count law (the split is a thinning, so it holds for the default law and
    /// for the scale-mixture laws alike). `min` runs over the layer's active channels only:
    /// an inactive channel has no rate to correlate with, and counting it as
    /// zero would silently collapse the layer's correlation to zero.
    ///
    /// Layers are the outer loop here because the shared draw needs all three
    /// channels of a layer. A channel's contributions are still added in
    /// ascending layer order into one accumulator, which is what keeps
    /// `corr = 0` bit-identical to the previous per-channel loop.
    // The layer index is a layer *number*: it selects the shared stream tag and
    // indexes three per-channel arrays, not one collection.
    #[allow(clippy::needless_range_loop)]
    fn accumulate(
        &self,
        i: i64,
        j: i64,
        layers: [[f64; 3]; 3],
        cell_area: f64,
    ) -> Result<[f64; 3], String> {
        let mut out = [0.0f64; 3];
        let index = cell_stream_index(i, j);
        for l in 0..3 {
            // Per-channel rate and mass of this layer; `active[c]` marks the
            // channels whose `(channel, layer)` contributes a count at all.
            let mut rate = [0.0f64; 3];
            let mut mass = [0.0f64; 3];
            let mut active = [false; 3];
            for (c, ch) in self.channels.iter().enumerate() {
                let target = layers[c][l];
                if target <= 0.0 || ch.cap[l] <= 0.0 {
                    continue;
                }
                let p = (target / ch.cap[l]).clamp(0.0, 1.0);
                let s = 1.0 - ch.uniformity * (1.0 - 1e-6) * p;
                mass[c] = ch.cap[l] * ch.area[l] * s / ch.frac[l];
                rate[c] = cell_area * ch.frac[l] * p / (ch.area[l] * s);
                active[c] = true;
            }

            // One shared rate serves the whole layer, so it must be one the
            // smallest rate can fund without going negative.
            let mut shared = 0.0f64;
            let mut active_channels = 0usize;
            let mut smallest = f64::INFINITY;
            for c in 0..3 {
                if active[c] && rate[c] > 0.0 {
                    active_channels += 1;
                    smallest = smallest.min(rate[c]);
                }
            }
            if active_channels >= 2 {
                shared = self.channel_correlation * smallest;
            }

            let shared_count = if self.expectation || shared <= 0.0 {
                0.0
            } else {
                let mut rng = FastStatsRng::stream(self.seed, SHARED_TAG_BASE + l as u64, index);
                self.mass_law.sample(&mut rng, shared)?
            };

            for (c, _) in self.channels.iter().enumerate() {
                if !active[c] {
                    continue;
                }
                let count = if self.expectation {
                    // The expectation is the marginal mean and must not move
                    // with the correlation, so it is not split at all.
                    rate[c]
                } else {
                    let independent = rate[c] - shared;
                    let own = if independent > 0.0 {
                        let mut rng = FastStatsRng::stream(self.seed, (c * 3 + l) as u64, index);
                        self.mass_law.sample(&mut rng, independent)?
                    } else {
                        0.0
                    };
                    shared_count + own
                };
                out[c] += mass[c] * count / cell_area;
                if !out[c].is_finite() {
                    return Err(
                        "film grain Dye: sampled dye mass exceeds finite density range".to_owned(),
                    );
                }
            }
        }
        Ok(out)
    }
}

impl Channel {
    #[allow(clippy::too_many_arguments)]
    fn new(
        params: &GrainV3Params,
        c: usize,
        maxima: Vec<f64>,
        composite: Vec<f64>,
        layers: [Vec<f64>; 3],
    ) -> Result<Self, String> {
        if composite.len() < 2 || composite.len() != params.layer_curves.len() {
            return Err(format!(
                "film grain Dye: channel {c} needs at least two composite knots shared with the \
                 layer curves (got {} knots and {} layer rows)",
                composite.len(),
                params.layer_curves.len()
            ));
        }
        if layers.iter().any(|column| column.len() != composite.len()) {
            return Err(format!(
                "film grain Dye: channel {c} layer curves must share the composite knot count"
            ));
        }
        let sign = if params.positive_film { -1.0 } else { 1.0 };
        let knots: Vec<f64> = composite.iter().map(|&v| v * sign).collect();
        if knots.iter().any(|v| !v.is_finite()) {
            return Err(format!(
                "film grain Dye: channel {c} composite curve contains a non-finite value"
            ));
        }
        let range = knots[knots.len() - 1] - knots[0];
        if !range.is_finite() || range <= 0.0 {
            return Err(format!(
                "film grain Dye: channel {c} composite density axis must have positive range \
                 (got {range})"
            ));
        }
        for window in knots.windows(2) {
            if window[1] < window[0] {
                return Err(format!(
                    "film grain Dye: channel {c} composite density axis must be nondecreasing"
                ));
            }
        }
        for (l, column) in layers.iter().enumerate() {
            if column.iter().any(|v| !v.is_finite() || *v < 0.0) {
                return Err(format!(
                    "film grain Dye: channel {c} layer {l} curve must be finite and nonnegative"
                ));
            }
        }
        let min = params.density_min[c];
        if !min.is_finite() || min < 0.0 {
            return Err(format!(
                "film grain Dye: density_min[{c}] must be finite and nonnegative (got {min})"
            ));
        }
        let uniformity = params.uniformity[c];
        if !uniformity.is_finite() || !(0.0..=1.0).contains(&uniformity) {
            return Err(format!(
                "film grain Dye: uniformity[{c}] must be finite in [0, 1] (got {uniformity})"
            ));
        }
        let mut max_total = 0.0f64;
        for &value in &maxima {
            if !value.is_finite() || value < 0.0 {
                return Err(format!(
                    "film grain Dye: channel {c} layer maxima must be finite and nonnegative"
                ));
            }
            max_total += value;
        }
        if !max_total.is_finite() || max_total <= 0.0 {
            return Err(format!(
                "film grain Dye: channel {c} has no positive layer capacity; the resolved profile \
                 must provide nonnegative sublayer curves with a positive sum"
            ));
        }
        let mut frac = [0.0f64; 3];
        let mut capacity = [0.0f64; 3];
        let mut area = [0.0f64; 3];
        for l in 0..3 {
            frac[l] = maxima[l] / max_total;
            capacity[l] = maxima[l] + frac[l] * min;
            let scale = params.channel_scale[c] * params.particle_scale_sublayers[l];
            let effective_area = params.event_area_um2 * scale;
            if !params.event_area_um2.is_finite()
                || params.event_area_um2 <= 0.0
                || !scale.is_finite()
                || scale <= 0.0
                || !effective_area.is_finite()
                || effective_area <= 0.0
            {
                return Err(format!(
                    "film grain Dye: event_area_um2 * channel_scale[{c}] * \
                     particle_scale_sublayers[{l}] must be finite and > 0 (got {} * {scale})",
                    params.event_area_um2
                ));
            }
            area[l] = effective_area;
        }
        let inv_dx = (0..knots.len() - 1)
            .map(|i| {
                let dx = knots[i + 1] - knots[i];
                if dx > 0.0 { 1.0 / dx } else { 0.0 }
            })
            .collect();
        Ok(Self {
            cap: capacity,
            cap_total: capacity.iter().sum(),
            frac,
            area,
            min,
            uniformity,
            knots,
            inv_dx,
            layers,
        })
    }
    /// Bound every occupancy before rendering. PTRS represents counts in
    /// f64, so rates above half its consecutive-integer range are unsupported.
    fn check_sampling_bounds(&self, c: usize, cell_area: f64) -> Result<(), String> {
        if !self.cap_total.is_finite() || self.cap_total <= 0.0 {
            return Err(format!(
                "film grain Dye: channel {c} total capacity must be finite and positive"
            ));
        }
        let s_min = 1.0 - self.uniformity * (1.0 - 1e-6);
        for l in 0..3 {
            if self.cap[l] == 0.0 {
                continue;
            }
            let numerator = cell_area * self.frac[l];
            let denominator = self.area[l] * s_min;
            let max_rate = numerator / denominator;
            let mass_product = self.cap[l] * self.area[l];
            let max_mass = mass_product / self.frac[l];
            let max_density = max_mass * ((1u64 << 53) as f64) / cell_area;
            if !self.cap[l].is_finite()
                || self.frac[l] <= 0.0
                || !numerator.is_finite()
                || numerator <= 0.0
                || !denominator.is_finite()
                || denominator <= 0.0
                || !max_rate.is_finite()
                || max_rate <= 0.0
                || max_rate > (1u64 << 52) as f64
                || !mass_product.is_finite()
                || mass_product <= 0.0
                || !max_mass.is_finite()
                || mass_product * s_min <= 0.0
                || !max_density.is_finite()
                || max_density > spektrafilm_math::precision::Scalar::MAX as f64 / 3.0
            {
                return Err(format!(
                    "film grain Dye: channel {c} layer {l} event area, capacity and field pitch produce an unrepresentable rate or dye mass (maximum rate {max_rate}, mass {max_mass}); adjust event area or field pitch; maximum supported rate is {}",
                    1u64 << 52
                ));
            }
        }
        Ok(())
    }
}

/// Fail before rendering when the developed target leaves the capacity
/// envelope `[-density_min, sum_l M_lc]`.
fn check_target_range(target: &ImageBuf, model: &Model) -> Result<(), String> {
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for px in target.pixels() {
        for c in 0..3 {
            let value = to_f64(px[c]);
            if value < low[c] {
                low[c] = value;
            }
            if value > high[c] {
                high[c] = value;
            }
        }
    }
    const TOLERANCE: f64 = 1e-6;
    for c in 0..3 {
        let floor = -model.channels[c].min - TOLERANCE;
        let ceiling = model.channels[c].cap_total - model.channels[c].min + TOLERANCE;
        if low[c] < floor || high[c] > ceiling {
            return Err(format!(
                "film grain Dye: channel {c} developed density spans [{:.6}, {:.6}], outside the \
                 supported [{:.6}, {:.6}] set by density_min {:.6} and the resolved layer \
                 capacities {:?}",
                low[c], high[c], floor, ceiling, model.channels[c].min, model.channels[c].cap
            ));
        }
    }
    Ok(())
}

/// Conservative overlap remap of allocated sublayer targets to one cell.
///
/// Weights are the exact area overlaps of the cell with the piecewise
/// constant source grid and sum to one, including the constant edge
/// extension outside the film bounds, so constants and integrated target
/// mass survive the transfer.
fn remap_layers(
    planes: &[Vec<[f64; 3]>; 3],
    xs: &[(usize, f64)],
    ys: &[(usize, f64)],
    origin: (usize, usize),
    row_len: usize,
) -> [[f64; 3]; 3] {
    let (x_origin, y_origin) = origin;
    let mut out = [[0.0f64; 3]; 3];
    for (channel, plane) in planes.iter().enumerate() {
        let mut layer = [0.0f64; 3];
        for &(py, wy) in ys {
            debug_assert!(py >= y_origin);
            let row = (py - y_origin) * row_len;
            for &(px, wx) in xs {
                debug_assert!(px >= x_origin);
                let weight = wy * wx;
                let value = plane[row + px - x_origin];
                for l in 0..3 {
                    layer[l] += weight * value[l];
                }
            }
        }
        out[channel] = layer;
    }
    out
}

/// Overlap weights of one cell interval `[lo, hi)` with the source pixel
/// grid, normalized to sum to one. Values outside the film bounds keep the
/// nearest edge source pixel.
fn axis_stencil(lo: f64, hi: f64, pitch: f64, extent: usize) -> Vec<(usize, f64)> {
    debug_assert!(extent > 0);
    let width = hi - lo;
    let end = extent as f64 * pitch;
    let mut out = Vec::with_capacity(3);
    if lo < 0.0 {
        out.push((0, (hi.min(0.0) - lo) / width));
    }
    let a = lo.max(0.0);
    let b = hi.min(end);
    if b > a {
        let first = ((a / pitch).floor() as usize).min(extent - 1);
        let past = ((b / pitch).ceil() as usize).clamp(1, extent);
        for p in first..past {
            let overlap = b.min((p + 1) as f64 * pitch) - a.max(p as f64 * pitch);
            if overlap > 0.0 {
                out.push((p, overlap / width));
            }
        }
    }
    if hi > end {
        out.push((extent - 1, (hi - end.max(lo)) / width));
    }
    out
}

/// Cells and normalized weights of one output footprint `[lo, hi)`.
fn readout_stencil(lo: f64, hi: f64, cell: f64) -> Vec<(i64, f64)> {
    let width = hi - lo;
    let first = (lo / cell).floor() as i64;
    let last = (hi / cell).ceil() as i64 - 1;
    let mut out = Vec::with_capacity((last - first + 1) as usize);
    for i in first..=last {
        let overlap = hi.min((i + 1) as f64 * cell) - lo.max(i as f64 * cell);
        if overlap > 0.0 {
            out.push((i, overlap / width));
        }
    }
    out
}

/// Horizontal formation pass: cell densities → formed cell averages.
fn horizontal_pass(
    src: &[f64],
    dst: &mut [f64],
    ext_rows: usize,
    ext_cols: usize,
    cols: usize,
    d: usize,
    taps: &[f64],
) {
    debug_assert_eq!(src.len(), ext_rows * ext_cols);
    dst.par_chunks_mut(cols).enumerate().for_each(|(r, row)| {
        let cells = &src[r * ext_cols..(r + 1) * ext_cols];
        for (ci, value) in row.iter_mut().enumerate() {
            let mut acc = 0.0f64;
            for (t, &weight) in taps.iter().enumerate() {
                acc += weight * cells[ci + 2 * d - t];
            }
            *value = acc;
        }
    });
}

/// Vertical formation pass over already horizontally formed rows.
fn vertical_pass(
    src: &[f64],
    dst: &mut [f64],
    ext_rows: usize,
    cols: usize,
    rows: usize,
    d: usize,
    taps: &[f64],
) {
    debug_assert_eq!(src.len(), ext_rows * cols);
    debug_assert_eq!(dst.len(), rows * cols);
    dst.par_chunks_mut(cols).enumerate().for_each(|(k, row)| {
        for (ci, value) in row.iter_mut().enumerate() {
            let mut acc = 0.0f64;
            for (t, &weight) in taps.iter().enumerate() {
                acc += weight * src[(k + 2 * d - t) * cols + ci];
            }
            *value = acc;
        }
    });
}

/// Cell-integrated taps of the truncated Gaussian, normalized to sum one.
///
/// Tap `d` is `integral_Cj integral_Si g(z - x) dx dz / h` for cell offset
/// `d`, so filtering cell densities with these taps yields formed cell
/// averages.
pub fn support_taps(sigma_um: f64, cell_um: f64) -> Vec<f64> {
    let radius = (SUPPORT_SIGMAS * sigma_um / cell_um).ceil() as i64 + 1;
    let mut taps: Vec<f64> = (-radius..=radius)
        .map(|offset| tap(offset as f64 * cell_um, cell_um, sigma_um))
        .collect();
    for value in taps.iter_mut() {
        if *value < 0.0 {
            *value = 0.0;
        }
    }
    let sum: f64 = taps.iter().sum();
    for value in taps.iter_mut() {
        *value /= sum;
    }
    taps
}

/// `k_d = (R(dh + h) - 2 R(dh) + R(dh - h)) / h` for the double
/// antiderivative `R` of the truncated normalized Gaussian.
fn tap(delta: f64, cell_um: f64, sigma_um: f64) -> f64 {
    (double_cdf(delta + cell_um, sigma_um) - 2.0 * double_cdf(delta, sigma_um)
        + double_cdf(delta - cell_um, sigma_um))
        / cell_um
}

/// Double antiderivative of the Gaussian truncated to `[-4 sigma, 4 sigma]`
/// and normalized to unit integral, anchored at `R(-4 sigma) = 0`.
///
/// `R'` is the truncated CDF and `R''` the normalized Gaussian.
fn double_cdf(y: f64, sigma: f64) -> f64 {
    let cdf = |x: f64| 0.5 * (1.0 + libm::erf(x / std::f64::consts::SQRT_2));
    let density = |x: f64| (-0.5 * x * x).exp() / (2.0 * std::f64::consts::PI).sqrt();
    let low = -SUPPORT_SIGMAS * sigma;
    let high = SUPPORT_SIGMAS * sigma;
    if y <= low {
        return 0.0;
    }
    // Double antiderivative `integral_a^y (y - x) g(x) dx` of the full
    // normalized Gaussian, with `y * cdf(y/sigma) + sigma * density(y/sigma)`
    // as its primitive.
    let raw = |x: f64| x * cdf(x / sigma) + sigma * density(x / sigma);
    let area = cdf(SUPPORT_SIGMAS) - cdf(-SUPPORT_SIGMAS);
    let base = raw(low);
    let lower_mass = cdf(-SUPPORT_SIGMAS);
    let inside = |x: f64| (raw(x) - base - lower_mass * (x - low)) / area;
    if y >= high {
        return inside(high) + (y - high);
    }
    inside(y)
}

/// Iterative weighted capacity allocation (`design.md#sublayer-target-allocation`).
///
/// Distributes `target` over the layer capacities: unsaturated layers share
/// the remaining target in proportion to their weights, a layer whose offer
/// reaches capacity is fixed there, and the rest repeats. If every remaining
/// weight is zero, the remaining capacity fractions replace them.
fn allocate(target: f64, capacity: &[f64; 3], weights: &[f64; 3]) -> [f64; 3] {
    let total: f64 = capacity.iter().sum();
    let mut out = [0.0f64; 3];
    if target <= 0.0 {
        return out;
    }
    if target >= total {
        return *capacity;
    }
    let mut active = [capacity[0] > 0.0, capacity[1] > 0.0, capacity[2] > 0.0];
    let mut remaining = target;
    while remaining > 0.0 && active.iter().any(|&a| a) {
        let mut share = [0.0f64; 3];
        let mut sum = 0.0f64;
        for l in 0..3 {
            if active[l] {
                share[l] = weights[l].max(0.0);
                sum += share[l];
            }
        }
        if sum <= 0.0 {
            let fixed: f64 = (0..3).filter(|&l| active[l]).map(|l| capacity[l]).sum();
            if fixed <= 0.0 {
                break;
            }
            for l in 0..3 {
                share[l] = if active[l] { capacity[l] / fixed } else { 0.0 };
            }
            sum = 1.0;
        }
        let mut saturated = false;
        for l in 0..3 {
            if !active[l] {
                continue;
            }
            let offer = remaining * share[l] / sum;
            if offer >= capacity[l] {
                out[l] = capacity[l];
                active[l] = false;
                saturated = true;
            } else {
                out[l] = offer;
            }
        }
        if !saturated {
            break;
        }
        // Drop the provisional offers of the still-active layers before the
        // next iteration computes the target left after the fixed ones.
        for l in 0..3 {
            if active[l] {
                out[l] = 0.0;
            }
        }
        remaining = target - out.iter().sum::<f64>();
    }
    out
}

#[cfg(test)]
mod tests;
