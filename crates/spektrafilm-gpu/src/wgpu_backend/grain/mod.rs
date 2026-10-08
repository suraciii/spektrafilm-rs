use super::*;

impl WgpuBackend {
    /// Execute the standalone display-domain Grain V2 shader.
    pub fn grain_v2_gpu(
        &self,
        img: &ImageBuf,
        params: &crate::GrainV2GpuParams,
    ) -> ImageBuf {
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct ShaderParams {
            dimensions: [u32; 4],
            controls: [f32; 4],
            geometry: [f32; 4],
            flags: [f32; 4],
        }

        let input = scalars_to_f32(&img.data);
        let output = vec![0u8; input.len() * std::mem::size_of::<f32>()];
        let shader_params = ShaderParams {
            dimensions: [img.width, img.height, params.seed, params.mode],
            controls: [
                params.amount,
                params.shadows,
                params.midtones,
                params.highlights,
            ],
            geometry: [
                params.raw_scale,
                params.cluster_size,
                params.rotation,
                params.color,
            ],
            flags: [
                params.resolution_factor,
                params.resolution_type as f32,
                params.colored as u32 as f32,
                params.clustered as u32 as f32,
            ],
        };
        let output = self.dispatch_compute(
            include_str!("grain_v2.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&shader_params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(input.as_ref())),
                GpuBuffer::storage_rw(&output),
            ],
            img.pixel_count() as u32,
            2,
        );
        ImageBuf::from_data(img.width, img.height, f32_to_scalars(output))
    }

}
