use super::*;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ShaderParams {
    dimensions: [u32; 4],
    controls: [f32; 4],
    geometry: [f32; 4],
    flags: [f32; 4],
}

fn shader_params(width: u32, height: u32, params: &crate::GrainV2GpuParams) -> ShaderParams {
    let timer = params
        .timer
        .unwrap_or_else(|| spektrafilm_math::grain::seeded_phase(params.seed));
    ShaderParams {
        dimensions: [width, height, timer.to_bits(), params.mode],
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
    }
}

impl WgpuBackend {
    pub fn grain_v2_gpu(&self, img: &ImageBuf, params: &crate::GrainV2GpuParams) -> ImageBuf {
        if !self.device.context.enabled() || img.width == 0 || img.height == 0 {
            return self.grain_v2_gpu_inner(img, params);
        }
        let (backend, _batch) = self.observed_batch("grain_v2");
        backend.grain_v2_gpu_inner(img, params)
    }
    /// Execute Grain V2 on native encoded RGB with no transfer or primaries conversion.
    fn grain_v2_gpu_inner(&self, img: &ImageBuf, params: &crate::GrainV2GpuParams) -> ImageBuf {
        if img.width == 0 || img.height == 0 {
            return img.clone();
        }
        let input = scalars_to_f32(&img.data);
        let original = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grain_encoded_input"),
                contents: bytemuck::cast_slice(input.as_ref()),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let state = build_grain_v2_state(
            &self.device,
            params,
            img.width,
            img.height,
            &original,
            None,
            self,
        );
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grain_readback"),
            size: state.n_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        state.encode_pass(&mut encoder, img.pixel_count() as u32, &readback);
        self.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        self.device.materialized(state.n_bytes);
        let data = slice.get_mapped_range();
        let output = f32_to_scalars(bytemuck::cast_slice(&data).to_vec());
        drop(data);
        readback.unmap();
        ImageBuf::from_data(img.width, img.height, output)
    }
}

/// Shared standalone/resident preparation, optional Analogue resolution filter,
/// and Grain V2 execution. Noise remains a single-pass image-resolution path.
pub(super) struct GrainV2State {
    output: wgpu::Buffer,
    _buffers: Vec<wgpu::Buffer>,
    filter: CachedPipelineRef,
    grain: CachedPipelineRef,
    prepare_group: wgpu::BindGroup,
    filter_groups: Option<[wgpu::BindGroup; 2]>,
    grain_group: wgpu::BindGroup,
    n_bytes: u64,
}

