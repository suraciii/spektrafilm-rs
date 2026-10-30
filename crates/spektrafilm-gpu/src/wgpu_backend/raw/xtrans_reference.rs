//! Scalar reference for darktable's Markesteijn kernel order.
//!
//! This is the oracle for the eventual parallel GPU implementation: the same
//! decomposition, written once in plain Rust where the index arithmetic can be
//! read and checked. It is compiled only for tests, because nothing in the
//! production path may depend on it.
//!
//! Stage 1 covers the two kernels that build the working buffers:
//! `markesteijn_initial_copy`, and the `allhex`/`sgreen` tables that
//! `markesteijn_green_minmax` consumes. Source:
//! `data/kernels/demosaic_markesteijn.cl` and `process_markesteijn_cl()` in
//! `src/iop/demosaicing/xtrans.c`.

/// Cardinal and diagonal step pairs, copied verbatim from darktable.
pub(crate) const ORTH: [i32; 12] = [1, 0, 0, 1, -1, 0, 0, -1, 1, 0, 0, 1];

/// Neighbour offset pattern per CFA class (`0` = non-green site, `1` = green).
pub(crate) const PATT: [[i32; 16]; 2] = [
    [0, 1, 0, -1, 2, 0, -1, 0, 1, 1, 1, -1, 0, 0, 0, 0],
    [0, 1, 0, -2, 1, 0, -2, 0, 1, 1, -2, -2, 1, -1, -1, 1],
];

/// Working buffer layout: four interleaved floats per pixel, channels `R,G,B,aux`.
pub(crate) const RGB_STRIDE: usize = 4;

/// `FCNxtrans`: the CFA class of a site, wrapped over the pattern tile.
pub(crate) fn fcn(cfa: &[u8], rows: usize, cols: usize, y: i32, x: i32) -> u8 {
    let yy = y.rem_euclid(rows as i32) as usize;
    let xx = x.rem_euclid(cols as i32) as usize;
    cfa[yy * cols + xx]
}

/// `allhex[row][col][k]` in `(x, y)` order, plus the solitary-green site.
pub(crate) struct Tables {
    pub(crate) allhex: [[[[i32; 2]; 8]; 3]; 3],
    pub(crate) sgreen: [i32; 2],
}

impl Tables {
    /// Port of the host-side construction in `process_markesteijn_cl`.
    pub(crate) fn new(cfa: &[u8], rows: usize, cols: usize) -> Self {
        let mut allhex = [[[[0i32; 2]; 8]; 3]; 3];
        let mut sgreen = [0i32; 2];
        for row in 0..3i32 {
            for col in 0..3i32 {
                let mut ng = 0i32;
                let mut d = 0usize;
                while d < 10 {
                    let g = i32::from(fcn(cfa, rows, cols, row, col) == 1);
                    let dy = ORTH[d];
                    let dx = ORTH[d + 2];
                    if fcn(cfa, rows, cols, row + dy, col + dx) == 1 {
                        ng = 0;
                    } else {
                        ng += 1;
                    }
                    if ng == 4 {
                        sgreen = [col, row];
                    }
                    if ng == g + 1 {
                        for c in 0..8 {
                            let v = ORTH[d] * PATT[g as usize][c * 2]
                                + ORTH[d + 1] * PATT[g as usize][c * 2 + 1];
                            let h = ORTH[d + 2] * PATT[g as usize][c * 2]
                                + ORTH[d + 3] * PATT[g as usize][c * 2 + 1];
                            let slot = (c as i32) ^ ((g * 2) & (d as i32));
                            allhex[row as usize][col as usize][slot as usize] = [h, v];
                        }
                    }
                    d += 2;
                }
            }
        }
        Self { allhex, sgreen }
    }
}

