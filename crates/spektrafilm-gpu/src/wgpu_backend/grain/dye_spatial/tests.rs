use super::*;

fn device_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::wgpu_backend::observation::TEST_DEVICE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// KNOWN-BAD G3: the remap pass compiled with un-normalized overlaps.
const RAW_REMAP_SHADERS: SpatialShaders = SpatialShaders {
    remap: concat!(
        include_str!("../dye_spatial_params.wgsl"),
        include_str!("../dye_spatial_remap_control.wgsl")
    ),
    form_h: concat!(
        include_str!("../dye_spatial_params.wgsl"),
        include_str!("../dye_spatial_form_h.wgsl")
    ),
    form_v: concat!(
        include_str!("../dye_spatial_params.wgsl"),
        include_str!("../dye_spatial_form_v.wgsl")
    ),
    readout: concat!(
        include_str!("../dye_spatial_params.wgsl"),
        include_str!("../dye_spatial_readout.wgsl")
    ),
};

/// KNOWN-BAD G5: the readout pass compiled with the cell width as the
/// overlap normalization instead of the footprint width.
const CELL_NORM_SHADERS: SpatialShaders = SpatialShaders {
    remap: concat!(
        include_str!("../dye_spatial_params.wgsl"),
        include_str!("../dye_spatial_remap.wgsl")
    ),
    form_h: concat!(
        include_str!("../dye_spatial_params.wgsl"),
        include_str!("../dye_spatial_form_h.wgsl")
    ),
    form_v: concat!(
        include_str!("../dye_spatial_params.wgsl"),
        include_str!("../dye_spatial_form_v.wgsl")
    ),
    readout: concat!(
        include_str!("../dye_spatial_params.wgsl"),
        include_str!("../dye_spatial_readout_control.wgsl")
    ),
};

/// KNOWN-BAD G5: the formation passes compiled with the tap offset
/// flipped in both axes.
const FLIPPED_TAPS_SHADERS: SpatialShaders = SpatialShaders {
    remap: concat!(
        include_str!("../dye_spatial_params.wgsl"),
        include_str!("../dye_spatial_remap.wgsl")
    ),
    form_h: concat!(
        include_str!("../dye_spatial_params.wgsl"),
        include_str!("../dye_spatial_form_h_control.wgsl")
    ),
    form_v: concat!(
        include_str!("../dye_spatial_params.wgsl"),
        include_str!("../dye_spatial_form_v_control.wgsl")
    ),
    readout: concat!(
        include_str!("../dye_spatial_params.wgsl"),
        include_str!("../dye_spatial_readout.wgsl")
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
        remap_error = remap_error.max(max_error(&layers, &oracle_remap(&case).expect("oracle")));
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
                    let b =
                        ((whole_cy as u32 * whole.band.ext_cols + whole_cx as u32) * 9) as usize;
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
                error = error.max((f64::from(full_output[a]) - f64::from(crop_output[b])).abs());
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
    let largest_buffer = |band: &DyeBand| band_bytes(band).into_iter().fold(0, u64::max);
    let one = band_geometry(g.src_w, g.src_h, &params, 0, 1).expect("one row");
    let full = band_geometry(g.src_w, g.src_h, &params, 0, BAND_OUTPUT_ROWS).expect("full band");
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
    let tall =
        DyePlan::new(g.src_w, g.src_h, &params, &wgpu::Limits::default()).expect("unbounded plan");
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
