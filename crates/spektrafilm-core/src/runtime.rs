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
/// `conversion_language_v1` renames the conversion-control paths
/// (`enlarger.print_exposure_mode`, `settings.print_exposure_ev_tracking`)
/// without changing rendered pixels.
pub const MODEL_VERSION: &str = "conversion_language_v1";

/// Last-call wall-clock timings exposed by the runtime facade.
pub type Timings = BTreeMap<String, f64>;
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
}

#[derive(Clone)]
pub struct Runtime {
    pipeline: Pipeline,
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
    pub fn new(
        film: profile::Profile,
        print: profile::Profile,
        params: RuntimeParams,
        data_dir: &Path,
    ) -> Result<Self, String> {
        let pipeline = Pipeline::new_with_spectral(film, print, params, data_dir)?;
        Ok(Self {
            pipeline,
            data_dir: data_dir.to_owned(),
            timings: Arc::new(Mutex::new(BTreeMap::new())),
            last_elapsed_seconds: Arc::new(Mutex::new(None)),
            cancellation: None,
        })
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

    pub fn process_preview(
        &self,
        image: ImageBuf,
        max_size: u32,
        backend: &dyn ComputeBackend,
    ) -> Result<ImageBuf, String> {
        let preview = crate::params_builder::resize_for_preview(&image, max_size);
        self.process(preview, backend)
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
        self.pipeline = self.pipeline.clone().rebuild_spectral(params)?;
        Ok(())
    }
    /// Return a runtime with updated parameters while retaining the facade state.
    pub fn with_params(mut self, params: RuntimeParams) -> Result<Self, String> {
        self.pipeline = self.pipeline.with_params(params)?;
        Ok(self)
    }

    pub fn params(&self) -> &RuntimeParams {
        &self.pipeline.params
    }

    pub fn telemetry_configuration(&self) -> crate::telemetry::RenderConfiguration {
        crate::telemetry::RenderConfiguration::from_runtime(
            &self.pipeline.params,
            &self.pipeline.film,
            &self.pipeline.print,
        )
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
    let runtime = Runtime::new(
        photo.film.clone(),
        photo.print.clone(),
        params,
        &photo.data_dir,
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
    let preview =
        crate::params_builder::resize_for_preview(&image, photo.params.settings.preview_max_size);
    simulate(preview, photo, backend, digest_params_first, print_timings)
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
