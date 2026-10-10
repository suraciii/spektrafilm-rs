//! Bundled magazine print appearance: CIELAB tone and chroma finishing.
//!
//! The definition is the selected vintage-magazine study: a monotone CIELAB
//! tone curve, hue-selective red/blue chroma gain, restrained green chroma, a
//! warm near-neutral highlight response, and a radial chroma fit to the
//! destination gamut. The study's reference conversion is a LittleCMS float
//! RGB-to-Lab transform; this module reproduces it with the shared CIE math and
//! Bradford adaptation (the ICC default) so both boundaries stay explicit.
//! `tests` pins the parity against the study's exported probes.
//!
//! The appearance maps the display-referred portion of the signal, the
//! `0..=1` interval of destination linear RGB. Values outside that interval
//! keep their residual, so floating headroom survives to the image writer.

use std::sync::LazyLock;

use rayon::prelude::*;
use spektrafilm_math::colorspace::{self, Cctf, RgbColorSpace};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::from_f64;

use crate::params::MagazinePrintColorParams;

/// Tone-curve input knots in CIELAB L*.
const TONE_IN: [f64; 10] = [0.0, 5.0, 10.0, 20.0, 35.0, 50.0, 65.0, 80.0, 92.0, 100.0];
/// Tone-curve output knots in CIELAB L*.
const TONE_OUT: [f64; 10] = [0.0, 3.5, 8.0, 22.0, 40.0, 56.0, 70.0, 83.0, 93.0, 99.0];

/// Hue center and angular width of each selective chroma weight, in degrees.
const RED_HUE: (f64, f64) = (35.0, 25.0);
const BLUE_HUE: (f64, f64) = (-65.0, 29.0);
const GREEN_HUE: (f64, f64) = (140.0, 30.0);

/// Chroma-fit bisection steps used by the reference study.
const GAMUT_STEPS: u32 = 13;

/// CIELAB piecewise breakpoint and slope constant (`6/29`, `29/3` cubed).
const LAB_EPSILON: f64 = 216.0 / 24389.0;
const LAB_KAPPA: f64 = 24389.0 / 27.0;

fn encode_headroom(value: f64, cctf: Cctf) -> f64 {
    if value < 0.0 {
        match cctf {
            Cctf::AdobeRgb1998 | Cctf::Gamma2_6 => -colorspace::cctf_encode(-value, cctf),
            _ => colorspace::cctf_encode(value, cctf),
        }
    } else {
        colorspace::cctf_encode(value, cctf)
    }
}

fn decode_headroom(value: f64, cctf: Cctf) -> f64 {
    if value < 0.0 {
        match cctf {
            Cctf::AdobeRgb1998 | Cctf::Gamma2_6 => -colorspace::cctf_decode(-value, cctf),
            _ => colorspace::cctf_decode(value, cctf),
        }
    } else {
        colorspace::cctf_decode(value, cctf)
    }
}

/// Monotone cubic Hermite curve with harmonic-mean interior slopes and
/// endpoint secant slopes — the reference study's tone curve.
struct ToneCurve {
    slopes: [f64; 10],
}

impl ToneCurve {
    fn vintage_magazine() -> Self {
        let mut interval = [0.0f64; 9];
        for i in 0..9 {
            interval[i] = (TONE_OUT[i + 1] - TONE_OUT[i]) / (TONE_IN[i + 1] - TONE_IN[i]);
        }
        let mut slopes = [0.0f64; 10];
        slopes[0] = interval[0];
        slopes[9] = interval[8];
        for i in 1..9 {
            slopes[i] = 2.0 / (1.0 / interval[i - 1] + 1.0 / interval[i]);
        }
        Self { slopes }
    }

