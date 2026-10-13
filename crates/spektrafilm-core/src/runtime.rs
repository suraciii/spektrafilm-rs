//! Stable runtime facade hiding the calibrated pipeline internals.
use crate::params::{RuntimeParams, SimulationEngine, Tap};
use crate::pipeline::Pipeline;
use crate::profile;
pub use crate::stages::scanning::ScannerSetup;
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_gpu::telemetry::{ObservationContext, ObservationKind, Purpose};
use spektrafilm_math::image::ImageBuf;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Last-call wall-clock timings exposed by the runtime facade.
pub type Timings = BTreeMap<String, f64>;
/// Parameters accepted by the upstream-style selective soft-update path.
#[derive(Debug, Clone, Default)]
pub struct SoftUpdate {
    pub exposure_compensation_ev: Option<f32>,
    pub print_exposure: Option<f32>,
    pub c_filter_neutral: Option<f32>,
    pub m_filter_neutral: Option<f32>,
    pub y_filter_neutral: Option<f32>,
    pub film_density_curves: Option<Vec<Vec<f64>>>,
    pub print_density_curves: Option<Vec<Vec<f64>>>,
}
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
    if params.engine == SimulationEngine::Fotufilm {
        return params;
    }
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
        if self.params.engine == SimulationEngine::Fotufilm {
            return Ok(self.params.clone());
        }
        let neutral = crate::neutral_filters::NeutralFilters::load(&self.data_dir)?;
        Ok(self.digested_params_with_neutral(mode, &neutral))
    }

    /// Build a calibrated runtime after applying the selected digest policy.
    pub fn into_runtime(self, mode: DigestMode) -> Result<Runtime, String> {
        if self.params.engine == SimulationEngine::Fotufilm {
            return Runtime::from_params(self.params, &self.data_dir);
        }
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
            if self.params.engine == SimulationEngine::Fotufilm {
                "fotufilm_source_prepare"
            } else {
                "spectral_prepare"
            },
            ObservationKind::Stage,
            Purpose::Calibration,
        );
        preparation.context().record_executor(
            spektrafilm_gpu::telemetry::Executor::Cpu,
            Some(spektrafilm_gpu::telemetry::CpuReason::BackendDefault),
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
    pipeline: Option<Pipeline>,
    fotufilm: Option<crate::fotufilm_engine::FotufilmEngine>,
    data_dir: std::path::PathBuf,
    timings: Arc<Mutex<Timings>>,
    last_elapsed_seconds: Arc<Mutex<Option<f64>>>,
    cancellation: Option<Arc<AtomicBool>>,
}

