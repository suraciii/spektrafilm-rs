//! GUI render snapshots and runtime preparation shared by Preview, Scan, and Export.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use spektrafilm_backend::telemetry::{ObservationContext, ObservationKind, Purpose};
use spektrafilm_core::params::{RuntimeParams, ScannerAutoSetup};
use spektrafilm_core::profile;
use spektrafilm_core::runtime::{DigestMode, Runtime, ScannerSetup};
use spektrafilm_core::telemetry::{IssueBoundary, IssueCategory};
use spektrafilm_math::image::ImageBuf;

use super::observed_result;

pub(super) type SharedScannerSetup = Arc<OnceLock<Result<ScannerSetup, String>>>;

#[derive(Clone)]
pub(super) struct RenderSnapshot {
    params: RuntimeParams,
    film_name: String,
    print_name: String,
    data_dir: PathBuf,
    special: serde_json::Value,
    scanner_setup: SharedScannerSetup,
    cancellation: Option<Arc<AtomicBool>>,
}

impl RenderSnapshot {
    pub(super) fn new(
        params: RuntimeParams,
        film_name: String,
        print_name: String,
        data_dir: PathBuf,
        special: serde_json::Value,
        scanner_setup: SharedScannerSetup,
    ) -> Self {
        Self {
            params,
            film_name,
            print_name,
            data_dir,
            special,
            scanner_setup,
            cancellation: None,
        }
    }

    /// Bind the owner's cancellation token to runtimes built from this snapshot.
    pub(super) fn with_cancellation(mut self, cancellation: Option<Arc<AtomicBool>>) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub(super) fn cancellation(&self) -> Option<Arc<AtomicBool>> {
        self.cancellation.clone()
    }

    #[cfg(test)]
    pub(super) fn params(&self) -> &RuntimeParams {
        &self.params
    }

    #[cfg(test)]
    pub(super) fn scanner_setup(&self) -> &SharedScannerSetup {
        &self.scanner_setup
    }

    fn digested_params(
        &self,
        preview: bool,
        film: &profile::Profile,
        print: &profile::Profile,
        neutral: &spektrafilm_core::neutral_filters::NeutralFilters,
    ) -> RuntimeParams {
        let mut params = self.params.clone();
        params.settings.preview_mode = preview;
        spektrafilm_core::runtime::digest_params_with_neutral(
            params,
            film,
            print,
            neutral,
            DigestMode::PreserveUserEdits,
        )
    }

    fn runtime_print_name(&self, params: &RuntimeParams) -> &str {
        if params.io.scan_film {
            &self.film_name
        } else {
            &self.print_name
        }
    }

    fn build_runtime(
        &self,
        mut film: profile::Profile,
        mut print: profile::Profile,
        params: RuntimeParams,
        context: &ObservationContext,
    ) -> Result<Runtime, String> {
        for (profile, key) in [
            (&mut film, "film_channel_swap"),
            (&mut print, "print_channel_swap"),
        ] {
            if let Some(order) = self.special[key].as_array() {
                for row in &mut profile.data.channel_density {
                    if row.len() >= 3 {
                        let original = [row[0], row[1], row[2]];
                        for ch in 0..3 {
                            row[ch] = original[order[ch].as_u64().unwrap_or(ch as u64) as usize];
                        }
                    }
                }
            }
        }
        Runtime::new_observed(film, print, params, &self.data_dir, context)
            .map_err(|e| format!("runtime build: {e}"))
    }

    pub(super) fn prepare_scanner_setup(
        &self,
        full_input: &ImageBuf,
        context: &ObservationContext,
    ) -> Result<Option<ScannerSetup>, String> {
        if self.params.scanner.auto_setup == ScannerAutoSetup::Off {
            return Ok(None);
        }
        if let Some(setup) = self.scanner_setup.get() {
            return setup.clone().map(Some);
        }
        let canonical = build_runtime_observed(self, false, context)?
            .with_cancellation(self.cancellation.clone());
        let setup = canonical.prepare_scanner_setup(full_input)?;
        let _ = self.scanner_setup.set(Ok(setup));
        Ok(Some(setup))
    }
}
#[derive(Clone, PartialEq)]
struct RuntimeCacheKey {
    data_dir: PathBuf,
    film_name: String,
    digest_print_name: String,
    runtime_print_name: String,
    special: serde_json::Value,
}

#[derive(Default)]
pub(super) struct RuntimeCache {
    entry: Option<RuntimeCacheEntry>,
}

