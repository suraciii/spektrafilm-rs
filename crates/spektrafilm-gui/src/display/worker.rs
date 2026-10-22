//! Viewer composition worker. Prepares disposable fit data and bounded
//! viewport pixels off the UI thread; immutable pipeline floats are shared.
use egui::{Color32, Pos2, Rect};
use parking_lot::{Condvar, Mutex};
use rayon::prelude::*;
use spektrafilm_math::image::ImageBuf;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use super::ViewLayer;
use super::colortransform::{input_display_raster, output_display_raster};
use super::decorations::{PolaroidState, virtual_paper_back};
use super::full_transform::{ProfileStamp, TransformCache, TransformIdentity};
use super::raster::{DisplayRaster, Interpolation, sample_rect};
#[derive(Clone)]
pub(super) struct PreparedDisplay {
    pub(super) source: Arc<ImageBuf>,
    pub(super) metadata: DisplaySource,
    pub(super) max_size: usize,
    pub(super) raster: DisplayRaster,
}
impl PreparedDisplay {
    fn matches(&self, source: &Arc<ImageBuf>, metadata: &DisplaySource, max_size: usize) -> bool {
        Arc::ptr_eq(&self.source, source) && self.metadata == *metadata && self.max_size == max_size
    }
}

#[derive(Clone)]
pub(super) struct DisplayRequest {
    pub(super) key: Vec<u64>,
    pub(super) generation: u64,
    pub(super) rect: Rect,
    pub(super) border: Rect,
    pub(super) paper_rect: Rect,
    pub(super) image_rect: Rect,
    pub(super) texture_size: [usize; 2],
    pub(super) gray: Color32,
    pub(super) layer: ViewLayer,
    pub(super) interpolation: Interpolation,
    pub(super) raster: Option<DisplayRaster>,
    pub(super) paper_size: [usize; 2],
    pub(super) previous: Option<DisplayRaster>,
    /// Before/after split: `(fraction of image_rect width, left-side raster)`.
    /// When present and `layer == Output`, pixels left of the split sample the
    /// left-side (input) raster while pixels right of it sample the output.
    pub(super) split: Option<(f32, DisplayRaster)>,
    pub(super) source: Option<Arc<ImageBuf>>,
    pub(super) prepared: Option<PreparedDisplay>,
    pub(super) metadata: DisplaySource,
    pub(super) max_size: usize,
    pub(super) full_source: bool,
    pub(super) polaroid: bool,
    pub(super) ctx: egui::Context,
    pub(super) frame: usize,
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct DisplaySource {
    pub(super) space: Option<String>,
    pub(super) encoded: bool,
    pub(super) transform: bool,
    pub(super) profile: Option<PathBuf>,
}

struct FitCacheEntry {
    prepared: PreparedDisplay,
    status: Option<String>,
}

#[derive(Default)]
struct CompositionCache {
    paper: Option<DisplayRaster>,
    polaroid: Option<(DisplayRaster, PolaroidState)>,
    fit: [Option<FitCacheEntry>; 2],
    full: TransformCache,
}

pub(super) struct DisplayResponse {
    pub(super) key: Vec<u64>,
    pub(super) generation: u64,
    pub(super) image: egui::ColorImage,
    pub(super) status: Option<String>,
    pub(super) prepared: Option<PreparedDisplay>,
}
impl DisplayResponse {
    pub(super) fn matches(&self, generation: u64, key: &[u64]) -> bool {
        self.generation == generation && self.key == key
    }
}
struct WorkerState {
    request: Option<DisplayRequest>,
    response: Option<DisplayResponse>,
    stopping: bool,
}
pub(super) struct DisplayWorker {
    state: Arc<(Mutex<WorkerState>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}
impl Default for DisplayWorker {
    fn default() -> Self {
        Self::new()
    }
}
impl DisplayWorker {
    fn new() -> Self {
        let state = Arc::new((
            Mutex::new(WorkerState {
                request: None,
                response: None,
                stopping: false,
            }),
            Condvar::new(),
        ));
        let shared = Arc::clone(&state);
        let thread = thread::Builder::new()
            .name("viewer-composition".into())
            .spawn(move || {
                let mut cache = CompositionCache::default();
                loop {
                    let request = {
                        let (lock, wake) = &*shared;
                        let mut state = lock.lock();
                        while state.request.is_none() && !state.stopping {
                            wake.wait(&mut state);
                        }
                        if state.stopping {
                            break;
                        }
                        state.request.take().unwrap()
                    };
                    let ctx = request.ctx.clone();
                    let response = compose_display(request, &mut cache);
                    let (lock, _) = &*shared;
                    let mut state = lock.lock();
                    if state.stopping {
                        break;
                    }
                    if state.request.is_none() {
                        state.response = Some(response);
                        drop(state);
                        ctx.request_repaint();
                    }
                }
            })
            .expect("viewer composition worker");
        Self {
            state,
            thread: Some(thread),
        }
    }
    pub(super) fn submit(&self, request: DisplayRequest) {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock();
        state.request = Some(request);
        state.response = None;
        wake.notify_one();
    }
    pub(super) fn take_response(&self) -> Option<DisplayResponse> {
        self.state.0.lock().response.take()
    }
}
impl Drop for DisplayWorker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock();
        state.stopping = true;
        state.request = None;
        state.response = None;
        wake.notify_one();
        drop(state);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl DisplaySource {
    /// Draws only visible pixels into a bounded texture. Large source images and
    /// offscreen pans therefore cannot exceed the GPU texture dimension limit.
    ///
    /// Every source tap is transformed before interpolation. The transform is
    /// pointwise in the source pixel, so [`TransformCache`] memoizes it per
    /// source pixel and the sampled pixels stay identical to the direct
    /// per-tap computation.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn full_source_view(
        &self,
        source: &Arc<ImageBuf>,
        layer: ViewLayer,
        image_rect: Rect,
        rect: Rect,
        texture_size: [usize; 2],
        interpolation: Interpolation,
        cache: &mut TransformCache,
    ) -> Option<(DisplayRaster, Option<String>)> {
        if source.width == 0 || source.height == 0 {
            return None;
        }
        let (name, encoded, enabled) = (self.space.as_deref(), self.encoded, self.transform);
        // Missing metadata must keep the prepared viewing artifact, never show raw output.
        let name = name?;
        let space = if enabled {
            Some(spektrafilm_math::colorspace::resolve(name).ok()?)
        } else {
            None
        };
        let mut status = None;
        let profile = if layer == ViewLayer::Output && enabled {
            self.profile.as_deref().and_then(|path| {
                use lcms2::{
                    DisallowCache, Flags, GlobalContext, Intent, PixelFormat, Profile, Transform,
                };
                // `NO_CACHE` makes the transform shareable across the
                // composition threads. The disabled cache is a lookup
                // optimization and does not change transformed pixels.
                let built: Result<
                    Transform<[u8; 3], [u8; 3], GlobalContext, DisallowCache>,
                    String,
                > = Profile::new_file(path)
                    .map_err(|error| error.to_string())
                    .and_then(|destination| {
                        Transform::new_flags_context(
                            GlobalContext::new(),
                            &Profile::new_srgb(),
                            PixelFormat::RGB_8,
                            &destination,
                            PixelFormat::RGB_8,
                            Intent::Perceptual,
                            Flags::NO_CACHE,
                        )
                        .map_err(|error| error.to_string())
                    });
                match built {
                    Ok(transform) => Some(transform),
                    Err(e) => {
                        status = Some(format!(
                            "Display transform: ICC failed ({e}); viewing sRGB preview"
                        ));
                        None
                    }
                }
            })
        } else {
            None
        };
        let transform = |value: [f32; 3]| -> [f32; 3] {
            let value = match space {
                Some(space) => spektrafilm_math::colorspace::display_rgb(
                    value.map(|v| v as f64),
                    space,
                    encoded,
                )
                .map(|v| v.clamp(0.0, 1.0) as f32),
                None => value,
            };
            if let Some(transform) = profile.as_ref() {
                let mut pixel = [value.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8)];
                transform.transform_in_place(&mut pixel);
                pixel[0].map(|v| v as f32 / 255.0)
            } else {
                value
            }
        };
        let identity = TransformIdentity::new(
            source,
            name,
            encoded,
            enabled,
            self.profile
                .as_deref()
                .map(|path| ProfileStamp::capture(path, profile.is_some())),
        );
        let rgb = cache.sample_view(
            &identity,
            source,
            &transform,
            image_rect,
            rect,
            texture_size,
            interpolation,
        );
        let raster = DisplayRaster::new(texture_size, rgb).ok()?;
        Some((raster, status))
    }
}

