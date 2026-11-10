//! Cached per-pixel display transforms for full-source viewer sampling.
//!
//! Full-source viewing transforms every source tap before filtering. The
//! transform is pointwise in the source pixel, so caching each touched source
//! pixel's transformed value keeps every sampled pixel identical while zoom
//! and pan frames reuse the work. Tiles keep one array index per tap and let
//! a pan refill only its newly exposed edge. A view whose tiles exceed the
//! budget samples directly, exactly as before.

use std::sync::Arc;

use egui::{Pos2, Rect};
use rayon::prelude::*;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::to_f32;

use super::geometry::{contains_half_open, source_coords};
use super::raster::{Interpolation, sample_footprint_f32};

/// Source pixels per cache tile edge.
const TILE: usize = 256;
/// Cache budget in transformed source pixels: 64 MiB of `[f32; 3]`.
const BUDGET_PX: usize = 64 * 1024 * 1024 / std::mem::size_of::<[f32; 3]>();

/// The display ICC state a cache identity must capture: the requested
/// profile, whether its transform actually built, and the file stamp so a
/// replaced profile file is a different identity.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct ProfileStamp {
    path: String,
    built: bool,
    file: Option<(u64, std::time::SystemTime)>,
}

impl ProfileStamp {
    pub(super) fn capture(path: &std::path::Path, built: bool) -> Self {
        let file = std::fs::metadata(path)
            .ok()
            .and_then(|m| Some((m.len(), m.modified().ok()?)));
        Self {
            path: path.display().to_string(),
            built,
            file,
        }
    }
}

/// Identity of one per-pixel transform: the sampled allocation and every
/// display setting the transform reads.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct TransformIdentity {
    source: usize,
    width: u32,
    height: u32,
    space: String,
    encoded: bool,
    transform: bool,
    profile: Option<ProfileStamp>,
}

impl TransformIdentity {
    pub(super) fn new(
        source: &Arc<ImageBuf>,
        space: &str,
        encoded: bool,
        transform: bool,
        profile: Option<ProfileStamp>,
    ) -> Self {
        Self {
            source: Arc::as_ptr(source) as usize,
            width: source.width,
            height: source.height,
            space: space.to_owned(),
            encoded,
            transform,
            profile,
        }
    }
}

/// Transformed source pixels, tiled. One cache serves one active identity: a
/// different source or display setting drops every tile.
pub(super) struct TransformCache {
    identity: Option<TransformIdentity>,
    tiles: Vec<Option<Vec<[f32; 3]>>>,
    stamps: Vec<u64>,
    stamp: u64,
    filled: usize,
    budget_px: usize,
}

impl Default for TransformCache {
    fn default() -> Self {
        Self::with_budget(BUDGET_PX)
    }
}

impl TransformCache {
    pub(super) fn with_budget(budget_px: usize) -> Self {
        Self {
            identity: None,
            tiles: Vec::new(),
            stamps: Vec::new(),
            stamp: 0,
            filled: 0,
            budget_px,
        }
    }

    fn reset(&mut self, identity: TransformIdentity) {
        let tiles_x = identity.width.div_ceil(TILE as u32) as usize;
        let tiles_y = identity.height.div_ceil(TILE as u32) as usize;
        let count = tiles_x.saturating_mul(tiles_y);
        self.tiles = vec![None; count];
        self.stamps = vec![0; count];
        self.filled = 0;
        self.identity = Some(identity);
    }

    fn tiles_x(&self) -> usize {
        self.identity
            .as_ref()
            .map(|identity| identity.width.div_ceil(TILE as u32) as usize)
            .unwrap_or(1)
    }

    fn evict_to(&mut self, budget_tiles: usize) {
        while self.filled > budget_tiles {
            let Some((victim, _)) = self
                .tiles
                .iter()
                .zip(&self.stamps)
                .enumerate()
                .filter(|(_, (tile, _))| tile.is_some())
                .min_by_key(|(_, (_, stamp))| *stamp)
            else {
                break;
            };
            self.tiles[victim] = None;
            self.filled -= 1;
        }
    }

