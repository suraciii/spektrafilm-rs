//! Scan-stage appearance applied after negative conversion and before Tone &
//! Color. The canonical transform is linear-sRGB, so the stage converts from
//! and back to the output space at its boundary.
use rayon::prelude::*;
use spektrafilm_math::colorspace::{self, RgbColorSpace};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::from_f64;

use crate::params::ScanProfileParams;
use crate::params::scan_profile::{MAX_SCAN_PROFILE_KNOTS, ScanCurve};

type Matrix = [[f64; 3]; 3];
const IDENTITY: Matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

#[inline]
fn mul(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn sample(curve: &ScanCurve, x: f64) -> f64 {
    let x = x.clamp(f64::from(curve.x[0]), f64::from(curve.x[curve.x.len() - 1]));
    let mut upper = 1;
    while upper < curve.x.len() && f64::from(curve.x[upper]) < x {
        upper += 1;
    }
    if upper == curve.x.len() {
        return f64::from(curve.y[curve.y.len() - 1]);
    }
    let x0 = f64::from(curve.x[upper - 1]);
    let x1 = f64::from(curve.x[upper]);
    let y0 = f64::from(curve.y[upper - 1]);
    let y1 = f64::from(curve.y[upper]);
    if x1 == x0 {
        y1
    } else {
        y0 + (y1 - y0) * (x - x0) / (x1 - x0)
    }
}

struct Prepared<'a> {
    strength: f64,
    matrix: Option<Matrix>,
    curves: Option<[&'a ScanCurve; 3]>,
    to_srgb: Matrix,
    from_srgb: Matrix,
}

impl Prepared<'_> {
    fn new<'a>(
        params: &'a ScanProfileParams,
        output_space: &RgbColorSpace,
    ) -> Option<Prepared<'a>> {
        if !params.is_effective() {
            return None;
        }
        let srgb = colorspace::resolve("sRGB").expect("sRGB is registered");
        let to_srgb = if output_space.name == srgb.name {
            IDENTITY
        } else {
            colorspace::conversion_matrix(output_space, srgb)
        };
        let from_srgb = if output_space.name == srgb.name {
            IDENTITY
        } else {
            colorspace::conversion_matrix(srgb, output_space)
        };
        let curves = params.transform.curves.as_ref()?;
        Some(Prepared {
            strength: f64::from(params.strength),
            matrix: params
                .transform
                .matrix
                .as_ref()
                .map(|m| m.map(|row| row.map(f64::from))),
            curves: Some([&curves[0], &curves[1], &curves[2]]),
            to_srgb,
            from_srgb,
        })
    }

    fn pixel(&self, rgb: [f64; 3]) -> [f64; 3] {
        let source = mul(&self.to_srgb, rgb);
        let mut transformed = match self.matrix {
            Some(matrix) => mul(&matrix, source),
            None => source,
        };
        if let Some(curves) = self.curves {
            transformed = [
                sample(curves[0], transformed[0]),
                sample(curves[1], transformed[1]),
                sample(curves[2], transformed[2]),
            ];
        }
        let mixed =
            std::array::from_fn(|i| source[i] + self.strength * (transformed[i] - source[i]));
        mul(&self.from_srgb, mixed)
    }
}

/// Apply the scan-stage appearance in the output space. The caller owns route
/// applicability and stores the same controls in runtime parameters.
pub fn apply(image: &mut ImageBuf, params: &ScanProfileParams, output_space: &RgbColorSpace) {
    let Some(prepared) = Prepared::new(params, output_space) else {
        return;
    };
    image.data.par_chunks_exact_mut(3).for_each(|px| {
        let result = prepared.pixel([px[0] as f64, px[1] as f64, px[2] as f64]);
        for i in 0..3 {
            px[i] = from_f64(result[i]);
        }
    });
}

/// Prepare a resident pointwise pass. `None` means exact bypass.
pub fn gpu_params(
    params: &ScanProfileParams,
    output_space: &RgbColorSpace,
) -> Option<spektrafilm_backend::ScanProfileGpuParams> {
    let prepared = Prepared::new(params, output_space)?;
    let mut knots = [[0.0f32; 4]; 3 * MAX_SCAN_PROFILE_KNOTS];
    let mut counts = [0u32; 4];
    if let Some(curves) = prepared.curves {
        for (channel, curve) in curves.iter().enumerate() {
            counts[channel] = curve.x.len() as u32;
            for (index, (&x, &y)) in curve.x.iter().zip(&curve.y).enumerate() {
                knots[channel * MAX_SCAN_PROFILE_KNOTS + index] = [x, y, 0.0, 0.0];
            }
        }
    }
    Some(spektrafilm_backend::ScanProfileGpuParams {
        strength: prepared.strength as f32,
        has_matrix: u32::from(prepared.matrix.is_some()),
        to_srgb: pack(prepared.to_srgb),
        from_srgb: pack(prepared.from_srgb),
        matrix: pack(prepared.matrix.unwrap_or(IDENTITY)),
        knots,
        counts,
    })
}

