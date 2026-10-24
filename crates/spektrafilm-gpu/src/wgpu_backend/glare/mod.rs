use super::grain::raster::{RasterStorage, paged_transport};
use super::grain::v1::blur::{GrainBlurState as BlurJob, build_grain_blur_paged};
use super::highlight::{PointwiseAccess, PointwiseJob, image_transport_bytes, pointwise_job};
use super::*;

/// Full-source glare: generation uses original pixel indices, then reference
/// filtering spans the entire noise field before any additive apply. There is
/// no image reduction or spatial normalization in the reference glare model.
/// Live transports are one RGB field, plus blur output and the reference
/// filter's intermediate when enabled. Pointwise scratch is bounded per binding.
pub(super) struct GlareState {
    gen_dispatch: PointwiseJob,
    blur: Option<BlurJob>,
    apply_dispatch: PointwiseJob,
}

pub(super) fn build_glare_state(
    _device: &ObservedDevice,
    gp: &spektrafilm_backend::GlareGpuParams,
    width: u32,
    height: u32,
    buf_b: &(impl RasterStorage + ?Sized),
    backend: &WgpuBackend,
) -> Result<GlareState, String> {
    let img_bytes = image_transport_bytes(backend, width, height, "glare")?;
    let n_pixels = (u64::from(width) * u64::from(height)) as u32;
    let source_pages = buf_b.pages();
    let buf_b = &source_pages;
    let scratch = paged_transport(backend, img_bytes, "glare_noise")?;
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct GenParams {
        n_pixels: u32,
        base_seed: u32,
        mu: f32,
        sigma: f32,
        first_pixel: u32,
        _pad: [u32; 3],
    }
    let gen_dispatch = pointwise_job(
        backend,
        u64::from(n_pixels),
        12,
        include_str!("glare_gen.wgsl"),
        &GenParams {
            n_pixels,
            base_seed: gp.base_seed,
            mu: gp.mu,
            sigma: gp.sigma,
            first_pixel: 0,
            _pad: [0; 3],
        },
        &[(&scratch, PointwiseAccess::Write)],
        Some(4),
        "glare_gen",
    )?;
    let blur = if gp.blur_px > 0.0 {
        let out = paged_transport(backend, img_bytes, "glare_blurred")?;
        let job = build_grain_blur_paged(
            backend,
            width,
            height,
            &scratch,
            &out,
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
        self.gen_dispatch.execute(backend)?;
        if let Some(blur) = &self.blur {
            blur.submit(backend)?;
        }
        self.apply_dispatch.execute(backend)
    }
}
