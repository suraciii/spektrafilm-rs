//! Bundled RGB appearance for the magazine-print route.
use rayon::prelude::*;
use spektrafilm_math::colorspace::{self, RgbColorSpace};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::from_f64;

use crate::params::MagazinePrintColorParams;

const PAPER: [f64; 3] = [1.02, 0.99, 0.93];
const CHROMA_COMPRESSION: f64 = 0.14;
const SHADOW_CYAN: f64 = 0.035;
const BLACK_LIFT: f64 = 0.025;
const PAPER_COMPRESSION: f64 = 0.94;

fn encode_headroom(value: f64, cctf: colorspace::Cctf) -> f64 {
    if value < 0.0 {
        match cctf {
            colorspace::Cctf::AdobeRgb1998 | colorspace::Cctf::Gamma2_6 => {
                -colorspace::cctf_encode(-value, cctf)
            }
            _ => colorspace::cctf_encode(value, cctf),
        }
    } else {
        colorspace::cctf_encode(value, cctf)
    }
}

fn decode_headroom(value: f64, cctf: colorspace::Cctf) -> f64 {
    if value < 0.0 {
        match cctf {
            colorspace::Cctf::AdobeRgb1998 | colorspace::Cctf::Gamma2_6 => {
                -colorspace::cctf_decode(-value, cctf)
            }
            _ => colorspace::cctf_decode(value, cctf),
        }
    } else {
        colorspace::cctf_decode(value, cctf)
    }
}

/// Apply the fixed first-version magazine print appearance in output-space RGB.
///
/// `image` is linear when `encoded` is false; in that case a temporary CCTF
/// round-trip provides the output-referred domain without changing the export
/// transfer state. The operation is in-place and performs no image allocation.
pub(crate) fn apply_pixel(
    mut rgb: [f64; 3],
    params: &MagazinePrintColorParams,
    output_space: &RgbColorSpace,
    encoded: bool,
) -> [f64; 3] {
    if !params.active || params.strength == 0.0 {
        return rgb;
    }
    let strength = params.strength.clamp(0.0, 1.0);
    if !encoded {
        rgb = rgb.map(|v| encode_headroom(v, output_space.cctf));
    }
    let y = (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]).max(0.0);
    let paper_weight = (y * 1.25).min(1.0);
    let shadow_weight = (1.0 - y).clamp(0.0, 1.0);
    let chroma = [rgb[0] - y, rgb[1] - y, rgb[2] - y];
    let chroma_scale = 1.0 - CHROMA_COMPRESSION * strength * (0.35 + 0.65 * y);
    let mut out = [0.0; 3];
    for c in 0..3 {
        let compressed = y + chroma[c] * chroma_scale;
        let paper = PAPER[c] * (BLACK_LIFT + PAPER_COMPRESSION * y);
        out[c] = compressed * (1.0 - strength * 0.10 * paper_weight)
            + paper * (strength * 0.10 * paper_weight);
    }
    out[0] -= strength * SHADOW_CYAN * shadow_weight;
    out[1] += strength * SHADOW_CYAN * 0.55 * shadow_weight;
    out[2] += strength * SHADOW_CYAN * shadow_weight;
    if !encoded {
        out = out.map(|v| decode_headroom(v, output_space.cctf));
    } else {
        // Pure-power consumers intentionally reject negative encoded values.
        // Keep the appearance finite for Adobe RGB and DCI-P3 output.
        out = out.map(|v| v.max(0.0));
    }
    out
}

pub fn apply(
    image: &mut ImageBuf,
    params: &MagazinePrintColorParams,
    output_space: &RgbColorSpace,
    encoded: bool,
) {
    image.data.par_chunks_exact_mut(3).for_each(|px| {
        let out = apply_pixel(
            [px[0] as f64, px[1] as f64, px[2] as f64],
            params,
            output_space,
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
        ] {
            let mut image =
                ImageBuf::from_data(1, 1, vec![from_f64(0.3), from_f64(0.4), from_f64(0.5)]);
            let before = image.data.clone();
            apply(&mut image, &params, space(), true);
            assert_eq!(image.data, before);
        }
    }
    #[test]
    fn enabled_transform_changes_pixel() {
        let mut image =
            ImageBuf::from_data(1, 1, vec![from_f64(0.7), from_f64(0.2), from_f64(0.1)]);
        let before = image.data.clone();
        apply(
            &mut image,
            &MagazinePrintColorParams {
                active: true,
                strength: 1.0,
            },
            space(),
            true,
        );
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
            &MagazinePrintColorParams {
                active: true,
                strength: 1.0,
            },
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
            &MagazinePrintColorParams {
                active: true,
                strength: 1.0,
            },
            colorspace::resolve("DCI-P3").unwrap(),
            false,
        );
        assert!(image.data.iter().all(|v: &Scalar| (*v as f64).is_finite()));
    }
}
