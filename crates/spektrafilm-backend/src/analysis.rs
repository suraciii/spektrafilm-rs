//! Native-pixel image analysis in the common, unbounded sRGB interpretation.
use crate::{RetainedImageMeta, SampleDomain};
use spektrafilm_math::colorspace::{self, SRGB_TO_XYZ_F64};
use spektrafilm_math::image::ImageBuf;

pub const BINS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisMode {
    Histogram,
    Waveform,
    Parade,
    Vectorscope,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisRequest {
    pub region: [u32; 4],
    pub mode: AnalysisMode,
    pub chroma_zoom: u32,
}

#[derive(Clone, Debug)]
pub struct AnalysisData {
    pub histogram: [Vec<u64>; 4],
    pub waveform: Vec<u64>,
    pub column_population: Vec<u64>,
    pub vectorscope: Vec<u64>,
    pub spatial_bins: u32,
    pub total: u64,
    pub finite: u64,
    pub invalid: u64,
    pub below: u64,
    pub above: u64,
    pub outside_window: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct AnalysisPixel {
    pub rgb: [f64; 3],
    pub tone: f64,
    pub ab: [f64; 2],
    pub valid: bool,
}

struct Transform {
    matrix: [[f64; 3]; 3],
    meta: RetainedImageMeta,
}

impl Transform {
    fn new(meta: RetainedImageMeta) -> Result<Self, String> {
        if !matches!(meta.domain, SampleDomain::Linear | SampleDomain::Encoded) {
            return Err("image analysis requires RGB samples".into());
        }
        let matrix = if meta.space.name == "sRGB" {
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        } else {
            meta.space.display_matrix()
        };
        Ok(Self { matrix, meta })
    }

    fn pixel(&self, source: [f64; 3]) -> (AnalysisPixel, [f64; 3]) {
        let mut linear = source;
        if self.meta.domain == SampleDomain::Encoded {
            linear = linear.map(|v| colorspace::cctf_decode(v, self.meta.space.cctf));
        }
        let linear = self
            .matrix
            .map(|m| m[0] * linear[0] + m[1] * linear[1] + m[2] * linear[2]);
        let y = SRGB_TO_XYZ_F64[1][0] * linear[0]
            + SRGB_TO_XYZ_F64[1][1] * linear[1]
            + SRGB_TO_XYZ_F64[1][2] * linear[2];
        let l =
            (0.4122214708 * linear[0] + 0.5363325363 * linear[1] + 0.0514459929 * linear[2]).cbrt();
        let m =
            (0.2119034982 * linear[0] + 0.6806995451 * linear[1] + 0.1073969566 * linear[2]).cbrt();
        let s =
            (0.0883024619 * linear[0] + 0.2817188376 * linear[1] + 0.6299787005 * linear[2]).cbrt();
        let rgb = linear.map(colorspace::srgb_encode_f64);
        let tone = colorspace::srgb_encode_f64(y);
        let ab = [
            1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
            0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
        ];
        let valid = source
            .iter()
            .chain(linear.iter())
            .chain(rgb.iter())
            .chain([tone].iter())
            .chain(ab.iter())
            .all(|v| v.is_finite());
        (
            AnalysisPixel {
                rgb,
                tone,
                ab,
                valid,
            },
            linear,
        )
    }
}

fn validate_image(image: &ImageBuf, meta: RetainedImageMeta) -> Result<(), String> {
    let size = (image.width as usize)
        .checked_mul(image.height as usize)
        .and_then(|n| n.checked_mul(3));
    if meta.width != image.width || meta.height != image.height || size != Some(image.data.len()) {
        return Err("image analysis dimensions do not match samples".into());
    }
    Ok(())
}

fn bin(value: f64) -> usize {
    ((value.clamp(0.0, 1.0) * BINS as f64) as usize).min(BINS - 1)
}

/// Inspect a native pixel without clipping the reported analysis coordinates.
pub fn inspect_host(
    image: &ImageBuf,
    meta: RetainedImageMeta,
    x: u32,
    y: u32,
) -> Result<AnalysisPixel, String> {
    validate_image(image, meta)?;
    if x >= image.width || y >= image.height {
        return Err("analysis pixel is outside image".into());
    }
    let transform = Transform::new(meta)?;
    Ok(transform.pixel(image.get(x, y).map(|v| v as f64)).0)
}

/// Reduce all native pixels of a half-open region; no image-sized intermediate is made.
pub fn analyze_host(
    image: &ImageBuf,
    meta: RetainedImageMeta,
    request: &AnalysisRequest,
    cancellation: Option<&std::sync::atomic::AtomicBool>,
) -> Result<AnalysisData, String> {
    validate_image(image, meta)?;
    let [x0, y0, x1, y1] = request.region;
    if x0 >= x1 || y0 >= y1 || x1 > image.width || y1 > image.height {
        return Err("analysis region is empty or outside image".into());
    }
    if !matches!(request.chroma_zoom, 1 | 2 | 4) {
        return Err("analysis chroma zoom must be 1, 2, or 4".into());
    }
    let transform = Transform::new(meta)?;
    let width = x1 - x0;
    let spatial_bins = width.min(BINS as u32);
    let waveform_mode = matches!(request.mode, AnalysisMode::Waveform | AnalysisMode::Parade);
    let mut data = AnalysisData {
        histogram: std::array::from_fn(|_| vec![0; BINS]),
        waveform: if waveform_mode {
            vec![0; 4 * spatial_bins as usize * BINS]
        } else {
            Vec::new()
        },
        column_population: vec![0; spatial_bins as usize],
        vectorscope: if request.mode == AnalysisMode::Vectorscope {
            vec![0; BINS * BINS]
        } else {
            Vec::new()
        },
        spatial_bins,
        total: u64::from(width) * u64::from(y1 - y0),
        finite: 0,
        invalid: 0,
        below: 0,
        above: 0,
        outside_window: 0,
    };
    let extent = 0.4 / f64::from(request.chroma_zoom);
    for y in y0..y1 {
        for x in x0..x1 {
            if (x - x0) % 4096 == 0
                && cancellation.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
            {
                return Err("operation cancelled".into());
            }
            let (pixel, linear) = transform.pixel(image.get(x, y).map(|v| v as f64));
            if !pixel.valid {
                data.invalid += 1;
                continue;
            }
            data.finite += 1;
            data.below += u64::from(linear.iter().any(|v| *v < 0.0));
            data.above += u64::from(linear.iter().any(|v| *v > 1.0));
            let signals = [pixel.tone, pixel.rgb[0], pixel.rgb[1], pixel.rgb[2]];
            let col = (((2 * u64::from(x - x0) + 1) * u64::from(spatial_bins))
                / (2 * u64::from(width))) as usize;
            for (signal, value) in signals.into_iter().enumerate() {
                let value_bin = bin(value);
                data.histogram[signal][value_bin] += 1;
                if waveform_mode {
                    data.waveform[(signal * spatial_bins as usize + col) * BINS + value_bin] += 1;
                }
            }
            data.column_population[col] += 1;
            if request.mode == AnalysisMode::Vectorscope {
                let [a, b] = pixel.ab;
                if a < -extent || a > extent || b < -extent || b > extent {
                    data.outside_window += 1;
                } else {
                    let x = bin((a + extent) / (2.0 * extent));
                    let y = bin((b + extent) / (2.0 * extent));
                    data.vectorscope[y * BINS + x] += 1;
                }
            }
        }
    }
    if cancellation.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed)) {
        return Err("operation cancelled".into());
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Precision;
    use spektrafilm_math::colorspace::{self, RgbColorSpace};
    use spektrafilm_math::precision::Scalar;

    fn meta(
        width: u32,
        height: u32,
        space: &'static RgbColorSpace,
        domain: SampleDomain,
    ) -> RetainedImageMeta {
        RetainedImageMeta {
            width,
            height,
            space,
            domain,
            precision: Precision::host(),
            pixel_pitch_um: None,
        }
    }
    fn image(width: u32, height: u32, values: &[f64]) -> ImageBuf {
        ImageBuf::from_data(width, height, values.iter().map(|v| *v as Scalar).collect())
    }
    fn request(region: [u32; 4], mode: AnalysisMode, chroma_zoom: u32) -> AnalysisRequest {
        AnalysisRequest {
            region,
            mode,
            chroma_zoom,
        }
    }
    #[test]
    fn roi_distributions_preserve_population_and_strict_range() {
        let im = image(
            4,
            1,
            &[
                0.0,
                0.0,
                0.0,
                1.0,
                1.0,
                1.0,
                -0.1,
                1.2,
                0.5,
                f64::NAN,
                0.0,
                0.0,
            ],
        );
        let m = meta(4, 1, &colorspace::SRGB, SampleDomain::Linear);
        let d = analyze_host(
            &im,
            m,
            &request([0, 0, 4, 1], AnalysisMode::Waveform, 1),
            None,
        )
        .unwrap();
        assert_eq!(
            (d.total, d.finite, d.invalid, d.below, d.above),
            (4, 3, 1, 1, 1)
        );
        assert_eq!(d.column_population, [1, 1, 1, 0]);
        assert_eq!(d.histogram[1][0], 2);
        assert_eq!(d.histogram[2][255], 2);
        for signal in 0..4 {
            assert_eq!(d.histogram[signal].iter().sum::<u64>(), 3);
        }
        assert_eq!(d.waveform.iter().sum::<u64>(), 12);
        let subset = analyze_host(
            &im,
            m,
            &request([1, 0, 3, 1], AnalysisMode::Histogram, 1),
            None,
        )
        .unwrap();
        assert_eq!(
            (subset.total, subset.finite, subset.below, subset.above),
            (2, 2, 1, 1)
        );
        assert!(
            analyze_host(
                &im,
                m,
                &request([1, 0, 5, 1], AnalysisMode::Histogram, 1),
                None
            )
            .is_err()
        );
    }
    #[test]
    fn encoding_and_primaries_affect_analysis_once() {
        let linear = image(1, 1, &[0.25, 0.5, 0.75]);
        let encoded = image(
            1,
            1,
            &[
                colorspace::srgb_encode_f64(0.25),
                colorspace::srgb_encode_f64(0.5),
                colorspace::srgb_encode_f64(0.75),
            ],
        );
        let a = inspect_host(
            &linear,
            meta(1, 1, &colorspace::SRGB, SampleDomain::Linear),
            0,
            0,
        )
        .unwrap();
        let b = inspect_host(
            &encoded,
            meta(1, 1, &colorspace::SRGB, SampleDomain::Encoded),
            0,
            0,
        )
        .unwrap();
        for c in 0..3 {
            assert!((a.rgb[c] - b.rgb[c]).abs() < 1e-6);
        }
        let p3 = inspect_host(
            &linear,
            meta(1, 1, &colorspace::DISPLAY_P3, SampleDomain::Linear),
            0,
            0,
        )
        .unwrap();
        assert!((p3.rgb[0] - a.rgb[0]).abs() > 0.01);
        assert!(
            inspect_host(
                &linear,
                meta(1, 1, &colorspace::SRGB, SampleDomain::Density),
                0,
                0
            )
            .is_err()
        );
    }
    #[test]
    fn spatial_intervals_use_pixel_centers_and_signal_bins_are_equal_width() {
        let mut values = vec![0.0; 257 * 3];
        values[3] = colorspace::srgb_decode_f64(0.501);
        let im = image(257, 1, &values);
        let m = meta(257, 1, &colorspace::SRGB, SampleDomain::Linear);
        let d = analyze_host(
            &im,
            m,
            &request([0, 0, 257, 1], AnalysisMode::Parade, 1),
            None,
        )
        .unwrap();
        assert_eq!(d.spatial_bins, 256);
        assert_eq!(d.column_population.iter().sum::<u64>(), 257);
        assert_eq!(d.column_population[0], 1);
        assert_eq!(d.column_population[128], 2);
        assert_eq!(d.histogram[1][128], 1);
        assert_eq!(d.waveform[(256 + 1) * BINS + 128], 1);
        assert_eq!(bin(0.0), 0);
        assert_eq!(bin(1.0), 255);
        assert_eq!(bin(0.5), 128);
        assert_eq!(bin(-1.0), 0);
        assert_eq!(bin(2.0), 255);
    }

    #[test]
    fn cancellation_leaves_the_host_image_reusable() {
        let im = image(2, 1, &[0.5; 6]);
        let m = meta(2, 1, &colorspace::SRGB, SampleDomain::Linear);
        let req = request([0, 0, 2, 1], AnalysisMode::Histogram, 1);
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        assert_eq!(
            analyze_host(&im, m, &req, Some(&cancelled)).unwrap_err(),
            "operation cancelled"
        );
        cancelled.store(false, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            analyze_host(&im, m, &req, Some(&cancelled)).unwrap().finite,
            2
        );
        let neutral = inspect_host(&im, m, 0, 0).unwrap();
        assert!(neutral.ab[0].abs() < 1e-6 && neutral.ab[1].abs() < 1e-6);
    }

    #[test]
    fn vectorscope_uses_oklab_and_counts_off_window() {
        let im = image(2, 1, &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let m = meta(2, 1, &colorspace::SRGB, SampleDomain::Linear);
        let red = inspect_host(&im, m, 0, 0).unwrap();
        assert!((red.ab[0] - 0.224863).abs() < 1e-5);
        assert!((red.ab[1] - 0.125846).abs() < 1e-5);
        let normal = analyze_host(
            &im,
            m,
            &request([0, 0, 2, 1], AnalysisMode::Vectorscope, 1),
            None,
        )
        .unwrap();
        assert_eq!(normal.vectorscope.iter().sum::<u64>(), 2);
        let zoomed = analyze_host(
            &im,
            m,
            &request([0, 0, 2, 1], AnalysisMode::Vectorscope, 4),
            None,
        )
        .unwrap();
        assert_eq!(
            (
                zoomed.outside_window,
                zoomed.vectorscope.iter().sum::<u64>()
            ),
            (1, 1)
        );
    }
}
