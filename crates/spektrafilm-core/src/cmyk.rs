//! CMYK ink style (`cmyk_ink_v1`): the terminal RGB appearance boundary.
//!
//! The definition is the fixed deterministic Lab grade from
//! `specs/cmyk-style`: a chroma-dependent lightness reduction with a
//! shadow-protection smoothstep and a uniform chroma scale, converted
//! through CIELAB D50 with Bradford adaptation (the ICC default). The
//! research `ink` grade supplies the constants; its offline FOGRA/YNSN
//! paper model is not a runtime dependency.
//!
//! The appearance runs once after every scanner-domain effect, before
//! output mix and writing. It preserves the caller's output primaries and
//! transfer state, keeps three-channel RGB, and preserves finite signed
//! headroom: values outside the display interval are not clipped here —
//! the existing writer rules own final integer clipping.

use rayon::prelude::*;
use spektrafilm_math::colorspace::{self, Cctf, RgbColorSpace};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::from_f64;

use crate::math::{mat_mul, mat_vec};
use crate::params::CmykParams;

/// `grade.k_l`: chroma-dependent lightness reduction.
const K_L: f64 = 0.28;
/// `grade.k_c`: chroma scale.
const K_C: f64 = 1.40;
/// `grade.contrast`: lightness contrast.
const CONTRAST: f64 = 1.03;
/// `grade.smoothstep`: shadow-protection interval in L*.
const SMOOTHSTEP_LOW: f64 = 8.0;
const SMOOTHSTEP_HIGH: f64 = 45.0;
/// Lightness floor of the grade (`max(4, …)`).
const L_FLOOR: f64 = 4.0;

/// CIELAB piecewise breakpoint and slope constant (`6/29`, `29/3` cubed).
const LAB_EPSILON: f64 = 216.0 / 24389.0;
const LAB_KAPPA: f64 = 24389.0 / 27.0;

/// Decode/encode at finite signed headroom: every registered space uses
/// sign-preserving powers, so the transfer stays finite and the writer,
/// not this boundary, owns final clipping.
fn encode_headroom(value: f64, cctf: Cctf) -> f64 {
    colorspace::cctf_encode(value, cctf)
}

fn decode_headroom(value: f64, cctf: Cctf) -> f64 {
    colorspace::cctf_decode(value, cctf)
}

/// `smoothstep(x) = t*t*(3 - 2*t)` with `t = clamp(x, 0, 1)`.
fn smoothstep(x: f64) -> f64 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Full-strength `cmyk_ink_v1` appearance in CIELAB D50.
///
/// `L' = max(4, 50 + (L - 50) * 1.03 - 0.28 * C * s)` with
/// `s = smoothstep((L - 8) / 37)`, and `a' = 1.40 * a`, `b' = 1.40 * b`.
/// No toe lift, no black-point compensation, no hue masks.
pub(crate) fn appearance(lab: [f64; 3]) -> [f64; 3] {
    let (l, a, b) = (lab[0], lab[1], lab[2]);
    let chroma = a.hypot(b);
    let s = smoothstep((l - SMOOTHSTEP_LOW) / (SMOOTHSTEP_HIGH - SMOOTHSTEP_LOW));
    let mapped_l = (50.0 + (l - 50.0) * CONTRAST - K_L * chroma * s).max(L_FLOOR);
    [mapped_l, K_C * a, K_C * b]
}

fn xyz_to_lab(xyz: [f64; 3]) -> [f64; 3] {
    let mut f = [0.0f64; 3];
    for c in 0..3 {
        let t = xyz[c] / colorspace::D50_XYZ_F64[c];
        f[c] = if t > LAB_EPSILON {
            t.cbrt()
        } else {
            (LAB_KAPPA * t + 16.0) / 116.0
        };
    }
    [
        116.0 * f[1] - 16.0,
        500.0 * (f[0] - f[1]),
        200.0 * (f[1] - f[2]),
    ]
}

fn lab_to_xyz(lab: [f64; 3]) -> [f64; 3] {
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    let inverse = |f: f64| {
        if f * f * f > LAB_EPSILON {
            f * f * f
        } else {
            (116.0 * f - 16.0) / LAB_KAPPA
        }
    };
    let mut xyz = [inverse(fx), inverse(fy), inverse(fz)];
    // The channel index aligns XYZ with the reference whitepoint.
    #[allow(clippy::needless_range_loop)]
    for c in 0..3 {
        xyz[c] *= colorspace::D50_XYZ_F64[c];
    }
    xyz
}

/// Destination linear RGB to CIELAB D50 and back, prepared once per image.
/// Uses the declared output color space and Bradford adaptation, so the
/// transform never assumes sRGB.
pub(crate) struct CmykTransform {
    to_lab: [[f64; 3]; 3],
    from_lab: [[f64; 3]; 3],
}