    /// Sample `texture_size` viewer pixels of `source` through `transform`.
    ///
    /// Cached and direct paths share source-cell area averaging for reductions
    /// and the selected interpolation for native-size or magnified views.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sample_view(
        &mut self,
        identity: &TransformIdentity,
        source: &Arc<ImageBuf>,
        transform: &(dyn Fn([f32; 3]) -> [f32; 3] + Sync),
        image_rect: Rect,
        rect: Rect,
        texture_size: [usize; 2],
        footprint: [f32; 2],
        interpolation: Interpolation,
    ) -> Vec<[f32; 3]> {
        let (w, h) = (source.width as usize, source.height as usize);
        let mut out = vec![[0.0f32; 3]; texture_size[0].saturating_mul(texture_size[1])];
        if w == 0 || h == 0 || out.is_empty() {
            return out;
        }
        if self.identity.as_ref() != Some(identity) {
            self.reset(identity.clone());
        }
        // Include every cell touched by either interpolation or the area filter.
        let corner = |x: usize, y: usize| {
            let p = Pos2::new(
                rect.min.x + (x as f32 + 0.5) * rect.width() / texture_size[0] as f32,
                rect.min.y + (y as f32 + 0.5) * rect.height() / texture_size[1] as f32,
            );
            source_coords(p, image_rect, identity.width, identity.height)
        };
        let margin = |side: f32| {
            let area = if interpolation == Interpolation::Nearest {
                0.0
            } else {
                side * 0.5
            };
            (interpolation.radius() as f32).max(area).ceil() as isize + 1
        };
        let (margin_x, margin_y) = (margin(footprint[0]), margin(footprint[1]));
        let (x0, y0) = corner(0, 0);
        let (x1, y1) = corner(texture_size[0] - 1, texture_size[1] - 1);
        let lo_x = (x0.min(x1).floor() as isize - margin_x).clamp(0, w as isize - 1) as usize;
        let hi_x = (x0.max(x1).floor() as isize + margin_x).clamp(0, w as isize - 1) as usize;
        let lo_y = (y0.min(y1).floor() as isize - margin_y).clamp(0, h as isize - 1) as usize;
        let hi_y = (y0.max(y1).floor() as isize + margin_y).clamp(0, h as isize - 1) as usize;
        let tiles_x = self.tiles_x();
        let (tx0, tx1) = (lo_x / TILE, hi_x / TILE);
        let (ty0, ty1) = (lo_y / TILE, hi_y / TILE);
        if (tx1 - tx0 + 1) * (ty1 - ty0 + 1) * TILE * TILE > self.budget_px {
            fill_rows(&mut out, texture_size, rect, &|p| {
                if contains_half_open(image_rect, p) {
                    let (sx, sy) = source_coords(p, image_rect, identity.width, identity.height);
                    sample_footprint_f32(sx, sy, interpolation, footprint, |x, y| {
                        transform(
                            source
                                .get(
                                    x.clamp(0, w as i32 - 1) as u32,
                                    y.clamp(0, h as i32 - 1) as u32,
                                )
                                .map(to_f32),
                        )
                    })
                } else {
                    [0.0; 3]
                }
            });
            return out;
        }
        self.stamp += 1;
        let stamp = self.stamp;
        let mut missing = Vec::new();
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let index = ty * tiles_x + tx;
                if self.tiles[index].is_some() {
                    self.stamps[index] = stamp;
                } else {
                    missing.push((tx, ty, index));
                }
            }
        }
        let filled: Vec<(usize, Vec<[f32; 3]>)> = missing
            .par_iter()
            .map(|&(tx, ty, _)| (ty * tiles_x + tx, fill_tile(source, transform, tx, ty)))
            .collect();
        for (index, pixels) in filled {
            self.tiles[index] = Some(pixels);
            self.stamps[index] = stamp;
            self.filled += 1;
        }
        self.evict_to((self.budget_px / (TILE * TILE)).max(1));
        let grid = TileGrid {
            tiles: &self.tiles,
            tiles_x,
            width: w,
            height: h,
        };
        fill_rows(&mut out, texture_size, rect, &|p| {
            if contains_half_open(image_rect, p) {
                let (sx, sy) = source_coords(p, image_rect, identity.width, identity.height);
                sample_footprint_f32(sx, sy, interpolation, footprint, |x, y| grid.fetch(x, y))
            } else {
                [0.0; 3]
            }
        });
        out
    }
}

struct TileGrid<'a> {
    tiles: &'a [Option<Vec<[f32; 3]>>],
    tiles_x: usize,
    width: usize,
    height: usize,
}

impl TileGrid<'_> {
    fn fetch(&self, x: i32, y: i32) -> [f32; 3] {
        let x = x.clamp(0, self.width as i32 - 1) as usize;
        let y = y.clamp(0, self.height as i32 - 1) as usize;
        let (tx, ty) = (x / TILE, y / TILE);
        self.tiles[ty * self.tiles_x + tx]
            .as_ref()
            .map(|tile| tile[(y - ty * TILE) * TILE + (x - tx * TILE)])
            // The sampled window fills every tile it taps before sampling.
            .unwrap_or([0.0; 3])
    }
}

