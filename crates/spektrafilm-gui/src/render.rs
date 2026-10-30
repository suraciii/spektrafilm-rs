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

/// Loaded source samples stay in their original domain until a consumer
/// explicitly requests a CPU reference or diagnostic materialization.
#[derive(Clone, Debug)]
pub(crate) enum InputImage {
    Host(Arc<spektrafilm_math::image::ImageBuf>),
    Device(spektrafilm_backend::DeviceImage),
}

impl InputImage {
    pub(crate) fn width(&self) -> u32 {
        match self {
            Self::Host(image) => image.width,
            Self::Device(image) => image.width(),
        }
    }

    pub(crate) fn height(&self) -> u32 {
        match self {
            Self::Host(image) => image.height,
            Self::Device(image) => image.height(),
        }
    }

    pub(crate) fn pixel_count(&self) -> usize {
        self.width() as usize * self.height() as usize
    }

    pub(crate) fn host_image(&self) -> Option<&Arc<spektrafilm_math::image::ImageBuf>> {
        match self {
            Self::Host(image) => Some(image),
            Self::Device(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn same_source(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Host(a), Self::Host(b)) => Arc::ptr_eq(a, b),
            (Self::Device(a), Self::Device(b)) => match (a.payload(), b.payload()) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            },
            _ => false,
        }
    }

    /// Only CPU reference processing and intentional host diagnostic actions
    /// may materialize a loaded device source.
    pub(crate) fn to_host(
        &self,
        backend: &dyn spektrafilm_backend::ComputeBackend,
    ) -> Result<spektrafilm_math::image::ImageBuf, String> {
        match self {
            Self::Host(image) => Ok((**image).clone()),
            Self::Device(image) => image.to_host(backend),
        }
    }

    /// Bind accepted controls to this consumer without converting or mutating
    /// the stored samples. CPU processing interprets the same samples directly.
    pub(crate) fn interpreted(
        &self,
        input_color_space: &str,
        input_cctf_decoding: bool,
    ) -> Result<Self, String> {
        match self {
            Self::Host(_) => Ok(self.clone()),
            Self::Device(image) => {
                let mut meta = image.meta();
                meta.space = spektrafilm_math::colorspace::resolve(input_color_space)?;
                meta.domain = if input_cctf_decoding {
                    spektrafilm_backend::SampleDomain::Encoded
                } else {
                    spektrafilm_backend::SampleDomain::Linear
                };
                Ok(Self::Device(image.with_meta(meta)))
            }
        }
    }

    pub(crate) fn rotated(
        &self,
        quarter_turns: i32,
        backend: &dyn spektrafilm_backend::ComputeBackend,
        input_color_space: &str,
        input_cctf_decoding: bool,
    ) -> Result<Self, String> {
        let interpreted = self.interpreted(input_color_space, input_cctf_decoding)?;
        let turns = quarter_turns.rem_euclid(4);
        if turns == 0 {
            return Ok(interpreted);
        }
        let retained;
        let image = match &interpreted {
            Self::Host(image) if backend.is_gpu() => {
                retained = backend.retain_image(
                    image,
                    &spektrafilm_backend::RetainedImageMeta {
                        width: image.width,
                        height: image.height,
                        domain: if input_cctf_decoding {
                            spektrafilm_backend::SampleDomain::Encoded
                        } else {
                            spektrafilm_backend::SampleDomain::Linear
                        },
                        precision: spektrafilm_backend::Precision::host(),
                        space: spektrafilm_math::colorspace::resolve(input_color_space)?,
                        pixel_pitch_um: None,
                    },
                )?;
                &retained
            }
            Self::Host(image) => {
                return Ok(Self::Host(Arc::new(image.rotated_quarter_turns(turns))));
            }
            Self::Device(image) => image,
        };
        let geometry = spektrafilm_backend::InputGeometryGpuParams {
            rotation_quarter_turns: turns as u32,
            crop: None,
            resize: None,
            pixel_pitch_um: image.pixel_pitch_um().unwrap_or(1.0),
        };
        let rotated = backend
            .input_geometry(image, &geometry)?
            .ok_or_else(|| "selected GPU does not support retained input orientation".to_owned())?;
        // Presentation rotation must preserve source colour/domain and
        // an unknown pitch rather than manufacture physical metadata.
        let mut meta = image.meta();
        meta.width = rotated.width();
        meta.height = rotated.height();
        Ok(Self::Device(rotated.with_meta(meta)))
    }
}

/// A transient display-only bypass. Only one module can be compared at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Comparison {
    #[default]
    None,
    Operator,
    Finishing,
}

