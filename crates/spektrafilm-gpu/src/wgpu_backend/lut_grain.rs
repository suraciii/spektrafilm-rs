use super::*;
use spektrafilm_backend::LutGrainGpuParams;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    n: u32,
    width: u32,
    height: u32,
    seed: u32,
    first_pixel: u32,
    amount: f32,
    size: f32,
    shadows: f32,
    midtones: f32,
    highlights: f32,
    chroma: f32,
    _pad0: f32,
    _pad1: f32,
}

impl WgpuBackend {
    pub(super) fn lut_grain_gpu(
        &self,
        img: &ImageBuf,
        params: &LutGrainGpuParams,
    ) -> Result<ImageBuf, String> {
        let n_pixels = img.pixel_count();
        if n_pixels == 0 {
            return Ok(img.clone());
        }
        let limit = super::bounded::storage_binding_limit(self)?;
        // The dispatch admits the output twice (readback plus the binding
        // walk), so budget three 12-byte image buffers per pixel.
        let chunk_pixels =
            (limit.saturating_sub(64) / (3 * 3 * 4)).clamp(1, u64::from(u32::MAX)) as usize;
        let input = scalars_to_f32(&img.data);
        let mut out = Vec::with_capacity(n_pixels * 3);
        for first in (0..n_pixels).step_by(chunk_pixels) {
            let count = chunk_pixels.min(n_pixels - first);
            let start = first * 3;
            let end = start + count * 3;
            let uniform = Params {
                n: count as u32,
                width: img.width,
                height: img.height,
                seed: params.seed,
                first_pixel: first as u32,
                amount: params.amount,
                size: params.size,
                shadows: params.shadows,
                midtones: params.midtones,
                highlights: params.highlights,
                chroma: params.chroma,
                _pad0: 0.0,
                _pad1: 0.0,
            };
            let output_bytes = vec![0u8; count * 3 * 4];
            let result = self
                .dispatch_compute(
                    include_str!("lut_grain.wgsl"),
                    &[
                        GpuBuffer::uniform(bytemuck::bytes_of(&uniform)),
                        GpuBuffer::storage_ro(bytemuck::cast_slice(&input[start..end])),
                        GpuBuffer::storage_rw(&output_bytes),
                    ],
                    count as u32,
                    2,
                    "lut_grain",
                )?
                .ok_or_else(|| "LUT grain: device resource limits prevent dispatch".to_owned())?;
            out.extend(f32_to_scalars(result));
        }
        Ok(ImageBuf::from_data(img.width, img.height, out))
    }
}