pub(super) fn build_grain_v2_state(
    device: &ObservedDevice,
    params: &crate::GrainV2GpuParams,
    width: u32,
    height: u32,
    input: &wgpu::Buffer,
    output_space: Option<&spektrafilm_math::colorspace::RgbColorSpace>,
    backend: &WgpuBackend,
) -> GrainV2State {
    use spektrafilm_math::colorspace::Cctf;
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct FilterParams {
        dimensions: [u32; 4],
        taps: [u32; 4],
        matrix: [[f32; 4]; 3],
    }
    let n_bytes = u64::from(width) * u64::from(height) * 3 * 4;
    let create = |label| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: n_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    };
    let prepared = create("grain_prepared_input");
    let output = create("grain_output");
    let a = params.amount.clamp(0., 1.);
    let gsf = (5200. / width as f32).max(3100. / height as f32);
    let base = (1. + (params.raw_scale - 1.) / 47.)
        * (1. - params.resolution_factor.clamp(0., 100.) / 100.)
        / gsf;
    let resolution_scale = if params.resolution_type == 1 {
        1.2
    } else {
        1.6
    };
    let radius = if params.mode != 0 {
        0.
    } else {
        base * resolution_scale * (0.7 * a * a + 0.3 * a + 0.05)
    };
    let optical = params.resolution_type == 0;
    let weights: Vec<[f32; 2]> = if optical {
        spektrafilm_math::grain::optical_weights(radius)
            .into_iter()
            .map(|w| [w, 0.])
            .collect()
    } else {
        spektrafilm_math::grain::fast_blur_weights(radius)
    };
    let weights_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("grain_resolution_weights"),
        contents: bytemuck::cast_slice(&weights),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let storage = wgpu::BufferBindingType::Storage { read_only: true };
    let writable = wgpu::BufferBindingType::Storage { read_only: false };
    let uniform = wgpu::BufferBindingType::Uniform;
    let filter = backend.cached_pipeline(
        include_str!("film_resolution.wgsl"),
        &[uniform, storage, storage, writable],
    );
    let grain = backend.cached_pipeline(
        include_str!("grain_v2.wgsl"),
        &[uniform, storage, writable, storage],
    );
    let bind = |pipeline: &CachedPipelineRef, buffers: &[&wgpu::Buffer]| {
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grain_pass"),
            layout: &pipeline.layout,
            entries: &entries,
        })
    };
    let curve = output_space.map_or(0, |space| match space.cctf {
        Cctf::Linear => 1,
        Cctf::Acescct => 2,
        Cctf::Srgb => 3,
        Cctf::ProPhoto => 4,
        Cctf::Rec2020 => 5,
        Cctf::AdobeRgb1998 => 6,
        Cctf::Gamma2_6 => 7,
    });
    let matrix = output_space.map_or([[0.; 4]; 3], |space| {
        space
            .rgb_to_rgb_identity()
            .map(|row| [row[0] as f32, row[1] as f32, row[2] as f32, 0.])
    });
    let filter_params = |axis| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_filter_params"),
            contents: bytemuck::bytes_of(&FilterParams {
                dimensions: [width, height, axis, optical as u32],
                taps: [weights.len() as u32, curve, 0, 0],
                matrix,
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        })
    };
    let prepare_params = filter_params(2);
    let prepare_group = bind(&filter, &[&prepare_params, input, &weights_buf, &prepared]);
    let mut buffers = vec![prepare_params, weights_buf];
    let filtered = (radius > 0.).then(|| create("grain_filtered_input"));
    let filter_groups = filtered.as_ref().map(|filtered| {
        let horizontal = filter_params(0);
        let vertical = filter_params(1);
        let h = bind(&filter, &[&horizontal, &prepared, &buffers[1], &output]);
        let v = bind(&filter, &[&vertical, &output, &buffers[1], filtered]);
        buffers.extend([horizontal, vertical]);
        [h, v]
    });
    let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("grain_params"),
        contents: bytemuck::bytes_of(&shader_params(width, height, params)),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let grain_group = bind(
        &grain,
        &[
            &params_buf,
            &prepared,
            &output,
            filtered.as_ref().unwrap_or(&prepared),
        ],
    );
    buffers.extend([params_buf, prepared]);
    buffers.extend(filtered);
    GrainV2State {
        output,
        _buffers: buffers,
        filter,
        grain,
        prepare_group,
        filter_groups,
        grain_group,
        n_bytes,
    }
}

impl GrainV2State {
    pub(super) fn encode_pass(
        &self,
        encoder: &mut ObservedEncoder,
        n_pixels: u32,
        output: &wgpu::Buffer,
    ) {
        let run = |encoder: &mut ObservedEncoder,
                   pipeline: &CachedPipelineRef,
                   group: &wgpu::BindGroup| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("grain_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, group, &[]);
            dispatch_linear(&mut pass, n_pixels);
        };
        run(encoder, &self.filter, &self.prepare_group);
        if let Some([horizontal, vertical]) = &self.filter_groups {
            run(encoder, &self.filter, horizontal);
            run(encoder, &self.filter, vertical);
        }
        run(encoder, &self.grain, &self.grain_group);
        encoder.copy_buffer_to_buffer(&self.output, 0, output, 0, self.n_bytes);
    }
}
