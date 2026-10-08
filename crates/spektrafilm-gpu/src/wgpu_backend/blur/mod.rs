use super::*;

pub(super) struct BlurJob {
    pub(super) _kernel_buf: wgpu::Buffer,
    pub(super) _params_buf: wgpu::Buffer,
    pub(super) bg_h: wgpu::BindGroup,
    pub(super) bg_v: wgpu::BindGroup,
}

#[cfg(feature = "wgpu-backend")]
pub(super) struct DispatchJob {
    pub(super) _params_buf: wgpu::Buffer,
    pub(super) pipeline: CachedPipelineRef,
    pub(super) bg: wgpu::BindGroup,
}


#[cfg(feature = "wgpu-backend")]
pub(super) struct SimpleBlurState {
    _dst: wgpu::Buffer,
    blur: BlurJob,
    blur_pipe_h: CachedPipelineRef,
    blur_pipe_v: CachedPipelineRef,
    n_bytes: u64,
}

#[cfg(feature = "wgpu-backend")]
pub(super) fn build_simple_blur_state(
    device: &wgpu::Device,
    sigma: f32,
    width: u32,
    height: u32,
    src: &wgpu::Buffer,
    mid: &wgpu::Buffer,
    label: &str,
    backend: &WgpuBackend,
) -> SimpleBlurState {
    use wgpu::util::DeviceExt;
    let n_bytes = (width as u64) * (height as u64) * 3 * 4;
    let dst = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("{label}_blur_dst")),
        size: n_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let blur_layout = &[
        wgpu::BufferBindingType::Uniform,
        wgpu::BufferBindingType::Storage { read_only: true },
        wgpu::BufferBindingType::Storage { read_only: true },
        wgpu::BufferBindingType::Storage { read_only: false },
    ];
    let blur_pipe_h = backend.cached_pipeline(
        include_str!("gaussian_blur_h.wgsl"),
        blur_layout,
    );
    let blur_pipe_v = backend.cached_pipeline(
        include_str!("gaussian_blur_v.wgsl"),
        blur_layout,
    );

    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct BlurParams {
        width: u32,
        height: u32,
        radius: u32,
        _pad: u32,
    }
    let sigma = sigma.max(0.01);
    let radius = fir_blur_radius(sigma);
    let kernel_size = (2 * radius + 1) as usize;
    let two_sigma_sq = 2.0 * (sigma as f64) * (sigma as f64);
    let r_i32 = radius as i32;
    let mut kernel = Vec::with_capacity(kernel_size);
    for k in 0..kernel_size {
        let x = (k as i32 - r_i32) as f64;
        kernel.push((-x * x / two_sigma_sq).exp());
    }
    let ksum: f64 = kernel.iter().sum();
    let kernel_f32: Vec<f32> = kernel.into_iter().map(|v| (v / ksum) as f32).collect();
    let kernel_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(&format!("{label}_blur_kernel")),
        contents: bytemuck::cast_slice(&kernel_f32),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(&format!("{label}_blur_params")),
        contents: bytemuck::bytes_of(&BlurParams {
            width,
            height,
            radius,
            _pad: 0,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bg_h = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("{label}_blur_h_bg")),
        layout: &blur_pipe_h.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: src.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: kernel_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: mid.as_entire_binding(),
            },
        ],
    });
    let bg_v = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&format!("{label}_blur_v_bg")),
        layout: &blur_pipe_v.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: mid.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: kernel_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: dst.as_entire_binding(),
            },
        ],
    });

    SimpleBlurState {
        _dst: dst,
        blur: BlurJob {
            _kernel_buf: kernel_buf,
            _params_buf: params_buf,
            bg_h,
            bg_v,
        },
        blur_pipe_h,
        blur_pipe_v,
        n_bytes,
    }
}

#[cfg(feature = "wgpu-backend")]
impl SimpleBlurState {
    pub(super) fn encode_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        wg_xy: (u32, u32),
        dst_main: &wgpu::Buffer,
    ) {
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("simple_blur_h"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.blur_pipe_h.pipeline);
            pass.set_bind_group(0, &self.blur.bg_h, &[]);
            pass.dispatch_workgroups(wg_xy.0, wg_xy.1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("simple_blur_v"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.blur_pipe_v.pipeline);
            pass.set_bind_group(0, &self.blur.bg_v, &[]);
            pass.dispatch_workgroups(wg_xy.0, wg_xy.1, 1);
        }
        encoder.copy_buffer_to_buffer(&self._dst, 0, dst_main, 0, self.n_bytes);
    }
}
