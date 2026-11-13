//! Isolated current-photograph candidate renders for the preset browser.
//!
//! This owner never touches accepted controls, history, retained output, or the
//! main render worker. One thread renders the visible demand set with the
//! production resolver, runtime, and display transform.
use super::errors::document_error_message;
use super::*;
use crate::render::{InputImage, RenderSnapshot, RuntimeCache};
use eframe::egui;
use spektrafilm_backend::telemetry::{CollectionMode, ObservationContext};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};

/// Candidate render identity: the accepted context signature, preset identity,
/// and the validated document contents.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ThumbnailKey {
    pub(crate) generation: u64,
    pub(crate) identity: String,
}

impl ThumbnailKey {
    pub(crate) fn new(generation: u64, group: PresetGroup, name: &str, content: u64) -> Self {
        Self {
            generation,
            identity: format!("{group:?}/{name}/{content:016x}"),
        }
    }
}

#[derive(Clone)]
pub(crate) struct ThumbnailContext {
    pub(crate) source: InputImage,
    pub(crate) params: RuntimeParams,
    pub(crate) film_name: String,
    pub(crate) print_name: String,
    pub(crate) data_dir: PathBuf,
    pub(crate) special: serde_json::Value,
    pub(crate) backend: Arc<dyn spektrafilm_backend::ComputeBackend>,
    pub(crate) display_enabled: bool,
    pub(crate) display_profile: Option<PathBuf>,
    /// Accepted presentation quarter turns, applied before presentation.
    pub(crate) rotation_quarter_turns: u32,
    /// Preset-library generation; a change drops prepared runtime state so
    /// referenced documents are re-read.
    pub(crate) cache_epoch: u64,
    pub(crate) size: u32,
}

pub(crate) struct Candidate {
    pub(crate) key: ThumbnailKey,
    pub(crate) preset: Preset,
}

struct Batch {
    context: ThumbnailContext,
    candidates: Vec<Candidate>,
    cancel: Arc<AtomicBool>,
    /// Scheduler request identity; results from an invalidated context are
    /// dropped instead of filling freshly cleared caches.
    request: u64,
}

/// One full-resolution host upload reused by every candidate in the accepted
/// context. Candidates must meter the full source like the main GPU preview,
/// not a preview-sized copy.
struct RetainedHost {
    source: std::sync::Weak<spektrafilm_math::image::ImageBuf>,
    backend: std::sync::Weak<dyn spektrafilm_backend::ComputeBackend>,
    device: Option<u64>,
    image: spektrafilm_backend::DeviceImage,
}

pub(crate) enum Outcome {
    Ready(egui::ColorImage),
    Failed(crate::app::Message),
}

pub(crate) struct ThumbnailResult {
    pub(crate) request: u64,
    pub(crate) key: ThumbnailKey,
    pub(crate) outcome: Outcome,
}