/// `markesteijn_initial_copy`: keep the measured CFA sample, zero the rest,
/// clamp negatives.
#[allow(clippy::too_many_arguments)]
pub(crate) fn initial_copy(
    cfa: &[u8],
    rows: usize,
    cols: usize,
    image: &[f32],
    width: usize,
    height: usize,
) -> Vec<f32> {
    let mut rgb = vec![0.0f32; width * height * RGB_STRIDE];
    for y in 0..height {
        for x in 0..width {
            // The image carries four channels per pixel; the CFA sample is the
            // one the pattern names, matching the native `Areadsingle` read.
            let f = fcn(cfa, rows, cols, y as i32, x as i32) as usize;
            let p = image[(y * width + x) * RGB_STRIDE + f].max(0.0);
            let base = (y * width + x) * RGB_STRIDE;
            rgb[base + f] = p;
        }
    }
    rgb
}

/// `markesteijn_green_minmax`: per non-green site, the min/max of the green
/// samples on its own hexagon and on the hexagon of the next non-green site
/// along the four axis directions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn green_minmax(
    cfa: &[u8],
    rows: usize,
    cols: usize,
    rgb: &[f32],
    width: usize,
    height: usize,
    border: usize,
    tables: &Tables,
) -> Vec<f32> {
    let mut gminmax = vec![0.0f32; width * height * 2];
    let green = |y: i32, x: i32| -> Option<f32> {
        if y < 0 || x < 0 || y >= height as i32 || x >= width as i32 {
            return None;
        }
        Some(rgb[(y as usize * width + x as usize) * RGB_STRIDE + 1])
    };
    for y in border..height.saturating_sub(border) {
        for x in border..width.saturating_sub(border) {
            if fcn(cfa, rows, cols, y as i32, x as i32) == 1 {
                continue;
            }
            let mut gmin = f32::MAX;
            let mut gmax = 0.0f32;
            // The staged native buffer reads 0 outside the image; it does not
            // skip the tap, so a zero sample takes part in min/max.
            let mut sample = |ox: i32, oy: i32, offset: [i32; 2]| {
                let v = green(oy + offset[1], ox + offset[0]).unwrap_or(0.0);
                gmin = gmin.min(v);
                gmax = gmax.max(v);
            };
            let hex = &tables.allhex[y % 3][x % 3];
            let (xi, yi) = (x as i32, y as i32);
            for offset in &hex[..6] {
                sample(xi, yi, *offset);
            }
            // The native loop overwrites its choice, so the last non-green axis
            // neighbour wins.
            let mut last: Option<(i32, i32)> = None;
            for n in 0..4 {
                let dy = ORTH[2 * n + 1];
                let dx = ORTH[2 * n];
                if fcn(cfa, rows, cols, y as i32 + dy, x as i32 + dx) == 1 {
                    continue;
                }
                last = Some((y as i32 + dy, x as i32 + dx));
            }
            if let Some((ny, nx)) = last {
                for offset in
                    &tables.allhex[ny.rem_euclid(3) as usize][nx.rem_euclid(3) as usize][..6]
                {
                    sample(nx, ny, *offset);
                }
            }
            let base = (y * width + x) * 2;
            gminmax[base] = gmin;
            gminmax[base + 1] = gmax;
        }
    }
    gminmax
}

