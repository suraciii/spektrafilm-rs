//! Mapping between an accepted raster and normalized, unrotated source areas.

use spektrafilm_core::params::RuntimeParams;

/// Immutable geometry of one accepted photographic subject. The crop is stored
/// in normalized coordinates of the oriented working frame, before its final
/// resize; `image_size` is the actual accepted raster extent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AnalysisGeometry {
    rotation: u32,
    oriented_crop: [f64; 4],
    image_size: [u32; 2],
}

impl AnalysisGeometry {
    pub(crate) fn original(source_size: [u32; 2], rotation: u32) -> Self {
        let rotation = rotation % 4;
        let image_size = if rotation.is_multiple_of(2) {
            source_size
        } else {
            [source_size[1], source_size[0]]
        };
        Self {
            rotation,
            oriented_crop: [0.0, 0.0, 1.0, 1.0],
            image_size,
        }
    }

    /// Resolve the same crop lattice used by the rendering route. Ordinary
    /// Preview first resizes the full oriented source, whereas full-frame
    /// scanner Preview applies the crop at full resolution before presentation.
    /// A failed resolution is analysis-unavailable, not a render failure.
    pub(crate) fn rendered(
        source_size: [u32; 2],
        rotation: u32,
        output_size: [u32; 2],
        params: &RuntimeParams,
    ) -> Option<Self> {
        if source_size.contains(&0) || output_size.contains(&0) {
            return None;
        }
        let mut geometry = Self::original(source_size, rotation);
        let mut working = geometry.image_size;
        if params.settings.preview_mode && !params.scanner.requires_full_frame() {
            let (w, h) = spektrafilm_core::resizing::preview_dimensions(
                working[0],
                working[1],
                params.settings.preview_max_size,
            );
            working = [w, h];
        }

        // Intermediate tap values may be densities, exposures or preprocessed
        // samples, not the declared output RGB interpretation. An injected tap
        // also lacks a proven mapping back to the loaded photograph.
        if params
            .taps
            .inject
            .as_deref()
            .is_some_and(|tap| tap != "rgb_in")
            || params
                .taps
                .collect
                .as_deref()
                .is_some_and(|tap| tap != "rgb_out")
        {
            return None;
        }
        let (x, y, w, h) =
            spektrafilm_core::resizing::requested_crop_rect(working[0], working[1], &params.io)
                .ok()?;
        geometry.oriented_crop = [
            x as f64 / working[0] as f64,
            y as f64 / working[1] as f64,
            (x + w) as f64 / working[0] as f64,
            (y + h) as f64 / working[1] as f64,
        ];
        geometry.image_size = output_size;
        Some(geometry)
    }

    pub(crate) fn image_size(&self) -> [u32; 2] {
        self.image_size
    }

    pub(crate) fn coverage(&self) -> [f64; 4] {
        self.source_rect_from_image([0.0, 0.0, 1.0, 1.0])
    }

    /// Transform a normalized image rectangle into the unrotated source.
    pub(crate) fn source_rect_from_image(&self, rect: [f64; 4]) -> [f64; 4] {
        let c = self.oriented_crop;
        let r = [
            c[0] + rect[0] * (c[2] - c[0]),
            c[1] + rect[1] * (c[3] - c[1]),
            c[0] + rect[2] * (c[2] - c[0]),
            c[1] + rect[3] * (c[3] - c[1]),
        ];
        match self.rotation {
            0 => r,
            1 => [1.0 - r[3], r[0], 1.0 - r[1], r[2]],
            2 => [1.0 - r[2], 1.0 - r[3], 1.0 - r[0], 1.0 - r[1]],
            _ => [r[1], 1.0 - r[2], r[3], 1.0 - r[0]],
        }
    }

    /// Return no rectangle if any part of the source area is absent; never
    /// silently intersect an area with this subject's crop.
    pub(crate) fn image_rect_from_source(&self, rect: [f64; 4]) -> Option<[f64; 4]> {
        if !rect.iter().all(|v| v.is_finite()) || rect[0] >= rect[2] || rect[1] >= rect[3] {
            return None;
        }
        let r = match self.rotation {
            0 => rect,
            1 => [rect[1], 1.0 - rect[2], rect[3], 1.0 - rect[0]],
            2 => [1.0 - rect[2], 1.0 - rect[3], 1.0 - rect[0], 1.0 - rect[1]],
            _ => [1.0 - rect[3], rect[0], 1.0 - rect[1], rect[2]],
        };
        let c = self.oriented_crop;
        let epsilon = 8.0 * f64::EPSILON;
        if r[0] < c[0] - epsilon
            || r[1] < c[1] - epsilon
            || r[2] > c[2] + epsilon
            || r[3] > c[3] + epsilon
        {
            return None;
        }
        Some([
            ((r[0] - c[0]) / (c[2] - c[0])).clamp(0.0, 1.0),
            ((r[1] - c[1]) / (c[3] - c[1])).clamp(0.0, 1.0),
            ((r[2] - c[0]) / (c[2] - c[0])).clamp(0.0, 1.0),
            ((r[3] - c[1]) / (c[3] - c[1])).clamp(0.0, 1.0),
        ])
    }

