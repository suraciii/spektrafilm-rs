/// Scanning stage: convert print/film density back to RGB.
///
/// Dispatches the spectral integration to the GPU backend when available.
use rayon::prelude::*;
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::pchip3d::{pchip_interp, prepare_pchip_3d};
use spektrafilm_math::precision::{Scalar, from_f64, to_f64};

use super::build_lut_grid;
use crate::params::RuntimeParams;
use crate::profile::Profile;

/// Scanner LUT bounds. When scan_film=true: `-grain.density_min` to
/// `nanmax(film.density_curves)`. Else: `nanmin..nanmax` of
/// `print.density_curves`. Mirrors Python `ScanningStage._density_to_rgb`.
fn scanner_lut_bounds(profile: &Profile, params: &RuntimeParams) -> ([f64; 3], [f64; 3]) {
    let curves = profile.density_curves_f64();
    let mut dmin_curves = [f64::INFINITY; 3];
    let mut dmax_curves = [f64::NEG_INFINITY; 3];
    for row in &curves {
        for c in 0..3 {
            if !row[c].is_nan() {
                if row[c] < dmin_curves[c] {
                    dmin_curves[c] = row[c];
                }
                if row[c] > dmax_curves[c] {
                    dmax_curves[c] = row[c];
                }
            }
        }
    }
    let (data_min, data_max) = if params.io.scan_film {
        let gmin = params.film_render.grain.density_min;
        ([-gmin[0], -gmin[1], -gmin[2]], dmax_curves)
    } else {
        (dmin_curves, dmax_curves)
    };
    let mut dm = data_min;
    let mut dx = data_max;
    for c in 0..3 {
        if !dm[c].is_finite() {
            dm[c] = 0.0;
        }
        if !dx[c].is_finite() {
            dx[c] = 1.0;
        }
        if dx[c] <= dm[c] {
            dx[c] = dm[c] + 1.0;
        }
    }
    (dm, dx)
}

/// Build a 17³ log_xyz LUT (matching Python's `cmy_to_log_xyz`),
/// PCHIP-interpolate per pixel, then apply `10^log_xyz → CAT →
/// xyz_to_rgb` to get RGB. Glare is added after this returns in RGB
/// space — algebraically equivalent to Python's add_glare-in-XYZ
/// because the matrices are linear (see the glare note in
/// `scan_with_options` below).
///
/// This LUTs the *same intermediate quantity* Python LUTs
/// (`cmy_to_log_xyz`), not the post-`10^x`/post-matrix RGB output —
/// keeps the PCHIP interpolating the smoother log10-domain surface
/// for bit-identical parity with Python's LUT path.
#[allow(clippy::too_many_arguments)]
fn scan_spectral_via_lut(
    density_cmy: &ImageBuf,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    illuminant: &[f64],
    normalization: f64,
    cat: &[[f64; 3]; 3],
    xyz_to_rgb: &[[f64; 3]; 3],
    _backend: &dyn ComputeBackend,
    data_min: [f64; 3],
    data_max: [f64; 3],
    steps: usize,
    color_ref: &crate::color_reference::ColorReference,
) -> ImageBuf {
    let _lut = super::StageObservation::cpu(
        _backend,
        "scanner_lut",
        spektrafilm_gpu::telemetry::CpuReason::SpectralLut,
    );
    let grid = build_lut_grid(steps, data_min, data_max);
    // log_xyz LUT — same function Python LUTs.
    let lut_log_xyz = spektrafilm_gpu::cpu_backend::scan_log_xyz_cpu(
        &grid,
        channel_density,
        base_density,
        illuminant,
        normalization,
    );
    let prepared = prepare_pchip_3d(lut_log_xyz, steps);
    let scale = (steps - 1) as f64;
    let inv = [
        scale / (data_max[0] - data_min[0]),
        scale / (data_max[1] - data_min[1]),
        scale / (data_max[2] - data_min[2]),
    ];
    let mut out = density_cmy.clone();
    out.data
        .par_chunks_exact_mut(3)
        .zip(density_cmy.data.par_chunks_exact(3))
        .for_each(|(dst, src)| {
            let r = (src[0] as f64 - data_min[0]) * inv[0];
            let g = (src[1] as f64 - data_min[1]) * inv[1];
            let b = (src[2] as f64 - data_min[2]) * inv[2];
            let log_xyz = pchip_interp(&prepared, r, g, b);
            // Post-LUT: 10^log_xyz → CAT → xyz_to_rgb, mirroring Python's
            // `_density_to_rgb` AFTER the LUT call. bw_correction and
            // glare run after this returns (RGB space).
            let mut xyz = [
                10.0f64.powf(log_xyz[0]),
                10.0f64.powf(log_xyz[1]),
                10.0f64.powf(log_xyz[2]),
            ];
            // B&W/slide luminance remap on the pre-CAT scan XYZ (Python's
            // black_white_xyz_correction). Scalar scale from the Y channel;
            // no-op when color_ref has no remap.
            let scale = color_ref.xyz_scale(xyz[1]);
            if scale != 1.0 {
                xyz[0] *= scale;
                xyz[1] *= scale;
                xyz[2] *= scale;
            }
            let xa = cat[0][0] * xyz[0] + cat[0][1] * xyz[1] + cat[0][2] * xyz[2];
            let ya = cat[1][0] * xyz[0] + cat[1][1] * xyz[1] + cat[1][2] * xyz[2];
            let za = cat[2][0] * xyz[0] + cat[2][1] * xyz[1] + cat[2][2] * xyz[2];
            dst[0] =
                from_f64(xyz_to_rgb[0][0] * xa + xyz_to_rgb[0][1] * ya + xyz_to_rgb[0][2] * za);
            dst[1] =
                from_f64(xyz_to_rgb[1][0] * xa + xyz_to_rgb[1][1] * ya + xyz_to_rgb[1][2] * za);
            dst[2] =
                from_f64(xyz_to_rgb[2][0] * xa + xyz_to_rgb[2][1] * ya + xyz_to_rgb[2][2] * za);
        });
    out
}

