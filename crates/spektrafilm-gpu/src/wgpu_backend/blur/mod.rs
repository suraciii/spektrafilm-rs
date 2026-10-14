use super::*;

/// Shared uniform ABI for the horizontal and vertical FIR shaders.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct BlurParams {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) radius: u32,
    pub(super) _pad: u32,
}

/// Binding layout of `gaussian_blur_h.wgsl` / `gaussian_blur_v.wgsl`: params,
/// read-only source, read-only kernel, read-write destination. Every blur
/// caller must compile its pipeline with this layout; a second copy would let
/// a shader receive mismatched bind groups.
pub(super) const BLUR_LAYOUT: [wgpu::BufferBindingType; 4] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
];

/// Normalize in f64 before casting weights to the shader's f32 storage.
/// Callers retain their own sigma clamping and CPU fallback policy.
pub(super) fn gaussian_kernel(sigma: f32) -> (u32, Vec<f32>) {
    let radius = fir_blur_radius(sigma);
    let kernel_size = (2 * radius + 1) as usize;
    let sigma_f64 = sigma as f64;
    let two_sigma_sq = 2.0 * sigma_f64 * sigma_f64;
    let r_i32 = radius as i32;
    let mut kernel = Vec::with_capacity(kernel_size);
    for k in 0..kernel_size {
        let x = (k as i32 - r_i32) as f64;
        kernel.push((-x * x / two_sigma_sq).exp());
    }
    let sum: f64 = kernel.iter().sum();
    let kernel_f32 = kernel.into_iter().map(|v| (v / sum) as f32).collect();
    (radius, kernel_f32)
}

pub(super) struct BlurJob {
    pub(super) _kernel_buf: wgpu::Buffer,
    pub(super) _params_buf: wgpu::Buffer,
    pub(super) bg_h: wgpu::BindGroup,
    pub(super) bg_v: wgpu::BindGroup,
}

/// Build a single H/V blur job from source → output, using `mid` as scratch.
///
/// The caller must create the pipeline refs (`blur_pipe_h`, `blur_pipe_v`)
/// and the `out` buffer. The returned `BlurJob` owns the kernel buffer,
/// params buffer, and bind groups.
#[allow(clippy::too_many_arguments)]
pub(super) fn make_blur_job(
    backend: &WgpuBackend,
    sigma: f32,
    width: u32,
    height: u32,
    src: &wgpu::Buffer,
    mid: &wgpu::Buffer,
    out: &wgpu::Buffer,
    label: &str,
    blur_pipe_h: &CachedPipelineRef,
    blur_pipe_v: &CachedPipelineRef,
) -> BlurJob {
    let device = &backend.device;
    let sigma = sigma.max(0.01);
    let (radius, kernel_f32) = gaussian_kernel(sigma);

    // The weights depend only on sigma, so the kernel comes from the pipe's
    // cross-frame cache and every frame after the first reuses it (pipe
    // architecture F3a). The params carry the image size, so they stay
    // per-frame.
    let kernel_buf = backend.pipe.static_buffer(
        &format!("{label}_blur_kernel"),
        bytemuck::cast_slice(&kernel_f32),
        wgpu::BufferUsages::STORAGE,
    );
    let params = BlurParams {
        width,
        height,
        radius,
        _pad: 0,
    };
    let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(&format!("{label}_blur_params")),
        contents: bytemuck::bytes_of(&params),
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
                resource: out.as_entire_binding(),
            },
        ],
    });
    BlurJob {
        _kernel_buf: kernel_buf,
        _params_buf: params_buf,
        bg_h,
        bg_v,
    }
}

pub(super) struct DispatchJob {
    pub(super) _params_buf: wgpu::Buffer,
    pub(super) pipeline: CachedPipelineRef,
    pub(super) bg: wgpu::BindGroup,
}

pub(super) struct SimpleBlurState {
    _dst: wgpu::Buffer,
    blur: BlurJob,
    blur_pipe_h: CachedPipelineRef,
    blur_pipe_v: CachedPipelineRef,
    n_bytes: u64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_simple_blur_state(
    device: &ObservedDevice,
    sigma: f32,
    width: u32,
    height: u32,
    src: &wgpu::Buffer,
    mid: &wgpu::Buffer,
    label: &str,
    backend: &WgpuBackend,
) -> SimpleBlurState {
    let n_bytes = (width as u64) * (height as u64) * 3 * 4;
    let dst = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("{label}_blur_dst")),
        size: n_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let blur_pipe_h = backend.cached_pipeline(include_str!("gaussian_blur_h.wgsl"), &BLUR_LAYOUT);
    let blur_pipe_v = backend.cached_pipeline(include_str!("gaussian_blur_v.wgsl"), &BLUR_LAYOUT);