/// `markesteijn_solitary_green`: one of the six ordered calls per pass. `aux`
/// carries state between calls and `d` decides whether the previous result wins,
/// so a single call is not independently meaningful.
#[allow(clippy::too_many_arguments)]
pub(crate) fn solitary_green(
    cfa: &[u8],
    rows: usize,
    cols: usize,
    rgb: &mut [f32],
    aux: &mut [f32],
    width: usize,
    height: usize,
    border: usize,
    d: i32,
    dir: [i32; 2],
    hcomp: u8,
    tables: &Tables,
) {
    let src = rgb.to_vec();
    let at = |y: i32, x: i32, c: usize| -> f32 {
        if y < 0 || x < 0 || y >= height as i32 || x >= width as i32 {
            return 0.0;
        }
        src[(y as usize * width + x as usize) * RGB_STRIDE + c]
    };
    for y in border..height.saturating_sub(border) {
        for x in border..width.saturating_sub(border) {
            let (xi, yi) = (x as i32, y as i32);
            if (xi - tables.sgreen[0]) % 3 != 0 || (yi - tables.sgreen[1]) % 3 != 0 {
                continue;
            }
            let mut h = (fcn(cfa, rows, cols, yi, xi + 1) ^ hcomp) as usize;
            let idx = (y * width + x) * RGB_STRIDE;
            let mut color = [0.0f32; 4];
            let mut ocolor = [0.0f32; 4];
            if d > 0 {
                color.copy_from_slice(&aux[idx..idx + 4]);
                ocolor = color;
            }
            let mut diff = 0.0f32;
            for c in 0..2 {
                let k = 1 << c;
                let (oy, ox) = (dir[1] * k, dir[0] * k);
                let gp = at(yi + oy, xi + ox, 1);
                let gm = at(yi - oy, xi - ox, 1);
                let g = 2.0 * at(yi, xi, 1) - gp - gm;
                color[h] = g + at(yi + oy, xi + ox, h) + at(yi - oy, xi - ox, h);
                if d > 1 {
                    let t = gp - gm - at(yi + oy, xi + ox, h) + at(yi - oy, xi - ox, h);
                    diff += t * t + g * g;
                }
                h ^= 2;
            }
            color[3] = diff;
            if d > 1 && d & 1 != 0 {
                for c in 0..2 {
                    if ocolor[3] >= diff {
                        color[c * 2] = ocolor[c * 2];
                    }
                }
            }
            if d < 2 || d & 1 != 0 {
                for c in 0..2 {
                    rgb[idx + c * 2] = color[c * 2] / 2.0;
                }
            }
            aux[idx..idx + 4].copy_from_slice(&color);
        }
    }
}

/// `markesteijn_recalculate_green`: refines the green channel of each direction
/// buffer from the samples two steps out. Native has no local staging here, so
/// the snapshot below replaces a genuine intra-kernel read/write hazard.
#[allow(clippy::too_many_arguments)]
pub(crate) fn recalculate_green(
    cfa: &[u8],
    rows: usize,
    cols: usize,
    dirs: &mut [Vec<f32>; 4],
    gminmax: &[f32],
    width: usize,
    height: usize,
    border: usize,
    tables: &Tables,
) {
    let src: [Vec<f32>; 4] = std::array::from_fn(|i| dirs[i].clone());
    let sample = |d: usize, y: i32, x: i32, c: usize| -> f32 {
        if y < 0 || x < 0 || y >= height as i32 || x >= width as i32 {
            return 0.0;
        }
        src[d][(y as usize * width + x as usize) * RGB_STRIDE + c]
    };
    for y in border..height.saturating_sub(border) {
        for x in border..width.saturating_sub(border) {
            let f = fcn(cfa, rows, cols, y as i32, x as i32);
            if f == 1 {
                continue;
            }
            let f = f as usize;
            let base = (y * width + x) * 2;
            let (gmin, gmax) = (gminmax[base], gminmax[base + 1]);
            let flip = usize::from(((y as i32 - tables.sgreen[1]) % 3) == 0);
            let hex = &tables.allhex[y % 3][x % 3];
            for (offset, h) in hex[3..6].iter().enumerate() {
                let d = offset + 3;
                let id = (d - 2) ^ flip;
                let h = *h;
                let (oy, ox) = (y as i32, x as i32);
                let g = |k: i32| sample(id, oy + h[1] * k, ox + h[0] * k, 1);
                let v = |k: i32| sample(id, oy + h[1] * k, ox + h[0] * k, f);
                let val = g(-2) + 2.0 * g(1) - v(-2) - 2.0 * v(1) + 3.0 * v(0);
                dirs[id][(y * width + x) * RGB_STRIDE + 1] = (val / 3.0).clamp(gmin, gmax);
            }
        }
    }
}

