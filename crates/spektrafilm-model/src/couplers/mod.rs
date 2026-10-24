// DIR (Development Inhibitor Release) coupler model.
// Handles same-layer and inter-layer inhibition with spatial diffusion.

use rayon::prelude::*;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};

use crate::density_curves::{max_density_f64, normalize_density_curves_f64};

/// DIR couplers matrix [3][3]. Row = donor layer, Column = receiver layer.
pub fn compute_dir_couplers_matrix(
    gamma_samelayer: [f64; 3],
    gamma_r_to_gb: [f64; 2],
    gamma_g_to_rb: [f64; 2],
    gamma_b_to_rg: [f64; 2],
    inhibition_samelayer: f64,
    inhibition_interlayer: f64,
) -> [[f64; 3]; 3] {
    let mut m = [[0.0f64; 3]; 3];
    m[0][0] = gamma_samelayer[0] * inhibition_samelayer;
    m[1][1] = gamma_samelayer[1] * inhibition_samelayer;
    m[2][2] = gamma_samelayer[2] * inhibition_samelayer;
    m[0][1] = gamma_r_to_gb[0] * inhibition_interlayer;
    m[0][2] = gamma_r_to_gb[1] * inhibition_interlayer;
    m[1][0] = gamma_g_to_rb[0] * inhibition_interlayer;
    m[1][2] = gamma_g_to_rb[1] * inhibition_interlayer;
    m[2][0] = gamma_b_to_rg[0] * inhibition_interlayer;
    m[2][1] = gamma_b_to_rg[1] * inhibition_interlayer;
    m
}

