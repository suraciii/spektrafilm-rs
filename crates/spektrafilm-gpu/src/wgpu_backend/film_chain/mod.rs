use super::{
    ImageBuf, WgpuBackend, build_dir_state, build_gamut_state, build_glare_state,
    build_grain_v2_state, build_halation_state, build_highlight_boost_state,
    build_simple_blur_state, build_unsharp_state, dispatch_linear, f32_to_scalars, scalars_to_f32,
};

impl WgpuBackend {
    pub fn run_film_chain(&self, p: &crate::FilmChainParams<'_>) -> Result<ImageBuf, String> {
        if !self.device.context.enabled() {
            return self.run_film_chain_inner(p);
        }
        let (backend, _batch) = self.observed_batch("film_chain");
        backend.run_film_chain_inner(p)
    }
}

mod buffers;
mod passes;
