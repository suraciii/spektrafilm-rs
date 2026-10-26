use super::blur::GaussianScratch;
use super::grain::raster::{RasterCommands, RasterStorage, paged_transport, transport, uniform};
use super::grain::v1::blur::{GrainBlurState as BlurJob, build_grain_blur_paged_with_scratch};
use super::highlight::{PointwiseAccess, PointwiseJob, image_transport_bytes, pointwise_job};
use super::*;

/// Full-source glare: generation follows the reference row-major stream, then
/// filtering spans the entire noise field before any additive apply. There is
/// no image reduction or spatial normalization in the reference glare model.
/// Live transports are one RGB field, plus blur output and the reference
/// filter's intermediate when enabled. Pointwise scratch is bounded per binding.
pub(super) struct GlareState {
    gen_dispatch: RasterCommands,
    blur: Option<BlurJob>,
    apply_dispatch: PointwiseJob,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_glare_state(
    _device: &ObservedDevice,
    gp: &spektrafilm_backend::GlareGpuParams,
    width: u32,
    height: u32,
    buf_b: &(impl RasterStorage + ?Sized),
    gaussian_scratch: &mut GaussianScratch,
    backend: &WgpuBackend,
) -> Result<GlareState, String> {
    let img_bytes = image_transport_bytes(backend, width, height, "glare")?;
    let n_pixels = (u64::from(width) * u64::from(height)) as u32;
    let source_pages = buf_b.pages();
    let buf_b = &source_pages;
    // Keep the reference word stream and cached normal across bounded chunks.
    // The transform uses f32; no native f64/int64 feature is required here.
    let limit = super::bounded::storage_binding_limit(backend)?;
    const STATE_BYTES: u64 = 2516;
    if limit < STATE_BYTES {
        return Err(format!(
            "glare MT19937 state needs {STATE_BYTES} bytes per binding; limit is {limit}"
        ));
    }
    let scratch = paged_transport(backend, img_bytes, "glare_noise")?;
    let capacity = (limit / 12).min(4096) as u32;
    let window = transport(
        backend,
        u64::from(capacity.min(n_pixels)) * 12,
        "glare_generator_window",
    )?;
    let generator = transport(backend, STATE_BYTES, "glare_mt19937_state")?;
    let pipeline = backend.cached_pipeline(
        concat!(
            include_str!("../../../../spektrafilm-shaders/wgsl/mt19937.wgsl"),
            include_str!("glare_gen.wgsl")
        ),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct GenParams {
        count: u32,
        first: u32,
        seed: u32,
        pad: u32,
        mu: f32,
        sigma: f32,
        pad_tail: [u32; 2],
    }
    let mut gen_dispatch = RasterCommands::default();
    for first in (0..n_pixels).step_by(capacity as usize) {
        backend.check_cancelled()?;
        let count = capacity.min(n_pixels - first);
        let params = uniform(
            backend,
            &GenParams {
                count,
                first,
                seed: gp.base_seed,
                pad: 0,
                mu: gp.mu,
                sigma: gp.sigma,
                pad_tail: [0; 2],
            },
        );
        gen_dispatch.dispatch(&pipeline, &[&params, &window, &generator], 1, true, backend);
        gen_dispatch.copy(
            &window,
            0,
            &scratch,
            u64::from(first) * 12,
            u64::from(count) * 12,
        );
    }
    let blur = if gp.blur_px > 0.0 {
        let buffers = gaussian_scratch.buffers(backend, width, height)?;
        let out = buffers.output;
        let job = build_grain_blur_paged_with_scratch(
            backend,
            width,
            height,
            &scratch,
            &out,
            Some(&buffers.intermediate),
            [gp.blur_px.max(0.01) as f64; 3],
            0,
            0,
        )
        .map_err(|e| format!("glare Gaussian: {e}"))?;
        Some((job, out))
    } else {
        None
    };
    let glare_input = blur.as_ref().map_or(&scratch, |(_, out)| out);
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct ApplyParams {
        n_pixels: u32,
        _pad: [u32; 3],
        offset: [f32; 4],
    }
    let apply_dispatch = pointwise_job(
        backend,
        u64::from(n_pixels),
        12,
        include_str!("glare_apply.wgsl"),
        &ApplyParams {
            n_pixels,
            _pad: [0; 3],
            offset: [gp.rgb_offset[0], gp.rgb_offset[1], gp.rgb_offset[2], 0.0],
        },
        &[
            (glare_input, PointwiseAccess::Read),
            (buf_b, PointwiseAccess::ReadWrite),
        ],
        None,
        "glare_apply",
    )?;
    Ok(GlareState {
        gen_dispatch,
        blur: blur.map(|(job, _)| job),
        apply_dispatch,
    })
}

impl GlareState {
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.gen_dispatch.submit_named(backend, "glare_gen")?;
        if let Some(blur) = &self.blur {
            blur.submit(backend)?;
        }
        self.apply_dispatch.execute(backend)
    }
}

#[cfg(test)]
mod tests;
