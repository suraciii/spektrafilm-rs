/// Parametric density-curve models and H-D characteristic-curve interpolation.
///
/// The sampled profile arrays are a cache of the parametric model.  Keeping
/// the evaluator here makes the model usable by both profile loading and the
/// hot interpolation path without duplicating the septic dispatch logic.
///
/// Maps log exposure → density for each CMY channel using the per-profile
/// density curve tables.
use rayon::prelude::*;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::interp;
use spektrafilm_math::precision::{Scalar, ONE, ZERO, from_f64};

const SEPT_K: f64 = 5.8013;
/// Evaluate upstream's six-parameter parametric H-D density model.
///
/// The returned layout is `[exposure][channel]`. The logarithmic sum is
/// evaluated in a stable form so high-exposure probes do not overflow even
/// though the reference formula is written as `log10(1 + 10**x)`.
pub fn parametric_density_curves_model(
    log_exposure: &[f64],
    gamma: [f64; 3],
    log_exposure_0: [f64; 3],
    density_max: [f64; 3],
    toe_size: [f64; 3],
    shoulder_size: [f64; 3],
) -> Vec<[f64; 3]> {
    fn log10_one_plus_pow10(value: f64) -> f64 {
        if value > 0.0 {
            value + (1.0 + 10.0_f64.powf(-value)).log10()
        } else {
            (1.0 + 10.0_f64.powf(value)).log10()
        }
    }
    log_exposure
        .iter()
        .map(|&exposure| {
            std::array::from_fn(|channel| {
                let g = gamma[channel];
                let toe = toe_size[channel];
                let shoulder = shoulder_size[channel];
                let toe_argument = (exposure - log_exposure_0[channel]) / toe;
                let shoulder_argument =
                    (exposure - log_exposure_0[channel] - density_max[channel] / g) / shoulder;
                g * toe * log10_one_plus_pow10(toe_argument)
                    - g * shoulder * log10_one_plus_pow10(shoulder_argument)
            })
        })
        .collect()
}


#[inline]
fn septic_smoothstep(v: f64) -> f64 {
    let v2 = v * v;
    (v2 * v2) * (35.0 + v * (-84.0 + v * (70.0 - 20.0 * v)))
}

#[inline]
fn septic_warp(z: f64, alpha: f64) -> (f64, f64) {
    let u = (z / SEPT_K + 0.5).clamp(0.0, 1.0);
    if alpha == 0.0 {
        return (u, 1.0);
    }
    let t = 2.0 * u - 1.0;
    let v = (u + alpha * u * (1.0 - u) * t * t).clamp(0.0, 1.0);
    let dv_du = 1.0 + alpha * (u * (u * (-16.0 * u + 24.0) - 10.0) + 1.0);
    (v, dv_du)
}

/// Evaluate one layer's sigmoid. Unknown model names are rejected rather than
/// silently changing the profile's characteristic curve.
pub fn layer_cdf(z: f64, model_type: &str, alpha: f64) -> Result<f64, String> {
    match model_type {
        "norm_cdfs" | "cdfs" => {
            let x = z * std::f64::consts::FRAC_1_SQRT_2;
            Ok(if x.abs() < std::f64::consts::FRAC_1_SQRT_2 {
                0.5 + 0.5 * libm::erf(x)
            } else {
                let y = 0.5 * libm::erfc(x.abs());
                if x > 0.0 { 1.0 - y } else { y }
            })
        }
        "sept_norm_cdfs" => {
            if !alpha.is_finite() || alpha.abs() >= 1.0 {
                return Err(format!("septic density-curve alpha must satisfy |alpha| < 1 (got {alpha})"));
            }
            Ok(septic_smoothstep(septic_warp(z, alpha).0))
        }
        other => Err(format!(
            "unknown density-curve model_type {other:?}; expected \"norm_cdfs\" or \"sept_norm_cdfs\""
        )),
    }
}

