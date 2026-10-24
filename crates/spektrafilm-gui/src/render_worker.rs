use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use eframe::egui;
use parking_lot::{Condvar, Mutex};
use spektrafilm_backend::ComputeBackend;
use spektrafilm_backend::telemetry::{CollectionMode, ObservationKind, ObservationScope, Purpose};
use spektrafilm_core::image_io::ImageMetadata;
use spektrafilm_core::telemetry::{IssueBoundary, IssueCategory};
use spektrafilm_core::telemetry::{Operation, Outcome};
use spektrafilm_math::image::ImageBuf;

use super::display;
use super::render::{RenderSnapshot, RuntimeCache};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RenderKind {
    Preview,
    Scan,
}

impl RenderKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Preview => "Preview",
            Self::Scan => "Scan",
        }
    }

    pub(super) fn message(self) -> crate::app::Message {
        crate::app::Message::new(match self {
            Self::Preview => "operations.preview",
            Self::Scan => "operations.scan",
        })
    }
}

#[derive(Clone)]
pub(super) struct RenderJob {
    pub(super) input_epoch: u64,
    pub(super) parameter_revision: u64,
    pub(super) request_id: u64,
    pub(super) kind: RenderKind,
    pub(super) animate: bool,
    /// Counter-clockwise presentation orientation; the source stays immutable.
    pub(super) rotation_quarter_turns: u32,
    pub(super) collection_mode: CollectionMode,
}

impl RenderJob {
    pub(super) fn matches(
        &self,
        latest: Option<&RenderJob>,
        input_epoch: u64,
        parameter_revision: u64,
    ) -> bool {
        latest.is_some_and(|job| job.request_id == self.request_id)
            && self.input_epoch == input_epoch
            && self.parameter_revision == parameter_revision
    }
}

pub(super) struct RenderRequest {
    pub(super) job: RenderJob,
    pub(super) image: super::render::InputImage,
    pub(super) snapshot: RenderSnapshot,
    pub(super) backend: Arc<dyn ComputeBackend>,
    pub(super) source_metadata: Option<ImageMetadata>,
    pub(super) display_enabled: bool,
    pub(super) display_profile: Option<std::path::PathBuf>,
    pub(super) ctx: egui::Context,
    pub(super) operation: Operation,
    pub(super) worker_wait: ObservationScope,
}

pub(super) struct RenderWorker {
    pub(super) state: Arc<(Mutex<Option<RenderRequest>>, Condvar)>,
    pub(super) active: Arc<Mutex<Option<(u64, RenderKind, CollectionMode)>>>,
    pub(super) rx: mpsc::Receiver<(
        RenderJob,
        Operation,
        Result<RenderResult, crate::app::Message>,
    )>,
    pub(super) shutdown: Arc<AtomicBool>,
    pub(super) handle: Option<JoinHandle<()>>,
    pub(super) next_request_id: u64,
    pub(super) active_cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    invalidate_cache: Arc<AtomicBool>,
}

pub(super) struct RenderResult {
    pub(super) output: Option<ImageBuf>,
    pub(super) retained: Option<spektrafilm_backend::DeviceImage>,
    pub(super) histogram: Option<crate::histogram::HistogramData>,
    pub(super) preview: display::DisplayRaster,
    pub(super) display_status: crate::app::Message,
    pub(super) display_enabled: bool,
    pub(super) display_profile: Option<std::path::PathBuf>,
    pub(super) display_max_size: u32,
    pub(super) output_color_space: String,
    pub(super) output_cctf_encoding: bool,
    pub(super) source_metadata: Option<ImageMetadata>,
    pub(super) operator_compare: bool,
}

/// One immutable raw upload for the accepted input, before metering or geometry.
/// Weak identities avoid retaining a second photograph or backend owner.
#[derive(Default)]
struct RetainedInputCache {
    entry: Option<RetainedInput>,
}

struct RetainedInput {
    source: std::sync::Weak<ImageBuf>,
    backend: std::sync::Weak<dyn ComputeBackend>,
    device: Option<u64>,
    input_epoch: u64,
    host_precision: spektrafilm_backend::Precision,
    image: spektrafilm_backend::DeviceImage,
}

