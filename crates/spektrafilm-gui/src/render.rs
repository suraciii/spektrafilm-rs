//! GUI render snapshots and runtime preparation shared by Preview, Scan, and Export.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use spektrafilm_backend::telemetry::{ObservationContext, ObservationKind, Purpose};
use spektrafilm_core::params::RuntimeParams;
use spektrafilm_core::profile;
use spektrafilm_core::runtime::{DigestMode, Runtime};
use spektrafilm_core::telemetry::{IssueBoundary, IssueCategory};

use super::observed_result;

#[derive(Clone)]
pub(super) struct RenderSnapshot {
    params: RuntimeParams,
    film_name: String,
    print_name: String,
    data_dir: PathBuf,
    special: serde_json::Value,
    cancellation: Option<Arc<AtomicBool>>,
    operator_compare: bool,
}

impl RenderSnapshot {
    pub(super) fn new(
        params: RuntimeParams,
        film_name: String,
        print_name: String,
        data_dir: PathBuf,
        special: serde_json::Value,
    ) -> Self {
        Self {
            params,
            film_name,
            print_name,
            data_dir,
            special,
            cancellation: None,
            operator_compare: false,
        }
    }

    pub(crate) fn export_dependencies(
        &self,
    ) -> anyhow::Result<Vec<crate::photograph::record::Dependency>> {
        let mut special = self.special.clone();
        for name in ["film_channel_swap", "print_channel_swap"] {
            if special.get(name).is_none() {
                special[name] = serde_json::json!([0, 1, 2]);
            }
        }
        let settings = crate::photograph::record::PhotographSettings {
            params: self.params.clone(),
            preparation: self.params.render_preparation_state(),
            film: self.film_name.clone(),
            print: self.print_name.clone(),
            special,
            raw: crate::state::GuiState::factory().sections["load_raw"].clone(),
            rotation: 0,
            applied: None,
        };
        crate::photograph::record::dependencies(&settings, &self.data_dir)
    }

    /// Bind the owner's cancellation token to runtimes built from this snapshot.
    pub(super) fn with_cancellation(mut self, cancellation: Option<Arc<AtomicBool>>) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub(super) fn cancellation(&self) -> Option<Arc<AtomicBool>> {
        self.cancellation.clone()
    }

    pub(super) fn with_operator_compare(mut self, comparing: bool) -> Self {
        self.operator_compare = comparing;
        if comparing {
            self.params.scanner.operator = Default::default();
        }
        self
    }

    pub(super) fn operator_compare(&self) -> bool {
        self.operator_compare
    }

    #[cfg(test)]
    pub(super) fn params(&self) -> &RuntimeParams {
        &self.params
    }

    fn digested_params(
        &self,
        preview: bool,
        film: &profile::Profile,
        print: &profile::Profile,
        neutral: &spektrafilm_core::neutral_filters::NeutralFilters,
    ) -> RuntimeParams {
        let mut params = self.params.clone();
        if !params.scanner.requires_full_frame() {
            params.scanner.operator = Default::default();
        }
        params.settings.preview_mode = preview && !params.scanner.requires_full_frame();
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
    ) -> Result<Runtime, crate::app::Message> {
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
        Runtime::new_observed(film, print, params, &self.data_dir, context).map_err(|e| {
            crate::app::Message::new("operations.runtime_build_error").arg("detail", e)
        })
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
    pub(super) fn prepare_observed(
        &mut self,
        snapshot: &RenderSnapshot,
        preview: bool,
        context: &ObservationContext,
    ) -> Result<&mut Runtime, crate::app::Message> {
        let prepare = context.scope("runtime_prepare", ObservationKind::Phase, Purpose::Image);
        let result = self.prepare(snapshot, preview, prepare.context());
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
        context: &ObservationContext,
    ) -> Result<&mut Runtime, crate::app::Message> {
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
                cached.runtime = cached.runtime.with_params(digested).map_err(|error| {
                    crate::app::Message::new("operations.runtime_update_error").arg("detail", error)
                })?;
                self.entry = Some(cached);
                return Ok(&mut self.entry.as_mut().expect("runtime cache restored").runtime);
            }
            self.entry = Some(cached);
        }