struct RuntimeCacheEntry {
    key: RuntimeCacheKey,
    film: spektrafilm_core::profile::Profile,
    digest_print: spektrafilm_core::profile::Profile,
    neutral: spektrafilm_core::neutral_filters::NeutralFilters,
    runtime: Runtime,
}

impl RuntimeCache {
    #[cfg(test)]
    pub(super) fn prepare_without_setup(
        &mut self,
        snapshot: &RenderSnapshot,
        preview: bool,
        context: &ObservationContext,
    ) -> Result<&mut Runtime, String> {
        self.prepare(snapshot, preview, None, context)
    }

    pub(super) fn prepare_observed(
        &mut self,
        snapshot: &RenderSnapshot,
        preview: bool,
        full_input: &ImageBuf,
        context: &ObservationContext,
    ) -> Result<&mut Runtime, String> {
        let prepare = context.scope("runtime_prepare", ObservationKind::Phase, Purpose::Image);
        let result = snapshot
            .prepare_scanner_setup(full_input, prepare.context())
            .and_then(|setup| self.prepare(snapshot, preview, setup, prepare.context()));
        observed_result(
            prepare,
            result,
            IssueCategory::Configuration,
            IssueBoundary::RuntimePrepare,
        )
    }

    fn prepare(
        &mut self,
        snapshot: &RenderSnapshot,
        preview: bool,
        setup: Option<ScannerSetup>,
        context: &ObservationContext,
    ) -> Result<&mut Runtime, String> {
        if let Some(mut cached) = self.entry.take() {
            let digested = snapshot.digested_params(
                preview,
                &cached.film,
                &cached.digest_print,
                &cached.neutral,
            );
            let runtime_print_name = snapshot.runtime_print_name(&digested);
            let key = RuntimeCacheKey {
                data_dir: snapshot.data_dir.clone(),
                film_name: snapshot.film_name.clone(),
                digest_print_name: snapshot.print_name.clone(),
                runtime_print_name: runtime_print_name.to_owned(),
                special: snapshot.special.clone(),
            };
            if cached.key == key {
                let runtime = cached.runtime.with_params(digested)?;
                cached.runtime = match setup {
                    Some(setup) => runtime.with_scanner_setup(setup),
                    None => runtime,
                };
                self.entry = Some(cached);
                return Ok(&mut self.entry.as_mut().expect("runtime cache restored").runtime);
            }
            self.entry = Some(cached);
        }

        let film = profile::load_profile_by_name(&snapshot.data_dir, &snapshot.film_name)
            .map_err(|e| format!("film profile '{}': {e}", snapshot.film_name))?;
        let digest_print = profile::load_profile_by_name(&snapshot.data_dir, &snapshot.print_name)
            .map_err(|e| format!("print profile '{}': {e}", snapshot.print_name))?;
        let neutral = spektrafilm_core::neutral_filters::NeutralFilters::load(&snapshot.data_dir)
            .map_err(|e| format!("neutral filters: {e}"))?;
        let digested = snapshot.digested_params(preview, &film, &digest_print, &neutral);
        let runtime_print_name = snapshot.runtime_print_name(&digested);
        let runtime_film = film.clone();
        let runtime_print = if digested.io.scan_film {
            film.clone()
        } else {
            digest_print.clone()
        };
        let runtime = snapshot.build_runtime(runtime_film, runtime_print, digested, context)?;
        let runtime = match setup {
            Some(setup) => runtime.with_scanner_setup(setup),
            None => runtime,
        };
        self.entry = Some(RuntimeCacheEntry {
            key: RuntimeCacheKey {
                data_dir: snapshot.data_dir.clone(),
                film_name: snapshot.film_name.clone(),
                digest_print_name: snapshot.print_name.clone(),
                runtime_print_name: runtime_print_name.to_owned(),
                special: snapshot.special.clone(),
            },
            film,
            digest_print,
            neutral,
            runtime,
        });
        Ok(&mut self.entry.as_mut().expect("runtime cache inserted").runtime)
    }
}

