//! Film-coordinate conditional dye-mass grain (V3).
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
use spektrafilm_math::stats::FastStatsRng;

/// Production field pitch as a fraction of the support sigma (`h = sigma/2`).
const CELLS_PER_SIGMA: f64 = 2.0;
/// Gaussian truncation half-width in sigmas.
const SUPPORT_SIGMAS: f64 = 4.0;
/// Output rows per readout band; bounds the field buffers of one band.
const BAND_OUTPUT_ROWS: usize = 128;
/// Largest cell count a single band may hold. The field is not silently
/// coarsened; an oversized request fails with an actionable error.
const MAX_BAND_CELLS: u64 = 1 << 24;

/// Source-pixel rectangle of the requested film region.
///
/// The rectangle selects the readout region only: field cells keep their
/// global film coordinates, so a crop never restarts texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// Resolved V3 inputs. Plain data — no profile or runtime types.
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
    pub particle_area_um2: f64,
    /// Per-channel event-area scale.
    pub particle_scale: [f64; 3],
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

/// Realize the conditional dye field over `rect` and read it at
/// `out_width x out_height`.
///
/// `target` is the full-source developed CMY density and `pitch_um` its pixel
/// pitch on the full film. The returned image holds the readout density with
/// `density_min` subtracted once.
pub fn apply(
    target: &ImageBuf,
    pitch_um: f64,
    rect: SourceRect,
    out_width: u32,
    out_height: u32,
    params: &GrainV3Params,
) -> Result<ImageBuf, String> {
    let model = Model::new(params, pitch_um)?;
    let src_w = target.width as usize;
    let src_h = target.height as usize;
    if rect.width == 0 || rect.height == 0 {
        return Err("film grain V3: the requested film rectangle is empty".to_owned());
    }
    if rect.x + rect.width > src_w || rect.y + rect.height > src_h {
        return Err(format!(
            "film grain V3: film rectangle {}x{}+({},{}) exceeds the {}x{} source",
            rect.width, rect.height, rect.x, rect.y, src_w, src_h
        ));
    }
    if out_width == 0 || out_height == 0 {
        return Err("film grain V3: output dimensions must be positive".to_owned());
    }
    check_target_range(target, &model)?;

    let h = model.cell_um;
    // Requested region in film coordinates.
    let x0 = rect.x as f64 * pitch_um;
    let x1 = (rect.x + rect.width) as f64 * pitch_um;
    let y0 = rect.y as f64 * pitch_um;
    let y1 = (rect.y + rect.height) as f64 * pitch_um;

    let d = model.radius;
    let dr = d as usize;
    // Cells the readout footprints can touch, in global cell indices.
    let inner_i0 = (x0 / h).floor() as i64;
    let inner_i1 = (x1 / h).ceil() as i64 - 1;
    let cols = (inner_i1 - inner_i0 + 1) as usize;
    let ext_cols = cols + 2 * d as usize;
    let col_w = (x1 - x0) / out_width as f64;
    let row_w = (y1 - y0) / out_height as f64;

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
        (((inner_i1 + d + 1) as f64 * h) / pitch_um).ceil() as i64 - 1,
        src_w,
    );

    let cell_area = h * h;
    let out_w = out_width as usize;
    let mut out = ImageBuf::new(out_width, out_height);
    let mut hbuf = Vec::new();
    let mut fbuf = Vec::new();
    let mut py = 0usize;
    while py < out_height as usize {
        let band_end = (py + BAND_OUTPUT_ROWS).min(out_height as usize);
        let j_lo = y_read[py].first().expect("non-empty stencil").0;
        let j_hi = y_read[band_end - 1].last().expect("non-empty stencil").0;
        let ext_j0 = j_lo - d;
        let ext_j1 = j_hi + d;
        let ext_rows = (ext_j1 - ext_j0 + 1) as usize;
        let rows = (j_hi - j_lo + 1) as usize;
        let y_stencils: Vec<Vec<(usize, f64)>> = (ext_j0..=ext_j1)
            .map(|j| axis_stencil(j as f64 * h, (j + 1) as f64 * h, pitch_um, src_h))
            .collect();
        let sy_lo = edge((ext_j0 as f64 * h / pitch_um).floor() as i64, src_h);
        let sy_hi = edge(
            (((ext_j1 + 1) as f64 * h) / pitch_um).ceil() as i64 - 1,
            src_h,
        );
        let px_rows = sy_hi - sy_lo + 1;
        let px_cols = sx_hi - sx_lo + 1;
        let band_cells = (ext_rows as u64) * (ext_cols as u64);
        let band_pixels = (px_rows as u64) * (px_cols as u64);
        if band_cells > MAX_BAND_CELLS || band_pixels > MAX_BAND_CELLS {
            return Err(format!(
                "film grain V3: one field band needs {band_cells} cells and {band_pixels} source \
                 pixels, above the {MAX_BAND_CELLS} limit; reduce the requested film region or \
                 output size"
            ));
        }

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
                .for_each(|(r, ((row0, row1), row2))| {
                    let j = ext_j0 + r as i64;
                    let yst = &y_stencils[r];
                    for ci in 0..ext_cols {
                        let i = inner_i0 - d + ci as i64;
                        let cell =
                            remap_layers(&planes, &x_stencils[ci], yst, (sx_lo, sy_lo), px_cols);
                        let add = model.accumulate(i, j, cell, cell_area);
                        row0[ci] = add[0];
                        row1[ci] = add[1];
                        row2[ci] = add[2];
                    }
                });
        }

        hbuf.resize(ext_rows * cols, 0.0);
        fbuf.resize(rows * cols, 0.0);
        for (c, s) in src.iter().enumerate() {
            horizontal_pass(s, &mut hbuf, ext_rows, ext_cols, cols, dr, &model.taps);
            vertical_pass(&hbuf, &mut fbuf, ext_rows, cols, rows, dr, &model.taps);
            let min = model.channels[c].min;
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
        py = band_end;
    }
    Ok(out)
}

