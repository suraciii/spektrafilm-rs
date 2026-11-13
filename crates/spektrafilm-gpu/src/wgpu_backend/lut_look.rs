use super::*;
use spektrafilm_backend::Lut3dLookGpuParams;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    n: u32,
    size: u32,
    shaper_size: u32,
    clamp_domain: u32,
    domain_min: [f32; 4],
    domain_span: [f32; 4],
    strength: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

impl WgpuBackend {
    pub(super) fn lut3d_look_gpu(
        &self,
        image: &ImageBuf,
        original: &ImageBuf,
        params: &Lut3dLookGpuParams<'_>,
    ) -> Result<ImageBuf, String> {
        if image.width != original.width || image.height != original.height {
            return Err("LUT look: input and original geometry differ".to_owned());
        }
        let expected = u64::from(params.size)
            .checked_pow(3)
            .and_then(|n| n.checked_mul(3))
            .ok_or_else(|| "LUT look: table dimensions overflow".to_owned())?;
        if params.size == 0 || expected != params.table.len() as u64 || params.table.is_empty() {
            return Err("LUT look: table dimensions do not match the declaration".to_owned());
        }
        if params.shaper_size != 0 && params.shaper.len() != params.shaper_size as usize * 3 {
            return Err("LUT look: shaper dimensions do not match the declaration".to_owned());
        }
        let n_pixels = image.pixel_count();
        if n_pixels == 0 {
            return Ok(ImageBuf::new(image.width, image.height));
        }
        let shaper = if params.shaper.is_empty() {
            vec![0.0f32]
        } else {
            params.shaper.to_vec()
        };
        let limit = super::bounded::storage_binding_limit(self)?;
        let table_bytes = (params.table.len() as u64)
            .checked_mul(4)
            .ok_or_else(|| "LUT look: table byte count overflow".to_owned())?;
        let shaper_bytes = (shaper.len() as u64)
            .checked_mul(4)
            .ok_or_else(|| "LUT look: shaper byte count overflow".to_owned())?;
        let reserved = table_bytes
            .checked_add(shaper_bytes)
            .and_then(|bytes| bytes.checked_add(64))
            .ok_or_else(|| "LUT look: working set byte count overflow".to_owned())?;
        if reserved >= limit {
            return Err(format!(
                "LUT look: table and shaper need {reserved} bytes; device binding limit is {limit}"
            ));
        }
        // The dispatch admits the output twice (readback plus the binding
        // walk), so budget four 12-byte image buffers per pixel.
        let chunk_pixels =
            ((limit - reserved) / (4 * 3 * 4)).clamp(1, u64::from(u32::MAX)) as usize;
        let input = scalars_to_f32(&image.data);
        let original = scalars_to_f32(&original.data);
        let mut out = Vec::with_capacity(n_pixels * 3);
        for first in (0..n_pixels).step_by(chunk_pixels) {
            let count = chunk_pixels.min(n_pixels - first);
            let start = first * 3;
            let end = start + count * 3;
            let uniform = Params {
                n: count as u32,
                size: params.size,
                shaper_size: params.shaper_size,
                clamp_domain: u32::from(params.clamp_domain),
                domain_min: [
                    params.domain_min[0],
                    params.domain_min[1],
                    params.domain_min[2],
                    0.0,
                ],
                domain_span: [
                    params.domain_span[0],
                    params.domain_span[1],
                    params.domain_span[2],
                    0.0,
                ],
                strength: params.strength,
                _pad0: 0.0,
                _pad1: 0.0,
                _pad2: 0.0,
            };
            let output_bytes = vec![0u8; count * 3 * 4];
            let result = self
                .dispatch_compute(
                    include_str!("lut_look.wgsl"),
                    &[
                        GpuBuffer::uniform(bytemuck::bytes_of(&uniform)),
                        GpuBuffer::storage_ro(bytemuck::cast_slice(&input[start..end])),
                        GpuBuffer::storage_ro(bytemuck::cast_slice(&original[start..end])),
                        GpuBuffer::storage_ro(bytemuck::cast_slice(params.table)),
                        GpuBuffer::storage_ro(bytemuck::cast_slice(&shaper)),
                        GpuBuffer::storage_rw(&output_bytes),
                    ],
                    count as u32,
                    5,
                    "lut3d_look",
                )?
                .ok_or_else(|| "LUT look: device resource limits prevent dispatch".to_owned())?;
            let chunk = f32_to_scalars(result);
            if chunk.iter().any(|value| !value.is_finite()) {
                return Err("LUT input is outside the declared domain".to_owned());
            }
            out.extend_from_slice(&chunk);
        }
        Ok(ImageBuf::from_data(image.width, image.height, out))
    }
}