/// Apply exposure correction from DIR couplers.
///
/// density_cmy contributes inhibitor to other layers through the couplers matrix.
/// The inhibitor diffuses spatially (Gaussian + exponential tail).
#[allow(clippy::too_many_arguments)]
pub fn compute_exposure_correction(
    log_raw: &ImageBuf,
    density_cmy: &ImageBuf,
    density_max: [f64; 3],
    couplers_matrix: &[[f64; 3]; 3],
    diffusion_size_pixel: f32,
    diffusion_tail_pixel: f32,
    diffusion_tail_weight: f64,
    positive: bool,
    donor_k: Option<[f64; 3]>,
    donor_ref: [f64; 3],
    receiver_k: Option<[f64; 3]>,
    receiver_ref: [f64; 3],
    _backend: &dyn ComputeBackend,
) -> ImageBuf {
    let mut density_silver = density_cmy.clone();

    if positive {
        let dmax = [
            from_f64(density_max[0]),
            from_f64(density_max[1]),
            from_f64(density_max[2]),
        ];
        density_silver.par_pixels_mut().for_each(|px| {
            for c in 0..3 {
                px[c] = dmax[c] - px[c];
            }
        });
    }
    if let Some(k) = donor_k {
        density_silver.par_pixels_mut().for_each(|px| {
            for c in 0..3 {
                px[c] = from_f64(langmuir(to_f64(px[c]), k[c], donor_ref[c]));
            }
        });
    }

    // log_raw_correction = einsum('ijk, km->ijm', density_silver, couplers_matrix)
    let cm: [[spektrafilm_math::precision::Scalar; 3]; 3] = [
        [
            from_f64(couplers_matrix[0][0]),
            from_f64(couplers_matrix[0][1]),
            from_f64(couplers_matrix[0][2]),
        ],
        [
            from_f64(couplers_matrix[1][0]),
            from_f64(couplers_matrix[1][1]),
            from_f64(couplers_matrix[1][2]),
        ],
        [
            from_f64(couplers_matrix[2][0]),
            from_f64(couplers_matrix[2][1]),
            from_f64(couplers_matrix[2][2]),
        ],
    ];
    let mut correction = ImageBuf::new(log_raw.width, log_raw.height);
    correction
        .data
        .par_chunks_exact_mut(3)
        .zip(density_silver.data.par_chunks_exact(3))
        .for_each(|(out, px)| {
            for m in 0..3 {
                out[m] = px[0] * cm[0][m] + px[1] * cm[1][m] + px[2] * cm[2][m];
            }
        });
    if diffusion_size_pixel > 0.0 {
        // Python: `(1 - w) * fast_gaussian_filter(corr, σ_size)
        //         + w * fast_exponential_filter(corr, σ_tail)`.
        // The tail kernel is EXPONENTIAL (3-Gaussian mixture), not
        // another Gaussian — using two Gaussians here was a structural
        // diff vs Python.
        use spektrafilm_math::gaussian::{exponential_filter_channel, gaussian_blur_channel};
        let w_img = correction.width;
        let h_img = correction.height;
        let n_pix = (w_img as usize) * (h_img as usize);
        let w = from_f64(diffusion_tail_weight);
        let one = from_f64(1.0);
        let mut blended_channels: [Vec<spektrafilm_math::precision::Scalar>; 3] =
            [vec![one; n_pix], vec![one; n_pix], vec![one; n_pix]];
        for (c, blended_channel) in blended_channels.iter_mut().enumerate() {
            let ch = correction.extract_channel(c);
            let g = gaussian_blur_channel(&ch, w_img, h_img, diffusion_size_pixel);
            let t = exponential_filter_channel(&ch, w_img, h_img, diffusion_tail_pixel);
            // Pixel indices align the output with both filtered channels.
            #[allow(clippy::needless_range_loop)]
            for i in 0..n_pix {
                blended_channel[i] = (one - w) * g[i] + w * t[i];
            }
        }
        for (c, blended_channel) in blended_channels.iter().enumerate() {
            correction.write_channel(c, blended_channel);
        }
    }

    // Reversal receiver responds to inhibitor arriving after spatial diffusion.
    if let Some(k) = receiver_k {
        correction.par_pixels_mut().for_each(|px| {
            for c in 0..3 {
                px[c] = from_f64(langmuir(to_f64(px[c]), k[c], receiver_ref[c]));
            }
        });
    }

    let mut result = log_raw.clone();
    result
        .data
        .par_iter_mut()
        .zip(correction.data.par_iter())
        .for_each(|(r, c)| {
            *r -= c;
        });
    result
}
/// Backend-neutral DIR inputs derived once from the film profile and
/// controls. Both the per-stage CPU path ([`apply_density_correction`]) and
/// the GPU-resident chain builder consume this so the matrix scaling, the
/// "curves before DIR" inversion, and the Langmuir chemistry cannot drift
/// apart.
#[derive(Debug, Clone)]
pub struct DirPrepared {
    /// `couplers_matrix * amount`, row-major.
    pub matrix_scaled: [[f64; 3]; 3],
    /// Density curves before DIR coupler effects (from the normalized
    /// composite curves), re-interpolated against the corrected exposure.
    pub curves_0: Vec<[f64; 3]>,
    /// Per-channel maximum of the normalized composite curves.
    pub density_max: [f64; 3],
    /// Donor Langmuir knee per channel (`langmuir_donor_k * d_max`). A
    /// channel with `K = inf` stays linear.
    pub donor_k: [f64; 3],
    /// Donor Langmuir reference per channel (`d_max / 2`); also the
    /// amplitude-match point of the exposure correction.
    pub donor_ref: [f64; 3],
    /// Receiver Langmuir knee per channel — positive films only.
    pub receiver_knee: Option<[f64; 3]>,
    /// Receiver operating-point inhibitor per channel — positive only.
    pub receiver_ref: Option<[f64; 3]>,
}

