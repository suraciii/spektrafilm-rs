// DIR (Development Inhibitor Release) coupler model.
// Handles same-layer and inter-layer inhibition with spatial diffusion.

use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::from_f64;
use rayon::prelude::*;

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
        for c in 0..3 {
            let ch = correction.extract_channel(c);
            let g = gaussian_blur_channel(&ch, w_img, h_img, diffusion_size_pixel);
            let t = exponential_filter_channel(&ch, w_img, h_img, diffusion_tail_pixel);
            for i in 0..n_pix {
                blended_channels[c][i] = (one - w) * g[i] + w * t[i];
            }
        }
        for c in 0..3 {
            correction.write_channel(c, &blended_channels[c]);
        }
    }

    let mut result = log_raw.clone();
    result.data.par_iter_mut().zip(correction.data.par_iter()).for_each(|(r, c)| {
        *r -= c;
    });
    result
}
/// Backend-neutral DIR inputs derived once from the film profile and
/// controls. Both the per-stage CPU path ([`apply_density_correction`]) and
/// the GPU-resident chain builder consume this so the matrix scaling, the
/// "curves before DIR" inversion, and the density maxima cannot drift apart.
#[derive(Debug, Clone)]
pub struct DirPrepared {
    /// `couplers_matrix * amount`, row-major.
    pub matrix_scaled: [[f64; 3]; 3],
    /// Density curves before DIR coupler effects (from the normalized
    /// composite curves), re-interpolated against the corrected exposure.
    pub curves_0: Vec<[f64; 3]>,
    /// Per-channel maximum of the normalized composite curves.
    pub density_max: [f64; 3],
}

/// Derive the shared DIR inputs: scaled couplers matrix, the pre-DIR
/// curves, and the normalized density maxima. Pure function of the film
/// tables and the DIR controls — no image input.
pub fn prepare_dir(
    density_curves: &[[f64; 3]],
    log_exposure: &[f64],
    couplers_matrix: &[[f64; 3]; 3],
    amount: f64,
    positive: bool,
) -> DirPrepared {
    let mut matrix_scaled = *couplers_matrix;
    for row in &mut matrix_scaled {
        for v in row.iter_mut() {
            *v *= amount;
        }
    }
    let norm_curves = normalize_density_curves_f64(density_curves);
    let curves_0 = compute_curves_before_dir(&norm_curves, log_exposure, &matrix_scaled, positive);
    let density_max = max_density_f64(&norm_curves);
    DirPrepared { matrix_scaled, curves_0, density_max }
}

/// Full DIR coupler density correction pipeline.
///
/// Port of Python `apply_density_correction_dir_couplers`.
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
    backend: &dyn ComputeBackend,
) -> ImageBuf {
    let prepared = prepare_dir(density_curves, log_exposure, couplers_matrix, amount, positive);

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

/// Compute density curves before DIR coupler effects.
pub fn compute_curves_before_dir(
    density_curves: &[[f64; 3]],
    log_exposure: &[f64],
    couplers_matrix: &[[f64; 3]; 3],
    positive: bool,
) -> Vec<[f64; 3]> {
    let k = density_curves.len();
    let mut dc_silver = density_curves.to_vec();

    if positive {
        let max_d = max_density_f64(density_curves);
        for row in &mut dc_silver {
            for c in 0..3 {
                row[c] = max_d[c] - row[c];
            }
        }
    }

    // couplers_amount = dc_silver @ couplers_matrix
    let mut couplers_amount = vec![[0.0f64; 3]; k];
    for j in 0..k {
        for m in 0..3 {
            couplers_amount[j][m] = dc_silver[j][0] * couplers_matrix[0][m]
                + dc_silver[j][1] * couplers_matrix[1][m]
                + dc_silver[j][2] * couplers_matrix[2][m];
        }
    }

    // log_exposure_0 = log_exposure - couplers_amount
    // Then re-interpolate density curves on the shifted axis
    let mut corrected = vec![[0.0f64; 3]; k];
    for c in 0..3 {
        let le_shifted: Vec<f64> = (0..k)
            .map(|j| log_exposure[j] - couplers_amount[j][c])
            .collect();
        let dc_col: Vec<f64> = density_curves.iter().map(|row| row[c]).collect();
        for j in 0..k {
            corrected[j][c] = interp_curve(&le_shifted, &dc_col, log_exposure[j]);
        }
    }

    corrected
}

fn interp_curve(x: &[f64], y: &[f64], query: f64) -> f64 {
    if query <= x[0] { return y[0]; }
    if query >= x[x.len() - 1] { return y[y.len() - 1]; }
    let i = x.partition_point(|&v| v <= query) - 1;
    y[i] + (query - x[i]) / (x[i + 1] - x[i]) * (y[i + 1] - y[i])
}

#[cfg(test)]
mod tests {
    use super::compute_curves_before_dir;

    #[test]
    fn dir_curve_precision_matches_pinned_python_for_both_film_types() {
        let curves = [[0.000000017, 0.000000023, 0.000000031],
            [0.800000041, 0.700000037, 0.900000053],
            [1.700000083, 1.500000071, 1.900000097]];
        let exposure = [-0.700000019, 0.200000029, 1.300000059];
        let matrix = [[0.110000013, 0.070000017, 0.030000019],
            [0.020000023, 0.130000029, 0.050000031],
            [0.040000037, 0.060000041, 0.170000043]];
        // spektrafilm 0.3.4 compute_density_curves_before_dir_couplers.
        let expected = [
            [[2.0748033627172783e-8, 2.9048643554935127e-8, 4.006541070380199e-8],
                [0.9314286886750215, 0.8841925720761311, 1.1456548769502841],
                [1.700000083, 1.500000071, 1.900000097]],
            [[0.2258189881354618, 0.272116360817424, 0.36339942090710675],
                [0.9111554686303236, 0.8368501221276716, 1.077262643538455],
                [1.700000083, 1.500000071, 1.900000097]],
        ];
        for (positive, expected) in [false, true].into_iter().zip(expected) {
            let actual = compute_curves_before_dir(&curves, &exposure, &matrix, positive);
            for (actual, expected) in actual.iter().zip(expected) {
                for c in 0..3 {
                    assert!((actual[c] - expected[c]).abs() < 1e-14);
                }
            }
        }
    }
}