impl RetainedInputCache {
    fn prepare(
        &mut self,
        input: &super::render::InputImage,
        input_epoch: u64,
        params: &spektrafilm_core::params::RuntimeParams,
        backend_owner: &Arc<dyn ComputeBackend>,
        backend: &dyn ComputeBackend,
    ) -> Result<super::render::InputImage, String> {
        let super::render::InputImage::Host(source) = input else {
            self.entry = None;
            return Ok(input.clone());
        };
        if !backend.is_gpu() {
            self.entry = None;
            return Ok(input.clone());
        }
        let space = spektrafilm_math::colorspace::resolve(&params.io.input_color_space)?;
        let domain = if params.io.input_cctf_decoding {
            spektrafilm_backend::SampleDomain::Encoded
        } else {
            spektrafilm_backend::SampleDomain::Linear
        };
        let precision = spektrafilm_backend::Precision::host();
        let device = backend.preparation_device_id();
        let reusable = self.entry.as_ref().is_some_and(|cached| {
            cached.source.as_ptr() == Arc::as_ptr(source)
                && std::ptr::addr_eq(cached.backend.as_ptr(), Arc::as_ptr(backend_owner))
                && cached.device == device
                && cached.input_epoch == input_epoch
                && cached.host_precision == precision
                && cached.image.space().name == space.name
                && cached.image.domain() == domain
        });
        if !reusable {
            self.entry = None;
            let image = backend.retain_image(
                source,
                &spektrafilm_backend::RetainedImageMeta {
                    width: source.width,
                    height: source.height,
                    domain,
                    precision,
                    space,
                    pixel_pitch_um: None,
                },
            )?;
            self.entry = Some(RetainedInput {
                source: Arc::downgrade(source),
                backend: Arc::downgrade(backend_owner),
                device,
                input_epoch,
                host_precision: precision,
                image,
            });
        }
        Ok(super::render::InputImage::Device(
            self.entry
                .as_ref()
                .expect("retained input installed")
                .image
                .clone(),
        ))
    }
}