/// Spectral integration of one density image under the scan colour context.
///
/// The faithful B&W/slide luminance remap (`color_ref.has_remap`) only exists
/// on the LUT path, so it forces that path; `settings.use_scanner_lut` selects
/// it otherwise. The bulk scan and the one-pixel reference samples share this
/// switch, so a new path condition cannot desynchronize them.
// Both scan paths share the complete spectral context.
#[allow(clippy::too_many_arguments)]
fn scan_spectral_rgb(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    color: &crate::chain_prep::ScanColorContext,
) -> ImageBuf {
    if color_ref.has_remap() || params.settings.use_scanner_lut {
        let (data_min, data_max) = scanner_lut_bounds(profile, params);
        scan_spectral_via_lut(
            density_cmy,
            channel_density,
            base_density,
            &color.illuminant,
            color.normalization,
            &color.adapt,
            &color.base_xyz_to_rgb,
            backend,
            data_min,
            data_max,
            params.settings.lut_resolution as usize,
            color_ref,
        )
    } else {
        backend.scan_spectral(
            density_cmy,
            channel_density,
            base_density,
            &color.illuminant,
            color.normalization,
            &color.adapt,
            &color.base_xyz_to_rgb,
        )
    }
}

fn positive_scan_requested(params: &RuntimeParams) -> bool {
    params.scanner.scan_output == "positive_scan"
}

struct PositiveChannel {
    density: Vec<f64>,
    log_exposure: Vec<f64>,
    clear: f64,
}

struct PositiveCalibration {
    channels: [PositiveChannel; 3],
}

