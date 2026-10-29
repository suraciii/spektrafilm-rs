//! Viewer composition worker. Prepares disposable fit data and bounded
//! viewport pixels off the UI thread; immutable pipeline floats are shared.
use egui::{Color32, Pos2, Rect};
use parking_lot::{Condvar, Mutex};
use rayon::prelude::*;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use super::ViewLayer;
use super::colortransform::{input_display_raster_with, output_display_raster_with};
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
    /// Pixel-grid rectangle for composition samples, aligned to device pixels.
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
    pub(super) source: Option<Arc<ImageBuf>>,
    pub(super) retained_source: Option<spektrafilm_backend::DeviceImage>,
    pub(super) prepared: Option<PreparedDisplay>,
    pub(super) metadata: DisplaySource,
    pub(super) max_size: usize,
    pub(super) full_source: bool,
    pub(super) polaroid: bool,
    pub(super) ctx: egui::Context,
    pub(super) frame: usize,
    /// Compute backend for the viewer's resample and display transform. The
    /// viewer shares the application's backend, so its disposable raster
    /// arithmetic runs on the device when the backend has a device path.
    pub(super) backend: Arc<dyn ComputeBackend>,
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct DisplaySource {
    pub(super) space: Option<String>,
    pub(super) encoded: bool,
    pub(super) transform: bool,
    pub(super) profile: Option<PathBuf>,
}
/// Matches device composition: the minimum edge belongs to the image, the maximum does not.
pub(super) fn contains_half_open(rect: Rect, point: Pos2) -> bool {
    rect.min.x <= point.x && point.x < rect.max.x && rect.min.y <= point.y && point.y < rect.max.y
}

struct FitCacheEntry {
    prepared: PreparedDisplay,
    status: Option<crate::app::Message>,
}

struct DeviceFitEntry {
    source: spektrafilm_backend::DeviceImage,
    metadata: DisplaySource,
    max_size: usize,
    raster: DisplayRaster,
    status: crate::app::Message,
}
impl DeviceFitEntry {
    fn matches(
        &self,
        source: &spektrafilm_backend::DeviceImage,
        metadata: &DisplaySource,
        max_size: usize,
    ) -> bool {
        self.source
            .payload()
            .zip(source.payload())
            .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
            && self.metadata == *metadata
            && self.max_size == max_size
    }
}

/// Raw host uploads are independent of viewport geometry and viewing transforms.
/// Each layer owns only its current source; replacement releases the old upload.
struct DeviceSourceEntry {
    source: Arc<ImageBuf>,
    backend: Arc<dyn ComputeBackend>,
    device_id: Option<u64>,
    space: Option<String>,
    encoded: bool,
    image: spektrafilm_backend::DeviceImage,
}
impl DeviceSourceEntry {
    fn matches(
        &self,
        source: &Arc<ImageBuf>,
        backend: &Arc<dyn ComputeBackend>,
        metadata: &DisplaySource,
    ) -> bool {
        Arc::ptr_eq(&self.source, source)
            && Arc::ptr_eq(&self.backend, backend)
            && self.device_id == backend.preparation_device_id()
            && self.space == metadata.space
            && self.encoded == metadata.encoded
    }
}

#[derive(Default)]
struct CompositionCache {
    paper: Option<DisplayRaster>,
    polaroid: Option<(DisplayRaster, PolaroidState)>,
    fit: [Option<FitCacheEntry>; 2],
    device_fit: [Option<DeviceFitEntry>; 4],
    device_source: [Option<DeviceSourceEntry>; 2],
    full: TransformCache,
    #[cfg(test)]
    pause: Option<Arc<CompositionPause>>,
}

pub(super) struct DisplayResponse {
    pub(super) key: Vec<u64>,
    pub(super) generation: u64,
    pub(super) image: egui::ColorImage,
    pub(super) status: Option<crate::app::Message>,
    pub(super) error: Option<String>,
    pub(super) prepared: Option<PreparedDisplay>,
    /// Raw Original upload made for full-source viewing, shared with analysis.
    pub(super) uploaded_input: Option<spektrafilm_backend::DeviceImage>,
}
impl DisplayResponse {
    pub(super) fn matches(&self, generation: u64, key: &[u64]) -> bool {
        self.generation == generation && self.key == key
    }
}
#[cfg(test)]
struct CompositionPause {
    started: std::sync::mpsc::Sender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    finished: std::sync::mpsc::Sender<(u64, Option<String>)>,
}

#[cfg(test)]
struct InspectionPause {
    started: std::sync::mpsc::Sender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

pub(super) struct InspectionRequest {
    pub(super) generation: u64,
    pub(super) image: spektrafilm_backend::DeviceImage,
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) backend: Arc<dyn ComputeBackend>,
    pub(super) ctx: egui::Context,
}

pub(super) struct InspectionResponse {
    pub(super) generation: u64,
    pub(super) rgb: Option<[f64; 3]>,
}