/// Derive the shared DIR inputs: scaled couplers matrix, the pre-DIR
/// Langmuir curves, the normalized density maxima and the Langmuir
/// parameters. Pure function of the film tables and the DIR controls — no
/// image input.
pub fn prepare_dir(
    density_curves: &[[f64; 3]],
    log_exposure: &[f64],
    couplers_matrix: &[[f64; 3]; 3],
    amount: f64,
    positive: bool,
    langmuir_donor_k: [f64; 3],
    langmuir_receiver_k: [f64; 3],
) -> DirPrepared {
    let mut matrix_scaled = *couplers_matrix;
    for row in &mut matrix_scaled {
        for v in row.iter_mut() {
            *v *= amount;
        }
    }
    let norm_curves = normalize_density_curves_f64(density_curves);
    let (donor_k, donor_ref) = langmuir_params(&norm_curves, langmuir_donor_k);
    // The receiver knee derives from the *unit* matrix (before the amount
    // scaling) so a pushed `amount` drives the response into saturation
    // instead of moving the knee.
    let receiver = receiver_params(donor_ref, couplers_matrix, langmuir_receiver_k);
    let curves_0 = curves_before_dir_langmuir(
        &norm_curves,
        log_exposure,
        &matrix_scaled,
        positive,
        donor_k,
        donor_ref,
        positive.then_some(receiver),
    );
    let density_max = max_density_f64(&norm_curves);
    let (receiver_knee, receiver_ref) = if positive {
        (Some(receiver.0), Some(receiver.1))
    } else {
        (None, None)
    };
    DirPrepared {
        matrix_scaled,
        curves_0,
        density_max,
        donor_k,
        donor_ref,
        receiver_knee,
        receiver_ref,
    }
}

fn langmuir(value: f64, k: f64, reference: f64) -> f64 {
    if k.is_infinite() {
        value
    } else {
        value * (k + reference) / (k + value)
    }
}

fn langmuir_params(curves: &[[f64; 3]], donor_k: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let dmax = max_density_f64(curves);
    let dref = [dmax[0] * 0.5, dmax[1] * 0.5, dmax[2] * 0.5];
    let k = [
        donor_k[0] * dmax[0],
        donor_k[1] * dmax[1],
        donor_k[2] * dmax[2],
    ];
    (k, dref)
}

fn receiver_params(
    donor_ref: [f64; 3],
    matrix_unit: &[[f64; 3]; 3],
    receiver_k: [f64; 3],
) -> ([f64; 3], [f64; 3]) {
    let mut reference = [0.0; 3];
    let mut knee = [0.0; 3];
    for c in 0..3 {
        reference[c] = (0..3).map(|k| donor_ref[k] * matrix_unit[k][c]).sum();
        knee[c] = receiver_k[c] * 2.0 * reference[c];
    }
    (knee, reference)
}

fn curves_before_dir_langmuir(
    curves: &[[f64; 3]],
    exposure: &[f64],
    matrix: &[[f64; 3]; 3],
    positive: bool,
    donor_k: [f64; 3],
    donor_ref: [f64; 3],
    receiver: Option<([f64; 3], [f64; 3])>,
) -> Vec<[f64; 3]> {
    let dmax = max_density_f64(curves);
    let mut out = vec![[0.0; 3]; curves.len()];
    for c in 0..3 {
        let mut shifted = Vec::with_capacity(curves.len());
        for row in curves {
            let mut donor = [row[0], row[1], row[2]];
            if positive {
                for k in 0..3 {
                    donor[k] = dmax[k] - donor[k];
                }
            }
            let mut inhibitor = 0.0;
            for k in 0..3 {
                let d = if positive {
                    donor[k]
                } else {
                    langmuir(donor[k], donor_k[k], donor_ref[k])
                };
                inhibitor += d * matrix[k][c];
            }
            if let Some((knee, reference)) = receiver {
                inhibitor = langmuir(inhibitor, knee[c], reference[c]);
            }
            shifted.push(exposure[shifted.len()] - inhibitor);
        }
        let values: Vec<f64> = curves.iter().map(|r| r[c]).collect();
        let neg_values: Vec<f64> = values.iter().map(|v| -*v).collect();
        for j in 0..curves.len() {
            let q = exposure[j];
            if q <= shifted[0] {
                out[j][c] = if positive { -neg_values[0] } else { values[0] };
            } else if q >= shifted[shifted.len() - 1] {
                out[j][c] = if positive {
                    -neg_values[values.len() - 1]
                } else {
                    values[values.len() - 1]
                };
            } else {
                let i = shifted.partition_point(|&v| v <= q) - 1;
                let weight = (q - shifted[i]) / (shifted[i + 1] - shifted[i]);
                let value = neg_values[i] + weight * (neg_values[i + 1] - neg_values[i]);
                out[j][c] = if positive {
                    -value
                } else {
                    values[i] + weight * (values[i + 1] - values[i])
                };
            }
        }
    }
    out
}