pub(crate) struct ThumbnailWorker {
    state: Arc<(Mutex<Option<Batch>>, Condvar)>,
    active_cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    foreground: Arc<AtomicBool>,
    rx: mpsc::Receiver<ThumbnailResult>,
    shutdown: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ThumbnailWorker {
    pub(crate) fn new() -> Self {
        let state: Arc<(Mutex<Option<Batch>>, Condvar)> =
            Arc::new((Mutex::new(None), Condvar::new()));
        let active_cancel: Arc<Mutex<Option<Arc<AtomicBool>>>> = Arc::new(Mutex::new(None));
        let foreground = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let worker_state = state.clone();
        let worker_cancel = active_cancel.clone();
        let worker_foreground = foreground.clone();
        let worker_shutdown = shutdown.clone();
        let handle = std::thread::Builder::new()
            .name("spektrafilm-preset-thumbnails".into())
            .spawn(move || {
                let mut cache = RuntimeCache::default();
                let mut upload: Option<RetainedHost> = None;
                let mut cache_epoch = None;
                loop {
                    let batch = {
                        let (lock, wake) = &*worker_state;
                        let mut queued = lock_mutex(lock);
                        while queued.is_none() && !worker_shutdown.load(Ordering::SeqCst) {
                            // Cancellation while idle must release the full
                            // source upload even though no batch is running.
                            if worker_foreground.load(Ordering::SeqCst) {
                                upload = None;
                            } else if let Some(token) = lock_mutex(&worker_cancel).as_ref()
                                && token.load(Ordering::SeqCst)
                            {
                                upload = None;
                            }
                            queued = wake
                                .wait(queued)
                                .unwrap_or_else(|poison| poison.into_inner());
                        }
                        if worker_shutdown.load(Ordering::SeqCst) {
                            return;
                        }
                        let batch = queued.take().expect("thumbnail batch signaled");
                        if !host_source_matches(upload.as_ref(), &batch.context.source) {
                            upload = None;
                        }
                        // Register the token while the queue lock is still
                        // held, so a concurrent cancel cannot observe neither
                        // a queued batch nor an active one.
                        *lock_mutex(&worker_cancel) = Some(batch.cancel.clone());
                        batch
                    };
                    if cache_epoch != Some(batch.context.cache_epoch) {
                        cache = RuntimeCache::default();
                        cache_epoch = Some(batch.context.cache_epoch);
                    }
                    for candidate in batch.candidates {
                        if worker_shutdown.load(Ordering::SeqCst)
                            || batch.cancel.load(Ordering::SeqCst)
                            || worker_foreground.load(Ordering::SeqCst)
                        {
                            upload = None;
                            break;
                        }
                        let outcome =
                            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                render_candidate(
                                    &mut cache,
                                    &mut upload,
                                    &batch.context,
                                    &candidate,
                                    &batch.cancel,
                                )
                            })) {
                                Ok(Ok(image)) => Outcome::Ready(image),
                                Ok(Err(message)) => {
                                    if cancelled(
                                        &batch.cancel,
                                        &worker_foreground,
                                        &worker_shutdown,
                                    ) {
                                        upload = None;
                                        break;
                                    }
                                    Outcome::Failed(message)
                                }
                                Err(_) => {
                                    if cancelled(
                                        &batch.cancel,
                                        &worker_foreground,
                                        &worker_shutdown,
                                    ) {
                                        upload = None;
                                        break;
                                    }
                                    Outcome::Failed(crate::app::Message::new(
                                        "surfaces.presets.thumbnail_unavailable",
                                    ))
                                }
                            };
                        if tx
                            .send(ThumbnailResult {
                                request: batch.request,
                                key: candidate.key,
                                outcome,
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            })
            .expect("preset thumbnail thread spawn");
        Self {
            state,
            active_cancel,
            foreground,
            rx,
            shutdown,
            handle: Some(handle),
        }
    }

    pub(crate) fn set_foreground(&self, busy: bool) {
        self.foreground.store(busy, Ordering::SeqCst);
    }

    pub(crate) fn cancel(&self) {
        // State lock first everywhere: the worker registers its active token
        // while holding it, so cancel can never miss both queues.
        let (lock, wake) = &*self.state;
        let queued = lock_mutex(lock);
        if let Some(batch) = queued.as_ref() {
            batch.cancel.store(true, Ordering::SeqCst);
        }
        if let Some(token) = lock_mutex(&self.active_cancel).as_ref() {
            token.store(true, Ordering::SeqCst);
        }
        drop(queued);
        wake.notify_one();
    }

    pub(crate) fn submit(
        &mut self,
        context: ThumbnailContext,
        candidates: Vec<Candidate>,
        request: u64,
    ) {
        let cancel = Arc::new(AtomicBool::new(false));
        let (lock, wake) = &*self.state;
        let mut queued = lock_mutex(lock);
        if let Some(token) = lock_mutex(&self.active_cancel).as_ref() {
            token.store(true, Ordering::SeqCst);
        }
        queued.replace(Batch {
            context,
            candidates,
            cancel,
            request,
        });
        drop(queued);
        wake.notify_one();
    }

    pub(crate) fn try_recv(&self) -> Option<ThumbnailResult> {
        self.rx.try_recv().ok()
    }

    pub(crate) fn shutdown(&mut self) {
        self.cancel();
        self.shutdown.store(true, Ordering::SeqCst);
        {
            let (lock, wake) = &*self.state;
            let _queued = lock_mutex(lock);
            wake.notify_one();
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ThumbnailWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn lock_mutex<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn cancelled(
    cancel: &Arc<AtomicBool>,
    foreground: &Arc<AtomicBool>,
    shutdown: &Arc<AtomicBool>,
) -> bool {
    cancel.load(Ordering::SeqCst)
        || foreground.load(Ordering::SeqCst)
        || shutdown.load(Ordering::SeqCst)
}

fn render_candidate(
    cache: &mut RuntimeCache,
    upload: &mut Option<RetainedHost>,
    context: &ThumbnailContext,
    candidate: &Candidate,
    cancel: &Arc<AtomicBool>,
) -> Result<egui::ColorImage, crate::app::Message> {
    let lut_candidate = candidate.preset.lut.is_some();
    let effects_candidate = candidate.preset.effects_parameters().is_some();
    let (params, film_name, print_name) = if lut_candidate {
        let resolved = candidate
            .preset
            .resolve_lut_detailed(&context.params)
            .map_err(|error| document_error_message(&error))?;
        (
            resolved.params,
            context.film_name.clone(),
            context.print_name.clone(),
        )
    } else if effects_candidate {
        let resolved = candidate
            .preset
            .resolve_effects_detailed(&context.params)
            .map_err(|error| document_error_message(&error))?;
        (
            resolved.params,
            context.film_name.clone(),
            context.print_name.clone(),
        )
    } else {
        let mut resolved = candidate
            .preset
            .resolve_detailed(&context.data_dir, &context.params)
            .map_err(|error| document_error_message(&error))?;
        // Same post-resolve normalization as an accepted look application.
        resolved.params.film_render.grain.monochrome = resolved.film.is_bw();
        resolved.params.workflow.mode = spektrafilm_core::params::WorkflowMode::Film;
        (resolved.params, resolved.film_name, resolved.print_name)
    };
    let snapshot = RenderSnapshot::new(
        params,
        film_name,
        print_name,
        context.data_dir.clone(),
        context.special.clone(),
    )
    .with_cancellation(Some(cancel.clone()));
    let observation = ObservationContext::new(CollectionMode::Off);
    let backend: &dyn spektrafilm_backend::ComputeBackend = context.backend.as_ref();
    let runtime = cache.prepare_observed_with_backend(&snapshot, true, &observation, backend)?;
    runtime.set_cancellation(Some(cancel.clone()));
    let params = runtime.params().clone();
    let cancellable = backend.with_cancellation(Some(cancel.clone()));
    let backend = cancellable.as_deref().unwrap_or(backend);
    let simulation_error = |detail: String| {
        crate::app::Message::new("operations.simulation_error").arg("detail", detail)
    };
    let source = prepared_source(upload, context, &params, backend)
        .and_then(|source| {
            source.interpreted(&params.io.input_color_space, params.io.input_cctf_decoding)
        })
        .map_err(simulation_error)?;
    let output = match &source {
        InputImage::Device(image) => {
            let retained = runtime
                .process_device_retained(
                    image,
                    spektrafilm_core::pipeline::RetainedRender {
                        rotation_quarter_turns: context.rotation_quarter_turns,
                        preview_max_size: Some(context.size),
                    },
                    backend,
                )
                .map_err(simulation_error)?;
            backend
                .materialize_retained(&retained)
                .map_err(simulation_error)?
        }
        InputImage::Host(image) => {
            let oriented = if context.rotation_quarter_turns == 0 {
                (**image).clone()
            } else {
                image.rotated_quarter_turns(context.rotation_quarter_turns as i32)
            };
            if runtime.params().requires_full_frame() {
                let full = runtime
                    .process(oriented, backend)
                    .map_err(simulation_error)?;
                spektrafilm_core::params_builder::resize_for_preview(&full, context.size)
            } else {
                // Match the accepted CPU preview's metering input: the
                // production preview resizes once to its own preview size,
                // then the meter samples that image. Downscaling to the
                // thumbnail size first would meter a different image.
                let render_size = context.size.max(runtime.params().settings.preview_max_size);
                let preview =
                    spektrafilm_core::params_builder::resize_for_preview(&oriented, render_size);
                let full = runtime
                    .process(preview, backend)
                    .map_err(simulation_error)?;
                spektrafilm_core::params_builder::resize_for_preview(&full, context.size)
            }
        }
    };
    let (raster, _status) = crate::display::output_display_raster_with(
        backend,
        &output,
        &params.io.output_color_space,
        params.io.output_cctf_encoding,
        context.display_enabled,
        context.display_profile.as_deref(),
        context.size as usize,
    )
    .map_err(|detail| crate::app::Message::new("operations.display_error").arg("detail", detail))?;
    raster_color_image(backend, &raster)
        .ok_or_else(|| crate::app::Message::new("surfaces.presets.thumbnail_unavailable"))
}

/// The retained upload belongs only to the host source it was created from;
/// a device source or another host image releases it.
fn host_source_matches(upload: Option<&RetainedHost>, source: &InputImage) -> bool {
    let (Some(upload), InputImage::Host(source)) = (upload, source) else {
        return false;
    };
    std::sync::Weak::as_ptr(&upload.source) == Arc::as_ptr(source).cast()
}

/// Upload a host source once for this accepted context, so GPU candidates meter
/// the full-resolution photograph exactly like the main retained preview.
fn prepared_source(
    upload: &mut Option<RetainedHost>,
    context: &ThumbnailContext,
    params: &RuntimeParams,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<InputImage, String> {
    let InputImage::Host(source) = &context.source else {
        return Ok(context.source.clone());
    };
    if !backend.is_gpu() {
        return Ok(context.source.clone());
    }
    let space = spektrafilm_math::colorspace::resolve(&params.io.input_color_space)?;
    let domain = if params.io.input_cctf_decoding {
        spektrafilm_backend::SampleDomain::Encoded
    } else {
        spektrafilm_backend::SampleDomain::Linear
    };
    let device = backend.preparation_device_id();
    let reusable = upload.as_ref().is_some_and(|cached| {
        std::sync::Weak::as_ptr(&cached.source) == Arc::as_ptr(source).cast()
            && std::ptr::addr_eq(
                std::sync::Weak::as_ptr(&cached.backend),
                Arc::as_ptr(&context.backend),
            )
            && cached.device == device
            && cached.image.space().name == space.name
            && cached.image.domain() == domain
    });
    if !reusable {
        *upload = None;
        let image = backend.retain_image(
            source,
            &spektrafilm_backend::RetainedImageMeta {
                width: source.width,
                height: source.height,
                domain,
                precision: spektrafilm_backend::Precision::host(),
                space,
                pixel_pitch_um: None,
            },
        )?;
        *upload = Some(RetainedHost {
            source: Arc::downgrade(source),
            backend: Arc::downgrade(&context.backend),
            device,
            image,
        });
    }
    Ok(InputImage::Device(
        upload
            .as_ref()
            .expect("retained host upload installed")
            .image
            .clone(),
    ))
}

fn raster_color_image(
    backend: &dyn spektrafilm_backend::ComputeBackend,
    raster: &crate::display::DisplayRaster,
) -> Option<egui::ColorImage> {
    if let Some(retained) = raster.retained.as_ref() {
        let host = backend.materialize_retained(retained).ok()?;
        return image_buffer_color_image(&host);
    }
    let [width, height] = raster.size;
    if width == 0 || height == 0 || raster.rgb.len() != width * height {
        return None;
    }
    Some(egui::ColorImage {
        size: [width, height],
        pixels: raster.rgb.iter().map(|pixel| encode(*pixel)).collect(),
    })
}

fn image_buffer_color_image(image: &spektrafilm_math::image::ImageBuf) -> Option<egui::ColorImage> {
    let width = image.width as usize;
    let height = image.height as usize;
    if width == 0 || height == 0 || image.data.len() != width * height * 3 {
        return None;
    }
    let (pixels, _) = image.data.as_chunks::<3>();
    let pixels = pixels
        .iter()
        .map(|pixel| encode([pixel[0] as f32, pixel[1] as f32, pixel[2] as f32]))
        .collect();
    Some(egui::ColorImage {
        size: [width, height],
        pixels,
    })
}

fn encode(pixel: [f32; 3]) -> egui::Color32 {
    let quantize = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgb(quantize(pixel[0]), quantize(pixel[1]), quantize(pixel[2]))
}