impl CmykTransform {
    pub(crate) fn new(space: &RgbColorSpace) -> Self {
        let white = space.whitepoint_xyz();
        let forward =
            colorspace::chromatic_adaptation_matrix_bradford_f64(white, colorspace::D50_XYZ_F64);
        let backward =
            colorspace::chromatic_adaptation_matrix_bradford_f64(colorspace::D50_XYZ_F64, white);
        Self {
            to_lab: mat_mul(&forward, &space.matrix_rgb_to_xyz),
            from_lab: mat_mul(&space.matrix_xyz_to_rgb, &backward),
        }
    }

    #[inline]
    pub(crate) fn to_lab(&self, linear: [f64; 3]) -> [f64; 3] {
        xyz_to_lab(mat_vec(&self.to_lab, linear))
    }

    #[inline]
    pub(crate) fn to_rgb(&self, lab: [f64; 3]) -> [f64; 3] {
        mat_vec(&self.from_lab, lab_to_xyz(lab))
    }

    /// Prepared Lab matrices for the device pass (pipe architecture, S4):
    /// destination RGB → CIELAB D50 and back. CPU and GPU share these
    /// constants.
    pub(crate) fn gpu_matrices(&self) -> ([[f64; 3]; 3], [[f64; 3]; 3]) {
        (self.to_lab, self.from_lab)
    }
}

fn map_pixel(
    rgb: [f64; 3],
    strength: f64,
    transform: &CmykTransform,
    cctf: Cctf,
    encoded: bool,
) -> [f64; 3] {
    let linear = if encoded {
        rgb.map(|value| decode_headroom(value, cctf))
    } else {
        rgb
    };
    let mapped = appearance(transform.to_lab(linear));
    let mapped_rgb = transform.to_rgb(mapped);
    let mut out = [0.0f64; 3];
    for c in 0..3 {
        out[c] = linear[c] + strength * (mapped_rgb[c] - linear[c]);
    }
    if encoded {
        out.map(|value| encode_headroom(value, cctf))
    } else {
        out
    }
}