fn compose_display(request: DisplayRequest, cache: &mut CompositionCache) -> DisplayResponse {
    let DisplayRequest {
        key,
        generation,
        rect,
        border,
        paper_rect,
        image_rect,
        texture_size,
        gray,
        layer,
        interpolation,
        mut raster,
        paper_size,
        previous,
        split,
        source,
        prepared,
        metadata,
        max_size,
        full_source,
        polaroid,
        frame,
        ..
    } = request;
    if (layer == ViewLayer::PaperBack || !image_rect.contains_rect(paper_rect))
        && cache
            .paper
            .as_ref()
            .is_none_or(|paper| paper.size != paper_size)
    {
        cache.paper = Some(virtual_paper_back(paper_size));
    }
    let mut status = None;
    let mut current_prepared = None;
    if layer != ViewLayer::PaperBack
        && let (Some(source), Some(space)) = (source.as_ref(), metadata.space.as_deref())
    {
        let fit = &mut cache.fit[usize::from(layer == ViewLayer::Output)];
        let cached = fit
            .as_ref()
            .is_some_and(|entry| entry.prepared.matches(source, &metadata, max_size));
        if metadata.profile.is_none()
            && !cached
            && let Some(seed) = prepared.filter(|seed| seed.matches(source, &metadata, max_size))
        {
            *fit = Some(FitCacheEntry {
                prepared: seed,
                status: None,
            });
        }
        let cached = fit
            .as_ref()
            .is_some_and(|entry| entry.prepared.matches(source, &metadata, max_size));
        if !cached {
            let result = if layer == ViewLayer::Input {
                input_display_raster(source, space, metadata.encoded, max_size)
                    .map(|raster| (raster, None))
            } else {
                output_display_raster(
                    source,
                    space,
                    metadata.encoded,
                    metadata.transform,
                    metadata.profile.as_deref(),
                    max_size,
                )
                .map(|(raster, status)| (raster, Some(status)))
            };
            match result {
                Ok((prepared, message)) => {
                    *fit = Some(FitCacheEntry {
                        prepared: PreparedDisplay {
                            source: Arc::clone(source),
                            metadata: metadata.clone(),
                            max_size,
                            raster: prepared,
                        },
                        status: message,
                    })
                }
                Err(error) => status = Some(format!("Display transform: {error}")),
            }
        }
        if let Some(entry) = fit.as_ref()
            && entry.prepared.matches(source, &metadata, max_size)
        {
            raster = Some(entry.prepared.raster.clone());
            status = entry.status.clone();
            current_prepared = Some(entry.prepared.clone());
        }
    }
    if layer == ViewLayer::PaperBack {
        raster = cache.paper.clone();
    }
    let animation = if polaroid {
        raster.as_ref().map(|raster| {
            if cache
                .polaroid
                .as_ref()
                .is_none_or(|(image, _)| !Arc::ptr_eq(&image.rgb, &raster.rgb))
            {
                cache.polaroid = Some((raster.clone(), PolaroidState::new(raster)));
            }
            cache
                .polaroid
                .as_ref()
                .unwrap()
                .1
                .frame(frame as f32 / 49.0)
        })
    } else {
        None
    };
    let raster = animation.as_ref().or(raster.as_ref());
    let full = if full_source {
        source.as_ref().and_then(|source| {
            metadata.full_source_view(
                source,
                layer,
                image_rect,
                rect,
                texture_size,
                interpolation,
                &mut cache.full,
            )
        })
    } else {
        None
    };
    if let Some((_, message)) = full.as_ref() {
        status = message.clone();
    }
    let mut pixels = vec![Color32::TRANSPARENT; texture_size[0] * texture_size[1]];
    let render_pixel = |x: usize, y: usize| -> Color32 {
        let p = Pos2::new(
            rect.min.x + (x as f32 + 0.5) * rect.width() / texture_size[0] as f32,
            rect.min.y + (y as f32 + 0.5) * rect.height() / texture_size[1] as f32,
        );
        let mut color = if border.contains(p) {
            Color32::WHITE
        } else {
            gray
        };
        if paper_rect.contains(p)
            && !image_rect.contains(p)
            && let Some(paper) = cache.paper.as_ref()
        {
            color = pack(sample_rect(paper, p, paper_rect, Interpolation::Spline36));
        }
        if image_rect.contains(p)
            && let Some(raster) = raster
        {
            let mut rgb = if let Some((view, _)) = full.as_ref() {
                view.rgb[y * texture_size[0] + x]
            } else {
                sample_rect(raster, p, image_rect, interpolation)
            };
            // Before/after split: left of the split samples the left-side
            // (input) raster while pixels right of it keep the output.
            if layer == ViewLayer::Output
                && let Some((fraction, left)) = split.as_ref()
            {
                let split_x = image_rect.min.x + image_rect.width() * fraction.clamp(0.0, 1.0);
                if p.x < split_x {
                    rgb = sample_rect(left, p, image_rect, interpolation);
                }
            }
            if layer == ViewLayer::Output
                && let Some(previous) = previous.as_ref()
                && split.is_none()
            {
                let a = frame as f32 / 10.0;
                let old = sample_rect(previous, p, image_rect, interpolation);
                for c in 0..3 {
                    rgb[c] = old[c].clamp(0.0, 1.0) * (1.0 - a) + rgb[c].clamp(0.0, 1.0) * a;
                }
            }
            color = pack(rgb);
        }
        color
    };
    // Debug builds sample about a microsecond per pixel and the headless
    // suites run their cases concurrently on the same rayon pool, so the
    // parallel fill loses there; release builds spread it over the cores.
    if cfg!(debug_assertions) {
        fill_canvas_serial(&mut pixels, texture_size[0], render_pixel);
    } else {
        fill_canvas_parallel(&mut pixels, texture_size[0], render_pixel);
    }
    DisplayResponse {
        key,
        generation,
        image: egui::ColorImage {
            size: texture_size,
            pixels,
        },
        status,
        prepared: current_prepared,
    }
}