/// Derived model state: taps, per-channel capacity/curve tables.
struct Model {
    /// One-dimensional cell-integrated formation taps for offsets
    /// `-radius..=radius`.
    taps: Vec<f64>,
    radius: i64,
    cell_um: f64,
    channels: [Channel; 3],
    seed: u64,
    expectation: bool,
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

impl Model {
    fn new(params: &GrainV3Params, pitch_um: f64) -> Result<Self, String> {
        if !params.support_um.is_finite() || params.support_um <= 0.0 {
            return Err(format!(
                "film grain V3: v3_dye_support_um must be finite and > 0 (got {})",
                params.support_um
            ));
        }
        if !pitch_um.is_finite() || pitch_um <= 0.0 {
            return Err(format!(
                "film grain V3: the film pitch must be finite and > 0 (got {pitch_um})"
            ));
        }
        let production_cell = params.support_um / CELLS_PER_SIGMA;
        let cell_um = params.cell_um.unwrap_or(production_cell);
        if !cell_um.is_finite() || cell_um <= 0.0 || cell_um > production_cell {
            return Err(format!(
                "film grain V3: the field pitch must be finite and at most {} um (got {cell_um})",
                production_cell
            ));
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
        Ok(Self {
            taps: support_taps(params.support_um, cell_um),
            radius: ((SUPPORT_SIGMAS * params.support_um / cell_um).ceil() as i64) + 1,
            cell_um,
            channels: channels
                .try_into()
                .map_err(|_| "film grain V3: channel state".to_owned())?,
            seed: params.seed,
            expectation: params.expectation,
        })
    }

    /// Absolute sublayer targets `t_lc` of one source pixel.
    ///
    /// Implements `design.md#sublayer-target-allocation`: bracket lookup on
    /// the normalized composite axis, weights `v_lc + b_lc`, then iterative
    /// weighted capacity allocation of `T_c = D_c + density_min_c`.
    fn layer_targets(&self, target: [f64; 3]) -> [[f64; 3]; 3] {
        let mut out = [[0.0f64; 3]; 3];
        for (c, ch) in self.channels.iter().enumerate() {
            let total = (target[c] + ch.min).clamp(0.0, ch.cap_total);
            let (low, t) = ch.bracket(target[c]);
            let mut weights = [0.0f64; 3];
            for l in 0..3 {
                if ch.cap[l] <= 0.0 {
                    continue;
                }
                let value = ch.layers[l][low] + t * (ch.layers[l][low + 1] - ch.layers[l][low]);
                weights[l] = (value + ch.frac[l] * ch.min).max(0.0);
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
    fn accumulate(&self, i: i64, j: i64, layers: [[f64; 3]; 3], cell_area: f64) -> [f64; 3] {
        let mut out = [0.0f64; 3];
        let index = cell_stream_index(i, j);
        for (c, ch) in self.channels.iter().enumerate() {
            let mut density = 0.0f64;
            for l in 0..3 {
                let target = layers[c][l];
                if target <= 0.0 || ch.cap[l] <= 0.0 {
                    continue;
                }
                let p = (target / ch.cap[l]).clamp(0.0, 1.0);
                let s = 1.0 - ch.uniformity * (1.0 - 1e-6) * p;
                let mass = ch.cap[l] * ch.area[l] * s / ch.frac[l];
                let lambda = cell_area * ch.frac[l] * p / (ch.area[l] * s);
                let count = if self.expectation {
                    lambda
                } else {
                    let mut rng = FastStatsRng::stream(self.seed, (c * 3 + l) as u64, index);
                    poisson_exact(&mut rng, lambda) as f64
                };
                density += mass * count / cell_area;
            }
            out[c] = density;
        }
        out
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
                "film grain V3: channel {c} needs at least two composite knots shared with the \
                 layer curves (got {} knots and {} layer rows)",
                composite.len(),
                params.layer_curves.len()
            ));
        }
        if layers.iter().any(|column| column.len() != composite.len()) {
            return Err(format!(
                "film grain V3: channel {c} layer curves must share the composite knot count"
            ));
        }
        let sign = if params.positive_film { -1.0 } else { 1.0 };
        let knots: Vec<f64> = composite.iter().map(|&v| v * sign).collect();
        if knots.iter().any(|v| !v.is_finite()) {
            return Err(format!(
                "film grain V3: channel {c} composite curve contains a non-finite value"
            ));
        }
        let range = knots[knots.len() - 1] - knots[0];
        if range <= 0.0 {
            return Err(format!(
                "film grain V3: channel {c} composite density axis must have positive range \
                 (got {range})"
            ));
        }
        for window in knots.windows(2) {
            if window[1] < window[0] {
                return Err(format!(
                    "film grain V3: channel {c} composite density axis must be nondecreasing"
                ));
            }
        }
        for (l, column) in layers.iter().enumerate() {
            if column.iter().any(|v| !v.is_finite() || *v < 0.0) {
                return Err(format!(
                    "film grain V3: channel {c} layer {l} curve must be finite and nonnegative"
                ));
            }
        }
        let min = params.density_min[c];
        if !min.is_finite() || min < 0.0 {
            return Err(format!(
                "film grain V3: density_min[{c}] must be finite and nonnegative (got {min})"
            ));
        }
        let uniformity = params.uniformity[c];
        if !uniformity.is_finite() || !(0.0..=1.0).contains(&uniformity) {
            return Err(format!(
                "film grain V3: uniformity[{c}] must be finite in [0, 1] (got {uniformity})"
            ));
        }
        let mut max_total = 0.0f64;
        for &value in &maxima {
            if !value.is_finite() || value < 0.0 {
                return Err(format!(
                    "film grain V3: channel {c} layer maxima must be finite and nonnegative"
                ));
            }
            max_total += value;
        }
        if max_total <= 0.0 {
            return Err(format!(
                "film grain V3: channel {c} has no positive layer capacity; the resolved profile \
                 must provide nonnegative sublayer curves with a positive sum"
            ));
        }
        let mut frac = [0.0f64; 3];
        let mut capacity = [0.0f64; 3];
        let mut area = [0.0f64; 3];
        for l in 0..3 {
            frac[l] = maxima[l] / max_total;
            capacity[l] = maxima[l] + frac[l] * min;
            let scale = params.particle_scale[c] * params.particle_scale_sublayers[l];
            if !params.particle_area_um2.is_finite()
                || params.particle_area_um2 <= 0.0
                || !scale.is_finite()
                || scale <= 0.0
            {
                return Err(format!(
                    "film grain V3: particle_area_um2 * particle_scale[{c}] * \
                     particle_scale_sublayers[{l}] must be finite and > 0 (got {} * {scale})",
                    params.particle_area_um2
                ));
            }
            area[l] = params.particle_area_um2 * scale;
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
                "film grain V3: channel {c} developed density spans [{:.6}, {:.6}], outside the \
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

/// Exact Poisson variate: Knuth's multiplication below 30, Hörmann's PTRS
/// above. Rounded normal approximations must not replace it.
fn poisson_exact(rng: &mut FastStatsRng, lambda: f64) -> u64 {
    if lambda <= 0.0 {
        return 0;
    }
    if lambda < 30.0 {
        let limit = (-lambda).exp();
        let mut product = 1.0f64;
        let mut count = 0u64;
        loop {
            product *= rng.rand();
            if product <= limit {
                return count;
            }
            count += 1;
        }
    }
    poisson_ptrs(rng, lambda)
}

/// Hörmann 1993 transformed rejection with squeeze (PTRS), the same
/// algorithm numpy uses for large rates.
fn poisson_ptrs(rng: &mut FastStatsRng, lambda: f64) -> u64 {
    let slam = lambda.sqrt();
    let loglam = lambda.ln();
    let b = 0.931 + 2.53 * slam;
    let a = -0.059 + 0.02483 * b;
    let inv_alpha = 1.1239 + 1.1328 / (b - 3.4);
    let vr = 0.9277 - 3.6224 / (b - 2.0);
    loop {
        let u = rng.rand() - 0.5;
        let v = rng.rand();
        let us = 0.5 - u.abs();
        let k = ((2.0 * a / us + b) * u + lambda + 0.43).floor();
        if us >= 0.07 && v <= vr {
            return k as u64;
        }
        if k < 0.0 || (us < 0.013 && v > us) {
            continue;
        }
        let lhs = v.ln() + inv_alpha.ln() - (a / (us * us) + b).ln();
        let rhs = -lambda + k * loglam - libm::lgamma(k + 1.0);
        if lhs <= rhs {
            return k as u64;
        }
    }
}

/// Global-cell address of one `(channel, layer)` stream. Crop bounds, tile
/// size, traversal order and thread scheduling must not enter the address.
fn cell_stream_index(i: i64, j: i64) -> u64 {
    let x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let y = (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    (x ^ y).rotate_left(31)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Source pixel pitch used by these tests, in micrometers.
    const PITCH: f64 = 6.0;
    /// Candidate default dye support.
    const SIGMA: f64 = 8.0;
    /// Raw layer maxima `[layer]`, equal across channels.
    const MAXIMA: [f64; 3] = [0.18, 0.09, 0.03];
    /// Base density offset, equal across channels.
    const MIN_DENSITY: f64 = 0.03;

    fn curves() -> (Vec<[f64; 3]>, Vec<[[f64; 3]; 3]>) {
        let knots = 17usize;
        let t = |k: usize| k as f64 / (knots - 1) as f64;
        let composite = (0..knots).map(|k| [t(k); 3]).collect();
        let layers = (0..knots)
            .map(|k| std::array::from_fn(|l| [MAXIMA[l] * t(k); 3]))
            .collect();
        (composite, layers)
    }

    fn params(cell_um: Option<f64>, expectation: bool, seed: u64, area: f64) -> GrainV3Params {
        let (composite_curves, layer_curves) = curves();
        GrainV3Params {
            support_um: SIGMA,
            cell_um,
            particle_area_um2: area,
            particle_scale: [1.0; 3],
            particle_scale_sublayers: [1.0; 3],
            density_min: [MIN_DENSITY; 3],
            uniformity: [0.5; 3],
            layer_max: std::array::from_fn(|l| [MAXIMA[l]; 3]),
            composite_curves,
            layer_curves,
            positive_film: false,
            seed,
            expectation,
        }
    }

    fn region(width: u32, height: u32) -> SourceRect {
        SourceRect {
            x: 0,
            y: 0,
            width: width as usize,
            height: height as usize,
        }
    }

    fn flat(width: u32, height: u32, value: f64) -> ImageBuf {
        let mut image = ImageBuf::new(width, height);
        for pixel in image.pixels_mut() {
            for c in 0..3 {
                pixel[c] = from_f64(value);
            }
        }
        image
    }

    /// Slanted edge, a narrow line, and flat dark/bright areas.
    fn patterned(width: u32, height: u32) -> ImageBuf {
        let mut image = ImageBuf::new(width, height);
        let line = (width * 5 / 8) as usize;
        let row_len = width as usize * 3;
        for (y, row) in image.data.chunks_mut(row_len).enumerate() {
            let edge = 0.25 * width as f64 + 0.3 * y as f64;
            for (x, pixel) in row.chunks_mut(3).enumerate() {
                let mut value = if (x as f64) < edge { 0.05 } else { 0.24 };
                if x == line || x == line + 1 {
                    value = 0.28;
                }
                for c in 0..3 {
                    pixel[c] = from_f64(value);
                }
            }
        }
        image
    }

    fn render(p: &GrainV3Params, image: &ImageBuf, rect: SourceRect, out: (u32, u32)) -> ImageBuf {
        apply(image, PITCH, rect, out.0, out.1, p).expect("V3 render")
    }

    fn pixel(image: &ImageBuf, px: usize, py: usize) -> [f64; 3] {
        let index = (py * image.width as usize + px) * 3;
        std::array::from_fn(|c| to_f64(image.data[index + c]))
    }

    /// Output tolerance of this numeric precision.
    fn precision_tol() -> f64 {
        if cfg!(feature = "precision-f64") {
            1e-11
        } else {
            2e-5
        }
    }

    /// Independent oracle: the normalized Gaussian truncated to
    /// `[-4 sigma, 4 sigma]`.
    fn trunc_cdf(x: f64, sigma: f64) -> f64 {
        let cdf = |v: f64| 0.5 * (1.0 + libm::erf(v / std::f64::consts::SQRT_2));
        let raw = (cdf(x / sigma) - cdf(-4.0)) / (cdf(4.0) - cdf(-4.0));
        raw.clamp(0.0, 1.0)
    }

    /// Independent oracle for the expected density of one output footprint.
    ///
    /// Midpoint quadrature over the footprint of the exact CDF-based integral
    /// of the truncated Gaussian over each source rectangle, with the same
    /// constant edge extension. It shares no filter or overlap helper with
    /// production.
    fn oracle_readout(
        image: &ImageBuf,
        rect: SourceRect,
        out: (u32, u32),
        px: usize,
        py: usize,
        step_um: f64,
    ) -> [f64; 3] {
        let width = image.width as usize;
        let height = image.height as usize;
        let x0 = rect.x as f64 * PITCH;
        let y0 = rect.y as f64 * PITCH;
        let col = rect.width as f64 * PITCH / out.0 as f64;
        let row = rect.height as f64 * PITCH / out.1 as f64;
        let (ax, bx) = (x0 + px as f64 * col, x0 + (px + 1) as f64 * col);
        let (ay, by) = (y0 + py as f64 * row, y0 + (py + 1) as f64 * row);
        let nx = ((bx - ax) / step_um).round().max(1.0) as usize;
        let ny = ((by - ay) / step_um).round().max(1.0) as usize;
        let dx = (bx - ax) / nx as f64;
        let dy = (by - ay) / ny as f64;
        let reach = SUPPORT_SIGMAS * SIGMA;
        let mut acc = [0.0f64; 3];
        for iy in 0..ny {
            let y = ay + (iy as f64 + 0.5) * dy;
            let sy_lo = ((y - reach) / PITCH).floor() as i64;
            let sy_hi = ((y + reach) / PITCH).ceil() as i64;
            for ix in 0..nx {
                let x = ax + (ix as f64 + 0.5) * dx;
                let sx_lo = ((x - reach) / PITCH).floor() as i64;
                let sx_hi = ((x + reach) / PITCH).ceil() as i64;
                for sy in sy_lo..=sy_hi {
                    let wy = trunc_cdf(y - sy as f64 * PITCH, SIGMA)
                        - trunc_cdf(y - (sy + 1) as f64 * PITCH, SIGMA);
                    if wy <= 0.0 {
                        continue;
                    }
                    let row_index = sy.clamp(0, height as i64 - 1) as usize * width;
                    for sx in sx_lo..=sx_hi {
                        let wx = trunc_cdf(x - sx as f64 * PITCH, SIGMA)
                            - trunc_cdf(x - (sx + 1) as f64 * PITCH, SIGMA);
                        if wx <= 0.0 {
                            continue;
                        }
                        let column = sx.clamp(0, width as i64 - 1) as usize;
                        let index = (row_index + column) * 3;
                        let weight = wx * wy;
                        for c in 0..3 {
                            acc[c] += weight * to_f64(image.data[index + c]);
                        }
                    }
                }
            }
        }
        let norm = 1.0 / (nx as f64 * ny as f64);
        std::array::from_fn(|c| acc[c] * norm)
    }

    /// Exact Poisson probability mass, used as an independent CDF oracle.
    fn poisson_pmf(k: u64, lambda: f64) -> f64 {
        (-lambda + k as f64 * lambda.ln() - libm::lgamma(k as f64 + 1.0)).exp()
    }

    /// Discrete readout weight of one cell's density for one output pixel:
    /// `sum_t k_t * w_j(n + t - radius)`.
    fn readout_weight(taps: &[f64], radius: i64, stencil: &[(i64, f64)], n: i64) -> f64 {
        let mut acc = 0.0;
        for (t, &k) in taps.iter().enumerate() {
            let index = n + t as i64 - radius;
            for &(cell, weight) in stencil {
                if cell == index {
                    acc += k * weight;
                }
            }
        }
        acc
    }

    #[test]
    fn allocation_saturates_capacity_and_preserves_the_target() {
        let capacity = [0.198, 0.099, 0.033];
        let weights = [0.06, 0.03, 0.01];
        let interior = allocate(0.2, &capacity, &weights);
        assert!(
            (interior.iter().sum::<f64>() - 0.2).abs() < 1e-12,
            "{interior:?}"
        );
        for l in 0..3 {
            assert!(interior[l] < capacity[l], "layer {l} must stay unsaturated");
            let proportional = 0.2 * weights[l] / weights.iter().sum::<f64>();
            assert!((interior[l] - proportional).abs() < 1e-12, "{interior:?}");
        }
        // Full and over-full targets commit every layer at its capacity.
        assert_eq!(allocate(0.33, &capacity, &weights), capacity);
        assert_eq!(allocate(1.0, &capacity, &weights), capacity);
        assert_eq!(allocate(0.0, &capacity, &weights), [0.0; 3]);
        // Zero weights fall back to unsaturated capacity fractions per
        // iteration, not to one multiplier equation on the original weights.
        let zero = allocate(0.25, &[0.2, 0.1, 0.05], &[1.0, 0.0, 0.0]);
        assert!((zero[0] - 0.2).abs() < 1e-12, "{zero:?}");
        assert!((zero[1] - 0.1 / 3.0).abs() < 1e-12, "{zero:?}");
        assert!((zero[2] - 0.05 / 3.0).abs() < 1e-12, "{zero:?}");
        assert!((zero.iter().sum::<f64>() - 0.25).abs() < 1e-12);
        // A layer without capacity receives nothing.
        let skip = allocate(0.15, &[0.2, 0.0, 0.05], &[1.0, 1.0, 1.0]);
        assert_eq!(skip[1], 0.0);
        assert!((skip.iter().sum::<f64>() - 0.15).abs() < 1e-12);
    }

    #[test]
    fn formation_taps_match_dense_integration_and_sum_to_one() {
        for cell in [SIGMA / 2.0, SIGMA / 8.0, 3.0 * SIGMA] {
            let taps = support_taps(SIGMA, cell);
            let radius = (SUPPORT_SIGMAS * SIGMA / cell).ceil() as i64 + 1;
            assert_eq!(taps.len() as i64, 2 * radius + 1);
            assert!(taps.iter().all(|&t| t >= 0.0));
            assert!((taps.iter().sum::<f64>() - 1.0).abs() < 1e-12);
            for (index, &value) in taps.iter().enumerate() {
                let offset = index as i64 - radius;
                // Dense midpoint quadrature of
                // `(1/h) * integral_dest integral_source g(x - z) dz dx`,
                // exact in z through the CDF and independent of the
                // production double antiderivative.
                let steps = 4096usize;
                let sub = cell / steps as f64;
                let mut acc = 0.0;
                for n in 0..steps {
                    let dest = offset as f64 * cell + (n as f64 + 0.5) * sub;
                    acc += (trunc_cdf(dest, SIGMA) - trunc_cdf(dest - cell, SIGMA)) * sub;
                }
                let expected = acc / cell;
                assert!(
                    (value - expected).abs() < 1e-6,
                    "cell {cell} offset {offset}: {value} vs {expected}"
                );
            }
        }
    }

    #[test]
    fn count_law_matches_poisson_probabilities() {
        let samples = 50_000u64;
        for &lambda in &[0.35f64, 3.0, 29.5, 30.5, 200.0] {
            let counts: Vec<u64> = (0..samples)
                .map(|index| {
                    let mut rng = FastStatsRng::stream(0x5EED_1234, 0, index);
                    poisson_exact(&mut rng, lambda)
                })
                .collect();
            let mean = counts.iter().sum::<u64>() as f64 / samples as f64;
            let variance = counts
                .iter()
                .map(|&k| (k as f64 - mean).powi(2))
                .sum::<f64>()
                / (samples as f64 - 1.0);
            let mean_sigma = (lambda / samples as f64).sqrt();
            assert!(
                (mean - lambda).abs() <= 5.0 * mean_sigma.max(1e-9),
                "lambda {lambda}: mean {mean}"
            );
            let variance_sigma = (2.0 * lambda * lambda / samples as f64).sqrt();
            assert!(
                (variance - lambda).abs() <= 5.0 * variance_sigma.max(1e-9),
                "lambda {lambda}: variance {variance}"
            );
            // CDF checks at zero, small, and tail-adjacent counts.
            let probes: [u64; 5] = [
                0,
                1,
                2,
                (lambda - 2.0 * lambda.sqrt()).max(3.0) as u64,
                lambda as u64,
            ];
            for &k in &probes {
                let exact: f64 = (0..=k).map(|j| poisson_pmf(j, lambda)).sum();
                if exact <= 0.0 || exact >= 1.0 {
                    continue;
                }
                let empirical = counts.iter().filter(|&&c| c <= k).count() as f64 / samples as f64;
                let sigma = (exact * (1.0 - exact) / samples as f64).sqrt();
                assert!(
                    (empirical - exact).abs() <= 5.0 * sigma + 1e-9,
                    "lambda {lambda}, k {k}: {empirical} vs {exact}"
                );
            }
        }
    }

    #[test]
    fn expectation_mode_reads_back_uniform_and_zero_targets() {
        let tol = precision_tol();
        let target = flat(48, 32, 0.12);
        let out = render(
            &params(None, true, 3, 1.0),
            &target,
            region(48, 32),
            (48, 32),
        );
        for py in 0..32usize {
            for px in 0..48usize {
                let value = pixel(&out, px, py);
                for c in 0..3 {
                    assert!((value[c] - 0.12).abs() < tol, "{px},{py} c{c}: {value:?}");
                }
            }
        }
        // Fractional footprints and the film boundary preserve constants.
        let frac = render(
            &params(None, true, 3, 1.0),
            &flat(20, 20, 0.12),
            region(20, 20),
            (13, 7),
        );
        for py in 0..7usize {
            for px in 0..13usize {
                let value = pixel(&frac, px, py);
                for c in 0..3 {
                    assert!(
                        (value[c] - 0.12).abs() < tol,
                        "fractional {px},{py}: {value:?}"
                    );
                }
            }
        }
        // A zero developed target reads back as zero.
        let zero = render(
            &params(None, true, 3, 1.0),
            &flat(16, 16, 0.0),
            region(16, 16),
            (16, 16),
        );
        for py in 0..16usize {
            for px in 0..16usize {
                let value = pixel(&zero, px, py);
                for c in 0..3 {
                    assert!(value[c].abs() < tol, "zero target {px},{py}: {value:?}");
                }
            }
        }
        // At the bottom of the supported set every count is exactly zero, so
        // realizations and the expectation coincide bit for bit.
        let floor = -MIN_DENSITY;
        let expected = render(
            &params(None, true, 1, 1.0),
            &flat(8, 8, floor),
            region(8, 8),
            (8, 8),
        );
        let realized = render(
            &params(None, false, 1, 1.0),
            &flat(8, 8, floor),
            region(8, 8),
            (8, 8),
        );
        for py in 0..8usize {
            for px in 0..8usize {
                let value = pixel(&expected, px, py);
                let actual = pixel(&realized, px, py);
                for c in 0..3 {
                    assert!((value[c] - floor).abs() < tol, "{px},{py}: {value:?}");
                    assert_eq!(actual[c], value[c], "zero-count floor {px},{py}");
                }
            }
        }
    }

    #[test]
    fn seed_ensemble_mean_matches_the_expectation_render() {
        let target = flat(32, 32, 0.12);
        let rect = region(32, 32);
        let out = (32u32, 32u32);
        let seeds = 32u64;
        let mut sum = vec![[0.0f64; 3]; (out.0 * out.1) as usize];
        let mut sum2 = vec![[0.0f64; 3]; sum.len()];
        for seed in 0..seeds {
            let image = render(&params(None, false, 0x1000 + seed, 1.0), &target, rect, out);
            for (index, value) in image.pixels().enumerate() {
                for c in 0..3 {
                    let v = to_f64(value[c]);
                    sum[index][c] += v;
                    sum2[index][c] += v * v;
                }
            }
        }
        let expectation = render(&params(None, true, 7, 1.0), &target, rect, out);
        let n = seeds as f64;
        let mut worst = 0.0f64;
        for (index, value) in expectation.pixels().enumerate() {
            for c in 0..3 {
                let mean = sum[index][c] / n;
                let variance = (sum2[index][c] - sum[index][c] * sum[index][c] / n) / (n - 1.0);
                let sigma = (variance / n).sqrt();
                let delta = (mean - to_f64(value[c])).abs();
                worst = worst.max(delta / sigma.max(1e-12));
            }
        }
        assert!(worst <= 5.0, "worst seed-mean deviation {worst} sigma");
    }

    #[test]
    fn readout_variance_scales_with_event_area() {
        let target = flat(32, 32, 0.12);
        let rect = region(32, 32);
        let out = (32u32, 32u32);
        // Exact statement: the per-cell variance scales with event area and
        // the support is untouched.
        let measure_area = |area: f64| -> (f64, f64) {
            let model =
                Model::new(&params(Some(SIGMA / 4.0), true, 1, area), PITCH).expect("model");
            let layers = model.layer_targets([0.12; 3]);
            let mut variance = 0.0;
            for c in 0..3 {
                let ch = &model.channels[c];
                for l in 0..3 {
                    let t = layers[c][l];
                    if t <= 0.0 || ch.cap[l] <= 0.0 {
                        continue;
                    }
                    let p = (t / ch.cap[l]).clamp(0.0, 1.0);
                    let s = 1.0 - ch.uniformity * (1.0 - 1e-6) * p;
                    let lambda = model.cell_um * model.cell_um * ch.frac[l] * p / (ch.area[l] * s);
                    let mass = ch.cap[l] * ch.area[l] * s / ch.frac[l];
                    variance += lambda * (mass / (model.cell_um * model.cell_um)).powi(2);
                }
            }
            (variance, model.taps.iter().sum())
        };
        let (v1, support1) = measure_area(0.5);
        let (v2, support2) = measure_area(2.0);
        assert!((support1 - 1.0).abs() < 1e-12 && (support2 - 1.0).abs() < 1e-12);
        assert!(
            (v2 / v1 - 4.0).abs() < 1e-9,
            "analytic variance ratio {}",
            v2 / v1
        );
        // Rendered evidence: the same scaling survives the realized path.
        let seeds = 24u64;
        let measure_render = |area: f64| -> (f64, f64) {
            let mut sum = vec![0.0f64; (out.0 * out.1) as usize];
            let mut sum2 = vec![0.0f64; sum.len()];
            for seed in 0..seeds {
                let image = render(
                    &params(None, false, 0x2000 + seed, area),
                    &target,
                    rect,
                    out,
                );
                for (index, value) in image.pixels().enumerate() {
                    let v = to_f64(value[1]);
                    sum[index] += v;
                    sum2[index] += v * v;
                }
            }
            let n = seeds as f64;
            let mut mean = 0.0;
            let mut variance = 0.0;
            for index in 0..sum.len() {
                mean += sum[index] / n;
                variance += (sum2[index] - sum[index] * sum[index] / n) / (n - 1.0);
            }
            (mean / sum.len() as f64, variance / sum.len() as f64)
        };
        let (mean1, var1) = measure_render(0.5);
        let (mean2, var2) = measure_render(2.0);
        assert!(
            (mean1 - mean2).abs() < 2e-3,
            "event area must not move the mean: {mean1} vs {mean2}"
        );
        let ratio = var2 / var1;
        assert!(
            (ratio - 4.0).abs() <= 1.0,
            "rendered variance ratio {ratio} (mean {mean1} vs {mean2})"
        );
    }

    #[test]
    fn crop_and_band_reads_agree_exactly() {
        let target = patterned(96, 200);
        let p = params(None, false, 0x99, 1.0);
        // Two output bands at the default band height.
        let full = render(&p, &target, region(96, 200), (96, 200));
        let crop = render(
            &p,
            &target,
            SourceRect {
                x: 8,
                y: 120,
                width: 32,
                height: 60,
            },
            (32, 60),
        );
        for py in 0..60usize {
            for px in 0..32usize {
                let a = pixel(&full, 8 + px, 120 + py);
                let b = pixel(&crop, px, py);
                for c in 0..3 {
                    assert_eq!(a[c], b[c], "crop pixel {px},{py} channel {c}");
                }
            }
        }
        // Identical footprints at an upscaled output agree too.
        let full2 = render(&p, &target, region(96, 96), (192, 192));
        let crop2 = render(
            &p,
            &target,
            SourceRect {
                x: 8,
                y: 40,
                width: 32,
                height: 24,
            },
            (64, 48),
        );
        for py in 0..48usize {
            for px in 0..64usize {
                let a = pixel(&full2, 16 + px, 80 + py);
                let b = pixel(&crop2, px, py);
                for c in 0..3 {
                    assert_eq!(a[c], b[c], "upscaled crop pixel {px},{py} channel {c}");
                }
            }
        }
    }

    #[test]
    fn renders_are_repeatable_and_seed_sensitive() {
        let target = patterned(32, 32);
        let a = render(
            &params(None, false, 5, 1.0),
            &target,
            region(32, 32),
            (32, 32),
        );
        let b = render(
            &params(None, false, 5, 1.0),
            &target,
            region(32, 32),
            (32, 32),
        );
        assert_eq!(a.data, b.data);
        let c = render(
            &params(None, false, 6, 1.0),
            &target,
            region(32, 32),
            (32, 32),
        );
        assert_ne!(a.data, c.data);
    }

    #[test]
    fn field_refinement_converges_to_the_independent_oracle() {
        let (width, height) = (48u32, 48u32);
        let target = patterned(width, height);
        let rect = region(width, height);
        let out = (width, height);
        let line = (width * 5 / 8) as usize;
        let probes = [
            (4usize, 24usize),
            (19usize, 24usize),
            (line, 24usize),
            (40usize, 24usize),
        ];
        // The oracle's own integration must converge.
        for &(px, py) in &probes {
            let coarse = oracle_readout(&target, rect, out, px, py, 1.0);
            let fine = oracle_readout(&target, rect, out, px, py, 0.5);
            for c in 0..3 {
                assert!(
                    (coarse[c] - fine[c]).abs() < 1e-4,
                    "oracle convergence {px},{py} c{c}: {coarse:?} vs {fine:?}"
                );
            }
        }
        let mut previous = f64::INFINITY;
        let mut production = 0.0f64;
        let mut certified = 0.0f64;
        // Production pitch `sigma/2`, then the certified pitches `sigma/4`
        // and `sigma/8`.
        for level in 0..3 {
            let cell = Some(SIGMA / (2.0 * (1 << level) as f64));
            let rendered = render(&params(cell, true, 0x77, 1.0), &target, rect, out);
            let mut worst = 0.0f64;
            for &(px, py) in &probes {
                let expected = oracle_readout(&target, rect, out, px, py, 0.5);
                let actual = pixel(&rendered, px, py);
                for c in 0..3 {
                    worst = worst.max((actual[c] - expected[c]).abs());
                }
            }
            assert!(
                worst <= previous.max(1e-9),
                "pitch refinement regressed at level {level}: {worst} vs {previous}"
            );
            if level == 0 {
                production = worst;
            }
            certified = worst;
            previous = worst;
        }
        // Declared production-pitch deviation, and the certification gate.
        assert!(
            production <= 5e-3,
            "production expected-density error {production}"
        );
        assert!(
            certified <= 1e-3,
            "certified expected-density error {certified}"
        );
    }

    #[test]
    fn discrete_covariance_tracks_the_continuum_oracle() {
        // Production pitch: measure how far the discrete operator is from the
        // continuum covariance of the declared model.
        let production = covariance_error(None);
        assert!(
            production.worst <= 0.10,
            "production-pitch covariance error {}, lag {}",
            production.worst,
            production.worst_lag
        );
        // Declared `sigma/4` deviation and the certified `sigma/8` gate.
        let quarter = covariance_error(Some(SIGMA / 4.0));
        assert!(
            quarter.worst <= 0.01,
            "sigma/4 covariance error {}, lag {}",
            quarter.worst,
            quarter.worst_lag
        );
        let certified = covariance_error(Some(SIGMA / 8.0));
        assert!(
            certified.worst <= 0.01,
            "certified covariance error {}, lag {}",
            certified.worst,
            certified.worst_lag
        );
        // A stable but wrongly scaled kernel must fail the comparison.
        let wrong = covariance_error_with_sigma(Some(SIGMA / 8.0), SIGMA * 1.25);
        assert!(
            wrong.worst > 0.01,
            "a wrong-width kernel must fail: {}",
            wrong.worst
        );
    }

    struct CovarianceError {
        worst: f64,
        worst_lag: usize,
    }

    fn covariance_error(cell: Option<f64>) -> CovarianceError {
        covariance_error_with_sigma(cell, SIGMA)
    }

    /// Relative error of the discrete readout covariance against the
    /// continuum `sum_lc v_lc * integral H_j(z) H_k(z) dz`, with `H` built
    /// from the oracle sigma. Production taps and readout weights supply the
    /// discrete side; the oracle side is an independent quadrature.
    fn covariance_error_with_sigma(cell: Option<f64>, oracle_sigma: f64) -> CovarianceError {
        let mut worst = 0.0f64;
        let mut worst_lag = 0usize;
        for (lag, discrete, oracle) in covariance_values(cell, oracle_sigma) {
            let relative = ((discrete - oracle) / oracle).abs();
            if relative > worst {
                worst = relative;
                worst_lag = lag;
            }
        }
        CovarianceError { worst, worst_lag }
    }

    /// `(lag, discrete, oracle)` readout covariances of the declared model.
    fn covariance_values(cell: Option<f64>, oracle_sigma: f64) -> Vec<(usize, f64, f64)> {
        let dense = 0.12;
        let p = params(cell, true, 0x31, 1.0);
        let model = Model::new(&p, PITCH).expect("model");
        let area = model.cell_um * model.cell_um;
        let layers = model.layer_targets([dense; 3]);
        let mut variance = [[0.0f64; 3]; 3];
        let mut continuum = [[0.0f64; 3]; 3];
        for c in 0..3 {
            let ch = &model.channels[c];
            for l in 0..3 {
                let t = layers[c][l];
                if t <= 0.0 || ch.cap[l] <= 0.0 {
                    continue;
                }
                let pl = (t / ch.cap[l]).clamp(0.0, 1.0);
                let s = 1.0 - ch.uniformity * (1.0 - 1e-6) * pl;
                let lambda = area * ch.frac[l] * pl / (ch.area[l] * s);
                let mass = ch.cap[l] * ch.area[l] * s / ch.frac[l];
                variance[c][l] = lambda * (mass / area).powi(2);
                continuum[c][l] = variance[c][l] * area;
            }
        }
        let d = model.radius;
        let footprint = PITCH;
        let x_read: Vec<Vec<(i64, f64)>> = (0..6)
            .map(|j| {
                readout_stencil(
                    30.0 + j as f64 * footprint,
                    30.0 + (j + 1) as f64 * footprint,
                    model.cell_um,
                )
            })
            .collect();
        let y_read = readout_stencil(30.0, 30.0 + footprint, model.cell_um);
        let (y_lo, y_hi) = (
            y_read.first().expect("stencil").0 - d - 1,
            y_read.last().expect("stencil").0 + d + 1,
        );
        let mut sy = 0.0;
        for n in y_lo..=y_hi {
            let w = readout_weight(&model.taps, d, &y_read, n);
            sy += w * w;
        }
        let oy = integrate(
            &|z: f64| {
                let h = (trunc_cdf(30.0 + footprint - z, oracle_sigma)
                    - trunc_cdf(30.0 - z, oracle_sigma))
                    / footprint;
                h * h
            },
            30.0 - 4.0 * oracle_sigma - footprint,
            30.0 + footprint + 4.0 * oracle_sigma + footprint,
        );
        let mut out = Vec::new();
        for lag in [0usize, 1, 2, 4] {
            let (n_lo, n_hi) = (
                x_read[0].first().expect("stencil").0 - d - 2,
                x_read[lag].last().expect("stencil").0 + d + 2,
            );
            let mut sx = 0.0;
            for n in n_lo..=n_hi {
                let a = readout_weight(&model.taps, d, &x_read[0], n);
                let b = readout_weight(&model.taps, d, &x_read[lag], n);
                sx += a * b;
            }
            let ox = integrate(
                &|z: f64| {
                    let left = |x0: f64| {
                        (trunc_cdf(x0 + footprint - z, oracle_sigma)
                            - trunc_cdf(x0 - z, oracle_sigma))
                            / footprint
                    };
                    left(30.0) * left(30.0 + lag as f64 * footprint)
                },
                30.0 - 4.0 * oracle_sigma - footprint,
                30.0 + (lag + 1) as f64 * footprint + 4.0 * oracle_sigma + footprint,
            );
            let mut discrete = 0.0;
            let mut oracle = 0.0;
            for c in 0..3 {
                for l in 0..3 {
                    discrete += variance[c][l] * sx * sy;
                    oracle += continuum[c][l] * ox * oy;
                }
            }
            out.push((lag, discrete, oracle));
        }
        out
    }

    /// Fine trapezoid integral on a fixed fine step.
    fn integrate(f: &dyn Fn(f64) -> f64, lo: f64, hi: f64) -> f64 {
        let steps = ((hi - lo) / (SIGMA / 400.0)).ceil() as usize;
        let step = (hi - lo) / steps as f64;
        let mut acc = 0.0;
        for i in 0..steps {
            let a = f(lo + i as f64 * step);
            let b = f(lo + (i + 1) as f64 * step);
            acc += 0.5 * (a + b) * step;
        }
        acc
    }

    #[test]
    fn invalid_inputs_fail_before_rendering() {
        let good = params(None, true, 1, 1.0);
        // Developed target outside the capacity envelope.
        let err = apply(&flat(8, 8, 0.4), PITCH, region(8, 8), 8, 8, &good).unwrap_err();
        assert!(err.contains("outside the supported"), "{err}");
        let err = apply(&flat(8, 8, -0.05), PITCH, region(8, 8), 8, 8, &good).unwrap_err();
        assert!(err.contains("outside the supported"), "{err}");
        // Region outside the source.
        let err = apply(
            &flat(8, 8, 0.1),
            PITCH,
            SourceRect {
                x: 4,
                y: 0,
                width: 8,
                height: 8,
            },
            8,
            8,
            &good,
        )
        .unwrap_err();
        assert!(err.contains("exceeds"), "{err}");
        let err = apply(
            &flat(8, 8, 0.1),
            PITCH,
            SourceRect {
                x: 0,
                y: 0,
                width: 0,
                height: 8,
            },
            8,
            8,
            &good,
        )
        .unwrap_err();
        assert!(err.contains("empty"), "{err}");
        // Support, pitch, and field pitch validation.
        let mut bad = good.clone();
        bad.support_um = 0.0;
        let err = apply(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
        assert!(err.contains("v3_dye_support_um"), "{err}");
        let err = apply(&flat(8, 8, 0.1), 0.0, region(8, 8), 8, 8, &good).unwrap_err();
        assert!(err.contains("pitch"), "{err}");
        let mut bad = good.clone();
        bad.cell_um = Some(SIGMA);
        let err = apply(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
        assert!(err.contains("field pitch"), "{err}");
        // Curve and control validation.
        let mut bad = good.clone();
        bad.composite_curves[3][1] = -1.0;
        let err = apply(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
        assert!(err.contains("nondecreasing"), "{err}");
        let mut bad = good.clone();
        bad.layer_curves[2][1][0] = -1.0;
        let err = apply(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
        assert!(err.contains("nonnegative"), "{err}");
        let mut bad = good.clone();
        bad.uniformity = [1.5, 0.0, 0.0];
        let err = apply(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
        assert!(err.contains("uniformity"), "{err}");
        let mut bad = good.clone();
        bad.density_min = [-0.1, 0.0, 0.0];
        let err = apply(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
        assert!(err.contains("density_min"), "{err}");
        let mut bad = good.clone();
        bad.particle_area_um2 = 0.0;
        let err = apply(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
        assert!(err.contains("particle_area_um2"), "{err}");
        let mut bad = good.clone();
        bad.layer_max = [[0.0; 3]; 3];
        let err = apply(&flat(8, 8, 0.1), PITCH, region(8, 8), 8, 8, &bad).unwrap_err();
        assert!(err.contains("capacity"), "{err}");
    }

    #[test]
    fn zero_capacity_layer_is_skipped() {
        let mut p = params(None, false, 0x44, 1.0);
        p.layer_max = [[0.18, 0.18, 0.18], [0.09, 0.09, 0.09], [0.0, 0.0, 0.0]];
        for row in p.layer_curves.iter_mut() {
            row[2] = [0.0; 3];
        }
        let target = flat(24, 24, 0.12);
        let out = render(&p, &target, region(24, 24), (24, 24));
        let expectation = render(
            &GrainV3Params {
                expectation: true,
                ..p.clone()
            },
            &target,
            region(24, 24),
            (24, 24),
        );
        for py in 0..24usize {
            for px in 0..24usize {
                let value = pixel(&out, px, py);
                let mean = pixel(&expectation, px, py);
                for c in 0..3 {
                    assert!((mean[c] - 0.12).abs() < 1e-6, "skipped layer mean {mean:?}");
                    assert!(value[c].is_finite());
                }
            }
        }
    }
}
