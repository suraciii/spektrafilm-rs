//! Native endpoint classification and bounded viewer warning masks.
use crate::{Precision, RetainedImageMeta, SampleDomain, ViewerClipping, ViewerComposeGpuParams};
use spektrafilm_math::{colorspace, image::ImageBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const SHADOW: u32 = 1;
pub const HIGHLIGHT: u32 = 2;
pub const SHADOW_RGBA: [u8; 4] = [64, 112, 255, 255];
pub const HIGHLIGHT_RGBA: [u8; 4] = [255, 64, 64, 255];

/// Encoded endpoint values are rounded to the precision of the carrier.
/// RGB primaries are deliberately irrelevant to native endpoint warnings.
pub fn endpoint_thresholds(meta: RetainedImageMeta) -> Result<[f64; 2], String> {
    let endpoints = match meta.domain {
        SampleDomain::Linear => [0.0, 1.0],
        SampleDomain::Encoded => [0.0, 1.0].map(|v| colorspace::cctf_encode(v, meta.space.cctf)),
        _ => return Err("viewer clipping requires RGB samples".into()),
    };
    Ok(match meta.precision {
        Precision::F32 => endpoints.map(|v| v as f32 as f64),
        Precision::F64 => endpoints,
    })
}

/// Apply a mask to a pixel known to be inside the photograph.
/// If both flags occupy a downsampled cell, enabled highlights take priority.
pub fn apply_rgba(pixel: [u8; 4], flags: u32, mode: ViewerClipping) -> [u8; 4] {
    match mode {
        ViewerClipping::Off => pixel,
        ViewerClipping::Overlay {
            shadows,
            highlights,
        } => {
            if highlights && flags & HIGHLIGHT != 0 {
                HIGHLIGHT_RGBA
            } else if shadows && flags & SHADOW != 0 {
                SHADOW_RGBA
            } else {
                pixel
            }
        }
        ViewerClipping::Shadows => {
            if flags & SHADOW != 0 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        }
        ViewerClipping::Highlights => {
            if flags & HIGHLIGHT != 0 {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        }
    }
}

fn cancelled(cancellation: Option<&AtomicBool>) -> Result<(), String> {
    if cancellation.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        Err("operation cancelled".into())
    } else {
        Ok(())
    }
}

/// Mapping of viewport cells to overlapping native cells along one axis.
pub struct ClippingAxis {
    origin: f64,
    cell: f64,
    image: [f64; 2],
    scale: f64,
    size: u32,
}

impl ClippingAxis {
    /// The caller supplies nonempty, finite rectangles and positive sizes.
    pub fn new(viewport_size: u32, viewport: [f32; 2], image: [f32; 2], size: u32) -> Self {
        let viewport = viewport.map(f64::from);
        let image = image.map(f64::from);
        Self {
            origin: viewport[0],
            cell: (viewport[1] - viewport[0]) / f64::from(viewport_size),
            image,
            scale: f64::from(size) / (image[1] - image[0]),
            size,
        }
    }

    /// None denotes a viewport cell whose center is outside the photograph.
    pub fn bounds(&self, coordinate: u32) -> Option<[u32; 2]> {
        let edge = self.origin + f64::from(coordinate) * self.cell;
        let center = edge + 0.5 * self.cell;
        if center < self.image[0] || center >= self.image[1] {
            return None;
        }
        Some([
            ((edge - self.image[0]) * self.scale)
                .floor()
                .clamp(0.0, f64::from(self.size)) as u32,
            ((edge + self.cell - self.image[0]) * self.scale)
                .ceil()
                .clamp(0.0, f64::from(self.size)) as u32,
        ])
    }
}

/// OR native source pixels whose cell footprints overlap each viewport cell.
/// Only cells whose centers lie inside the half-open image rectangle are marked.
/// Storage is bounded by the viewport and traversal by the visible source area.
pub fn mask_host(
    image: &ImageBuf,
    meta: RetainedImageMeta,
    params: &ViewerComposeGpuParams,
    cancellation: Option<&AtomicBool>,
) -> Result<Vec<u32>, String> {
    let samples = (image.width as usize)
        .checked_mul(image.height as usize)
        .and_then(|n| n.checked_mul(3));
    if meta.width != image.width || meta.height != image.height || samples != Some(image.data.len())
    {
        return Err("viewer clipping dimensions do not match samples".into());
    }
    let thresholds = endpoint_thresholds(meta)?;
    let count = (params.width as usize)
        .checked_mul(params.height as usize)
        .ok_or("viewer clipping viewport is too large")?;
    cancelled(cancellation)?;
    let mut mask = vec![0; count];
    if !params.clipping.active() || image.width == 0 || image.height == 0 || count == 0 {
        return Ok(mask);
    }
    let rect = params.rect.map(f64::from);
    let image_rect = params.image_rect.map(f64::from);
    if !rect.iter().chain(image_rect.iter()).all(|v| v.is_finite())
        || rect[2] <= rect[0]
        || rect[3] <= rect[1]
        || image_rect[2] <= image_rect[0]
        || image_rect[3] <= image_rect[1]
    {
        return Err("viewer clipping requires nonempty finite rectangles".into());
    }
    let x_axis = ClippingAxis::new(
        params.width,
        [params.rect[0], params.rect[2]],
        [params.image_rect[0], params.image_rect[2]],
        image.width,
    );
    let y_axis = ClippingAxis::new(
        params.height,
        [params.rect[1], params.rect[3]],
        [params.image_rect[1], params.image_rect[3]],
        image.height,
    );
    let mut x_bounds = Vec::with_capacity(params.width as usize);
    for x in 0..params.width {
        if x & 4095 == 0 {
            cancelled(cancellation)?;
        }
        x_bounds.push(x_axis.bounds(x));
    }
    let mut y_bounds = Vec::with_capacity(params.height as usize);
    for y in 0..params.height {
        if y & 4095 == 0 {
            cancelled(cancellation)?;
        }
        y_bounds.push(y_axis.bounds(y));
    }
    for (y, bounds) in y_bounds.into_iter().enumerate() {
        cancelled(cancellation)?;
        let Some([sy0, sy1]) = bounds else {
            continue;
        };
        for (x, bounds) in x_bounds.iter().enumerate() {
            let Some([sx0, sx1]) = *bounds else {
                continue;
            };
            let mut flags = 0;
            for sy in sy0..sy1 {
                cancelled(cancellation)?;
                for sx in sx0..sx1 {
                    if sx & 4095 == 0 {
                        cancelled(cancellation)?;
                    }
                    let rgb = image.get(sx, sy).map(|v| v as f64);
                    if rgb.iter().all(|v| v.is_finite()) {
                        if rgb.iter().all(|v| *v <= thresholds[0]) {
                            flags |= SHADOW;
                        }
                        if rgb.iter().any(|v| *v >= thresholds[1]) {
                            flags |= HIGHLIGHT;
                        }
                        if flags == (SHADOW | HIGHLIGHT) {
                            break;
                        }
                    }
                }
                if flags == (SHADOW | HIGHLIGHT) {
                    break;
                }
            }
            mask[y * params.width as usize + x] = flags;
        }
    }
    Ok(mask)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{AnalysisMode, AnalysisRequest, analyze_host};
    use spektrafilm_math::precision::Scalar;

    fn image(width: u32, height: u32, values: &[f64]) -> ImageBuf {
        ImageBuf::from_data(width, height, values.iter().map(|v| *v as Scalar).collect())
    }

    fn meta(
        image: &ImageBuf,
        space: &'static colorspace::RgbColorSpace,
        domain: SampleDomain,
    ) -> RetainedImageMeta {
        RetainedImageMeta {
            width: image.width,
            height: image.height,
            space,
            domain,
            precision: Precision::host(),
            pixel_pitch_um: None,
        }
    }

    fn params(
        width: u32,
        height: u32,
        rect: [f32; 4],
        image_rect: [f32; 4],
    ) -> ViewerComposeGpuParams {
        ViewerComposeGpuParams {
            width,
            height,
            rect,
            image_rect,
            border: rect,
            paper_rect: rect,
            gray: [0.0; 3],
            interpolation: 0,
            full_source: false,
            output_layer: false,
            fade: 1.0,
            clipping: ViewerClipping::Overlay {
                shadows: true,
                highlights: true,
            },
        }
    }

    #[test]
    fn native_endpoints_invalids_and_analysis_have_distinct_rules() {
        let im = image(
            7,
            1,
            &[
                0.0,
                0.0,
                0.0,
                1.0,
                1.0,
                1.0,
                -0.1,
                0.2,
                0.3,
                -0.1,
                -0.2,
                0.0,
                0.2,
                1.0,
                0.3,
                f64::NAN,
                1.0,
                0.0,
                f64::INFINITY,
                -1.0,
                -1.0,
            ],
        );
        let m = meta(&im, &colorspace::SRGB, SampleDomain::Linear);
        let p = params(7, 1, [0.0, 0.0, 7.0, 1.0], [0.0, 0.0, 7.0, 1.0]);
        assert_eq!(
            mask_host(&im, m, &p, None).unwrap(),
            [SHADOW, HIGHLIGHT, 0, SHADOW, HIGHLIGHT, 0, 0]
        );
        let analysis = analyze_host(
            &im,
            m,
            &AnalysisRequest {
                region: [0, 0, 7, 1],
                mode: AnalysisMode::Histogram,
                chroma_zoom: 1,
            },
            None,
        )
        .unwrap();
        assert_eq!(
            (
                analysis.finite,
                analysis.invalid,
                analysis.below,
                analysis.above
            ),
            (5, 2, 2, 0)
        );
        assert_eq!(im.get(0, 0), [0.0, 0.0, 0.0]);
        assert_eq!(im.get(1, 0), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn native_primaries_are_not_converted() {
        let im = image(2, 1, &[0.0, 0.4, 0.2, 0.0, 0.0, 0.0]);
        let m = meta(&im, &colorspace::DISPLAY_P3, SampleDomain::Linear);
        let p = params(2, 1, [0.0, 0.0, 2.0, 1.0], [0.0, 0.0, 2.0, 1.0]);
        assert_eq!(mask_host(&im, m, &p, None).unwrap(), [0, SHADOW]);
    }

    #[test]
    fn encoded_acescct_endpoints_follow_carrier_precision() {
        let space = colorspace::resolve("ACEScct").unwrap();
        let dummy = image(1, 1, &[0.0; 3]);
        let mut m = meta(&dummy, space, SampleDomain::Encoded);
        m.precision = Precision::F64;
        let full = endpoint_thresholds(m).unwrap();
        assert_eq!(
            full,
            [
                colorspace::cctf_encode(0.0, space.cctf),
                colorspace::cctf_encode(1.0, space.cctf)
            ]
        );
        assert!(full[0] > 0.0 && full[1] < 1.0);
        m.precision = Precision::F32;
        assert_eq!(
            endpoint_thresholds(m).unwrap(),
            full.map(|v| v as f32 as f64)
        );
        let endpoints = endpoint_thresholds(meta(&dummy, space, SampleDomain::Encoded)).unwrap();
        let im = image(
            4,
            1,
            &[
                endpoints[0],
                endpoints[0],
                endpoints[0],
                endpoints[1],
                0.2,
                0.2,
                endpoints[0] + 0.001,
                endpoints[0],
                endpoints[0],
                endpoints[1] - 0.001,
                0.2,
                0.2,
            ],
        );
        let mut m = meta(&im, space, SampleDomain::Encoded);
        let p = params(4, 1, [0.0, 0.0, 4.0, 1.0], [0.0, 0.0, 4.0, 1.0]);
        assert_eq!(
            mask_host(&im, m, &p, None).unwrap(),
            [SHADOW, HIGHLIGHT, 0, 0]
        );
        m.domain = SampleDomain::Density;
        assert!(endpoint_thresholds(m).is_err());
    }

    #[test]
    fn encoded_endpoint_neighbors_are_classified_at_host_precision() {
        let space = colorspace::resolve("ACEScct").unwrap();
        let dummy = image(1, 1, &[0.0; 3]);
        let [black, white] = endpoint_thresholds(meta(&dummy, space, SampleDomain::Encoded))
            .unwrap()
            .map(|v| v as Scalar);
        let above_black = Scalar::from_bits(black.to_bits() + 1);
        let below_white = Scalar::from_bits(white.to_bits() - 1);
        let im = ImageBuf::from_data(
            4,
            1,
            vec![
                black,
                black,
                black,
                above_black,
                black,
                black,
                white,
                black,
                black,
                below_white,
                black,
                black,
            ],
        );
        let m = meta(&im, space, SampleDomain::Encoded);
        let p = params(4, 1, [0.0, 0.0, 4.0, 1.0], [0.0, 0.0, 4.0, 1.0]);
        assert_eq!(
            mask_host(&im, m, &p, None).unwrap(),
            [SHADOW, 0, HIGHLIGHT, 0]
        );
    }

    #[test]
    fn downscale_preserves_single_native_pixel_and_ors_flags() {
        let im = image(
            8,
            1,
            &[
                0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5,
                0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5, 0.5,
            ],
        );
        let m = meta(&im, &colorspace::SRGB, SampleDomain::Linear);
        let p = params(2, 1, [0.0, 0.0, 2.0, 1.0], [0.0, 0.0, 2.0, 1.0]);
        assert_eq!(mask_host(&im, m, &p, None).unwrap(), [SHADOW, HIGHLIGHT]);
        let p = params(1, 1, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(mask_host(&im, m, &p, None).unwrap(), [SHADOW | HIGHLIGHT]);
    }

    #[test]
    fn offscreen_and_half_open_centers_do_not_mark_padding() {
        let im = image(
            4,
            1,
            &[0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 1.0, 0.5, 0.5, 0.0, 0.0, 0.0],
        );
        let m = meta(&im, &colorspace::SRGB, SampleDomain::Linear);
        let p = params(3, 1, [0.0, 0.0, 3.0, 1.0], [-1.0, 0.0, 3.0, 1.0]);
        assert_eq!(mask_host(&im, m, &p, None).unwrap(), [0, HIGHLIGHT, SHADOW]);
        let p = params(3, 1, [0.0, 0.0, 3.0, 1.0], [0.5, 0.0, 2.5, 1.0]);
        assert_eq!(mask_host(&im, m, &p, None).unwrap(), [SHADOW, HIGHLIGHT, 0]);
        let p = params(3, 1, [0.0, 0.0, 3.0, 1.0], [4.0, 0.0, 8.0, 1.0]);
        assert_eq!(mask_host(&im, m, &p, None).unwrap(), [0, 0, 0]);
    }

    #[test]
    fn overlay_and_isolation_colors_are_independent() {
        let pixel = [12, 34, 56, 78];
        let both = ViewerClipping::Overlay {
            shadows: true,
            highlights: true,
        };
        assert_eq!(apply_rgba(pixel, 0, both), pixel);
        assert_eq!(apply_rgba(pixel, SHADOW, both), SHADOW_RGBA);
        assert_eq!(apply_rgba(pixel, HIGHLIGHT | SHADOW, both), HIGHLIGHT_RGBA);
        assert_eq!(
            apply_rgba(
                pixel,
                SHADOW,
                ViewerClipping::Overlay {
                    shadows: false,
                    highlights: true
                }
            ),
            pixel
        );
        assert_eq!(
            apply_rgba(pixel, SHADOW, ViewerClipping::Shadows),
            [0, 0, 0, 255]
        );
        assert_eq!(
            apply_rgba(pixel, HIGHLIGHT, ViewerClipping::Shadows),
            [255, 255, 255, 255]
        );
        assert_eq!(
            apply_rgba(pixel, HIGHLIGHT, ViewerClipping::Highlights),
            [255, 255, 255, 255]
        );
        assert_eq!(
            apply_rgba(pixel, SHADOW, ViewerClipping::Highlights),
            [0, 0, 0, 255]
        );
        assert!(
            !ViewerClipping::Overlay {
                shadows: false,
                highlights: false
            }
            .active()
        );
    }

    #[test]
    fn cancellation_and_bad_metadata_return_errors() {
        let im = image(1, 1, &[0.0, 0.0, 0.0]);
        let mut m = meta(&im, &colorspace::SRGB, SampleDomain::Linear);
        let p = params(1, 1, [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0]);
        assert!(mask_host(&im, m, &p, Some(&AtomicBool::new(true))).is_err());
        m.width = 2;
        assert!(mask_host(&im, m, &p, None).is_err());
    }
}
