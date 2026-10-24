//! Device passes for the optical stages: the diffusion-filter PSF
//! convolution and the enlarger PCHIP LUT.
//!
//! Both are per-stage host dispatches (the per-stage route, and the fence
//! replacements on the resident chain). The kernels reproduce the CPU
//! reference exactly — same tap weights, same reflect extension, same
//! Hermite walk — so the device path is a numerical equivalent, not an
//! approximation.
//!
//! Device resources are bounded before allocation: the diffusion kernel is
//! a 2-D tap convolution whose cost grows with the kernel area, so an
//! oversized request is declined by the host before it reaches the device
//! (a device radius cap would change the filter, see
//! `spektrafilm_backend::FilmChainParams::gpu_blurs_supported`).

use super::*;
mod bounded;

// Kernel support is the reference's resolved support; storage is paged below.

impl WgpuBackend {
    /// Device diffusion filter: per-channel discrete PSF convolution with
    /// reflect extension, then the scatter blend.
    ///
    /// CPU equivalent:
    /// `spektrafilm_model::diffusion::apply_diffusion_filter_um`. The PSF
    /// taps and the scatter fraction are resolved by the caller from the
    /// same reference coefficients, so the shader only evaluates the
    /// convolution the reference performs.
    pub fn optical_diffusion_gpu(
        &self,
        img: &ImageBuf,
        params: &spektrafilm_backend::OpticalDiffusionGpuParams<'_>,
    ) -> Result<ImageBuf, String> {
        let pages = self.allocate_image_pages(img.width,img.height,"diffusion_input")?;
        for page in &pages {
            let start = page.first_pixel as usize*3;
            let end = start+page.n_pixels as usize*3;
            let samples = scalars_to_f32(&img.data[start..end]);
            self.queue.write_buffer(&page.buffer,0,bytemuck::cast_slice(&samples));
        }
        let source = super::film_chain::device_image::BufferPayload::from_pages(self.backend_id(),pages,img.width,img.height);
        let output = self.diffusion_pages(&source,params,None)?;
        let result = self.retain_image_pages(output,img.width,img.height,
            spektrafilm_backend::SampleDomain::Linear,&spektrafilm_math::colorspace::SRGB,None);
        self.materialize_buffer(&result)
    }

    /// Device enlarger LUT: per-pixel 3-D PCHIP interpolation.
    ///
    /// CPU equivalent: `apply_pchip_lut` in `stages::printing`.
    pub fn enlarger_lut_gpu(
        &self,
        img: &ImageBuf,
        params: &spektrafilm_backend::EnlargerLutGpuParams<'_>,
    ) -> Result<ImageBuf, String> {
        self.pchip_chunks(img, params)
    }
}

#[cfg(test)]
mod tests;