/// Evaluate a fitted model onto an exposure axis.  `centers`, `amplitudes`,
/// `sigmas`, and optional `alphas` are indexed `[channel][layer]`.
pub fn evaluate_density_curves(
    log_exposure: &[f64],
    model_type: &str,
    centers: &[Vec<f64>],
    amplitudes: &[Vec<f64>],
    sigmas: &[Vec<f64>],
    alphas: Option<&[Vec<f64>]>,
    positive: bool,
) -> Result<Vec<Vec<f64>>, String> {
    if centers.len() != amplitudes.len() || centers.len() != sigmas.len() {
        return Err("density-curve model arrays must have matching channel counts".into());
    }
    validate_alphas(model_type, centers, alphas)?;
    let mut out = vec![vec![0.0; centers.len()]; log_exposure.len()];
    for (ch, ((cs, amps), ss)) in centers.iter().zip(amplitudes).zip(sigmas).enumerate() {
        if cs.len() != amps.len() || cs.len() != ss.len() {
            return Err(format!("density-curve model channel {ch} has inconsistent layer counts"));
        }
        for (layer, ((&center, &amp), &sigma)) in cs.iter().zip(amps).zip(ss).enumerate() {
            if !sigma.is_finite() || sigma <= 0.0 {
                return Err(format!("density-curve model sigma must be finite and positive (channel {ch}, layer {layer})"));
            }
            let alpha = alphas.and_then(|a| a.get(ch).and_then(|r| r.get(layer))).copied().unwrap_or(0.0);
            for (row, &x) in out.iter_mut().zip(log_exposure) {
                let z = if positive { -(x - center) / sigma } else { (x - center) / sigma };
                row[ch] += amp * layer_cdf(z, model_type, alpha)?;
            }
        }
    }
    Ok(out)
}

/// Evaluate per-layer curves as `[exposure][layer][channel]`.
pub fn evaluate_density_curves_layers(
    log_exposure: &[f64],
    model_type: &str,
    centers: &[Vec<f64>],
    amplitudes: &[Vec<f64>],
    sigmas: &[Vec<f64>],
    alphas: Option<&[Vec<f64>]>,
    positive: bool,
) -> Result<Vec<Vec<Vec<f64>>>, String> {
    let n_layers = centers.first().map_or(0, Vec::len);
    if centers.iter().any(|r| r.len() != n_layers) {
        return Err("density-curve model channels must have equal layer counts".into());
    }
    validate_alphas(model_type, centers, alphas)?;
    let mut out = vec![vec![vec![0.0; centers.len()]; n_layers]; log_exposure.len()];
    for ch in 0..centers.len() {
        if amplitudes.get(ch).map_or(true, |r| r.len() != n_layers)
            || sigmas.get(ch).map_or(true, |r| r.len() != n_layers)
        {
            return Err(format!("density-curve model channel {ch} has inconsistent layer counts"));
        }
        for layer in 0..n_layers {
            let alpha = alphas.and_then(|a| a.get(ch).and_then(|r| r.get(layer))).copied().unwrap_or(0.0);
            for (k, &x) in log_exposure.iter().enumerate() {
                let sigma = sigmas[ch][layer];
                if !sigma.is_finite() || sigma <= 0.0 {
                    return Err(format!("density-curve model sigma must be finite and positive (channel {ch}, layer {layer})"));
                }
                let z = if positive { -(x - centers[ch][layer]) / sigma } else { (x - centers[ch][layer]) / sigma };
                out[k][layer][ch] = amplitudes[ch][layer] * layer_cdf(z, model_type, alpha)?;
            }
        }
    }
    Ok(out)
}

fn validate_alphas(model_type: &str, centers: &[Vec<f64>], alphas: Option<&[Vec<f64>]>) -> Result<(), String> {
    if model_type == "sept_norm_cdfs" {
        if let Some(alphas) = alphas {
            if alphas.len() != centers.len() || alphas.iter().zip(centers).any(|(a, c)| a.len() != c.len()) {
                return Err("septic density-curve alphas must match the channel and layer counts".into());
            }
            if alphas.iter().flatten().any(|a| !a.is_finite() || a.abs() >= 1.0) {
                return Err("septic density-curve alpha must satisfy |alpha| < 1".into());
            }
        }
    }
    Ok(())
}

