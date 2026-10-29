//! Bounded scalar effective-record law, independent of tone-curve fitting.
use super::*;

/// Scalar record law. Fixed positive event contributions are a hypothesis,
/// not a calibrated material claim. No field is inferred from tone-curve fits.
#[derive(Clone, Copy, Debug)]
pub struct RecordFieldParams {
    /// Integral of one event's density response, in D * um^2. Not grain area.
    pub event_contribution_d_um2: f64,
    /// Effective Gaussian formation sigma, not a measured particle radius.
    pub support_um: f64,
    /// Explicit numerical pitch, at most half the formation sigma.
    pub cell_um: f64,
    pub seed: u64,
    pub expectation: bool,
}

impl RecordFieldParams {
    fn formation_radius(&self) -> Result<usize, String> {
        let fail =
            || "film grain record law: invalid or over-budget formation parameters".to_owned();
        let h = self.cell_um;
        let area = h * h;
        let density_per_event = self.event_contribution_d_um2 / area;
        if [
            h,
            self.support_um,
            self.event_contribution_d_um2,
            area,
            density_per_event,
        ]
        .iter()
        .any(|x| !x.is_finite() || *x <= 0.0)
            || h > self.support_um / CELLS_PER_SIGMA
            || density_per_event > max_scalar_f64()
        {
            return Err(fail());
        }
        let radius = (SUPPORT_SIGMAS * self.support_um / h).ceil() + 1.0;
        if !radius.is_finite() || (2.0 * radius + 1.0).powi(2) > MAX_SCRATCH_CELLS as f64 {
            return Err(fail());
        }
        Ok(radius as usize)
    }

    /// Resolve the same bounded formation operator for CPU and device packing.
    pub fn formation_taps(&self) -> Result<Vec<f64>, String> {
        self.formation_radius()?;
        Ok(support_taps(self.support_um, self.cell_um))
    }
}

/// Form an absolute nonnegative scalar mean-density target, then average density.
/// All three input lanes must agree. Expectation is the formed target, not an
/// unchanged sharp input. No statistical floor is injected or subtracted.
pub fn apply_record_field(
    target: &ImageBuf,
    pitch_um: f64,
    grid: ReadoutGrid,
    params: &RecordFieldParams,
) -> Result<ImageBuf, String> {
    render(target, pitch_um, grid, params, None)
}

/// Apply a pointwise, shape-preserving optical response before averaging.
/// The callback sees formed absolute density in three identical lanes. It may
/// return different optical lanes. No spatial or image-dependent processing is
/// allowed. The caller owns base absorption, spectra, and encoding after readout.
pub fn apply_record_field_with_response(
    target: &ImageBuf,
    pitch_um: f64,
    grid: ReadoutGrid,
    params: &RecordFieldParams,
    response: impl Fn(&ImageBuf) -> Result<ImageBuf, String> + Sync,
) -> Result<ImageBuf, String> {
    render(target, pitch_um, grid, params, Some(&response))
}

// Explicit full-image limits are independent of the bounded per-tile scratch.
const MAX_SOURCE_PIXELS: u64 = 1 << 26;
const MAX_OUTPUT_PIXELS: u64 = 1 << 26;
const MAX_SCRATCH_CELLS: u64 = 1 << 18;
const MAX_STENCIL_ENTRIES: u64 = 1 << 18;

fn geometry_error() -> String {
    "film grain record law: footprint exceeds the scratch budget or address range".to_owned()
}

/// A footprint always uses the original request's pixel index, never a tile's
/// rounded origin plus a local offset. Adjacent pixels share the very same
/// endpoint expression, including at tile seams.
#[derive(Clone, Copy)]
struct Footprint {
    lo: f64,
    hi: f64,
    first: i64,
    past: i64,
}

