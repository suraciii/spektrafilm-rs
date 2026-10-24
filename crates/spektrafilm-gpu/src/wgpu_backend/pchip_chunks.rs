use super::bounded::{chunk_elements, storage_binding_limit};
use super::*;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    n_pixels: u32,
    size: u32,
    size_m1: u32,
    pad: u32,
    data_min: [f32; 4],
    data_scale: [f32; 4],
    origin: [u32; 4],
    extent: [u32; 4],
}

impl WgpuBackend {
    pub(super) fn pchip_chunks(
        &self,
        image: &ImageBuf,
        p: &spektrafilm_backend::EnlargerLutGpuParams<'_>,
    ) -> Result<ImageBuf, String> {
        let side = p.size as usize;
        #[cfg(test)]
        self.pipe.test_operation("enlarger_lut_interp")?;
        let total = side
            .checked_pow(3)
            .and_then(|n| n.checked_mul(3))
            .ok_or_else(|| "PCHIP lattice dimensions overflow".to_owned())?;
        if side < 2
            || total > u32::MAX as usize
            || [p.lut, p.slope_x, p.slope_y, p.slope_z]
                .iter()
                .any(|v| v.len() != total)
        {
            return Err(
                "PCHIP requires a matching device-indexable lattice of side >= 2".to_owned(),
            );
        }
        let cells = side - 1;
        if p.cell_min.len() != cells * cells * cells * 3
            || p.cell_max.len() != cells * cells * cells * 3
        {
            return Err("PCHIP cell hull dimensions do not match the lattice".to_owned());
        }
        let limit = storage_binding_limit(self)?;
        let pixels = chunk_elements(limit, 12)?;
        let mut edge = 2usize;
        if limit < 2 * 2 * 2 * 12 {
            return Err(
                "PCHIP minimum working set requires 96 bytes per lattice binding".to_owned(),
            );
        }
        while edge < side && (edge as u64 + 1).pow(3) * 12 <= limit {
            edge += 1;
        }
        let stride = edge - 1;
        let types = [
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ];
        let pipeline = self.cached_pipeline(
            include_str!("../../../spektrafilm-shaders/wgsl/enlarger_lut.wgsl"),
            &types,
        );
        let mut output = Vec::with_capacity(image.data.len());
        for samples in image.data.chunks(pixels * 3) {
            let input = scalars_to_f32(samples);
            let n = (input.len() / 3) as u32;
            let upload = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("image_input"),
                    contents: bytemuck::cast_slice(&input),
                    usage: wgpu::BufferUsages::STORAGE,
                });
            let result = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pchip_chunk_output"),
                size: u64::from(n) * 12,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            for x in (0..cells).step_by(stride) {
                for y in (0..cells).step_by(stride) {
                    for z in (0..cells).step_by(stride) {
                        let extent = [edge.min(side - x), edge.min(side - y), edge.min(side - z)];
                        let pack = |values: &[f64], cell: bool| {
                            let dimensions = if cell {
                                [extent[0] - 1, extent[1] - 1, extent[2] - 1]
                            } else {
                                extent
                            };
                            let source_side = if cell { cells } else { side };
                            let mut packed =
                                Vec::with_capacity(dimensions.iter().product::<usize>() * 3);
                            for i in 0..dimensions[0] {
                                for j in 0..dimensions[1] {
                                    for k in 0..dimensions[2] {
                                        let base =
                                            (((x + i) * source_side + y + j) * source_side + z + k)
                                                * 3;
                                        packed.extend(
                                            values[base..base + 3].iter().map(|&v| {
                                                if v.is_finite() { v as f32 } else { 0.0 }
                                            }),
                                        );
                                    }
                                }
                            }
                            self.device
                                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("pchip_brick"),
                                    contents: bytemuck::cast_slice(&packed),
                                    usage: wgpu::BufferUsages::STORAGE,
                                })
                        };
                        let lattice = pack(p.lut, false);
                        let sx = pack(p.slope_x, false);
                        let sy = pack(p.slope_y, false);
                        let sz = pack(p.slope_z, false);
                        let lo = pack(p.cell_min, true);
                        let hi = pack(p.cell_max, true);
                        let params = Params {
                            n_pixels: n,
                            size: p.size,
                            size_m1: p.size - 1,
                            pad: 0,
                            data_min: [
                                p.data_min[0] as f32,
                                p.data_min[1] as f32,
                                p.data_min[2] as f32,
                                0.0,
                            ],
                            data_scale: [
                                p.data_scale[0] as f32,
                                p.data_scale[1] as f32,
                                p.data_scale[2] as f32,
                                0.0,
                            ],
                            origin: [x as u32, y as u32, z as u32, 0],
                            extent: [extent[0] as u32, extent[1] as u32, extent[2] as u32, 0],
                        };
                        let uniform =
                            self.device
                                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("pchip_brick_params"),
                                    contents: bytemuck::bytes_of(&params),
                                    usage: wgpu::BufferUsages::UNIFORM,
                                });
                        let buffers = [
                            &uniform, &upload, &lattice, &sx, &sy, &sz, &lo, &hi, &result,
                        ];
                        let entries: Vec<_> = buffers
                            .iter()
                            .enumerate()
                            .map(|(i, b)| wgpu::BindGroupEntry {
                                binding: i as u32,
                                resource: b.as_entire_binding(),
                            })
                            .collect();
                        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("pchip_brick"),
                            layout: &pipeline.layout,
                            entries: &entries,
                        });
                        let mut encoder = self.device.create_command_encoder(&Default::default());
                        {
                            let mut pass =
                                encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                                    label: Some("enlarger_lut_interpolate"),
                                    timestamp_writes: None,
                                });
                            pass.set_pipeline(&pipeline.pipeline);
                            pass.set_bind_group(0, &group, &[]);
                            dispatch_linear(&mut pass, n);
                        }
                        self.queue.submit(Some(encoder.finish()));
                    }
                }
            }
            output.extend(f32_to_scalars(
                self.pipe.readback_buffer(&result, u64::from(n) * 12),
            ));
        }
        self.device.context.record_executor(Executor::Gpu, None);
        Ok(ImageBuf::from_data(image.width, image.height, output))
    }
}