/// Interpolate density from log exposure for a single pixel value.
#[inline]
pub fn interpolate_density(
    log_exposure_axis: &[f32],
    density_curves: &[[f32; 3]],
    log_exp: f32,
    gamma: f32,
) -> [f32; 3] {
    if (gamma - 1.0).abs() < 1e-6 {
        interp::interp_uniform_3ch(
            log_exposure_axis[0],
            *log_exposure_axis.last().unwrap(),
            density_curves,
            log_exp,
        )
    } else {
        // Per-channel gamma: scale the x-axis by 1/gamma per channel
        // In Python: log_exposure[:,None]/gamma_factor[None,:]
        // This means we query at log_exp but on a stretched x-axis
        let x_min = log_exposure_axis[0];
        let x_max = *log_exposure_axis.last().unwrap();
        [
            interp::interp_uniform(
                x_min / gamma,
                x_max / gamma,
                &extract_col(density_curves, 0),
                log_exp,
            ),
            interp::interp_uniform(
                x_min / gamma,
                x_max / gamma,
                &extract_col(density_curves, 1),
                log_exp,
            ),
            interp::interp_uniform(
                x_min / gamma,
                x_max / gamma,
                &extract_col(density_curves, 2),
                log_exp,
            ),
        ]
    }
}

/// f64 variant — preserves precision through the interpolation.
pub fn interpolate_exposure_to_density_f64(
    log_raw: &ImageBuf,
    density_curves: &[[f64; 3]],
    log_exposure: &[f64],
    gamma_factor: f64,
) -> ImageBuf {
    if (gamma_factor - 1.0).abs() < 1e-12 {
        interp::fast_interp_image_f64(log_raw, log_exposure, density_curves)
    } else {
        let x_axes: Vec<[f64; 3]> = log_exposure
            .iter()
            .map(|&le| [le / gamma_factor, le / gamma_factor, le / gamma_factor])
            .collect();
        interp::fast_interp_image_perchannel_f64(log_raw, &x_axes, density_curves)
    }
}

/// Interpolate density for a full image. Port of Python `interpolate_exposure_to_density`.
pub fn interpolate_exposure_to_density(
    log_raw: &ImageBuf,
    density_curves: &[[f32; 3]],
    log_exposure: &[f32],
    gamma_factor: f32,
) -> ImageBuf {
    if (gamma_factor - 1.0).abs() < 1e-6 {
        interp::fast_interp_image(log_raw, log_exposure, density_curves)
    } else {
        // Build per-channel x-axes: log_exposure / gamma_factor
        let x_axes: Vec<[f32; 3]> = log_exposure
            .iter()
            .map(|&le| [le / gamma_factor, le / gamma_factor, le / gamma_factor])
            .collect();
        interp::fast_interp_image_perchannel(log_raw, &x_axes, density_curves)
    }
}

/// f64 variant — normalize density curves by subtracting the per-channel minimum.
pub fn normalize_density_curves_f64(curves: &[[f64; 3]]) -> Vec<[f64; 3]> {
    let mut min = [f64::INFINITY; 3];
    for row in curves {
        for c in 0..3 {
            if row[c].is_finite() && row[c] < min[c] {
                min[c] = row[c];
            }
        }
    }
    curves
        .iter()
        .map(|row| [row[0] - min[0], row[1] - min[1], row[2] - min[2]])
        .collect()
}

/// Normalize density curves by subtracting the per-channel minimum.
pub fn normalize_density_curves(curves: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut min = [f32::INFINITY; 3];
    for row in curves {
        for c in 0..3 {
            if row[c].is_finite() && row[c] < min[c] {
                min[c] = row[c];
            }
        }
    }
    curves
        .iter()
        .map(|row| [row[0] - min[0], row[1] - min[1], row[2] - min[2]])
        .collect()
}

/// Get max density per channel from curves.
pub fn max_density(curves: &[[f32; 3]]) -> [f32; 3] {
    let mut max = [f32::NEG_INFINITY; 3];
    for row in curves {
        for c in 0..3 {
            if row[c].is_finite() && row[c] > max[c] {
                max[c] = row[c];
            }
        }
    }
    max
}

/// Get max density per channel from f64 curves — Python parity for grain
/// which reads the profile's f64 `density_curves` directly.
pub fn max_density_f64(curves: &[[f64; 3]]) -> [f64; 3] {
    let mut max = [f64::NEG_INFINITY; 3];
    for row in curves {
        for c in 0..3 {
            if row[c].is_finite() && row[c] > max[c] {
                max[c] = row[c];
            }
        }
    }
    max
}