/// Apply the CMYK ink style appearance to output-space RGB.
///
/// `image` is linear when `encoded` is false; in that case the appearance
/// decodes the transfer state for its own domain and restores it
/// afterwards, leaving the caller's transfer state unchanged. The
/// operation is in-place and performs no image allocation. Inactive
/// parameters and strength zero are exact no-ops.
pub fn apply(
    image: &mut ImageBuf,
    params: &CmykParams,
    output_space: &RgbColorSpace,
    encoded: bool,
) {
    if !params.active || params.strength <= 0.0 {
        return;
    }
    let strength = params.strength.clamp(0.0, 1.0);
    let transform = CmykTransform::new(output_space);
    let cctf = output_space.cctf;
    image.data.par_chunks_exact_mut(3).for_each(|px| {
        let out = map_pixel(
            [px[0] as f64, px[1] as f64, px[2] as f64],
            strength,
            &transform,
            cctf,
            encoded,
        );
        for c in 0..3 {
            px[c] = from_f64(out[c]);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::CmykParams;
    use spektrafilm_math::colorspace;
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::{Scalar, from_f64};

    fn space() -> &'static colorspace::RgbColorSpace {
        colorspace::resolve("sRGB").unwrap()
    }

    fn enabled() -> CmykParams {
        CmykParams {
            active: true,
            strength: 1.0,
        }
    }

    #[test]
    fn inactive_and_zero_strength_are_exact_noops() {
        for params in [
            CmykParams {
                active: false,
                strength: 1.0,
            },
            CmykParams {
                active: true,
                strength: 0.0,
            },
            CmykParams {
                active: true,
                strength: -1.0,
            },
        ] {
            let mut image =
                ImageBuf::from_data(1, 1, vec![from_f64(0.3), from_f64(0.4), from_f64(-0.1)]);
            let before = image.data.clone();
            apply(&mut image, &params, space(), true);
            assert_eq!(image.data, before);
        }
    }

    #[test]
    fn grade_matches_the_frozen_equations() {
        // L' = max(4, 50 + (L-50)*1.03 - 0.28*C*s), a' = 1.40a, b' = 1.40b.
        let lab: [f64; 3] = [40.0, 12.0, -8.0];
        let chroma = lab[1].hypot(lab[2]);
        let s = smoothstep((lab[0] - 8.0) / 37.0);
        let want_l = (50.0 + (lab[0] - 50.0) * 1.03 - 0.28 * chroma * s).max(4.0);
        let got = appearance(lab);
        assert!((got[0] - want_l).abs() < 1e-12, "L' {got:?} vs {want_l}");
        assert!((got[1] - 1.40 * lab[1]).abs() < 1e-12);
        assert!((got[2] - 1.40 * lab[2]).abs() < 1e-12);
    }

    #[test]
    fn lightness_floor_holds_in_the_deep_shadow() {
        // A very dark, very chromatic pixel must not fall below L* 4.
        let got = appearance([1.0, 60.0, 60.0]);
        assert!(got[0] >= 4.0 - 1e-12, "floor violated: {got:?}");
    }

    #[test]
    fn smoothstep_boundaries_are_exact() {
        // Below L* 8 the shadow-protection weight is 0; above L* 45 it is 1.
        let below = appearance([7.0, 30.0, 0.0]);
        assert!(
            (below[0] - (50.0 + (7.0 - 50.0) * 1.03)).abs() < 1e-12,
            "shadow weight must be zero below L* 8: {below:?}"
        );
        let above = appearance([46.0, 30.0, 0.0]);
        let chroma = 30.0;
        let want = 50.0 + (46.0 - 50.0) * 1.03 - 0.28 * chroma;
        assert!(
            (above[0] - want).abs() < 1e-12,
            "shadow weight must be one above L* 45: {above:?} vs {want}"
        );
    }

    #[test]
    fn neutral_ramp_stays_monotone_and_neutral() {
        let transform = CmykTransform::new(space());
        let mut previous = -1.0f64;
        for step in 0..=100 {
            let value = step as f64 / 100.0;
            let out = map_pixel([value; 3], 1.0, &transform, Cctf::Srgb, false);
            let lightness = transform.to_lab(out)[0];
            assert!(
                lightness >= previous - 1e-6,
                "neutral ramp reversed at {value}: {lightness} after {previous}"
            );
            previous = lightness;
            // A neutral stays neutral: a* and b* are scaled by 1.40 from
            // (near) zero, and the RGB round-trip keeps the cast bounded.
            let mean = (out[0] + out[1] + out[2]) / 3.0;
            let spread = out.iter().map(|c| (c - mean).abs()).fold(0.0, f64::max);
            assert!(
                spread <= 1e-3 * mean.max(1e-6),
                "neutral cast at {value}: {out:?}"
            );
        }
    }

    #[test]
    fn strength_blends_the_full_appearance_in_linear_rgb() {
        let transform = CmykTransform::new(space());
        for rgb in [[0.7, 0.2, 0.1], [0.2, 0.6, 0.3], [0.9, 0.8, 0.7]] {
            let full = map_pixel(rgb, 1.0, &transform, Cctf::Srgb, false);
            let half = map_pixel(rgb, 0.5, &transform, Cctf::Srgb, false);
            for c in 0..3 {
                let expected = 0.5 * (rgb[c] + full[c]);
                assert!(
                    (half[c] - expected).abs() < 1e-12,
                    "strength must blend base and appearance linearly"
                );
            }
        }
    }

    #[test]
    fn headroom_outside_the_display_interval_is_preserved() {
        let transform = CmykTransform::new(space());
        let out = map_pixel([1.5, 0.4, 0.3], 1.0, &transform, Cctf::Srgb, false);
        assert!(out[0] > 1.0, "super-white red keeps its excess: {out:?}");
        let dark = map_pixel([-0.2, 0.3, 0.3], 1.0, &transform, Cctf::Srgb, false);
        assert!(dark[0] < -0.1, "negative headroom is preserved: {dark:?}");
        assert!(out.iter().all(|v| v.is_finite()) && dark.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn encoded_pure_power_output_stays_decodable() {
        let mut image =
            ImageBuf::from_data(1, 1, vec![from_f64(0.0), from_f64(0.0), from_f64(0.0)]);
        apply(
            &mut image,
            &enabled(),
            colorspace::resolve("DCI-P3").unwrap(),
            true,
        );
        assert!(
            image
                .data
                .iter()
                .map(|value: &Scalar| {
                    colorspace::cctf_decode(*value as f64, colorspace::Cctf::Gamma2_6)
                })
                .all(f64::is_finite)
        );
    }

    #[test]
    fn linear_api_returns_linear_finite_headroom() {
        let mut image =
            ImageBuf::from_data(1, 1, vec![from_f64(-0.2), from_f64(0.4), from_f64(1.5)]);
        apply(
            &mut image,
            &enabled(),
            colorspace::resolve("DCI-P3").unwrap(),
            false,
        );
        assert!(image.data.iter().all(|v: &Scalar| (*v as f64).is_finite()));
    }

    #[test]
    fn wide_gamut_output_stays_finite() {
        for name in ["ITU-R BT.2020", "ProPhoto RGB", "Display P3", "ACES2065-1"] {
            let space = colorspace::resolve(name).unwrap();
            let transform = CmykTransform::new(space);
            for rgb in [
                [0.0, 0.0, 0.0],
                [1.0, 1.0, 1.0],
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
            ] {
                let out = map_pixel(rgb, 1.0, &transform, space.cctf, false);
                assert!(
                    out.iter().all(|v| v.is_finite()),
                    "{name} {rgb:?} -> {out:?}"
                );
            }
        }
    }
}
