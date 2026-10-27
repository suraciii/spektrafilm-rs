//! Parametric density-curve models and H-D characteristic-curve interpolation.
//!
//! The sampled profile arrays are a cache of the parametric model. Keeping
//! the evaluator here makes the model usable by both profile loading and the
//! hot interpolation path without duplicating the septic dispatch logic.
//!
//! Maps log exposure → density for each CMY channel using the per-profile
//! density curve tables.

const SEPT_K: f64 = 5.8013;

#[derive(Debug, Clone)]
pub enum DensityCurveError {
    AlphaValue(f64),
    ModelType(String),
    ChannelCounts,
    LayerCounts(usize),
    Sigma {
        channel: usize,
        layer: usize,
        value: f64,
    },
    EqualLayers,
    AlphaShape,
    AlphaBounds(f64),
}

impl std::fmt::Display for DensityCurveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlphaValue(value) => write!(
                f,
                "septic density-curve alpha must satisfy |alpha| < 1 (got {value})"
            ),
            Self::ModelType(value) => write!(
                f,
                "unknown density-curve model_type {value:?}; expected \"norm_cdfs\" or \"sept_norm_cdfs\""
            ),
            Self::ChannelCounts => {
                f.write_str("density-curve model arrays must have matching channel counts")
            }
            Self::LayerCounts(channel) => write!(
                f,
                "density-curve model channel {channel} has inconsistent layer counts"
            ),
            Self::Sigma { channel, layer, .. } => write!(
                f,
                "density-curve model sigma must be finite and positive (channel {channel}, layer {layer})"
            ),
            Self::EqualLayers => {
                f.write_str("density-curve model channels must have equal layer counts")
            }
            Self::AlphaShape => {
                f.write_str("septic density-curve alphas must match the channel and layer counts")
            }
            Self::AlphaBounds(_) => {
                f.write_str("septic density-curve alpha must satisfy |alpha| < 1")
            }
        }
    }
}
impl std::error::Error for DensityCurveError {}
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
    layer_cdf_detailed(z, model_type, alpha).map_err(|error| error.to_string())
}

pub fn layer_cdf_detailed(z: f64, model_type: &str, alpha: f64) -> Result<f64, DensityCurveError> {
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
                return Err(DensityCurveError::AlphaValue(alpha));
            }
            Ok(septic_smoothstep(septic_warp(z, alpha).0))
        }
        other => Err(DensityCurveError::ModelType(other.to_owned())),
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
    evaluate_density_curves_detailed(
        log_exposure,
        model_type,
        centers,
        amplitudes,
        sigmas,
        alphas,
        positive,
    )
    .map_err(|error| error.to_string())
}