    fn eval(&self, l: f64) -> f64 {
        if !l.is_finite() {
            return TONE_OUT[0];
        }
        if l <= TONE_IN[0] {
            return TONE_OUT[0];
        }
        if l >= TONE_IN[9] {
            return TONE_OUT[9];
        }
        let i = TONE_IN.partition_point(|&knot| knot <= l) - 1;
        let h = TONE_IN[i + 1] - TONE_IN[i];
        let t = (l - TONE_IN[i]) / h;
        let t2 = t * t;
        let t3 = t2 * t;
        (2.0 * t3 - 3.0 * t2 + 1.0) * TONE_OUT[i]
            + (t3 - 2.0 * t2 + t) * h * self.slopes[i]
            + (-2.0 * t3 + 3.0 * t2) * TONE_OUT[i + 1]
            + (t3 - t2) * h * self.slopes[i + 1]
    }
}

static TONE: LazyLock<ToneCurve> = LazyLock::new(ToneCurve::vintage_magazine);

fn smooth(x: f64) -> f64 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Gaussian weight on the shortest signed hue distance.
fn hue_weight(hue: f64, center_deg: f64, width_deg: f64) -> f64 {
    let center = center_deg.to_radians();
    let width = width_deg.to_radians();
    let d = (hue - center).sin().atan2((hue - center).cos());
    (-0.5 * (d / width).powi(2)).exp()
}

/// Full-strength appearance in CIELAB D50.
///
/// The mapped lightness keeps the source hue while the selective weights
/// dampen or lift chroma; the caller fits the result back to the destination
/// gamut. Low-chroma pixels keep their source position, so the treatment is
/// not a skin mask.
pub(crate) fn appearance(lab: [f64; 3]) -> [f64; 3] {
    let (l, a, b) = (lab[0], lab[1], lab[2]);
    let chroma = a.hypot(b);
    let hue = b.atan2(a);
    let chroma_weight = smooth((chroma - 20.0) / 28.0);
    let mid_weight = smooth((l - 8.0) / 15.0) * (1.0 - smooth((l - 78.0) / 18.0));
    let weight =
        |(center, width): (f64, f64)| hue_weight(hue, center, width) * chroma_weight * mid_weight;
    let red = weight(RED_HUE);
    let blue = weight(BLUE_HUE);
    let green = weight(GREEN_HUE);
    let mapped_l = TONE.eval(l) - 4.0 * red - 3.5 * blue;
    let chroma_scale = 1.0 + 0.22 * red + 0.10 * blue - 0.04 * green;
    let highlight = smooth((l - 75.0) / 25.0) * (1.0 - smooth(chroma / 18.0));
    [
        mapped_l,
        a * chroma_scale + 0.12 * highlight,
        b * chroma_scale + 0.7 * highlight,
    ]
}