struct WorkerState {
    request: Option<DisplayRequest>,
    response: Option<DisplayResponse>,
    active_cancel: Option<Arc<AtomicBool>>,
    inspection_request: Option<InspectionRequest>,
    inspection_response: Option<InspectionResponse>,
    active_inspection_cancel: Option<Arc<AtomicBool>>,
    #[cfg(test)]
    inspection_pause: Option<InspectionPause>,
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
        Self::start(
            #[cfg(test)]
            None,
        )
    }

    fn start(#[cfg(test)] pause: Option<Arc<CompositionPause>>) -> Self {
        let state = Arc::new((
            Mutex::new(WorkerState {
                request: None,
                response: None,
                active_cancel: None,
                inspection_request: None,
                inspection_response: None,
                active_inspection_cancel: None,
                #[cfg(test)]
                inspection_pause: None,
                stopping: false,
            }),
            Condvar::new(),
        ));
        let shared = Arc::clone(&state);
        let thread = thread::Builder::new()
            .name("viewer-composition".into())
            .spawn(move || {
                #[cfg(test)]
                let completion = pause.clone();
                let mut cache = CompositionCache {
                    #[cfg(test)]
                    pause,
                    ..CompositionCache::default()
                };
                loop {
                    let (request, cancellation) = {
                        let (lock, wake) = &*shared;
                        let mut state = lock.lock();
                        while state.request.is_none()
                            && state.inspection_request.is_none()
                            && !state.stopping
                        {
                            wake.wait(&mut state);
                        }
                        if state.stopping {
                            break;
                        }
                        // Hover has its own latest-only slot: pointer movement
                        // must not cancel the pending presentation.
                        if state.request.is_none() {
                            let request = state.inspection_request.take().unwrap();
                            let cancellation = Arc::new(AtomicBool::new(false));
                            state.active_inspection_cancel = Some(cancellation.clone());
                            #[cfg(test)]
                            let pause = state.inspection_pause.take();
                            drop(state);
                            #[cfg(test)]
                            if let Some(pause) = pause {
                                let _ = pause.started.send(());
                                let _ = pause.release.lock().recv();
                            }
                            let cancellable = request
                                .backend
                                .with_cancellation(Some(cancellation.clone()));
                            let backend =
                                cancellable.as_deref().unwrap_or(request.backend.as_ref());
                            let rgb = backend
                                .inspect_retained_pixel(&request.image, request.x, request.y)
                                .ok()
                                .map(|rgb| rgb.map(f64::from));
                            let mut state = lock.lock();
                            state.active_inspection_cancel = None;
                            if state.stopping {
                                break;
                            }
                            if state.inspection_request.is_none()
                                && !cancellation.load(Ordering::Relaxed)
                            {
                                state.inspection_response = Some(InspectionResponse {
                                    generation: request.generation,
                                    rgb,
                                });
                                drop(state);
                                request.ctx.request_repaint();
                            }
                            continue;
                        }
                        let cancellation = Arc::new(AtomicBool::new(false));
                        state.active_cancel = Some(cancellation.clone());
                        (state.request.take().unwrap(), cancellation)
                    };
                    let ctx = request.ctx.clone();
                    let response = compose_display_cancellable(
                        request,
                        &mut cache,
                        Some(cancellation.clone()),
                    );
                    #[cfg(test)]
                    if let Some(pause) = &completion {
                        let _ = pause
                            .finished
                            .send((response.generation, response.error.clone()));
                    }
                    let (lock, _) = &*shared;
                    let mut state = lock.lock();
                    state.active_cancel = None;
                    if state.stopping {
                        break;
                    }
                    if state.request.is_none() && !cancellation.load(Ordering::Relaxed) {
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
        if let Some(cancellation) = &state.active_cancel {
            cancellation.store(true, Ordering::Relaxed);
        }
        state.request = Some(request);
        state.response = None;
        wake.notify_one();
    }
    pub(super) fn take_response(&self) -> Option<DisplayResponse> {
        self.state.0.lock().response.take()
    }
    pub(super) fn has_pending(&self) -> bool {
        let state = self.state.0.lock();
        state.request.is_some() || state.active_cancel.is_some() || state.response.is_some()
    }
    /// Replace or clear hover work without disturbing composition.
    pub(super) fn inspect(&self, request: Option<InspectionRequest>) {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock();
        if let Some(cancellation) = &state.active_inspection_cancel {
            cancellation.store(true, Ordering::Relaxed);
        }
        state.inspection_request = request;
        state.inspection_response = None;
        wake.notify_one();
    }
    pub(super) fn take_inspection(&self) -> Option<InspectionResponse> {
        self.state.0.lock().inspection_response.take()
    }
}
impl Drop for DisplayWorker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock();
        if let Some(cancellation) = &state.active_cancel {
            cancellation.store(true, Ordering::Relaxed);
        }
        if let Some(cancellation) = &state.active_inspection_cancel {
            cancellation.store(true, Ordering::Relaxed);
        }
        state.stopping = true;
        state.request = None;
        state.response = None;
        state.inspection_request = None;
        state.inspection_response = None;
        wake.notify_one();
        drop(state);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl DisplaySource {
    #[cfg(test)]
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
    ) -> Option<(DisplayRaster, Option<crate::app::Message>)> {
        self.full_source_view_with(
            &spektrafilm_backend::cpu_backend::CpuBackend,
            source,
            layer,
            image_rect,
            rect,
            texture_size,
            interpolation,
            cache,
        )
    }

    /// Samples the full source using the selected backend's display transform,
    /// ICC pipeline and interpolation. A selected GPU has no host fallback.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn full_source_view_with(
        &self,
        backend: &dyn ComputeBackend,
        source: &Arc<ImageBuf>,
        layer: ViewLayer,
        image_rect: Rect,
        rect: Rect,
        texture_size: [usize; 2],
        interpolation: Interpolation,
        cache: &mut TransformCache,
    ) -> Option<(DisplayRaster, Option<crate::app::Message>)> {
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
        if backend.is_gpu() {
            let (prepared, message) = super::colortransform::output_display_raster_with(
                backend,
                source,
                name,
                encoded,
                enabled,
                if layer == ViewLayer::Output {
                    self.profile.as_deref()
                } else {
                    None
                },
                usize::MAX,
            )
            .ok()?;
            let transformed = prepared.retained.as_ref()?;
            let image = backend
                .viewer_full_source_retained(
                    transformed,
                    &spektrafilm_backend::ViewerFullSourceGpuParams {
                        tex_width: texture_size[0] as u32,
                        tex_height: texture_size[1] as u32,
                        rect_min: [rect.min.x, rect.min.y],
                        rect_size: [rect.width(), rect.height()],
                        image_min: [image_rect.min.x, image_rect.min.y],
                        image_size: [image_rect.width(), image_rect.height()],
                        interpolation: interpolation.discriminant(),
                        transform: false,
                        src_cctf: 0,
                        decode: false,
                        matrix: [[0.0; 3]; 3],
                        icc: false,
                        icc_matrix: false,
                        icc_matrix_fixed14: [[0; 3]; 3],
                        icc_offset_fixed14: [0; 3],
                        icc_first: &[Vec::new(), Vec::new(), Vec::new()],
                        icc_second: &[Vec::new(), Vec::new(), Vec::new()],
                    },
                )
                .ok()?;
            // This API is only the explicit host-raster probe; production uses
            // compose_display_device and transfers packed presentation bytes.
            let host = image.to_host(backend).ok()?;
            let rgb = host
                .data
                .as_chunks::<3>()
                .0
                .iter()
                .map(|v| [v[0] as f32, v[1] as f32, v[2] as f32])
                .collect();
            return Some((DisplayRaster::new(texture_size, rgb).ok()?, Some(message)));
        }
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
                        status = Some(
                            crate::app::Message::new("surfaces.display.failed").arg("error", e),
                        );
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

#[cfg(test)]
fn compose_display(request: DisplayRequest, cache: &mut CompositionCache) -> DisplayResponse {
    compose_display_cancellable(request, cache, None)
}

fn compose_display_cancellable(
    request: DisplayRequest,
    cache: &mut CompositionCache,
    cancellation: Option<Arc<AtomicBool>>,
) -> DisplayResponse {
    if request.backend.is_gpu() {
        return compose_display_device(request, cache, cancellation);
    }
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
        source,
        retained_source,
        prepared,
        metadata,
        max_size,
        full_source,
        polaroid,
        frame,
        backend,
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
    let mut error = None;
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
                input_display_raster_with(
                    backend.as_ref(),
                    source,
                    space,
                    metadata.encoded,
                    max_size,
                )
                .map(|raster| (raster, None))
            } else {
                if let Some(retained) = retained_source.as_ref() {
                    super::colortransform::output_display_raster_retained(
                        backend.as_ref(),
                        retained,
                        space,
                        metadata.encoded,
                        metadata.transform,
                        metadata.profile.as_deref(),
                        max_size,
                    )
                    .map(|(raster, status)| (raster, Some(status)))
                } else {
                    output_display_raster_with(
                        backend.as_ref(),
                        source,
                        space,
                        metadata.encoded,
                        metadata.transform,
                        metadata.profile.as_deref(),
                        max_size,
                    )
                    .map(|(raster, status)| (raster, Some(status)))
                }
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
                Err(message) => {
                    status = Some(
                        crate::app::Message::new("surfaces.display.error").arg("error", &message),
                    );
                    error = Some(message);
                }
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
            metadata.full_source_view_with(
                backend.as_ref(),
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
        let mut color = if contains_half_open(border, p) {
            Color32::WHITE
        } else {
            gray
        };
        if contains_half_open(paper_rect, p)
            && !contains_half_open(image_rect, p)
            && let Some(paper) = cache.paper.as_ref()
        {
            color = pack(sample_rect(paper, p, paper_rect, Interpolation::Spline36));
        }
        if contains_half_open(image_rect, p)
            && let Some(raster) = raster
        {
            let mut rgb = if let Some((view, _)) = full.as_ref() {
                view.rgb[y * texture_size[0] + x]
            } else {
                sample_rect(raster, p, image_rect, interpolation)
            };
            if layer == ViewLayer::Output
                && let Some(previous) = previous.as_ref()
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
        error,
        prepared: current_prepared,
        uploaded_input: None,
    }
}

/// A selected GPU never enters the host compositor. Missing accepted output
/// publishes an empty device-composed canvas; actual device failures keep the
/// last accepted texture and fit cache.
fn compose_display_device(
    request: DisplayRequest,
    cache: &mut CompositionCache,
    cancellation: Option<Arc<AtomicBool>>,
) -> DisplayResponse {
    let mut status = None;
    let mut prepared = None;
    let mut uploaded_input = None;
    let result = (|| -> Result<egui::ColorImage, String> {
        let cancellable = request.backend.with_cancellation(cancellation);
        let backend = cancellable.as_deref().unwrap_or(request.backend.as_ref());
        if (request.layer == ViewLayer::PaperBack
            || !request.image_rect.contains_rect(request.paper_rect))
            && cache
                .paper
                .as_ref()
                .is_none_or(|r| r.size != request.paper_size || r.retained.is_none())
        {
            let (w, h, alpha) = super::decorations::logo_alpha();
            cache.paper = Some(DisplayRaster::device(backend.viewer_paper_retained(
                request.paper_size[0] as u32,
                request.paper_size[1] as u32,
                &alpha,
                w as u32,
                h as u32,
            )?));
        }
        let mut raster = request.raster.clone();
        if request.layer != ViewLayer::PaperBack {
            if let Some(source) = request.source.as_ref() {
                let metadata = &request.metadata;
                let space = metadata
                    .space
                    .as_deref()
                    .ok_or("viewer source has no color space")?;
                let slot = &mut cache.fit[usize::from(request.layer == ViewLayer::Output)];
                if !slot
                    .as_ref()
                    .is_some_and(|e| e.prepared.matches(source, metadata, request.max_size))
                {
                    if let Some(seed) = request.prepared.as_ref().filter(|s| {
                        s.matches(source, metadata, request.max_size) && s.raster.retained.is_some()
                    }) {
                        *slot = Some(FitCacheEntry {
                            prepared: seed.clone(),
                            status: None,
                        });
                    } else {
                        let (fit, message) = if let Some(image) = request.retained_source.as_ref() {
                            super::colortransform::output_display_raster_retained(
                                backend,
                                image,
                                space,
                                metadata.encoded,
                                metadata.transform,
                                metadata.profile.as_deref(),
                                request.max_size,
                            )?
                        } else {
                            output_display_raster_with(
                                backend,
                                source,
                                space,
                                metadata.encoded,
                                metadata.transform,
                                metadata.profile.as_deref(),
                                request.max_size,
                            )?
                        };
                        *slot = Some(FitCacheEntry {
                            prepared: PreparedDisplay {
                                source: source.clone(),
                                metadata: metadata.clone(),
                                max_size: request.max_size,
                                raster: fit,
                            },
                            status: Some(message),
                        });
                    }
                }
                if let Some(entry) = slot.as_ref() {
                    raster = Some(entry.prepared.raster.clone());
                    status = entry.status.clone();
                    prepared = Some(entry.prepared.clone());
                }
            } else if let Some(source) = request.retained_source.as_ref() {
                let metadata = &request.metadata;
                let slot = &mut cache.device_fit[usize::from(request.layer == ViewLayer::Output)];
                if !slot
                    .as_ref()
                    .is_some_and(|e| e.matches(source, metadata, request.max_size))
                {
                    let (fit, message) = super::colortransform::output_display_raster_retained(
                        backend,
                        source,
                        metadata
                            .space
                            .as_deref()
                            .ok_or("viewer source has no color space")?,
                        metadata.encoded,
                        metadata.transform,
                        metadata.profile.as_deref(),
                        request.max_size,
                    )?;
                    *slot = Some(DeviceFitEntry {
                        source: source.clone(),
                        metadata: metadata.clone(),
                        max_size: request.max_size,
                        raster: fit,
                        status: message,
                    });
                }
                let entry = slot.as_ref().expect("device fit prepared");
                raster = Some(entry.raster.clone());
                status = Some(entry.status.clone());
            }
        } else {
            raster = cache.paper.clone();
        }
        let mut image = raster
            .as_ref()
            .map(|r| {
                r.retained
                    .clone()
                    .ok_or("viewer raster is not device-retained")
            })
            .transpose()?;
        if request.polaroid
            && let Some(current) = image.as_ref()
        {
            image = Some(backend.viewer_polaroid_retained(
                current,
                super::decorations::layer_coefficients(request.frame as f32 / 49.0),
            )?);
        }
        #[cfg(test)]
        if let Some(pause) = cache.pause.take() {
            let _ = pause.started.send(());
            let _ = pause.release.lock().recv();
        }
        if request.full_source && request.layer != ViewLayer::PaperBack && image.is_some() {
            let source = request.source.as_ref();
            let metadata = &request.metadata;
            let raw = if let Some(retained) = request.retained_source.as_ref() {
                retained.clone()
            } else {
                let source = source.ok_or("full-source viewer has no accepted source")?;
                let slot =
                    &mut cache.device_source[usize::from(request.layer == ViewLayer::Output)];
                if !slot
                    .as_ref()
                    .is_some_and(|entry| entry.matches(source, &request.backend, metadata))
                {
                    let space = spektrafilm_math::colorspace::resolve(
                        metadata
                            .space
                            .as_deref()
                            .ok_or("viewer source has no color space")?,
                    )?;
                    let image = backend.retain_image(
                        source,
                        &spektrafilm_backend::RetainedImageMeta {
                            width: source.width,
                            height: source.height,
                            domain: if metadata.encoded {
                                spektrafilm_backend::SampleDomain::Encoded
                            } else {
                                spektrafilm_backend::SampleDomain::Linear
                            },
                            precision: spektrafilm_backend::Precision::host(),
                            space,
                            pixel_pitch_um: None,
                        },
                    )?;
                    *slot = Some(DeviceSourceEntry {
                        source: source.clone(),
                        backend: request.backend.clone(),
                        device_id: request.backend.preparation_device_id(),
                        space: metadata.space.clone(),
                        encoded: metadata.encoded,
                        image,
                    });
                }
                slot.as_ref().expect("full source uploaded").image.clone()
            };
            if request.layer == ViewLayer::Input && request.retained_source.is_none() {
                uploaded_input = Some(raw.clone());
            }
            // Pointwise viewing transform before interpolation, exactly as the
            // CPU full-source tap cache. Same-size resampling is an identity.
            let slot = &mut cache.device_fit[2 + usize::from(request.layer == ViewLayer::Output)];
            if !slot
                .as_ref()
                .is_some_and(|e| e.matches(&raw, metadata, usize::MAX))
            {
                let (transformed, message) = super::colortransform::output_display_raster_retained(
                    backend,
                    &raw,
                    metadata
                        .space
                        .as_deref()
                        .ok_or("viewer source has no color space")?,
                    metadata.encoded,
                    metadata.transform,
                    metadata.profile.as_deref(),
                    usize::MAX,
                )?;
                *slot = Some(DeviceFitEntry {
                    source: raw.clone(),
                    metadata: metadata.clone(),
                    max_size: usize::MAX,
                    raster: transformed,
                    status: message,
                });
            }
            let entry = slot
                .as_ref()
                .expect("full-source viewing transform prepared");
            let transformed = &entry.raster;
            status = Some(entry.status.clone());
            image = Some(
                backend.viewer_full_source_retained(
                    transformed
                        .retained
                        .as_ref()
                        .ok_or("missing transformed source")?,
                    &spektrafilm_backend::ViewerFullSourceGpuParams {
                        tex_width: request.texture_size[0] as u32,
                        tex_height: request.texture_size[1] as u32,
                        rect_min: [request.rect.min.x, request.rect.min.y],
                        rect_size: [request.rect.width(), request.rect.height()],
                        image_min: [request.image_rect.min.x, request.image_rect.min.y],
                        image_size: [request.image_rect.width(), request.image_rect.height()],
                        interpolation: request.interpolation.discriminant(),
                        transform: false,
                        src_cctf: 0,
                        decode: false,
                        matrix: [[0.0; 3]; 3],
                        icc: false,
                        icc_matrix: false,
                        icc_matrix_fixed14: [[0; 3]; 3],
                        icc_offset_fixed14: [0; 3],
                        icc_first: &[Vec::new(), Vec::new(), Vec::new()],
                        icc_second: &[Vec::new(), Vec::new(), Vec::new()],
                    },
                )?,
            );
        }
        let previous = request
            .previous
            .as_ref()
            .filter(|_| image.is_some())
            .map(|r| {
                r.retained
                    .as_ref()
                    .ok_or("previous viewer raster belongs to host backend")
            })
            .transpose()?;
        let rect = |r: Rect| [r.min.x, r.min.y, r.max.x, r.max.y];
        let pixels = backend.viewer_compose_retained(
            image.as_ref(),
            cache.paper.as_ref().and_then(|r| r.retained.as_ref()),
            previous,
            &spektrafilm_backend::ViewerComposeGpuParams {
                width: request.texture_size[0] as u32,
                height: request.texture_size[1] as u32,
                rect: rect(request.rect),
                border: rect(request.border),
                paper_rect: rect(request.paper_rect),
                image_rect: rect(request.image_rect),
                gray: [
                    request.gray.r() as f32 / 255.0,
                    request.gray.g() as f32 / 255.0,
                    request.gray.b() as f32 / 255.0,
                ],
                interpolation: request.interpolation.discriminant(),
                full_source: request.full_source && request.layer != ViewLayer::PaperBack,
                output_layer: request.layer == ViewLayer::Output,
                fade: request.frame as f32 / 10.0,
            },
        )?;
        Ok(egui::ColorImage {
            size: request.texture_size,
            pixels: pixels
                .into_iter()
                .map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]))
                .collect(),
        })
    })();
    let (image, error) = match result {
        Ok(image) => (image, None),
        Err(error) => (
            egui::ColorImage {
                size: [0, 0],
                pixels: Vec::new(),
            },
            Some(error),
        ),
    };
    DisplayResponse {
        key: request.key,
        generation: request.generation,
        image,
        status,
        error,
        prepared,
        uploaded_input,
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
    pub(super) fn request(generation: u64) -> DisplayRequest {
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
            prepared: None,
            retained_source: None,
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
            backend: Arc::new(spektrafilm_backend::cpu_backend::CpuBackend),
            ctx: egui::Context::default(),
            frame: 0,
        }
    }

    #[test]
    fn full_source_upload_survives_pan_and_view_transform_but_not_source_or_interpretation() {
        use spektrafilm_backend::DeviceImagePayload;
        use std::sync::Weak;

        fn raw(cache: &CompositionCache) -> Weak<dyn DeviceImagePayload> {
            Arc::downgrade(
                cache.device_source[1]
                    .as_ref()
                    .unwrap()
                    .image
                    .payload()
                    .unwrap(),
            )
        }
        fn transformed(cache: &CompositionCache) -> Weak<dyn DeviceImagePayload> {
            Arc::downgrade(
                cache.device_fit[3]
                    .as_ref()
                    .unwrap()
                    .raster
                    .retained
                    .as_ref()
                    .unwrap()
                    .payload()
                    .unwrap(),
            )
        }
        let gpu = spektrafilm_gpu::test_support::explicit_adapter();
        let backend: Arc<dyn ComputeBackend> = Arc::new(gpu.derived_explicit());
        let mut current = request(1);
        current.backend = backend;
        current.max_size = 4;
        current.frame = 10;
        let mut cache = CompositionCache::default();
        let first = compose_display(current.clone(), &mut cache);
        assert!(first.error.is_none(), "{:?}", first.error);
        let uploaded = raw(&cache);
        let viewed = transformed(&cache);

        current.image_rect = current.image_rect.translate(Vec2::new(1.0, 0.0));
        let panned = compose_display(current.clone(), &mut cache);
        assert!(panned.error.is_none(), "{:?}", panned.error);
        assert!(std::sync::Weak::ptr_eq(&uploaded, &raw(&cache)));
        assert!(std::sync::Weak::ptr_eq(&viewed, &transformed(&cache)));
        let mut host = current.clone();
        host.backend = Arc::new(spektrafilm_backend::cpu_backend::CpuBackend);
        let expected = compose_display(host, &mut CompositionCache::default());
        assert_eq!(panned.image.pixels, expected.image.pixels);

        current.metadata.transform = true;
        let response = compose_display(current.clone(), &mut cache);
        assert!(response.error.is_none(), "{:?}", response.error);
        assert!(std::sync::Weak::ptr_eq(&uploaded, &raw(&cache)));
        assert!(!std::sync::Weak::ptr_eq(&viewed, &transformed(&cache)));

        current.metadata.encoded = false;
        let interpreted = compose_display(current.clone(), &mut cache);
        assert!(interpreted.error.is_none(), "{:?}", interpreted.error);
        assert!(
            uploaded.upgrade().is_none(),
            "interpretation replaces the upload"
        );
        let old = raw(&cache);
        current.source = Some(Arc::new((**current.source.as_ref().unwrap()).clone()));
        let replaced = compose_display(current.clone(), &mut cache);
        assert!(replaced.error.is_none(), "{:?}", replaced.error);
        assert!(
            old.upgrade().is_none(),
            "source replacement releases old device images"
        );
        assert_eq!(interpreted.image.pixels, replaced.image.pixels);

        let old = raw(&cache);
        current.backend = Arc::new(gpu.derived_explicit());
        let rebound = compose_display(current, &mut cache);
        assert!(rebound.error.is_none(), "{:?}", rebound.error);
        assert!(
            old.upgrade().is_none(),
            "backend replacement invalidates the upload"
        );
        assert_eq!(replaced.image.pixels, rebound.image.pixels);
    }

    #[test]
    fn original_full_source_upload_is_shared_with_analysis() {
        let gpu = spektrafilm_gpu::test_support::explicit_adapter();
        let mut current = request(1);
        current.layer = ViewLayer::Input;
        current.backend = Arc::new(gpu.derived_explicit());
        current.max_size = 4;
        let mut cache = CompositionCache::default();
        let response = compose_display(current, &mut cache);
        assert!(response.error.is_none(), "{:?}", response.error);
        let shared = response.uploaded_input.unwrap();
        let cached = &cache.device_source[0].as_ref().unwrap().image;
        assert!(Arc::ptr_eq(
            shared.payload().unwrap(),
            cached.payload().unwrap()
        ));
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
        let seed = output_display_raster_with(
            &spektrafilm_backend::cpu_backend::CpuBackend,
            fit.source.as_ref().unwrap(),
            "sRGB",
            true,
            false,
            None,
            4,
        )
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

    /// The magnified full-source view must compose on the device exactly as
    /// the host composition does: the same viewport geometry, the same
    /// separable interpolation kernel, the same display transform. A device
    /// port that changed any of the three would change what the magnified
    /// viewer shows.
    ///
    /// The comparison is against the host reference over a chromatic source
    /// and every interpolation mode, because the kernels differ in radius and
    /// in weight and an error in either shows up only at the edges.
    #[test]
    fn device_full_source_matches_the_host_reference() {
        use spektrafilm_math::colorspace::resolve;
        use spektrafilm_math::precision::from_f64;

        let backend = spektrafilm_gpu::test_support::adapter();
        let (w, h) = (37u32, 23u32);
        // A chromatic, out-of-range source: a smooth ramp would hide a wrong
        // tap or a wrong clamp behind the average.
        let image = std::sync::Arc::new(ImageBuf::from_data(
            w,
            h,
            (0..w * h)
                .flat_map(|i| {
                    let x = (i % w) as f64 / (w - 1) as f64;
                    let y = (i / w) as f64 / (h - 1) as f64;
                    let encode = |v: f64| from_f64(1.4 * v - 0.2);
                    [encode(x), encode(y * 0.8 + 0.1), encode(x * y + 0.05)]
                })
                .collect(),
        ));
        let space = resolve("Display P3").unwrap();
        let metadata = DisplaySource {
            space: Some("Display P3".into()),
            encoded: true,
            transform: true,
            profile: None,
        };
        let modes = [
            Interpolation::Nearest,
            Interpolation::Linear,
            Interpolation::Cubic,
            Interpolation::Spline16,
            Interpolation::Spline36,
            Interpolation::Lanczos,
            Interpolation::Blackman,
        ];
        for interpolation in modes {
            // A viewport that fits the source with a fractional offset, so the
            // kernel's taps land off the pixel centres.
            let image_rect = Rect::from_min_size(Pos2::new(3.0, 2.0), Vec2::new(31.0, 17.0));
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(64.0, 48.0));
            let texture_size = [64usize, 48usize];
            let device = metadata
                .full_source_view_with(
                    backend.backend(),
                    &image,
                    ViewLayer::Output,
                    image_rect,
                    rect,
                    texture_size,
                    interpolation,
                    &mut TransformCache::default(),
                )
                .expect("the device composition provides the magnified view");
            let host = metadata
                .full_source_view(
                    &image,
                    ViewLayer::Output,
                    image_rect,
                    rect,
                    texture_size,
                    interpolation,
                    &mut TransformCache::default(),
                )
                .expect("the host reference provides the magnified view");
            assert_eq!(device.0.size, host.0.size);
            let mut worst = 0.0f64;
            let mut compared = 0usize;
            for (got, want) in device.0.rgb.iter().zip(host.0.rgb.iter()) {
                for c in 0..3 {
                    worst = worst.max((f64::from(got[c]) - f64::from(want[c])).abs());
                }
                compared += 1;
            }
            assert_eq!(compared, texture_size[0] * texture_size[1]);
            // f32 evaluation of the host's f64 display transform, plus the
            // interpolation weights the host evaluates in f32 already. The
            // kernels are f32 in both, so the budget is the display
            // transform's precision, not the kernel's.
            assert!(
                worst < 2e-6,
                "{:?}: the device full-source composition differs from the host reference by {worst:.3e}",
                interpolation
            );
            // The source image is read-only.
            let _ = space;
        }
    }

    /// The magnified view must leave the retained source unchanged: it reads
    /// the source and writes only the disposable viewport raster.
    #[test]
    fn device_full_source_never_writes_the_source_image() {
        use spektrafilm_math::precision::from_f64;

        let backend = spektrafilm_gpu::test_support::adapter();
        let image = std::sync::Arc::new(ImageBuf::from_data(
            8,
            4,
            (0..8 * 4)
                .flat_map(|i| {
                    let v = f64::from(i) / f64::from(8 * 4 - 1);
                    [from_f64(v), from_f64(1.0 - v), from_f64(v * 0.5)]
                })
                .collect(),
        ));
        let retained = image.data.clone();
        let metadata = DisplaySource {
            space: Some("sRGB".into()),
            encoded: true,
            transform: true,
            profile: None,
        };
        let image_rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(8.0, 4.0));
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(16.0, 8.0));
        let (raster, _) = metadata
            .full_source_view_with(
                backend.backend(),
                &image,
                ViewLayer::Output,
                image_rect,
                rect,
                [16, 8],
                Interpolation::Spline36,
                &mut TransformCache::default(),
            )
            .expect("the device composition provides the magnified view");
        assert_eq!(raster.size, [16, 8]);
        assert_eq!(image.data, retained);
    }
    #[test]
    fn resident_composition_preserves_samples_and_crossfades_or_reports_failure() {
        let device = spektrafilm_gpu::test_support::explicit_adapter();
        let backend: Arc<dyn ComputeBackend> = Arc::new(device.derived_explicit());
        let mut request = request(1);
        request.full_source = false;
        request.max_size = 4;
        request.frame = 5;
        request.backend = backend.clone();
        let original = request.source.as_ref().unwrap().clone();
        let saved = original.data.clone();
        let meta = spektrafilm_backend::RetainedImageMeta {
            width: original.width,
            height: original.height,
            domain: spektrafilm_backend::SampleDomain::Encoded,
            precision: spektrafilm_backend::Precision::host(),
            space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
            pixel_pitch_um: None,
        };
        let retained = backend.retain_image(&original, &meta).unwrap();
        request.retained_source = Some(retained.clone());
        let old = ImageBuf::from_data(4, 1, vec![0.0 as _; 12]);
        request.previous = Some(
            super::super::colortransform::output_display_raster_with(
                backend.as_ref(),
                &old,
                "sRGB",
                true,
                false,
                None,
                4,
            )
            .unwrap()
            .0,
        );
        let mut cache = CompositionCache::default();
        let composed = compose_display(request.clone(), &mut cache);
        assert!(composed.error.is_none(), "{:?}", composed.error);
        assert_eq!(
            composed.image.pixels,
            vec![
                Color32::BLACK,
                Color32::from_gray(128),
                Color32::BLACK,
                Color32::from_gray(128)
            ]
        );
        assert_eq!(original.data, saved);
        assert_eq!(retained.to_host(backend.as_ref()).unwrap().data, saved);
        request.generation = 2;
        request.frame = 10;
        let final_frame = compose_display(request.clone(), &mut cache);
        assert_eq!(
            final_frame.image.pixels,
            vec![
                Color32::BLACK,
                Color32::WHITE,
                Color32::BLACK,
                Color32::WHITE
            ]
        );
        // A host carrier cannot be reinterpreted as this device's storage.
        request.retained_source = Some(spektrafilm_backend::DeviceImage::host(
            (*original).clone(),
            meta,
        ));
        request.max_size = 3;
        let failed = compose_display(request, &mut cache);
        assert!(failed.error.is_some());
        assert!(
            failed.image.pixels.is_empty(),
            "failure must not publish substitute pixels"
        );
        assert_eq!(retained.to_host(backend.as_ref()).unwrap().data, saved);
    }
    #[test]
    fn missing_device_output_matches_empty_cpu_canvas_and_reuses_healthy_fit() {
        let device = spektrafilm_gpu::test_support::explicit_adapter();
        let backend: Arc<dyn ComputeBackend> = Arc::new(device.derived_explicit());
        let mut healthy = request(1);
        healthy.backend = backend.clone();
        healthy.raster = None;
        healthy.max_size = 4;
        let mut cache = CompositionCache::default();
        let accepted = compose_display(healthy.clone(), &mut cache);
        assert!(accepted.error.is_none(), "{:?}", accepted.error);
        for full_source in [false, true] {
            let mut absent = healthy.clone();
            absent.generation = 2;
            absent.source = None;
            absent.retained_source = None;
            absent.full_source = full_source;
            absent.polaroid = true;
            // Animation must not resurrect a previous frame without output.
            absent.previous = Some(DisplayRaster::new([1, 1], vec![[0.0; 3]]).unwrap());
            let mut host = absent.clone();
            host.backend = Arc::new(spektrafilm_backend::cpu_backend::CpuBackend);
            let expected = compose_display(host, &mut CompositionCache::default());
            let empty = compose_display(absent, &mut cache);
            assert!(empty.error.is_none(), "{:?}", empty.error);
            assert_eq!(empty.image.size, expected.image.size);
            assert_eq!(empty.image.pixels, expected.image.pixels);
            assert_eq!(empty.image.pixels, vec![Color32::WHITE; 4]);
            assert_ne!(empty.image.pixels, accepted.image.pixels);
            assert!(empty.prepared.is_none());
        }
        let reused = compose_display(healthy.clone(), &mut cache);
        assert!(reused.error.is_none(), "{:?}", reused.error);
        assert_eq!(reused.image.pixels, accepted.image.pixels);
        let fit = cache.fit[1].as_ref().unwrap();
        assert!(
            fit.prepared
                .matches(healthy.source.as_ref().unwrap(), &healthy.metadata, 4)
        );
    }

    #[test]
    fn resident_paper_and_development_keep_the_pinned_viewer_algorithms() {
        let backend = spektrafilm_gpu::test_support::explicit_adapter();
        let (w, h, alpha) = super::super::decorations::logo_alpha();
        // Odd full-frame length exercises PCG low/high words across the two
        // whole-image random draws, rather than interleaved per-pixel draws.
        let paper = backend
            .viewer_paper_retained(65, 33, &alpha, w as u32, h as u32)
            .unwrap();
        let got = paper.to_host(backend.backend()).unwrap();
        let reference = super::super::decorations::virtual_paper_back([65, 33]);
        for (pixel, want) in got.data.as_chunks::<3>().0.iter().zip(reference.rgb.iter()) {
            for c in 0..3 {
                assert!(
                    (pixel[c] as f32 - want[c]).abs() <= 2e-5,
                    "paper pixel {pixel:?} differs from {want:?}"
                );
            }
        }
        let source = ImageBuf::from_data(
            4,
            2,
            (0..24).map(|i| (0.02 + i as f32 * 0.039) as _).collect(),
        );
        let meta = spektrafilm_backend::RetainedImageMeta {
            width: 4,
            height: 2,
            domain: spektrafilm_backend::SampleDomain::Encoded,
            precision: spektrafilm_backend::Precision::host(),
            space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
            pixel_pitch_um: None,
        };
        let retained = backend.retain_image(&source, &meta).unwrap();
        let raster = DisplayRaster::new(
            [4, 2],
            source
                .data
                .as_chunks::<3>()
                .0
                .iter()
                .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
                .collect(),
        )
        .unwrap();
        let state = PolaroidState::new(&raster);
        for t in [0.0, 0.5, 1.0] {
            let developed = backend
                .viewer_polaroid_retained(
                    &retained,
                    super::super::decorations::layer_coefficients(t),
                )
                .unwrap()
                .to_host(backend.backend())
                .unwrap();
            let reference = state.frame(t);
            for (pixel, want) in developed
                .data
                .as_chunks::<3>()
                .0
                .iter()
                .zip(reference.rgb.iter())
            {
                for c in 0..3 {
                    assert!(
                        (pixel[c] as f32 - want[c]).abs() <= 2e-5,
                        "development t={t}, channel={c}: {} != {}",
                        pixel[c],
                        want[c]
                    );
                }
            }
        }
        assert_eq!(
            retained.to_host(backend.backend()).unwrap().data,
            source.data
        );
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn viewer_supersession_cancels_active_gpu_work_and_accepts_healthy_pixels() {
        let gpu = spektrafilm_gpu::test_support::explicit_adapter();
        let backend: Arc<dyn ComputeBackend> = Arc::new(gpu.derived_explicit());
        let (started, ready) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let (finished, results) = std::sync::mpsc::channel();
        let worker = DisplayWorker::start(Some(Arc::new(CompositionPause {
            started,
            release: Mutex::new(released),
            finished,
        })));
        let mut first = super::tests::request(1);
        first.backend = backend.clone();
        first.max_size = 4;
        first.frame = 10;
        let (repaint, repainted) = std::sync::mpsc::channel();
        first.ctx.set_request_repaint_callback(move |_| {
            let _ = repaint.send(());
        });
        worker.submit(first.clone());
        ready
            .recv_timeout(Duration::from_secs(30))
            .expect("first GPU fit completed");
        let cancellation = worker.state.0.lock().active_cancel.clone().unwrap();
        assert!(!cancellation.load(Ordering::Relaxed));
        first.generation = 2;
        first.key = vec![2];
        worker.submit(first);
        let cancelled = cancellation.load(Ordering::Relaxed);
        release.send(()).unwrap();
        assert!(
            cancelled,
            "supersession must cancel active work, not just reject its response"
        );
        assert_eq!(
            results.recv_timeout(Duration::from_secs(30)).unwrap(),
            (1, Some("operation cancelled".into()))
        );
        repainted
            .recv_timeout(Duration::from_secs(30))
            .expect("replacement viewer completed");
        let response = worker.take_response().expect("replacement presentation");
        assert!(response.matches(2, &[2]));
        assert!(response.error.is_none(), "{:?}", response.error);
        assert_eq!(
            response.image.pixels,
            vec![
                Color32::BLACK,
                Color32::WHITE,
                Color32::BLACK,
                Color32::WHITE
            ]
        );
        drop(worker);
        let healthy = backend
            .retain_image(
                &ImageBuf::from_data(1, 1, vec![0.5 as _; 3]),
                &spektrafilm_backend::RetainedImageMeta {
                    width: 1,
                    height: 1,
                    domain: spektrafilm_backend::SampleDomain::Encoded,
                    precision: spektrafilm_backend::Precision::host(),
                    space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
                    pixel_pitch_um: None,
                },
            )
            .unwrap();
        assert_eq!(
            backend
                .viewer_fit_retained(&healthy, 1, 1, None, None)
                .unwrap()
                .to_host(backend.as_ref())
                .unwrap()
                .data,
            vec![spektrafilm_math::precision::from_f64(0.5); 3]
        );
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn viewer_drop_cancels_active_gpu_composition_before_joining() {
        let gpu = spektrafilm_gpu::test_support::explicit_adapter();
        let backend: Arc<dyn ComputeBackend> = Arc::new(gpu.derived_explicit());
        let (started, ready) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let (finished, results) = std::sync::mpsc::channel();
        let worker = DisplayWorker::start(Some(Arc::new(CompositionPause {
            started,
            release: Mutex::new(released),
            finished,
        })));
        let mut request = super::tests::request(1);
        request.backend = backend;
        request.max_size = 4;
        worker.submit(request);
        ready
            .recv_timeout(Duration::from_secs(30))
            .expect("GPU fit completed before shutdown");
        let state = worker.state.clone();
        let cancellation = state.0.lock().active_cancel.clone().unwrap();
        std::thread::scope(|scope| {
            let dropped = scope.spawn(move || drop(worker));
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut locked = state.0.lock();
            while !locked.stopping {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .expect("drop must signal worker shutdown");
                state.1.wait_for(&mut locked, remaining);
            }
            let cancelled = cancellation.load(Ordering::Relaxed);
            drop(locked);
            release.send(()).unwrap();
            assert!(cancelled, "drop must cancel before joining");
            assert_eq!(
                results.recv_timeout(Duration::from_secs(30)).unwrap(),
                (1, Some("operation cancelled".into()))
            );
            dropped.join().unwrap();
        });
    }
}

#[cfg(test)]
mod inspection_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn image(backend: &dyn ComputeBackend, value: f64) -> spektrafilm_backend::DeviceImage {
        backend
            .retain_image(
                &ImageBuf::from_data(
                    2,
                    1,
                    vec![
                        value as _, 1.5 as _, -0.25 as _, 0.75 as _, 0.5 as _, value as _,
                    ],
                ),
                &spektrafilm_backend::RetainedImageMeta {
                    width: 2,
                    height: 1,
                    domain: spektrafilm_backend::SampleDomain::Linear,
                    precision: spektrafilm_backend::Precision::host(),
                    space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
                    pixel_pitch_um: None,
                },
            )
            .unwrap()
    }

    #[test]
    fn inspection_worker_coalesces_latest_source_and_coordinate_without_cancelling_composition() {
        let gpu = spektrafilm_gpu::test_support::explicit_adapter();
        let backend: Arc<dyn ComputeBackend> = Arc::new(gpu.derived_explicit());
        let original = image(backend.as_ref(), 0.0);
        let replacement = image(backend.as_ref(), 1.0);
        let worker = DisplayWorker::new();
        let (started, ready) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        worker.state.0.lock().inspection_pause = Some(InspectionPause {
            started,
            release: Mutex::new(released),
        });
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |_| {});
        }
        let (repaint, repainted) = std::sync::mpsc::channel();
        ctx.set_request_repaint_callback(move |_| {
            let _ = repaint.send(());
        });
        worker.inspect(Some(InspectionRequest {
            generation: 1,
            image: original,
            x: 0,
            y: 0,
            backend: backend.clone(),
            ctx: ctx.clone(),
        }));
        ready
            .recv_timeout(Duration::from_secs(30))
            .expect("inspection started off thread");
        let cancellation = worker
            .state
            .0
            .lock()
            .active_inspection_cancel
            .clone()
            .unwrap();
        worker.submit(super::tests::request(99));
        for generation in 2..=21 {
            worker.inspect(Some(InspectionRequest {
                generation,
                image: replacement.clone(),
                x: (generation % 2) as u32,
                y: 0,
                backend: backend.clone(),
                ctx: ctx.clone(),
            }));
        }
        let state = worker.state.0.lock();
        let pending = state.inspection_request.as_ref().unwrap();
        let latest = pending.generation == 21
            && pending.x == 1
            && Arc::ptr_eq(
                pending.image.payload().unwrap(),
                replacement.payload().unwrap(),
            );
        let composition_pending = state.request.as_ref().is_some_and(|r| r.generation == 99);
        let cancelled = cancellation.load(Ordering::Relaxed);
        drop(state);
        release.send(()).unwrap();
        assert!(latest, "only the latest hover may remain pending");
        assert!(composition_pending, "hover must preserve composition work");
        assert!(cancelled, "movement must cancel active readback work");
        repainted
            .recv_timeout(Duration::from_secs(30))
            .expect("latest hover repaint");
        let response = worker.take_inspection().expect("latest hover published");
        assert_eq!(response.generation, 21);
        assert_eq!(response.rgb, Some([0.75, 0.5, 1.0]));
        let composition = worker
            .take_response()
            .expect("composition was not superseded by hover");
        assert_eq!(composition.generation, 99);
        assert!(composition.error.is_none());
        worker.inspect(None);
        assert!(worker.take_inspection().is_none());
    }

    #[test]
    fn viewer_drop_cancels_active_inspection_before_joining() {
        let gpu = spektrafilm_gpu::test_support::explicit_adapter();
        let backend: Arc<dyn ComputeBackend> = Arc::new(gpu.derived_explicit());
        let retained = image(backend.as_ref(), 0.5);
        let worker = DisplayWorker::new();
        let (started, ready) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        worker.state.0.lock().inspection_pause = Some(InspectionPause {
            started,
            release: Mutex::new(released),
        });
        worker.inspect(Some(InspectionRequest {
            generation: 1,
            image: retained.clone(),
            x: 0,
            y: 0,
            backend: backend.clone(),
            ctx: egui::Context::default(),
        }));
        ready
            .recv_timeout(Duration::from_secs(30))
            .expect("inspection started");
        let state = worker.state.clone();
        let cancellation = state.0.lock().active_inspection_cancel.clone().unwrap();
        std::thread::scope(|scope| {
            let dropped = scope.spawn(move || drop(worker));
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut locked = state.0.lock();
            while !locked.stopping {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .expect("drop must signal inspection shutdown");
                state.1.wait_for(&mut locked, remaining);
            }
            let cancelled = cancellation.load(Ordering::Relaxed);
            drop(locked);
            release.send(()).unwrap();
            assert!(cancelled, "drop must cancel inspection before joining");
            dropped.join().unwrap();
        });
        assert!(state.0.lock().inspection_response.is_none());
        assert_eq!(
            backend.inspect_retained_pixel(&retained, 0, 0).unwrap(),
            [0.5, 1.5, -0.25],
            "shutdown cancellation must not poison the shared backend",
        );
    }
}
