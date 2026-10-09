//! scikit-image 0.26 `rescale(order=3, channel_axis=2)` for float images.
//! Mirrors scipy 1.17 Gaussian antialiasing, mirror boundaries, cubic
//! spline prefilter, half-pixel grid coordinates and input-range clipping.
use crate::image::ImageBuf;
use crate::precision::{from_f64, to_f64};
use rayon::prelude::*;

fn mirror(i: i64, n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let period = 2 * n as i64 - 2;
    let j = i.rem_euclid(period);
    if j >= n as i64 {
        (period - j) as usize
    } else {
        j as usize
    }
}

fn spline(line: &mut [f64]) {
    let n = line.len();
    if n <= 1 {
        return;
    }
    let z = -0.267949192431122706472553658494127633f64;
    let gain = (1.0 - z) * (1.0 - 1.0 / z);
    for v in line.iter_mut() {
        *v *= gain;
    }
    let zn = z.powf((n - 1) as f64);
    let mut zi = z;
    let mut first = line[0] + zn * line[n - 1];
    for i in 1..n - 1 {
        first += zi * (line[i] + zn * line[n - 1 - i]);
        zi *= z;
    }
    line[0] = first / (1.0 - zn * zn);
    for i in 1..n {
        line[i] += z * line[i - 1];
    }
    line[n - 1] = (z * line[n - 2] + line[n - 1]) * z / (z * z - 1.0);
    for i in (0..n - 1).rev() {
        line[i] = z * (line[i + 1] - line[i]);
    }
}

fn axis(data: &mut [f64], w: usize, h: usize, which: usize, sigma: Option<f64>) {
    let (lines, len) = if which == 0 { (w, h) } else { (h, w) };
    let kernel = sigma.filter(|&s| s > 1e-15).map(|s| {
        let radius = (4.0 * s + 0.5) as i64;
        let mut k: Vec<f64> = (-radius..=radius)
            .map(|x| (-0.5 / (s * s) * x as f64 * x as f64).exp())
            .collect();
        let sum: f64 = k.iter().sum();
        for v in &mut k {
            *v /= sum;
        }
        k
    });
    if sigma.is_some() && kernel.is_none() {
        return;
    }
    let mut line = vec![0.0; len];
    let mut filtered = vec![0.0; len];
    for l in 0..lines {
        for c in 0..3 {
            let index = |i| {
                if which == 0 {
                    (i * w + l) * 3 + c
                } else {
                    (l * w + i) * 3 + c
                }
            };
            for i in 0..len {
                line[i] = data[index(i)];
            }
            if let Some(k) = &kernel {
                let radius = (k.len() / 2) as i64;
                for i in 0..len {
                    filtered[i] = k
                        .iter()
                        .enumerate()
                        .map(|(j, &v)| v * line[mirror(i as i64 + j as i64 - radius, len)])
                        .sum();
                }
                line.copy_from_slice(&filtered);
            } else {
                spline(&mut line);
            }
            for i in 0..len {
                data[index(i)] = line[i];
            }
        }
    }
}

fn taps(n: usize, out: usize) -> Vec<([usize; 4], [f64; 4])> {
    (0..out)
        .map(|i| {
            let mut x = (i as f64 + 0.5) * (n as f64 / out as f64) - 0.5;
            if n <= 1 {
                x = 0.0;
            } else {
                let period = (2 * n - 2) as f64;
                x = x.rem_euclid(period);
                if x > (n - 1) as f64 {
                    x = period - x;
                }
            }
            let start = x.floor() as i64 - 1;
            let y = x - x.floor();
            let z = 1.0 - y;
            let mut weights = [
                z * z * z / 6.0,
                (y * y * (y - 2.0) * 3.0 + 4.0) / 6.0,
                (z * z * (z - 2.0) * 3.0 + 4.0) / 6.0,
                0.0,
            ];
            weights[3] = 1.0 - weights[0] - weights[1] - weights[2];
            (
                [
                    mirror(start, n),
                    mirror(start + 1, n),
                    mirror(start + 2, n),
                    mirror(start + 3, n),
                ],
                weights,
            )
        })
        .collect()
}

