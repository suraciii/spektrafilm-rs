use rayon::prelude::*;
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::to_f64;

use crate::params::RuntimeParams;

/// Auto-exposure compensation, Python-compatible
/// (`spektrafilm/utils/autoexposure.py`).
///
/// Python downsamples the image to ≤ 256 px on the long edge using
/// `skimage.transform.rescale(order=0)` before measuring
/// (`small_preview`). Scikit-image's default `anti_aliasing=True` fires
/// for order 0 as well: a Gaussian (`sigma = max(0, (factor-1)/2)` per
/// axis, mirror boundary) is applied to the float source *before* the
/// nearest sampling. The CCTF decode happens after that downsample,
/// inside `colour.RGB_to_XYZ` — never on the full source — so `cctf`
/// decodes the sampled preview pixels here.
///
/// `method` selects the metering pattern: `average`, `median`,
/// `center_weighted`, `partial`, `matrix`, `multi_zone`,
/// `highlight_weighted`. Anything else meters a flat 1.0 (0 EV), matching
/// the Python `else` branch.
///
/// The whole chain — preview, Y matrix, meter, `/0.184`, `log2` EV —
/// stays f64 like Python's float64 pipeline; only GPU boundaries round
/// to f32.
pub fn measure_autoexposure_ev(
    image: &ImageBuf,
    rgb_to_xyz: &[[f64; 3]; 3],
    cctf: Option<colorspace::Cctf>,
    method: &str,
) -> f64 {
    const MAX_SIZE: usize = 256;
    let w = image.width as usize;
    let h = image.height as usize;
    let max_dim = w.max(h);
    // Python `small_preview`: `rescale(order=0)` with scale
    // MAX_SIZE/max_dim. Each output pixel takes the nearest antialiased
    // source via `floor((o+0.5)·src/out)` (`ndi.zoom(grid_mode=True,
    // order=0)`); skimage rounds each axis independently, so the effective
    // per-axis factor is src_dim/out_dim — which differs from the global
    // scale on the short edge (e.g. 200/171 vs 300/256). Long edge ≤ 256
    // keeps the source itself as the preview.
    let (preview, sw, sh) = if max_dim > MAX_SIZE {
        let scale = MAX_SIZE as f64 / max_dim as f64;
        spektrafilm_math::resize::rescale_nearest0(image, scale)
            .expect("small_preview scale is 256/max_dim with max_dim > 256")
    } else {
        (
            image.data.iter().map(|&v| to_f64(v)).collect::<Vec<f64>>(),
            w,
            h,
        )
    };

    // Downsampled luminance grid (row-major, sw × sh): decode the sampled
    // pixels, then Y = rgb_to_xyz[1] · rgb, all f64.
    let decode = |v: f64| cctf.map_or(v, |c| colorspace::cctf_decode(v, c));
    let m = rgb_to_xyz[1];
    let lum: Vec<f64> = preview
        .par_chunks_exact(3)
        .map(|px| {
            let (r, g, b) = (decode(px[0]), decode(px[1]), decode(px[2]));
            m[0] * r + m[1] * g + m[2] * b
        })
        .collect();

    let metered = meter_luminance(&lum, sw, sh, method);
    let exposure = metered / 0.184;
    // Upstream final lines: `ev = -np.log2(exposure)`; only an infinite
    // EV clamps to 0.0 (with a warning) — a negative exposure yields NaN
    // and propagates, exactly like `np.log2`.
    let ev = -exposure.log2();
    if ev.is_infinite() {
        tracing::warn!(target: "spektrafilm_core::stages::filming", "Autoexposure is Inf. Setting autoexposure compensation to 0 EV.");
        return 0.0;
    }
    tracing::info!(
        target: "spektrafilm_core::stages::filming",
        sw = sw,
        sh = sh,
        method = method,
        metered = metered,
        exposure_div_184 = exposure,
        ev = ev,
        "autoexposure"
    );
    ev
}

