//! Common positive-image finishing in fixed linear sRGB and Oklab.
//!
//! Coefficients define model `common_finishing_v1`; the public controls are
//! normalized and all preparation is independent of the image contents.
use rayon::prelude::*;
use spektrafilm_math::colorspace::{self, RgbColorSpace};
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::from_f64;

use crate::WorkflowRoute;
use crate::params::{FinishingParams, RuntimeParams};

type Matrix = [[f64; 3]; 3];
const IDENTITY: Matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
// Public linear-sRGB Oklab definition. The inverse matrices are derived from
// these exact forward coefficients rather than rounded published inverses.
const RGB_TO_LMS: Matrix = [
    [0.4122214708, 0.5363325363, 0.0514459929],
    [0.2119034982, 0.6806995451, 0.1073969566],
    [0.0883024619, 0.2817188376, 0.6299787005],
];
const LMS_TO_LAB: Matrix = [
    [0.2104542553, 0.7936177850, -0.0040720468],
    [1.9779984951, -2.4285922050, 0.4505937099],
    [0.0259040371, 0.7827717662, -0.8086757660],
];
const BRIGHTNESS_SCALE: f64 = 0.08;
const CONTRAST_SCALE: f64 = 0.10;
const WARMTH_SCALE: f64 = 0.012;
const SATURATION_SCALE: f64 = 0.40;
const CHROMA_REFERENCE: f64 = 0.15;
const WARM_A: f64 = 0.35;
const WARM_B: f64 = 1.0;

/// Applicability depends on the completed route and the actual scanned medium.
/// Paper reflection is a positive reading even for negative-type paper.
pub fn is_applicable(params: &RuntimeParams, medium_is_positive: bool) -> bool {
    let route = params.workflow.route_enum();
    let completed = matches!(
        route,
        WorkflowRoute::FilmScan
            | WorkflowRoute::FilmPrintScan
            | WorkflowRoute::StoredFilmScan
            | WorkflowRoute::StoredFilmPrintScan
            | WorkflowRoute::StoredPrintScan
            | WorkflowRoute::ConvertFilmPrintScan
            | WorkflowRoute::ConvertFilmScan
    );
    completed
        && (medium_is_positive
            || (matches!(
                route,
                WorkflowRoute::FilmScan
                    | WorkflowRoute::StoredFilmScan
                    | WorkflowRoute::ConvertFilmScan
            ) && params.scanner.scan_output == "negative_to_positive"))
}