    let blur = make_blur_job(
        backend,
        sigma,
        width,
        height,
        src,
        mid,
        &dst,
        label,
        &blur_pipe_h,
        &blur_pipe_v,
    );

    SimpleBlurState {
        _dst: dst,
        blur,
        blur_pipe_h,
        blur_pipe_v,
        n_bytes,
    }
}

impl SimpleBlurState {
    pub(super) fn encode_passes(
        &self,
        encoder: &mut ObservedEncoder,
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
impl WgpuBackend {
    /// GPU separable Gaussian blur via two FIR passes (horizontal then vertical).
    pub fn gaussian_blur_gpu(&self, img: &ImageBuf, sigma: f32) -> ImageBuf {
        if !self.device.context.enabled() || sigma <= 0.0 || !crate::gpu_blur_supported(sigma) {
            return self.gaussian_blur_gpu_inner(img, sigma);
        }
        let (backend, _batch) = self.observed_batch("gaussian_blur");
        backend.gaussian_blur_gpu_inner(img, sigma)
    }
    /// Kernel weights are computed on CPU and uploaded as a storage buffer.
    /// Two ping-pong image buffers minimize allocations.
    fn gaussian_blur_gpu_inner(&self, img: &ImageBuf, sigma: f32) -> ImageBuf {
        if sigma <= 0.0 || !crate::gpu_blur_supported(sigma) {
            tracing::info!(target: "spektrafilm_gpu::wgpu_backend", sigma, execution = "cpu", "using faithful CPU Gaussian blur");
            return cpu_backend::CpuBackend.gaussian_blur(img, sigma);
        }
        let w = img.width;
        let h = img.height;
        let n_pixels = (w as usize) * (h as usize);
        let img_bytes = n_pixels * 3 * 4;

        let make_buf = |label: &str| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: img_bytes as u64,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let buf_in = make_buf("blur_in");
        let buf_mid = make_buf("blur_mid");
        let buf_out = make_buf("blur_out");

        let input_f32 = scalars_to_f32(&img.data);
        self.queue
            .write_buffer(&buf_in, 0, bytemuck::cast_slice(&input_f32));

        let h_pipe = self.cached_pipeline(include_str!("gaussian_blur_h.wgsl"), &BLUR_LAYOUT);
        let v_pipe = self.cached_pipeline(include_str!("gaussian_blur_v.wgsl"), &BLUR_LAYOUT);
        let blur = make_blur_job(
            self, sigma, w, h, &buf_in, &buf_mid, &buf_out, "blur", &h_pipe, &v_pipe,
        );

        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blur_readback"),
            size: img_bytes as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let wg_x = w.div_ceil(16);
        let wg_y = h.div_ceil(16);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("blur_h"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&h_pipe.pipeline);
            pass.set_bind_group(0, &blur.bg_h, &[]);
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("blur_v"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&v_pipe.pipeline);
            pass.set_bind_group(0, &blur.bg_v, &[]);
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }
        encoder.copy_buffer_to_buffer(&buf_out, 0, &readback, 0, img_bytes as u64);
        self.queue.submit(Some(encoder.finish()));

        let out_f32 = self.read_mapped_f32(&readback, img_bytes as u64);
        ImageBuf::from_data(w, h, f32_to_scalars(out_f32))
    }

    pub fn gaussian_blur_multi_gpu(&self, img: &ImageBuf, sigmas: &[f32]) -> Vec<ImageBuf> {
        if !self.device.context.enabled()
            || sigmas
                .iter()
                .any(|&s| s <= 0.0 || !crate::gpu_blur_supported(s))
        {
            return self.gaussian_blur_multi_gpu_inner(img, sigmas);
        }
        let (backend, _batch) = self.observed_batch("gaussian_blur_multi");
        backend.gaussian_blur_multi_gpu_inner(img, sigmas)
    }
    /// Blur `img` with every sigma in `sigmas`, all within a single command
    /// buffer. One upload, N pairs of H/V dispatches, one submit, one
    /// readback that fans out into N output `ImageBuf`s.
    ///
    /// Used by halation (multi-bounce blurs of the same source) and any
    /// caller that needs the same input at several radii.
    fn gaussian_blur_multi_gpu_inner(&self, img: &ImageBuf, sigmas: &[f32]) -> Vec<ImageBuf> {
        assert!(!sigmas.is_empty(), "gaussian_blur_multi_gpu: empty sigmas");
        if sigmas
            .iter()
            .any(|&sigma| sigma <= 0.0 || !crate::gpu_blur_supported(sigma))
        {
            tracing::info!(target: "spektrafilm_gpu::wgpu_backend", execution = "cpu", "using faithful CPU Gaussian blur batch");
            return cpu_backend::CpuBackend.gaussian_blur_multi(img, sigmas);
        }

        let w = img.width;
        let h = img.height;
        let n_pixels = (w as usize) * (h as usize);
        let img_bytes = n_pixels * 3 * 4;

        let make_buf = |label: &str| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: img_bytes as u64,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };

        // Input is uploaded once. Mid is shared between the H and V passes
        // of each sigma — wgpu inserts a barrier between compute passes so
        // pass N's H write of `mid` waits on pass N-1's V read.
        let buf_in = make_buf("blur_multi_in");
        let buf_mid = make_buf("blur_multi_mid");

        let input_f32 = scalars_to_f32(&img.data);
        self.queue
            .write_buffer(&buf_in, 0, bytemuck::cast_slice(&input_f32));

        // One output buffer per sigma.
        let bufs_out: Vec<_> = (0..sigmas.len())
            .map(|i| make_buf(&format!("blur_multi_out_{i}")))
            .collect();

        // Pipelines (cached across calls).
        let h_pipe = self.cached_pipeline(include_str!("gaussian_blur_h.wgsl"), &BLUR_LAYOUT);
        let v_pipe = self.cached_pipeline(include_str!("gaussian_blur_v.wgsl"), &BLUR_LAYOUT);

        // Pre-build per-sigma kernel/params/bind groups. Each job owns
        // the kernel/params buffers so they outlive the encoder.
        let per_sigma: Vec<BlurJob> = sigmas
            .iter()
            .enumerate()
            .map(|(i, &sigma)| {
                make_blur_job(
                    self,
                    sigma,
                    w,
                    h,
                    &buf_in,
                    &buf_mid,
                    &bufs_out[i],
                    &format!("blur_multi_{i}"),
                    &h_pipe,
                    &v_pipe,
                )
            })
            .collect();

        // One readback buffer per sigma — wgpu caps a single buffer at
        // ~256 MB on Metal, so a 4×6 MP combined readback would overflow.
        // Per-sigma buffers stay well under the cap (72 MB at 6 MP).
        let readbacks: Vec<_> = (0..sigmas.len())
            .map(|i| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("blur_multi_readback_{i}")),
                    size: img_bytes as u64,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();

        let wg_x = w.div_ceil(16);
        let wg_y = h.div_ceil(16);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for (i, ps) in per_sigma.iter().enumerate() {
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(&format!("blur_multi_h_{i}")),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&h_pipe.pipeline);
                pass.set_bind_group(0, &ps.bg_h, &[]);
                pass.dispatch_workgroups(wg_x, wg_y, 1);
            }
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some(&format!("blur_multi_v_{i}")),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&v_pipe.pipeline);
                pass.set_bind_group(0, &ps.bg_v, &[]);
                pass.dispatch_workgroups(wg_x, wg_y, 1);
            }
            encoder.copy_buffer_to_buffer(&bufs_out[i], 0, &readbacks[i], 0, img_bytes as u64);
        }
        // `per_sigma` and `bufs_out` are alive until the function returns,
        // which is after `queue.submit()` — so all referenced buffers stay
        // valid for the encoded work.
        self.queue.submit(Some(encoder.finish()));

        // Map every readback, then poll once.
        for rb in &readbacks {
            let slice = rb.slice(..);
            slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
        }
        self.device.poll(wgpu::Maintain::Wait);

        let mut out_imgs = Vec::with_capacity(sigmas.len());
        for rb in &readbacks {
            let slice = rb.slice(..);
            let data = slice.get_mapped_range();
            self.device.materialized(img_bytes as u64);
            let chunk: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
            out_imgs.push(ImageBuf::from_data(w, h, f32_to_scalars(chunk)));
            drop(data);
            rb.unmap();
        }

        out_imgs
    }
}