impl RenderWorker {
    pub(super) fn new() -> Self {
        let state = Arc::new((Mutex::new(None::<RenderRequest>), Condvar::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let active = Arc::new(Mutex::new(None));
        let active_cancel = Arc::new(Mutex::new(None::<Arc<AtomicBool>>));
        let invalidate_cache = Arc::new(AtomicBool::new(false));
        let invalidate_cache_for_worker = invalidate_cache.clone();
        let active_cancel_for_worker = active_cancel.clone();
        let active_for_worker = active.clone();
        let (tx, rx) = mpsc::channel();
        let state_for_worker = state.clone();
        let shutdown_for_worker = shutdown.clone();
        let handle = std::thread::Builder::new()
            .name("spektrafilm-render".into())
            .spawn(move || {
                let mut runtime_cache = RuntimeCache::default();
                let mut scan_cache = spektrafilm_core::runtime::ScanFrameCache::default();
                let mut retained_input = RetainedInputCache::default();
                let mut oriented_host: Option<(u64, u32, Arc<ImageBuf>)> = None;
                loop {
                    let request = {
                        let (lock, wake) = &*state_for_worker;
                        let mut queued = lock.lock();
                        while queued.is_none() && !shutdown_for_worker.load(Ordering::SeqCst) {
                            wake.wait(&mut queued);
                        }
                        if shutdown_for_worker.load(Ordering::SeqCst) {
                            return;
                        }
                        let request = queued.take().expect("render request signaled");
                        *active_cancel_for_worker.lock() = request.snapshot.cancellation().clone();
                        request
                    };
                    drop(request.worker_wait);
                    if invalidate_cache_for_worker.swap(false, Ordering::SeqCst) {
                        runtime_cache = RuntimeCache::default();
                        scan_cache = spektrafilm_core::runtime::ScanFrameCache::default();
                        oriented_host = None;
                        retained_input = RetainedInputCache::default();
                    }
                    let mut operation = request.operation;
                    let attempt = operation.attempt(1);
                    let observation = attempt.context().clone();
                    let job = request.job;
                    *active_for_worker.lock() =
                        Some((job.request_id, job.kind, job.collection_mode));
                    let ctx = request.ctx.clone();
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let prepare_unit = spektrafilm_core::progress::report("input_prepare");
                        let pipeline_template = runtime_cache.prepare_observed_with_backend(
                            &request.snapshot,
                            job.kind == RenderKind::Preview,
                            &observation,
                            request.backend.as_ref(),
                        )?;
                        let pipeline_template = pipeline_template
                            .clone()
                            .with_cancellation(request.snapshot.cancellation().clone());
                        if operation.enabled() {
                            operation.set_render_configuration(
                                pipeline_template.telemetry_configuration(),
                            );
                        }
                        let runtime_params = pipeline_template.params();
                        let rotation = job.rotation_quarter_turns;
                        let input_prepare = observation.scope(
                            "input_prepare",
                            ObservationKind::Phase,
                            Purpose::Image,
                        );
                        // GPU geometry stays on device; the CPU full-frame scanner
                        // receives the oriented source before any preview resize.
                        let image = request
                            .image
                            .interpreted(
                                &runtime_params.io.input_color_space,
                                runtime_params.io.input_cctf_decoding,
                            )
                            .map_err(|error| {
                                crate::app::Message::new("operations.simulation_error")
                                    .arg("detail", error)
                            })?;
                        let host_input = (|| -> Result<Option<Arc<ImageBuf>>, String> {
                            if request.backend.is_gpu() {
                                return Ok(None);
                            }
                            let bound = input_prepare.context().enabled().then(|| {
                                spektrafilm_backend::bind_backend(
                                    request.backend.as_ref(),
                                    input_prepare.context().clone(),
                                )
                            });
                            let backend = bound.as_deref().unwrap_or(request.backend.as_ref());
                            let _span = input_prepare.context().span().map(|span| span.enter());
                            let source = if rotation == 0 {
                                match &image {
                                    super::render::InputImage::Host(image) => image.clone(),
                                    super::render::InputImage::Device(_) => {
                                        Arc::new(image.to_host(backend)?)
                                    }
                                }
                            } else if let Some((epoch, turns, image)) = oriented_host.as_ref()
                                && *epoch == job.input_epoch
                                && *turns == rotation
                            {
                                image.clone()
                            } else {
                                let image = Arc::new(
                                    image
                                        .to_host(backend)?
                                        .rotated_quarter_turns(rotation as i32),
                                );
                                oriented_host = Some((job.input_epoch, rotation, image.clone()));
                                image
                            };
                            Ok(Some(
                                if runtime_params.settings.preview_mode
                                    && !runtime_params.scanner.requires_full_frame()
                                {
                                    Arc::new(spektrafilm_core::params_builder::resize_for_preview(
                                        &source,
                                        runtime_params.settings.preview_max_size,
                                    ))
                                } else {
                                    source
                                },
                            ))
                        })();
                        let host_input = observed_result(
                            input_prepare,
                            host_input,
                            IssueCategory::Input,
                            IssueBoundary::InputPrepare,
                        )
                        .map_err(|error| {
                            crate::app::Message::new("operations.simulation_error")
                                .arg("detail", error)
                        })?;
                        drop(prepare_unit);
                        let simulation =
                            observation.scope("simulation", ObservationKind::Phase, Purpose::Image);
                        let processed = {
                            let _unit = spektrafilm_core::progress::report("simulation");
                            if !runtime_params.scanner.requires_full_frame() {
                                // A cached scan baseline also owns its raw source.
                                // Release it when leaving the full-frame route.
                                scan_cache = spektrafilm_core::runtime::ScanFrameCache::default();
                            }
                            let cancellable = request
                                .backend
                                .with_cancellation(request.snapshot.cancellation());
                            let backend =
                                cancellable.as_deref().unwrap_or(request.backend.as_ref());
                            let bound =
                                (backend.is_gpu() && simulation.context().enabled()).then(|| {
                                    spektrafilm_backend::bind_backend(
                                        backend,
                                        simulation.context().clone(),
                                    )
                                });
                            let backend = bound.as_deref().unwrap_or(backend);
                            let _span = simulation.context().span().map(|span| span.enter());
                            let prepared_input = if request.backend.is_gpu()
                                && request.snapshot.cancellation().is_some()
                                && cancellable.is_none()
                            {
                                Err("selected GPU cannot bind operation cancellation".to_owned())
                            } else {
                                retained_input.prepare(
                                    &image,
                                    job.input_epoch,
                                    runtime_params,
                                    &request.backend,
                                    backend,
                                )
                            };
                            let prepared = prepared_input.and_then(|image| {
                                if request.backend.is_gpu() {
                                    let super::render::InputImage::Device(image) = image else {
                                        unreachable!("GPU input prepared on device");
                                    };
                                    let geometry = spektrafilm_core::pipeline::RetainedRender {
                                        rotation_quarter_turns: rotation,
                                        preview_max_size: runtime_params
                                            .settings
                                            .preview_mode
                                            .then_some(runtime_params.settings.preview_max_size),
                                    };
                                    if runtime_params.scanner.requires_full_frame() {
                                        pipeline_template
                                            .process_device_scan_cached_retained_observed(
                                                &image,
                                                &mut scan_cache,
                                                geometry,
                                                &request.backend,
                                                simulation.context(),
                                            )
                                            .map(Some)
                                    } else {
                                        pipeline_template
                                            .process_device_retained(&image, geometry, backend)
                                            .map(Some)
                                    }
                                } else {
                                    Ok(None)
                                }
                            });
                            match prepared {
                                // Explicit diagnostic taps alone materialize samples.
                                Ok(Some(image))
                                    if matches!(
                                        image.domain(),
                                        spektrafilm_backend::SampleDomain::LogExposure
                                            | spektrafilm_backend::SampleDomain::Density
                                    ) =>
                                {
                                    image.to_host(backend).map(|image| (Some(image), None))
                                }
                                Ok(Some(image)) => Ok((None, Some(image))),
                                Ok(None) => {
                                    let image = host_input.expect("CPU working image prepared");
                                    if runtime_params.scanner.requires_full_frame() {
                                        pipeline_template.process_scan_cached_observed(
                                            &image,
                                            &mut scan_cache,
                                            &request.backend,
                                            simulation.context(),
                                        )
                                    } else {
                                        pipeline_template.process_observed(
                                            Arc::try_unwrap(image)
                                                .unwrap_or_else(|image| (*image).clone()),
                                            request.backend.as_ref(),
                                            simulation.context(),
                                        )
                                    }
                                    .map(|image| (Some(image), None))
                                }
                                Err(error) => Err(error),
                            }
                        };
                        let (output, retained) = observed_result(
                            simulation,
                            processed,
                            IssueCategory::Simulation,
                            IssueBoundary::Simulation,
                        )
                        .map_err(|error| {
                            crate::app::Message::new("operations.simulation_error")
                                .arg("detail", error)
                        })?;
                        let display_prepare = observation.scope(
                            "display_prepare",
                            ObservationKind::Phase,
                            Purpose::Image,
                        );
                        let display_result = (|| {
                            let cancellable = request
                                .backend
                                .with_cancellation(request.snapshot.cancellation());
                            let backend =
                                cancellable.as_deref().unwrap_or(request.backend.as_ref());
                            let bound = display_prepare.context().enabled().then(|| {
                                spektrafilm_backend::bind_backend(
                                    backend,
                                    display_prepare.context().clone(),
                                )
                            });
                            let backend = bound.as_deref().unwrap_or(backend);
                            let _span = display_prepare.context().span().map(|span| span.enter());
                            let histogram = retained
                                .as_ref()
                                .map(|image| {
                                    backend
                                        .retained_histogram(image)
                                        .map(crate::histogram::HistogramData::from_bins)
                                })
                                .transpose()?;
                            let _unit = spektrafilm_core::progress::report("display_prepare");
                            let displayed = if let Some(retained) = retained.as_ref() {
                                display::output_display_raster_retained(
                                    backend,
                                    retained,
                                    &runtime_params.io.output_color_space,
                                    runtime_params.io.output_cctf_encoding,
                                    request.display_enabled,
                                    request.display_profile.as_deref(),
                                    runtime_params.settings.preview_max_size as usize,
                                )
                            } else {
                                display::output_display_raster_with(
                                    backend,
                                    output.as_ref().expect("CPU route produced host output"),
                                    &runtime_params.io.output_color_space,
                                    runtime_params.io.output_cctf_encoding,
                                    request.display_enabled,
                                    request.display_profile.as_deref(),
                                    runtime_params.settings.preview_max_size as usize,
                                )
                            };
                            displayed.map(|(preview, status)| (preview, status, histogram))
                        })();
                        let (preview, display_status, histogram) = observed_result(
                            display_prepare,
                            display_result,
                            IssueCategory::Backend,
                            IssueBoundary::DisplayPrepare,
                        )
                        .map_err(|error| {
                            crate::app::Message::new("operations.display_error")
                                .arg("detail", error)
                        })?;
                        Ok(RenderResult {
                            operator_compare: request.snapshot.operator_compare(),
                            source_metadata: request.source_metadata,
                            output_color_space: runtime_params.io.output_color_space.clone(),
                            output_cctf_encoding: runtime_params.io.output_cctf_encoding,
                            output,
                            retained,
                            histogram,
                            preview,
                            display_status,
                            display_enabled: request.display_enabled,
                            display_profile: request.display_profile,
                            display_max_size: runtime_params.settings.preview_max_size,
                        })
                    }))
                    .unwrap_or_else(|panic| Err(panic_message(&panic)));
                    let cancelled = request
                        .snapshot
                        .cancellation()
                        .is_some_and(|token| token.load(Ordering::SeqCst));
                    let result = if cancelled {
                        Err(crate::app::Message::new("operations.operation_cancelled"))
                    } else {
                        result
                    };
                    attempt.finish(if cancelled {
                        Outcome::Cancelled
                    } else if result.is_ok() {
                        Outcome::Succeeded
                    } else {
                        Outcome::Failed
                    });
                    let _ = tx.send((job, operation, result));
                    *active_for_worker.lock() = None;
                    *active_cancel_for_worker.lock() = None;
                    ctx.request_repaint();
                }
            });
        Self {
            state,
            active,
            rx,
            shutdown,
            handle: Some(handle.expect("OS thread spawn")),
            next_request_id: 0,
            active_cancel,
            invalidate_cache,
        }
    }

    pub(super) fn submit(&mut self, mut request: RenderRequest) -> (RenderJob, Option<Operation>) {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        request.job.request_id = self.next_request_id;
        let job = request.job.clone();
        let (lock, wake) = &*self.state;
        let mut queued = lock.lock();
        if let Some(token) = self.active_cancel.lock().as_ref() {
            token.store(true, Ordering::SeqCst);
        }
        let replaced = queued.replace(request).map(|request| {
            drop(request.worker_wait);
            request.operation
        });
        drop(queued);
        wake.notify_one();
        (job, replaced)
    }

    /// The next request must reload assets after an accepted context replacement.
    pub(crate) fn invalidate_runtime_cache(&self) {
        self.invalidate_cache.store(true, Ordering::SeqCst);
    }

    /// Cancel the active and queued work after the input identity changed.
    pub(super) fn cancel_active(&self) {
        let (lock, wake) = &*self.state;
        let queued = lock.lock();
        if let Some(token) = self.active_cancel.lock().as_ref() {
            token.store(true, Ordering::SeqCst);
        }
        if let Some(token) = queued
            .as_ref()
            .and_then(|request| request.snapshot.cancellation())
        {
            token.store(true, Ordering::SeqCst);
        }
        drop(queued);
        wake.notify_one();
    }

    pub(super) fn shutdown(&mut self) -> Option<Operation> {
        let cancelled;
        {
            let (lock, wake) = &*self.state;
            let mut queued = lock.lock();
            self.shutdown.store(true, Ordering::SeqCst);
            if let Some(token) = self.active_cancel.lock().as_ref() {
                token.store(true, Ordering::SeqCst);
            }
            cancelled = queued.take().map(|request| {
                if let Some(token) = request.snapshot.cancellation() {
                    token.store(true, Ordering::SeqCst);
                }
                drop(request.worker_wait);
                request.operation
            });
            wake.notify_one();
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        cancelled
    }
}

