//! Stable runtime facade hiding the calibrated pipeline internals.
use crate::params::{RuntimeParams, Tap};
use crate::pipeline::Pipeline;
use crate::profile;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_backend::telemetry::{ObservationContext, ObservationKind, Purpose};
use spektrafilm_math::image::ImageBuf;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Numerical model revision shared by image, recipe and LUT provenance.
/// Operator controls follow full-frame standard setup. Negative tone has fixed
/// linear-sRGB primaries before conversion to the selected output space.
pub const MODEL_VERSION: &str = "lab_scan_operator_v1";

/// Last-call wall-clock timings exposed by the runtime facade.
pub type Timings = BTreeMap<String, f64>;

pub use crate::pipeline::scan_frame::ScanFrameCache;
/// Selects whether profile-specific defaults are applied when building a runtime.
///
/// GUI edits use `PreserveUserEdits`; batch and LUT construction use
/// `ApplyStockSpecifics`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestMode {
    PreserveUserEdits,
    ApplyStockSpecifics,
}

impl DigestMode {
    fn apply_stock_specifics(self) -> bool {
        matches!(self, Self::ApplyStockSpecifics)
    }
}

/// Rust representation of upstream `RuntimePhotoParams`.
///
/// Profiles remain explicit because the Rust runtime does not use Python's
/// process-global profile registry.
#[derive(Clone)]
pub struct RuntimePhotoParams {
    pub film: profile::Profile,
    pub print: profile::Profile,
    pub params: RuntimeParams,
    pub data_dir: std::path::PathBuf,
}
/// Apply the shared upstream-compatible parameter digest in one place.
pub fn digest_params_with_neutral(
    params: RuntimeParams,
    film: &profile::Profile,
    print: &profile::Profile,
    neutral: &crate::neutral_filters::NeutralFilters,
    mode: DigestMode,
) -> RuntimeParams {
    crate::params_builder::digest_params(
        params,
        film,
        print,
        Some(neutral),
        mode.apply_stock_specifics(),
    )
}

impl RuntimePhotoParams {
    /// Resolve user-facing parameters with the supplied neutral-filter data.
    pub fn digested_params_with_neutral(
        &self,
        mode: DigestMode,
        neutral: &crate::neutral_filters::NeutralFilters,
    ) -> RuntimeParams {
        digest_params_with_neutral(self.params.clone(), &self.film, &self.print, neutral, mode)
    }

    /// Resolve user-facing parameters using the shared digest precedence.
    pub fn digested_params(&self, mode: DigestMode) -> Result<RuntimeParams, String> {
        let neutral = crate::neutral_filters::NeutralFilters::load(&self.data_dir)?;
        Ok(self.digested_params_with_neutral(mode, &neutral))
    }

    /// Build a calibrated runtime after applying the selected digest policy.
    pub fn into_runtime(self, mode: DigestMode) -> Result<Runtime, String> {
        let neutral = crate::neutral_filters::NeutralFilters::load(&self.data_dir)?;
        let params =
            digest_params_with_neutral(self.params, &self.film, &self.print, &neutral, mode);
        Runtime::new(self.film, self.print, params, &self.data_dir)
    }

    /// Build the runtime under the caller-owned preparation observation.
    pub fn into_runtime_observed(
        self,
        mode: DigestMode,
        context: &ObservationContext,
    ) -> Result<Runtime, String> {
        if !context.enabled() {
            return self.into_runtime(mode);
        }
        let mut preparation = context.scope(
            "spectral_prepare",
            ObservationKind::Stage,
            Purpose::Calibration,
        );
        preparation.context().record_executor(
            spektrafilm_backend::telemetry::Executor::Cpu,
            Some(spektrafilm_backend::telemetry::CpuReason::BackendDefault),
        );
        let _span = preparation.context().span().map(|span| span.enter());
        let result = self.into_runtime(mode);
        drop(_span);
        preparation.set_complete(result.is_ok());
        drop(preparation);
        result
    }

    pub fn into_runtime_with_backend(
        self,
        mode: DigestMode,
        backend: &dyn ComputeBackend,
    ) -> Result<Runtime, String> {
        let neutral = crate::neutral_filters::NeutralFilters::load(&self.data_dir)?;
        let params =
            digest_params_with_neutral(self.params, &self.film, &self.print, &neutral, mode);
        Runtime::new_with_backend(self.film, self.print, params, &self.data_dir, backend)
    }

    pub fn into_runtime_observed_with_backend(
        self,
        mode: DigestMode,
        backend: &dyn ComputeBackend,
        context: &ObservationContext,
    ) -> Result<Runtime, String> {
        let neutral = crate::neutral_filters::NeutralFilters::load(&self.data_dir)?;
        let params =
            digest_params_with_neutral(self.params, &self.film, &self.print, &neutral, mode);
        Runtime::new_observed_with_backend(
            self.film,
            self.print,
            params,
            &self.data_dir,
            backend,
            context,
        )
    }
}

#[derive(Clone)]
pub struct Runtime {
    pipeline: Pipeline,
    record_configuration: Option<crate::telemetry::RecordConfiguration>,
    data_dir: std::path::PathBuf,
    timings: Arc<Mutex<Timings>>,
    last_elapsed_seconds: Arc<Mutex<Option<f64>>>,
    cancellation: Option<Arc<AtomicBool>>,
}

impl Runtime {
    /// Bind a caller-owned cancellation signal while retaining prepared film grain caches.
    pub fn with_cancellation(mut self, cancellation: Option<Arc<AtomicBool>>) -> Self {
        self.cancellation = cancellation.clone();
        self.pipeline = self.pipeline.with_cancellation(cancellation);
        self
    }
    /// Prepare the CPU reference runtime. For selected-GPU processing use
    /// [`Self::new_with_backend`], or migrate with [`Self::with_params_with_backend`]
    /// before processing; immutable renders never upload a CPU-prepared TC table.
    pub fn new(
        film: profile::Profile,
        print: profile::Profile,
        params: RuntimeParams,
        data_dir: &Path,
    ) -> Result<Self, String> {
        let pipeline = Pipeline::new_with_spectral(film, print, params, data_dir)?;
        let record_configuration =
            crate::stages::grain_v3::resolve_record(&pipeline.film, &pipeline.params)?;
        Ok(Self {
            pipeline,
            record_configuration,
            data_dir: data_dir.to_owned(),
            timings: Arc::new(Mutex::new(BTreeMap::new())),
            last_elapsed_seconds: Arc::new(Mutex::new(None)),
            cancellation: None,
        })
    }