impl Runtime {
    /// Bind a caller-owned cancellation signal while retaining prepared film grain caches.
    pub fn with_cancellation(mut self, cancellation: Option<Arc<AtomicBool>>) -> Self {
        self.cancellation = cancellation.clone();
        self.pipeline = self
            .pipeline
            .map(|pipeline| pipeline.with_cancellation(cancellation));
        self
    }
    pub fn new(
        film: profile::Profile,
        print: profile::Profile,
        params: RuntimeParams,
        data_dir: &Path,
    ) -> Result<Self, String> {
        if params.engine == SimulationEngine::Fotufilm {
            return Self::from_params(params, data_dir);
        }
        let pipeline = Pipeline::new_with_spectral(film, print, params, data_dir)?;
        Ok(Self {
            pipeline: Some(pipeline),
            fotufilm: None,
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
            if params.engine == SimulationEngine::Fotufilm {
                "fotufilm_source_prepare"
            } else {
                "spectral_prepare"
            },
            ObservationKind::Stage,
            Purpose::Calibration,
        );
        preparation.context().record_executor(
            spektrafilm_gpu::telemetry::Executor::Cpu,
            Some(spektrafilm_gpu::telemetry::CpuReason::BackendDefault),
        );
        let _span = preparation.context().span().map(|span| span.enter());
        let result = Self::new(film, print, params, data_dir);
        drop(_span);
        preparation.set_complete(result.is_ok());
        drop(preparation);
        result
    }

    /// Construct the original engine without loading historical spectral profiles.
    pub fn from_params(params: RuntimeParams, data_dir: &Path) -> Result<Self, String> {
        if params.engine != SimulationEngine::Fotufilm {
            return Err("SpektraFilm requires explicit film and print profiles".into());
        }
        Ok(Self {
            pipeline: None,
            fotufilm: Some(crate::fotufilm_engine::FotufilmEngine::new(
                params, data_dir,
            )?),
            data_dir: data_dir.to_owned(),
            timings: Arc::new(Mutex::new(BTreeMap::new())),
            last_elapsed_seconds: Arc::new(Mutex::new(None)),
            cancellation: None,
        })
    }

    pub fn from_resolved(
        resolved: crate::process_params::ResolvedProcessParams,
        data_dir: &Path,
    ) -> Result<Self, String> {
        if resolved.params.engine == SimulationEngine::Fotufilm {
            return Self::from_params(resolved.params, data_dir);
        }
        Self::new(
            resolved.film.ok_or("missing SpektraFilm film profile")?,
            resolved.print.ok_or("missing SpektraFilm print profile")?,
            resolved.params,
            data_dir,
        )
    }

    pub fn from_stocks(
        film: &str,
        print: &str,
        params: RuntimeParams,
        data_dir: &Path,
    ) -> Result<Self, String> {
        if params.engine == SimulationEngine::Fotufilm {
            return Self::from_params(params, data_dir);
        }
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

    /// Measure scanner placement on the full input before any preview geometry.
    pub fn prepare_scanner_setup(&self, full_input: &ImageBuf) -> Result<ScannerSetup, String> {
        match &self.pipeline {
            Some(pipeline) => pipeline.prepare_scanner_setup(full_input),
            None => Ok(ScannerSetup::IDENTITY),
        }
    }

    /// Attach the immutable setup shared by renders of one source revision.
    pub fn with_scanner_setup(mut self, setup: ScannerSetup) -> Self {
        self.pipeline = self
            .pipeline
            .map(|pipeline| pipeline.with_scanner_setup(setup));
        self
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
        let bound = spektrafilm_gpu::bind_backend(backend, context.clone());
        self.process_with_taps(image, bound.as_ref(), inject, collect)
    }

    /// Keep the borrowed resident route available to operation-owned GUI work.
    pub fn process_resident_borrowed_observed(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
        context: &ObservationContext,
    ) -> Result<Option<ImageBuf>, String> {
        let Some(pipeline) = &self.pipeline else {
            return Ok(None);
        };
        if !context.enabled() {
            return pipeline.process_resident_borrowed(image, backend);
        }
        let _span = context.span().map(|span| span.enter());
        let bound = spektrafilm_gpu::bind_backend(backend, context.clone());
        pipeline.process_resident_borrowed(image, bound.as_ref())
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
        let result = if let Some(engine) = &self.fotufilm {
            engine.process(
                image,
                backend,
                inject,
                collect,
                self.cancellation.as_deref(),
            )
        } else {
            self.pipeline
                .as_ref()
                .expect("selected legacy pipeline")
                .process_with_taps_timed(image, backend, inject, collect, Some(&mut stage_timings))
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
        if self.fotufilm.is_some() {
            let output = self.process(image, backend)?;
            if self
                .params()
                .fotufilm
                .native
                .get("stage")
                .and_then(serde_json::Value::as_str)
                == Some("negative")
                || self.params().taps.collect.as_deref() == Some("cmy_film")
            {
                return Ok(output);
            }
            return Ok(crate::params_builder::resize_for_preview(&output, max_size));
        }
        if self
            .pipeline
            .as_ref()
            .expect("selected legacy pipeline")
            .has_scanner_setup()
        {
            let preview = crate::params_builder::resize_for_preview(&image, max_size);
            return self.process(preview, backend);
        }
        let setup = self.prepare_scanner_setup(&image)?;
        let prepared = self.clone().with_scanner_setup(setup);
        let preview = crate::params_builder::resize_for_preview(&image, max_size);
        prepared.process(preview, backend)
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
        if params.engine == SimulationEngine::Fotufilm {
            self.fotufilm = Some(crate::fotufilm_engine::FotufilmEngine::new(
                params,
                &self.data_dir,
            )?);
            self.pipeline = None;
            return Ok(());
        }
        if self.pipeline.is_none() {
            return Err("switching to SpektraFilm requires explicit film and print profiles; construct a runtime from stocks".into());
        }
        // Film-curve controls are absent from the spectral and calibration
        // control keys, so refresh the resolved film unconditionally and keep
        // the prepared grain cache and cancellation binding across the rebuild.
        self.pipeline = Some(
            self.pipeline
                .as_ref()
                .expect("selected legacy pipeline")
                .clone()
                .rebuild_spectral(params)?,
        );
        self.fotufilm = None;
        Ok(())
    }
    /// Return a runtime with updated parameters while retaining the facade state.
    pub fn with_params(mut self, params: RuntimeParams) -> Result<Self, String> {
        self.update(params)?;
        Ok(self)
    }

    pub fn soft_update(&mut self, params: RuntimeParams) -> Result<(), String> {
        self.update(params)
    }
    /// Apply only the controls permitted by upstream `soft_update`.
    ///
    /// Rebuilding the owned pipeline is deliberate: print-filter and density
    /// changes affect calibration state, so retaining derived values would
    /// produce stale output.
    pub fn soft_update_fields(&mut self, update: SoftUpdate) -> Result<(), String> {
        if self.fotufilm.is_some() {
            return Err("Fotufilm soft updates use fotufilm.controls/native via Runtime::update; legacy density and enlarger controls do not apply".into());
        }
        if update.film_density_curves.is_none() && update.print_density_curves.is_none() {
            let mut params = self.params().clone();
            if let Some(value) = update.exposure_compensation_ev {
                params.camera.exposure_compensation_ev = value;
            }
            if let Some(value) = update.print_exposure {
                params.enlarger.print_exposure = value;
            }
            if let Some(value) = update.c_filter_neutral {
                params.enlarger.c_filter_neutral = value;
            }
            if let Some(value) = update.m_filter_neutral {
                params.enlarger.m_filter_neutral = value;
            }
            if let Some(value) = update.y_filter_neutral {
                params.enlarger.y_filter_neutral = value;
            }
            return self.update(params);
        }
        let mut params = self.params().clone();
        if let Some(value) = update.exposure_compensation_ev {
            params.camera.exposure_compensation_ev = value;
        }
        if let Some(value) = update.print_exposure {
            params.enlarger.print_exposure = value;
        }
        if let Some(value) = update.c_filter_neutral {
            params.enlarger.c_filter_neutral = value;
        }
        if let Some(value) = update.m_filter_neutral {
            params.enlarger.m_filter_neutral = value;
        }
        if let Some(value) = update.y_filter_neutral {
            params.enlarger.y_filter_neutral = value;
        }
        params.validate()?;
        let mut film = self
            .pipeline
            .as_ref()
            .expect("selected legacy pipeline")
            .film
            .clone();
        let mut print = self
            .pipeline
            .as_ref()
            .expect("selected legacy pipeline")
            .print
            .clone();
        if let Some(curves) = update.film_density_curves {
            film.data.density_curves = curves;
        }
        if let Some(curves) = update.print_density_curves {
            print.data.density_curves = curves;
        }
        self.pipeline = Some(
            Pipeline::new_with_spectral(film, print, params, &self.data_dir)?
                .with_cancellation(self.cancellation.clone()),
        );
        Ok(())
    }

    pub fn params(&self) -> &RuntimeParams {
        match &self.fotufilm {
            Some(engine) => &engine.params,
            None => {
                &self
                    .pipeline
                    .as_ref()
                    .expect("selected legacy pipeline")
                    .params
            }
        }
    }

    pub fn telemetry_configuration(&self) -> crate::telemetry::RenderConfiguration {
        if self.fotufilm.is_some() {
            return crate::telemetry::RenderConfiguration::from_fotufilm(self.params());
        }
        let pipeline = self.pipeline.as_ref().expect("selected legacy pipeline");
        crate::telemetry::RenderConfiguration::from_runtime(
            &pipeline.params,
            &pipeline.film,
            &pipeline.print,
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
    if photo.params.engine == SimulationEngine::Fotufilm {
        let runtime = photo.clone().into_runtime(if digest_params_first {
            DigestMode::ApplyStockSpecifics
        } else {
            DigestMode::PreserveUserEdits
        })?;
        let output = runtime.process_configured_preview(image, backend);
        if print_timings {
            runtime.print_timings();
        }
        return output;
    }
    if photo.params.scanner.auto_setup == crate::params::ScannerAutoSetup::Off {
        let preview = crate::params_builder::resize_for_preview(
            &image,
            photo.params.settings.preview_max_size,
        );
        return simulate(preview, photo, backend, digest_params_first, print_timings);
    }
    let mut canonical = photo.clone();
    canonical.params.settings.preview_mode = false;
    let mode = if digest_params_first {
        DigestMode::ApplyStockSpecifics
    } else {
        DigestMode::PreserveUserEdits
    };
    let measurement = if digest_params_first {
        canonical.into_runtime(mode)?
    } else {
        Runtime::from_photo_params(canonical)?
    };
    let setup = measurement.prepare_scanner_setup(&image)?;
    let params = if digest_params_first {
        photo.digested_params(mode)?
    } else {
        photo.params.clone()
    };
    let runtime = Runtime::new(
        photo.film.clone(),
        photo.print.clone(),
        params,
        &photo.data_dir,
    )?
    .with_scanner_setup(setup);
    let preview =
        crate::params_builder::resize_for_preview(&image, photo.params.settings.preview_max_size);
    let result = runtime.process(preview, backend);
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
    use spektrafilm_gpu::cpu_backend::CpuBackend;
    use spektrafilm_math::precision::from_f64;

    fn data_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data")
    }

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
        params.scanner.auto_setup = crate::params::ScannerAutoSetup::PerFrame;
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
    fn preview_preserves_attached_scanner_setup() {
        let mut params = scanner_params();
        params.settings.preview_max_size = 2;
        let runtime = Runtime::from_stocks(
            "kodak_portra_400",
            "kodak_portra_endura",
            params,
            &data_dir(),
        )
        .unwrap();
        let backend = CpuBackend;
        let image = scanner_image();
        let mut setup = runtime.prepare_scanner_setup(&image).unwrap();
        setup.density_ev += 2.0;
        setup.balance_ev = [0.5, 0.0, -0.5];
        let attached = runtime.clone().with_scanner_setup(setup);
        let resized = crate::params_builder::resize_for_preview(&image, 2);
        let expected = attached.process(resized, &backend).unwrap();
        let remeasured = runtime.process_preview(image.clone(), 2, &backend).unwrap();
        assert!(
            expected
                .data
                .iter()
                .zip(&remeasured.data)
                .any(|(a, b)| { (*a as f64 - *b as f64).abs() > 1e-3 })
        );
        let preview = attached
            .process_preview(image.clone(), 2, &backend)
            .unwrap();
        let configured = attached
            .process_configured_preview(image, &backend)
            .unwrap();
        assert_eq!((preview.width, preview.height), (2, 2));
        assert_eq!(preview.data, expected.data);
        assert_eq!(configured.data, expected.data);
    }

    #[test]
    fn per_frame_setup_accepts_small_valid_positive_density() {
        let mut params = scanner_params();
        params.scanner.positive_dmax = 1e-18;
        let runtime = Runtime::from_stocks(
            "kodak_portra_400",
            "kodak_portra_endura",
            params,
            &data_dir(),
        )
        .unwrap();
        let backend = CpuBackend;
        let image = scanner_image();
        let setup = runtime.prepare_scanner_setup(&image).unwrap();
        assert!(setup.density_ev.is_finite());
        assert!(setup.balance_ev.iter().all(|v| v.is_finite()));
        assert!((0.5..=2.0).contains(&setup.contrast_scale));
        let output = runtime.process(image.clone(), &backend).unwrap();
        let prepared = runtime
            .with_scanner_setup(setup)
            .process(image, &backend)
            .unwrap();
        assert!(output.data.iter().all(|v| v.is_finite()));
        assert!(output.data.windows(3).any(|w| w[0] != w[2]));
        assert_eq!(output.data, prepared.data);
    }

    #[test]
    fn explicit_rgb_input_overrides_persisted_diagnostic_inject_for_setup() {
        let mut params = scanner_params();
        params.taps.inject = Some("log_e_film".into());
        let runtime = Runtime::from_stocks(
            "kodak_portra_400",
            "kodak_portra_endura",
            params,
            &data_dir(),
        )
        .unwrap();
        let backend = CpuBackend;
        let image = scanner_image();
        assert_eq!(
            runtime.process(image.clone(), &backend).unwrap_err(),
            "per_frame scanner setup requires the full rgb_in input"
        );
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
        params.scanner.auto_setup = crate::params::ScannerAutoSetup::Off;
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
        params.scanner.auto_setup = crate::params::ScannerAutoSetup::Off;
        params.film_render.grain.active = true;
        params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
        params.film_render.grain.fotufilm.amount = 1.0;
        let mut runtime =
            Runtime::from_stocks("kodak_portra_400", "kodak_portra_endura", params, &dir).unwrap();
        let backend = CpuBackend;
        runtime.process(scanner_image(), &backend).unwrap();
        let first = runtime
            .pipeline
            .as_ref()
            .unwrap()
            .fotufilm_bank_identity()
            .unwrap();
        let mut updated = runtime.params().clone();
        updated.film_render.grain.fotufilm.amount = 2.0;
        runtime.update(updated).unwrap();
        runtime.process(scanner_image(), &backend).unwrap();
        let second = runtime
            .pipeline
            .as_ref()
            .unwrap()
            .fotufilm_bank_identity()
            .unwrap();
        assert_eq!(
            first, second,
            "amount-only update rebuilt the Fotufilm bank"
        );
    }
}