impl PositiveCalibration {
    #[allow(clippy::too_many_arguments)]
    fn from_profile(
        profile: &Profile,
        params: &RuntimeParams,
        backend: &dyn ComputeBackend,
        color_ref: &crate::color_reference::ColorReference,
        channel_density: &[[f64; 3]],
        base_density: &[f64],
        color: &crate::chain_prep::ScanColorContext,
    ) -> Result<Self, String> {
        let axis = &profile.data.log_exposure;
        if axis.len() < 2 || profile.data.density_curves.len() != axis.len() {
            return Err("positive_scan requires matching profile calibration curves with at least two samples".into());
        }
        let rows = ImageBuf::from_data(
            axis.len() as u32,
            1,
            profile
                .data
                .density_curves
                .iter()
                .flat_map(|row| row.iter().copied().map(from_f64))
                .collect(),
        );
        let captured = scan_spectral_rgb(
            &rows,
            profile,
            params,
            backend,
            color_ref,
            channel_density,
            base_density,
            color,
        );
        let mut channels = Vec::with_capacity(3);
        for channel in 0..3 {
            let mut density: Vec<f64> = Vec::with_capacity(axis.len());
            let mut exposure = Vec::with_capacity(axis.len());
            let mut clear = None;
            let mut ended = false;
            for (i, &log_e) in axis.iter().enumerate() {
                let value = to_f64(captured.data[i * 3 + channel]);
                if !log_e.is_finite() || !value.is_finite() || value <= 0.0 {
                    ended |= !density.is_empty();
                    continue;
                }
                if ended {
                    return Err(format!(
                        "positive_scan calibration has an invalid interior sample in channel {channel}"
                    ));
                }
                let reference = *clear.get_or_insert(value);
                let d = -(value / reference).log10();
                density.push(d);
                exposure.push(log_e);
            }
            // Pool adjacent violations to regularize measurement noise in the
            // inverse calibration without changing the forward film curves.
            let mut blocks: Vec<(f64, f64, usize)> = Vec::with_capacity(density.len());
            for (&d, &e) in density.iter().zip(&exposure) {
                blocks.push((d, e, 1));
                while blocks.len() >= 2 {
                    let last = blocks.len() - 1;
                    let (left_sum, _, left_count) = blocks[last - 1];
                    let (right_sum, _, right_count) = blocks[last];
                    if left_sum / (left_count as f64) < right_sum / (right_count as f64) {
                        break;
                    }
                    let right = blocks.pop().unwrap();
                    let left = blocks.last_mut().unwrap();
                    left.0 += right.0;
                    left.1 += right.1;
                    left.2 += right.2;
                }
            }
            density.clear();
            exposure.clear();
            let tolerance = if std::mem::size_of::<Scalar>() == 4 {
                2e-6
            } else {
                1e-12
            };
            for (density_sum, exposure_sum, count) in blocks {
                let d = density_sum / count as f64;
                if density.last().is_some_and(|last| d - *last <= tolerance) {
                    continue;
                }
                density.push(d);
                exposure.push(exposure_sum / count as f64);
            }
            if density.len() < 2 {
                return Err(format!(
                    "positive_scan calibration has fewer than two positive distinct samples in channel {channel}"
                ));
            }
            channels.push(PositiveChannel {
                density,
                log_exposure: exposure,
                clear: clear.unwrap(),
            });
        }
        Ok(Self {
            channels: channels
                .try_into()
                .map_err(|_| "positive_scan calibration requires three channels")?,
        })
    }
}

impl PositiveChannel {
    fn estimate(&self, value: f64) -> f64 {
        let d = -(value / self.clear).log10();
        let upper = self
            .density
            .partition_point(|x| *x < d)
            .clamp(1, self.density.len() - 1);
        let lower = upper - 1;
        let fraction = (d - self.density[lower]) / (self.density[upper] - self.density[lower]);
        self.log_exposure[lower] + fraction * (self.log_exposure[upper] - self.log_exposure[lower])
    }
}

fn interpret_negative_capture(
    mut rgb: ImageBuf,
    calibration: &PositiveCalibration,
    params: &RuntimeParams,
) -> ImageBuf {
    let contrast = params.scanner.positive_contrast;
    let dmax = params.scanner.positive_dmax;
    let denominator = -(-std::f64::consts::LN_10 * dmax).exp_m1();
    let camera_ev = if params.workflow.route == "input > film > scan" {
        params.camera.exposure_compensation_ev as f64
    } else {
        0.0
    };
    let tone = |log_e: f64, channel: usize| {
        let e = log_e / std::f64::consts::LOG10_2 - camera_ev
            + params.scanner.positive_density_ev
            + params.scanner.positive_balance_ev[channel];
        let z = contrast * e;
        let t = if z >= 0.0 {
            1.0 / (1.0 + (-z).exp())
        } else {
            let a = z.exp();
            a / (1.0 + a)
        };
        let density = dmax * (1.0 - t);
        let positive = (-std::f64::consts::LN_10 * density).exp_m1();
        let output = (positive + denominator) / denominator;
        let derivative = std::f64::consts::LN_10
            * dmax
            * contrast
            * t
            * (1.0 - t)
            * (-std::f64::consts::LN_10 * density).exp()
            / (denominator * std::f64::consts::LOG10_2);
        (output, derivative)
    };
    let endpoints: [[(f64, f64, f64); 2]; 3] = std::array::from_fn(|c| {
        let k = &calibration.channels[c];
        let last = k.density.len() - 1;
        std::array::from_fn(|side| {
            let i = if side == 0 { 0 } else { last };
            let (a, b) = if side == 0 { (0, 1) } else { (last - 1, last) };
            let capture = k.clear * 10.0f64.powf(-k.density[i]);
            let (output, derivative) = tone(k.log_exposure[i], c);
            let slope = -derivative * (k.log_exposure[b] - k.log_exposure[a])
                / ((k.density[b] - k.density[a]) * std::f64::consts::LN_10 * capture);
            (capture, output, slope)
        })
    });
    rgb.data.par_chunks_exact_mut(3).for_each(|px| {
        for channel in 0..3 {
            let value = to_f64(px[channel]);
            let (clear, low, low_slope) = endpoints[channel][0];
            let (dense, high, high_slope) = endpoints[channel][1];
            let output = if value >= clear {
                low + (value - clear) * low_slope
            } else if value <= dense {
                high + (value - dense) * high_slope
            } else {
                tone(calibration.channels[channel].estimate(value), channel).0
            };
            px[channel] = from_f64(output);
        }
    });
    rgb
}