        let film = profile::load_profile_by_name(&snapshot.data_dir, &snapshot.film_name).map_err(
            |e| {
                crate::app::Message::new("operations.film_profile_error")
                    .arg("name", &snapshot.film_name)
                    .arg("detail", e)
            },
        )?;
        let digest_print = profile::load_profile_by_name(&snapshot.data_dir, &snapshot.print_name)
            .map_err(|e| {
                crate::app::Message::new("operations.print_profile_error")
                    .arg("name", &snapshot.print_name)
                    .arg("detail", e)
            })?;
        let neutral = spektrafilm_core::neutral_filters::NeutralFilters::load(&snapshot.data_dir)
            .map_err(|e| {
            crate::app::Message::new("operations.neutral_filters_error").arg("detail", e)
        })?;
        let digested = snapshot.digested_params(preview, &film, &digest_print, &neutral);
        let runtime_print_name = snapshot.runtime_print_name(&digested);
        let runtime_film = film.clone();
        let runtime_print = if digested.io.scan_film {
            film.clone()
        } else {
            digest_print.clone()
        };
        let runtime = snapshot.build_runtime(runtime_film, runtime_print, digested, context)?;
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
) -> Result<Runtime, crate::app::Message> {
    let prepare = context.scope("runtime_prepare", ObservationKind::Phase, Purpose::Image);
    let result = (|| {
        let film = profile::load_profile_by_name(&snapshot.data_dir, &snapshot.film_name).map_err(
            |e| {
                crate::app::Message::new("operations.film_profile_error")
                    .arg("name", &snapshot.film_name)
                    .arg("detail", e)
            },
        )?;
        let print = profile::load_profile_by_name(&snapshot.data_dir, &snapshot.print_name)
            .map_err(|e| {
                crate::app::Message::new("operations.print_profile_error")
                    .arg("name", &snapshot.print_name)
                    .arg("detail", e)
            })?;
        let neutral = spektrafilm_core::neutral_filters::NeutralFilters::load(&snapshot.data_dir)
            .map_err(|e| {
            crate::app::Message::new("operations.neutral_filters_error").arg("detail", e)
        })?;
        let params = snapshot.digested_params(preview, &film, &print, &neutral);
        let film = profile::load_profile_by_name(&snapshot.data_dir, &snapshot.film_name).map_err(
            |e| {
                crate::app::Message::new("operations.film_profile_error")
                    .arg("name", &snapshot.film_name)
                    .arg("detail", e)
            },
        )?;
        let effective_print_name = snapshot.runtime_print_name(&params);
        let print = profile::load_profile_by_name(&snapshot.data_dir, effective_print_name)
            .map_err(|e| {
                crate::app::Message::new("operations.print_profile_error")
                    .arg("name", effective_print_name)
                    .arg("detail", e)
            })?;
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
    use spektrafilm_core::params::RuntimeParams;
    use spektrafilm_math::image::ImageBuf;

    fn snapshot_and_input() -> (RenderSnapshot, ImageBuf) {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
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
    fn runtime_cache_recovers_from_profile_and_update_failures() {
        let (snapshot, image) = snapshot_and_input();
        let input = &image;
        let backend = CpuBackend;
        let context = ObservationContext::default();
        let mut cache = RuntimeCache::default();
        let expected = cache
            .prepare_observed(&snapshot, false, &context)
            .unwrap()
            .process(input.clone(), &backend)
            .unwrap();

        let missing = tempfile::tempdir().unwrap();
        let mut failed_profile = snapshot.clone();
        failed_profile.data_dir = missing.path().to_owned();
        assert!(
            cache
                .prepare_observed(&failed_profile, false, &context)
                .err()
                .unwrap()
                .to_string()
                .contains("kodak_portra_400")
        );
        assert!(cache.entry.is_some());
        let recovered = cache
            .prepare_observed(&snapshot, false, &context)
            .unwrap()
            .process(input.clone(), &backend)
            .unwrap();
        assert_eq!(recovered.data, expected.data);

        let mut failed_update = snapshot.clone();
        failed_update.params.scanner.conversion_style = "../etc/passwd".into();
        assert!(
            cache
                .prepare_observed(&failed_update, false, &context)
                .err()
                .unwrap()
                .to_string()
                .contains("conversion_style")
        );
        assert!(cache.entry.is_none());
        let rebuilt = cache
            .prepare_observed(&snapshot, false, &context)
            .unwrap()
            .process(input.clone(), &backend)
            .unwrap();
        assert_eq!(rebuilt.data, expected.data);
    }
}
