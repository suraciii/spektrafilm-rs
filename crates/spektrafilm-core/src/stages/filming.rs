/// Filming stage: expose the digital image onto virtual film.
///
/// Full Hanatos2025 path: RGB → XYZ → xy chromaticity → tc coordinates →
/// 2D LUT lookup (spectra × sensitivity) → per-channel film raw exposure.
use rayon::prelude::*;
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f32, from_f64, to_f64};
use spektrafilm_math::spectral::TcLut;
use std::time::Instant;

use crate::params::RuntimeParams;
use crate::profile::Profile;

fn stage_timings_enabled() -> bool {
    std::env::var_os("SPEKTRAFILM_STAGE_TIMINGS").is_some()
}

fn print_stage_timing(enabled: bool, stage: &str, start: Instant) {
    if enabled {
        eprintln!("stage {stage}: {} ms", start.elapsed().as_millis());
    }
}

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
        tracing::warn!(
            "Autoexposure is Inf. Setting autoexposure compensation to 0 EV."
        );
        return 0.0;
    }
    tracing::info!(
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
    let space = colorspace::resolve(&params.io.input_color_space).expect("validated input color space");
    let rgb_to_xyz = space.matrix_rgb_to_xyz;
    let cctf = params.io.input_cctf_decoding.then_some(space.cctf);
    measure_autoexposure_ev(image, &rgb_to_xyz, cctf, &params.camera.auto_exposure_method)
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

/// Per-pixel `raw = m · rgb` for the Mallett2019 path (f64 matmul, rayon-parallel).
fn apply_mallett_matrix(image: &ImageBuf, m: &[[f64; 3]; 3]) -> ImageBuf {
    let mut out = image.clone();
    out.data
        .par_chunks_exact_mut(3)
        .zip(image.data.par_chunks_exact(3))
        .for_each(|(dst, src)| {
            let rgb = [src[0] as f64, src[1] as f64, src[2] as f64];
            let v = crate::mallett::apply(m, rgb);
            dst[0] = from_f64(v[0]);
            dst[1] = from_f64(v[1]);
            dst[2] = from_f64(v[2]);
        });
    out
}

/// Expose: convert RGB to film raw exposure.
///
/// Dispatches on the configured upsampler: the Mallett2019 per-pixel matrix
/// (when `mallett_core` is set), the full Hanatos2025 spectral path (when a
/// TC LUT is provided), or a simplified RGB → log10 fallback.
///
/// `pixel_size_um` is the working-geometry pitch derived by
/// [`crate::resizing::crop_and_rescale`] from the full (pre-crop)
/// image; `ae_ev` is the auto-exposure EV metered on the full input
/// (Python meters before crop/resize and scales the encoded source in
/// `_preprocess`; `Pipeline::apply_autoexposure` owns that multiply, so
/// this in-stage scaling stays a no-op for pipeline callers).
#[allow(clippy::too_many_arguments)]
pub fn expose(
    image: &ImageBuf,
    _film: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    tc_lut: Option<&TcLut>,
    mallett_core: Option<&[[f64; 3]; 3]>,
    front_illuminant: &[f32],
    bw_filming_correction: f64,
    pixel_size_um: f64,
    ae_ev: f64,
) -> ImageBuf {
    let pix_um = pixel_size_um as f32;
    let input_space = colorspace::resolve(&params.io.input_color_space)
        .expect("input color space must be validated before filming");

    // Auto-exposure EV is metered upstream on the full input (the meter
    // previews/decodes internally in `small_preview` order).
    let mut rgb = image.clone();
    if params.io.input_cctf_decoding {
        rgb.data.par_iter_mut().for_each(|v| {
            *v = from_f64(colorspace::cctf_decode(to_f64(*v), input_space.cctf));
        });
    }

    // Auto-exposure scales linear irradiance after metering decoded luminance.
    if params.camera.auto_exposure {
        let scale = from_f64(2.0f64.powf(ae_ev));
        rgb.data.par_iter_mut().for_each(|v| *v *= scale);
    }

    // RGB → film raw exposure
    let mut raw = if let Some(core) = mallett_core {
        // Mallett2019: per-pixel `raw = (core · M_cs) · rgb`, a single 3×3
        // matrix folding the input-colour-space → linear-sRGB conversion.
        let m = crate::mallett::film_matrix(core, &params.io.input_color_space);
        apply_mallett_matrix(&rgb, &m)
    } else if let Some(lut) = tc_lut {
        // Full Hanatos2025 spectral upsampling with CAT02 adaptation.
        backend.hanatos2025_rgb_to_raw(
            &rgb,
            lut,
            &params.io.input_color_space,
            front_illuminant,
            params.settings.use_cat16,
        )
    } else {
        // Simplified fallback: treat RGB values as proportional to raw exposure
        rgb.clone()
    };

    // Python applies camera compensation to film raw, before optical effects.
    let exp_comp = from_f32(2.0f32.powf(params.camera.exposure_compensation_ev));
    raw.data.par_iter_mut().for_each(|v| *v *= exp_comp);

    // Order mirrors Python filming: boost → diffusion → lens_blur → halation.

    // Highlight boost: reconstruct pre-clip highlight irradiance before the
    // optical-scatter effects. No-op when boost_ev == 0.
    let hal_boost = &params.film_render.halation;
    if hal_boost.boost_ev != 0.0 {
        raw = spektrafilm_model::diffusion::boost_highlights(
            &raw,
            hal_boost.boost_ev as f64,
            hal_boost.boost_range as f64,
            hal_boost.protect_ev as f64,
        );
    }

    // Diffusion filter (camera): lens diffusion-filter PSF on linear raw.
    // Preview and export use the exact sampled PSF convolution.
    let df = &params.camera.diffusion_filter;
    if df.active {
        let dm = df.to_model();
        // An invalid family is rejected by `RuntimeParams::validate` /
        // `Pipeline::new_with_spectral` before any stage runs; reaching
        // this point with an unknown family is a programming error.
        raw = if backend.is_gpu() {
            spektrafilm_model::diffusion::apply_diffusion_filter_blur(
                &raw,
                &dm,
                pixel_size_um,
                backend,
            )
            .expect("camera diffusion filter family validated at pipeline entry")
        } else {
            spektrafilm_model::diffusion::apply_diffusion_filter_um(&raw, &dm, pixel_size_um)
                .expect("camera diffusion filter family validated at pipeline entry")
        };
    }

    // Lens blur
    if params.camera.lens_blur_um > 0.0 {
        raw = spektrafilm_model::diffusion::apply_gaussian_blur_um(
            &raw,
            params.camera.lens_blur_um,
            pix_um,
            backend,
        );
    }

    // Halation (on linear raw)
    let halation = &params.film_render.halation;
    if halation.active {
        raw = spektrafilm_model::diffusion::apply_halation_um(
            &raw,
            pix_um,
            halation.scatter_amount,
            halation.scatter_spatial_scale,
            halation.scatter_core_um,
            halation.scatter_tail_um,
            halation.scatter_tail_weight,
            halation.halation_amount,
            halation.halation_spatial_scale,
            halation.halation_strength,
            halation.halation_first_sigma_um,
            halation.halation_n_bounces,
            halation.halation_bounce_decay,
            halation.halation_renormalize,
            backend,
        );
    }

    // B&W / slide scanner exposure correction (Python applies this last,
    // before the log10). No-op at factor 1.0.
    if bw_filming_correction != 1.0 {
        let f = from_f64(bw_filming_correction);
        raw.data.par_iter_mut().for_each(|v| *v *= f);
    }

    // Convert to log10 exposure. Mirror Python's
    // `np.log10(np.fmax(raw, 0.0) + 1e-10)` exactly — adding 1e-10
    // after the floor-at-zero shifts every value by a constant in
    // log-space and is *not* the same as `log10(max(raw, 1e-10))`.
    let zero = from_f64(0.0);
    let eps = from_f64(1e-10);
    raw.data.par_iter_mut().for_each(|v| {
        *v = ((*v).max(zero) + eps).log10();
    });

    raw
}

/// Develop: log_raw → density_cmy via density curves + DIR couplers + grain.
///
/// `pixel_size_um` is the working-geometry pitch from
/// [`crate::resizing::crop_and_rescale`] (Python's
/// `ResizingService.pixel_size_um`).
pub fn develop(
    log_raw: &ImageBuf,
    film: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    pixel_size_um: f64,
) -> ImageBuf {
    let stage_timings = stage_timings_enabled();
    let pix_um = pixel_size_um as f32;
    // f64 chain for Python parity — curves are f64 in the profile JSON.
    let log_exposure_f64 = film.log_exposure_f64();
    let density_curves_f64 = film.density_curves_f64();
    let gamma = params.film_render.density_curve_gamma;

    // Filming.develop uses NORMALIZED curves (Python `develop` subtracts nanmin).
    let norm_curves_f64 =
        spektrafilm_model::density_curves::normalize_density_curves_f64(&density_curves_f64);
    let t = Instant::now();
    let mut density_cmy =
        backend.density_curve_interp(log_raw, &log_exposure_f64, &norm_curves_f64, gamma as f64);
    print_stage_timing(stage_timings, "filming_develop.density_interp", t);

    // DIR couplers
    let dir = &params.film_render.dir_couplers;
    if dir.active {
        let t = Instant::now();
        let matrix = spektrafilm_model::couplers::compute_dir_couplers_matrix(
            dir.gamma_samelayer_rgb,
            dir.gamma_interlayer_r_to_gb,
            dir.gamma_interlayer_g_to_rb,
            dir.gamma_interlayer_b_to_rg,
            dir.inhibition_samelayer,
            dir.inhibition_interlayer,
        );
        density_cmy = spektrafilm_model::couplers::apply_density_correction(
            &density_cmy,
            log_raw,
            pix_um,
            &log_exposure_f64,
            &density_curves_f64,
            &matrix,
            dir.amount,
            dir.diffusion_size_um,
            dir.diffusion_tail_um,
            dir.diffusion_tail_weight,
            film.is_positive(),
            gamma,
            dir.langmuir_donor_k_rgb,
            dir.langmuir_receiver_k_rgb,
            backend,
        );
        print_stage_timing(stage_timings, "filming_develop.dir_couplers", t);
    }

    // Grain — the experimental contract always uses the profile's three
    // emulsion sublayers. Runtime-only compatibility flags are not GUI
    // controls and do not alter this path.
    let grain = &params.film_render.grain;
    if grain.active {
        let t = Instant::now();
        let norm_curves_f64 = spektrafilm_model::density_curves::normalize_density_curves_f64(
            &film.density_curves_f64(),
        );
        let layers_tensor = film.density_curves_layers_f64();
        assert!(
            !layers_tensor.is_empty(),
            "grain requires the film profile to provide density_curves_layers \
             ([n][3 sublayers][3 channels]); profile '{}' has none",
            film.info.stock.as_deref().unwrap_or("<unnamed>"),
        );
        let density_cmy_layers =
            spektrafilm_model::density_curves::interp_density_cmy_layers(
                &density_cmy,
                &norm_curves_f64,
                &layers_tensor,
                film.is_positive(),
            );
        let density_max_layers =
            spektrafilm_model::density_curves::density_max_layers_f64(&layers_tensor);
        assert!(
            density_max_layers.iter().all(|row| row.iter().all(|&v| v > 0.0)),
            "grain requires positive per-sublayer density maxima; profile '{}' yields zero maxima",
            film.info.stock.as_deref().unwrap_or("<unnamed>"),
        );
        let particle_area_um2 = spektrafilm_model::grain::particle_area_from_rms_granularity(
            &layers_tensor,
            &density_max_layers,
            grain.density_min,
            grain.uniformity,
            grain.rms_granularity,
            grain.particle_scale_sublayers,
        );
        density_cmy = spektrafilm_model::grain::apply_grain_to_density_layers(
            &density_cmy_layers,
            &density_max_layers,
            density_cmy.width,
            density_cmy.height,
            pixel_size_um,
            particle_area_um2,
            grain.particle_scale_sublayers,
            grain.density_min,
            grain.uniformity,
            grain.blur,
            grain.blur_dye_clouds_um,
            grain.micro_structure,
            grain.mult_usm_sigma,
            grain.mult_usm_amount,
            grain.monochrome,
            params.settings.use_fast_stats,
            params.random_seed,
            backend,
        );
        print_stage_timing(stage_timings, "filming_develop.grain", t);
    }

    density_cmy
}

/// Full filming stage: expose + develop on an already-prepared working
/// image. Geometry (crop/upscale) is the caller's responsibility — see
/// [`crate::resizing::crop_and_rescale`]; the pixel pitch passed here
/// must be the one derived from the full input.
pub fn process(
    image: &ImageBuf,
    film: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    tc_lut: Option<&TcLut>,
    pixel_size_um: f64,
) -> ImageBuf {
    let ref_illuminant = select_illuminant(&film.info.reference_illuminant);
    let ae_ev = meter_autoexposure_ev(image, params);
    let log_raw = expose(
        image,
        film,
        params,
        backend,
        tc_lut,
        None,
        &ref_illuminant,
        1.0,
        pixel_size_um,
        ae_ev,
    );
    develop(&log_raw, film, params, backend, pixel_size_um)
}

use crate::spectral_service::select_illuminant;

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
        let rgb_to_xyz = colorspace::resolve("sRGB").expect("registered space").matrix_rgb_to_xyz;
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
        let rgb_to_xyz = colorspace::resolve("sRGB").expect("registered space").matrix_rgb_to_xyz;
        let ev = measure_autoexposure_ev(&img, &rgb_to_xyz, None, "bogus");
        assert_eq!(ev, 0.0);
    }

    #[cfg(test)]
    mod grain_dispatch {
        use super::super::*;
        use crate::params::RuntimeParams;
        use crate::profile::{self, Profile};
        use spektrafilm_gpu::cpu_backend::CpuBackend;
        use spektrafilm_math::image::ImageBuf;
        use spektrafilm_math::precision::{Scalar, from_f64, to_f64};

        fn data_dir() -> std::path::PathBuf {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("data")
        }

        fn portra() -> Option<Profile> {
            let path = data_dir().join("profiles/kodak_portra_400.json");
            if !path.exists() {
                eprintln!("Skipping test — profile not found at {}", path.display());
                return None;
            }
            Some(profile::load_profile(&path).unwrap())
        }

        /// Small log-exposure image spread across the curve axis.
        fn log_raw() -> ImageBuf {
            let (w, h) = (16usize, 12usize);
            let mut data = Vec::with_capacity(w * h * 3);
            for y in 0..h {
                for x in 0..w {
                    let t = (x + y * w) as f64 / (w * h) as f64;
                    data.push(from_f64(-3.5 + 3.0 * t));
                    data.push(from_f64(-3.2 + 3.0 * t));
                    data.push(from_f64(-2.9 + 3.0 * t));
                }
            }
            ImageBuf {
                width: w as u32,
                height: h as u32,
                data,
            }
        }

        fn base_params() -> RuntimeParams {
            let mut params = RuntimeParams::default();
            params.film_render.dir_couplers.active = false;
            params.film_render.halation.active = false;
            // The small test images would otherwise imply ~2 mm pixels and
            // wash the particle noise out; 0.192 mm over the 16 px long
            // edge gives a realistic 12 µm pixel.
            params.camera.film_format_mm = 0.192;
            params
        }

        fn ch_mean(img: &ImageBuf, ch: usize) -> f64 {
            img.pixels().map(|px| to_f64(px[ch])).sum::<f64>() / img.pixel_count() as f64
        }

        /// Grain off: develop must return the plain density interpolation
        /// (deterministic LUT-mode bypass — Python's `lut_mode` forces
        /// `grain.active = False`, and the non-spatial film behavior is
        /// untouched).
        #[test]
        fn grain_disabled_is_density_identity() {
            let Some(film) = portra() else { return };
            let backend = CpuBackend;
            let mut params = base_params();
            params.film_render.grain.active = false;
            let log_raw = log_raw();

            let out = develop(&log_raw, &film, &params, &backend, 12.0);

            let norm = spektrafilm_model::density_curves::normalize_density_curves_f64(
                &film.density_curves_f64(),
            );
            let expected = backend.density_curve_interp(
                &log_raw,
                &film.log_exposure_f64(),
                &norm,
                params.film_render.density_curve_gamma as f64,
            );
            assert_eq!(out.data, expected.data);
        }

        #[test]
        fn canonical_grain_groups_change_rendered_output() {
            let Some(film) = portra() else { return };
            let backend = CpuBackend;
            let log_raw = log_raw();
            let mut baseline_params = base_params();
            baseline_params.film_render.grain.blur = 0.0;
            baseline_params.film_render.grain.mult_usm_sigma = 0.0;
            baseline_params.film_render.grain.mult_usm_amount = 0.0;
            baseline_params.film_render.grain.blur_dye_clouds_um = 0.0;
            baseline_params.film_render.grain.micro_structure = [0.0, 0.0];
            let baseline = develop(&log_raw, &film, &baseline_params, &backend, 12.0);

            let mut rms = baseline_params.clone();
            rms.film_render.grain.rms_granularity = [18.0, 20.0, 22.0];
            assert_ne!(
                baseline.data,
                develop(&log_raw, &film, &rms, &backend, 12.0).data,
                "RMS granularity must affect grain output"
            );

            let mut statistics = baseline_params.clone();
            statistics.film_render.grain.density_min = [0.2, 0.21, 0.22];
            statistics.film_render.grain.uniformity = [0.7, 0.72, 0.74];
            statistics.film_render.grain.particle_scale_sublayers = [1.5, 0.75, 0.35];
            assert_ne!(
                baseline.data,
                develop(&log_raw, &film, &statistics, &backend, 12.0).data,
                "pixel statistics must affect grain output"
            );

            let mut texture = baseline_params.clone();
            texture.film_render.grain.blur = 1.1;
            texture.film_render.grain.mult_usm_sigma = 0.8;
            texture.film_render.grain.mult_usm_amount = 1.7;
            assert_ne!(
                baseline.data,
                develop(&log_raw, &film, &texture, &backend, 12.0).data,
                "texture controls must affect grain output"
            );

            let mut micro = baseline_params;
            micro.film_render.grain.blur_dye_clouds_um = 3.0;
            micro.film_render.grain.micro_structure = [0.6, 40.0];
            assert_ne!(
                baseline.data,
                develop(&log_raw, &film, &micro, &backend, 12.0).data,
                "micro substructure controls must affect grain output"
            );
        }

        /// Runtime-only compatibility flags cannot create a second grain
        /// model; the experimental route is always the three-sublayer path.
        #[test]
        fn runtime_only_layer_flags_do_not_change_experimental_output() {
            let Some(film) = portra() else { return };
            let backend = CpuBackend;
            let log_raw = log_raw();
            let mut params = base_params();
            let baseline = develop(&log_raw, &film, &params, &backend, 12.0);
            params.film_render.grain.sublayers_active = false;
            params.film_render.grain.n_sub_layers = 4;
            let migrated = develop(&log_raw, &film, &params, &backend, 12.0);
            assert_eq!(baseline.data, migrated.data);
        }

        /// `n_sub_layers` is a migration-only field and must not alter the
        /// canonical experimental grain route.
        #[test]
        fn n_sub_layers_is_ignored_by_experimental_route() {
            let Some(film) = portra() else { return };
            let backend = CpuBackend;
            let log_raw = log_raw();
            let mut params = base_params();
            params.film_render.grain.n_sub_layers = 1;
            let one = develop(&log_raw, &film, &params, &backend, 12.0);
            params.film_render.grain.n_sub_layers = 4;
            let four = develop(&log_raw, &film, &params, &backend, 12.0);
            assert_eq!(one.data, four.data);
        }

        /// `use_fast_stats` switches the layered sampler's RNG regime:
        /// different texture, same statistical center (bounded comparison
        /// — bit parity is impossible for upstream's numba kernels).
        #[test]
        fn use_fast_stats_switches_regime() {
            let Some(film) = portra() else { return };
            let backend = CpuBackend;
            let log_raw = log_raw();
            let mut params = base_params();
            params.film_render.grain.blur = 0.0; // raw particle field
            params.film_render.grain.blur_dye_clouds_um = 0.0;

            params.settings.use_fast_stats = false;
            let scipy = develop(&log_raw, &film, &params, &backend, 12.0);
            params.settings.use_fast_stats = true;
            let fast = develop(&log_raw, &film, &params, &backend, 12.0);
            assert_ne!(scipy.data, fast.data);
            for ch in 0..3 {
                assert!(
                    (ch_mean(&scipy, ch) - ch_mean(&fast, ch)).abs() < 0.1,
                    "ch {ch}: {} vs {}",
                    ch_mean(&scipy, ch),
                    ch_mean(&fast, ch)
                );
            }
        }

        /// Positive colour profiles reach the layered path through the
        /// negated-axis branch of `interp_density_cmy_layers` (upstream
        /// `-density_cmy` / `-density_curves` lookup): output stays finite
        /// and the negated lookup decorrelates it from the negative-profile
        /// run of the same input.
        #[test]
        fn layered_grain_positive_profile_runs_negated_axis() {
            let path = data_dir().join("profiles/fujifilm_provia_100f.json");
            if !path.exists() {
                eprintln!("Skipping test — profile not found at {}", path.display());
                return;
            }
            let film = profile::load_profile(&path).unwrap();
            assert!(film.is_positive());
            let backend = CpuBackend;
            let params = base_params();
            let log_raw = log_raw();

            let out = develop(&log_raw, &film, &params, &backend, 12.0);
            let again = develop(&log_raw, &film, &params, &backend, 12.0);
            assert!(out.data.iter().all(|v| v.is_finite()));
            assert_eq!(out.data, again.data);
            // Densities came out of the curve tables (not the raw log
            // exposure), and grain actually fired.
            assert_ne!(out.data, log_raw.data);
        }

        /// Layered grain on a profile without layer curves fails with an
        /// actionable message instead of producing garbage — Python's
        /// `np.nanmax` on the empty tensor raises for the same condition.
        #[test]
        #[should_panic(expected = "grain requires the film profile")]
        fn sublayers_without_layer_curves_fails() {
            let Some(mut film) = portra() else { return };
            film.data.density_curves_layers.clear();
            let backend = CpuBackend;
            let params = base_params();
            let _ = develop(&log_raw(), &film, &params, &backend, 12.0);
        }

        /// A monochrome (B&W) film runs the layered path with one shared
        /// noise field: identical per-channel input stays identical after
        /// grain (regression coverage for the Rust-only B&W addition; not
        /// part of the 0.3.4 baseline).
        #[test]
        fn layered_grain_monochrome_keeps_channels_equal() {
            let path = data_dir().join("profiles/kodak_doublex.json");
            if !path.exists() {
                eprintln!("Skipping test — profile not found at {}", path.display());
                return;
            }
            let film = profile::resolve_for_render(
                profile::load_profile(&path).unwrap(),
                None,
            );
            let backend = CpuBackend;
            let mut params = base_params();
            // Mirror the B&W runtime layout: one shared noise field and
            // channel-0 values for every canonical per-channel tuple.
            params.film_render.grain.monochrome = true;
            let g = &mut params.film_render.grain;
            g.rms_granularity = [g.rms_granularity[0]; 3];
            g.density_min = [g.density_min[0]; 3];
            g.uniformity = [g.uniformity[0]; 3];
            g.blur = 0.0;
            g.blur_dye_clouds_um = 0.0;
            // 8 px long edge at 0.096 mm → 12 µm pixels.
            params.camera.film_format_mm = 0.096;

            // Identical per-channel log exposure → identical densities.
            let (w, h) = (8usize, 8usize);
            let mut data = Vec::with_capacity(w * h * 3);
            for i in 0..w * h {
                let t = i as f64 / (w * h) as f64;
                let v = from_f64(-3.0 + 2.5 * t);
                data.extend_from_slice(&[v, v, v]);
            }
            let log_raw = ImageBuf {
                width: w as u32,
                height: h as u32,
                data,
            };

            let out = develop(&log_raw, &film, &params, &backend, 12.0);
            let c0: Vec<Scalar> = out.pixels().map(|px| px[0]).collect();
            let c1: Vec<Scalar> = out.pixels().map(|px| px[1]).collect();
            let c2: Vec<Scalar> = out.pixels().map(|px| px[2]).collect();
            assert_eq!(c0, c1);
            assert_eq!(c0, c2);
        }
    }
}