#[inline]
fn mul(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn inverse(m: Matrix) -> Matrix {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let inv = 1.0 / det;
    [
        [
            (e * i - f * h) * inv,
            (c * h - b * i) * inv,
            (b * f - c * e) * inv,
        ],
        [
            (f * g - d * i) * inv,
            (a * i - c * g) * inv,
            (c * d - a * f) * inv,
        ],
        [
            (d * h - e * g) * inv,
            (b * g - a * h) * inv,
            (a * e - b * d) * inv,
        ],
    ]
}

struct Prepared {
    tone: [f64; 4],
    color: [f64; 3],
    expand: Option<(f64, f64)>,
    to_srgb: Matrix,
    from_srgb: Matrix,
    lab_to_lms: Matrix,
    lms_to_rgb: Matrix,
}

impl Prepared {
    fn new(params: &FinishingParams, output_space: &RgbColorSpace) -> Self {
        let srgb = colorspace::resolve("sRGB").expect("sRGB is registered");
        let to_srgb = if output_space.name == srgb.name {
            IDENTITY
        } else {
            colorspace::conversion_matrix(output_space, srgb)
        };
        let length = (WARM_A * WARM_A + WARM_B * WARM_B).sqrt();
        Self {
            tone: [
                params.brightness * BRIGHTNESS_SCALE,
                params.contrast * CONTRAST_SCALE,
                params.warmth * WARMTH_SCALE,
                params.saturation * SATURATION_SCALE,
            ],
            color: [CHROMA_REFERENCE, WARM_A / length, WARM_B / length],
            expand: params.expand.enabled.then(|| {
                (
                    params.expand.black,
                    1.0 / (params.expand.white - params.expand.black),
                )
            }),
            to_srgb,
            from_srgb: inverse(to_srgb),
            lab_to_lms: inverse(LMS_TO_LAB),
            lms_to_rgb: inverse(RGB_TO_LMS),
        }
    }

    fn pixel(&self, rgb: [f64; 3]) -> [f64; 3] {
        let mut rgb = mul(&self.to_srgb, rgb);
        if let Some((black, inverse_span)) = self.expand {
            rgb = rgb.map(|x| {
                let encoded = colorspace::srgb_encode_f64(x.abs()).copysign(x);
                let mapped = ((encoded - black) * inverse_span).clamp(0.0, 1.0);
                colorspace::srgb_decode_f64(mapped)
            });
        }
        if self.tone != [0.0; 4] {
            let lms = mul(&RGB_TO_LMS, rgb).map(f64::cbrt);
            let [mut l, mut a, mut b] = mul(&LMS_TO_LAB, lms);
            let [brightness, contrast, warmth, saturation] = self.tone;
            l += contrast * (2.0 * l - 1.0) * window(l);
            l += brightness * window(l);
            let chroma = a.hypot(b);
            let displacement = warmth * window(l) / (1.0 + (chroma / self.color[0]).powi(2));
            a += displacement * self.color[1];
            b += displacement * self.color[2];
            let chroma = a.hypot(b);
            let scale = (saturation / (1.0 + (chroma / self.color[0]).powi(2))).exp();
            a *= scale;
            b *= scale;
            let lms = mul(&self.lab_to_lms, [l, a, b]).map(|x| x * x * x);
            rgb = mul(&self.lms_to_rgb, lms);
        }
        mul(&self.from_srgb, rgb)
    }
}

#[inline]
fn window(l: f64) -> f64 {
    if l > 0.0 && l < 1.0 {
        16.0 * l * l * (1.0 - l) * (1.0 - l)
    } else {
        0.0
    }
}

/// Apply finishing in place. The caller owns route applicability.
pub fn apply(image: &mut ImageBuf, params: &FinishingParams, output_space: &RgbColorSpace) {
    if params.is_neutral() {
        return;
    }
    let prepared = Prepared::new(params, output_space);
    image.data.par_chunks_exact_mut(3).for_each(|px| {
        let result = prepared.pixel([px[0] as f64, px[1] as f64, px[2] as f64]);
        for i in 0..3 {
            px[i] = from_f64(result[i]);
        }
    });
}

/// Prepare the same constants and transforms for a resident device pass.
/// A neutral setting never needs a device dispatch.
pub fn gpu_params(
    params: &FinishingParams,
    output_space: &RgbColorSpace,
) -> Option<spektrafilm_backend::FinishingGpuParams> {
    if params.is_neutral() {
        return None;
    }
    let p = Prepared::new(params, output_space);
    let matrix = |m: Matrix| m.map(|row| row.map(|v| v as f32));
    Some(spektrafilm_backend::FinishingGpuParams {
        tone: p.tone.map(|v| v as f32),
        color: [p.color[0] as f32, p.color[1] as f32, p.color[2] as f32, 0.0],
        expand_enabled: p.expand.is_some(),
        black: p.expand.map_or(0.0, |v| v.0 as f32),
        inverse_span: p.expand.map_or(1.0, |v| v.1 as f32),
        to_srgb: matrix(p.to_srgb),
        from_srgb: matrix(p.from_srgb),
        rgb_to_lms: matrix(RGB_TO_LMS),
        lms_to_lab: matrix(LMS_TO_LAB),
        lab_to_lms: matrix(p.lab_to_lms),
        lms_to_rgb: matrix(p.lms_to_rgb),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_device_response_matches_cpu_across_output_primaries() {
        use spektrafilm_backend::ComputeBackend;
        let gpu = spektrafilm_gpu::test_support::explicit_adapter();
        let image = ImageBuf::from_data(
            7,
            1,
            [
                [0.18, 0.3, 0.5],
                [-0.08, 0.2, 1.4],
                [1.0, 0.05, 0.01],
                [0.0; 3],
                [1.0; 3],
                [-0.2, 0.5, 2.0],
                [1e-7, -1e-7, 0.2],
            ]
            .into_iter()
            .flatten()
            .map(from_f64)
            .collect(),
        );
        for space in ["sRGB", "Display P3", "ProPhoto RGB"] {
            let space = colorspace::resolve(space).unwrap();
            for expand in [false, true] {
                let mut params = FinishingParams {
                    brightness: 0.6,
                    contrast: -0.4,
                    warmth: 0.3,
                    saturation: 0.7,
                    ..Default::default()
                };
                params.expand.enabled = expand;
                let mut expected = image.clone();
                apply(&mut expected, &params, space);
                let device = gpu
                    .finishing(&image, &gpu_params(&params, space).unwrap())
                    .unwrap()
                    .expect("selected GPU finishing operation");
                for (&actual, &expected) in device.data.iter().zip(&expected.data) {
                    assert!(
                        (actual as f64 - expected as f64).abs() < 5e-5,
                        "{} Expand={expand}: {actual} != {expected}",
                        space.name
                    );
                }
            }
        }
    }

    fn srgb() -> &'static RgbColorSpace {
        colorspace::resolve("sRGB").unwrap()
    }

    fn close(actual: [f64; 3], expected: [f64; 3], tolerance: f64) {
        for (a, e) in actual.into_iter().zip(expected) {
            assert!((a - e).abs() <= tolerance, "{actual:?} != {expected:?}");
        }
    }

    #[test]
    fn public_oklab_primaries_and_signed_roundtrip_use_true_inverse() {
        let red = mul(
            &LMS_TO_LAB,
            mul(&RGB_TO_LMS, [1.0, 0.0, 0.0]).map(f64::cbrt),
        );
        close(red, [0.62795536, 0.22486306, 0.12584630], 2e-8);
        for rgb in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [-0.2, 0.5, 2.0],
        ] {
            let lab = mul(&LMS_TO_LAB, mul(&RGB_TO_LMS, rgb).map(f64::cbrt));
            let lms = mul(&inverse(LMS_TO_LAB), lab).map(|v| v * v * v);
            close(mul(&inverse(RGB_TO_LMS), lms), rgb, 2e-14);
        }
    }

    #[test]
    fn tone_extremes_preserve_order_and_identity_extension() {
        for contrast in [-CONTRAST_SCALE, CONTRAST_SCALE] {
            for brightness in [-BRIGHTNESS_SCALE, BRIGHTNESS_SCALE] {
                let tone = |l: f64| {
                    let l = l + contrast * (2.0 * l - 1.0) * window(l);
                    l + brightness * window(l)
                };
                assert_eq!(tone(-0.2), -0.2);
                assert_eq!(tone(0.0), 0.0);
                assert_eq!(tone(1.0), 1.0);
                assert_eq!(tone(1.2), 1.2);
                let mut last = tone(0.0);
                for i in 1..=10000 {
                    let next = tone(i as f64 / 10000.0);
                    assert!(next > last, "tone reversed at {i}");
                    last = next;
                }
            }
        }
    }

    #[test]
    fn neutral_bypass_preserves_signed_samples_and_inactive_endpoints() {
        let mut params = FinishingParams::default();
        params.expand.black = -0.2;
        params.expand.white = 1.1;
        let samples = vec![from_f64(-0.5), from_f64(0.2), from_f64(2.0)];
        let mut image = ImageBuf::from_data(1, 1, samples.clone());
        apply(
            &mut image,
            &params,
            colorspace::resolve("Display P3").unwrap(),
        );
        assert_eq!(image.data, samples);
        assert!(gpu_params(&params, srgb()).is_none());
    }

    #[test]
    fn declared_output_primaries_change_only_boundary_transforms() {
        let params = FinishingParams {
            warmth: 0.7,
            contrast: -0.5,
            ..Default::default()
        };
        let p3 = colorspace::resolve("Display P3").unwrap();
        let output = Prepared::new(&params, p3);
        let work = Prepared::new(&params, srgb());
        let sample = [0.5, 0.1, 0.3];
        let in_work = mul(&output.to_srgb, sample);
        close(
            mul(&output.to_srgb, output.pixel(sample)),
            work.pixel(in_work),
            1e-14,
        );
        close(mul(&output.from_srgb, in_work), sample, 1e-14);
    }

    #[test]
    fn enabled_expand_clips_default_and_maps_encoded_references() {
        let mut params = FinishingParams::default();
        params.expand.enabled = true;
        let p = Prepared::new(&params, srgb());
        close(p.pixel([-0.4, 0.25, 1.5]), [0.0, 0.25, 1.0], 1e-14);
        assert!(gpu_params(&params, srgb()).is_some());
        params.expand.black = 0.2;
        params.expand.white = 0.8;
        let p = Prepared::new(&params, srgb());
        let decode = colorspace::srgb_decode_f64;
        close(
            p.pixel([decode(0.2), decode(0.5), decode(0.8)]),
            [0.0, decode(0.5), 1.0],
            1e-14,
        );
    }

    #[test]
    fn qualified_combination_matches_independent_reference_samples() {
        let params = FinishingParams {
            brightness: 0.6,
            contrast: -0.4,
            warmth: 0.3,
            saturation: 0.7,
            ..Default::default()
        };
        let response = Prepared::new(&params, srgb());
        // Public-matrix float64 reference evaluated independently from this module.
        for (input, expected) in [
            (
                [0.18, 0.3, 0.5],
                [0.18596996315832406, 0.34263715691283403, 0.6178060914006991],
            ),
            (
                [-0.08, 0.2, 1.4],
                [
                    -0.09520178721278538,
                    0.23485707442881354,
                    1.6587566537471647,
                ],
            ),
            (
                [1.0, 0.05, 0.01],
                [
                    1.1822082154119227,
                    0.044300445590241495,
                    0.002171333747422655,
                ],
            ),
        ] {
            close(response.pixel(input), expected, 2e-14);
        }
    }

    #[test]
    fn adjustments_respect_signed_headroom_and_primary_effects() {
        let mut p = FinishingParams {
            brightness: 1.0,
            ..Default::default()
        };
        let bright = Prepared::new(&p, srgb()).pixel([0.18; 3]);
        assert!(bright.iter().all(|&v| v > 0.18));
        p.brightness = -1.0;
        let dark = Prepared::new(&p, srgb()).pixel([0.18; 3]);
        assert!(dark.iter().all(|&v| v < 0.18));
        p.brightness = 0.0;
        p.warmth = 1.0;
        let warm = Prepared::new(&p, srgb()).pixel([0.18; 3]);
        assert!(warm[0] > warm[2]);
        p.warmth = 0.0;
        p.saturation = 1.0;
        let saturated = Prepared::new(&p, srgb()).pixel([1.0, 0.0, 0.0]);
        assert!(saturated[0] > 1.0 || saturated[1] < 0.0 || saturated[2] < 0.0);
        let neutral = Prepared::new(&p, srgb()).pixel([0.18; 3]);
        close(neutral, [0.18; 3], 1e-7);
    }

    #[test]
    fn route_requires_a_completed_positive_reading() {
        let mut params = RuntimeParams::default();
        for route in [
            "input",
            "input > film",
            "input > film > print",
            "film > print",
            "input > convert-film > scan-minus-base",
            "unknown",
        ] {
            params.workflow.route = route.into();
            assert!(!is_applicable(&params, true), "{route}");
            assert!(!is_applicable(&params, false), "{route}");
        }
        for route in [
            "input > film > scan",
            "film > scan",
            "input > convert-film > scan",
        ] {
            params.workflow.route = route.into();
            params.scanner.scan_output = "direct_scan".into();
            assert!(is_applicable(&params, true), "{route}");
            assert!(!is_applicable(&params, false), "{route}");
            params.scanner.scan_output = "negative_to_positive".into();
            assert!(is_applicable(&params, false), "{route}");
        }
        for route in [
            "input > film > print > scan",
            "film > print > scan",
            "print > scan",
            "input > convert-film > print > scan",
        ] {
            params.workflow.route = route.into();
            assert!(is_applicable(&params, true), "{route}");
            assert!(!is_applicable(&params, false), "{route}");
        }
    }

    #[test]
    fn captured_paper_is_finished_but_direct_negative_is_not() {
        let data = crate::test_support::data_dir();
        let paper = crate::profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
        let film = crate::profile::load_profile_by_name(&data, "kodak_doublex").unwrap();
        assert!(paper.is_paper());
        assert!(film.is_negative());
        let backend = spektrafilm_backend::cpu_backend::CpuBackend;
        let gamut = crate::gamut_compression::OutputGamutCompress::identity();
        let mut params = RuntimeParams::default();
        params.io.output_cctf_encoding = false;
        params.scanner.unsharp_mask = [0.0; 2];
        params.print_render.glare.active = false;
        params.film_render.grain.active = false;
        params.cmyk.active = false;
        params.finishing.brightness = 1.0;
        let image = ImageBuf::from_data(1, 1, vec![from_f64(0.18); 3]);
        params.workflow.route = "print > scan".into();
        params.io.scan_film = false;
        let actual = crate::stages::scanning::finish_capture(
            image.clone(),
            &paper,
            &params,
            &backend,
            &gamut,
        )
        .unwrap();
        let mut expected = image.clone();
        apply(&mut expected, &params.finishing, srgb());
        assert_eq!(actual.data, expected.data);
        assert_ne!(actual.data, image.data);
        params.workflow.route = "film > scan".into();
        params.io.scan_film = true;
        let negative = crate::stages::scanning::finish_capture(
            image.clone(),
            &film,
            &params,
            &backend,
            &gamut,
        )
        .unwrap();
        assert_eq!(negative.data, image.data);
    }
}