pub fn evaluate_density_curves_detailed(
    log_exposure: &[f64],
    model_type: &str,
    centers: &[Vec<f64>],
    amplitudes: &[Vec<f64>],
    sigmas: &[Vec<f64>],
    alphas: Option<&[Vec<f64>]>,
    positive: bool,
) -> Result<Vec<Vec<f64>>, DensityCurveError> {
    if centers.len() != amplitudes.len() || centers.len() != sigmas.len() {
        return Err(DensityCurveError::ChannelCounts);
    }
    validate_alphas(model_type, centers, alphas)?;
    let mut out = vec![vec![0.0; centers.len()]; log_exposure.len()];
    for (ch, ((cs, amps), ss)) in centers.iter().zip(amplitudes).zip(sigmas).enumerate() {
        if cs.len() != amps.len() || cs.len() != ss.len() {
            return Err(DensityCurveError::LayerCounts(ch));
        }
        for (layer, ((&center, &amp), &sigma)) in cs.iter().zip(amps).zip(ss).enumerate() {
            if !sigma.is_finite() || sigma <= 0.0 {
                return Err(DensityCurveError::Sigma {
                    channel: ch,
                    layer,
                    value: sigma,
                });
            }
            let alpha = alphas
                .and_then(|a| a.get(ch).and_then(|r| r.get(layer)))
                .copied()
                .unwrap_or(0.0);
            for (row, &x) in out.iter_mut().zip(log_exposure) {
                let z = if positive {
                    -(x - center) / sigma
                } else {
                    (x - center) / sigma
                };
                row[ch] += amp * layer_cdf_detailed(z, model_type, alpha)?;
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
    evaluate_density_curves_layers_detailed(
        log_exposure,
        model_type,
        centers,
        amplitudes,
        sigmas,
        alphas,
        positive,
    )
    .map_err(|error| error.to_string())
}

pub fn evaluate_density_curves_layers_detailed(
    log_exposure: &[f64],
    model_type: &str,
    centers: &[Vec<f64>],
    amplitudes: &[Vec<f64>],
    sigmas: &[Vec<f64>],
    alphas: Option<&[Vec<f64>]>,
    positive: bool,
) -> Result<Vec<Vec<Vec<f64>>>, DensityCurveError> {
    let n_layers = centers.first().map_or(0, Vec::len);
    if centers.iter().any(|r| r.len() != n_layers) {
        return Err(DensityCurveError::EqualLayers);
    }
    validate_alphas(model_type, centers, alphas)?;
    let mut out = vec![vec![vec![0.0; centers.len()]; n_layers]; log_exposure.len()];
    for ch in 0..centers.len() {
        if amplitudes.get(ch).is_none_or(|r| r.len() != n_layers)
            || sigmas.get(ch).is_none_or(|r| r.len() != n_layers)
        {
            return Err(DensityCurveError::LayerCounts(ch));
        }
        for layer in 0..n_layers {
            let alpha = alphas
                .and_then(|a| a.get(ch).and_then(|r| r.get(layer)))
                .copied()
                .unwrap_or(0.0);
            for (k, &x) in log_exposure.iter().enumerate() {
                let sigma = sigmas[ch][layer];
                if !sigma.is_finite() || sigma <= 0.0 {
                    return Err(DensityCurveError::Sigma {
                        channel: ch,
                        layer,
                        value: sigma,
                    });
                }
                let z = if positive {
                    -(x - centers[ch][layer]) / sigma
                } else {
                    (x - centers[ch][layer]) / sigma
                };
                out[k][layer][ch] =
                    amplitudes[ch][layer] * layer_cdf_detailed(z, model_type, alpha)?;
            }
        }
    }
    Ok(out)
}

fn validate_alphas(
    model_type: &str,
    centers: &[Vec<f64>],
    alphas: Option<&[Vec<f64>]>,
) -> Result<(), DensityCurveError> {
    if model_type == "sept_norm_cdfs"
        && let Some(alphas) = alphas
    {
        if alphas.len() != centers.len()
            || alphas.iter().zip(centers).any(|(a, c)| a.len() != c.len())
        {
            return Err(DensityCurveError::AlphaShape);
        }
        if let Some(&value) = alphas
            .iter()
            .flatten()
            .find(|a| !a.is_finite() || a.abs() >= 1.0)
        {
            return Err(DensityCurveError::AlphaBounds(value));
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detailed_density_failures_keep_english_and_rejected_facts() {
        let centers = vec![vec![0.0]];
        let amplitudes = vec![vec![1.0]];
        let sigmas = vec![vec![0.0]];
        let error = evaluate_density_curves_detailed(
            &[0.0],
            "norm_cdfs",
            &centers,
            &amplitudes,
            &sigmas,
            None,
            false,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            DensityCurveError::Sigma {
                channel: 0,
                layer: 0,
                value: 0.0
            }
        ));
        assert_eq!(
            error.to_string(),
            evaluate_density_curves(
                &[0.0],
                "norm_cdfs",
                &centers,
                &amplitudes,
                &sigmas,
                None,
                false
            )
            .unwrap_err()
        );
        let alphas = vec![vec![f64::NAN]];
        let error = evaluate_density_curves_detailed(
            &[0.0],
            "sept_norm_cdfs",
            &centers,
            &amplitudes,
            &[vec![1.0]],
            Some(&alphas),
            false,
        )
        .unwrap_err();
        assert!(matches!(error, DensityCurveError::AlphaBounds(value) if value.is_nan()));
        assert_eq!(
            error.to_string(),
            "septic density-curve alpha must satisfy |alpha| < 1"
        );
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
    fn parametric_model_is_monotonic_and_near_zero_in_toe() {
        let x: Vec<f64> = (-60..=20).map(|v| v as f64 / 10.0).collect();
        let curves = parametric_density_curves_model(
            &x,
            [0.6, 0.65, 0.7],
            [-1.0, -1.1, -0.9],
            [2.2, 2.0, 1.8],
            [0.3, 0.3, 0.3],
            [0.5, 0.5, 0.5],
        );
        assert!(curves[0].iter().all(|value| value.abs() < 1e-10));
        for channel in 0..3 {
            for pair in curves.windows(2) {
                assert!(
                    pair[1][channel] + 1e-14 >= pair[0][channel],
                    "channel {channel} decreased beyond floating-point tolerance: {} -> {}",
                    pair[0][channel],
                    pair[1][channel]
                );
            }
        }
    }

    #[test]
    fn septic_curves_preserve_median_polarity_and_layer_sums() {
        let centers = vec![vec![0.0, 0.0]];
        let amplitudes = vec![vec![0.6, 1.4]];
        let sigmas = vec![vec![1.0, 1.0]];
        let alphas = vec![vec![0.7, -0.4]];
        let axis = [-4.0, 0.0, 4.0];
        for positive in [false, true] {
            let total = evaluate_density_curves(
                &axis,
                "sept_norm_cdfs",
                &centers,
                &amplitudes,
                &sigmas,
                Some(&alphas),
                positive,
            )
            .unwrap();
            let layers = evaluate_density_curves_layers(
                &axis,
                "sept_norm_cdfs",
                &centers,
                &amplitudes,
                &sigmas,
                Some(&alphas),
                positive,
            )
            .unwrap();
            assert_eq!(total[1][0], 1.0);
            assert_eq!(total[0][0], if positive { 2.0 } else { 0.0 });
            assert_eq!(total[2][0], if positive { 0.0 } else { 2.0 });
            for i in 0..axis.len() {
                assert_eq!(layers[i][0][0] + layers[i][1][0], total[i][0]);
            }
        }
        let incomplete = vec![vec![0.7]];
        assert!(
            evaluate_density_curves(
                &axis,
                "sept_norm_cdfs",
                &centers,
                &amplitudes,
                &sigmas,
                Some(&incomplete),
                false
            )
            .is_err()
        );
        assert!(
            evaluate_density_curves_layers(
                &axis,
                "sept_norm_cdfs",
                &centers,
                &amplitudes,
                &sigmas,
                Some(&incomplete),
                false
            )
            .is_err()
        );
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