/// `markesteijn_interpolate_green`: the four directional green estimates for
/// every non-green site, written into the green channel of the four direction
/// buffers. `dirs[i]` is one interleaved `R,G,B,aux` buffer of `width * height`
/// pixels; darktable passes them as four separate allocations.
#[allow(clippy::too_many_arguments)]
pub(crate) fn interpolate_green(
    cfa: &[u8],
    rows: usize,
    cols: usize,
    dirs: &mut [Vec<f32>; 4],
    gminmax: &[f32],
    width: usize,
    height: usize,
    border: usize,
    tables: &Tables,
) {
    // The native kernel stages rgb_0 into local memory before writing, so every
    // read sees the state at entry, never a concurrent write.
    let src = dirs[0].clone();
    let sample = |y: i32, x: i32, c: usize| -> f32 {
        if y < 0 || x < 0 || y >= height as i32 || x >= width as i32 {
            return 0.0;
        }
        src[(y as usize * width + x as usize) * RGB_STRIDE + c]
    };
    for y in border..height.saturating_sub(border) {
        for x in border..width.saturating_sub(border) {
            let f = fcn(cfa, rows, cols, y as i32, x as i32);
            if f == 1 {
                continue;
            }
            let f = f as usize;
            let base = (y * width + x) * 2;
            let gmin = gminmax[base];
            let gmax = gminmax[base + 1];
            let hex = &tables.allhex[y % 3][x % 3];
            let g = |d: [i32; 2], k: i32| sample(y as i32 + d[1] * k, x as i32 + d[0] * k, 1);
            let v = |d: [i32; 2], k: i32| sample(y as i32 + d[1] * k, x as i32 + d[0] * k, f);
            let mut color = [0.0f32; 4];
            color[0] = 0.679_687_5 * (g(hex[1], 1) + g(hex[0], 1))
                - 0.179_687_5 * (g(hex[1], 2) + g(hex[0], 2));
            color[1] = 0.871_093_75 * g(hex[3], 1)
                + 0.13 * g(hex[2], 1)
                + 0.359_375 * (sample(y as i32, x as i32, f) - v(hex[2], -1));
            for c in 0..2 {
                color[2 + c] = 0.640_625 * g(hex[4 + c], 1)
                    + 0.359_375 * g(hex[4 + c], -2)
                    + 0.128_906_25
                        * (2.0 * sample(y as i32, x as i32, f)
                            - v(hex[4 + c], 3)
                            - v(hex[4 + c], -3));
            }
            let flip = usize::from(((y as i32 - tables.sgreen[1]) % 3) == 0);
            let glidx = (y * width + x) * RGB_STRIDE;
            for c in 0..4 {
                dirs[c ^ flip][glidx + 1] = color[c].clamp(gmin, gmax);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 6x6 Fujifilm X-Trans pattern already used by the WGSL tests.
    const PATTERN: [u8; 36] = [
        1, 0, 1, 1, 2, 1, 2, 1, 2, 0, 1, 0, 1, 0, 1, 1, 2, 1, 1, 2, 1, 1, 0, 1, 0, 1, 0, 2, 1, 2,
        1, 2, 1, 1, 0, 1,
    ];

    #[test]
    fn allhex_alternates_between_green_and_non_green_sites() {
        let tables = Tables::new(&PATTERN, 6, 6);
        for row in 0..3i32 {
            for col in 0..3i32 {
                let centre_green = fcn(&PATTERN, 6, 6, row, col) == 1;
                // darktable maps "a green hexagon around each non-green pixel and
                // vice versa": around a non-green site every entry is a green one.
                if centre_green {
                    continue;
                }
                for offset in &tables.allhex[row as usize][col as usize] {
                    // Unused slots stay zero; a real offset always moves.
                    if offset == &[0, 0] {
                        continue;
                    }
                    assert!(
                        offset[0].abs() <= 2 && offset[1].abs() <= 2,
                        "offset {offset:?} escapes the native hexagon"
                    );
                    let neighbour = fcn(&PATTERN, 6, 6, row + offset[1], col + offset[0]);
                    assert_eq!(
                        neighbour, 1,
                        "phase ({row},{col}) neighbour {offset:?} is not a green site"
                    );
                }
            }
        }
    }

    #[test]
    fn sgreen_names_a_green_site() {
        let tables = Tables::new(&PATTERN, 6, 6);
        assert_eq!(
            fcn(&PATTERN, 6, 6, tables.sgreen[1], tables.sgreen[0]),
            1,
            "sgreen must be the solitary green site"
        );
    }

    #[test]
    fn initial_copy_keeps_only_the_measured_channel_and_clamps() {
        let (width, height) = (6usize, 6usize);
        let mut image = vec![0.0f32; width * height * RGB_STRIDE];
        for i in 0..width * height {
            for c in 0..RGB_STRIDE {
                image[i * RGB_STRIDE + c] = (i as f32) + c as f32;
            }
        }
        // One negative sample proves the clamp.
        image[RGB_STRIDE] = -7.0;
        let rgb = initial_copy(&PATTERN, 6, 6, &image, width, height);
        for y in 0..height {
            for x in 0..width {
                let f = fcn(&PATTERN, 6, 6, y as i32, x as i32) as usize;
                let base = (y * width + x) * RGB_STRIDE;
                for c in 0..3 {
                    if c == f {
                        assert!(rgb[base + c] >= 0.0);
                    } else {
                        assert_eq!(rgb[base + c], 0.0, "channel {c} at ({x},{y}) must be empty");
                    }
                }
            }
        }
        assert_eq!(rgb[RGB_STRIDE + 1], 0.0);
    }

    #[test]
    fn green_minmax_brackets_every_green_neighbour() {
        let (width, height) = (12usize, 12usize);
        let mut rgb = vec![0.0f32; width * height * RGB_STRIDE];
        for y in 0..height {
            for x in 0..width {
                rgb[(y * width + x) * RGB_STRIDE + 1] = (y * width + x) as f32;
            }
        }
        let tables = Tables::new(&PATTERN, 6, 6);
        let gminmax = green_minmax(&PATTERN, 6, 6, &rgb, width, height, 3, &tables);
        for y in 3..height - 3 {
            for x in 3..width - 3 {
                if fcn(&PATTERN, 6, 6, y as i32, x as i32) == 1 {
                    continue;
                }
                let base = (y * width + x) * 2;
                assert!(gminmax[base] <= gminmax[base + 1], "min/max inverted");
                assert!(gminmax[base + 1] > 0.0, "no green neighbour sampled");
            }
        }
    }

    #[test]
    fn interpolate_green_writes_each_direction_and_clamps() {
        let (width, height) = (12usize, 12usize);
        let mut dirs: [Vec<f32>; 4] =
            std::array::from_fn(|_| vec![0.0f32; width * height * RGB_STRIDE]);
        for dir in &mut dirs {
            for i in 0..width * height {
                dir[i * RGB_STRIDE + 1] = 100.0;
            }
        }
        let mut gminmax = vec![0.0f32; width * height * 2];
        for i in 0..width * height {
            gminmax[i * 2] = 90.0;
            gminmax[i * 2 + 1] = 110.0;
        }
        let tables = Tables::new(&PATTERN, 6, 6);
        interpolate_green(
            &PATTERN, 6, 6, &mut dirs, &gminmax, width, height, 3, &tables,
        );
        // Coefficient sums: color[0], color[2], color[3] sum to exactly 1 over a
        // constant green field; color[1] sums to 1.00109375 by construction.
        let expected = [100.0f32, 100.109_375, 100.0, 100.0];
        for y in 3..height - 3 {
            for x in 3..width - 3 {
                if fcn(&PATTERN, 6, 6, y as i32, x as i32) == 1 {
                    continue;
                }
                let flip = usize::from(((y as i32 - tables.sgreen[1]) % 3) == 0);
                let glidx = (y * width + x) * RGB_STRIDE;
                for c in 0..4 {
                    let got = dirs[c ^ flip][glidx + 1];
                    assert!(
                        (got - expected[c]).abs() < 1e-3,
                        "direction {c} at ({x},{y}) = {got}, want {}",
                        expected[c]
                    );
                }
            }
        }
    }

    #[test]
    fn gpu_initial_copy_matches_the_reference() {
        use super::super::RawBuffer;
        use super::super::demosaic::XTRANS_INIT_SHADER;
        use wgpu::BufferBindingType::{Storage, Uniform};
        let (width, height) = (12u32, 12u32);
        let count = width * height;
        let gpu = crate::test_support::adapter();
        let image: Vec<u32> = (0..count * 4).map(|i| (i * 97 % 65536) as u32).collect();
        let cfa: Vec<u32> = PATTERN.iter().map(|&c| u32::from(c)).collect();
        let params = [width, height, 6u32, 6u32];
        let image_buf = RawBuffer::upload(
            &gpu.device,
            "xtrans_init_image",
            bytemuck::cast_slice(&image),
            wgpu::BufferUsages::STORAGE,
        );
        let cfa_buf = RawBuffer::upload(
            &gpu.device,
            "xtrans_init_cfa",
            bytemuck::cast_slice(&cfa),
            wgpu::BufferUsages::STORAGE,
        );
        let params_buf = RawBuffer::upload(
            &gpu.device,
            "xtrans_init_params",
            bytemuck::cast_slice(&params),
            wgpu::BufferUsages::UNIFORM,
        );
        let out = RawBuffer::zeros(&gpu.device, "xtrans_init_rgb", u64::from(count) * 16);
        let dispatch = gpu.raw_dispatch(
            XTRANS_INIT_SHADER,
            &[&params_buf, &image_buf, &cfa_buf, &out],
            &[
                Uniform,
                Storage { read_only: true },
                Storage { read_only: true },
                Storage { read_only: false },
            ],
            count,
            "xtrans_init",
        );
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        dispatch.encode(&mut encoder, "xtrans_init");
        gpu.queue.submit(Some(encoder.finish()));
        let actual = gpu
            .pipe
            .readback_buffer_as::<f32>(&out.buffer, u64::from(count) * 16)
            .unwrap();

        let as_float: Vec<f32> = image.iter().map(|&v| v as f32).collect();
        let expected = initial_copy(&PATTERN, 6, 6, &as_float, width as usize, height as usize);
        assert_eq!(actual.len(), expected.len());
        for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(a, e, "sample {i} differs");
        }
    }

    #[test]
    fn gpu_green_minmax_matches_the_reference() {
        use super::super::RawBuffer;
        use super::super::demosaic::XTRANS_MINMAX_SHADER;
        use wgpu::BufferBindingType::{Storage, Uniform};
        let (width, height) = (12u32, 12u32);
        let count = width * height;
        let gpu = crate::test_support::adapter();
        let tables = Tables::new(&PATTERN, 6, 6);
        let image: Vec<f32> = (0..count * 4).map(|i| (i * 97 % 4096) as f32).collect();
        let rgb = initial_copy(&PATTERN, 6, 6, &image, width as usize, height as usize);
        let mut hexflat: Vec<[i32; 2]> = Vec::with_capacity(72);
        for row in &tables.allhex {
            for col in row {
                for k in col {
                    hexflat.push(*k);
                }
            }
        }
        let params = [width, height, 3u32, 6u32, 6u32, 0u32, 0u32, 0u32];
        let rgb_buf = RawBuffer::upload(
            &gpu.device,
            "mm_rgb",
            bytemuck::cast_slice(&rgb),
            wgpu::BufferUsages::STORAGE,
        );
        let cfa_buf = RawBuffer::upload(
            &gpu.device,
            "mm_cfa",
            bytemuck::cast_slice(&PATTERN.map(u32::from)),
            wgpu::BufferUsages::STORAGE,
        );
        let hex_buf = RawBuffer::upload(
            &gpu.device,
            "mm_hex",
            bytemuck::cast_slice(&hexflat),
            wgpu::BufferUsages::STORAGE,
        );
        let params_buf = RawBuffer::upload(
            &gpu.device,
            "mm_params",
            bytemuck::cast_slice(&params),
            wgpu::BufferUsages::UNIFORM,
        );
        let out = RawBuffer::zeros(&gpu.device, "mm_out", u64::from(count) * 8);
        let dispatch = gpu.raw_dispatch(
            XTRANS_MINMAX_SHADER,
            &[&params_buf, &rgb_buf, &cfa_buf, &hex_buf, &out],
            &[
                Uniform,
                Storage { read_only: true },
                Storage { read_only: true },
                Storage { read_only: true },
                Storage { read_only: false },
            ],
            count,
            "xtrans_minmax",
        );
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        dispatch.encode(&mut encoder, "xtrans_minmax");
        gpu.queue.submit(Some(encoder.finish()));
        let actual = gpu
            .pipe
            .readback_buffer_as::<f32>(&out.buffer, u64::from(count) * 8)
            .unwrap();
        let expected = green_minmax(
            &PATTERN,
            6,
            6,
            &rgb,
            width as usize,
            height as usize,
            3,
            &tables,
        );
        let mut checked = 0;
        for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(a, e, "gminmax sample {i} differs");
            if i % 2 == 1 && *e > 0.0 {
                checked += 1;
            }
        }
        assert!(checked > 0, "no non-green site was compared");
    }

    #[test]
    fn gpu_interpolate_green_matches_the_reference() {
        use super::super::RawBuffer;
        use super::super::demosaic::XTRANS_GREEN_SHADER;
        use wgpu::BufferBindingType::{Storage, Uniform};
        let (width, height) = (12u32, 12u32);
        let count = width * height;
        let gpu = crate::test_support::adapter();
        let tables = Tables::new(&PATTERN, 6, 6);
        let image: Vec<f32> = (0..count * 4).map(|i| (i * 97 % 4096) as f32).collect();
        let rgb = initial_copy(&PATTERN, 6, 6, &image, width as usize, height as usize);
        let gminmax = green_minmax(
            &PATTERN,
            6,
            6,
            &rgb,
            width as usize,
            height as usize,
            3,
            &tables,
        );

        let mut cpu: [Vec<f32>; 4] = std::array::from_fn(|_| rgb.clone());
        interpolate_green(
            &PATTERN,
            6,
            6,
            &mut cpu,
            &gminmax,
            width as usize,
            height as usize,
            3,
            &tables,
        );

        let mut hexflat: Vec<[i32; 2]> = Vec::with_capacity(72);
        for row in &tables.allhex {
            for col in row {
                for k in col {
                    hexflat.push(*k);
                }
            }
        }
        let params = [
            width,
            height,
            3u32,
            6u32,
            6u32,
            tables.sgreen[0] as u32,
            tables.sgreen[1] as u32,
            0u32,
        ];
        let bytes = u64::from(count) * 16;
        let src = RawBuffer::upload(
            &gpu.device,
            "ig_src",
            bytemuck::cast_slice(&rgb),
            wgpu::BufferUsages::STORAGE,
        );
        let cfa_buf = RawBuffer::upload(
            &gpu.device,
            "ig_cfa",
            bytemuck::cast_slice(&PATTERN.map(u32::from)),
            wgpu::BufferUsages::STORAGE,
        );
        let hex_buf = RawBuffer::upload(
            &gpu.device,
            "ig_hex",
            bytemuck::cast_slice(&hexflat),
            wgpu::BufferUsages::STORAGE,
        );
        let mm_buf = RawBuffer::upload(
            &gpu.device,
            "ig_mm",
            bytemuck::cast_slice(&gminmax),
            wgpu::BufferUsages::STORAGE,
        );
        let params_buf = RawBuffer::upload(
            &gpu.device,
            "ig_params",
            bytemuck::cast_slice(&params),
            wgpu::BufferUsages::UNIFORM,
        );
        let outs: Vec<RawBuffer> = (0..4)
            .map(|_| {
                RawBuffer::upload(
                    &gpu.device,
                    "ig_out",
                    bytemuck::cast_slice(&rgb),
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                )
            })
            .collect();
        let dispatch = gpu.raw_dispatch(
            XTRANS_GREEN_SHADER,
            &[
                &params_buf,
                &src,
                &cfa_buf,
                &hex_buf,
                &mm_buf,
                &outs[0],
                &outs[1],
                &outs[2],
                &outs[3],
            ],
            &[
                Uniform,
                Storage { read_only: true },
                Storage { read_only: true },
                Storage { read_only: true },
                Storage { read_only: true },
                Storage { read_only: false },
                Storage { read_only: false },
                Storage { read_only: false },
                Storage { read_only: false },
            ],
            count,
            "xtrans_green",
        );
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        dispatch.encode(&mut encoder, "xtrans_green");
        gpu.queue.submit(Some(encoder.finish()));
        for d in 0..4 {
            let actual = gpu
                .pipe
                .readback_buffer_as::<f32>(&outs[d].buffer, bytes)
                .unwrap();
            for (i, (a, e)) in actual.iter().zip(&cpu[d]).enumerate() {
                assert_eq!(a, e, "direction {d} sample {i} differs");
            }
        }
    }

    #[test]
    fn recalculate_green_is_the_identity_on_a_flat_field() {
        let (width, height) = (12usize, 12usize);
        let image: Vec<f32> = (0..width * height * 4)
            .map(|i| if i % 4 == 1 { 100.0 } else { 0.0 })
            .collect();
        let rgb = initial_copy(&PATTERN, 6, 6, &image, width, height);
        let mut gminmax = vec![0.0f32; width * height * 2];
        for i in 0..width * height {
            gminmax[i * 2] = 90.0;
            gminmax[i * 2 + 1] = 110.0;
        }
        let tables = Tables::new(&PATTERN, 6, 6);
        let mut dirs: [Vec<f32>; 4] = std::array::from_fn(|_| rgb.clone());
        recalculate_green(
            &PATTERN, 6, 6, &mut dirs, &gminmax, width, height, 3, &tables,
        );
        // val = 100 + 2*100 - 0 - 2*0 + 3*0 = 300, so val / 3 == 100 exactly.
        let mut written = [false; 4];
        for y in 3..height - 3 {
            for x in 3..width - 3 {
                if fcn(&PATTERN, 6, 6, y as i32, x as i32) == 1 {
                    continue;
                }
                let flip = usize::from(((y as i32 - tables.sgreen[1]) % 3) == 0);
                for d in 3..6 {
                    let id = (d - 2) ^ flip;
                    written[id] = true;
                    let got = dirs[id][(y * width + x) * RGB_STRIDE + 1];
                    assert_eq!(got, 100.0, "direction {id} at ({x},{y})");
                }
            }
        }
        // `id = (d - 2) ^ flip` with d in 3..6 only reaches 1..3 when flip is 0,
        // so direction 0 is written only on the rows where flip is 1.
        assert!(
            written[1] && written[2] && written[3],
            "some direction never written"
        );
    }

    #[test]
    fn solitary_green_is_the_identity_on_a_flat_field() {
        let (width, height) = (12usize, 12usize);
        let tables = Tables::new(&PATTERN, 6, 6);
        let v = 50.0f32;
        let mut rgb = vec![v; width * height * RGB_STRIDE];
        let mut aux = vec![0.0f32; width * height * RGB_STRIDE];
        solitary_green(
            &PATTERN,
            6,
            6,
            &mut rgb,
            &mut aux,
            width,
            height,
            3,
            0,
            [1, 0],
            0,
            &tables,
        );
        let mut checked = 0;
        for y in 3..height - 3 {
            for x in 3..width - 3 {
                let (xi, yi) = (x as i32, y as i32);
                if (xi - tables.sgreen[0]) % 3 != 0 || (yi - tables.sgreen[1]) % 3 != 0 {
                    continue;
                }
                checked += 1;
                let idx = (y * width + x) * RGB_STRIDE;
                // g = 2V - V - V = 0, color[h] = 2V, so rgb[c*2] = V and aux[h] = 2V.
                assert_eq!(rgb[idx], v, "red at ({x},{y})");
                assert_eq!(rgb[idx + 2], v, "blue at ({x},{y})");
                let h = (fcn(&PATTERN, 6, 6, yi, xi + 1)) as usize;
                assert_eq!(aux[idx + h], 2.0 * v, "aux[{h}] at ({x},{y})");
                assert_eq!(aux[idx + 3], 0.0, "diff at ({x},{y})");
            }
        }
        assert!(checked > 0, "no solitary green site in the fixture");
    }
}
