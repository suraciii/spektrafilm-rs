use super::*;

impl WgpuBackend {
    /// Execute Grain V2 on native encoded RGB with no transfer or primaries conversion.
    pub fn grain_v2_gpu(
        &self,
        img: &ImageBuf,
        params: &crate::GrainV2GpuParams,
    ) -> ImageBuf {
        use wgpu::util::DeviceExt;
        if img.width == 0 || img.height == 0 { return img.clone(); }
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct ShaderParams {
            dimensions: [u32; 4],
            controls: [f32; 4],
            geometry: [f32; 4],
            flags: [f32; 4],
        }

        let input = scalars_to_f32(&img.data);
        let bytes = (input.len() * std::mem::size_of::<f32>()) as u64;
        let shader_params = ShaderParams {
            dimensions: [img.width, img.height, spektrafilm_math::grain::seeded_phase(params.seed).to_bits(), params.mode],
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
                params.film_type as f32,
                params.colored as u32 as f32,
                params.clustered as u32 as f32,
            ],
        };
        let storage = wgpu::BufferBindingType::Storage { read_only: true };
        let writable = wgpu::BufferBindingType::Storage { read_only: false };
        let uniform = wgpu::BufferBindingType::Uniform;
        let filter = self.cached_pipeline(include_str!("film_resolution.wgsl"), &[uniform, storage, storage, writable]);
        let grain = self.cached_pipeline(include_str!("grain_v2.wgsl"), &[uniform, storage, writable, storage]);
        let create = |label| self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label), size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let original = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_encoded_input"), contents: bytemuck::cast_slice(input.as_ref()), usage: wgpu::BufferUsages::STORAGE,
        });
        let prepared = create("grain_prepared_input");
        let output = create("grain_output");
        let a = params.amount.clamp(0., 1.);
        let gsf = (5200. / img.width as f32).max(3100. / img.height as f32);
        let base = (1. + (params.raw_scale - 1.) / 47.) * (1. - params.resolution_factor.clamp(0., 100.) / 100.) / gsf;
        let radius = base * if params.mode != 0 {
            1.87 * (0.12 * a * a + 0.68 * a + 0.2)
        } else {
            (if params.film_type == 1 { 1.2 } else { 1.6 }) * (0.7 * a * a + 0.3 * a + 0.05)
        };
        let optical = params.film_type != 1;
        let weights: Vec<[f32; 2]> = if optical {
            spektrafilm_math::grain::optical_weights(radius).into_iter().map(|w| [w, 0.]).collect()
        } else { spektrafilm_math::grain::fast_blur_weights(radius) };
        let weights_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_resolution_weights"), contents: bytemuck::cast_slice(&weights), usage: wgpu::BufferUsages::STORAGE,
        });
        let bind = |pipeline: &CachedPipelineRef, buffers: &[&wgpu::Buffer]| {
            let entries: Vec<_> = buffers.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32, resource: b.as_entire_binding(),
            }).collect();
            self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("grain_pass"), layout: &pipeline.layout, entries: &entries })
        };
        let filter_params = |axis| self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_filter_params"),
            contents: bytemuck::cast_slice(&[img.width, img.height, axis, optical as u32, weights.len() as u32, 0, 0, 0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let prepare_params = filter_params(2);
        let prepare_group = bind(&filter, &[&prepare_params, &original, &weights_buf, &prepared]);
        let filtered = (radius > 0.).then(|| create("grain_filtered_input"));
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let run = |encoder: &mut wgpu::CommandEncoder, pipeline: &CachedPipelineRef, group: &wgpu::BindGroup| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("grain_pass"), timestamp_writes: None });
            pass.set_pipeline(&pipeline.pipeline); pass.set_bind_group(0, group, &[]);
            dispatch_linear(&mut pass, img.pixel_count() as u32);
        };
        run(&mut encoder, &filter, &prepare_group);
        if let Some(filtered) = &filtered {
            let horizontal = filter_params(0); let vertical = filter_params(1);
            let h = bind(&filter, &[&horizontal, &prepared, &weights_buf, &output]);
            let v = bind(&filter, &[&vertical, &output, &weights_buf, filtered]);
            run(&mut encoder, &filter, &h); run(&mut encoder, &filter, &v);
        }
        let params_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_params"), contents: bytemuck::bytes_of(&shader_params), usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = bind(&grain, &[&params_buf, &prepared, &output, filtered.as_ref().unwrap_or(&prepared)]);
        run(&mut encoder, &grain, &group);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grain_readback"), size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        self.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { tx.send(r).unwrap(); });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();
        let output = f32_to_scalars(bytemuck::cast_slice(&data).to_vec());
        drop(data); readback.unmap();
        ImageBuf::from_data(img.width, img.height, output)
    }

}