pub(super) fn build_runtime_observed(
    snapshot: &RenderSnapshot,
    preview: bool,
    context: &ObservationContext,
) -> Result<Runtime, String> {
    let prepare = context.scope("runtime_prepare", ObservationKind::Phase, Purpose::Image);
    let result = (|| {
        let film = profile::load_profile_by_name(&snapshot.data_dir, &snapshot.film_name)
            .map_err(|e| format!("film profile '{}': {e}", snapshot.film_name))?;
        let print = profile::load_profile_by_name(&snapshot.data_dir, &snapshot.print_name)
            .map_err(|e| format!("print profile '{}': {e}", snapshot.print_name))?;
        let neutral = spektrafilm_core::neutral_filters::NeutralFilters::load(&snapshot.data_dir)
            .map_err(|e| format!("parameter digest: {e}"))?;
        let params = snapshot.digested_params(preview, &film, &print, &neutral);
        let film = profile::load_profile_by_name(&snapshot.data_dir, &snapshot.film_name)
            .map_err(|e| format!("film profile '{}': {e}", snapshot.film_name))?;
        let effective_print_name = snapshot.runtime_print_name(&params);
        let print = profile::load_profile_by_name(&snapshot.data_dir, effective_print_name)
            .map_err(|e| format!("print profile '{effective_print_name}': {e}"))?;
        snapshot.build_runtime(film, print, params, prepare.context())
    })();
    observed_result(
        prepare,
        result,
        IssueCategory::Configuration,
        IssueBoundary::RuntimePrepare,
    )
}

#[cfg(test)]
mod tests {
    use super::{RenderSnapshot, RuntimeCache};
    use spektrafilm_backend::cpu_backend::CpuBackend;
    use spektrafilm_backend::telemetry::ObservationContext;
    use spektrafilm_core::params::{RuntimeParams, ScannerAutoSetup};
    use spektrafilm_math::image::ImageBuf;
    use std::sync::{Arc, OnceLock};

    fn snapshot_and_input() -> (RenderSnapshot, ImageBuf) {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.auto_setup = ScannerAutoSetup::Off;
        params.scanner.white_correction = false;
        params.scanner.black_correction = false;
        params.film_render.grain.active = false;
        params.camera.auto_exposure = false;
        params.print_render.glare.active = false;
        params.settings.preview_max_size = 4;
        let snapshot = RenderSnapshot::new(
            params,
            "kodak_portra_400".into(),
            "kodak_portra_endura".into(),
            crate::headless::data_dir(),
            serde_json::json!({}),
            Arc::new(OnceLock::new()),
        );
        let input = ImageBuf::from_data(
            24,
            8,
            (0..24 * 8 * 3)
                .map(|i| {
                    spektrafilm_math::precision::from_f64(if i / 3 % 24 < 5 { 0.01 } else { 0.7 })
                })
                .collect(),
        );
        (snapshot, input)
    }

    #[test]
    fn runtime_cache_recovers_from_setup_update_and_profile_failures() {
        let (snapshot, image) = snapshot_and_input();
        let input = &image;
        let backend = CpuBackend;
        let context = ObservationContext::default();
        let mut cache = RuntimeCache::default();
        let expected = cache
            .prepare_observed(&snapshot, false, input, &context)
            .unwrap()
            .process(input.clone(), &backend)
            .unwrap();

        let mut failed_setup = snapshot.clone();
        failed_setup.params.scanner.auto_setup = ScannerAutoSetup::Full;
        failed_setup.scanner_setup = Arc::new(OnceLock::new());
        failed_setup
            .scanner_setup
            .set(Err("scanner setup unavailable".into()))
            .unwrap();
        assert_eq!(
            cache
                .prepare_observed(&failed_setup, false, input, &context)
                .err()
                .unwrap(),
            "scanner setup unavailable"
        );
        assert!(cache.entry.is_some());

        let missing = tempfile::tempdir().unwrap();
        let mut failed_profile = snapshot.clone();
        failed_profile.data_dir = missing.path().to_owned();
        assert!(
            cache
                .prepare_observed(&failed_profile, false, input, &context)
                .err()
                .unwrap()
                .contains("film profile 'kodak_portra_400'")
        );
        assert!(cache.entry.is_some());
        let recovered = cache
            .prepare_observed(&snapshot, false, input, &context)
            .unwrap()
            .process(input.clone(), &backend)
            .unwrap();
        assert_eq!(recovered.data, expected.data);

        let mut failed_update = snapshot.clone();
        failed_update.params.scanner.conversion_brightness_ev = f64::NAN;
        assert!(
            cache
                .prepare_observed(&failed_update, false, input, &context)
                .err()
                .unwrap()
                .contains("scanner.conversion_brightness_ev")
        );
        assert!(cache.entry.is_none());
        let rebuilt = cache
            .prepare_observed(&snapshot, false, input, &context)
            .unwrap()
            .process(input.clone(), &backend)
            .unwrap();
        assert_eq!(rebuilt.data, expected.data);
    }
}
