//! Assembled CPU fences for the resident chain.
//!
//! Each fence replaces a device stage with a host stage inside the resident
//! route. They are pure code motion out of `try_gpu_resident`: the bodies are
//! unchanged, only the enclosing locals become explicit parameters.
use spektrafilm_backend::{ComputeBackend, CpuStageFn};
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
    reason: spektrafilm_backend::telemetry::CpuReason,
) -> Option<Box<dyn ComputeBackend>> {
    backend
        .observation_context()
        .filter(|context| context.enabled())
        .map(|context| spektrafilm_backend::bind_cpu_fallback(context.clone(), reason))
}

impl super::super::Pipeline {
    /// Host grain as an assembled fence on the film density buffer.
    ///
    /// `dye_on_device` marks a Dye field the device pass realizes and
    /// `spektra_on_device` marks a Spektra request the device pass realizes;
    /// the fence then serves the host grain engines only, so a supported
    /// active engine keeps no host arithmetic at all.
    pub(crate) fn density_cpu_stage<'a>(
        &'a self,
        film_curves_norm: &'a [[f64; 3]],
        backend: &'a dyn ComputeBackend,
        pixel_size_um: f64,
        dye_on_device: bool,
        spektra_on_device: bool,
    ) -> Option<CpuStageFn<'a>> {
        if self.params.film_render.grain.active {
            match self.params.film_render.grain.engine {
                crate::params::grain::GrainEngine::Spektra if !spektra_on_device => {
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
                        crate::stages::filming::apply_spektra_grain(
                            density.clone(),
                            film,
                            grain,
                            norm_curves,
                            use_fast_stats,
                            random_seed,
                            pixel_size_um,
                            observed,
                        )
                    });
                    Some(closure)
                }
                // Spatial deactivation must bypass Dye before its field is
                // resolved (spec film-grain/v3 §Controls and resolved inputs),
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
    ///
    /// The modeled receiver capture and the PCHIP scanner LUT now have
    /// device paths (`stages::scanning::scan_spectral_rgb` calls them
    /// first), so neither fences. Digital Reference and the density reading
    /// replace the acquisition with host calibrations that have no device
    /// equivalent, so they still fence.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scan_acquisition_cpu_stage<'a>(
        &'a self,
        density_reading: bool,
        scan_context: &'a crate::chain_prep::ScanColorContext,
        scan_profile: &'a crate::profile::Profile,
        scan_params: &'a crate::params::RuntimeParams,
        scan_channel_density: &'a [[f64; 3]],
        scan_base_density: &'a [f64],
        backend: &'a dyn ComputeBackend,
    ) -> Option<CpuStageFn<'a>> {
        if density_reading || self.params.settings.use_scanner_lut {
            let profile = scan_profile;
            let params = scan_params;
            let tones = self.conversion_tones;
            let channel_density = scan_channel_density;
            let base_density = scan_base_density;
            let fence_reason = if density_reading {
                spektrafilm_backend::telemetry::CpuReason::NegativeToPositiveOutput
            } else {
                spektrafilm_backend::telemetry::CpuReason::SpectralLut
            };
            let observed = fence_cpu_backend(backend, fence_reason);
            let closure: CpuStageFn<'_> = Box::new(move |density: &ImageBuf| {
                static CPU_BACKEND: spektrafilm_backend::cpu_backend::CpuBackend =
                    spektrafilm_backend::cpu_backend::CpuBackend;
                let backend: &dyn spektrafilm_backend::ComputeBackend =
                    observed.as_deref().unwrap_or(&CPU_BACKEND);
                if density_reading {
                    crate::stages::scanning::interpret_negative_density_stage(
                        density, profile, &tones, params, backend,
                    )
                } else {
                    crate::stages::scanning::scan_spectral_rgb(
                        density,
                        profile,
                        params,
                        backend,
                        channel_density,
                        base_density,
                        scan_context,
                    )
                }
            });
            Some(closure)
        } else {
            None
        }
    }
}
