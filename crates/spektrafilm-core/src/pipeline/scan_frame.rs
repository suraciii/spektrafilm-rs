//! Immutable, frame-owned negative preparation for interactive operator edits.
use super::*;
use crate::stages::scanning::{self, PreparedNegative};
use std::sync::Weak;

/// A single prepared image, not a cross-frame automatic exposure lock.
/// Dropping this cache releases the f64 exposure raster. The input is borrowed
/// weakly so a cache cannot keep an otherwise closed source image alive.
#[derive(Default)]
pub struct ScanFrameCache {
    entry: Option<PreparedFrame>,
}

impl ScanFrameCache {
    /// Retained raster storage, excluding the caller-owned source and output.
    pub fn retained_bytes(&self) -> usize {
        self.entry.as_ref().map_or(0, |frame| {
            frame.exposure.cached_bytes()
                + frame.mix_source.as_ref().map_or(0, |source| {
                    source.data.capacity()
                        * std::mem::size_of::<spektrafilm_math::precision::Scalar>()
                })
        })
    }
}

struct PreparedFrame {
    input: Weak<ImageBuf>,
    film: Arc<Profile>,
    backend: Weak<dyn ComputeBackend>,
    parameters: RuntimeParams,
    exposure: PreparedNegative,
    mix_source: Option<ImageBuf>,
}

impl Pipeline {
    pub(crate) fn process_scan_cached(
        &self,
        image: &Arc<ImageBuf>,
        cache: &mut ScanFrameCache,
        backend: &dyn ComputeBackend,
        backend_owner: &Arc<dyn ComputeBackend>,
    ) -> Result<ImageBuf, String> {
        self.check_cancelled()?;
        // Optical grain readout owns its full-frame context and final crop.
        // Recompute that exact route rather than substitute a density tap.
        let cacheable = self.params.scanner.requires_full_frame()
            && self.params.taps.inject.is_none()
            && self.params.taps.collect.is_none()
            && !crate::stages::grain_v3::active(&self.params)
            && !crate::stages::grain_fotufilm::active(&self.params);
        if !cacheable {
            cache.entry = None;
            return self.process(image.as_ref().clone(), backend);
        }
        self.params.validate()?;
        crate::pipeline_calibration::validate_scan_output(&self.params, &self.film)?;
        if backend.explicit_gpu_request() {
            return Err("explicit GPU request cannot run the complete computation scope on WGPU (host-executed stages: scanner acquisition); retry with an explicit CPU request to use the per-stage route".into());
        }
        let mut parameters = self.params.clone();
        parameters.scanner.operator = Default::default();
        // Preview affects presentation only on this exact path.
        parameters.settings.preview_mode = false;
        parameters.settings.preview_max_size = 0;
        let reusable = cache.entry.as_ref().is_some_and(|frame| {
            frame.input.as_ptr() == Arc::as_ptr(image)
                && Arc::ptr_eq(&frame.film, &self.original_film)
                && frame.backend.as_ptr() as *const () == Arc::as_ptr(backend_owner) as *const ()
                && frame.parameters == parameters
        });
        if let Some(context) = backend.observation_context() {
            context.set_path(if backend.is_gpu() {
                spektrafilm_backend::telemetry::ExecutionPath::PerStage
            } else {
                spektrafilm_backend::telemetry::ExecutionPath::Cpu
            });
        }
        let cpu_observed = backend.observation_context().map(|context| {
            spektrafilm_backend::bind_cpu_fallback(
                context.clone(),
                spektrafilm_backend::telemetry::CpuReason::NegativeToPositiveOutput,
            )
        });
        static CPU: spektrafilm_backend::cpu_backend::CpuBackend =
            spektrafilm_backend::cpu_backend::CpuBackend;
        let scan_backend: &dyn ComputeBackend = cpu_observed.as_deref().unwrap_or(&CPU);
        if !reusable {
            // Release the old large raster before allocating the replacement.
            cache.entry = None;
            let mix_source = if self.params.output_mix.amount < 1.0 {
                Some(self.prepare_output_mix_source(image)?)
            } else {
                None
            };
            let density = if self.params.workflow.route_enum()
                == crate::WorkflowRoute::ConvertFilmScan
            {
                if backend.explicit_gpu_request() {
                    return Err("explicit GPU request cannot run the complete conversion route on WGPU; retry with an explicit CPU request".into());
                }
                let (working, _) = self.rescale_working(image, backend)?;
                let observation = crate::stages::StageObservation::cpu(
                    backend,
                    "converting",
                    spektrafilm_backend::telemetry::CpuReason::BackendDefault,
                );
                let converted =
                    crate::stages::converting::process(&working, &self.film, &self.params)?;
                drop(observation);
                converted
            } else {
                self.run_from(
                    image.as_ref().clone(),
                    Tap::RgbIn,
                    Tap::CmyFilm,
                    backend,
                    None,
                    None,
                    None,
                )?
            };
            self.check_cancelled()?;
            let exposure =
                scanning::prepare_negative(&density, &self.film, &self.params, scan_backend)?;
            self.check_cancelled()?;
            cache.entry = Some(PreparedFrame {
                input: Arc::downgrade(image),
                film: self.original_film.clone(),
                backend: Arc::downgrade(backend_owner),
                parameters,
                exposure,
                mix_source,
            });
        }
        let frame = cache.entry.as_ref().expect("prepared frame installed");
        if let Some(context) = backend.observation_context() {
            let (width, height) = frame.exposure.dimensions();
            context.set_working_dimensions(width, height);
        }
        let rgb = scanning::render_prepared_negative(
            &frame.exposure,
            &self.conversion_tones,
            &self.params,
            scan_backend,
        )?;
        self.check_cancelled()?;
        let output = scanning::finish_capture(
            rgb,
            &self.film,
            &self.params,
            scan_backend,
            &self.output_gamut,
        )?;
        let output = self.compose_output_mix(output, frame.mix_source.clone(), backend, None)?;
        self.check_cancelled()?;
        Ok(output)
    }
}