impl Footprint {
    fn new(
        grid: ReadoutGrid,
        axis: usize,
        pixel: u32,
        h: f64,
        radius: usize,
    ) -> Result<Self, String> {
        let endpoint = |i: u32| grid.origin_um[axis] + f64::from(i) * grid.pitch_um[axis];
        let lo = endpoint(pixel);
        let hi = endpoint(pixel + 1);
        let address = i32::MAX as f64 - radius as f64 - 2.0;
        if !lo.is_finite()
            || !hi.is_finite()
            || hi <= lo
            || (lo / h).abs() > address
            || (hi / h).abs() > address
        {
            return Err(geometry_error());
        }
        let first = (lo / h).floor() as i64;
        let past = (hi / h).ceil() as i64;
        if past <= first || (past - first) as u64 > MAX_SCRATCH_CELLS {
            return Err(geometry_error());
        }
        Ok(Self {
            lo,
            hi,
            first,
            past,
        })
    }

    fn weights(self, h: f64) -> impl Iterator<Item = (i64, f64)> {
        (self.first..self.past).filter_map(move |i| {
            let overlap = self.hi.min((i + 1) as f64 * h) - self.lo.max(i as f64 * h);
            (overlap > 0.0).then_some((i, overlap / (self.hi - self.lo)))
        })
    }
}

#[derive(Clone, Copy)]
struct Tile {
    pixel: [u32; 2],
    size: [u32; 2],
    first: [i64; 2],
    cells: [usize; 2],
}

/// Plan one bounded tile without allocating. The same plan is recomputed in
/// the rendering pass, so preflight needs no image-proportional tile storage.
fn plan_tile(
    target: &ImageBuf,
    pitch: f64,
    grid: ReadoutGrid,
    h: f64,
    radius: usize,
    pixel: [u32; 2],
    size: [u32; 2],
) -> Result<Tile, String> {
    let mut first = [i64::MAX; 2];
    let mut past = [i64::MIN; 2];
    for axis in 0..2 {
        for p in pixel[axis]..pixel[axis] + size[axis] {
            let footprint = Footprint::new(grid, axis, p, h, radius)?;
            first[axis] = first[axis].min(footprint.first);
            past[axis] = past[axis].max(footprint.past);
        }
    }
    let cells = std::array::from_fn(|axis| (past[axis] - first[axis]) as usize);
    let ext = cells.map(|n| n + 2 * radius);
    if (ext[0] as u64).saturating_mul(ext[1] as u64) > MAX_SCRATCH_CELLS {
        return Err(geometry_error());
    }
    // Each source interval has at most ceil(h/pitch)+1 interior entries and
    // two edge-extension entries; cap the bound by the entire source axis.
    let mut entries = 0_u64;
    for axis in 0..2 {
        let source_extent = [target.width, target.height][axis];
        let per_cell = ((h / pitch).ceil() as u64)
            .saturating_add(3)
            .min(u64::from(source_extent) + 2);
        entries = entries.saturating_add((ext[axis] as u64).saturating_mul(per_cell));
        if entries > MAX_STENCIL_ENTRIES {
            return Err(geometry_error());
        }
        for offset in 0..ext[axis] {
            let cell = first[axis] - radius as i64 + offset as i64;
            let lo = cell as f64 * h;
            let hi = (cell + 1) as f64 * h;
            if !lo.is_finite() || !hi.is_finite() || hi <= lo {
                return Err(geometry_error());
            }
        }
    }
    Ok(Tile {
        pixel,
        size,
        first,
        cells,
    })
}