/// Fills canvas pixels in row-major order on the calling thread. The parallel
/// fill must agree with this one pixel for pixel, which the test below checks.
fn fill_canvas_serial(
    pixels: &mut [Color32],
    width: usize,
    render: impl Fn(usize, usize) -> Color32,
) {
    for (y, row) in pixels.chunks_mut(width.max(1)).enumerate() {
        for (x, slot) in row.iter_mut().enumerate() {
            *slot = render(x, y);
        }
    }
}

/// Fills canvas pixels in row-major order across the rayon pool. Pixels are
/// independent and keep their own arithmetic order, so the result is identical
/// to `fill_canvas_serial` while the work spreads over the available cores.
fn fill_canvas_parallel(
    pixels: &mut [Color32],
    width: usize,
    render: impl Fn(usize, usize) -> Color32 + Send + Sync,
) {
    if width == 0 || pixels.is_empty() {
        return;
    }
    pixels
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, slot) in row.iter_mut().enumerate() {
                *slot = render(x, y);
            }
        });
}

fn pack(p: [f32; 3]) -> Color32 {
    Color32::from_rgb(
        (p[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (p[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (p[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Vec2;
    use std::time::Duration;

    #[test]
    fn parallel_canvas_fill_matches_the_serial_fill() {
        let (w, h) = (37usize, 19usize);
        let render = |x: usize, y: usize| {
            Color32::from_rgba_premultiplied(
                (x * 7 % 251) as u8,
                (y * 11 % 253) as u8,
                ((x + 3 * y) % 255) as u8,
                255,
            )
        };
        let mut serial = vec![Color32::TRANSPARENT; w * h];
        let mut parallel = vec![Color32::TRANSPARENT; w * h];
        fill_canvas_serial(&mut serial, w, render);
        fill_canvas_parallel(&mut parallel, w, render);
        assert_eq!(serial, parallel);
        // Row-major slot mapping: index x + y * width.
        assert_eq!(serial[3 + 5 * w], render(3, 5));
    }
    fn request(generation: u64) -> DisplayRequest {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(4.0, 1.0));
        DisplayRequest {
            key: vec![1],
            generation,
            rect,
            border: rect,
            paper_rect: rect,
            image_rect: rect,
            texture_size: [4, 1],
            gray: Color32::BLACK,
            layer: ViewLayer::Output,
            interpolation: Interpolation::Nearest,
            raster: Some(DisplayRaster::new([1, 1], vec![[0.5; 3]]).unwrap()),
            paper_size: [128, 128],
            previous: None,
            split: None,
            prepared: None,
            source: Some(Arc::new(ImageBuf::from_data(
                4,
                1,
                vec![
                    0.0 as _, 0.0 as _, 0.0 as _, 1.0 as _, 1.0 as _, 1.0 as _, 0.0 as _, 0.0 as _,
                    0.0 as _, 1.0 as _, 1.0 as _, 1.0 as _,
                ],
            ))),
            metadata: DisplaySource {
                space: Some("sRGB".into()),
                encoded: true,
                transform: false,
                profile: None,
            },
            max_size: 1,
            full_source: true,
            polaroid: false,
            ctx: egui::Context::default(),
            frame: 0,
        }
    }

    #[test]
    fn worker_composition_keeps_full_resolution_and_rejects_old_generation() {
        let mut cache = CompositionCache {
            paper: Some(DisplayRaster::new([128, 128], vec![[1.0; 3]; 128 * 128]).unwrap()),
            ..CompositionCache::default()
        };
        let response = compose_display(request(2), &mut cache);
        assert_eq!(
            response.image.pixels,
            vec![
                Color32::BLACK,
                Color32::WHITE,
                Color32::BLACK,
                Color32::WHITE
            ]
        );
        assert!(response.matches(2, &[1]));
        assert!(!response.matches(3, &[1]));
        assert!(!response.matches(2, &[2]));
    }

    #[test]
    fn fit_composition_reuses_prepared_pixels_when_geometry_changes() {
        let mut cache = CompositionCache {
            paper: Some(DisplayRaster::new([128, 128], vec![[1.0; 3]; 128 * 128]).unwrap()),
            ..CompositionCache::default()
        };
        let mut fit = request(1);
        fit.full_source = false;
        fit.max_size = 4;
        let seed =
            output_display_raster(fit.source.as_ref().unwrap(), "sRGB", true, false, None, 4)
                .unwrap()
                .0;
        fit.prepared = Some(PreparedDisplay {
            source: fit.source.as_ref().unwrap().clone(),
            metadata: fit.metadata.clone(),
            max_size: 4,
            raster: seed.clone(),
        });
        let response = compose_display(fit.clone(), &mut cache);
        assert_eq!(
            response.image.pixels,
            vec![
                Color32::BLACK,
                Color32::WHITE,
                Color32::BLACK,
                Color32::WHITE
            ]
        );
        let pixels = Arc::clone(&cache.fit[1].as_ref().unwrap().prepared.raster.rgb);
        assert!(Arc::ptr_eq(&pixels, &seed.rgb));
        fit.generation = 2;
        fit.image_rect = fit.image_rect.translate(Vec2::new(1.0, 0.0));
        compose_display(fit, &mut cache);
        assert!(Arc::ptr_eq(
            &pixels,
            &cache.fit[1].as_ref().unwrap().prepared.raster.rgb
        ));
    }

    #[test]
    fn fit_composition_rejects_stale_cache_and_seed() {
        let mut original = request(1);
        original.full_source = false;
        original.max_size = 4;
        for change in 0..3 {
            let mut cache = CompositionCache::default();
            let seed = compose_display(original.clone(), &mut cache)
                .prepared
                .unwrap();
            let mut changed = original.clone();
            changed.generation = 2;
            changed.prepared = Some(seed.clone());
            match change {
                0 => changed.source = request(2).source,
                1 => changed.metadata.encoded = false,
                _ => changed.max_size = 2,
            }
            let response = compose_display(changed.clone(), &mut cache);
            let prepared = response.prepared.unwrap();
            assert!(Arc::ptr_eq(
                &prepared.source,
                changed.source.as_ref().unwrap()
            ));
            assert!(prepared.metadata == changed.metadata);
            assert_eq!(prepared.max_size, changed.max_size);
            assert!(!Arc::ptr_eq(&prepared.raster.rgb, &seed.raster.rgb));
            assert!(Arc::ptr_eq(
                &prepared.raster.rgb,
                &cache.fit[1].as_ref().unwrap().prepared.raster.rgb
            ));
        }
    }

    #[test]
    fn the_shared_icc_transform_matches_the_cached_one() {
        // The composition threads share one transform created with
        // `NO_CACHE`. The disabled cache is a lookup optimization: it must not
        // change transformed pixels.
        use lcms2::{DisallowCache, Flags, GlobalContext, Intent, PixelFormat, Profile, Transform};
        let cached = Transform::<[u8; 3], [u8; 3]>::new(
            &Profile::new_srgb(),
            PixelFormat::RGB_8,
            &Profile::new_srgb(),
            PixelFormat::RGB_8,
            Intent::Perceptual,
        )
        .unwrap();
        let shared: Transform<[u8; 3], [u8; 3], GlobalContext, DisallowCache> =
            Transform::new_flags_context(
                GlobalContext::new(),
                &Profile::new_srgb(),
                PixelFormat::RGB_8,
                &Profile::new_srgb(),
                PixelFormat::RGB_8,
                Intent::Perceptual,
                Flags::NO_CACHE,
            )
            .unwrap();
        for value in 0..=255u8 {
            let mut was = [[value, 255 - value, value / 2]];
            let mut now = was;
            cached.transform_in_place(&mut was);
            shared.transform_in_place(&mut now);
            assert_eq!(was, now);
        }
    }

    #[test]
    fn persistent_worker_publishes_latest_composition_and_shuts_down() {
        let worker = DisplayWorker::new();
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let latest = request(7);
            for _ in 0..3 {
                let _ = latest.ctx.run(egui::RawInput::default(), |_| {});
            }
            latest.ctx.set_request_repaint_callback(move |_| {
                let _ = tx.send(());
            });
            worker.submit(latest);
            rx.recv_timeout(Duration::from_secs(10))
                .expect("composition worker repaint timed out");
            let response = worker
                .take_response()
                .expect("composition ready before repaint");
            assert!(response.matches(7, &[1]));
            assert_eq!(
                response.image.pixels,
                vec![
                    Color32::BLACK,
                    Color32::WHITE,
                    Color32::BLACK,
                    Color32::WHITE
                ]
            );
        }
        drop(worker);
    }
}