#[inline]
fn mul3(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0f64; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

#[inline]
fn mul3v(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
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
    for c in 0..3 {
        xyz[c] *= colorspace::D50_XYZ_F64[c];
    }
    xyz
}

/// Destination linear RGB to CIELAB D50 and back, prepared once per image.
struct MagazineTransform {
    to_lab: [[f64; 3]; 3],
    from_lab: [[f64; 3]; 3],
}

impl MagazineTransform {
    fn new(space: &RgbColorSpace) -> Self {
        let white = space.whitepoint_xyz();
        let forward =
            colorspace::chromatic_adaptation_matrix_bradford_f64(white, colorspace::D50_XYZ_F64);
        let backward =
            colorspace::chromatic_adaptation_matrix_bradford_f64(colorspace::D50_XYZ_F64, white);
        Self {
            to_lab: mul3(&forward, &space.matrix_rgb_to_xyz),
            from_lab: mul3(&space.matrix_xyz_to_rgb, &backward),
        }
    }

    #[inline]
    fn to_lab(&self, linear: [f64; 3]) -> [f64; 3] {
        xyz_to_lab(mul3v(&self.to_lab, linear))
    }

    #[inline]
    fn to_rgb(&self, lab: [f64; 3]) -> [f64; 3] {
        mul3v(&self.from_lab, lab_to_xyz(lab))
    }
}

#[inline]
fn in_gamut(rgb: [f64; 3]) -> bool {
    rgb.iter().all(|value| (0.0..=1.0).contains(value))
}

/// Largest chroma scale at fixed mapped lightness and hue that fits the
/// destination gamut. Achromatic output at the mapped lightness always fits,
/// so the bisection interval starts at a valid lower bound.
fn gamut_chroma_scale(transform: &MagazineTransform, lab: [f64; 3]) -> f64 {
    if in_gamut(transform.to_rgb(lab)) {
        return 1.0;
    }
    let (mut low, mut high) = (0.0f64, 1.0f64);
    for _ in 0..GAMUT_STEPS {
        let mid = 0.5 * (low + high);
        let trial = [lab[0], lab[1] * mid, lab[2] * mid];
        if in_gamut(transform.to_rgb(trial)) {
            low = mid;
        } else {
            high = mid;
        }
    }
    low
}

fn map_pixel(
    rgb: [f64; 3],
    strength: f64,
    transform: &MagazineTransform,
    cctf: Cctf,
    encoded: bool,
) -> [f64; 3] {
    let linear = if encoded {
        rgb.map(|value| decode_headroom(value, cctf))
    } else {
        rgb
    };
    let mut base = [0.0f64; 3];
    let mut residual = [0.0f64; 3];
    for c in 0..3 {
        base[c] = linear[c].clamp(0.0, 1.0);
        residual[c] = linear[c] - base[c];
    }
    let mapped = appearance(transform.to_lab(base));
    let scale = gamut_chroma_scale(transform, mapped);
    let mapped_rgb = transform.to_rgb([mapped[0], mapped[1] * scale, mapped[2] * scale]);
    let mut out = [0.0f64; 3];
    for c in 0..3 {
        out[c] = base[c] + strength * (mapped_rgb[c] - base[c]) + residual[c];
    }
    if encoded {
        out.map(|value| encode_headroom(value, cctf).max(0.0))
    } else {
        out
    }
}

/// Apply the bundled magazine print appearance to output-space RGB.
///
/// `image` is linear when `encoded` is false; in that case the appearance
/// decodes the transfer state for its own domain and restores it afterwards,
/// leaving the caller's transfer state unchanged. The operation is in-place
/// and performs no image allocation. Inactive parameters and strength zero
/// are exact no-ops.
pub fn apply(
    image: &mut ImageBuf,
    params: &MagazinePrintColorParams,
    output_space: &RgbColorSpace,
    encoded: bool,
) {
    if !params.active || params.strength <= 0.0 {
        return;
    }
    let strength = params.strength.clamp(0.0, 1.0);
    let transform = MagazineTransform::new(output_space);
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
    use crate::params::MagazinePrintColorParams;
    use spektrafilm_math::colorspace;
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::{Scalar, from_f64};

    fn space() -> &'static colorspace::RgbColorSpace {
        colorspace::resolve("sRGB").unwrap()
    }

    fn enabled() -> MagazinePrintColorParams {
        MagazinePrintColorParams {
            active: true,
            strength: 1.0,
        }
    }

    #[test]
    fn inactive_and_zero_strength_are_exact_noops() {
        for params in [
            MagazinePrintColorParams {
                active: false,
                strength: 1.0,
            },
            MagazinePrintColorParams {
                active: true,
                strength: 0.0,
            },
            MagazinePrintColorParams {
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

    /// The exported reference probes and their study outputs.
    ///
    /// `expected` is the study's `color()` output for sRGB-encoded input. The
    /// study converts through LittleCMS; this module uses the shared CIE math
    /// with Bradford adaptation, so the budgets below bound that difference.
    fn reference_probes() -> (Vec<[f64; 3]>, Vec<[f64; 3]>) {
        const FIXTURE: &str = include_str!("fixtures/magazine_appearance_probes.json");
        let value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        let collect = |key: &str| {
            value[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| {
                    let row = row.as_array().unwrap();
                    [
                        row[0].as_f64().unwrap(),
                        row[1].as_f64().unwrap(),
                        row[2].as_f64().unwrap(),
                    ]
                })
                .collect::<Vec<[f64; 3]>>()
        };
        (collect("input"), collect("expected"))
    }

    #[test]
    fn matches_the_selected_study_within_budget() {
        let (input, expected) = reference_probes();
        assert!(input.len() >= 512);
        let mut worst = 0.0f64;
        let mut mean = 0.0f64;
        for (src, want) in input.iter().zip(&expected) {
            let got = map_pixel(
                *src,
                1.0,
                &MagazineTransform::new(space()),
                Cctf::Srgb,
                true,
            );
            for c in 0..3 {
                let delta = (got[c] - want[c]).abs();
                worst = worst.max(delta);
                mean += delta / 3.0;
            }
        }
        mean /= input.len() as f64;
        assert!(worst <= 2.0e-3, "worst channel delta {worst}");
        assert!(mean <= 2.0e-4, "mean channel delta {mean}");
    }

    #[test]
    fn neutral_ramp_stays_monotone_and_neutral() {
        let transform = MagazineTransform::new(space());
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
            if value < 0.2 {
                // Shadow neutrals carry no selective chroma and the highlight
                // response has no reach below L* 75, so the appearance must
                // return the shadow's own a*/b* unchanged.
                let lab = transform.to_lab([value; 3]);
                let mapped = appearance(lab);
                assert_eq!(mapped[1], lab[1], "shadow a* moved at {value}");
                assert_eq!(mapped[2], lab[2], "shadow b* moved at {value}");
                // The remaining asymmetry of the full path is the destination
                // space's own RGB → XYZ → RGB rounding (4-decimal matrices) at
                // ~1e-4 relative — orders below visibility. Guard the cast.
                let mean = (out[0] + out[1] + out[2]) / 3.0;
                let spread = out.iter().map(|c| (c - mean).abs()).fold(0.0, f64::max);
                assert!(
                    spread <= 1e-3 * mean,
                    "shadow neutral cast at {value}: {out:?}"
                );
            }
        }
    }

    #[test]
    fn strength_blends_the_full_appearance_in_linear_rgb() {
        let transform = MagazineTransform::new(space());
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
        let transform = MagazineTransform::new(space());
        let out = map_pixel([1.5, 0.4, 0.3], 1.0, &transform, Cctf::Srgb, false);
        assert!(out[0] > 1.0, "super-white red keeps its excess: {out:?}");
        let dark = map_pixel([-0.2, 0.3, 0.3], 1.0, &transform, Cctf::Srgb, false);
        assert!(dark[0] < -0.1, "negative headroom is preserved: {dark:?}");
        assert!(out.iter().all(|v| v.is_finite()) && dark.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn enabled_transform_changes_pixel() {
        let mut image =
            ImageBuf::from_data(1, 1, vec![from_f64(0.7), from_f64(0.2), from_f64(0.1)]);
        let before = image.data.clone();
        apply(&mut image, &enabled(), space(), true);
        assert_ne!(image.data, before);
    }

    #[test]
    fn neutral_gray_stays_neutral_when_disabled() {
        let mut image =
            ImageBuf::from_data(1, 1, vec![from_f64(0.4), from_f64(0.4), from_f64(0.4)]);
        apply(
            &mut image,
            &MagazinePrintColorParams {
                active: true,
                strength: 0.0,
            },
            space(),
            true,
        );
        assert_eq!(image.data[0], image.data[1]);
        assert_eq!(image.data[1], image.data[2]);
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
    fn wide_gamut_output_stays_finite_and_in_presentation_range() {
        for name in ["ITU-R BT.2020", "ProPhoto RGB", "Display P3", "ACES2065-1"] {
            let space = colorspace::resolve(name).unwrap();
            let transform = MagazineTransform::new(space);
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
                assert!(
                    out.iter().all(|v| (-1e-9..=1.0 + 1e-9).contains(v)),
                    "{name} {rgb:?} leaves the display interval: {out:?}"
                );
            }
        }
    }
}