    /// Prepare filming without resolving or calibrating an unused printing material.
    pub fn new_film(
        film: profile::Profile,
        params: RuntimeParams,
        data_dir: &Path,
    ) -> Result<Self, String> {
        if params.workflow.route_enum() != crate::WorkflowRoute::InputFilm {
            return Err("new_film requires input > film".into());
        }
        Self::new(film.clone(), film, params, data_dir)
    }

    pub fn process_stage(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        seed: u64,
    ) -> Result<crate::stage_artifact::MaterialState, String> {
        let started = Instant::now();
        let result = self.pipeline.process_stage(image, backend, seed);
        self.record_elapsed(started, None);
        result
    }

    /// Preserve construction semantics while attributing preparation to this operation.
    pub fn new_observed(
        film: profile::Profile,
        print: profile::Profile,
        params: RuntimeParams,
        data_dir: &Path,
        context: &ObservationContext,
    ) -> Result<Self, String> {
        if !context.enabled() {
            return Self::new(film, print, params, data_dir);
        }
        let mut preparation = context.scope(
            "spectral_prepare",
            ObservationKind::Stage,
            Purpose::Calibration,
        );
        preparation.context().record_executor(
            spektrafilm_backend::telemetry::Executor::Cpu,
            Some(spektrafilm_backend::telemetry::CpuReason::BackendDefault),
        );
        let _span = preparation.context().span().map(|span| span.enter());
        let result = Self::new(film, print, params, data_dir);
        drop(_span);
        preparation.set_complete(result.is_ok());
        drop(preparation);
        result
    }

    pub fn new_with_backend(
        film: profile::Profile,
        print: profile::Profile,
        params: RuntimeParams,
        data_dir: &Path,
        backend: &dyn ComputeBackend,
    ) -> Result<Self, String> {
        let pipeline = Pipeline::new_with_spectral_backend(film, print, params, data_dir, backend)?;
        let record_configuration =
            crate::stages::grain_v3::resolve_record(&pipeline.film, &pipeline.params)?;
        Ok(Self {
            pipeline,
            record_configuration,
            data_dir: data_dir.to_owned(),
            timings: Arc::new(Mutex::new(BTreeMap::new())),
            last_elapsed_seconds: Arc::new(Mutex::new(None)),
            cancellation: None,
        })
    }

    pub fn new_observed_with_backend(
        film: profile::Profile,
        print: profile::Profile,
        params: RuntimeParams,
        data_dir: &Path,
        backend: &dyn ComputeBackend,
        context: &ObservationContext,
    ) -> Result<Self, String> {
        let mut preparation = context.scope(
            "spectral_prepare",
            ObservationKind::Stage,
            Purpose::Calibration,
        );
        if !backend.is_gpu() {
            preparation.context().record_executor(
                spektrafilm_backend::telemetry::Executor::Cpu,
                Some(spektrafilm_backend::telemetry::CpuReason::CpuSelected),
            );
        }
        let bound = spektrafilm_backend::bind_backend(backend, preparation.context().clone());
        let _span = preparation.context().span().map(|span| span.enter());
        let result = Self::new_with_backend(film, print, params, data_dir, bound.as_ref());
        drop(_span);
        preparation.set_complete(result.is_ok());
        drop(bound);
        drop(preparation);
        result
    }

    /// Load stock profiles and prepare the CPU reference runtime.
    /// Selected-GPU callers must migrate with [`Self::with_params_with_backend`]
    /// or load profiles and construct with [`Self::new_with_backend`].
    pub fn from_stocks(
        film: &str,
        print: &str,
        params: RuntimeParams,
        data_dir: &Path,
    ) -> Result<Self, String> {
        let film_profile = profile::load_profile_by_name(data_dir, film)
            .map_err(|e| format!("loading film profile {film:?}: {e}"))?;
        let print_profile = profile::load_profile_by_name(data_dir, print)
            .map_err(|e| format!("loading print profile {print:?}: {e}"))?;
        Self::new(film_profile, print_profile, params, data_dir)
    }
    pub fn from_photo_params(photo: RuntimePhotoParams) -> Result<Self, String> {
        Self::new(photo.film, photo.print, photo.params, &photo.data_dir)
    }

    fn record_elapsed(&self, started: Instant, stage_timings: Option<Timings>) {
        let elapsed = started.elapsed().as_secs_f64();
        if let Ok(mut timings) = self.timings.lock() {
            *timings = stage_timings.unwrap_or_default();
            timings.insert("total".into(), elapsed);
        }
        if let Ok(mut last) = self.last_elapsed_seconds.lock() {
            *last = Some(elapsed);
        }
    }

