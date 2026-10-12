//! LabScan-style scanner finishing for positive scans.
//!
//! The constants and Oklab treatment are adapted from FotufilmCore's
//! `LabScanFinish.swift`, SHA-256
//! `fd2352193a8c91232729e51cc83cf748236b3a4b3cfaed07e703f5ae0943fe73`,
//! at pinned research revision `59d43471dbef45c85ae442e507a291fdefb2ec5`
//! (`Sources/FotufilmCore/LabScanFinish.swift`). Fotufilm project-authored
//! source is Apache-2.0 licensed; this port keeps source provenance here.

use rayon::prelude::*;
use spektrafilm_math::colorspace::{self, RgbColorSpace, SRGB};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};

const HIGHLIGHT_LIFT: f64 = 0.006;
const BLACK_PULL: f64 = 0.019;
const SHADOW_CAST: [f64; 2] = [-0.045, -0.033];
const CAST_SATURATION: f64 = 0.1;
const CORRECTION_FADE_START: f64 = 0.08;
const CORRECTION_FADE_END: f64 = 0.20;

#[derive(Clone, Copy)]
struct HueCorrection {
    hue: f64,
    width: f64,
    chroma: f64,
    turn: f64,
}

const CORRECTIONS: [HueCorrection; 3] = [
    HueCorrection {
        hue: 45.0,
        width: 35.0,
        chroma: 0.97,
        turn: 2.3,
    },
    HueCorrection {
        hue: 135.0,
        width: 35.0,
        chroma: 0.89,
        turn: 5.5,
    },
    HueCorrection {
        hue: 240.0,
        width: 35.0,
        chroma: 1.05,
        turn: -6.4,
    },
];

#[inline]
fn clamp(value: f64, low: f64, high: f64) -> f64 {
    value.clamp(low, high)
}

#[inline]
fn mul(matrix: &[[f64; 3]; 3], rgb: [f64; 3]) -> [f64; 3] {
    [
        matrix[0][0] * rgb[0] + matrix[0][1] * rgb[1] + matrix[0][2] * rgb[2],
        matrix[1][0] * rgb[0] + matrix[1][1] * rgb[1] + matrix[1][2] * rgb[2],
        matrix[2][0] * rgb[0] + matrix[2][1] * rgb[1] + matrix[2][2] * rgb[2],
    ]
}