fn render(
    target: &ImageBuf,
    pitch: f64,
    grid: ReadoutGrid,
    p: &RecordFieldParams,
    response: Option<CellResponse<'_>>,
) -> Result<ImageBuf, String> {
    let fail = || "film grain record law: invalid or over-budget input".to_owned();
    let h = p.cell_um;
    let q = p.event_contribution_d_um2;
    let radius = p.formation_radius()?;
    if !pitch.is_finite()
        || pitch <= 0.0
        || grid.origin_um.iter().any(|x| !x.is_finite())
        || grid.pitch_um.iter().any(|x| !x.is_finite() || *x <= 0.0)
    {
        return Err(fail());
    }
    let density_per_event = q / (h * h);
    for (w, ht, limit) in [
        (target.width, target.height, MAX_SOURCE_PIXELS),
        (grid.width, grid.height, MAX_OUTPUT_PIXELS),
    ] {
        if w == 0 || ht == 0 || u64::from(w) * u64::from(ht) > limit {
            return Err(fail());
        }
    }
    if target.data.len() != target.width as usize * target.height as usize * 3 {
        return Err(fail());
    }
    for rgb in target.data.as_chunks::<3>().0 {
        let d = to_f64(rgb[0]);
        if !d.is_finite() || d < 0.0 || rgb[1] != rgb[0] || rgb[2] != rgb[0] {
            return Err(
                "film grain record law: target must be finite, nonnegative and scalar".into(),
            );
        }
        let rate = d / density_per_event;
        if !rate.is_finite() || rate > (1_u64 << 52) as f64 || (d > 0.0 && rate == 0.0) {
            return Err("film grain record law: event rate is outside the supported range".into());
        }
    }
    if !(f64::from(target.width) * pitch).is_finite()
        || !(f64::from(target.height) * pitch).is_finite()
    {
        return Err(fail());
    }
    let tile_size = grid
        .pitch_um
        .map(|step| (128.0 * h / step).floor().clamp(1.0, 128.0) as u32);
    // Preflight EVERY tile, including source stencils and halo addresses, before
    // allocating the full output or invoking any caller-provided response.
    for y in (0..grid.height).step_by(tile_size[1] as usize) {
        for x in (0..grid.width).step_by(tile_size[0] as usize) {
            plan_tile(
                target,
                pitch,
                grid,
                h,
                radius,
                [x, y],
                [
                    tile_size[0].min(grid.width - x),
                    tile_size[1].min(grid.height - y),
                ],
            )?;
        }
    }
    let taps = support_taps(p.support_um, h);
    let mut out = ImageBuf::new(grid.width, grid.height);
    let mut scratch = Scratch::default();
    for y in (0..grid.height).step_by(tile_size[1] as usize) {
        for x in (0..grid.width).step_by(tile_size[0] as usize) {
            let tile = plan_tile(
                target,
                pitch,
                grid,
                h,
                radius,
                [x, y],
                [
                    tile_size[0].min(grid.width - x),
                    tile_size[1].min(grid.height - y),
                ],
            )?;
            render_tile(
                target,
                pitch,
                grid,
                p,
                tile,
                &taps,
                response,
                &mut out,
                &mut scratch,
            )?;
        }
    }
    Ok(out)
}

#[derive(Default)]
struct Scratch {
    xs: Vec<Vec<(usize, f64)>>,
    ys: Vec<Vec<(usize, f64)>>,
    cells: Vec<f64>,
    horizontal: Vec<f64>,
    formed: Vec<f64>,
}