    /// Pixel centers in the half-open source area, expressed as native image
    /// pixel bounds. A fully covered area can still contain no pixel centers.
    pub(crate) fn pixel_region(&self, rect: [f64; 4]) -> Option<[u32; 4]> {
        let r = self.image_rect_from_source(rect)?;
        let bound = |v: f64, n: u32, reversed: bool| {
            let coordinate = v * n as f64 - 0.5;
            let nearest = coordinate.round();
            let coordinate =
                if (coordinate - nearest).abs() <= 8.0 * f64::EPSILON * (n as f64).max(1.0) {
                    nearest
                } else {
                    coordinate
                };
            let index = if reversed {
                coordinate.floor() + 1.0
            } else {
                coordinate.ceil()
            };
            (index as i64).clamp(0, n as i64) as u32
        };
        let reverse_x = self.rotation == 2 || self.rotation == 3;
        let reverse_y = self.rotation == 1 || self.rotation == 2;
        let pixels = [
            bound(r[0], self.image_size[0], reverse_x),
            bound(r[1], self.image_size[1], reverse_y),
            bound(r[2], self.image_size[0], reverse_x),
            bound(r[3], self.image_size[1], reverse_y),
        ];
        (pixels[0] < pixels[2] && pixels[1] < pixels[3]).then_some(pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 4], b: [f64; 4]) {
        for (a, b) in a.into_iter().zip(b) {
            assert!((a - b).abs() < 1e-12, "{a} != {b}");
        }
    }

    #[test]
    fn rotations_map_source_rectangles_and_pixel_centers() {
        let area = [0.1, 0.25, 0.5, 0.75];
        let expected = [[1, 1, 5, 3], [1, 5, 3, 9], [5, 1, 9, 3], [1, 1, 3, 5]];
        for rotation in 0..4 {
            let g = AnalysisGeometry::original([10, 4], rotation);
            let image = g.image_rect_from_source(area).unwrap();
            close(g.source_rect_from_image(image), area);
            assert_eq!(g.pixel_region(area), Some(expected[rotation as usize]));
            assert_eq!(
                g.pixel_region(g.coverage()),
                Some([0, 0, g.image_size[0], g.image_size[1]])
            );
        }
    }

    #[test]
    fn crop_uses_preview_lattice_and_rejects_partial_coverage() {
        let mut params = RuntimeParams::default();
        params.io.crop = true;
        params.io.crop_center = [0.5, 0.5];
        params.io.crop_size = [0.5, 0.5];
        params.settings.preview_mode = true;
        params.settings.preview_max_size = 9;
        let g = AnalysisGeometry::rendered([19, 11], 0, [4, 4], &params).unwrap();
        let (w, h) = spektrafilm_core::resizing::preview_dimensions(19, 11, 9);
        let (x, y, cw, ch) =
            spektrafilm_core::resizing::requested_crop_rect(w, h, &params.io).unwrap();
        close(
            g.coverage(),
            [
                x as f64 / w as f64,
                y as f64 / h as f64,
                (x + cw) as f64 / w as f64,
                (y + ch) as f64 / h as f64,
            ],
        );
        assert_eq!(g.pixel_region(g.coverage()), Some([0, 0, 4, 4]));
        let mut partial = g.coverage();
        partial[0] -= 0.01;
        assert_eq!(g.pixel_region(partial), None);
    }

    #[test]
    fn full_frame_scanner_resolves_crop_before_preview_resize() {
        let mut params = RuntimeParams::default();
        params.io.crop = true;
        params.io.crop_center = [0.5, 0.5];
        params.io.crop_size = [0.5, 0.5];
        params.settings.preview_mode = true;
        params.settings.preview_max_size = 9;
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.experimental_setup = true;
        let g = AnalysisGeometry::rendered([19, 11], 0, [4, 4], &params).unwrap();
        let (x, y, w, h) =
            spektrafilm_core::resizing::requested_crop_rect(19, 11, &params.io).unwrap();
        close(
            g.coverage(),
            [
                x as f64 / 19.0,
                y as f64 / 11.0,
                (x + w) as f64 / 19.0,
                (y + h) as f64 / 11.0,
            ],
        );
    }

    #[test]
    fn intermediate_or_injected_taps_do_not_claim_output_rgb_geometry() {
        let mut params = RuntimeParams::default();
        params.taps.collect = Some("cmy_film".into());
        assert!(AnalysisGeometry::rendered([10, 10], 0, [10, 10], &params).is_none());
        params.taps.collect = Some("rgb_out".into());
        params.taps.inject = Some("log_e_film".into());
        assert!(AnalysisGeometry::rendered([10, 10], 0, [10, 10], &params).is_none());
    }

    #[test]
    fn half_open_membership_uses_pixel_centers_not_intersections() {
        let g = AnalysisGeometry::original([4, 2], 0);
        assert_eq!(g.pixel_region([0.125, 0.0, 0.625, 1.0]), Some([0, 0, 2, 2]));
        let rotated = AnalysisGeometry::original([4, 2], 1);
        assert_eq!(
            rotated.pixel_region([0.125, 0.0, 0.625, 1.0]),
            Some([0, 2, 2, 4])
        );
        assert_eq!(g.pixel_region([0.26, 0.0, 0.37, 1.0]), None);
        assert_eq!(g.pixel_region([0.0, 0.0, 1.001, 1.0]), None);
    }
}