#[derive(Clone)]
pub(super) struct RenderSnapshot {
    params: RuntimeParams,
    film_name: String,
    print_name: String,
    data_dir: PathBuf,
    special: serde_json::Value,
    cancellation: Option<Arc<AtomicBool>>,
    comparison: Comparison,
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
            comparison: Comparison::None,
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

    pub(super) fn with_comparison(mut self, comparison: Comparison) -> Self {
        self.comparison = comparison;
        match comparison {
            Comparison::None => {}
            Comparison::Operator => self.params.scanner.operator = Default::default(),
            Comparison::Finishing => self.params.finishing = Default::default(),
        }
        self
    }

    pub(super) fn comparison(&self) -> Comparison {
        self.comparison
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
        if !params.scanner.operator_eligible() {
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
        backend: &dyn spektrafilm_backend::ComputeBackend,
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
        Runtime::new_observed_with_backend(film, print, params, &self.data_dir, backend, context)
            .map_err(|e| {
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
    preparation_device: Option<u64>,
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
    pub(super) fn prepare_observed(
        &mut self,
        snapshot: &RenderSnapshot,
        preview: bool,
        context: &ObservationContext,
    ) -> Result<&mut Runtime, crate::app::Message> {
        self.prepare_observed_with_backend(
            snapshot,
            preview,
            context,
            &spektrafilm_backend::cpu_backend::CpuBackend,
        )
    }

    pub(super) fn prepare_observed_with_backend(
        &mut self,
        snapshot: &RenderSnapshot,
        preview: bool,
        context: &ObservationContext,
        backend: &dyn spektrafilm_backend::ComputeBackend,
    ) -> Result<&mut Runtime, crate::app::Message> {
        let cancellable = backend.with_cancellation(snapshot.cancellation());
        let backend = cancellable.as_deref().unwrap_or(backend);
        let prepare = context.scope("runtime_prepare", ObservationKind::Phase, Purpose::Image);
        let result = self.prepare(snapshot, preview, prepare.context(), backend);
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
        backend: &dyn spektrafilm_backend::ComputeBackend,
    ) -> Result<&mut Runtime, crate::app::Message> {
        let cached_update = self.entry.as_ref().and_then(|cached| {
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
                preparation_device: backend.preparation_device_id(),
            };
            (cached.key == key).then_some(digested)
        });
        if let Some(digested) = cached_update {
            let cached = self.entry.as_mut().expect("matching runtime cache entry");
            cached
                .runtime
                .update_with_backend(digested, backend)
                .map_err(|error| {
                    crate::app::Message::new("operations.runtime_update_error").arg("detail", error)
                })?;
            return Ok(&mut cached.runtime);
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
        let runtime =
            snapshot.build_runtime(runtime_film, runtime_print, digested, context, backend)?;
        self.entry = Some(RuntimeCacheEntry {
            key: RuntimeCacheKey {
                data_dir: snapshot.data_dir.clone(),
                film_name: snapshot.film_name.clone(),
                digest_print_name: snapshot.print_name.clone(),
                runtime_print_name: runtime_print_name.to_owned(),
                special: snapshot.special.clone(),
                preparation_device: backend.preparation_device_id(),
            },
            film,
            digest_print,
            neutral,
            runtime,
        });
        Ok(&mut self.entry.as_mut().expect("runtime cache inserted").runtime)
    }
}

#[cfg(test)]
pub(super) fn build_runtime_observed(
    snapshot: &RenderSnapshot,
    preview: bool,
    context: &ObservationContext,
) -> Result<Runtime, crate::app::Message> {
    build_runtime_observed_with_backend(
        snapshot,
        preview,
        context,
        &spektrafilm_backend::cpu_backend::CpuBackend,
    )
}

pub(super) fn build_runtime_observed_with_backend(
    snapshot: &RenderSnapshot,
    preview: bool,
    context: &ObservationContext,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> Result<Runtime, crate::app::Message> {
    let cancellable = backend.with_cancellation(snapshot.cancellation());
    let backend = cancellable.as_deref().unwrap_or(backend);
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
        snapshot.build_runtime(film, print, params, prepare.context(), backend)
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
    use crate::render::Comparison;

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
    fn comparison_neutralizes_only_the_selected_module_in_its_snapshot() {
        let (mut snapshot, _) = snapshot_and_input();
        snapshot.params.scanner.operator.exposure_ev = 0.75;
        snapshot.params.finishing.brightness = 0.5;
        snapshot.params.finishing.expand.enabled = true;
        snapshot.params.finishing.expand.black = -0.1;
        let accepted = snapshot.params.clone();

        let finishing = snapshot.clone().with_comparison(Comparison::Finishing);
        assert_eq!(finishing.params.finishing, Default::default());
        assert_eq!(finishing.params.scanner, accepted.scanner);
        assert_eq!(finishing.params.random_seed, accepted.random_seed);
        let operator = snapshot.clone().with_comparison(Comparison::Operator);
        assert_eq!(operator.params.scanner.operator, Default::default());
        assert_eq!(operator.params.finishing, accepted.finishing);
        assert_eq!(snapshot.params, accepted);
    }

    #[test]
    fn operator_correction_survives_the_frame_independent_default() {
        // The default readout runs no frame setup, so the render path must keep
        // manual corrections; gating them on the setup flag silently zeroed
        // every saved correction in the default configuration.
        let (mut snapshot, _image) = snapshot_and_input();
        snapshot.params.scanner.operator.exposure_ev = 0.5;
        assert!(!snapshot.params.scanner.experimental_setup);
        assert!(snapshot.params.scanner.operator_eligible());
        let corrected = RuntimeCache::default()
            .prepare_observed(&snapshot, false, &ObservationContext::default())
            .unwrap()
            .params()
            .scanner
            .operator;
        assert_eq!(corrected.exposure_ev, 0.5);

        // An ineligible route rejects correction, so the path drops it.
        let mut fixed = snapshot.clone();
        fixed.params.scanner.scan_output = "direct_scan".into();
        assert!(!fixed.params.scanner.operator_eligible());
        let dropped = RuntimeCache::default()
            .prepare_observed(&fixed, false, &ObservationContext::default())
            .unwrap()
            .params()
            .scanner
            .operator;
        assert_eq!(dropped, Default::default());
    }

    #[test]
    fn runtime_cache_recovers_from_profile_and_update_failures() {
        fn link_tree(source: &std::path::Path, destination: &std::path::Path) {
            std::fs::create_dir_all(destination).unwrap();
            for entry in std::fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let target = destination.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    link_tree(&entry.path(), &target);
                } else if std::fs::hard_link(entry.path(), &target).is_err() {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }

        let (mut snapshot, image) = snapshot_and_input();
        let data = tempfile::tempdir().unwrap();
        link_tree(&snapshot.data_dir, data.path());
        snapshot.data_dir = data.path().to_owned();
        let input = &image;
        let backend = CpuBackend;
        let context = ObservationContext::default();
        let mut cache = RuntimeCache::default();
        let expected = cache
            .prepare_observed(&snapshot, false, &context)
            .unwrap()
            .process(input.clone(), &backend)
            .unwrap();
        let committed_params = cache.entry.as_ref().unwrap().runtime.params().clone();

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
        assert_eq!(
            cache.entry.as_ref().unwrap().runtime.params(),
            &committed_params
        );
        let recovered = cache
            .prepare_observed(&snapshot, false, &context)
            .unwrap()
            .process(input.clone(), &backend)
            .unwrap();
        assert_eq!(recovered.data, expected.data);

        // A different key can load its profiles but still fail runtime setup.
        let mut failed_build = snapshot.clone();
        failed_build.special["film_channel_swap"] = serde_json::json!([2, 1, 0]);
        failed_build.params.scanner.conversion_style = "missing-cache-build-style".into();
        assert!(
            cache
                .prepare_observed(&failed_build, false, &context)
                .err()
                .unwrap()
                .to_string()
                .contains("conversion_style")
        );
        let retained = &cache.entry.as_ref().unwrap().runtime;
        assert_eq!(retained.params(), &committed_params);
        assert_eq!(
            retained.process(input.clone(), &backend).unwrap().data,
            expected.data
        );

        // Any cold rebuild must now fail, while the committed snapshot remains usable.
        std::fs::remove_dir_all(data.path().join("profiles")).unwrap();

        let mut failed_update = snapshot.clone();
        failed_update.params.film_render.base.scale = 2.4;
        failed_update.params.scanner.conversion_style = "missing-cache-update-style".into();
        assert!(
            cache
                .prepare_observed(&failed_update, false, &context)
                .err()
                .unwrap()
                .to_string()
                .contains("conversion_style")
        );
        let retained = &cache.entry.as_ref().unwrap().runtime;
        assert_eq!(retained.params(), &committed_params);
        assert_eq!(
            retained.process(input.clone(), &backend).unwrap().data,
            expected.data
        );
        let recovered = cache
            .prepare_observed(&snapshot, false, &context)
            .unwrap()
            .process(input.clone(), &backend)
            .unwrap();
        assert_eq!(recovered.data, expected.data);
    }
}