fn pack(m: Matrix) -> [[f32; 3]; 3] {
    std::array::from_fn(|row| [m[row][0] as f32, m[row][1] as f32, m[row][2] as f32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::precision::from_f64;

    fn srgb() -> &'static RgbColorSpace {
        colorspace::resolve("sRGB").unwrap()
    }

    #[test]
    fn bypass_and_zero_strength_are_exact() {
        let mut image = ImageBuf::from_data(
            2,
            1,
            [0.1, 0.4, 0.8, -0.1, 1.2, 0.3]
                .into_iter()
                .map(from_f64)
                .collect(),
        );
        let before = image.data.clone();
        apply(&mut image, &ScanProfileParams::default(), srgb());
        assert_eq!(image.data, before);

        let mut params = crate::params::scan_profile::builtin_sp3000_inspired();
        params.strength = 0.0;
        apply(&mut image, &params, srgb());
        assert_eq!(image.data, before);
    }

    #[test]
    fn builtin_changes_a_midgray_and_returns_finite_values() {
        let mut image =
            ImageBuf::from_data(1, 1, [0.18, 0.18, 0.18].into_iter().map(from_f64).collect());
        let before = image.data[0];
        apply(
            &mut image,
            &crate::params::scan_profile::builtin_sp3000_inspired(),
            srgb(),
        );
        assert!(image.data.iter().all(|value| value.is_finite()));
        assert_ne!(image.data[0], before);
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use spektrafilm_math::precision::from_f64;

    fn srgb() -> &'static RgbColorSpace {
        colorspace::resolve("sRGB").unwrap()
    }

    #[test]
    fn builtin_scan_profile_changes_image_and_bypass_is_exact() {
        let mut image = ImageBuf::from_data(
            3,
            1,
            [0.1, 0.4, 0.8, 0.2, 0.5, 0.3, 0.6, 0.7, 0.9]
                .into_iter()
                .map(from_f64)
                .collect(),
        );
        let before = image.data.clone();
        let builtin = crate::params::scan_profile::builtin_sp3000_inspired();
        apply(&mut image, &builtin, srgb());
        assert_ne!(image.data, before);
        assert!(image.data.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn strength_zero_is_exact_bypass() {
        let mut image = ImageBuf::from_data(
            2,
            1,
            [0.1, 0.4, 0.8, -0.1, 1.2, 0.3]
                .into_iter()
                .map(from_f64)
                .collect(),
        );
        let before = image.data.clone();
        let mut params = crate::params::scan_profile::builtin_sp3000_inspired();
        params.strength = 0.0;
        apply(&mut image, &params, srgb());
        assert_eq!(image.data, before);
    }

    #[test]
    fn matrix_curve_applies_before_curves() {
        let mut image =
            ImageBuf::from_data(1, 1, [0.5, 0.5, 0.5].into_iter().map(from_f64).collect());
        let params = crate::params::ScanProfileParams {
            strength: 1.0,
            transform: crate::params::ScanProfileTransform::matrix_curve(
                [[1.1, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.9]],
                [
                    crate::params::ScanCurve::new(vec![0.0, 1.0], vec![0.0, 1.0]),
                    crate::params::ScanCurve::new(vec![0.0, 1.0], vec![0.0, 1.0]),
                    crate::params::ScanCurve::new(vec![0.0, 1.0], vec![0.0, 1.0]),
                ],
            ),
        };
        apply(&mut image, &params, srgb());
        // Matrix scales R by 1.1, B by 0.9; curves are identity; strength=1.
        assert!((image.data[0] as f64 - 0.55).abs() < 1e-6);
        assert!((image.data[1] as f64 - 0.5).abs() < 1e-6);
        assert!((image.data[2] as f64 - 0.45).abs() < 1e-6);
    }

    #[test]
    fn custom_curve_reports_custom_name() {
        let mut params = crate::params::scan_profile::builtin_sp3000_inspired();
        params.transform.curves.as_mut().unwrap()[0].y[4] += 0.01;
        assert_eq!(
            params.effective_name(),
            crate::params::ScanProfileName::Custom
        );
    }
}