#[allow(clippy::too_many_arguments)]
fn render_tile(
    target: &ImageBuf,
    pitch: f64,
    grid: ReadoutGrid,
    p: &RecordFieldParams,
    tile: Tile,
    taps: &[f64],
    response: Option<CellResponse<'_>>,
    out: &mut ImageBuf,
    scratch: &mut Scratch,
) -> Result<(), String> {
    let h = p.cell_um;
    let density_per_event = p.event_contribution_d_um2 / (h * h);
    let dr = taps.len() / 2;
    let [cols, rows] = tile.cells;
    let ext_cols = cols + 2 * dr;
    let ext_rows = rows + 2 * dr;
    let ext = tile.first.map(|i| i - dr as i64);
    let Scratch {
        xs,
        ys,
        cells,
        horizontal,
        formed,
    } = scratch;
    xs.resize_with(xs.len().max(ext_cols), Vec::new);
    ys.resize_with(ys.len().max(ext_rows), Vec::new);
    for (i, weights) in xs[..ext_cols].iter_mut().enumerate() {
        let cell = ext[0] + i as i64;
        fill_axis_stencil(
            weights,
            cell as f64 * h,
            (cell + 1) as f64 * h,
            pitch,
            target.width as usize,
        );
    }
    for (j, weights) in ys[..ext_rows].iter_mut().enumerate() {
        let cell = ext[1] + j as i64;
        fill_axis_stencil(
            weights,
            cell as f64 * h,
            (cell + 1) as f64 * h,
            pitch,
            target.height as usize,
        );
    }
    cells.resize(ext_cols * ext_rows, 0.0);
    cells
        .par_chunks_mut(ext_cols)
        .zip(ys[..ext_rows].par_iter())
        .enumerate()
        .try_for_each(|(j, (row, wy))| -> Result<(), String> {
            for (i, wx) in xs[..ext_cols].iter().enumerate() {
                let mut mean = 0.0;
                for &(sy, b) in wy {
                    for &(sx, a) in wx {
                        mean += a * b * to_f64(target.data[(sy * target.width as usize + sx) * 3]);
                    }
                }
                let value = if p.expectation {
                    mean
                } else {
                    let address = cell_stream_index(ext[0] + i as i64, ext[1] + j as i64);
                    let mut rng = FastStatsRng::stream(p.seed, 0, address);
                    density_per_event * poisson_exact(&mut rng, mean / density_per_event)? as f64
                };
                if !value.is_finite() || value > max_scalar_f64() {
                    return Err(geometry_error());
                }
                row[i] = value;
            }
            Ok(())
        })?;
    horizontal.resize(ext_rows * cols, 0.0);
    formed.resize(rows * cols, 0.0);
    horizontal_pass(cells, horizontal, ext_rows, ext_cols, cols, dr, taps);
    vertical_pass(horizontal, formed, ext_rows, cols, rows, dr, taps);
    if let Some(f) = response {
        let image = ImageBuf::from_data(
            cols as u32,
            rows as u32,
            formed.iter().flat_map(|&v| [from_f64(v); 3]).collect(),
        );
        let image = f(&image)?;
        if image.width != cols as u32
            || image.height != rows as u32
            || image.data.len() != cols * rows * 3
            || image.data.iter().any(|&x| !to_f64(x).is_finite())
        {
            return Err("film grain record law: response must preserve shape and be finite".into());
        }
        read_tile(
            grid,
            h,
            tile,
            dr,
            |index| std::array::from_fn(|c| to_f64(image.data[index * 3 + c])),
            out,
        )
    } else {
        read_tile(
            grid,
            h,
            tile,
            dr,
            |index| [to_f64(from_f64(formed[index])); 3],
            out,
        )
    }
}

fn read_tile(
    grid: ReadoutGrid,
    h: f64,
    tile: Tile,
    radius: usize,
    sample: impl Fn(usize) -> [f64; 3],
    out: &mut ImageBuf,
) -> Result<(), String> {
    let cols = tile.cells[0];
    for y in tile.pixel[1]..tile.pixel[1] + tile.size[1] {
        let wy = Footprint::new(grid, 1, y, h, radius)?;
        for x in tile.pixel[0]..tile.pixel[0] + tile.size[0] {
            let wx = Footprint::new(grid, 0, x, h, radius)?;
            let mut sum = [0.0; 3];
            for (j, b) in wy.weights(h) {
                for (i, a) in wx.weights(h) {
                    let index = (j - tile.first[1]) as usize * cols + (i - tile.first[0]) as usize;
                    for (value, lane) in sum.iter_mut().zip(sample(index)) {
                        *value += a * b * lane;
                    }
                }
            }
            for (c, value) in sum.into_iter().enumerate() {
                if !value.is_finite() || value.abs() > max_scalar_f64() {
                    return Err(geometry_error());
                }
                out.data[(y as usize * grid.width as usize + x as usize) * 3 + c] = from_f64(value);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
