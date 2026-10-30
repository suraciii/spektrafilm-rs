//! Immutable, frame-owned negative preparation for interactive operator edits.
use super::*;
use crate::stages::scanning::{self, PreparedNegative};
use std::sync::Weak;

/// A single prepared image, not a cross-frame automatic exposure lock.
/// Dropping this cache releases its host or device baseline. Host inputs are
/// borrowed weakly; device inputs retain immutable payload ownership.
#[derive(Default)]
pub struct ScanFrameCache {
    entry: Option<PreparedFrame>,
    device_entry: Option<PreparedDeviceFrame>,
}

impl ScanFrameCache {
    /// Host raster storage, excluding caller-owned input/output and device allocations.
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

enum DeviceFrameSource {
    Host(Weak<ImageBuf>),
    Device(spektrafilm_backend::DeviceImage),
}

struct PreparedDeviceFrame {
    source: DeviceFrameSource,
    film: Arc<Profile>,
    backend: Weak<dyn ComputeBackend>,
    parameters: RuntimeParams,
    rotation_quarter_turns: u32,
    density: spektrafilm_backend::DeviceImage,
    mix_source: Option<spektrafilm_backend::DeviceImage>,
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
    fn device_scan_cache_parameters(&self) -> RuntimeParams {
        let mut parameters = self.params.clone();
        parameters.scanner.operator = Default::default();
        parameters.finishing = Default::default();
        parameters.settings.preview_mode = false;
        parameters.settings.preview_max_size = 0;
        parameters
    }

    fn device_scan_cacheable(&self) -> bool {
        self.params.scanner.requires_full_frame()
            && self.params.taps.inject.is_none()
            && self.params.taps.collect.is_none()
            && !crate::stages::grain_dye::active(&self.params)
            && !crate::stages::grain_fotufilm::active(&self.params)
    }

    fn device_frame_reusable(
        &self,
        frame: &PreparedDeviceFrame,
        request: RetainedRender,
        backend: &Arc<dyn ComputeBackend>,
    ) -> bool {
        Arc::ptr_eq(&frame.film, &self.original_film)
            && frame.backend.as_ptr() as *const () == Arc::as_ptr(backend) as *const ()
            && frame.rotation_quarter_turns == request.rotation_quarter_turns % 4
            && frame.parameters == self.device_scan_cache_parameters()
    }

    pub(crate) fn process_host_scan_cached_retained(
        &self,
        image: &Arc<ImageBuf>,
        cache: &mut ScanFrameCache,
        request: RetainedRender,
        backend: &dyn ComputeBackend,
        backend_owner: &Arc<dyn ComputeBackend>,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.check_cancelled()?;
        if !backend.is_gpu() {
            return Err("retained scan cache requires the selected GPU".into());
        }
        let cancellation_backend = self
            .cancellation
            .as_ref()
            .map(|token| {
                backend
                    .with_cancellation(Some(token.clone()))
                    .ok_or_else(|| "selected GPU cannot bind operation cancellation".to_owned())
            })
            .transpose()?;
        let backend = cancellation_backend.as_deref().unwrap_or(backend);
        cache.entry = None;
        if !self.device_scan_cacheable() {
            cache.device_entry = None;
            return self
                .process_retained(image, request, backend)?
                .ok_or_else(|| "selected GPU declined retained scan".into());
        }
        let reusable = cache.device_entry.as_ref().is_some_and(|frame| {
            matches!(&frame.source, DeviceFrameSource::Host(source) if source.as_ptr() == Arc::as_ptr(image))
                && self.device_frame_reusable(frame, request, backend_owner)
        });
        if !reusable {
            cache.device_entry = None;
            let space = spektrafilm_math::colorspace::resolve(&self.params.io.input_color_space)?;
            let source = backend.retain_image(
                image,
                &spektrafilm_backend::RetainedImageMeta {
                    width: image.width,
                    height: image.height,
                    domain: if self.params.io.input_cctf_decoding {
                        spektrafilm_backend::SampleDomain::Encoded
                    } else {
                        spektrafilm_backend::SampleDomain::Linear
                    },
                    precision: spektrafilm_backend::Precision::host(),
                    space,
                    pixel_pitch_um: None,
                },
            )?;
            cache.device_entry = Some(self.prepare_device_scan_frame(
                &source,
                DeviceFrameSource::Host(Arc::downgrade(image)),
                request,
                backend,
                backend_owner,
            )?);
        }
        self.render_device_scan_frame(
            cache.device_entry.as_ref().expect("device frame installed"),
            request,
            backend,
        )
    }