/// Full DIR coupler density correction pipeline.
///
/// Port of Python `apply_density_correction_dir_couplers`.
/// Returns backend density-curve interpolation errors unchanged.
#[allow(clippy::too_many_arguments)]
pub fn apply_density_correction(
    density_cmy: &ImageBuf,
    log_raw: &ImageBuf,
    pixel_size_um: f32,
    log_exposure: &[f64],
    density_curves: &[[f64; 3]],
    couplers_matrix: &[[f64; 3]; 3],
    amount: f64,
    diffusion_size_um: f64,
    diffusion_tail_um: f64,
    diffusion_tail_weight: f64,
    positive: bool,
    gamma_factor: f32,
    langmuir_donor_k: [f64; 3],
    langmuir_receiver_k: [f64; 3],
    backend: &dyn ComputeBackend,
) -> Result<ImageBuf, String> {
    let prepared = prepare_dir(
        density_curves,
        log_exposure,
        couplers_matrix,
        amount,
        positive,
        langmuir_donor_k,
        langmuir_receiver_k,
    );
    let (donor, receiver, rref) = if positive {
        (
            None,
            prepared.receiver_knee,
            prepared.receiver_ref.unwrap_or([0.0; 3]),
        )
    } else {
        (Some(prepared.donor_k), None, [0.0; 3])
    };
    let diffusion_size_px = (diffusion_size_um / pixel_size_um as f64) as f32;
    let diffusion_tail_px = (diffusion_tail_um / pixel_size_um as f64) as f32;
    let log_raw_corrected = compute_exposure_correction(
        log_raw,
        density_cmy,
        prepared.density_max,
        &prepared.matrix_scaled,
        diffusion_size_px,
        diffusion_tail_px,
        diffusion_tail_weight,
        positive,
        donor,
        prepared.donor_ref,
        receiver,
        rref,
        backend,
    );

    // Profile tables stay at their native precision until the backend boundary.
    backend.density_curve_interp(
        &log_raw_corrected,
        log_exposure,
        &prepared.curves_0,
        gamma_factor as f64,
    )
}

#[cfg(test)]
mod tests {
    use super::curves_before_dir_langmuir;

    #[test]
    fn dir_curve_precision_matches_pinned_python_for_both_film_types() {
        let curves = [
            [0.000000017, 0.000000023, 0.000000031],
            [0.800000041, 0.700000037, 0.900000053],
            [1.700000083, 1.500000071, 1.900000097],
        ];
        let exposure = [-0.700000019, 0.200000029, 1.300000059];
        let matrix = [
            [0.110000013, 0.070000017, 0.030000019],
            [0.020000023, 0.130000029, 0.050000031],
            [0.040000037, 0.060000041, 0.170000043],
        ];
        // An infinite donor knee and no receiver side degenerate to the
        // linear 0.3.4 model, whose `compute_density_curves_before_dir_couplers`
        // output is pinned below (empty-curve channels clamp, middle row
        // interpolates on the shifted exposure).
        let mut dmax = [0.0_f64; 3];
        for row in &curves {
            for c in 0..3 {
                dmax[c] = dmax[c].max(row[c]);
            }
        }
        let donor_ref = [dmax[0] * 0.5, dmax[1] * 0.5, dmax[2] * 0.5];
        let expected = [
            [
                [
                    2.0748033627172783e-8,
                    2.9048643554935127e-8,
                    4.006541070380199e-8,
                ],
                [0.9314286886750215, 0.8841925720761311, 1.1456548769502841],
                [1.700000083, 1.500000071, 1.900000097],
            ],
            [
                [0.2258189881354618, 0.272116360817424, 0.36339942090710675],
                [0.9111554686303236, 0.8368501221276716, 1.077262643538455],
                [1.700000083, 1.500000071, 1.900000097],
            ],
        ];
        for (positive, expected) in [false, true].into_iter().zip(expected) {
            let actual = curves_before_dir_langmuir(
                &curves,
                &exposure,
                &matrix,
                positive,
                [f64::INFINITY; 3],
                donor_ref,
                None,
            );
            for (actual, expected) in actual.iter().zip(expected) {
                for c in 0..3 {
                    assert!((actual[c] - expected[c]).abs() < 1e-14);
                }
            }
        }
    }
}