fn fill_tile(
    source: &ImageBuf,
    transform: &(dyn Fn([f32; 3]) -> [f32; 3] + Sync),
    tx: usize,
    ty: usize,
) -> Vec<[f32; 3]> {
    let (w, h) = (source.width as usize, source.height as usize);
    let (x0, y0) = (tx * TILE, ty * TILE);
    let mut pixels = vec![[0.0f32; 3]; TILE * TILE];
    pixels
        .par_chunks_mut(TILE)
        .enumerate()
        .for_each(|(ly, row)| {
            let sy = (y0 + ly).min(h - 1) as u32;
            for (lx, slot) in row.iter_mut().enumerate() {
                let sx = (x0 + lx).min(w - 1) as u32;
                *slot = transform(source.get(sx, sy).map(to_f32));
            }
        });
    pixels
}

fn fill_rows(
    out: &mut [[f32; 3]],
    texture_size: [usize; 2],
    rect: Rect,
    sample: &(impl Fn(Pos2) -> [f32; 3] + Sync),
) {
    out.par_chunks_mut(texture_size[0].max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, slot) in row.iter_mut().enumerate() {
                let p = Pos2::new(
                    rect.min.x + (x as f32 + 0.5) * rect.width() / texture_size[0] as f32,
                    rect.min.y + (y as f32 + 0.5) * rect.height() / texture_size[1] as f32,
                );
                *slot = sample(p);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(w: u32, h: u32) -> Arc<ImageBuf> {
        let mut data = Vec::with_capacity(w as usize * h as usize * 3);
        for y in 0..h {
            for x in 0..w {
                for c in 0..3 {
                    data.push((((x * 3 + y * 7 + c * 11) % 251) as f32 / 251.0) as _);
                }
            }
        }
        Arc::new(ImageBuf::from_data(w, h, data))
    }

    fn identity(source: &Arc<ImageBuf>) -> TransformIdentity {
        TransformIdentity::new(source, "sRGB", true, true, None)
    }

    fn transform(value: [f32; 3]) -> [f32; 3] {
        value.map(|v| (v * 0.5 + 0.1).clamp(0.0, 1.0))
    }

    fn inverted(value: [f32; 3]) -> [f32; 3] {
        value.map(|v| 1.0 - v)
    }

    fn view(source: &Arc<ImageBuf>, zoom: f32) -> (Rect, Rect, [usize; 2]) {
        let size = egui::vec2(source.width as f32, source.height as f32);
        let rect = Rect::from_min_size(Pos2::ZERO, size * zoom);
        (rect, rect, [source.width as usize, source.height as usize])
    }

    #[test]
    fn cached_sampling_matches_direct_sampling() {
        let source = source(300, 200);
        let identity = identity(&source);
        // The zero budget forces the direct path on the same code the old
        // sampler ran, so the comparison is cache against direct pixels.
        let mut direct = TransformCache::with_budget(0);
        let mut cached = TransformCache::default();
        for (zoom, interpolation) in [
            (1.0, Interpolation::Nearest),
            (1.0, Interpolation::Spline36),
            (3.0, Interpolation::Spline36),
        ] {
            let (image_rect, rect, texture_size) = view(&source, zoom);
            let expected = direct.sample_view(
                &identity,
                &source,
                &transform,
                image_rect,
                rect,
                texture_size,
                [1.0, 1.0],
                interpolation,
            );
            let got = cached.sample_view(
                &identity,
                &source,
                &transform,
                image_rect,
                rect,
                texture_size,
                [1.0, 1.0],
                interpolation,
            );
            assert_eq!(expected, got, "zoom {zoom} {interpolation:?}");
        }
    }

    #[test]
    fn reduced_grain_averages_source_cells_after_the_display_transform() {
        let source = Arc::new(ImageBuf::from_data(
            99,
            63,
            (0..99 * 63)
                .flat_map(|i| {
                    [
                        (if i % 3 == 2 { 0.75 } else { 0.25 }) as _,
                        (if (i / 99) % 3 == 2 { 0.75 } else { 0.25 }) as _,
                        0.5 as _,
                    ]
                })
                .collect(),
        ));
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(33.0, 21.0));
        for budget in [0, BUDGET_PX] {
            let pixels = TransformCache::with_budget(budget).sample_view(
                &identity(&source),
                &source,
                &|rgb| rgb.map(|v| v * v),
                rect,
                rect,
                [33, 21],
                [3.0, 3.0],
                Interpolation::Spline36,
            );
            for rgb in pixels {
                for (got, expected) in rgb.into_iter().zip([11.0 / 48.0, 11.0 / 48.0, 0.25]) {
                    assert!((got - expected).abs() < 1e-6, "{got} != {expected}");
                }
            }
        }
    }

    #[test]
    fn explicit_footprint_selects_filter_independently_of_translation() {
        let source = Arc::new(ImageBuf::from_data(
            4,
            1,
            (0..4).flat_map(|x| [((x % 2) as f32) as _; 3]).collect(),
        ));
        let identity = identity(&source);
        for budget in [0, BUDGET_PX] {
            let mut cache = TransformCache::with_budget(budget);
            for origin in [0.0, 100.0, 2000.0] {
                let image_rect =
                    Rect::from_min_size(Pos2::new(origin, origin), egui::vec2(4.0, 1.0));
                let rect =
                    Rect::from_min_size(Pos2::new(origin + 0.25, origin), egui::vec2(1.0, 1.0));
                for (footprint, expected) in
                    [([1.0, 1.0], 1.890625 / 6.0), ([1.001, 1.0], 0.2505 / 1.001)]
                {
                    let pixels = cache.sample_view(
                        &identity,
                        &source,
                        &|rgb| rgb,
                        image_rect,
                        rect,
                        [1, 1],
                        footprint,
                        Interpolation::Cubic,
                    );
                    assert!(
                        (pixels[0][0] - expected).abs() < 1e-6,
                        "origin {origin} footprint {footprint:?}: {} != {expected}",
                        pixels[0][0]
                    );
                }
            }
        }
    }

    #[test]
    fn reduced_pixel_reads_both_sides_of_a_cache_tile_boundary() {
        let source = Arc::new(ImageBuf::from_data(
            600,
            1,
            (0..600).flat_map(|x| [(x < 256) as u8 as _; 3]).collect(),
        ));
        let image_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 1.0));
        let rect = Rect::from_min_size(Pos2::new(240.0, 0.0), egui::vec2(40.0, 1.0));
        for budget in [0, BUDGET_PX] {
            let pixels = TransformCache::with_budget(budget).sample_view(
                &identity(&source),
                &source,
                &|rgb| rgb,
                image_rect,
                rect,
                [1, 1],
                [40.0, 1.0],
                Interpolation::Spline36,
            );
            for value in pixels[0] {
                assert!((value - 0.4).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn warm_tiles_survive_a_pan_and_settings_changes_start_fresh() {
        let source = source(400, 300);
        let identity = identity(&source);
        let mut cache = TransformCache::default();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
        let image_rect = rect;
        let texture_size = [200usize, 150];
        let _ = cache.sample_view(
            &identity,
            &source,
            &transform,
            image_rect,
            rect,
            texture_size,
            [2.0, 2.0],
            Interpolation::Nearest,
        );
        let filled = cache.filled;
        assert!(filled > 0);
        // A pan keeps every tile the new window still taps.
        let panned = image_rect.translate(egui::vec2(4.0, 4.0));
        let _ = cache.sample_view(
            &identity,
            &source,
            &transform,
            panned,
            rect,
            texture_size,
            [2.0, 2.0],
            Interpolation::Nearest,
        );
        assert!(cache.filled >= filled);
        // A different display setting is a different transform: the old tiles
        // must not serve it.
        let mut changed = identity.clone();
        changed.encoded = false;
        let pixels = cache.sample_view(
            &changed,
            &source,
            &inverted,
            rect,
            rect,
            texture_size,
            [2.0, 2.0],
            Interpolation::Nearest,
        );
        let mut fresh = TransformCache::default();
        let expected = fresh.sample_view(
            &changed,
            &source,
            &inverted,
            rect,
            rect,
            texture_size,
            [2.0, 2.0],
            Interpolation::Nearest,
        );
        assert_eq!(expected, pixels, "stale tiles must not survive the change");
    }

    #[test]
    fn a_failed_profile_build_is_a_different_cache_identity() {
        let source = source(4, 4);
        let path = std::path::Path::new("/nonexistent/spektra-display.icc");
        let built = TransformIdentity::new(
            &source,
            "sRGB",
            true,
            true,
            Some(ProfileStamp::capture(path, true)),
        );
        let failed = TransformIdentity::new(
            &source,
            "sRGB",
            true,
            true,
            Some(ProfileStamp::capture(path, false)),
        );
        assert!(
            built != failed,
            "tiles from an unbuilt profile must not serve a built one"
        );
    }

    #[test]
    fn a_view_over_budget_samples_directly_without_filling_tiles() {
        let source = source(300, 200);
        let identity = identity(&source);
        let mut cache = TransformCache::default();
        let (image_rect, rect, texture_size) = view(&source, 1.0);
        let _ = cache.sample_view(
            &identity,
            &source,
            &transform,
            image_rect,
            rect,
            texture_size,
            [1.0, 1.0],
            Interpolation::Nearest,
        );
        assert!(cache.filled > 0);
        // The same window with a budget too small for its tiles goes direct.
        let mut tiny = TransformCache::with_budget(TILE * TILE / 2);
        let _ = tiny.sample_view(
            &identity,
            &source,
            &transform,
            image_rect,
            rect,
            texture_size,
            [1.0, 1.0],
            Interpolation::Nearest,
        );
        assert_eq!(tiny.filled, 0);
    }
}
