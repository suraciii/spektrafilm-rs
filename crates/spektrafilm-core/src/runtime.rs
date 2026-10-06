//! Stable runtime facade hiding the calibrated pipeline internals.
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
use crate::params::{RuntimeParams, Tap};
use crate::pipeline::Pipeline;
use crate::profile;

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
#[derive(Clone)]
pub struct Runtime {
    pipeline: Pipeline,
    data_dir: std::path::PathBuf,
    timings: Arc<Mutex<Timings>>,
    last_elapsed_seconds: Arc<Mutex<Option<f64>>>,
}

impl Runtime {
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
        })
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

    fn record_elapsed(&self, started: Instant, mut stage_timings: Option<Timings>) {
        let elapsed = started.elapsed().as_secs_f64();
        if let Ok(mut timings) = self.timings.lock() {
            timings.clear();
            if let Some(stage_timings) = stage_timings.as_mut() {
                timings.extend(stage_timings.iter().map(|(name, elapsed)| (name.clone(), *elapsed)));
            }
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
        let started = Instant::now();
        let mut stage_timings = BTreeMap::new();
        let result = self.pipeline.process_with_timings(image, backend, &mut stage_timings);
        self.record_elapsed(started, Some(stage_timings));
        result
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
        );
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
        self.pipeline = Pipeline::new_with_spectral(
            self.pipeline.film.clone(),
            self.pipeline.print.clone(),
            params,
            &self.data_dir,
        )?;
        Ok(())
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
        let mut params = self.pipeline.params.clone();
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
        let mut film = self.pipeline.film.clone();
        let mut print = self.pipeline.print.clone();
        if let Some(curves) = update.film_density_curves {
            film.data.density_curves = curves;
        }
        if let Some(curves) = update.print_density_curves {
            print.data.density_curves = curves;
        }
        self.pipeline = Pipeline::new_with_spectral(film, print, params, &self.data_dir)?;
        Ok(())
    }

    pub fn params(&self) -> &RuntimeParams {
        &self.pipeline.params
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn get_timings(&self) -> Timings {
        self.timings.lock().map(|timings| timings.clone()).unwrap_or_default()
    }

    pub fn get_total_elapsed_time(&self) -> Option<f64> {
        self.last_elapsed_seconds.lock().ok().and_then(|elapsed| *elapsed)
    }

    pub fn format_timings(&self) -> String {
        let mut output = String::from("Simulation timings\n");
        match self.get_total_elapsed_time() {
            Some(total) => {
                output.push_str(&format!("  Total  {:.3} ms  100.0%\n", total * 1000.0));
                for (label, elapsed) in self.get_timings() {
                    if label != "total" {
                        let percentage = if total > 0.0 { elapsed / total * 100.0 } else { 0.0 };
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
    Ok(photo_params(film, print, RuntimeParams::default(), data_dir))
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
        let neutral = crate::neutral_filters::NeutralFilters::load(&photo.data_dir);
        crate::params_builder::digest_params(
            photo.params.clone(),
            &photo.film,
            &photo.print,
            Some(&neutral),
            true,
        )
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
    let preview = crate::params_builder::resize_for_preview(
        &image,
        photo.params.settings.preview_max_size,
    );
    simulate(
        preview,
        photo,
        backend,
        digest_params_first,
        print_timings,
    )
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
    fn facade_process_records_stage_timings_and_configured_preview() {
        let dir = data_dir();
        let mut params = RuntimeParams::default();
        params.camera.auto_exposure = false;
        params.settings.preview_max_size = 1;
        let runtime = Runtime::from_stocks(
            "kodak_portra_400",
            "kodak_portra_endura",
            params,
            &dir,
        )
        .unwrap();
        let backend = CpuBackend;
        let image = ImageBuf::from_data(
            2,
            2,
            (0..12)
                .map(|i| from_f64(0.1 + i as f64 * 0.01))
                .collect(),
        );
        let output = runtime.process_configured_preview(image, &backend).unwrap();
        assert_eq!((output.width, output.height), (1, 1));
        let timings = runtime.get_timings();
        assert!(timings.contains_key("total"));
        assert!(timings.contains_key("filming_expose"));
        assert!(runtime.get_total_elapsed_time().is_some());
    }
}
