use super::bounded::{chunk_elements, storage_binding_limit};
use super::*;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    n: u32,
    size: u32,
    first: u32,
    len: u32,
}

impl WgpuBackend {
    pub(super) fn lut3d_chunks(&self, image: &ImageBuf, lut: &Lut3D) -> Result<ImageBuf, String> {
        #[cfg(test)]
        self.pipe.test_operation("lut3d_interp")?;
        self.lut3d_chunks_with_limit(image, lut, storage_binding_limit(self)?)
    }

    fn lut3d_chunks_with_limit(
        &self,
        image: &ImageBuf,
        lut: &Lut3D,
        limit: u64,
    ) -> Result<ImageBuf, String> {
        let expected = u64::from(lut.size)
            .checked_pow(3)
            .and_then(|n| n.checked_mul(3))
            .ok_or_else(|| "LUT dimensions overflow".to_owned())?;
        if lut.size == 0 || expected != lut.data.len() as u64 || expected > u64::from(u32::MAX) {
            return Err("LUT dimensions do not match device-indexable lattice samples".to_owned());
        }
        let chunk = chunk_elements(limit, 8 * 3 * 4)?;
        let pages = self.upload_pages_with_limit(&lut.data, "lut_page", limit)?;
        let types = [
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: false },
        ];
        let gather = self.cached_pipeline_at(include_str!("lut_chunks.wgsl"), &types, "gather");
        let finish = self.cached_pipeline_at(include_str!("lut_chunks.wgsl"), &types, "finish");
        let mut output = Vec::with_capacity(image.data.len());
        for samples in image.data.chunks(chunk * 3) {
            let input = scalars_to_f32(samples);
            let n = (input.len() / 3) as u32;
            let upload = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("image_input"),
                    contents: bytemuck::cast_slice(&input),
                    usage: wgpu::BufferUsages::STORAGE,
                });
            let scratch = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lut_corners"),
                size: u64::from(n) * 96,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let result = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lut_chunk_output"),
                size: u64::from(n) * 12,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            for (index, page) in pages.iter().enumerate() {
                let uniform = Params {
                    n,
                    size: lut.size,
                    first: page.first,
                    len: page.len,
                };
                let params = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("lut_page_params"),
                        contents: bytemuck::bytes_of(&uniform),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let entries = [&params, &upload, &page.buffer, &scratch, &result];
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("lut_page"),
                    layout: &gather.layout,
                    entries: &entries
                        .iter()
                        .enumerate()
                        .map(|(i, b)| wgpu::BindGroupEntry {
                            binding: i as u32,
                            resource: b.as_entire_binding(),
                        })
                        .collect::<Vec<_>>(),
                });
                let mut encoder = self.device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("lut_gather"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&gather.pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    dispatch_linear(&mut pass, n);
                }
                if index + 1 == pages.len() {
                    let finish_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("lut_finish"),
                        layout: &finish.layout,
                        entries: &entries
                            .iter()
                            .enumerate()
                            .map(|(i, b)| wgpu::BindGroupEntry {
                                binding: i as u32,
                                resource: b.as_entire_binding(),
                            })
                            .collect::<Vec<_>>(),
                    });
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("lut3d_interp"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&finish.pipeline);
                    pass.set_bind_group(0, &finish_group, &[]);
                    dispatch_linear(&mut pass, n);
                }
                self.submit_commands(Some(encoder.finish()))?;
            }
            output.extend(f32_to_scalars(
                self.pipe
                    .readback_buffer_cancellable(&result, u64::from(n) * 12)?,
            ));
        }
        self.device.context.record_executor(Executor::Gpu, None);
        Ok(ImageBuf::from_data(image.width, image.height, output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::precision::from_f64;

    #[test]
    fn paged_lut_preserves_corner_channels_and_partial_last_chunk() {
        let gpu = crate::test_support::adapter();
        let size = 5;
        let mut data = Vec::new();
        for x in 0..size {
            for y in 0..size {
                for z in 0..size {
                    data.extend([
                        x as f32 * 0.17 + y as f32 * 0.031,
                        y as f32 * 0.13 + z as f32 * 0.023,
                        z as f32 * 0.19 + x as f32 * 0.011,
                    ]);
                }
            }
        }
        let lut = Lut3D { size, data };
        let samples = (0..35)
            .flat_map(|i| {
                [
                    from_f64((i % 7) as f64 / 6.0),
                    from_f64((i % 5) as f64 / 4.0),
                    from_f64((i % 3) as f64 / 2.0),
                ]
            })
            .collect();
        let image = ImageBuf::from_data(7, 5, samples);
        // 101 bytes splits RGB lattice nodes across pages and leaves three
        // image pixels after the final full eight-corner scratch chunk.
        let tiled = gpu.lut3d_chunks_with_limit(&image, &lut, 101).unwrap();
        let untiled = gpu.lut3d_chunks_with_limit(&image, &lut, 1 << 20).unwrap();
        let cpu = cpu_backend::CpuBackend.lut3d_interp(&image, &lut).unwrap();
        assert_eq!((tiled.width, tiled.height), (7, 5));
        assert_eq!(tiled.data, untiled.data);
        for (actual, expected) in tiled.data.iter().zip(&cpu.data) {
            assert!((*actual as f64 - *expected as f64).abs() < 2e-6);
        }
    }
}