// Scan options preserve the existing stage interface.
#[allow(clippy::too_many_arguments)]
pub fn scan_with_options(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    gamut: &crate::gamut_compression::OutputGamutCompress,
    scan_illuminant: Option<&str>,
    include_base: bool,
) -> Result<ImageBuf, String> {
    let positive_scan = positive_scan_requested(params);
    let _positive_scan = positive_scan.then(|| {
        super::StageObservation::cpu(
            backend,
            "positive_scan",
            spektrafilm_gpu::telemetry::CpuReason::PositiveScanOutput,
        )
    });
    let cpu_fallback = positive_scan
        .then(|| {
            backend
                .observation_context()
                .filter(|context| context.enabled())
                .map(|context| {
                    spektrafilm_gpu::bind_cpu_fallback(
                        context.clone(),
                        spektrafilm_gpu::telemetry::CpuReason::PositiveScanOutput,
                    )
                })
        })
        .flatten();
    let backend = if positive_scan {
        static CPU_BACKEND: spektrafilm_gpu::cpu_backend::CpuBackend =
            spektrafilm_gpu::cpu_backend::CpuBackend;
        cpu_fallback
            .as_deref()
            .unwrap_or(&CPU_BACKEND as &dyn ComputeBackend)
    } else {
        backend
    };
    let channel_density = crate::chain_prep::channel_density(profile);
    let base_density = if include_base {
        std::borrow::Cow::Borrowed(profile.data.base_density.as_slice())
    } else {
        std::borrow::Cow::Owned(vec![0.0; profile.data.base_density.len()])
    };

    let output_space = colorspace::resolve(&params.io.output_color_space)
        .expect("output color space must be validated before scanning");
    let prepared = crate::chain_prep::PreparedChain::for_scan(profile, params, scan_illuminant);
    let scan_context = &prepared.scan;
    let base_density = base_density.as_ref();

    // Dispatch spectral integration to backend (GPU or CPU). The backend
    // applies the two matrices in sequence — we pass them separately.
    //
    // `use_scanner_lut` path: evaluate `scan_spectral` on a 17³ CMY
    // grid and PCHIP-interpolate per pixel. Glare is added in RGB
    // space after this returns (the existing post-step at the bottom
    // of this function), so swapping to the LUT path is glare-safe.
    // The path selection (including the faithful B&W/slide luminance
    // remap) lives in `scan_spectral_rgb` so the reference samples
    // below cannot diverge from this bulk scan.
    let mut rgb = scan_spectral_rgb(
        density_cmy,
        profile,
        params,
        backend,
        color_ref,
        &channel_density,
        base_density,
        scan_context,
    );

    // White/black correction is fully handled by color_ref's XYZ luminance
    // remap (applied inside the scan paths above). For the combos where no
    // remap is built we apply nothing: scan_film+negative because upstream
    // Python explicitly skips it (`black_white_xyz_correction`: "do not
    // correct negative film scans"), and a positive print paper because
    // upstream crashes there (its printing exposure correction returns None)
    // — unsupported, and unrepresentable with the shipped profiles anyway.

    // Viewing glare (Python: `add_glare(xyz, illuminant_xyz, glare)` between scan_spectral
    // and chromatic adapt + matrix). Since CAT+matrix is linear, we equivalently add
    // glare in RGB space by pre-multiplying the illuminant XYZ through the same matrix.
    //   Python: rgb = M @ (xyz + g*illu) = M@xyz + g*(M@illu)
    //   Rust:   rgb = M @ xyz; then rgb += g * (M@illu)
    //
    // Python 0.3.4 `ScanningStage._density_to_rgb` sets `glare = None` on the
    // `io.scan_film` path — direct-film scans get no viewing glare, and
    // `film_render.glare` is never read anywhere upstream. Only the print
    // path consumes `print_render.glare` (scan_film=false).
    let glare = (!params.io.scan_film)
        .then_some(&params.print_render.glare)
        .filter(|g| g.active && g.percent > 0.0);
    if let Some(glare) = glare {
        let _glare = super::StageObservation::cpu(
            backend,
            "glare",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        // Shared scan context keeps the CPU two-step CAT→RGB operation
        // order used by the reference path.
        let glare_rgb_offset_f64 = crate::chain_prep::glare_rgb_offset_f64(scan_context);
        let glare_rgb_offset: [Scalar; 3] = glare_rgb_offset_f64.map(from_f64);
        let glare_amount = spektrafilm_model::glare::compute_random_glare_amount(
            rgb.width,
            rgb.height,
            glare.percent,
            glare.roughness,
            glare.blur,
            params.random_seed,
        );
        spektrafilm_model::glare::add_glare_with_amount(&mut rgb, &glare_amount, glare_rgb_offset);
    }

    // Positive interpretation is scanner-domain: optical capture and lens
    // blur happen before the calibrated negative endpoint inversion.
    if positive_scan {
        if params.scanner.lens_blur > 0.0 {
            rgb = backend.gaussian_blur(&rgb, params.scanner.lens_blur);
        }
        let calibration = PositiveCalibration::from_profile(
            profile,
            params,
            backend,
            color_ref,
            &channel_density,
            base_density,
            scan_context,
        )?;
        rgb = interpret_negative_capture(rgb, &calibration, params);
    }

    // Output gamut compression — direct scans preserve the historical
    // capture order; positive scans compress only after interpretation.
    if gamut.is_active() {
        let _gamut = super::StageObservation::cpu(
            backend,
            "output_gamut",
            spektrafilm_gpu::telemetry::CpuReason::UnsupportedOutputGamut,
        );
        rgb.data.par_chunks_exact_mut(3).for_each(|px| {
            let out = gamut.compress([px[0] as f64, px[1] as f64, px[2] as f64]);
            px[0] = from_f64(out[0]);
            px[1] = from_f64(out[1]);
            px[2] = from_f64(out[2]);
        });
    }

    // Direct scans apply optical blur after gamut compression, preserving
    // the established direct-scan behavior.
    if !positive_scan && params.scanner.lens_blur > 0.0 {
        rgb = backend.gaussian_blur(&rgb, params.scanner.lens_blur);
    }

    // Unsharp mask
    let [usm_sigma, usm_amount] = params.scanner.unsharp_mask;
    if usm_sigma > 0.0 && usm_amount > 0.0 {
        rgb = spektrafilm_model::optics::apply_unsharp_mask(&rgb, usm_sigma, usm_amount, backend);
    }

    // Grain consumes native display-encoded RGB, with no internal transfer
    // or primaries conversion. The scanned image supplies its output space.
    let grain_v2_active = params.film_render.grain.active
        && params.settings.rgb_to_raw_method != "mallett2019"
        && matches!(
            params.film_render.grain.engine,
            crate::params::grain::GrainEngine::V2
        );
    // Magazine appearance is optional finishing, not a route: it applies to
    // any scanned base workflow after the scanner-domain effects.
    let magazine_active =
        params.magazine_print_color.active && params.magazine_print_color.strength > 0.0;
    let encoded_domain = params.io.output_cctf_encoding || grain_v2_active;
    if encoded_domain {
        let _encoding = super::StageObservation::cpu(
            backend,
            "output_transfer",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        rgb.data.par_chunks_exact_mut(3).for_each(|px| {
            let encoded =
                colorspace::encode_rgb([px[0] as f64, px[1] as f64, px[2] as f64], output_space);
            px[0] = from_f64(encoded[0]);
            px[1] = from_f64(encoded[1]);
            px[2] = from_f64(encoded[2]);
        });
    }
    if grain_v2_active {
        let grain_observation = super::StageObservation::new(backend, "grain_v2");
        let backend = grain_observation.backend(backend);
        let mut grain = params.film_render.grain.resolved_grain_v2();
        if params.debug.deactivate_spatial_effects {
            grain.resolution_factor = 100.0;
        }
        grain.seed = params.random_seed as u32;
        let gpu_params = params
            .film_render
            .grain
            .gpu_params(params.random_seed, params.debug.deactivate_spatial_effects);
        rgb = backend.grain_v2(&rgb, &gpu_params).unwrap_or_else(|| {
            let _cpu = super::StageObservation::cpu(
                backend,
                "grain_v2_cpu",
                spektrafilm_gpu::telemetry::CpuReason::GrainV2Unsupported,
            );
            spektrafilm_model::grain::v2::apply_cpu(&rgb, grain)
        });
    }
    // Magazine color is the final post-scan appearance, after scanner-domain
    // grain. A linear export uses the helper's finite transfer round-trip and
    // remains linear at the API boundary.
    if magazine_active {
        let _magazine = super::StageObservation::cpu(
            backend,
            "magazine_print_color",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        crate::magazine_print_color::apply(
            &mut rgb,
            &params.magazine_print_color,
            output_space,
            encoded_domain,
        );
    }
    // Linear exports retain the same grain realization as encoded exports.
    // Decode only the transfer; the stored same-space matrix ran above once.
    if grain_v2_active && !params.io.output_cctf_encoding {
        let _encoding = super::StageObservation::cpu(
            backend,
            "output_transfer",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        rgb.data.par_iter_mut().for_each(|value| {
            *value = from_f64(colorspace::cctf_decode(*value as f64, output_space.cctf));
        });
    }

    Ok(rgb)
}

/// Scan with the profile's own viewing illuminant and the film base included
/// — the per-stage topology entry point.
pub fn process(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    gamut: &crate::gamut_compression::OutputGamutCompress,
) -> Result<ImageBuf, String> {
    scan_with_options(
        density_cmy,
        profile,
        params,
        backend,
        color_ref,
        gamut,
        None,
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calibration() -> PositiveCalibration {
        PositiveCalibration {
            channels: std::array::from_fn(|_| PositiveChannel {
                density: vec![0.0, 1.0, 2.0],
                log_exposure: vec![-1.0, 0.0, 1.0],
                clear: 1.0,
            }),
        }
    }

    #[test]
    fn calibrated_response_preserves_polarity_and_unbounded_headroom() {
        let params = RuntimeParams::default();
        let input = ImageBuf::from_data(
            5,
            1,
            [100.0, 1.0, 0.1, 0.01, -100.0]
                .into_iter()
                .flat_map(|v| [from_f64(v); 3])
                .collect(),
        );
        let output = interpret_negative_capture(input, &calibration(), &params);
        let values: Vec<f64> = output.data.iter().step_by(3).map(|v| to_f64(*v)).collect();
        assert!(values.windows(2).all(|w| w[1] > w[0]));
        assert!(values[0] < 0.0 && values[4] > 1.0);
    }

    #[test]
    fn density_and_balance_controls_change_only_requested_channels() {
        let input = ImageBuf::from_data(1, 1, vec![from_f64(0.1); 3]);
        let mut params = RuntimeParams::default();
        let baseline = interpret_negative_capture(input.clone(), &calibration(), &params);
        params.scanner.positive_density_ev += 1.0;
        let brighter = interpret_negative_capture(input.clone(), &calibration(), &params);
        assert!(brighter.data.iter().zip(&baseline.data).all(|(a, b)| a > b));
        params.scanner.positive_density_ev -= 1.0;
        params.scanner.positive_balance_ev = [1.0, 0.0, -1.0];
        let balanced = interpret_negative_capture(input, &calibration(), &params);
        assert!(balanced.data[0] > baseline.data[0]);
        assert_eq!(balanced.data[1], baseline.data[1]);
        assert!(balanced.data[2] < baseline.data[2]);
    }

    #[test]
    fn very_small_output_density_stays_finite_and_ordered() {
        let mut params = RuntimeParams::default();
        params.scanner.positive_dmax = 1e-18;
        let input = ImageBuf::from_data(
            2,
            1,
            [0.5, 0.5, 0.5, 0.1, 0.1, 0.1]
                .into_iter()
                .map(from_f64)
                .collect(),
        );
        let output = interpret_negative_capture(input, &calibration(), &params);
        assert!(output.data.iter().all(|v| v.is_finite()));
        assert!(output.data[3] > output.data[0]);
    }
}