    pub fn process(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<ImageBuf, String> {
        self.process_with_taps(image, backend, None, None)
    }

    /// Process with an explicit operation context, independent of last-call timings.
    pub fn process_observed(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        context: &ObservationContext,
    ) -> Result<ImageBuf, String> {
        self.process_with_taps_observed(image, backend, None, None, context)
    }

    /// Reuse an immutable full-workframe scan baseline for operator edits.
    /// The input Arc identifies the loaded frame; no automatic offset crosses frames.
    pub fn process_scan_cached_observed(
        &self,
        image: &Arc<ImageBuf>,
        cache: &mut ScanFrameCache,
        backend: &Arc<dyn ComputeBackend>,
        context: &ObservationContext,
    ) -> Result<ImageBuf, String> {
        let started = Instant::now();
        let result = if context.enabled() {
            let _span = context.span().map(|span| span.enter());
            let bound = spektrafilm_backend::bind_backend(backend.as_ref(), context.clone());
            self.pipeline
                .process_scan_cached(image, cache, bound.as_ref(), backend)
        } else {
            self.pipeline
                .process_scan_cached(image, cache, backend.as_ref(), backend)
        };
        self.record_elapsed(started, None);
        result
    }

    pub fn process_with_taps_observed(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
        context: &ObservationContext,
    ) -> Result<ImageBuf, String> {
        if !context.enabled() {
            return self.process_with_taps(image, backend, inject, collect);
        }
        let _span = context.span().map(|span| span.enter());
        let bound = spektrafilm_backend::bind_backend(backend, context.clone());
        self.process_with_taps(image, bound.as_ref(), inject, collect)
    }

    /// Keep the borrowed resident route available to operation-owned GUI work.
    pub fn process_resident_borrowed_observed(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
        context: &ObservationContext,
    ) -> Result<Option<ImageBuf>, String> {
        if !context.enabled() {
            return self.pipeline.process_resident_borrowed(image, backend);
        }
        let _span = context.span().map(|span| span.enter());
        let bound = spektrafilm_backend::bind_backend(backend, context.clone());
        self.pipeline
            .process_resident_borrowed(image, bound.as_ref())
    }

    pub fn process_with_taps(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
    ) -> Result<ImageBuf, String> {
        let started = Instant::now();
        let mut stage_timings = BTreeMap::new();
        let result = self.pipeline.process_with_taps_timed(
            image,
            backend,
            inject,
            collect,
            Some(&mut stage_timings),
            None,
        );
        self.record_elapsed(started, Some(stage_timings));
        result
    }

    /// Process with an explicit operation context while capturing the native
    /// stage evidence of the executed render. The capture observes the run;
    /// it must not change its route, parameters or output pixels.
    pub fn process_captured(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
        context: &ObservationContext,
        capture: &mut crate::stage_evidence::StageCapture,
    ) -> Result<ImageBuf, String> {
        let started = Instant::now();
        let mut stage_timings = BTreeMap::new();
        let run = |backend: &dyn ComputeBackend| {
            self.pipeline.process_with_taps_timed(
                image,
                backend,
                inject,
                collect,
                Some(&mut stage_timings),
                Some(capture),
            )
        };
        let result = if !context.enabled() {
            run(backend)
        } else {
            let _span = context.span().map(|span| span.enter());
            let bound = spektrafilm_backend::bind_backend(backend, context.clone());
            run(bound.as_ref())
        };
        self.record_elapsed(started, Some(stage_timings));
        result
    }

    /// Render one image and keep the result on the device.
    ///
    /// This is the production entry point for uploaded raster sources. RAW
    /// sources already produced on-device use [`Self::process_device_retained`]
    /// instead. Both retain samples through saving conversion and viewing.
    /// `Ok(None)` is reserved for an initially selected CPU backend.
    pub fn process_retained(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        self.process_retained_with(image, crate::pipeline::RetainedRender::FULL, backend)
    }

    /// Render an input already owned by the selected GPU, without a numerical
    /// readback or source upload. Preview and deferred orientation use the same
    /// full-source metering and working geometry as uploaded raster sources.
    pub fn process_device_retained(
        &self,
        image: &spektrafilm_backend::DeviceImage,
        request: crate::pipeline::RetainedRender,
        backend: &dyn ComputeBackend,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.process_device_retained_with_taps(image, request, backend, None, None)
    }

    /// Retained RAW/raster diagnostic route. Explicit tap overrides retain
    /// the same precedence and topology validation as the CPU reference.
    pub fn process_device_retained_with_taps(
        &self,
        image: &spektrafilm_backend::DeviceImage,
        request: crate::pipeline::RetainedRender,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        let started = Instant::now();
        let result = self
            .pipeline
            .process_device_retained_with_taps(image, request, backend, inject, collect);
        self.record_elapsed(started, None);
        result
    }
    /// Render a host source using the frame-owned immutable device scan baseline.
    pub fn process_host_scan_cached_retained(
        &self,
        image: &Arc<ImageBuf>,
        cache: &mut ScanFrameCache,
        request: crate::pipeline::RetainedRender,
        backend: &Arc<dyn ComputeBackend>,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        let started = Instant::now();
        let result = self.pipeline.process_host_scan_cached_retained(
            image,
            cache,
            request,
            backend.as_ref(),
            backend,
        );
        self.record_elapsed(started, None);
        result
    }

    /// Render a device source using the frame-owned immutable device scan baseline.
    pub fn process_device_scan_cached_retained(
        &self,
        image: &spektrafilm_backend::DeviceImage,
        cache: &mut ScanFrameCache,
        request: crate::pipeline::RetainedRender,
        backend: &Arc<dyn ComputeBackend>,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        let started = Instant::now();
        let result = self.pipeline.process_device_scan_cached_retained(
            image,
            cache,
            request,
            backend.as_ref(),
            backend,
        );
        self.record_elapsed(started, None);
        result
    }

    /// Observed host-source cache entry retaining the original backend owner identity.
    pub fn process_host_scan_cached_retained_observed(
        &self,
        image: &Arc<ImageBuf>,
        cache: &mut ScanFrameCache,
        request: crate::pipeline::RetainedRender,
        backend: &Arc<dyn ComputeBackend>,
        context: &ObservationContext,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        if !context.enabled() {
            return self.process_host_scan_cached_retained(image, cache, request, backend);
        }
        let started = Instant::now();
        let _span = context.span().map(|span| span.enter());
        let bound = spektrafilm_backend::bind_backend(backend.as_ref(), context.clone());
        let result = self.pipeline.process_host_scan_cached_retained(
            image,
            cache,
            request,
            bound.as_ref(),
            backend,
        );
        self.record_elapsed(started, None);
        result
    }

    /// Observed device-source cache entry retaining the original backend owner identity.
    pub fn process_device_scan_cached_retained_observed(
        &self,
        image: &spektrafilm_backend::DeviceImage,
        cache: &mut ScanFrameCache,
        request: crate::pipeline::RetainedRender,
        backend: &Arc<dyn ComputeBackend>,
        context: &ObservationContext,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        if !context.enabled() {
            return self.process_device_scan_cached_retained(image, cache, request, backend);
        }
        let started = Instant::now();
        let _span = context.span().map(|span| span.enter());
        let bound = spektrafilm_backend::bind_backend(backend.as_ref(), context.clone());
        let result = self.pipeline.process_device_scan_cached_retained(
            image,
            cache,
            request,
            bound.as_ref(),
            backend,
        );
        self.record_elapsed(started, None);
        result
    }

    /// [`Self::process_retained`] with a deferred input rotation.
    pub fn process_retained_rotated(
        &self,
        image: &ImageBuf,
        quarter_turns: u32,
        backend: &dyn ComputeBackend,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        self.process_retained_with(
            image,
            crate::pipeline::RetainedRender::rotated(quarter_turns),
            backend,
        )
    }

    /// Preview render that measures the full source and keeps the result on the
    /// device. The long-edge resample is the preview's triangle reference.
    pub fn process_preview_retained(
        &self,
        image: &ImageBuf,
        max_size: u32,
        backend: &dyn ComputeBackend,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        self.process_retained_with(
            image,
            crate::pipeline::RetainedRender::preview(max_size),
            backend,
        )
    }

    /// [`Self::process_preview_retained`] with a deferred input rotation.
    pub fn process_preview_retained_rotated(
        &self,
        image: &ImageBuf,
        max_size: u32,
        quarter_turns: u32,
        backend: &dyn ComputeBackend,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        self.process_retained_with(
            image,
            crate::pipeline::RetainedRender {
                rotation_quarter_turns: quarter_turns,
                preview_max_size: Some(max_size),
            },
            backend,
        )
    }

    /// Shared retained-render body: timings and cancellation follow the host
    /// route. Scanner auto-setup is a device reduction over the retained
    /// source, so a per-frame policy no longer selects the host route.
    fn process_retained_with(
        &self,
        image: &ImageBuf,
        request: crate::pipeline::RetainedRender,
        backend: &dyn ComputeBackend,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        self.process_retained_with_taps(image, request, backend, None, None)
    }

    /// Host-source diagnostic counterpart of the retained device entry.
    pub fn process_retained_with_taps(
        &self,
        image: &ImageBuf,
        request: crate::pipeline::RetainedRender,
        backend: &dyn ComputeBackend,
        inject: Option<Tap>,
        collect: Option<Tap>,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        let started = Instant::now();
        let result = self
            .pipeline
            .process_retained_with_taps(image, request, backend, inject, collect);
        self.record_elapsed(started, None);
        result
    }

    pub fn process_preview(
        &self,
        image: ImageBuf,
        max_size: u32,
        backend: &dyn ComputeBackend,
    ) -> Result<ImageBuf, String> {
        if backend.is_gpu() {
            let retained = self
                .process_preview_retained(&image, max_size, backend)?
                .ok_or_else(|| "selected GPU declined the retained preview route".to_owned())?;
            return backend.materialize_retained(&retained);
        }
        if self.params().scanner.requires_full_frame() {
            let output = self.process(image, backend)?;
            if self
                .params()
                .taps
                .collect
                .as_deref()
                .is_none_or(|tap| tap == "rgb_out")
            {
                Ok(crate::params_builder::resize_for_preview(&output, max_size))
            } else {
                Ok(output)
            }
        } else {
            let preview = crate::params_builder::resize_for_preview(&image, max_size);
            self.process(preview, backend)
        }
    }

    /// Process using `settings.preview_max_size`, matching upstream
    /// `simulate_preview` without duplicating the preview limit at call sites.
    pub fn process_configured_preview(
        &self,
        image: ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<ImageBuf, String> {
        self.process_preview(image, self.params().settings.preview_max_size, backend)
    }

    pub fn update(&mut self, params: RuntimeParams) -> Result<(), String> {
        params.validate()?;
        // Film-curve controls are absent from the spectral and calibration
        // control keys, so refresh the resolved film unconditionally and keep
        // the prepared grain cache and cancellation binding across the rebuild.
        let pipeline = self.pipeline.clone().rebuild_spectral(params)?;
        let record_configuration =
            crate::stages::grain_v3::resolve_record(&pipeline.film, &pipeline.params)?;
        self.pipeline = pipeline;
        self.record_configuration = record_configuration;
        Ok(())
    }
    /// Return a runtime with updated parameters while retaining the facade state.
    pub fn with_params(mut self, params: RuntimeParams) -> Result<Self, String> {
        self.pipeline = self.pipeline.with_params(params)?;
        self.record_configuration =
            crate::stages::grain_v3::resolve_record(&self.pipeline.film, &self.pipeline.params)?;
        Ok(self)
    }

    pub fn with_params_with_backend(
        mut self,
        params: RuntimeParams,
        backend: &dyn ComputeBackend,
    ) -> Result<Self, String> {
        self.pipeline = self.pipeline.with_params_with_backend(params, backend)?;
        self.record_configuration =
            crate::stages::grain_v3::resolve_record(&self.pipeline.film, &self.pipeline.params)?;
        Ok(self)
    }

    pub fn params(&self) -> &RuntimeParams {
        &self.pipeline.params
    }

    pub fn telemetry_configuration(&self) -> crate::telemetry::RenderConfiguration {
        let mut configuration = crate::telemetry::RenderConfiguration::from_runtime(
            &self.pipeline.params,
            &self.pipeline.film,
            &self.pipeline.print,
        );
        configuration.grain.record = self.record_configuration;
        configuration
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn get_timings(&self) -> Timings {
        self.timings
            .lock()
            .map(|timings| timings.clone())
            .unwrap_or_default()
    }

    pub fn get_total_elapsed_time(&self) -> Option<f64> {
        self.last_elapsed_seconds
            .lock()
            .ok()
            .and_then(|elapsed| *elapsed)
    }

    pub fn format_timings(&self) -> String {
        let mut output = String::from("Simulation timings\n");
        match self.get_total_elapsed_time() {
            Some(total) => {
                output.push_str(&format!("  Total  {:.3} ms  100.0%\n", total * 1000.0));
                for (label, elapsed) in self.get_timings() {
                    if label != "total" {
                        let percentage = if total > 0.0 {
                            elapsed / total * 100.0
                        } else {
                            0.0
                        };
                        output.push_str(&format!(
                            "  {label:<24} {:.3} ms  {percentage:.1}%\n",
                            elapsed * 1000.0
                        ));
                    }
                }
            }
            None => output.push_str("  No recorded timings\n"),
        }
        output
    }

    pub fn print_timings(&self) {
        print!("{}", self.format_timings());
    }
}

/// Build default runtime parameters from named profiles, matching upstream
/// `init_params` while making the data directory explicit.
pub fn init_params(
    film_profile: &str,
    print_profile: &str,
    data_dir: &Path,
) -> Result<RuntimePhotoParams, String> {
    let film = profile::load_profile_by_name(data_dir, film_profile)
        .map_err(|e| format!("loading film profile {film_profile:?}: {e}"))?;
    let print = profile::load_profile_by_name(data_dir, print_profile)
        .map_err(|e| format!("loading print profile {print_profile:?}: {e}"))?;
    Ok(photo_params(
        film,
        print,
        RuntimeParams::default(),
        data_dir,
    ))
}

/// Build the explicit Rust equivalent of upstream `photo_params`.
pub fn photo_params(
    film: profile::Profile,
    print: profile::Profile,
    params: RuntimeParams,
    data_dir: &Path,
) -> RuntimePhotoParams {
    RuntimePhotoParams {
        film,
        print,
        params,
        data_dir: data_dir.to_owned(),
    }
}

/// Single-call simulation equivalent to upstream `simulate`.
pub fn simulate(
    image: ImageBuf,
    photo: &RuntimePhotoParams,
    backend: &dyn ComputeBackend,
    digest_params_first: bool,
    print_timings: bool,
) -> Result<ImageBuf, String> {
    let params = if digest_params_first {
        photo.digested_params(DigestMode::ApplyStockSpecifics)?
    } else {
        photo.params.clone()
    };
    let runtime = Runtime::new_with_backend(
        photo.film.clone(),
        photo.print.clone(),
        params,
        &photo.data_dir,
        backend,
    )?;
    let result = runtime.process(image, backend);
    if print_timings {
        runtime.print_timings();
    }
    result
}

/// Preview simulation equivalent to upstream `simulate_preview`.
pub fn simulate_preview(
    image: ImageBuf,
    photo: &RuntimePhotoParams,
    backend: &dyn ComputeBackend,
    digest_params_first: bool,
    print_timings: bool,
) -> Result<ImageBuf, String> {
    let params = if digest_params_first {
        photo.digested_params(DigestMode::ApplyStockSpecifics)?
    } else {
        photo.params.clone()
    };
    let runtime = Runtime::new_with_backend(
        photo.film.clone(),
        photo.print.clone(),
        params,
        &photo.data_dir,
        backend,
    )?;
    let result = runtime.process_configured_preview(image, backend);
    if print_timings {
        runtime.print_timings();
    }
    result
}

/// Legacy ART compatibility name.
pub type AgXPhoto = Runtime;

/// Compatibility name matching upstream's user-facing simulator wrapper.
pub type Simulator = Runtime;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::data_dir;
    use spektrafilm_backend::cpu_backend::CpuBackend;
    use spektrafilm_math::precision::from_f64;

    #[test]
    fn runtime_preparation_rejects_removed_scan_output_before_calibration() {
        let data = data_dir();
        let film = profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let print = profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "positive_scan".into();
        let error = Runtime::new(film, print, params, &data)
            .err()
            .expect("removed scan output must fail runtime preparation");
        assert!(error.contains("scanner.scan_output"));
        assert!(error.contains("positive_scan"));
    }

    fn scanner_params() -> RuntimeParams {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.camera.auto_exposure = false;
        params.film_render.grain.active = false;
        params.film_render.halation.active = false;
        params.film_render.dir_couplers.active = false;
        params.film_render.glare.active = false;
        params.scanner.scan_output = "negative_to_positive".into();
        params
    }

    fn scanner_image() -> ImageBuf {
        ImageBuf::from_data(
            4,
            4,
            (0..16)
                .flat_map(|i| [from_f64(0.08 + i as f64 * 0.02); 3])
                .collect(),
        )
    }

    #[test]
    fn operator_cache_matches_fresh_render_across_edits_and_frames() {
        let mut params = scanner_params();
        params.io.output_cctf_encoding = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.scanner.unsharp_mask = [0.0; 2];
        let runtime = Runtime::from_stocks(
            "kodak_portra_400",
            "kodak_portra_endura",
            params,
            &data_dir(),
        )
        .unwrap();
        let backend: Arc<dyn ComputeBackend> = Arc::new(CpuBackend);
        let image = Arc::new(scanner_image());
        let mut cache = ScanFrameCache::default();
        let render = |runtime: &Runtime, image: &Arc<ImageBuf>, cache: &mut ScanFrameCache| {
            let cached = runtime
                .process_scan_cached_observed(
                    image,
                    cache,
                    &backend,
                    &ObservationContext::default(),
                )
                .unwrap();
            let fresh = runtime
                .process(image.as_ref().clone(), backend.as_ref())
                .unwrap();
            assert_eq!(cached.data, fresh.data);
        };
        render(&runtime, &image, &mut cache);
        let mut corrected = runtime.params().clone();
        corrected.scanner.operator.exposure_ev = 0.5;
        corrected.scanner.operator.warmth_ev = 0.25;
        corrected.scanner.operator.contrast = 0.75;
        let corrected = runtime.clone().with_params(corrected).unwrap();
        render(&corrected, &image, &mut cache);
        let mut upstream = corrected.params().clone();
        upstream.camera.exposure_compensation_ev = 1.0;
        let upstream = corrected.with_params(upstream).unwrap();
        render(&upstream, &image, &mut cache);
        let mut second = scanner_image();
        second.data[..3].fill(from_f64(8.0));
        render(&upstream, &Arc::new(second), &mut cache);
    }

    #[test]
    fn cached_scan_matches_fresh_cpu_after_convert_geometry_mix_and_spatial_edits() {
        for route in ["input > film > scan", "input > convert-film > scan"] {
            let mut params = scanner_params();
            params.workflow.route = route.into();
            params.io.input_cctf_decoding = false;
            params.io.output_cctf_encoding = false;
            params.io.output_gamut_compress.algorithm = "off".into();
            params.scanner.unsharp_mask = [0.0; 2];
            params.io.crop = false;
            let mut runtime = Runtime::from_stocks(
                "kodak_portra_400",
                "kodak_portra_endura",
                params,
                &data_dir(),
            )
            .unwrap();
            let backend: Arc<dyn ComputeBackend> = Arc::new(CpuBackend);
            let image = Arc::new(scanner_image());
            let mut cache = ScanFrameCache::default();
            let render = |runtime: &Runtime, cache: &mut ScanFrameCache, dimensions| {
                let cached = runtime
                    .process_scan_cached_observed(
                        &image,
                        cache,
                        &backend,
                        &ObservationContext::default(),
                    )
                    .unwrap();
                let fresh = runtime
                    .process(image.as_ref().clone(), backend.as_ref())
                    .unwrap();
                assert_eq!((cached.width, cached.height), dimensions, "{route}");
                assert_eq!((fresh.width, fresh.height), dimensions, "{route}");
                assert!(cached.data.iter().all(|value| value.is_finite()), "{route}");
                assert_eq!(cached.data, fresh.data, "{route}");
                assert!(
                    cache.retained_bytes() > 0,
                    "{route} must exercise the cache"
                );
                cached
            };
            let baseline = render(&runtime, &mut cache, (4, 4));
            assert!(
                baseline
                    .data
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .any(|pixel| pixel.as_slice() != &baseline.data[..3]),
                "{route}: the fixture must contain spatially varying output"
            );

            let mut params = runtime.params().clone();
            params.io.crop = true;
            params.io.crop_center = [0.5, 0.5];
            params.io.crop_size = [0.5, 0.5];
            runtime = runtime.with_params(params).unwrap();
            render(&runtime, &mut cache, (2, 2));

            let mut params = runtime.params().clone();
            params.io.upscale_factor = 2.0;
            runtime = runtime.with_params(params).unwrap();
            let unmixed = render(&runtime, &mut cache, (4, 4));

            let mut params = runtime.params().clone();
            params.output_mix.amount = 0.35;
            runtime = runtime.with_params(params).unwrap();
            let mixed = render(&runtime, &mut cache, (4, 4));
            assert!(
                mixed
                    .data
                    .iter()
                    .zip(&unmixed.data)
                    .any(|(a, b)| (*a as f64 - *b as f64).abs() > 1e-6),
                "{route}: output mixing must change real pixels"
            );

            let mut params = runtime.params().clone();
            params.scanner.lens_blur = 1.0;
            runtime = runtime.with_params(params).unwrap();
            let blurred = render(&runtime, &mut cache, (4, 4));
            assert!(
                blurred
                    .data
                    .iter()
                    .zip(&mixed.data)
                    .any(|(a, b)| (*a as f64 - *b as f64).abs() > 1e-6),
                "{route}: scanner blur must invalidate the prepared acquisition"
            );
            render(&runtime, &mut cache, (4, 4));
        }
    }

    #[test]
    fn cached_scan_selected_gpu_keeps_acquisition_and_preview_on_device() {
        use spektrafilm_backend::telemetry::{CollectionMode, Executor};

        let mut params = scanner_params();
        params.scanner.lens_blur = 1.0;
        params.scanner.unsharp_mask = [0.0; 2];
        params.io.output_color_space = "Display P3".into();
        params.io.output_gamut_compress.algorithm = "off".into();
        params.scanner.operator.exposure_ev = 0.5;
        params.scanner.operator.warmth_ev = 0.25;
        params.scanner.operator.tint_ev = -0.125;
        params.scanner.operator.contrast = 0.75;
        let runtime = Runtime::from_stocks(
            "kodak_portra_400",
            "kodak_portra_endura",
            params,
            &data_dir(),
        )
        .unwrap();
        let image = Arc::new(scanner_image());
        let expected = runtime
            .process_preview(image.as_ref().clone(), 2, &CpuBackend)
            .unwrap();
        for explicit in [true, false] {
            let gpu = if explicit {
                spektrafilm_gpu::test_support::explicit_adapter()
            } else {
                spektrafilm_gpu::test_support::adapter()
            };
            gpu.with_shared_backend(|backend| {
                let runtime = runtime.clone()
                    .with_params_with_backend(runtime.params().clone(), backend.as_ref()).unwrap();
                let mut cache = ScanFrameCache::default();
                let context = ObservationContext::new(CollectionMode::Summary);
                let cached = runtime.process_scan_cached_observed(&image, &mut cache, backend, &context).unwrap();
                assert_eq!((cached.width, cached.height), (4, 4));
                assert!(cached.data.iter().all(|value| value.is_finite()));
                assert_eq!(cache.retained_bytes(), 0, "GPU acquisition must not retain a host exposure cache");
                let snapshot = context.snapshot();
                assert!(!snapshot.observations.iter().any(|stage| stage.executor == Some(Executor::Cpu)),
                    "selected GPU executed CPU image arithmetic: {:?}", snapshot.observations);
                assert_eq!(snapshot.observations.iter().filter(|stage| stage.name == "standard_setup").count(), 1);
                let reused = ObservationContext::new(CollectionMode::Summary);
                let preview = runtime.process_host_scan_cached_retained_observed(&image, &mut cache,
                    crate::pipeline::RetainedRender::preview(2), backend, &reused).unwrap().to_host(backend.as_ref()).unwrap();
                assert!(!reused.snapshot().observations.iter().any(|stage|
                    matches!(stage.name.as_str(), "standard_setup" | "filming_expose" | "film_density" | "converting")),
                    "unchanged prepared scan must not repeat upstream computation or setup");
                assert_eq!((preview.width, preview.height), (2, 2));
                let mut sum = 0.0;
                let mut maximum = 0.0_f64;
                for (actual, expected) in preview.data.iter().zip(&expected.data) {
                    let difference = (*actual as f64 - *expected as f64).abs();
                    sum += difference;
                    maximum = maximum.max(difference);
                }
                let mean = sum / expected.data.len() as f64;
                assert!(mean < 5e-3 && maximum < 5e-2,
                    "full-frame standard/operator/primaries preview divergence: mean={mean} max={maximum}");
                let mut corrected = runtime.params().clone();
                corrected.scanner.operator.exposure_ev += 0.5;
                let corrected = runtime.clone().with_params_with_backend(corrected, backend.as_ref()).unwrap();
                let changed = ObservationContext::new(CollectionMode::Summary);
                let actual = corrected.process_host_scan_cached_retained_observed(&image, &mut cache,
                    crate::pipeline::RetainedRender::FULL, backend, &changed).unwrap().to_host(backend.as_ref()).unwrap();
                assert!(!changed.snapshot().observations.iter().any(|stage| stage.name == "standard_setup"),
                    "operator edit must reuse immutable automatic setup");
                assert!(actual.data.iter().zip(&cached.data).any(|(a,b)| (*a as f64 - *b as f64).abs() > 1e-4),
                    "operator edit must change actual output pixels");
                let fresh = corrected.process(image.as_ref().clone(), backend.as_ref()).unwrap();
                for (actual, fresh) in actual.data.iter().zip(&fresh.data) {
                    assert!((*actual as f64 - *fresh as f64).abs() < 1e-5, "cached operator output differs from fresh fullframe render");
                }
            });
        }
    }

    #[test]
    fn preview_entry_points_preserve_diagnostic_frame_and_samples() {
        let data = data_dir();
        let film = profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let print = profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
        let image = scanner_image();
        let backend = CpuBackend;
        for collect in ["rgb_in", "rgb_pre", "log_e_film", "cmy_film"] {
            let mut params = scanner_params();
            params.settings.preview_max_size = 2;
            params.io.crop = false;
            params.taps.collect = Some(collect.into());
            let photo = RuntimePhotoParams {
                film: film.clone(),
                print: print.clone(),
                params,
                data_dir: data.clone(),
            };
            let runtime = Runtime::from_photo_params(photo.clone()).unwrap();
            let expected = runtime.process(image.clone(), &backend).unwrap();
            assert_eq!((expected.width, expected.height), (4, 4), "{collect}");
            assert!(
                expected.data.iter().all(|value| value.is_finite()),
                "{collect}"
            );
            assert!(
                expected
                    .data
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .any(|pixel| pixel.as_slice() != &expected.data[..3]),
                "{collect}: diagnostic fixture must have varying samples"
            );
            if collect == "rgb_in" {
                assert_eq!(expected.data, image.data);
            }
            for preview in [
                runtime.process_preview(image.clone(), 2, &backend).unwrap(),
                runtime
                    .process_configured_preview(image.clone(), &backend)
                    .unwrap(),
                simulate_preview(image.clone(), &photo, &backend, false, false).unwrap(),
            ] {
                assert_eq!((preview.width, preview.height), (4, 4), "{collect}");
                assert_eq!(
                    preview.data, expected.data,
                    "{collect}: samples were resampled"
                );
            }
        }
    }

    #[test]
    fn standard_preview_resamples_finished_full_workframe() {
        let mut params = scanner_params();
        params.taps.inject = Some("cmy_film".into());
        params.scanner.unsharp_mask = [0.0; 2];
        params.settings.preview_max_size = 2;
        params.io.crop = false;
        let runtime = Runtime::from_stocks(
            "kodak_portra_400",
            "kodak_portra_endura",
            params,
            &data_dir(),
        )
        .unwrap();
        let mut density = ImageBuf::from_data(16, 16, vec![from_f64(0.6); 16 * 16 * 3]);
        density.data[..3].fill(from_f64(3.0));
        let backend = CpuBackend;
        let full = runtime.process(density.clone(), &backend).unwrap();
        let expected = crate::params_builder::resize_for_preview(&full, 2);
        let preview = runtime
            .process_configured_preview(density, &backend)
            .unwrap();
        assert_eq!((preview.width, preview.height), (2, 2));
        assert_eq!(preview.data, expected.data);
    }

    #[test]
    fn explicit_rgb_input_overrides_persisted_diagnostic_inject() {
        let mut params = scanner_params();
        params.taps.inject = Some("log_e_film".into());
        params.scanner.scan_output = "negative_to_positive".into();
        let runtime = Runtime::from_stocks(
            "kodak_portra_400",
            "kodak_portra_endura",
            params,
            &data_dir(),
        )
        .unwrap();
        let backend = CpuBackend;
        let image = scanner_image();
        let explicit = runtime
            .process_with_taps(image.clone(), &backend, Some(Tap::RgbIn), Some(Tap::RgbOut))
            .unwrap();
        let mut params = runtime.params().clone();
        params.taps.inject = None;
        let default = runtime
            .with_params(params)
            .unwrap()
            .process(image, &backend)
            .unwrap();
        assert_eq!((explicit.width, explicit.height), (4, 4));
        assert_eq!(explicit.data, default.data);
    }

    #[test]
    fn public_gpu_partial_taps_keep_autoexposure_on_the_retained_device_route() {
        use spektrafilm_backend::telemetry::{
            CollectionMode, Executor, ObservationContext, ResidentDeclineReason,
        };
        let gpu = spektrafilm_gpu::test_support::adapter();
        let dir = data_dir();
        let mut params = scanner_params();
        params.camera.auto_exposure = true;
        params.io.input_color_space = "ACES2065-1".into();
        params.io.input_cctf_decoding = false;
        params.settings.rgb_to_raw_method = "mallett2019".into();
        params.io.output_gamut_compress.algorithm = "off".into();
        params.taps.inject = Some("rgb_in".into());
        params.taps.collect = Some("rgb_pre".into());
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let runtime = Runtime::new(film, print, params, &dir).unwrap();
        let image = scanner_image();
        let expected = runtime
            .process_with_taps(
                image.clone(),
                &CpuBackend,
                Some(Tap::RgbIn),
                Some(Tap::RgbPre),
            )
            .unwrap();
        let params = runtime.params().clone();
        let runtime = runtime
            .with_params_with_backend(params, gpu.backend())
            .unwrap();
        let context = ObservationContext::new(CollectionMode::Summary);
        let actual = runtime
            .process_with_taps_observed(
                image.clone(),
                gpu.backend(),
                Some(Tap::RgbIn),
                Some(Tap::RgbPre),
                &context,
            )
            .unwrap();
        assert_eq!((actual.width, actual.height), (image.width, image.height));
        assert!(
            expected
                .data
                .iter()
                .zip(&image.data)
                .any(|(exposed, source)| (*exposed as f64 - *source as f64).abs() > 1e-4),
            "fixture must exercise a nonidentity auto-exposure scale"
        );
        for (actual, expected) in actual.data.iter().zip(&expected.data) {
            assert!(
                (*actual as f64 - *expected as f64).abs() < 2e-5,
                "partial-tap exposure differs: {actual} versus {expected}"
            );
        }
        let snapshot = context.snapshot();
        assert!(snapshot.totals.dispatch_count.unwrap_or(0) > 0);
        assert!(
            !snapshot
                .observations
                .iter()
                .any(|stage| stage.executor == Some(Executor::Cpu)),
            "public partial taps executed a CPU numerical stage: {:?}",
            snapshot.observations
        );
        assert!(
            !snapshot
                .execution
                .resident_decline_reasons
                .contains(&ResidentDeclineReason::DiagnosticTapRoute)
        );
        assert!(
            !snapshot.observations.iter().any(|stage| matches!(
                stage.name.as_str(),
                "filming_expose" | "film_density" | "scan_spectral"
            )),
            "rgb_pre collection must stop before film exposure"
        );
        let borrowed_context = ObservationContext::new(CollectionMode::Summary);
        let borrowed = runtime
            .process_resident_borrowed_observed(&image, gpu.backend(), &borrowed_context)
            .unwrap()
            .expect("selected GPU must serve borrowed diagnostic taps");
        assert_eq!(
            borrowed.data, actual.data,
            "borrowed diagnostic route changed exposed pixels"
        );
        let borrowed_snapshot = borrowed_context.snapshot();
        assert!(borrowed_snapshot.totals.dispatch_count.unwrap_or(0) > 0);
        assert!(
            !borrowed_snapshot
                .observations
                .iter()
                .any(|stage| matches!(stage.executor, Some(Executor::Cpu | Executor::Mixed))),
            "borrowed diagnostic tap executed host image arithmetic"
        );
        assert!(
            !borrowed_snapshot
                .execution
                .resident_decline_reasons
                .contains(&ResidentDeclineReason::DiagnosticTapRoute)
        );
    }

    #[test]
    fn facade_process_records_stage_timings_and_configured_preview() {
        let dir = data_dir();
        let mut params = RuntimeParams::default();
        params.camera.auto_exposure = false;
        params.settings.preview_max_size = 1;
        let runtime =
            Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", params, &dir).unwrap();
        let backend = CpuBackend;
        let image = ImageBuf::from_data(
            2,
            2,
            (0..12).map(|i| from_f64(0.1 + i as f64 * 0.01)).collect(),
        );
        let output = runtime.process_configured_preview(image, &backend).unwrap();
        assert_eq!((output.width, output.height), (1, 1));
        let timings = runtime.get_timings();
        assert!(timings.contains_key("total"));
        assert!(timings.contains_key("filming_expose"));
        assert!(runtime.get_total_elapsed_time().is_some());
    }
    #[test]
    fn update_refreshes_film_curve_gamma_in_processed_pixels() {
        let dir = data_dir();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.camera.auto_exposure = false;
        params.film_render.grain.active = false;
        let mut runtime =
            Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", params, &dir).unwrap();
        let backend = CpuBackend;
        let image = scanner_image();
        let baseline = runtime.process(image.clone(), &backend).unwrap();
        let mut updated = runtime.params().clone();
        updated.film_render.density_curve_gamma = 2.0;
        runtime.update(updated).unwrap();
        let changed = runtime.process(image, &backend).unwrap();
        assert!(
            baseline
                .data
                .iter()
                .zip(&changed.data)
                .any(|(before, after)| (*before as f64 - *after as f64).abs() > 1e-6),
            "film curve gamma update did not change processed pixels"
        );
    }

    #[test]
    fn amount_only_update_reuses_prepared_fotufilm_bank() {
        let dir = data_dir();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.camera.auto_exposure = false;
        params.film_render.grain.active = true;
        params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
        params.film_render.grain.fotufilm.amount = 1.0;
        let mut runtime =
            Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", params, &dir).unwrap();
        let backend = CpuBackend;
        runtime.process(scanner_image(), &backend).unwrap();
        let first = runtime.pipeline.fotufilm_bank_identity().unwrap();
        let mut updated = runtime.params().clone();
        updated.film_render.grain.fotufilm.amount = 2.0;
        runtime.update(updated).unwrap();
        runtime.process(scanner_image(), &backend).unwrap();
        let second = runtime.pipeline.fotufilm_bank_identity().unwrap();
        assert_eq!(
            first, second,
            "amount-only update rebuilt the Fotufilm bank"
        );
    }
}
