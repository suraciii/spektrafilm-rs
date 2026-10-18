//! In-emulsion scatter and back-reflection halation in physical units.

use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{Scalar, ZERO, from_f64, to_f64};

///
/// Port of Python `apply_halation_um`.
///
/// Ordering: scatter → halation.
/// Scatter: energy-preserving mixture of Gaussian core + exponential tail.
/// Halation: additive multi-bounce sum of Gaussians with sqrt(k)-spaced widths.
#[allow(clippy::too_many_arguments)]
pub fn apply_halation_um(
    raw: &ImageBuf,
    pixel_size_um: f32,
    scatter_amount: f64,
    scatter_spatial_scale: f64,
    scatter_core_um: [f64; 3],
    scatter_tail_um: [f64; 3],
    scatter_tail_weight: [f64; 3],
    halation_amount: f64,
    halation_spatial_scale: f64,
    source_compression: f64,
    halation_strength: [f64; 3],
    halation_first_sigma_um: [f64; 3],
    halation_n_bounces: u32,
    halation_bounce_decay: f64,
    halation_renormalize: bool,

    _backend: &dyn ComputeBackend,
) -> ImageBuf {
    // Per-channel implementation matching Python's `apply_halation_um`.
    // The `_backend` argument is retained for API compat with the GPU
    // resident path; on the CPU export path we operate channel-by-
    // channel using `gaussian_blur_channel` and
    // `exponential_filter_channel` so that:
    //   * scatter blur σ is per-channel (Python passes a length-3
    //     `sigma_c_px` array to `fast_gaussian_filter`).
    //   * scatter tail is the proper exponential filter (3-Gaussian
    //     mixture, matching Python `fast_exponential_filter`).
    //   * halation σ is per-channel.
    //   * halation strength is per-channel.
    //   * tail weight is per-channel.
    // i.e. every place Python takes a length-3 array, we treat it as a
    // length-3 array, not as `(sum / 3.0)`.
    use spektrafilm_math::gaussian::{exponential_filter_channel, gaussian_blur_channel};

    let w = raw.width;
    let h = raw.height;
    let n_pix = (w as usize) * (h as usize);
    let mut channels: [Vec<Scalar>; 3] = [
        raw.extract_channel(0),
        raw.extract_channel(1),
        raw.extract_channel(2),
    ];

    // 1. Scatter pass — per-channel core gaussian + tail exponential.
    // f64 sigmas/lambdas — Python casts these via
    // `np.asarray(..., dtype=np.float64) * s_scale / pixel_size_um`.
    let pix_um_f64 = pixel_size_um as f64;
    if scatter_amount > 0.0 {
        for c in 0..3 {
            let sigma_c_px_f64 = scatter_core_um[c] * scatter_spatial_scale / pix_um_f64;
            let lambda_t_px_f64 = scatter_tail_um[c] * scatter_spatial_scale / pix_um_f64;
            if sigma_c_px_f64 <= 0.0 && lambda_t_px_f64 <= 0.0 {
                continue;
            }
            let sigma_c_px = sigma_c_px_f64.max(1e-6);
            let lambda_t_px = lambda_t_px_f64.max(1e-6);
            let core = gaussian_blur_channel(&channels[c], w, h, sigma_c_px);
            let tail = exponential_filter_channel(&channels[c], w, h, lambda_t_px);
            let one = from_f64(1.0);
            let wt = from_f64(scatter_tail_weight[c]);
            let sa = from_f64(scatter_amount);
            for i in 0..n_pix {
                let scattered = (one - wt) * core[i] + wt * tail[i];
                channels[c][i] = (one - sa) * channels[c][i] + sa * scattered;
            }
        }
    }

    // 2. Halation pass — per-channel σ + per-channel strength. The optional
    // source response is computed once from the post-scatter source and is
    // shared by every bounce; it does not alter the base raw image.
    let a_tot: [f64; 3] = [
        halation_strength[0] * halation_amount,
        halation_strength[1] * halation_amount,
        halation_strength[2] * halation_amount,
    ];
    let source_channels = if source_compression > 0.0
        && halation_n_bounces > 0
        && a_tot.iter().any(|&value| value > 0.0)
    {
        let mut source = channels.clone();
        for i in 0..n_pix {
            let peak = channels
                .iter()
                .map(|channel| to_f64(channel[i]))
                .fold(0.0_f64, f64::max);
            // Compress only the sub-anchor response. Values at or above the
            // fixed linear anchor remain unchanged, so the brightest source
            // keeps its halation while weaker sources are lifted toward it.
            let denominator = if peak > 1.0 {
                1.0
            } else {
                (1.0 - source_compression) + source_compression * peak
            };
            for channel in &mut source {
                channel[i] = from_f64(to_f64(channel[i]) / denominator);
            }
        }
        Some(source)
    } else {
        None
    };

    if halation_n_bounces >= 1 && (a_tot[0] > 0.0 || a_tot[1] > 0.0 || a_tot[2] > 0.0) {
        let n_bounces = halation_n_bounces as usize;
        // Decay computed in f64 to match Python's `rho ** (k - 1)` and
        // subsequent normalize-by-sum at full f64 precision.
        let mut decay = vec![0.0f64; n_bounces];
        for (k, slot) in decay.iter_mut().enumerate() {
            *slot = halation_bounce_decay.powi(k as i32);
        }
        let decay_sum: f64 = decay.iter().sum();
        for d in &mut decay {
            *d /= decay_sum;
        }

        for c in 0..3 {
            if a_tot[c] == 0.0 {
                continue;
            }
            let sigma_first_px_f64 =
                halation_first_sigma_um[c] * halation_spatial_scale / pix_um_f64;
            if sigma_first_px_f64 <= 0.0 {
                continue;
            }
            let mut hb = vec![ZERO; n_pix];
            for (k, &wk) in decay.iter().enumerate() {
                let sigma_k = (sigma_first_px_f64 * ((k as f64) + 1.0).sqrt()).max(1e-6);
                let blurred = source_channels.as_ref().map_or_else(
                    || gaussian_blur_channel(&channels[c], w, h, sigma_k),
                    |source| gaussian_blur_channel(&source[c], w, h, sigma_k),
                );
                let wk_s = from_f64(wk);
                for i in 0..n_pix {
                    hb[i] += wk_s * blurred[i];
                }
            }
            let a_c = from_f64(a_tot[c]);
            for i in 0..n_pix {
                channels[c][i] += a_c * hb[i];
            }
        }

        if halation_renormalize {
            let one = from_f64(1.0);
            for c in 0..3 {
                let denom = one + from_f64(a_tot[c]);
                for v in channels[c].iter_mut() {
                    *v /= denom;
                }
            }
        }
    }

    // Pack channels back into a single interleaved ImageBuf.
    let mut out = ImageBuf::new(w, h);

    for (c, channel) in channels.iter().enumerate() {
        out.write_channel(c, channel);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::cpu_backend::CpuBackend;
    use spektrafilm_math::precision::to_f64;

    fn render(source_compression: f64, strength: [f64; 3]) -> ImageBuf {
        let mut image = ImageBuf::new(9, 1);
        image.set(2, 0, [from_f64(1.0), ZERO, ZERO]);
        image.set(6, 0, [from_f64(0.25), ZERO, ZERO]);
        apply_halation_um(
            &image,
            1.0,
            0.0,
            1.0,
            [0.0; 3],
            [0.0; 3],
            [0.0; 3],
            0.1,
            1.0,
            source_compression,
            strength,
            [1.0; 3],
            1,
            0.5,
            true,
            &CpuBackend,
        )
    }

    #[test]
    fn source_compression_does_not_change_base_without_halation() {
        let linear = render(0.0, [0.0; 3]);
        let compressed = render(0.75, [0.0; 3]);
        assert_eq!(linear.data, compressed.data);
    }

    #[test]
    fn source_compression_raises_secondary_source_ratio() {
        let linear = render(0.0, [1.0, 0.0, 0.0]);
        let compressed = render(0.75, [1.0, 0.0, 0.0]);
        let linear_secondary = to_f64(linear.get(5, 0)[0]);
        let linear_primary = to_f64(linear.get(1, 0)[0]);
        let compressed_secondary = to_f64(compressed.get(5, 0)[0]);
        let compressed_primary = to_f64(compressed.get(1, 0)[0]);
        assert!(compressed_secondary / compressed_primary > linear_secondary / linear_primary);
    }
}