#[inline]
fn oklab(rgb: [f64; 3]) -> [f64; 3] {
    let l = (0.4122214708 * rgb[0] + 0.5363325363 * rgb[1] + 0.0514459929 * rgb[2]).cbrt();
    let m = (0.2119034982 * rgb[0] + 0.6806995451 * rgb[1] + 0.1073969566 * rgb[2]).cbrt();
    let s = (0.0883024619 * rgb[0] + 0.2817188376 * rgb[1] + 0.6299787005 * rgb[2]).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

#[inline]
fn linear_from_oklab(lab: [f64; 3]) -> [f64; 3] {
    let l = lab[0] + 0.3963377774 * lab[1] + 0.2158037573 * lab[2];
    let m = lab[0] - 0.1055613458 * lab[1] - 0.0638541728 * lab[2];
    let s = lab[0] - 0.0894841775 * lab[1] - 1.2914855480 * lab[2];
    let l3 = l * l * l;
    let m3 = m * m * m;
    let s3 = s * s * s;
    [
        4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
        -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
        -0.0041960863 * l3 - 0.7034186147 * m3 + 1.7076147010 * s3,
    ]
}

#[inline]
fn graded(rgb: [f64; 3], weights: [f64; 3], strength: f64) -> [f64; 3] {
    let luminance = weights[0] * rgb[0] + weights[1] * rgb[1] + weights[2] * rgb[2];
    if luminance <= 1e-6 || !luminance.is_finite() {
        return rgb;
    }
    let level = luminance.cbrt();
    let upper = clamp((level - 0.55) / 0.45, 0.0, 1.0);
    let lifted = level
        + strength
            * (HIGHLIGHT_LIFT * (std::f64::consts::PI * upper).sin()
                - BLACK_PULL * clamp(1.0 - level / 0.35, 0.0, 1.0));
    let target = lifted.max(0.0);
    let scale = target * target * target / luminance;
    [rgb[0] * scale, rgb[1] * scale, rgb[2] * scale]
}

#[inline]
fn finish_pixel(
    rgb: [f64; 3],
    output_to_srgb: &[[f64; 3]; 3],
    srgb_to_output: &[[f64; 3]; 3],
    weights: [f64; 3],
    strength: f64,
    chromatic: bool,
) -> [f64; 3] {
    let toned = graded(rgb, weights, strength);
    if !chromatic {
        return toned;
    }
    let mut lab = oklab(mul(output_to_srgb, toned));
    let level = lab[0];
    // The simple core API has no per-frame floor; LabScanFinish's default
    // floor of zero makes the crossover onset fully active.
    let onset = 1.0;
    let shadow =
        clamp((0.70 - level) / 0.40, 0.0, 1.0) * clamp((level - 0.04) / 0.16, 0.0, 1.0) * onset;
    let chroma = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
    let saturation = chroma / level.max(1e-3);
    let neutral = level * (-saturation / CAST_SATURATION).exp();
    let cast = strength * neutral * shadow;
    lab[1] += cast * SHADOW_CAST[0];
    lab[2] += cast * SHADOW_CAST[1];

    let chroma = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
    let hue = lab[2].atan2(lab[1]) * 180.0 / std::f64::consts::PI;
    let mut gain = 1.0;
    let mut turn = 0.0;
    for correction in CORRECTIONS {
        let mut distance = (hue - correction.hue) % 360.0;
        if distance > 180.0 {
            distance -= 360.0;
        }
        if distance < -180.0 {
            distance += 360.0;
        }
        let ratio = distance / correction.width;
        let weight = (-0.5 * ratio * ratio).exp();
        gain *= 1.0 + strength * (correction.chroma - 1.0) * weight;
        turn += strength * correction.turn * weight;
    }
    let saturated = clamp(
        (chroma - CORRECTION_FADE_START) / (CORRECTION_FADE_END - CORRECTION_FADE_START),
        0.0,
        1.0,
    );
    let smooth = saturated * saturated * (3.0 - 2.0 * saturated);
    let fade =
        clamp(level / 0.25, 0.0, 1.0) * clamp((1.0 - level) / 0.08, 0.0, 1.0) * (1.0 - smooth);
    let corrected_chroma = chroma * (1.0 + (gain - 1.0) * fade);
    let corrected_hue = (hue + turn * fade) * std::f64::consts::PI / 180.0;
    lab[1] = corrected_chroma * corrected_hue.cos();
    lab[2] = corrected_chroma * corrected_hue.sin();
    mul(srgb_to_output, linear_from_oklab(lab))
}

/// Apply the LabScan finish in-place to linear RGB in `output_space`.
///
/// `strength == 0` is an exact bypass. Monochrome scans (`chromatic == false`)
/// receive only the gradation and therefore preserve channel neutrality.
pub fn apply(image: &mut ImageBuf, output_space: &RgbColorSpace, strength: f64, chromatic: bool) {
    if strength == 0.0 {
        return;
    }
    let strength = if strength.is_finite() {
        strength.clamp(0.0, 1.0)
    } else {
        1.0
    };
    if strength == 0.0 {
        return;
    }
    let output_to_srgb = colorspace::conversion_matrix(output_space, &SRGB);
    let srgb_to_output = colorspace::conversion_matrix(&SRGB, output_space);
    let weights = output_space.matrix_rgb_to_xyz[1];
    image.data.par_chunks_exact_mut(3).for_each(|pixel| {
        let rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
        let finished = finish_pixel(
            rgb,
            &output_to_srgb,
            &srgb_to_output,
            weights,
            strength,
            chromatic,
        );
        pixel[0] = from_f64(finished[0]);
        pixel[1] = from_f64(finished[1]);
        pixel[2] = from_f64(finished[2]);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::colorspace::DISPLAY_P3;
    use spektrafilm_math::precision::to_f64;

    #[test]
    fn zero_strength_is_an_exact_bypass() {
        let mut image =
            ImageBuf::from_data(1, 1, vec![from_f64(-0.2), from_f64(0.5), from_f64(1.7)]);
        let before = image.data.clone();
        apply(&mut image, &DISPLAY_P3, 0.0, true);
        assert_eq!(image.data, before);
    }

    #[test]
    fn monochrome_finish_stays_neutral() {
        let mut image =
            ImageBuf::from_data(1, 1, vec![from_f64(0.3), from_f64(0.3), from_f64(0.3)]);
        apply(&mut image, &DISPLAY_P3, 1.0, false);
        assert!((to_f64(image.data[0]) - to_f64(image.data[1])).abs() < 1e-12);
        assert!((to_f64(image.data[1]) - to_f64(image.data[2])).abs() < 1e-12);
    }

    #[test]
    fn chromatic_finish_changes_hue_and_remains_finite_extended_range() {
        let mut image =
            ImageBuf::from_data(1, 1, vec![from_f64(0.26), from_f64(0.12), from_f64(0.08)]);
        let before = image.data.clone();
        apply(&mut image, &DISPLAY_P3, 1.0, true);
        assert!(image.data.iter().all(|v| to_f64(*v).is_finite()));
        assert!(
            image
                .data
                .iter()
                .zip(before)
                .any(|(a, b)| (to_f64(*a) - to_f64(b)).abs() > 1e-10)
        );

        let mut extended =
            ImageBuf::from_data(1, 1, vec![from_f64(-0.4), from_f64(1.8), from_f64(0.2)]);
        apply(&mut extended, &DISPLAY_P3, 1.0, true);
        assert!(extended.data.iter().all(|v| to_f64(*v).is_finite()));
    }
}
