//! Exact separable optical response for a wavelength-invariant scalar record.

/// Unnormalized XYZ for density `D` is `xyz * 10^(-coefficient * D)`.
/// Preparation proves the basis from the resolved spectra, not a material label.
#[derive(Debug, Clone, Copy)]
pub struct ScalarScanCoefficients {
    pub coefficient: f64,
    pub xyz: [f64; 3],
    minimum_log_scale: f64,
    maximum_log_scale: f64,
}

impl ScalarScanCoefficients {
    /// Bound `x = -coefficient * D` so the exponential, lamp multiplication,
    /// and CMF projection retain normal precision before any later rescaling.
    /// An inverted interval disables factoring; callers fall back, never clip D.
    pub fn exponent_bounds(&self, minimum: f64, maximum: f64) -> [f64; 2] {
        [
            minimum - self.minimum_log_scale,
            maximum - self.maximum_log_scale,
        ]
    }

    /// Use the direct CPU integrator's effective wavelength and optional-base rules.
    /// NaN light rows are excluded before CMF projection, just as in that integrator.
    /// Unsupported or non-finite spectra remain on the generic spectral path.
    pub fn prepare(
        channel_density: &[[f64; 3]],
        base_density: &[f64],
        illuminant: &[f64],
        cmfs: &[[f64; 3]],
    ) -> Option<Self> {
        let n = channel_density.len().min(illuminant.len()).min(cmfs.len());
        let has_base = !base_density.is_empty() && base_density.len() >= n;
        let mut coefficient = None;
        let mut xyz = [0.0; 3];
        let mut minimum_log_scale = 0.0_f64;
        let mut maximum_log_scale = 0.0_f64;
        for i in 0..n {
            let dye = channel_density[i];
            let base = if has_base { base_density[i] } else { 0.0 };
            // Even a masked light row propagates a non-finite CMF in the generic GEMM.
            if !cmfs[i].iter().all(|v| v.is_finite()) {
                return None;
            }
            if dye.iter().any(|v| v.is_nan()) || base.is_nan() || illuminant[i].is_nan() {
                continue;
            }
            if !dye.iter().all(|v| v.is_finite())
                || !base.is_finite()
                || !illuminant[i].is_finite()
                || illuminant[i] < 0.0
                || cmfs[i].iter().any(|&v| v < 0.0)
                || dye[1] != 0.0
                || dye[2] != 0.0
            {
                return None;
            }
            match coefficient {
                Some(k) if k != dye[0] => return None,
                None => coefficient = Some(dye[0]),
                _ => {}
            }
            minimum_log_scale = minimum_log_scale.min(-base);
            maximum_log_scale = maximum_log_scale.max(-base);
            let base_transmission = 10.0f64.powf(-base);
            // Factoring must not reuse a base power that already lost its
            // normal precision before a large lamp or negative D rescales it.
            if !base_transmission.is_normal() {
                return None;
            }
            let light = base_transmission * illuminant[i];
            if !light.is_finite() || (illuminant[i] != 0.0 && !light.is_normal()) {
                return None;
            }
            let light_log_scale = (illuminant[i] > 0.0).then(|| -base + illuminant[i].log10());
            if let Some(scale) = light_log_scale {
                minimum_log_scale = minimum_log_scale.min(scale);
                maximum_log_scale = maximum_log_scale.max(scale);
            }
            for (c, value) in xyz.iter_mut().enumerate() {
                let contribution = light * cmfs[i][c];
                if !contribution.is_finite()
                    || (light != 0.0 && cmfs[i][c] != 0.0 && !contribution.is_normal())
                {
                    return None;
                }
                if let Some(scale) = light_log_scale
                    && cmfs[i][c] > 0.0
                {
                    let scale = scale + cmfs[i][c].log10();
                    minimum_log_scale = minimum_log_scale.min(scale);
                    maximum_log_scale = maximum_log_scale.max(scale);
                }
                *value += contribution;
            }
        }
        if !xyz.iter().all(|v| v.is_finite()) {
            return None;
        }
        Some(Self {
            coefficient: coefficient?,
            xyz,
            minimum_log_scale,
            maximum_log_scale,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu_backend::{PreparedScanCpu, scan_spectral_cpu_with_cmfs};
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::{from_f64, to_f64};

    const CMFS: [[f64; 3]; 5] = [
        [0.1, 0.2, 0.8],
        [0.3, 0.7, 0.4],
        [0.6, 0.9, 0.1],
        [0.8, 0.4, 0.0],
        [0.2, 0.1, 0.0],
    ];
    const CAT: [[f64; 3]; 3] = [[1.02, -0.03, 0.01], [0.02, 0.98, 0.01], [-0.01, 0.03, 1.01]];
    const RGB: [[f64; 3]; 3] = [[3.2, -1.5, -0.5], [-0.9, 1.8, 0.04], [0.05, -0.2, 1.05]];

    // Declared before the comparisons: f64 factoring/reduction error, or the
    // final f32 conversion at the public ImageBuf boundary. Not pixel identity.
    fn assert_rgb_close(actual: &ImageBuf, expected: &ImageBuf) {
        let tolerance = if cfg!(feature = "precision-f64") {
            2e-12
        } else {
            2e-6
        };
        for (&a, &e) in actual.data.iter().zip(&expected.data) {
            let (a, e) = (to_f64(a), to_f64(e));
            assert!((a - e).abs() <= tolerance * (1.0 + e.abs()), "{a} != {e}");
        }
    }

    #[test]
    fn factored_integral_matches_independent_wavelength_sum_with_missing_rows() {
        let cd = [
            [0.73, 0.0, 0.0],
            [f64::NAN, 0.0, 0.0],
            [0.73, 0.0, 0.0],
            [0.73, 0.0, 0.0],
            [0.73, 0.0, 0.0],
        ];
        let base = [0.11, 0.3, 0.37, f64::NAN, 0.02];
        let lamp = [0.7, 1.2, 1.1, 0.9, f64::NAN];
        let scalar = ScalarScanCoefficients::prepare(&cd, &base, &lamp, &CMFS).unwrap();
        for density in [-2.0, -0.1, 0.0, 0.2, 1.0, 4.0, 12.0] {
            let mut reference = [0.0; 3];
            for i in 0..cd.len() {
                let light = 10.0f64.powf(-(density * cd[i][0] + base[i])) * lamp[i];
                let light = if light.is_nan() { 0.0 } else { light };
                for c in 0..3 {
                    reference[c] += light * CMFS[i][c];
                }
            }
            for (c, &expected) in reference.iter().enumerate() {
                let actual = scalar.xyz[c] * 10.0f64.powf(-scalar.coefficient * density);
                assert!((actual - expected).abs() <= 2e-13 * (1.0 + expected.abs()));
            }
        }
    }

    #[test]
    fn prepared_capture_matches_generic_sweep_endpoints_and_headroom() {
        let cd = [[1.13, 0.0, 0.0]; 5];
        let base = [0.07, 0.4, 0.23, 0.6, 0.15];
        let lamp = [0.7, 1.2, 1.1, 0.9, 0.4];
        let prepared = PreparedScanCpu::with_cmfs(&cd, &base, &lamp, &CMFS, 2.1, &CAT, &RGB);
        // Includes negative density/headroom, near-clear, and the epsilon-dominated tail.
        let densities = [-2.0, -0.1, 0.0, 1e-6, 0.2, 0.9, 2.0, 4.0, 12.0, 100.0];
        let image = ImageBuf::from_data(
            densities.len() as u32,
            1,
            densities
                .into_iter()
                .flat_map(|d| [from_f64(d), from_f64(0.7), from_f64(-0.2)])
                .collect(),
        );
        let expected =
            scan_spectral_cpu_with_cmfs(&image, &cd, &base, &lamp, &CMFS, 2.1, &CAT, &RGB);
        let actual = prepared.capture(&image);
        assert_rgb_close(&actual, &expected);
        assert!(actual.data[..3].iter().any(|&v| to_f64(v) > 1.0));
        assert!(
            actual.data[actual.data.len() - 3..]
                .iter()
                .all(|&v| to_f64(v).is_finite() && to_f64(v) != 0.0)
        );
        // Reusing the same owner for another tile must not rebuild or change response.
        let tile = ImageBuf::from_data(1, 1, vec![from_f64(1.7); 3]);
        assert_rgb_close(
            &prepared.capture(&tile),
            &scan_spectral_cpu_with_cmfs(&tile, &cd, &base, &lamp, &CMFS, 2.1, &CAT, &RGB),
        );
    }

    #[test]
    fn optional_base_truncation_and_nan_rows_match_generic_capture() {
        let cd = [
            [0.8, 0.0, 0.0],
            [f64::NAN, 0.0, 0.0],
            [0.8, 0.0, 0.0],
            [0.8, 0.0, 0.0],
            [7.0, 2.0, 1.0],
        ];
        // Effective n is four. The fifth non-scalar row is outside the integral.
        let lamp = [0.7, 1.0, 0.9, 1.1];
        let image = ImageBuf::from_data(2, 1, vec![from_f64(0.2); 6]);
        for base in [&[][..], &[0.3, 0.7][..], &[0.3, 0.7, f64::NAN, 0.1][..]] {
            assert!(ScalarScanCoefficients::prepare(&cd, base, &lamp, &CMFS).is_some());
            let prepared = PreparedScanCpu::with_cmfs(&cd, base, &lamp, &CMFS, 1.3, &CAT, &RGB);
            assert_rgb_close(
                &prepared.capture(&image),
                &scan_spectral_cpu_with_cmfs(&image, &cd, base, &lamp, &CMFS, 1.3, &CAT, &RGB),
            );
        }
    }

    #[test]
    fn nonconstant_or_multichannel_basis_retains_generic_response() {
        let base = [0.1; 5];
        let lamp = [1.0; 5];
        let image = ImageBuf::from_data(
            2,
            1,
            [0.0, 0.3, 0.8, 1.7, 0.1, -0.2]
                .into_iter()
                .map(from_f64)
                .collect(),
        );
        for replacement in [[0.81, 0.0, 0.0], [0.8, 0.01, 0.0], [0.8, 0.0, 0.02]] {
            let mut cd = [[0.8, 0.0, 0.0]; 5];
            cd[2] = replacement;
            assert!(ScalarScanCoefficients::prepare(&cd, &base, &lamp, &CMFS).is_none());
            let prepared = PreparedScanCpu::with_cmfs(&cd, &base, &lamp, &CMFS, 1.0, &CAT, &RGB);
            assert_rgb_close(
                &prepared.capture(&image),
                &scan_spectral_cpu_with_cmfs(&image, &cd, &base, &lamp, &CMFS, 1.0, &CAT, &RGB),
            );
        }
    }

    #[test]
    fn nonfinite_pixels_and_extreme_powers_use_generic_masking() {
        let cd = [[1.0, 0.0, 0.0]; 5];
        let base = [0.2; 5];
        let lamp = [1.0; 5];
        let prepared = PreparedScanCpu::with_cmfs(&cd, &base, &lamp, &CMFS, 1.0, &CAT, &RGB);
        for pixel in [
            [0.2, f64::NAN, 0.0],
            [f64::INFINITY, 0.0, 0.0],
            [400.0, 0.0, 0.0],
        ] {
            let image = ImageBuf::from_data(1, 1, pixel.into_iter().map(from_f64).collect());
            assert_rgb_close(
                &prepared.capture(&image),
                &scan_spectral_cpu_with_cmfs(&image, &cd, &base, &lamp, &CMFS, 1.0, &CAT, &RGB),
            );
        }
    }
}

#[cfg(test)]
mod exponent_domain_tests {
    use super::ScalarScanCoefficients;
    use crate::cpu_backend::{PreparedScanCpu, scan_spectral_cpu_with_cmfs};
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::{from_f64, to_f64};

    const IDENTITY: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    fn assert_same_classification_and_value(actual: &ImageBuf, reference: &ImageBuf) {
        let tolerance = if cfg!(feature = "precision-f64") {
            2e-12
        } else {
            2e-6
        };
        for (&actual, &reference) in actual.data.iter().zip(&reference.data) {
            let (actual, reference) = (to_f64(actual), to_f64(reference));
            if reference.is_nan() {
                assert!(actual.is_nan(), "expected NaN, got {actual}");
            } else if reference.is_infinite() {
                assert_eq!(actual, reference);
            } else {
                assert!(actual.is_finite());
                assert!(
                    (actual - reference).abs() <= tolerance * (1.0 + reference.abs()),
                    "{actual} != {reference}"
                );
            }
        }
    }

    #[test]
    fn prepared_capture_preserves_generic_overflow_before_lamp_rescaling() {
        let cd = [[1.0, 0.0, 0.0]];
        let base = [-300.0];
        let lamp = [1e-300];
        let cmfs = [[1.0; 3]];
        assert!(ScalarScanCoefficients::prepare(&cd, &base, &lamp, &cmfs).is_some());
        let prepared =
            PreparedScanCpu::with_cmfs(&cd, &base, &lamp, &cmfs, 1.0, &IDENTITY, &IDENTITY);
        let density = ImageBuf::from_data(1, 1, vec![from_f64(-100.0); 3]);
        let reference = scan_spectral_cpu_with_cmfs(
            &density, &cd, &base, &lamp, &cmfs, 1.0, &IDENTITY, &IDENTITY,
        );
        // The generic exponential overflows before multiplication by the tiny
        // lamp. Its non-finite colour result must not become a finite scalar one.
        assert!(reference.data.iter().any(|&v| !to_f64(v).is_finite()));
        assert_same_classification_and_value(&prepared.capture(&density), &reference);
    }

    #[test]
    fn prepared_capture_preserves_generic_underflow_before_lamp_rescaling() {
        let cd = [[1.0, 0.0, 0.0]];
        let base = [300.0];
        let lamp = [1e308];
        let cmfs = [[1.0; 3]];
        assert!(ScalarScanCoefficients::prepare(&cd, &base, &lamp, &cmfs).is_some());
        // This normalization makes a spuriously recovered factored signal
        // observable instead of hiding it below the ordinary log epsilon.
        let normalization = 1e-100;
        let prepared = PreparedScanCpu::with_cmfs(
            &cd,
            &base,
            &lamp,
            &cmfs,
            normalization,
            &IDENTITY,
            &IDENTITY,
        );
        let density = ImageBuf::from_data(1, 1, vec![from_f64(30.0); 3]);
        let reference = scan_spectral_cpu_with_cmfs(
            &density,
            &cd,
            &base,
            &lamp,
            &cmfs,
            normalization,
            &IDENTITY,
            &IDENTITY,
        );
        assert!(
            reference
                .data
                .iter()
                .all(|&v| to_f64(v).is_finite() && to_f64(v) < 1e-9)
        );
        assert_same_classification_and_value(&prepared.capture(&density), &reference);
    }

    #[test]
    fn signed_valid_weights_stay_generic_but_masked_rows_do_not_bound_exponents() {
        let cd = [[1.0, 0.0, 0.0]; 2];
        assert!(
            ScalarScanCoefficients::prepare(&cd, &[0.1; 2], &[1.0, -0.2], &[[1.0; 3]; 2],)
                .is_none()
        );
        assert!(
            ScalarScanCoefficients::prepare(&cd, &[0.1; 2], &[1.0; 2], &[[1.0; 3], [-0.2; 3]],)
                .is_none()
        );
        let masked_cd = [[1.0, 0.0, 0.0], [f64::NAN, 0.0, 0.0]];
        let base = [0.1, -1e300];
        let lamp = [1.0, -0.2];
        let cmfs = [[1.0; 3], [-0.2; 3]];
        let scalar = ScalarScanCoefficients::prepare(&masked_cd, &base, &lamp, &cmfs).unwrap();
        let bounds = scalar.exponent_bounds(-30.0, 30.0);
        assert!(bounds[0] < 0.0 && bounds[1] > 0.0);
        let density = ImageBuf::from_data(1, 1, vec![from_f64(0.4); 3]);
        let prepared =
            PreparedScanCpu::with_cmfs(&masked_cd, &base, &lamp, &cmfs, 1.0, &IDENTITY, &IDENTITY);
        let reference = scan_spectral_cpu_with_cmfs(
            &density, &masked_cd, &base, &lamp, &cmfs, 1.0, &IDENTITY, &IDENTITY,
        );
        assert_same_classification_and_value(&prepared.capture(&density), &reference);
    }

    #[test]
    fn subnormal_base_power_is_not_reused_after_density_rescaling() {
        let cd = [[1.0, 0.0, 0.0]];
        let base = [320.0];
        let lamp = [1e308];
        let cmfs = [[1.0; 3]];
        assert!(ScalarScanCoefficients::prepare(&cd, &base, &lamp, &cmfs).is_none());
        let prepared =
            PreparedScanCpu::with_cmfs(&cd, &base, &lamp, &cmfs, 1e100, &IDENTITY, &IDENTITY);
        let density = ImageBuf::from_data(1, 1, vec![from_f64(-100.0); 3]);
        let reference = scan_spectral_cpu_with_cmfs(
            &density, &cd, &base, &lamp, &cmfs, 1e100, &IDENTITY, &IDENTITY,
        );
        assert_same_classification_and_value(&prepared.capture(&density), &reference);
    }

    #[test]
    fn prepared_capture_keeps_intermediate_precision_before_rescaling() {
        let cd = [[1.0, 0.0, 0.0]];
        for (base, lamp, weight, normalization, density) in [
            (300.0, 1e-20, 1.0, 1e-300, -20.0),
            (0.0, 1e-300, 1e300, 1e-20, 20.0),
        ] {
            let base = [base];
            let lamp = [lamp];
            let cmfs = [[weight; 3]];
            let image = ImageBuf::from_data(1, 1, vec![from_f64(density); 3]);
            let prepared = PreparedScanCpu::with_cmfs(
                &cd,
                &base,
                &lamp,
                &cmfs,
                normalization,
                &IDENTITY,
                &IDENTITY,
            );
            let reference = scan_spectral_cpu_with_cmfs(
                &image,
                &cd,
                &base,
                &lamp,
                &cmfs,
                normalization,
                &IDENTITY,
                &IDENTITY,
            );
            assert!(to_f64(reference.data[0]) > 0.9);
            assert_same_classification_and_value(&prepared.capture(&image), &reference);
        }
    }
}