    pub(crate) fn process_device_scan_cached_retained(
        &self,
        image: &spektrafilm_backend::DeviceImage,
        cache: &mut ScanFrameCache,
        request: RetainedRender,
        backend: &dyn ComputeBackend,
        backend_owner: &Arc<dyn ComputeBackend>,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.check_cancelled()?;
        if !backend.is_gpu() {
            return Err("retained scan cache requires the selected GPU".into());
        }
        let cancellation_backend = self
            .cancellation
            .as_ref()
            .map(|token| {
                backend
                    .with_cancellation(Some(token.clone()))
                    .ok_or_else(|| "selected GPU cannot bind operation cancellation".to_owned())
            })
            .transpose()?;
        let backend = cancellation_backend.as_deref().unwrap_or(backend);
        cache.entry = None;
        if !self.device_scan_cacheable() {
            cache.device_entry = None;
            return self.process_device_retained(image, request, backend);
        }
        let reusable = cache.device_entry.as_ref().is_some_and(|frame| {
            matches!(&frame.source, DeviceFrameSource::Device(source)
                if source.payload().zip(image.payload()).is_some_and(|(a,b)| Arc::ptr_eq(a,b))
                    && source.width() == image.width() && source.height() == image.height()
                    && source.domain() == image.domain() && source.space().name == image.space().name)
                && self.device_frame_reusable(frame, request, backend_owner)
        });
        if !reusable {
            cache.device_entry = None;
            cache.device_entry = Some(self.prepare_device_scan_frame(
                image,
                DeviceFrameSource::Device(image.clone()),
                request,
                backend,
                backend_owner,
            )?);
        }
        self.render_device_scan_frame(
            cache.device_entry.as_ref().expect("device frame installed"),
            request,
            backend,
        )
    }

    fn prepare_device_scan_frame(
        &self,
        source: &spektrafilm_backend::DeviceImage,
        source_key: DeviceFrameSource,
        request: RetainedRender,
        backend: &dyn ComputeBackend,
        backend_owner: &Arc<dyn ComputeBackend>,
    ) -> Result<PreparedDeviceFrame, String> {
        self.check_cancelled()?;
        let full = RetainedRender {
            preview_max_size: None,
            ..request
        };
        let density = self.process_device_retained_with_taps(
            source,
            full,
            backend,
            Some(Tap::RgbIn),
            Some(Tap::CmyFilm),
        )?;
        let mix_source = if self.params.output_mix.amount < 1.0 {
            let source_pitch = crate::resizing::pixel_size_um(
                self.params.camera.film_format_mm,
                source.width(),
                source.height(),
            );
            let (geometry, _) = crate::resizing::resolve_device_geometry(
                source.width(),
                source.height(),
                &self.params.io,
                request.rotation_quarter_turns as i32,
                None,
                source_pitch / self.params.io.upscale_factor as f64,
            )?;
            Some(self.retained_output_mix_source(
                source,
                &[geometry],
                spektrafilm_math::colorspace::resolve(&self.params.io.input_color_space)?,
                spektrafilm_math::colorspace::resolve(&self.params.io.output_color_space)?,
                backend,
            )?)
        } else {
            None
        };
        self.check_cancelled()?;
        Ok(PreparedDeviceFrame {
            source: source_key,
            film: self.original_film.clone(),
            backend: Arc::downgrade(backend_owner),
            parameters: self.device_scan_cache_parameters(),
            rotation_quarter_turns: request.rotation_quarter_turns % 4,
            density,
            mix_source,
        })
    }