/// Per-sublayer density maxima from the raw layer tensor, Python
/// `np.nanmax(density_curves_layers, axis=0)`: max over the exposure
/// axis, keeping the `[sublayer][channel]` layout.
pub fn density_max_layers_f64(layers: &[[[f64; 3]; 3]]) -> [[f64; 3]; 3] {
    let mut max = [[f64::NEG_INFINITY; 3]; 3];
    for knot in layers {
        for sl in 0..3 {
            for ch in 0..3 {
                let v = knot[sl][ch];
                if v.is_finite() && v > max[sl][ch] {
                    max[sl][ch] = v;
                }
            }
        }
    }
    max
}

/// Split a composite CMY density image into per-sublayer densities.
///
/// Port of Python `interp_density_cmy_layers`: for each channel, the
/// composite density is the lookup key against the normalized composite
/// curve `density_curves[:, ch]`, and each sublayer reads its own curve
/// column `density_curves_layers[:, sl, ch]`. Positive films negate both
/// the image values and the x-axis (density decreases as exposure rises on
/// positives, so the axis only ascends once flipped); the y-tables stay
/// raw, exactly like upstream.
///
/// Returns `[sublayer][channel]` planes of `w * h` values. The
/// interpolation is linear in y, so sublayer densities of a profile whose
/// layer curves sum to the composite curve sum back to the composite
/// density at every pixel.
pub fn interp_density_cmy_layers(
    density_cmy: &ImageBuf,
    density_curves: &[[f64; 3]],
    density_curves_layers: &[[[f64; 3]; 3]],
    positive_film: bool,
) -> [[Vec<Scalar>; 3]; 3] {
    assert!(!density_curves.is_empty(), "density_curves must be non-empty");
    assert_eq!(
        density_curves_layers.len(),
        density_curves.len(),
        "density_curves_layers must share the exposure axis with density_curves"
    );
    let k = density_curves.len();
    let n = density_cmy.pixel_count();
    let mut planes = std::array::from_fn(|_| std::array::from_fn(|_| vec![ZERO; n]));

    for ch in 0..3 {
        // x-axis per channel, negated for positive films — Python passes
        // `-density_curves[:, ch]` with `-density_cmy` for the lookup.
        let sign = if positive_film { -1.0 } else { 1.0 };
        let xa: Vec<Scalar> = density_curves
            .iter()
            .map(|&row| from_f64(row[ch] * sign))
            .collect();
        let inv_dx: Vec<Scalar> = (0..k - 1)
            .map(|i| {
                let dx = xa[i + 1] - xa[i];
                if dx != ZERO {
                    1.0 / dx
                } else {
                    ZERO
                }
            })
            .collect();

        // Locate once per pixel, then interpolate every sublayer against
        // the same bracket. Matches `fast_interp`'s endpoint clamping and
        // searchsorted(side='right') - 1 bracketing.
        let x_col: Vec<Scalar> = density_cmy
            .pixels()
            .map(|px| if positive_film { -px[ch] } else { px[ch] })
            .collect();
        let located: Vec<(usize, Scalar)> = x_col
            .par_iter()
            .map(|&x| {
                if x <= xa[0] {
                    (0usize, ZERO)
                } else if x >= xa[k - 1] {
                    (k - 2, ONE)
                } else {
                    let idx = xa.partition_point(|&v| v <= x);
                    let low = if idx > 0 { idx - 1 } else { 0 };
                    let t = (x - xa[low]) * inv_dx[low];
                    (low, t)
                }
            })
            .collect();

        for sl in 0..3 {
            let plane = &mut planes[sl][ch];
            located
                .par_iter()
                .zip(plane.par_iter_mut())
                .for_each(|(&(low, t), out)| {
                    let y0 = from_f64(density_curves_layers[low][sl][ch]);
                    let y1 = from_f64(density_curves_layers[low + 1][sl][ch]);
                    *out = y0 + t * (y1 - y0);
                });
        }
    }

    planes
}