/// Meter the auto-exposure EV for `image` using the input color space
/// and metering method configured in `params`. Python meters on the
/// full input in `_preprocess`, before `crop_and_rescale`; the ≤256 px
/// preview downsample and the CCTF decode both happen inside the meter
/// (decode after the preview, like upstream).
pub fn meter_autoexposure_ev(image: &ImageBuf, params: &RuntimeParams) -> f64 {
    let space =
        colorspace::resolve(&params.io.input_color_space).expect("validated input color space");
    let rgb_to_xyz = space.matrix_rgb_to_xyz;
    let cctf = params.io.input_cctf_decoding.then_some(space.cctf);
    measure_autoexposure_ev(
        image,
        &rgb_to_xyz,
        cctf,
        &params.camera.auto_exposure_method,
    )
}

/// Normalized pixel coordinate along an axis: `(i/dim - 0.5) * (dim/maxdim)`,
/// so the long edge spans [-0.5, 0.5] (Python `_normalized_coords`).
fn norm_coord(i: usize, dim: usize, max_dim: usize) -> f64 {
    (i as f64 / dim as f64 - 0.5) * (dim as f64 / max_dim as f64)
}

/// Meter the downsampled luminance grid with the named pattern, returning the
/// mean luminance the autoexposure should map to mid-grey (0.184).
fn meter_luminance(lum: &[f64], sw: usize, sh: usize, method: &str) -> f64 {
    let max_dim = sw.max(sh);
    match method {
        "average" => lum.iter().sum::<f64>() / lum.len() as f64,

        "median" => {
            let mut sorted = lum.to_vec();
            sorted.sort_by(|a, b| a.total_cmp(b));
            let n = sorted.len();
            if n % 2 == 1 {
                sorted[n / 2]
            } else {
                0.5 * (sorted[n / 2 - 1] + sorted[n / 2])
            }
        }

        "center_weighted" => {
            let sigma = 0.2f64;
            let inv_2sigma2 = 1.0 / (2.0 * sigma * sigma);
            let mut weighted = 0.0;
            let mut total = 0.0;
            for y in 0..sh {
                let ny = norm_coord(y, sh, max_dim);
                let ny2 = ny * ny;
                for x in 0..sw {
                    let nx = norm_coord(x, sw, max_dim);
                    let w = (-(nx * nx + ny2) * inv_2sigma2).exp();
                    weighted += lum[y * sw + x] * w;
                    total += w;
                }
            }
            weighted / total
        }

        "partial" => {
            // Hard circular region, ~15% radius (Canon Partial).
            let mut sum = 0.0;
            let mut count = 0usize;
            for y in 0..sh {
                let ny = norm_coord(y, sh, max_dim);
                for x in 0..sw {
                    let nx = norm_coord(x, sw, max_dim);
                    if (nx * nx + ny * ny).sqrt() < 0.15 {
                        sum += lum[y * sw + x];
                        count += 1;
                    }
                }
            }
            if count == 0 {
                lum.iter().sum::<f64>() / lum.len() as f64
            } else {
                sum / count as f64
            }
        }

        "matrix" => {
            // 5×5 grid; each cell weighted by a raised-cosine of its distance
            // from centre so corner zones contribute less.
            let (n_rows, n_cols) = (5usize, 5usize);
            let cell_h = sh / n_rows;
            let cell_w = sw / n_cols;
            let mut means = Vec::with_capacity(n_rows * n_cols);
            let mut weights = Vec::with_capacity(n_rows * n_cols);
            for r in 0..n_rows {
                for c in 0..n_cols {
                    if cell_h == 0 || cell_w == 0 {
                        continue;
                    }
                    let mut sum = 0.0;
                    for yy in r * cell_h..(r + 1) * cell_h {
                        for xx in c * cell_w..(c + 1) * cell_w {
                            sum += lum[yy * sw + xx];
                        }
                    }
                    means.push(sum / (cell_h * cell_w) as f64);
                    let dy = (r as f64 - (n_rows - 1) as f64 / 2.0) / ((n_rows - 1) as f64 / 2.0);
                    let dx = (c as f64 - (n_cols - 1) as f64 / 2.0) / ((n_cols - 1) as f64 / 2.0);
                    let dist = (dx * dx + dy * dy).sqrt() / 2.0f64.sqrt();
                    weights.push(0.5 * (1.0 + (std::f64::consts::PI * dist).cos()));
                }
            }
            let wsum: f64 = weights.iter().sum();
            means.iter().zip(&weights).map(|(m, w)| m * w / wsum).sum()
        }

        "multi_zone" => {
            // Three concentric rings weighted 50/30/20.
            let rings = [(0.00, 0.05, 0.50), (0.05, 0.25, 0.30), (0.25, 0.50, 0.20)];
            let mut weighted_sum = 0.0;
            let mut weight_total = 0.0;
            for &(r_min, r_max, weight) in &rings {
                let mut sum = 0.0;
                let mut count = 0usize;
                for y in 0..sh {
                    let ny = norm_coord(y, sh, max_dim);
                    for x in 0..sw {
                        let nx = norm_coord(x, sw, max_dim);
                        let radius = (nx * nx + ny * ny).sqrt();
                        if radius >= r_min && radius < r_max {
                            sum += lum[y * sw + x];
                            count += 1;
                        }
                    }
                }
                if count == 0 {
                    continue;
                }
                weighted_sum += weight * (sum / count as f64);
                weight_total += weight;
            }
            if weight_total > 0.0 {
                weighted_sum / weight_total
            } else {
                lum.iter().sum::<f64>() / lum.len() as f64
            }
        }

        "highlight_weighted" => {
            // Bias toward bright pixels (weight = Y²) to protect highlights.
            let mut weighted = 0.0;
            let mut total = 0.0;
            for &y in lum {
                let w = y * y;
                weighted += y * w;
                total += w;
            }
            if total < 1e-12 {
                lum.iter().sum::<f64>() / lum.len() as f64
            } else {
                weighted / total
            }
        }

        // Python's `else` branch sets `exposure = 1.0` (already post-`/0.184`),
        // i.e. 0 EV. Returning the mid-grey target makes that division cancel.
        _ => 0.184,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::precision::from_f64;

    /// Deterministic synthetic image, mirrored bit-for-bit in the Python
    /// reference: a 300×200 gradient where each channel is
    /// `((x*7 + y*13 + c*29) % 100) / 100`.
    fn synthetic_image() -> ImageBuf {
        let (w, h) = (300usize, 200usize);
        let mut data = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                for c in 0..3 {
                    let v = ((x * 7 + y * 13 + c * 29) % 100) as f64 / 100.0;
                    data.push(from_f64(v));
                }
            }
        }
        ImageBuf {
            width: w as u32,
            height: h as u32,
            data,
        }
    }

    /// Every metering pattern matches the upstream `measure_autoexposure_ev`
    /// (`spektrafilm/utils/autoexposure.py`) on the synthetic image, with the
    /// sRGB RGB→XYZ matrix and `apply_cctf_decoding=False`. The reference is
    /// metered on the ≤256 px `small_preview` downsample — the faithful
    /// pipeline order (downsample, then meter), which makes our nearest-edge
    /// mapping bit-exact with `skimage.transform.rescale(order=0)`.
    #[test]
    fn autoexposure_methods_match_python_reference() {
        let img = synthetic_image();
        let rgb_to_xyz = colorspace::resolve("sRGB")
            .expect("registered space")
            .matrix_rgb_to_xyz;
        let cases = [
            ("average", -1.427705567203),
            ("median", -1.308011314552),
            ("center_weighted", -1.427664142652),
            ("partial", -1.425832251352),
            ("matrix", -1.427803049578),
            ("multi_zone", -1.439398829551),
            ("highlight_weighted", -1.783882472180),
        ];
        for (method, expected) in cases {
            let ev = measure_autoexposure_ev(&img, &rgb_to_xyz, None, method);
            assert!(
                (ev - expected).abs() < 1e-5,
                "method {method}: got {ev}, expected {expected}",
            );
        }
    }

    /// An unknown method name meters a flat 1.0 → 0 EV, matching the Python
    /// `else` branch (`exposure = 1.0`).
    #[test]
    fn autoexposure_unknown_method_is_zero_ev() {
        let img = synthetic_image();
        let rgb_to_xyz = colorspace::resolve("sRGB")
            .expect("registered space")
            .matrix_rgb_to_xyz;
        let ev = measure_autoexposure_ev(&img, &rgb_to_xyz, None, "bogus");
        assert_eq!(ev, 0.0);
    }
}