    fn render_device_scan_frame(
        &self,
        frame: &PreparedDeviceFrame,
        request: RetainedRender,
        backend: &dyn ComputeBackend,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.check_cancelled()?;
        let cancellation_backend = self
            .cancellation
            .as_ref()
            .map(|token| {
                backend
                    .with_cancellation(Some(token.clone()))
                    .ok_or_else(|| "selected GPU cannot bind operation cancellation".to_owned())
            })
            .transpose()?;
        let backend = cancellation_backend.as_deref().unwrap_or(backend);
        let pitch = frame
            .density
            .pixel_pitch_um()
            .ok_or("prepared scan density has no working pixel pitch")?;
        if let Some(context) = backend.observation_context() {
            context.set_working_dimensions(frame.density.width(), frame.density.height());
        }
        let mut result = self
            .try_gpu_resident_retained_with_taps(
                &frame.density,
                backend,
                pitch,
                0.0,
                Tap::CmyFilm,
                Tap::RgbOut,
            )?
            .ok_or("selected GPU declined prepared scanner tail")?;
        if let Some(source) = &frame.mix_source {
            let output = spektrafilm_math::colorspace::resolve(&self.params.io.output_color_space)?;
            result = backend
                .mix_output_retained(
                    &result,
                    source,
                    &spektrafilm_backend::OutputMixGpuParams {
                        amount: self.params.output_mix.amount as f32,
                        encoded: self.params.io.output_cctf_encoding,
                        cctf_kind: spektrafilm_backend::cctf_kind(output.cctf),
                    },
                )?
                .ok_or("selected GPU cannot blend the prepared scan source")?;
        }
        if let Some(max_size) = request.preview_max_size {
            let (width, height) =
                crate::resizing::preview_dimensions(result.width(), result.height(), max_size);
            let pitch =
                pitch * result.width().max(result.height()) as f64 / width.max(height) as f64;
            let (geometry, _) = crate::resizing::resolve_device_geometry(
                result.width(),
                result.height(),
                &self.params.io,
                0,
                Some(max_size),
                pitch,
            )?;
            result = self.retained_geometry_stage(&result, &geometry, backend)?;
        }
        self.check_cancelled()?;
        Ok(result)
    }
    pub(crate) fn process_scan_cached(
        &self,
        image: &Arc<ImageBuf>,
        cache: &mut ScanFrameCache,
        backend: &dyn ComputeBackend,
        backend_owner: &Arc<dyn ComputeBackend>,
    ) -> Result<ImageBuf, String> {
        self.check_cancelled()?;
        if backend.is_gpu() {
            let retained = self.process_host_scan_cached_retained(
                image,
                cache,
                RetainedRender::FULL,
                backend,
                backend_owner,
            )?;
            return backend.materialize_retained(&retained);
        }
        cache.device_entry = None;
        // Optical grain readout owns its full-frame context and final crop.
        // Recompute that exact route rather than substitute a density tap.
        let cacheable = self.params.scanner.requires_full_frame()
            && self.params.taps.inject.is_none()
            && self.params.taps.collect.is_none()
            && !crate::stages::grain_dye::active(&self.params)
            && !crate::stages::grain_fotufilm::active(&self.params);
        if !cacheable {
            cache.entry = None;
            return self.process(image.as_ref().clone(), backend);
        }
        self.params.validate()?;
        crate::pipeline_calibration::validate_scan_output(&self.params, &self.film)?;
        let mut parameters = self.params.clone();
        parameters.scanner.operator = Default::default();
        parameters.finishing = Default::default();
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
            context.set_path(spektrafilm_backend::telemetry::ExecutionPath::Cpu);
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
                Some(self.prepare_output_mix_source(image, backend)?)
            } else {
                None
            };
            let density =
                if self.params.workflow.route_enum() == crate::WorkflowRoute::ConvertFilmScan {
                    let (working, _) = self.rescale_working(image, backend)?;
                    let observation = crate::stages::StageObservation::cpu(
                        backend,
                        "converting",
                        spektrafilm_backend::telemetry::CpuReason::BackendDefault,
                    );
                    let converted = self
                        .convert_converter
                        .as_ref()
                        .expect("convert-film route has a prepared converter")
                        .process(&working);
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
