//! Assembled CPU fences for the resident chain.
//!
//! Each fence replaces a device stage with a host stage inside the resident
//! route. They are pure code motion out of `try_gpu_resident`: the bodies are
//! unchanged, only the enclosing locals become explicit parameters.
use super::chain::CpuStageFn;
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;

/// Observed CPU backend for a fence stage, carrying the published reason.
///
/// A fenced CPU stage executes inside the resident chain, so it must keep the
/// executor observations the per-stage route publishes (telemetry metrics: an
/// actual CPU image stage is identified from executor observations). Fence
/// closures pass this observed CPU backend to the shared stage functions so
/// the stage's own stage markers stay live.
pub(crate) fn fence_cpu_backend(
    backend: &dyn ComputeBackend,
    reason: spektrafilm_gpu::telemetry::CpuReason,
) -> Option<Box<dyn ComputeBackend>> {
    backend
        .observation_context()
        .filter(|context| context.enabled())
        .map(|context| spektrafilm_gpu::bind_cpu_fallback(context.clone(), reason))
}

impl super::super::Pipeline {
    /// Host grain as an assembled fence on the film density buffer.
    ///
    /// `dye_on_device` marks a Dye field the device pass realizes; the
    /// fence then serves the host grain engine only, so a supported active
    /// Dye keeps no host arithmetic at all.
    pub(crate) fn density_cpu_stage<'a>(
        &'a self,
        film_curves_norm: &'a [[f64; 3]],
        backend: &'a dyn ComputeBackend,
        pixel_size_um: f64,
        dye_on_device: bool,
    ) -> Option<CpuStageFn<'a>> {
        if self.params.film_render.grain.active {
            match self.params.film_render.grain.engine {
                crate::params::grain::GrainEngine::Spektra => {
                    let film = &self.film;
                    let grain = &self.params.film_render.grain;
                    let norm_curves = film_curves_norm;
                    let use_fast_stats = self.params.settings.use_fast_stats;
                    let random_seed = self.params.random_seed;
                    let seed_backend = backend;
                    let closure: CpuStageFn<'_> = Box::new(move |density| {
                        let observation =
                            crate::stages::StageObservation::new(seed_backend, "grain_v1");
                        let observed = observation.backend(seed_backend);
                        Ok(crate::stages::filming::apply_spektra_grain(
                            density.clone(),
                            film,
                            grain,
                            norm_curves,
                            use_fast_stats,
                            random_seed,
                            pixel_size_um,
                            observed,
                        ))
                    });
                    Some(closure)
                }
                // Spatial deactivation must bypass Dye before its field is
                // resolved (spec film-grain/v3 §Controls and compatibility),
                // exactly like the per-stage route: `grain_v3::active` is the
                // one authoritative gate. The fence holds the host field only
                // for a request the device cannot realize; a device-realized
                // field runs as the `grain_dye` pass instead.
                crate::params::grain::GrainEngine::Dye
                    if crate::stages::grain_v3::active(&self.params) && !dye_on_device =>
                {
                    let film = &self.film;
                    let params = &self.params;
                    let seed_backend = backend;
                    let source_pitch_um = pixel_size_um;
                    let closure: CpuStageFn<'_> = Box::new(move |density| {
                        // The field reads the density the chain developed on
                        // the device, with the identity readout geometry: the
                        // request resolves to the delivered raster, so the
                        // fence writes the field back through the same buffer.
                        // `readout` reports its own `grain_v3_field` CPU
                        // observation with the published reason.
                        let grid =
                            crate::stages::grain_v3::geometry(density, params, source_pitch_um)?
                                .grid();
                        if grid.width != density.width || grid.height != density.height {
                            return Err(format!(
                                "film grain Dye: readout geometry {}x{} does not match the \
                                 resident raster {}x{}",
                                grid.width, grid.height, density.width, density.height
                            ));
                        }
                        crate::stages::grain_v3::readout(
                            density,
                            film,
                            params,
                            source_pitch_um,
                            grid,
                            seed_backend,
                        )
                    });
                    Some(closure)
                }
                _ => None,
            }
        } else {
            None
        }
    }

    /// Scanner acquisition replacement as an assembled CPU fence.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scan_acquisition_cpu_stage<'a>(
        &'a self,
        reference_mode: bool,
        density_reading: bool,
        n2p_requested: bool,
        scan_context: &'a crate::chain_prep::ScanColorContext,
        scan_profile: &'a crate::profile::Profile,
        scan_params: &'a crate::params::RuntimeParams,
        scan_channel_density: &'a [[f64; 3]],
        scan_base_density: &'a [f64],
        color_ref: &'a crate::color_reference::ColorReference,
        backend: &'a dyn ComputeBackend,
    ) -> Option<CpuStageFn<'a>> {
        if reference_mode
            || density_reading
            || scan_context.receiver.is_some()
            || self.params.settings.use_scanner_lut
            || color_ref.has_remap()
        {
            let profile = scan_profile;
            let params = scan_params;
            let channel_density = scan_channel_density;
            let base_density = scan_base_density;
            let reference = self.digital_reference.as_deref();
            let defer_correction = !n2p_requested;
            let fence_reason = if reference_mode || density_reading {
                spektrafilm_gpu::telemetry::CpuReason::NegativeToPositiveOutput
            } else if scan_context.receiver.is_some() {
                spektrafilm_gpu::telemetry::CpuReason::ScannerReceiver
            } else {
                spektrafilm_gpu::telemetry::CpuReason::SpectralLut
            };
            let observed = fence_cpu_backend(backend, fence_reason);
            let closure: CpuStageFn<'_> = Box::new(move |density: &ImageBuf| {
                static CPU_BACKEND: spektrafilm_gpu::cpu_backend::CpuBackend =
                    spektrafilm_gpu::cpu_backend::CpuBackend;
                let backend: &dyn spektrafilm_gpu::ComputeBackend =
                    observed.as_deref().unwrap_or(&CPU_BACKEND);
                if reference_mode {
                    // `stages::scanning` validates the prepared assets
                    // at scan entry and construction prepares them, so
                    // this only guards the library entry points.
                    let reference = reference.ok_or_else(|| {
                        "Digital Reference requires runtime-prepared spectral assets".to_owned()
                    })?;
                    reference.convert(density, params, backend)
                } else if density_reading {
                    crate::stages::scanning::interpret_negative_density_stage(
                        density, profile, params, backend,
                    )
                } else {
                    Ok(crate::stages::scanning::scan_spectral_rgb(
                        density,
                        profile,
                        params,
                        backend,
                        color_ref,
                        channel_density,
                        base_density,
                        scan_context,
                        defer_correction,
                    ))
                }
            });
            Some(closure)
        } else {
            None
        }
    }

    /// Camera optical diffusion as an assembled fence on the raw exposure buffer.
    pub(crate) fn camera_diffusion_cpu_stage<'a>(
        &'a self,
        backend: &'a dyn ComputeBackend,
        pixel_size_um: f64,
    ) -> Option<CpuStageFn<'a>> {
        if self.params.camera.diffusion_filter.active {
            let dm = self.params.camera.diffusion_filter.to_model();
            let seed_backend = backend;
            let closure: CpuStageFn<'_> =
                Box::new(move |raw: &ImageBuf| {
                    // The filter runs on the host here, so it carries the
                    // published `optical_diffusion` reason exactly like the
                    // per-stage route's marker in `filming::expose`.
                    let _diffusion = crate::stages::StageObservation::cpu(
                        seed_backend,
                        "optical_diffusion",
                        spektrafilm_gpu::telemetry::CpuReason::OpticalDiffusion,
                    );
                    Ok(spektrafilm_model::diffusion::apply_diffusion_filter_um(
                        raw,
                        &dm,
                        pixel_size_um,
                    )
                    .expect("camera diffusion filter family validated at pipeline entry"))
                });
            Some(closure)
        } else {
            None
        }
    }

    /// Print acquisition replacement as an assembled CPU fence.
    pub(crate) fn print_acquisition_cpu_stage<'a>(
        &'a self,
        print_fence_requested: bool,
        color_ref: &'a crate::color_reference::ColorReference,
        backend: &'a dyn ComputeBackend,
        pixel_size_um: f64,
    ) -> Option<CpuStageFn<'a>> {
        if print_fence_requested && !self.params.io.scan_film {
            let film = &self.film;
            let print = &self.print;
            let params = &self.params;
            let illuminant = &self.print_illuminant;
            let exposure_factor = self.print_exposure_factor;
            let preflash = self.preflash_raw;
            let bw = color_ref.printing_exposure_correction;
            let fence_reason = if self.params.settings.use_enlarger_lut {
                spektrafilm_gpu::telemetry::CpuReason::SpectralLut
            } else {
                spektrafilm_gpu::telemetry::CpuReason::OpticalDiffusion
            };
            let observed = fence_cpu_backend(backend, fence_reason);
            let closure: CpuStageFn<'_> = Box::new(move |density: &ImageBuf| {
                static CPU_BACKEND: spektrafilm_gpu::cpu_backend::CpuBackend =
                    spektrafilm_gpu::cpu_backend::CpuBackend;
                let backend: &dyn spektrafilm_gpu::ComputeBackend =
                    observed.as_deref().unwrap_or(&CPU_BACKEND);
                Ok(crate::stages::printing::expose_calibrated(
                    density,
                    film,
                    print,
                    params,
                    backend,
                    illuminant,
                    exposure_factor,
                    preflash,
                    bw,
                    pixel_size_um,
                ))
            });
            Some(closure)
        } else {
            None
        }
    }

    /// Readout interpretation as an assembled CPU fence on the scan tail.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scan_tail_cpu_stage<'a>(
        &'a self,
        n2p_capture_method: bool,
        reference_mode: bool,
        density_reading: bool,
        slide_points_active: bool,
        n2p_requested: bool,
        scan_context: &'a crate::chain_prep::ScanColorContext,
        scan_profile: &'a crate::profile::Profile,
        scan_params: &'a crate::params::RuntimeParams,
        scan_channel_density: &'a [[f64; 3]],
        scan_base_density: &'a [f64],
        scan_output_space: &'a spektrafilm_math::colorspace::RgbColorSpace,
        color_ref: &'a crate::color_reference::ColorReference,
        backend: &'a dyn ComputeBackend,
    ) -> Option<CpuStageFn<'a>> {
        if n2p_capture_method
            || reference_mode
            || density_reading
            || color_ref.has_remap()
            || slide_points_active
        {
            let profile = scan_profile;
            let params = scan_params;
            let channel_density = scan_channel_density;
            let base_density = scan_base_density;
            let n2p = n2p_requested;
            let seed_backend = backend;
            // Negative-to-positive binds its calibration captures to the CPU
            // backend, exactly like the per-stage route; the observed wrapper
            // keeps the stage's own executor reasons published.
            let n2p_backend = fence_cpu_backend(
                backend,
                spektrafilm_gpu::telemetry::CpuReason::NegativeToPositiveOutput,
            );
            let closure: CpuStageFn<'_> = Box::new(move |rgb: &ImageBuf| {
                let observation =
                        // The assembled readout fence is the resident
                        // route's scanner stage, so it reports under the
                        // published stage vocabulary ("scanning"), like
                        // the per-stage route's scan stage.
                        crate::stages::StageObservation::new(seed_backend, "scanning");
                let observed = observation.backend(seed_backend);
                static CPU_BACKEND: spektrafilm_gpu::cpu_backend::CpuBackend =
                    spektrafilm_gpu::cpu_backend::CpuBackend;
                let interp_backend: &dyn spektrafilm_gpu::ComputeBackend = if n2p {
                    n2p_backend.as_deref().unwrap_or(&CPU_BACKEND)
                } else {
                    observed
                };
                // Active slide points execute on the CPU here, so they
                // carry the published `scanner_film_points` reason
                // (telemetry metrics; GPU execution of the points
                // would not).
                let _points = slide_points_active.then(|| {
                    crate::stages::StageObservation::cpu(
                        seed_backend,
                        "scanner_film_points",
                        spektrafilm_gpu::telemetry::CpuReason::ScannerFilmPoints,
                    )
                });
                let mut image = rgb.clone();
                crate::stages::scanning::apply_scan_readout_interpretation(
                    &mut image,
                    profile,
                    params,
                    interp_backend,
                    color_ref,
                    channel_density,
                    base_density,
                    scan_context,
                    scan_output_space,
                    n2p,
                    reference_mode,
                    density_reading,
                )?;
                Ok(image)
            });
            Some(closure)
        } else {
            None
        }
    }
}