impl Drop for RenderWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(super) fn observed_result<T, E>(
    scope: ObservationScope,
    result: Result<T, E>,
    category: IssueCategory,
    boundary: IssueBoundary,
) -> Result<T, E> {
    let outcome = if result.is_ok() {
        Outcome::Succeeded
    } else {
        scope.context().issue(category, boundary);
        Outcome::Failed
    };
    scope.finish(outcome);
    result
}

/// Extract the human-readable payload from a caught panic. Panics carry
/// either a `&str` (literal messages) or a `String` (formatted ones);
/// anything else gets a generic label.
pub(super) fn panic_message(panic: &(dyn std::any::Any + Send)) -> crate::app::Message {
    if let Some(s) = panic.downcast_ref::<&str>() {
        crate::app::Message::new("operations.worker_panic").arg("detail", s)
    } else if let Some(s) = panic.downcast_ref::<String>() {
        crate::app::Message::new("operations.worker_panic").arg("detail", s)
    } else {
        crate::app::Message::new("operations.worker_panic_no_detail")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::InputImage;
    use spektrafilm_backend::{DeviceImage, SampleDomain};
    use spektrafilm_core::params::RuntimeParams;

    fn retained(input: InputImage) -> DeviceImage {
        let InputImage::Device(image) = input else {
            panic!("GPU source must remain retained");
        };
        image
    }

    fn same_payload(a: &DeviceImage, b: &DeviceImage) -> bool {
        Arc::ptr_eq(a.payload().unwrap(), b.payload().unwrap())
    }

    #[test]
    fn retained_input_cache_reuses_raw_samples_and_releases_invalidated_sources() {
        spektrafilm_gpu::test_support::explicit_adapter().with_shared_backend_pair(
            |backend, other| {
                let backend = Arc::clone(backend);
                let source = Arc::new(ImageBuf::from_data(
                    3,
                    2,
                    (0..18)
                        .map(|i| i as spektrafilm_math::precision::Scalar / 32.0)
                        .collect(),
                ));
                let input = InputImage::Host(source.clone());
                let mut params = RuntimeParams::default();
                params.io.input_color_space = "sRGB".into();
                params.io.input_cctf_decoding = true;
                let mut cache = RetainedInputCache::default();
                let first = retained(
                    cache
                        .prepare(&input, 1, &params, &backend, backend.as_ref())
                        .unwrap(),
                );
                let warm = retained(
                    cache
                        .prepare(&input, 1, &params, &backend, backend.as_ref())
                        .unwrap(),
                );
                assert!(same_payload(&first, &warm));
                assert_eq!(first.to_host(backend.as_ref()).unwrap().data, source.data);
                assert_eq!(
                    Arc::strong_count(&source),
                    2,
                    "the cache only borrows host samples weakly"
                );
                params.settings.preview_max_size = 1;
                params.settings.preview_mode = true;
                let preview = retained(
                    cache
                        .prepare(&input, 1, &params, &backend, backend.as_ref())
                        .unwrap(),
                );
                assert!(same_payload(&first, &preview));
                assert_eq!(
                    (preview.width(), preview.height()),
                    (3, 2),
                    "cache precedes preview geometry"
                );
                drop(warm);

                // Cancelling an operation does not poison the retained raw carrier.
                let cancellation = Arc::new(AtomicBool::new(false));
                let operation = backend
                    .with_cancellation(Some(cancellation.clone()))
                    .unwrap();
                let warm = retained(
                    cache
                        .prepare(&input, 1, &params, &backend, operation.as_ref())
                        .unwrap(),
                );
                cancellation.store(true, Ordering::SeqCst);
                assert!(
                    InputImage::Device(warm.clone())
                        .rotated(1, operation.as_ref(), "sRGB", true,)
                        .unwrap_err()
                        .contains("cancelled")
                );
                drop(operation);
                let fresh = backend
                    .with_cancellation(Some(Arc::new(AtomicBool::new(false))))
                    .unwrap();
                let rotated = InputImage::Device(warm.clone())
                    .rotated(1, fresh.as_ref(), "sRGB", true)
                    .unwrap();
                assert_eq!((rotated.width(), rotated.height()), (2, 3));
                assert!(same_payload(&first, &warm));
                drop((warm, preview, rotated, fresh));
                let released = Arc::downgrade(first.payload().unwrap());
                drop(first);
                let epoch = retained(
                    cache
                        .prepare(&input, 2, &params, &backend, backend.as_ref())
                        .unwrap(),
                );
                assert!(
                    released.upgrade().is_none(),
                    "an epoch change releases the old upload"
                );

                let replacement = InputImage::Host(Arc::new((*source).clone()));
                let replaced = retained(
                    cache
                        .prepare(&replacement, 2, &params, &backend, backend.as_ref())
                        .unwrap(),
                );
                assert!(
                    !same_payload(&epoch, &replaced),
                    "equal samples do not alias different accepted inputs"
                );
                params.io.input_cctf_decoding = false;
                let linear = retained(
                    cache
                        .prepare(&replacement, 2, &params, &backend, backend.as_ref())
                        .unwrap(),
                );
                assert!(!same_payload(&replaced, &linear));
                assert_eq!(linear.domain(), SampleDomain::Linear);
                params.io.input_color_space = "Display P3".into();
                let color = retained(
                    cache
                        .prepare(&replacement, 2, &params, &backend, backend.as_ref())
                        .unwrap(),
                );
                assert!(!same_payload(&linear, &color));
                assert_eq!(color.space().name, "Display P3");
                assert_eq!(color.to_host(backend.as_ref()).unwrap().data, source.data);

                let other = Arc::clone(other);
                let switched = retained(
                    cache
                        .prepare(&replacement, 2, &params, &other, other.as_ref())
                        .unwrap(),
                );
                assert!(!same_payload(&color, &switched));
                assert!(backend.image_payload(&switched).is_err());
                assert_eq!(switched.to_host(other.as_ref()).unwrap().data, source.data);

                let released = Arc::downgrade(switched.payload().unwrap());
                drop(switched);
                let cpu: Arc<dyn ComputeBackend> =
                    Arc::new(spektrafilm_backend::cpu_backend::CpuBackend);
                let host = cache
                    .prepare(&replacement, 2, &params, &cpu, cpu.as_ref())
                    .unwrap();
                assert!(host.same_source(&replacement));
                assert!(
                    released.upgrade().is_none(),
                    "CPU selection releases the GPU upload"
                );
                let cancelled = backend
                    .with_cancellation(Some(Arc::new(AtomicBool::new(true))))
                    .unwrap();
                assert!(
                    cache
                        .prepare(&replacement, 2, &params, &backend, cancelled.as_ref())
                        .unwrap_err()
                        .contains("cancelled")
                );
                assert!(
                    cache.entry.is_none(),
                    "cancelled uploads must not populate the cache"
                );
                drop(cancelled);
                let uploaded = retained(
                    cache
                        .prepare(&replacement, 2, &params, &backend, backend.as_ref())
                        .unwrap(),
                );
                let device_input = InputImage::Device(color.clone());
                let device = retained(
                    cache
                        .prepare(&device_input, 3, &params, &backend, backend.as_ref())
                        .unwrap(),
                );
                assert!(
                    same_payload(&device, &color),
                    "device RAW is not uploaded or copied"
                );
                assert!(
                    cache.entry.is_none(),
                    "a device source evicts the old host upload"
                );
                assert!(!same_payload(&uploaded, &device));
            },
        );
    }
}
