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
    ShaderParams {
        dimensions: [width, height, params.seed, params.mode],
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
    /// Execute the standalone display-domain Grain V2 shader.
    pub fn grain_v2_gpu(&self, img: &ImageBuf, params: &crate::GrainV2GpuParams) -> ImageBuf {
        let shader_uniform = shader_params(img.width, img.height, params);
        let input = scalars_to_f32(&img.data);
        let output = vec![0u8; input.len() * std::mem::size_of::<f32>()];

        let output = self.dispatch_compute(
            include_str!("grain_v2.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&shader_uniform)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(input.as_ref())),
                GpuBuffer::storage_rw(&output),
            ],
            img.pixel_count() as u32,
            2,
        );
        ImageBuf::from_data(img.width, img.height, f32_to_scalars(output))
    }
}

/// Resident-chain Grain V2 pass. The standalone path uses a full
/// upload/dispatch/readback; this state keeps the image on the chain's
/// ping-pong buffers and copies only between GPU buffers.
pub(super) struct GrainV2State {
    _out_buf: wgpu::Buffer,
    _params_buf: wgpu::Buffer,
    pipeline: CachedPipelineRef,
    bind_group: wgpu::BindGroup,
    n_bytes: u64,
}

pub(super) fn build_grain_v2_state(
    device: &wgpu::Device,
    params: &crate::GrainV2GpuParams,
    width: u32,
    height: u32,
    input: &wgpu::Buffer,
    backend: &WgpuBackend,
) -> GrainV2State {
    use wgpu::util::DeviceExt;

    let n_pixels = width as usize * height as usize;
    let n_bytes = (n_pixels * 3 * std::mem::size_of::<f32>()) as u64;
    let out_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("grain_v2_resident_out"),
        size: n_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let shader_uniform = shader_params(width, height, params);
    let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("grain_v2_resident_params"),
        contents: bytemuck::bytes_of(&shader_uniform),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let pipeline = backend.cached_pipeline(
        include_str!("grain_v2.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("grain_v2_resident_bind_group"),
        layout: &pipeline.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: out_buf.as_entire_binding(),
            },
        ],
    });
    GrainV2State {
        _out_buf: out_buf,
        _params_buf: params_buf,
        pipeline,
        bind_group,
        n_bytes,
    }
}

impl GrainV2State {
    pub(super) fn encode_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        n_pixels: u32,
        output: &wgpu::Buffer,
    ) {
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("grain_v2_resident"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            dispatch_linear(&mut pass, n_pixels);
        }
        encoder.copy_buffer_to_buffer(&self._out_buf, 0, output, 0, self.n_bytes);
    }
}