fn extract_col(data: &[[f32; 3]], c: usize) -> Vec<f32> {
    data.iter().map(|row| row[c]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolate_density_midpoint() {
        let axis: Vec<f32> = (0..5).map(|i| i as f32).collect();
        let curves = vec![
            [0.0, 0.0, 0.0],
            [0.5, 0.4, 0.3],
            [1.0, 0.8, 0.6],
            [1.5, 1.2, 0.9],
            [2.0, 1.6, 1.2],
        ];
        let d = interpolate_density(&axis, &curves, 1.5, 1.0);
        assert!((d[0] - 0.75).abs() < 1e-5);
        assert!((d[1] - 0.60).abs() < 1e-5);
        assert!((d[2] - 0.45).abs() < 1e-5);
    }

    #[test]
    fn test_normalize_density_curves() {
        let curves = vec![[1.0, 2.0, 3.0], [2.0, 3.0, 4.0]];
        let norm = normalize_density_curves(&curves);
        assert_eq!(norm[0], [0.0, 0.0, 0.0]);
        assert_eq!(norm[1], [1.0, 1.0, 1.0]);
    }

    #[test]
    fn test_max_density() {
        let curves = vec![[0.1, 0.2, 0.3], [2.0, 1.5, 1.0], [1.5, 1.8, 0.8]];
        let max = max_density(&curves);
        assert_eq!(max, [2.0, 1.8, 1.0]);
    }

    #[test]
    fn test_interp_density_cmy_layers_sums_to_composite() {
        // Layer curves that exactly partition the composite curve: the
        // interpolation is linear in y, so the sublayer split must sum back
        // to the composite density at every pixel (between knots too).
        let composite: Vec<[f64; 3]> = (0..=4)
            .map(|i| {
                let v = i as f64 * 0.5;
                [v, v * 0.8, v * 1.1]
            })
            .collect();
        let layers: Vec<[[f64; 3]; 3]> = composite
            .iter()
            .map(|&row| {
                [
                    [row[0] * 0.2, row[1] * 0.5, row[2] * 0.3],
                    [row[0] * 0.3, row[1] * 0.2, row[2] * 0.3],
                    [row[0] * 0.5, row[1] * 0.3, row[2] * 0.4],
                ]
            })
            .collect();

        let mut img = ImageBuf::new(6, 4);
        for y in 0..4u32 {
            for x in 0..6u32 {
                let t = (x + y * 6) as f64 / 24.0;
                img.set(x, y, [from_f64(t * 2.0), from_f64(t * 1.6), from_f64(t * 2.2)]);
            }
        }

        let planes = interp_density_cmy_layers(&img, &composite, &layers, false);
        for (i, px) in img.pixels().enumerate() {
            for ch in 0..3 {
                let sum: Scalar = (0..3).map(|sl| planes[sl][ch][i]).sum();
                assert!(
                    (sum - px[ch]).abs() < from_f64(1e-6),
                    "pixel {i} ch {ch}: sum {sum} vs {}",
                    px[ch]
                );
            }
        }
    }

    #[test]
    fn test_interp_density_cmy_layers_positive_negates_axis() {
        // Positive films negate the lookup key and the x-axis; the y-tables
        // stay raw. Check sublayer 0 against a hand-rolled negated lookup.
        let composite: Vec<[f64; 3]> = (0..=3)
            .map(|i| {
                let v = i as f64 * 0.5;
                [1.0 - v, 1.2 - v, 0.8 - v] // descending, ascending once negated
            })
            .collect();
        let layers: Vec<[[f64; 3]; 3]> = composite
            .iter()
            .map(|&row| {
                [
                    [row[0] * 0.25, row[1] * 0.5, row[2] * 0.1],
                    [row[0] * 0.35, row[1] * 0.2, row[2] * 0.3],
                    [row[0] * 0.40, row[1] * 0.3, row[2] * 0.6],
                ]
            })
            .collect();

        let mut img = ImageBuf::new(4, 2);
        for y in 0..2u32 {
            for x in 0..4u32 {
                let t = (x + y * 4) as f64 / 8.0;
                img.set(x, y, [from_f64(0.1 + t), from_f64(0.3 + t), from_f64(0.0 + t)]);
            }
        }

        let pos = interp_density_cmy_layers(&img, &composite, &layers, true);
        for (i, px) in img.pixels().enumerate() {
            for ch in 0..3 {
                let x = -(px[ch] as f64);
                let xa: Vec<f64> = composite.iter().map(|r| -r[ch]).collect();
                let expect = if x <= xa[0] {
                    layers[0][0][ch]
                } else if x >= xa[xa.len() - 1] {
                    layers[xa.len() - 1][0][ch]
                } else {
                    let idx = xa.partition_point(|&v| v <= x);
                    let low = idx.saturating_sub(1);
                    let t = (x - xa[low]) / (xa[low + 1] - xa[low]);
                    layers[low][0][ch] + t * (layers[low + 1][0][ch] - layers[low][0][ch])
                };
                assert!(
                    (pos[0][ch][i] - from_f64(expect)).abs() < from_f64(1e-6),
                    "pixel {i} ch {ch}: {} vs {expect}",
                    pos[0][ch][i]
                );
            }
        }
    }

    #[test]
    fn test_density_max_layers_f64() {
        let layers = vec![
            [[0.1, 0.2, 0.3], [0.4, 0.5, 0.6], [0.7, 0.8, 0.9]],
            [[1.1, 0.1, 0.2], [0.3, 1.4, 0.5], [0.6, 0.7, 1.8]],
        ];
        let max = density_max_layers_f64(&layers);
        assert_eq!(max, [[1.1, 0.2, 0.3], [0.4, 1.4, 0.6], [0.7, 0.8, 1.8]]);
    }

    #[test]
    fn septic_curves_preserve_median_polarity_and_layer_sums() {
        let centers = vec![vec![0.0, 0.0]];
        let amplitudes = vec![vec![0.6, 1.4]];
        let sigmas = vec![vec![1.0, 1.0]];
        let alphas = vec![vec![0.7, -0.4]];
        let axis = [-4.0, 0.0, 4.0];
        for positive in [false, true] {
            let total = evaluate_density_curves(&axis, "sept_norm_cdfs", &centers, &amplitudes, &sigmas, Some(&alphas), positive).unwrap();
            let layers = evaluate_density_curves_layers(&axis, "sept_norm_cdfs", &centers, &amplitudes, &sigmas, Some(&alphas), positive).unwrap();
            assert_eq!(total[1][0], 1.0);
            assert_eq!(total[0][0], if positive { 2.0 } else { 0.0 });
            assert_eq!(total[2][0], if positive { 0.0 } else { 2.0 });
            for i in 0..axis.len() { assert_eq!(layers[i][0][0] + layers[i][1][0], total[i][0]); }
        }
        let incomplete = vec![vec![0.7]];
        assert!(evaluate_density_curves(&axis, "sept_norm_cdfs", &centers, &amplitudes, &sigmas, Some(&incomplete), false).is_err());
        assert!(evaluate_density_curves_layers(&axis, "sept_norm_cdfs", &centers, &amplitudes, &sigmas, Some(&incomplete), false).is_err());
    }
    #[test]
    fn parametric_density_model_matches_reference_formula() {
        let axis = [-1.0, 0.0, 1.0];
        let got = parametric_density_curves_model(
            &axis,
            [1.1, 0.9, 1.0],
            [-0.2, 0.1, 0.0],
            [1.4, 1.2, 1.0],
            [0.7, 0.8, 0.9],
            [0.6, 0.7, 0.8],
        );
        for (row, &exposure) in got.iter().zip(&axis) {
            for channel in 0..3 {
                let g = [1.1, 0.9, 1.0][channel];
                let e0 = [-0.2, 0.1, 0.0][channel];
                let dmax = [1.4, 1.2, 1.0][channel];
                let toe = [0.7, 0.8, 0.9][channel];
                let shoulder = [0.6, 0.7, 0.8][channel];
                let reference = g * toe * (1.0 + 10.0_f64.powf((exposure - e0) / toe)).log10()
                    - g * shoulder
                        * (1.0 + 10.0_f64.powf((exposure - e0 - dmax / g) / shoulder)).log10();
                assert!((row[channel] - reference).abs() < 1e-12);
            }
        }
    }
}
