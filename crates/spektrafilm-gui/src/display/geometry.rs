//! Coordinate mapping between the viewport, the presented image, and source
//! pixels. All display surfaces share these half-open and pixel-center rules.
use egui::{Pos2, Rect, Vec2};

/// [width,height], with a common long edge of one. The output is fitted into
/// this input bounding box rather than independently normalized.
pub(super) fn normalized_world_size(size: [usize; 2]) -> Vec2 {
    let long = size[0].max(size[1]).max(1) as f32;
    Vec2::new(size[0] as f32 / long, size[1] as f32 / long)
}

pub(super) fn fitted_world_size(size: [usize; 2], bounds: Vec2) -> Vec2 {
    let p = Vec2::new(size[0].max(1) as f32, size[1].max(1) as f32);
    p * (bounds.x / p.x).min(bounds.y / p.y)
}

/// Places presentation samples at device-pixel centers inside the viewport.
/// The returned rectangle maps those centers to bounded texture texels.
pub(super) fn presentation_grid(
    rect: Rect,
    dpi: f32,
    max_side: usize,
) -> (Rect, [usize; 2], [f64; 2]) {
    let axis = |min: f32, max: f32| {
        // Pixel indices k satisfy min*dpi <= k+0.5 < max*dpi.
        let start = (f64::from(min) * f64::from(dpi) - 0.5).ceil();
        let end = (f64::from(max) * f64::from(dpi) - 0.5).ceil();
        (start, (end - start).max(1.0))
    };
    let (x, width) = axis(rect.min.x, rect.max.x);
    let (y, height) = axis(rect.min.y, rect.max.y);
    let size = [
        width.min(max_side as f64) as usize,
        height.min(max_side as f64) as usize,
    ];
    let grid = Rect::from_min_max(
        Pos2::new((x / f64::from(dpi)) as f32, (y / f64::from(dpi)) as f32),
        Pos2::new(
            ((x + width) / f64::from(dpi)) as f32,
            ((y + height) / f64::from(dpi)) as f32,
        ),
    );
    (
        grid,
        size,
        [width / size[0] as f64, height / size[1] as f64],
    )
}

/// Normalized image coordinates of `pos`, clamped to the image rectangle.
pub(super) fn image_point(rect: Rect, pos: Pos2) -> [f64; 2] {
    let uv = (pos - rect.min) / rect.size();
    [uv.x.clamp(0.0, 1.0) as f64, uv.y.clamp(0.0, 1.0) as f64]
}

/// Screen rectangle for normalized image bounds.
pub(super) fn image_overlay(rect: Rect, bounds: [f64; 4]) -> Rect {
    Rect::from_min_max(
        rect.min + rect.size() * Vec2::new(bounds[0] as f32, bounds[1] as f32),
        rect.min + rect.size() * Vec2::new(bounds[2] as f32, bounds[3] as f32),
    )
}

/// The minimum edge belongs to the image; the maximum does not.
pub(super) fn contains_half_open(rect: Rect, point: Pos2) -> bool {
    rect.min.x <= point.x && point.x < rect.max.x && rect.min.y <= point.y && point.y < rect.max.y
}

/// Source coordinates, in image pixels, of viewer position `p` mapped over
/// `rect`. The shared mapping of full-source sampling; the `- 0.5` keeps a
/// sampled position over the center of source pixel 0.
pub(super) fn source_coords(p: Pos2, rect: Rect, width: u32, height: u32) -> (f32, f32) {
    let uv = (p - rect.min) / rect.size();
    (uv.x * width as f32 - 0.5, uv.y * height as f32 - 0.5)
}