/// Python preprocess converts to float64; interpolation remains f64 here
/// until the configured Scalar boundary. The channel spline roundtrip is
/// mathematically identity and is omitted (roundoff only).
pub fn rescale_spline3(image: &ImageBuf, scale: f64) -> Result<ImageBuf, String> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(format!(
            "io.upscale_factor must be finite and > 0 (got {scale})"
        ));
    }
    let (w, h) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 {
        return Err("cannot resize an empty image".into());
    }
    let ow = (w as f64 * scale).round_ties_even().max(1.0);
    let oh = (h as f64 * scale).round_ties_even().max(1.0);
    if ow > u32::MAX as f64 || oh > u32::MAX as f64 {
        return Err("resize dimensions exceed u32 image limits".into());
    }
    let (fx, fy) = (w as f64 / ow, h as f64 / oh);
    let (zx, zy) = (1.0 / fx, 1.0 / fy);
    let (ow, oh) = (
        (w as f64 * zx).round_ties_even() as usize,
        (h as f64 * zy).round_ties_even() as usize,
    );
    if zx == 1.0 && zy == 1.0 {
        return Ok(image.clone());
    }
    let mut data: Vec<f64> = image.data.iter().map(|&v| to_f64(v)).collect();
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if ow < w || oh < h {
        axis(&mut data, w, h, 0, Some(((fy - 1.0) / 2.0).max(0.0)));
        axis(&mut data, w, h, 1, Some(((fx - 1.0) / 2.0).max(0.0)));
    }
    axis(&mut data, w, h, 0, None);
    axis(&mut data, w, h, 1, None);
    let (xt, yt) = (taps(w, ow), taps(h, oh));
    let len = ow
        .checked_mul(oh)
        .and_then(|n| n.checked_mul(3))
        .ok_or("resize allocation overflows address space")?;
    let mut output = vec![from_f64(0.0); len];
    output
        .par_chunks_mut(ow * 3)
        .zip(yt.par_iter())
        .for_each(|(row, (yi, yw))| {
            for (x, (xi, xw)) in xt.iter().enumerate() {
                for c in 0..3 {
                    let mut value = 0.0;
                    for a in 0..4 {
                        for b in 0..4 {
                            value += data[(yi[a] * w + xi[b]) * 3 + c] * yw[a] * xw[b];
                        }
                    }
                    row[x * 3 + c] = from_f64(value.clamp(min, max));
                }
            }
        });
    Ok(ImageBuf::from_data(ow as u32, oh as u32, output))
}

/// `skimage.transform.rescale(order=0, channel_axis=2)` for the
/// autoexposure `small_preview`: Gaussian antialiasing on the float
/// source (mirror boundary, `sigma = max(0, (factor-1)/2)` per axis —
/// scikit-image's default `anti_aliasing=True` fires for order 0 too),
/// then nearest-neighbour sampling on the half-pixel grid
/// (`floor((o+0.5)·src/out)`, `ndi.zoom(grid_mode=True, order=0)`).
///
/// Returns the raw f64 preview and its `(width, height)` — the metering
/// chain stays f64 (no Scalar rounding), and the CCTF decode happens
/// downstream, after this downsample, exactly like `colour.RGB_to_XYZ(
/// apply_cctf_decoding=True)` inside upstream `measure_autoexposure_ev`.
pub fn rescale_nearest0(image: &ImageBuf, scale: f64) -> Result<(Vec<f64>, usize, usize), String> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(format!(
            "preview scale must be finite and > 0 (got {scale})"
        ));
    }
    let (w, h) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 {
        return Err("cannot resize an empty image".into());
    }
    let ow = (w as f64 * scale).round_ties_even().max(1.0);
    let oh = (h as f64 * scale).round_ties_even().max(1.0);
    let (fx, fy) = (w as f64 / ow, h as f64 / oh);
    let (ow, oh) = (
        (w as f64 * (1.0 / fx)).round_ties_even() as usize,
        (h as f64 * (1.0 / fy)).round_ties_even() as usize,
    );
    let mut data: Vec<f64> = image.data.iter().map(|&v| to_f64(v)).collect();
    // Per-axis antialiasing; `axis` no-ops an axis that is not being
    // downscaled (sigma clamps to 0), like `gaussian_filter` with sigma 0.
    axis(&mut data, w, h, 0, Some(((fy - 1.0) / 2.0).max(0.0)));
    axis(&mut data, w, h, 1, Some(((fx - 1.0) / 2.0).max(0.0)));
    let nearest = |out_dim: usize, src_dim: usize| -> Vec<usize> {
        let factor = src_dim as f64 / out_dim as f64;
        (0..out_dim)
            .map(|o| (((o as f64 + 0.5) * factor).floor() as usize).min(src_dim - 1))
            .collect()
    };
    let (ix, iy) = (nearest(ow, w), nearest(oh, h));
    let len = ow
        .checked_mul(oh)
        .and_then(|n| n.checked_mul(3))
        .ok_or("resize allocation overflows address space")?;
    let mut output = vec![0.0f64; len];
    output
        .par_chunks_mut(ow * 3)
        .zip(iy.par_iter())
        .for_each(|(row, &y)| {
            let src_row = y * w * 3;
            for (x, &sx) in ix.iter().enumerate() {
                row[x * 3] = data[src_row + sx * 3];
                row[x * 3 + 1] = data[src_row + sx * 3 + 1];
                row[x * 3 + 2] = data[src_row + sx * 3 + 2];
            }
        });
    Ok((output, ow, oh))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shape_and_constant_channels() {
        let image = ImageBuf::from_data(
            3,
            5,
            (0..15)
                .flat_map(|_| [from_f64(0.25), from_f64(0.5), from_f64(0.75)])
                .collect(),
        );
        for (factor, w, h) in [(0.5, 2, 2), (1.0, 3, 5), (2.0, 6, 10), (0.01, 1, 1)] {
            let out = rescale_spline3(&image, factor).unwrap();
            assert_eq!((out.width, out.height), (w, h));
            for (i, &v) in out.data.iter().enumerate() {
                assert!((to_f64(v) - [0.25, 0.5, 0.75][i % 3]).abs() < 1e-6);
            }
        }
    }
    #[test]
    fn singleton_axes_stay_constant() {
        let image = ImageBuf::from_data(1, 1, vec![from_f64(0.25); 3]);
        let out = rescale_spline3(&image, 3.0).unwrap();
        assert_eq!((out.width, out.height), (3, 3));
        assert!(out.data.iter().all(|&v| (to_f64(v) - 0.25).abs() < 1e-6));
    }
}
