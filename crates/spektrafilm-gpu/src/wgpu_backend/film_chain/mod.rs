use super::{
    ImageBuf, WgpuBackend, build_dir_state, build_gamut_state, build_glare_state,
    build_grain_procedural_state, build_halation_state, build_highlight_boost_state,
    build_input_decode_state, build_simple_blur_state, build_spektra_grain_state,
    build_unsharp_state, dispatch_linear, f32_to_scalars, scalars_to_f32,
};

impl WgpuBackend {
    /// Run the resident chain and return its output as a device-retained
    /// carrier. The production entry point: no readback on this route.
    pub fn run_film_chain_retained(
        &self,
        input: &spektrafilm_backend::DeviceImage,
        p: &spektrafilm_backend::FilmChainParams<'_>,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        if !self.device.context.enabled() {
            return self.run_film_chain_inner(p, input, &self.device.context);
        }
        let (backend, _batch) = self.observed_batch("film_chain");
        backend.run_film_chain_inner(p, input, &self.device.context)
    }

    /// Compatibility entry point: the same resident chain, materialized once
    /// at this boundary. Production callers use
    /// [`Self::run_film_chain_retained`] so the samples never come back.
    pub fn run_film_chain(
        &self,
        input: &spektrafilm_backend::DeviceImage,
        p: &spektrafilm_backend::FilmChainParams<'_>,
    ) -> Result<ImageBuf, String> {
        let retained = self.run_film_chain_retained(input, p)?;
        self.materialize_buffer(&retained)
    }
}

mod buffers;
pub(super) mod device_image;
mod effect_resources;
mod input_geometry;
mod output_samples;
mod passes;
pub(super) use passes::{required_buffer_exceeds_limits, required_chain_buffers};
