use super::blur::GaussianScratch;
use super::grain::raster::RasterStorage;
use super::grain::v1::blur::{GrainBlurState as BlurJob, build_grain_blur_paged_with_scratch};
use super::highlight::{PointwiseAccess, PointwiseJob, image_transport_bytes, pointwise_job};
use super::*;

/// Reference blur is completed over the full source before bounded pointwise
/// combine. Only the current chunk is copied back; no chunk becomes a blur
/// endpoint. Live full-image transports are the blur output and reference
/// filter intermediate, in addition to the caller's original image.
pub(super) struct UnsharpState {
    blur: BlurJob,
    combine: PointwiseJob,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_unsharp_state(
    _device: &ObservedDevice,
    up: &spektrafilm_backend::UnsharpGpuParams,
    width: u32,
    height: u32,
    buf_b: &(impl RasterStorage + ?Sized),
    gaussian_scratch: &mut GaussianScratch,
    backend: &WgpuBackend,
) -> Result<UnsharpState, String> {
    image_transport_bytes(backend, width, height, "unsharp")?;
    let n_pixels = (u64::from(width) * u64::from(height)) as u32;
    let source_pages = buf_b.pages();
    let buf_b = &source_pages;
    let buffers = gaussian_scratch.buffers(backend, width, height)?;
    let blur_dst = buffers.output;
    let blur = build_grain_blur_paged_with_scratch(
        backend,
        width,
        height,
        buf_b,
        &blur_dst,
        Some(&buffers.intermediate),
        [up.sigma_px.max(0.01) as f64; 3],
        0,
        0,
    )
    .map_err(|e| format!("unsharp Gaussian: {e}"))?;
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct CombineParams {
        n_pixels: u32,
        amount: f32,
        _pad: [u32; 2],
    }
    let combine = pointwise_job(
        backend,
        u64::from(n_pixels),
        12,
        include_str!("unsharp_combine.wgsl"),
        &CombineParams {
            n_pixels,
            amount: up.amount,
            _pad: [0; 2],
        },
        &[
            (buf_b, PointwiseAccess::Read),
            (&blur_dst, PointwiseAccess::Read),
            (buf_b, PointwiseAccess::Write),
        ],
        None,
        "unsharp_combine",
    )?;
    Ok(UnsharpState { blur, combine })
}

impl UnsharpState {
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.blur.submit(backend)?;
        self.combine.execute(backend)
    }
}
