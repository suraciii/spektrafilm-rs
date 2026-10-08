/// wgpu compute backend — dispatches WGSL shaders on GPU via Metal/Vulkan/DX12.

#[cfg(feature = "wgpu-backend")]
use parking_lot::Mutex;
#[cfg(feature = "wgpu-backend")]
use spektrafilm_math::image::ImageBuf;
use std::borrow::Cow;

use crate::gpu_helpers::{
    f32_to_scalars, is_uniform_grid_endpoint, sanitize_spectral_inputs, scalars_to_f32,
};
#[cfg(feature = "wgpu-backend")]
use crate::{ComputeBackend, Lut3D, cpu_backend};

/// FIR Gaussian-blur half-width `ceil(3σ)`, hard-capped so a pathological σ
/// can never build a multi-thousand-tap kernel that hangs the GPU (a
/// monster kernel froze the display once). 256 → at
/// most a 513-tap separable kernel; well above any legitimate σ here
/// (halation tops out at tens of pixels).
const MAX_BLUR_RADIUS: u32 = 256;

// Keep in sync with the linear WGSL kernels. 256 is supported by the
// default device limits; a two-dimensional grid preserves large-image support.
#[cfg(feature = "wgpu-backend")]
const LINEAR_WORKGROUP_SIZE: u32 = 256;

#[cfg(feature = "wgpu-backend")]
fn dispatch_grid(workgroups: u32) -> (u32, u32) {
    let x = workgroups.clamp(1, 65535);
    (x, workgroups.div_ceil(x))
}

#[cfg(feature = "wgpu-backend")]
fn dispatch_linear(pass: &mut wgpu::ComputePass<'_>, n_values: u32) {
    let (x, y) = dispatch_grid(n_values.div_ceil(LINEAR_WORKGROUP_SIZE));
    pass.dispatch_workgroups(x, y, 1);
}

#[inline]
fn fir_blur_radius(sigma: f32) -> u32 {
    ((3.0_f32 * sigma).ceil() as u32).min(MAX_BLUR_RADIUS)
}

#[cfg(feature = "wgpu-backend")]
pub struct WgpuBackend {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Cache compiled compute pipelines keyed by shader source pointer.
    /// `&'static str` is fine because all our shader sources come from `include_str!`.
    pipeline_cache: Mutex<std::collections::HashMap<usize, CachedPipeline>>,
}

#[cfg(feature = "wgpu-backend")]
struct CachedPipeline {
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

#[cfg(feature = "wgpu-backend")]
impl WgpuBackend {
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))?;

        // The default `Limits` cap storage buffer bindings at 128 MB,
        // which a 6-channel ≥ 14 MP image exceeds (image_bytes =
        // width × height × 3 × 4). Bump every relevant limit up to the
        // adapter's hardware ceiling so we can render arbitrary
        // megapixel counts (within RAM).
        let adapter_info = adapter.get_info();
        let adapter_limits = adapter.limits();
        let mut limits = wgpu::Limits::default();
        limits.max_storage_buffer_binding_size = adapter_limits.max_storage_buffer_binding_size;
        limits.max_buffer_size = adapter_limits.max_buffer_size;
        limits.max_compute_workgroups_per_dimension =
            adapter_limits.max_compute_workgroups_per_dimension;
        limits.max_bind_groups = adapter_limits.max_bind_groups.max(limits.max_bind_groups);
        // Linear kernels use 256 invocations and blur kernels use 16 × 16,
        // both within the default compute workgroup limits.
        // MAPPABLE_PRIMARY_BUFFERS lets the input/output STORAGE buffers also
        // be MAP_WRITE / MAP_READ, so they map directly for a zero-copy
        // upload/readback. On unified-memory GPUs this skips the slow
        // Private↔Shared staging blits (~0.5 GB/s) that otherwise dominate the
        // per-frame cost. No-op (falls back to the blit path) if unsupported.
        let opt_feats = if adapter_info.device_type == wgpu::DeviceType::IntegratedGpu {
            wgpu::Features::MAPPABLE_PRIMARY_BUFFERS & adapter.features()
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("spektrafilm"),
                required_features: opt_feats,
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .ok()?;

        tracing::info!(
            adapter = adapter_info.name,
            backend = ?adapter_info.backend,
            "wgpu backend initialized"
        );

        Some(Self {
            device,
            queue,
            pipeline_cache: Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// Get or compile + cache a pipeline keyed by shader source pointer.
    /// All shader sources come from `include_str!` so the pointer is stable.
    fn get_or_compile_pipeline<F>(
        &self,
        shader_source: &'static str,
        layout_entries_fn: F,
    ) -> CachedPipelineRef
    where
        F: FnOnce() -> Vec<wgpu::BindGroupLayoutEntry>,
    {
        let key = shader_source.as_ptr() as usize;
        let mut cache = self.pipeline_cache.lock();
        if !cache.contains_key(&key) {
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("compute_shader"),
                    source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(shader_source)),
                });
            let entries = layout_entries_fn();
            let bind_group_layout =
                self.device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("compute_layout"),
                        entries: &entries,
                    });
            let pipeline_layout =
                self.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("compute_pipeline_layout"),
                        bind_group_layouts: &[&bind_group_layout],
                        push_constant_ranges: &[],
                    });
            let pipeline = self
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("compute_pipeline"),
                    layout: Some(&pipeline_layout),
                    module: &shader,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                });
            cache.insert(
                key,
                CachedPipeline {
                    bind_group_layout,
                    pipeline,
                },
            );
        }
        let cached = cache.get(&key).unwrap();
        CachedPipelineRef {
            pipeline: cached.pipeline.clone(),
            layout: cached.bind_group_layout.clone(),
        }
    }

    /// Generic GPU compute dispatch helper.
    ///
    /// `shader_source` must be a `'static str` (typically from `include_str!`) so
    /// the cache can key by pointer identity.
    fn dispatch_compute(
        &self,
        shader_source: &'static str,
        bindings: &[GpuBuffer],
        n_pixels: u32,
        output_idx: usize,
    ) -> Vec<f32> {
        let t_start = std::time::Instant::now();
        let layout_entries: Vec<wgpu::BindGroupLayoutEntry> = bindings
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupLayoutEntry {
                binding: i as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: b.binding_type,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let entries_for_compile = layout_entries.clone();
        let cached = self.get_or_compile_pipeline(shader_source, || entries_for_compile);
        let pipeline = &cached.pipeline;
        let bind_group_layout = &cached.layout;
        let t_compile = t_start.elapsed();

        // Create GPU buffers
        let gpu_buffers: Vec<wgpu::Buffer> = bindings
            .iter()
            .map(|b| {
                use wgpu::util::DeviceExt;
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("buffer"),
                        contents: &b.data,
                        usage: b.usage,
                    })
            })
            .collect();

        let bind_entries: Vec<wgpu::BindGroupEntry> = gpu_buffers
            .iter()
            .enumerate()
            .map(|(i, buf)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: buf.as_entire_binding(),
            })
            .collect();

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compute_bind_group"),
            layout: &bind_group_layout,
            entries: &bind_entries,
        });

        // Readback buffer
        let output_size = bindings[output_idx].data.len() as u64;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Dispatch
        // Linear kernels flatten a two-dimensional workgroup grid in WGSL.

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("compute_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            dispatch_linear(&mut pass, n_pixels);
        }
        encoder.copy_buffer_to_buffer(&gpu_buffers[output_idx], 0, &readback, 0, output_size);
        self.queue.submit(Some(encoder.finish()));

        // Read back
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();

        let data = slice.get_mapped_range();
        let result: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        readback.unmap();
        let _t_total = t_start.elapsed();
        let _ = t_compile;
        result
    }

    /// GPU separable Gaussian blur via two FIR passes (horizontal then vertical).
    /// Kernel weights are computed on CPU and uploaded as a storage buffer.
    /// Two ping-pong image buffers minimize allocations.
    pub fn gaussian_blur_gpu(&self, img: &ImageBuf, sigma: f32) -> ImageBuf {
        use wgpu::util::DeviceExt;
        if sigma <= 0.0 || !crate::gpu_blur_supported(sigma) {
            tracing::info!(sigma, execution = "cpu", "using faithful CPU Gaussian blur");
            return cpu_backend::CpuBackend.gaussian_blur(img, sigma);
        }
        let radius = fir_blur_radius(sigma);
        let kernel_size = (2 * radius + 1) as usize;

        // Pre-compute normalized Gaussian kernel on CPU.
        let sigma_f64 = sigma as f64;
        let two_sigma_sq = 2.0 * sigma_f64 * sigma_f64;
        let mut kernel = Vec::with_capacity(kernel_size);
        let r_i32 = radius as i32;
        for i in 0..kernel_size {
            let x = (i as i32 - r_i32) as f64;
            kernel.push((-x * x / two_sigma_sq).exp());
        }
        let sum: f64 = kernel.iter().sum();
        let kernel_f32: Vec<f32> = kernel.into_iter().map(|v| (v / sum) as f32).collect();

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

        let kernel_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gaussian_kernel"),
                contents: bytemuck::cast_slice(&kernel_f32),
                usage: wgpu::BufferUsages::STORAGE,
            });

        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            width: u32,
            height: u32,
            radius: u32,
            _pad: u32,
        }
        let params = Params {
            width: w,
            height: h,
            radius,
            _pad: 0,
        };
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gaussian_params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let layout = &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ];
        let h_pipe = self.cached_pipeline(include_str!("blur/gaussian_blur_h.wgsl"), layout);
        let v_pipe = self.cached_pipeline(include_str!("blur/gaussian_blur_v.wgsl"), layout);

        let bg_h = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg_blur_h"),
            layout: &h_pipe.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_in.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: kernel_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buf_mid.as_entire_binding(),
                },
            ],
        });
        let bg_v = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg_blur_v"),
            layout: &v_pipe.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_mid.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: kernel_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buf_out.as_entire_binding(),
                },
            ],
        });

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
            pass.set_bind_group(0, &bg_h, &[]);
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("blur_v"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&v_pipe.pipeline);
            pass.set_bind_group(0, &bg_v, &[]);
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }
        encoder.copy_buffer_to_buffer(&buf_out, 0, &readback, 0, img_bytes as u64);
        self.queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();
        let out_f32: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        readback.unmap();

        ImageBuf::from_data(w, h, f32_to_scalars(out_f32))
    }

    /// Blur `img` with every sigma in `sigmas`, all within a single command
    /// buffer. One upload, N pairs of H/V dispatches, one submit, one
    /// readback that fans out into N output `ImageBuf`s.
    ///
    /// Used by halation (multi-bounce blurs of the same source) and any
    /// caller that needs the same input at several radii.
    pub fn gaussian_blur_multi_gpu(&self, img: &ImageBuf, sigmas: &[f32]) -> Vec<ImageBuf> {
        use wgpu::util::DeviceExt;
        assert!(!sigmas.is_empty(), "gaussian_blur_multi_gpu: empty sigmas");
        if sigmas
            .iter()
            .any(|&sigma| sigma <= 0.0 || !crate::gpu_blur_supported(sigma))
        {
            tracing::info!(execution = "cpu", "using faithful CPU Gaussian blur batch");
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
        let layout = &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ];
        let h_pipe = self.cached_pipeline(include_str!("blur/gaussian_blur_h.wgsl"), layout);
        let v_pipe = self.cached_pipeline(include_str!("blur/gaussian_blur_v.wgsl"), layout);

        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            width: u32,
            height: u32,
            radius: u32,
            _pad: u32,
        }

        // Pre-build per-sigma kernel/params/bind groups. The struct owns
        // the kernel/params buffers so they outlive the encoder.
        #[allow(dead_code)]
        struct PerSigma {
            params_buf: wgpu::Buffer,
            kernel_buf: wgpu::Buffer,
            bg_h: wgpu::BindGroup,
            bg_v: wgpu::BindGroup,
        }

        let per_sigma: Vec<PerSigma> = sigmas
            .iter()
            .enumerate()
            .map(|(i, &sigma)| {
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
                let kernel_f32: Vec<f32> = kernel.into_iter().map(|v| (v / sum) as f32).collect();

                let kernel_buf =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("blur_multi_kernel_{i}")),
                            contents: bytemuck::cast_slice(&kernel_f32),
                            usage: wgpu::BufferUsages::STORAGE,
                        });

                let params = Params {
                    width: w,
                    height: h,
                    radius,
                    _pad: 0,
                };
                let params_buf =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("blur_multi_params_{i}")),
                            contents: bytemuck::bytes_of(&params),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });

                let bg_h = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!("bg_blur_multi_h_{i}")),
                    layout: &h_pipe.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: params_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: buf_in.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: kernel_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: buf_mid.as_entire_binding(),
                        },
                    ],
                });
                let bg_v = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!("bg_blur_multi_v_{i}")),
                    layout: &v_pipe.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: params_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: buf_mid.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: kernel_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: bufs_out[i].as_entire_binding(),
                        },
                    ],
                });

                PerSigma {
                    params_buf,
                    kernel_buf,
                    bg_h,
                    bg_v,
                }
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
            let chunk: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
            out_imgs.push(ImageBuf::from_data(w, h, f32_to_scalars(chunk)));
            drop(data);
            rb.unmap();
        }

        out_imgs
    }

    /// GPU-resident pipeline: runs the front pass (hanatos LUT lookup or
    /// mallett matmul), highlight boost, camera lens blur, halation,
    /// density curves, DIR, print spectral, scan spectral, glare,
    /// gamut compression, scanner lens blur,
    /// and unsharp as a single command buffer with ping-pong image storage.
    /// Only one upload at the start and one readback at the end.
    pub fn run_film_chain(&self, p: &crate::FilmChainParams<'_>) -> ImageBuf {
        use wgpu::util::DeviceExt;
        let t_start = std::time::Instant::now();
        // Pull all references into locals so the existing body below
        // doesn't need a rewrite — only the param sources change.
        let image = p.image;
        let film_log_exposure = p.film_log_exposure;
        let film_density_curves_normalized = p.film_density_curves_normalized;
        let film_gamma = p.film_gamma;
        let film_channel_density = p.film_channel_density;
        let film_base_density = p.film_base_density;
        let print_illuminant = p.print_illuminant;
        let print_sensitivity = p.print_sensitivity;
        let print_normalization_factor = p.print_normalization_factor;
        let print_log_exposure = p.print_log_exposure;
        let print_density_curves = p.print_density_curves;
        let print_gamma = p.print_gamma;
        let preflash = p.preflash;
        let print_channel_density = p.print_channel_density;
        let print_base_density = p.print_base_density;
        let viewing_illuminant = p.viewing_illuminant;
        let scan_normalization = p.scan_normalization;
        let scan_xyz_to_rgb = p.scan_xyz_to_rgb;

        let n_pixels = image.pixel_count() as u32;
        let img_bytes = n_pixels as usize * 3 * 4;

        // Ping-pong image buffers (each holds H*W*3 f32 values).
        let make_img_buf = |label: &str| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: img_bytes as u64,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let mappable = self
            .device
            .features()
            .contains(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS);

        // buf_a is the input/ping-pong buffer. The dominant per-frame cost was
        // the input upload: `queue.write_buffer` stages CPU→GPU through a slow
        // (~0.5 GB/s) blit on this unified-memory GPU. With
        // MAPPABLE_PRIMARY_BUFFERS we create buf_a already mapped (MAP_WRITE)
        // and memcpy the input straight into its (shared) memory — no staging.
        let input_f32 = scalars_to_f32(&image.data);
        let buf_a = if mappable {
            let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("img_a"),
                size: img_bytes as u64,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::MAP_WRITE,
                mapped_at_creation: true,
            });
            buf.slice(..)
                .get_mapped_range_mut()
                .copy_from_slice(bytemuck::cast_slice(&input_f32));
            buf.unmap();
            buf
        } else {
            let buf = make_img_buf("img_a");
            self.queue
                .write_buffer(&buf, 0, bytemuck::cast_slice(&input_f32));
            buf
        };

        // buf_b holds the final RGB. With MAPPABLE_PRIMARY_BUFFERS we add
        // MAP_READ so it can be mapped directly for a zero-copy readback,
        // skipping the slow Private→Shared blit on unified memory.
        let buf_b = {
            let mut usage = wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC;
            if mappable {
                usage |= wgpu::BufferUsages::MAP_READ;
            }
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("img_b"),
                size: img_bytes as u64,
                usage,
                mapped_at_creation: false,
            })
        };

        // Static (LUT) buffers — uploaded once.
        let mk_storage = |label: &str, bytes: &[u8]| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytes,
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let mk_uniform = |label: &str, bytes: &[u8]| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytes,
                    usage: wgpu::BufferUsages::UNIFORM,
                })
        };

        // ── Pre-compute every static GPU-side buffer ─────────────────────
        // Filming density curves — already normalized by caller.
        let film_log_exp_f32: Vec<f32> = film_log_exposure.iter().map(|&v| v as f32).collect();
        let film_curves_f32: Vec<f32> = film_density_curves_normalized
            .iter()
            .flat_map(|r| r.iter().map(|&v| if v.is_nan() { 0.0 } else { v as f32 }))
            .collect();
        let film_log_exp_buf = mk_storage("film_log_exp", bytemuck::cast_slice(&film_log_exp_f32));
        let film_curves_buf = mk_storage("film_curves", bytemuck::cast_slice(&film_curves_f32));

        // Film spectral data (for printing pass).
        let (film_cd_f32, mut film_bd_f32) = sanitize_spectral_inputs(
            film_channel_density,
            film_base_density,
            film_channel_density.len(),
        );
        film_bd_f32.resize(film_channel_density.len(), 0.0);
        let film_cd_buf = mk_storage("film_cd", bytemuck::cast_slice(&film_cd_f32));
        let film_bd_buf = mk_storage("film_bd", bytemuck::cast_slice(&film_bd_f32));

        let print_illu_f32: Vec<f32> = print_illuminant.iter().map(|&v| v as f32).collect();
        let print_sens_f32: Vec<f32> = print_sensitivity
            .iter()
            .flat_map(|r| r.iter().map(|&v| if v.is_nan() { 0.0 } else { v as f32 }))
            .collect();
        let print_illu_buf = mk_storage("print_illu", bytemuck::cast_slice(&print_illu_f32));
        let print_sens_buf = mk_storage("print_sens", bytemuck::cast_slice(&print_sens_f32));

        // Print density curves (RAW, no normalization for print path).
        let print_log_exp_f32: Vec<f32> = print_log_exposure.iter().map(|&v| v as f32).collect();
        let print_curves_f32: Vec<f32> = print_density_curves
            .iter()
            .flat_map(|r| r.iter().map(|&v| if v.is_nan() { 0.0 } else { v as f32 }))
            .collect();
        let print_log_exp_buf =
            mk_storage("print_log_exp", bytemuck::cast_slice(&print_log_exp_f32));
        let print_curves_buf = mk_storage("print_curves", bytemuck::cast_slice(&print_curves_f32));

        // Spectral dye-density data for the scanning pass. For scan_film we
        // scan the developed film directly, so feed the film's spectral
        // densities; otherwise the print's. (print_spectral uses
        // sensitivity, not these — this buffer feeds only scan_spectral.)
        let (scan_cd_src, scan_bd_src): (&[[f64; 3]], &[f64]) = if p.scan_film {
            (film_channel_density, film_base_density)
        } else {
            (print_channel_density, print_base_density)
        };
        let (scan_cd_f32, mut scan_bd_f32) =
            sanitize_spectral_inputs(scan_cd_src, scan_bd_src, scan_cd_src.len());
        scan_bd_f32.resize(scan_cd_src.len(), 0.0);
        let scan_cd_buf = mk_storage("scan_cd", bytemuck::cast_slice(&scan_cd_f32));
        let scan_bd_buf = mk_storage("scan_bd", bytemuck::cast_slice(&scan_bd_f32));

        let view_illu_f32: Vec<f32> = viewing_illuminant.iter().map(|&v| v as f32).collect();
        let view_illu_buf = mk_storage("view_illu", bytemuck::cast_slice(&view_illu_f32));
        let cmf_x_buf = mk_storage(
            "cmf_x",
            bytemuck::cast_slice(&spektrafilm_math::spectral::CMF_X),
        );
        let cmf_y_buf = mk_storage(
            "cmf_y",
            bytemuck::cast_slice(&spektrafilm_math::spectral::CMF_Y),
        );
        let cmf_z_buf = mk_storage(
            "cmf_z",
            bytemuck::cast_slice(&spektrafilm_math::spectral::CMF_Z),
        );

        // ── Param structs ────────────────────────────────────────────────
        // Front pass (hanatos LUT lookup or mallett matmul). Both shaders
        // share this uniform layout; `lut_size` is hanatos-only (0 for
        // mallett, where the field is a pad).
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct FrontParams {
            width: u32,
            height: u32,
            lut_size: u32,
            _pad: u32,
            col0: [f32; 4],
            col1: [f32; 4],
            col2: [f32; 4],
        }
        let (m, front_lut_size) = match &p.front {
            crate::FrontPass::Hanatos2025 {
                tc_lut,
                rgb_to_adapted_xyz,
            } => (rgb_to_adapted_xyz, tc_lut.size as u32),
            crate::FrontPass::Mallett2019 { matrix } => (matrix, 0u32),
        };
        let front_params = FrontParams {
            width: image.width,
            height: image.height,
            lut_size: front_lut_size,
            _pad: 0,
            col0: [m[0][0] as f32, m[1][0] as f32, m[2][0] as f32, 0.0],
            col1: [m[0][1] as f32, m[1][1] as f32, m[2][1] as f32, 0.0],
            col2: [m[0][2] as f32, m[1][2] as f32, m[2][2] as f32, 0.0],
        };
        let front_params_buf = mk_uniform("front_params", bytemuck::bytes_of(&front_params));

        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct DensityParams {
            width: u32,
            height: u32,
            k: u32,
            uniform_grid: u32,
            gamma_inv: [f32; 3],
            _pad: f32,
        }
        let film_density_params = DensityParams {
            width: image.width,
            height: image.height,
            k: film_log_exposure.len() as u32,
            uniform_grid: if is_uniform_grid_endpoint(film_log_exposure) {
                1
            } else {
                0
            },
            gamma_inv: [(1.0 / film_gamma) as f32; 3],
            _pad: 0.0,
        };
        let film_density_params_buf =
            mk_uniform("film_dens_params", bytemuck::bytes_of(&film_density_params));

        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct PrintParams {
            width: u32,
            height: u32,
            n_wavelengths: u32,
            normalization_factor: f32,
            preflash: [f32; 3],
            _pad: f32,
        }
        let print_params = PrintParams {
            width: image.width,
            height: image.height,
            n_wavelengths: film_channel_density.len() as u32,
            normalization_factor: print_normalization_factor as f32,
            preflash: [preflash[0] as f32, preflash[1] as f32, preflash[2] as f32],
            _pad: 0.0,
        };
        let print_params_buf = mk_uniform("print_params", bytemuck::bytes_of(&print_params));

        let print_density_params = DensityParams {
            width: image.width,
            height: image.height,
            k: print_log_exposure.len() as u32,
            uniform_grid: if is_uniform_grid_endpoint(print_log_exposure) {
                1
            } else {
                0
            },
            gamma_inv: [(1.0 / print_gamma) as f32; 3],
            _pad: 0.0,
        };
        let print_density_params_buf = mk_uniform(
            "print_dens_params",
            bytemuck::bytes_of(&print_density_params),
        );

        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct ScanParams {
            width: u32,
            height: u32,
            n_wavelengths: u32,
            normalization: f32,
            col0: [f32; 4],
            col1: [f32; 4],
            col2: [f32; 4],
            bw: [f32; 4],
        }
        let s = scan_xyz_to_rgb;
        // B&W/slide luminance remap (m, q); z=1 enables it in the shader.
        // Preserve floating range before gamut compression and destination encoding.
        let bw = match p.bw_xyz_remap {
            Some((m, q)) => [m as f32, q as f32, 1.0, 0.0],
            None => [1.0, 0.0, 0.0, 0.0],
        };
        let scan_params = ScanParams {
            width: image.width,
            height: image.height,
            n_wavelengths: scan_cd_src.len() as u32,
            normalization: scan_normalization as f32,
            col0: [s[0][0] as f32, s[1][0] as f32, s[2][0] as f32, 0.0],
            col1: [s[0][1] as f32, s[1][1] as f32, s[2][1] as f32, 0.0],
            col2: [s[0][2] as f32, s[1][2] as f32, s[2][2] as f32, 0.0],
            bw,
        };
        let scan_params_buf = mk_uniform("scan_params", bytemuck::bytes_of(&scan_params));

        // ── Pre-compile pipelines (cached after first call) ──────────────
        // Each shader's bindings layout is fixed and known here.
        let density_pipe = self.cached_pipeline(
            include_str!("../../../spektrafilm-shaders/wgsl/spectral/density_curve_interp.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let print_pipe = self.cached_pipeline(
            include_str!("../../../spektrafilm-shaders/wgsl/spectral/print_spectral.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let scan_pipe = self.cached_pipeline(
            include_str!("../../../spektrafilm-shaders/wgsl/spectral/scan_spectral.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );

        // ── Build bind groups (per dispatch, but no buffer creation) ─────
        // Front pass: hanatos (params + rgb_in + tc_lut + raw_out) or mallett
        // (params + rgb_in + raw_out). The tc_lut buffer rides along in the
        // tuple to outlive the encoder on the hanatos arm.
        let (front_pipe, bg_front, _front_tc_lut) = match &p.front {
            crate::FrontPass::Hanatos2025 { tc_lut, .. } => {
                let tc_lut_f32: Vec<f32> = tc_lut.data.iter().map(|&v| v as f32).collect();
                let tc_lut_buf = mk_storage("tc_lut", bytemuck::cast_slice(&tc_lut_f32));
                let pipe = self.cached_pipeline(
                    include_str!(
                        "../../../spektrafilm-shaders/wgsl/spectral/hanatos2025_rgb_to_raw.wgsl"
                    ),
                    &[
                        wgpu::BufferBindingType::Uniform,
                        wgpu::BufferBindingType::Storage { read_only: true },
                        wgpu::BufferBindingType::Storage { read_only: true },
                        wgpu::BufferBindingType::Storage { read_only: false },
                    ],
                );
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("bg_hanatos"),
                    layout: &pipe.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: front_params_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: buf_a.as_entire_binding(),
                        }, // rgb_in
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: tc_lut_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: buf_b.as_entire_binding(),
                        }, // raw_out
                    ],
                });
                (pipe, bg, Some(tc_lut_buf))
            }
            crate::FrontPass::Mallett2019 { .. } => {
                let pipe = self.cached_pipeline(
                    include_str!(
                        "../../../spektrafilm-shaders/wgsl/spectral/mallett_rgb_to_raw.wgsl"
                    ),
                    &[
                        wgpu::BufferBindingType::Uniform,
                        wgpu::BufferBindingType::Storage { read_only: true },
                        wgpu::BufferBindingType::Storage { read_only: false },
                    ],
                );
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("bg_mallett"),
                    layout: &pipe.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: front_params_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: buf_a.as_entire_binding(),
                        }, // rgb_in
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: buf_b.as_entire_binding(),
                        }, // raw_out
                    ],
                });
                (pipe, bg, None)
            }
        };
        // After the front pass: log10 + density curve interp into normalized
        // film curves. The front pass outputs raw (not log_raw), so a small
        // log10 shader transforms buf_b in-place, then density_curve_interp
        // reads buf_b → buf_a.

        let bg_log10 = {
            let pipe = self.cached_pipeline(
                include_str!("../../../spektrafilm-shaders/wgsl/spectral/log10_inplace.wgsl"),
                &[
                    wgpu::BufferBindingType::Uniform,
                    wgpu::BufferBindingType::Storage { read_only: false },
                ],
            );
            #[repr(C)]
            #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
            struct Log10Params {
                // WGSL struct: `n: u32 + _pad: vec3<u32>`. vec3 has 16-byte alignment,
                // so the struct is 32 bytes total. We pad on the Rust side accordingly.
                n_pixels: u32,
                _pad: [u32; 7],
            }
            let log10_params_buf = mk_uniform(
                "log10_params",
                bytemuck::bytes_of(&Log10Params {
                    n_pixels,
                    _pad: [0; 7],
                }),
            );
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bg_log10"),
                layout: &pipe.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: log10_params_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buf_b.as_entire_binding(),
                    },
                ],
            });
            // Return both the buffer (to keep it alive) and the bind group.
            (pipe, bg, log10_params_buf)
        };

        let bg_density_film = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg_density_film"),
            layout: &density_pipe.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: film_density_params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_b.as_entire_binding(),
                }, // log_raw
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: film_log_exp_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: film_curves_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buf_a.as_entire_binding(),
                }, // density_cmy
            ],
        });
        let bg_print = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg_print"),
            layout: &print_pipe.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: print_params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_a.as_entire_binding(),
                }, // density_cmy
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: film_cd_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: film_bd_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: print_illu_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: print_sens_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: buf_b.as_entire_binding(),
                }, // log_raw_print
            ],
        });
        let bg_density_print = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg_density_print"),
            layout: &density_pipe.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: print_density_params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_b.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: print_log_exp_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: print_curves_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buf_a.as_entire_binding(),
                },
            ],
        });
        let bg_scan = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg_scan"),
            layout: &scan_pipe.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: scan_params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_a.as_entire_binding(),
                }, // density_print
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: scan_cd_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: scan_bd_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: view_illu_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: cmf_x_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: cmf_y_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: cmf_z_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: buf_b.as_entire_binding(),
                }, // final rgb
            ],
        });

        // Readback buffer (for the final image only) — only needed when we
        // can't map buf_b directly. Skipping it on the mappable path also
        // avoids allocating a second full-image buffer per frame.
        let readback = (!mappable).then(|| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: img_bytes as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });

        // ── Halation auxiliary buffers + bind groups (only if active) ────
        // Allocated up-front so they live for the encoder. The two ping-pong
        // buffers `buf_a` / `buf_b` are reused as blur input/mid; the new
        // `buf_c` and `buf_d` hold the scatter outputs and the halation
        // accumulator. None of this is touched when `p.halation` is `None`.
        let halation_state = p.halation.as_ref().map(|hp| {
            build_halation_state(
                &self.device,
                hp,
                image.width,
                image.height,
                &buf_a,
                &buf_b,
                self,
            )
        });

        // ── Highlight boost state ────────────────────────────────────────
        // Runs on raw film exposure immediately after the front pass.
        let highlight_state = p
            .highlight_boost
            .as_ref()
            .map(|hp| build_highlight_boost_state(&self.device, hp, n_pixels, &buf_b, self));

        let camera_lens_blur_state = p.camera_lens_blur_px.and_then(|sigma| {
            (sigma > 0.0).then(|| {
                build_simple_blur_state(
                    &self.device,
                    sigma,
                    image.width,
                    image.height,
                    &buf_b,
                    &buf_a,
                    "camera_lens",
                    self,
                )
            })
        });

        // ── Unsharp mask state ───────────────────────────────────────────
        // Last pass before readback. Blurs buf_b → buf_c (via buf_a mid),
        // then combines: buf_b_out = (1+amount)*buf_b - amount*buf_c.
        // Since the combine writes back to buf_b in-place we need to
        // route via a temporary buffer to satisfy wgpu aliasing rules.
        let unsharp_state = p.unsharp.as_ref().map(|up| {
            build_unsharp_state(
                &self.device,
                up,
                image.width,
                image.height,
                &buf_a,
                &buf_b,
                self,
            )
        });
        // Grain V2 is a display-domain pass. Keep it inside the resident
        // command buffer so it does not force a full-image readback.
        let grain_v2_state = p.grain_v2.as_ref().map(|gp| {
            build_grain_v2_state(&self.device, gp, image.width, image.height, &buf_b, self)
        });

        // ── Output gamut compression state ───────────────────────────────
        // Single per-pixel dispatch in place on buf_b, after glare and
        // before unsharp (the CPU scanning order). The C_max table is
        // baked once at pipeline construction; only the upload happens here.
        let gamut_state = p
            .gamut
            .as_ref()
            .map(|gp| build_gamut_state(&self.device, gp, &buf_b, n_pixels, self));

        // ── Glare state ──────────────────────────────────────────────────
        // Applied in place on buf_b (the scan_spectral output) just before
        // readback. Generates per-pixel lognormal noise into a scratch
        // buffer, optionally blurs it, then adds `g * rgb_offset[c]` to
        // the image. Uses buf_a (free after scan_spectral consumed
        // density_cmy) and one fresh scratch buffer.
        let glare_state = p.glare.as_ref().map(|gp| {
            build_glare_state(
                &self.device,
                gp,
                image.width,
                image.height,
                &buf_a,
                &buf_b,
                self,
            )
        });

        let scanner_lens_blur_state = p.scanner_lens_blur_px.and_then(|sigma| {
            (sigma > 0.0).then(|| {
                build_simple_blur_state(
                    &self.device,
                    sigma,
                    image.width,
                    image.height,
                    &buf_b,
                    &buf_a,
                    "scanner_lens",
                    self,
                )
            })
        });

        // ── DIR couplers state ────────────────────────────────────────────
        // Allocated lazily when the DIR stage is active. Reads buf_a
        // (density_cmy from film density curve) and buf_b (log_raw),
        // produces a corrected buf_a via re-interpolation of
        // `density_curves_0` against `log_raw - correction`. Uses three
        // owned scratch buffers (correction, mid, accumulator). buf_a is
        // reused as blur mid since density_cmy is no longer needed once
        // the matmul has consumed it (the final density_curve_interp
        // overwrites buf_a anyway).
        let dir_state = p.dir_couplers.as_ref().map(|dp| {
            build_dir_state(
                &self.device,
                dp,
                image.width,
                image.height,
                &buf_a,
                &buf_b,
                self,
            )
        });

        // ── Single command buffer chaining everything ────────────────────
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let dispatch = |encoder: &mut wgpu::CommandEncoder,
                        pipe: &wgpu::ComputePipeline,
                        bg: &wgpu::BindGroup,
                        n: u32| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bg, &[]);
            dispatch_linear(&mut pass, n);
        };

        // 1. Front pass (hanatos or mallett): buf_a (rgb in) → buf_b (raw)
        dispatch(&mut encoder, &front_pipe.pipeline, &bg_front, n_pixels);
        // 1a. Highlight boost on raw, before optical scatter.
        if let Some(hs) = highlight_state.as_ref() {
            hs.encode_passes(&mut encoder);
        }
        // 1b. Camera lens blur on raw before halation.
        if let Some(bs) = camera_lens_blur_state.as_ref() {
            let wg_xy = (image.width.div_ceil(16), image.height.div_ceil(16));
            bs.encode_passes(&mut encoder, wg_xy, &buf_b);
        }
        // 1b. Halation in-place on buf_b. Uses buf_a as blur scratch (the
        //     input RGB image is no longer needed), buf_c / buf_d for the
        //     scatter outputs and halation accumulator.
        if let Some(hs) = halation_state.as_ref() {
            let wg_xy = (image.width.div_ceil(16), image.height.div_ceil(16));
            hs.encode_passes(&mut encoder, n_pixels, wg_xy);
        }
        // 2. log10 in-place on buf_b: raw → log_raw (3 channels per thread).
        let (log10_pipe, log10_bg, _keepalive) = &bg_log10;
        dispatch(&mut encoder, &log10_pipe.pipeline, log10_bg, n_pixels);
        // 3. Density curve (film, normalized): buf_b (log_raw) → buf_a (density_cmy)
        dispatch(
            &mut encoder,
            &density_pipe.pipeline,
            &bg_density_film,
            n_pixels,
        );
        // 3b. DIR couplers (operates on buf_a, mutates buf_b → log_raw_corrected,
        //     re-interps density curve back into buf_a).
        if let Some(ds) = dir_state.as_ref() {
            let wg_xy = (image.width.div_ceil(16), image.height.div_ceil(16));
            ds.encode_passes(&mut encoder, n_pixels, wg_xy);
        }
        // 4 + 5. Printing: print_spectral (buf_a → buf_b) then the print
        //     density curve (buf_b → buf_a). Skipped for scan_film — the
        //     scan pass consumes the film density already in buf_a.
        if !p.scan_film {
            // 4. Print spectral: buf_a → buf_b (log_raw_print)
            dispatch(&mut encoder, &print_pipe.pipeline, &bg_print, n_pixels);
            // 5. Density curve (print, raw curves): buf_b → buf_a (density_print)
            dispatch(
                &mut encoder,
                &density_pipe.pipeline,
                &bg_density_print,
                n_pixels,
            );
        }
        // 6. Scan spectral: buf_a → buf_b (linear RGB).
        dispatch(&mut encoder, &scan_pipe.pipeline, &bg_scan, n_pixels);
        // 6b. Glare (in place on buf_b).
        if let Some(gs) = glare_state.as_ref() {
            let wg_xy = (image.width.div_ceil(16), image.height.div_ceil(16));
            gs.encode_passes(&mut encoder, n_pixels, wg_xy);
        }
        // 6c. Output gamut compression (in place on buf_b) — after glare,
        //     before unsharp, mirroring the CPU scanning order.
        if let Some(gs) = gamut_state.as_ref() {
            gs.encode_passes(&mut encoder, n_pixels);
        }
        // 6d. Scanner lens blur — after glare/gamut, before unsharp.
        if let Some(bs) = scanner_lens_blur_state.as_ref() {
            let wg_xy = (image.width.div_ceil(16), image.height.div_ceil(16));
            bs.encode_passes(&mut encoder, wg_xy, &buf_b);
        }
        // 6d. Unsharp mask — last in-flight pass. Writes the final image
        //     back to buf_b so the readback path below is unchanged.
        if let Some(us) = unsharp_state.as_ref() {
            let wg_xy = (image.width.div_ceil(16), image.height.div_ceil(16));
            us.encode_passes(&mut encoder, n_pixels, wg_xy, &buf_b, img_bytes as u64);

        }
        // 6e. Grain V2 is the last resident pass, before CPU destination
        // transfer encoding. It writes back to the final ping-pong buffer.
        if let Some(gs) = grain_v2_state.as_ref() {
            gs.encode_pass(&mut encoder, n_pixels, &buf_b);
        }

        // Zero-copy path: when buf_b is mappable, skip the blit and map it
        // directly below. Otherwise stage it into the MAP_READ readback buffer.
        if let Some(rb) = readback.as_ref() {
            encoder.copy_buffer_to_buffer(&buf_b, 0, rb, 0, img_bytes as u64);
        }
        // Everything since function entry: CPU-side param prep, buffer
        // creation/uploads, and command encoding.
        let cpu_setup_ms = t_start.elapsed().as_secs_f32() * 1000.0;
        self.queue.submit(Some(encoder.finish()));

        // Single sync point at the end. Map buf_b directly on the zero-copy
        // path, or the staging buffer otherwise.
        let map_target = readback.as_ref().unwrap_or(&buf_b);
        let slice = map_target.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        let gpu_wait_ms = t_start.elapsed().as_secs_f32() * 1000.0 - cpu_setup_ms;
        let data = slice.get_mapped_range();
        let out_f32: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        map_target.unmap();

        let out = ImageBuf::from_data(image.width, image.height, f32_to_scalars(out_f32));
        tracing::debug!(
            cpu_setup_ms = format!("{cpu_setup_ms:.1}"),
            gpu_wait_ms = format!("{gpu_wait_ms:.1}"),
            readback_ms = format!(
                "{:.1}",
                t_start.elapsed().as_secs_f32() * 1000.0 - cpu_setup_ms - gpu_wait_ms
            ),
            "film chain timings"
        );
        out
    }

    /// Get-or-compile a pipeline by shader source + binding layout. Cached by
    /// shader source pointer.
    fn cached_pipeline(
        &self,
        shader_source: &'static str,
        binding_types: &[wgpu::BufferBindingType],
    ) -> CachedPipelineRef {
        let entries: Vec<wgpu::BindGroupLayoutEntry> = binding_types
            .iter()
            .enumerate()
            .map(|(i, &ty)| wgpu::BindGroupLayoutEntry {
                binding: i as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        self.get_or_compile_pipeline(shader_source, || entries)
    }
}

#[cfg(feature = "wgpu-backend")]
#[derive(Clone)]
struct CachedPipelineRef {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

#[cfg(feature = "wgpu-backend")]
struct GpuBuffer {
    data: Vec<u8>,
    binding_type: wgpu::BufferBindingType,
    usage: wgpu::BufferUsages,
}

#[cfg(feature = "wgpu-backend")]
impl GpuBuffer {
    fn uniform(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            binding_type: wgpu::BufferBindingType::Uniform,
            usage: wgpu::BufferUsages::UNIFORM,
        }
    }
    fn storage_ro(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            binding_type: wgpu::BufferBindingType::Storage { read_only: true },
            usage: wgpu::BufferUsages::STORAGE,
        }
    }
    fn storage_rw(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            binding_type: wgpu::BufferBindingType::Storage { read_only: false },
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        }
    }
}

#[cfg(feature = "wgpu-backend")]
impl ComputeBackend for WgpuBackend {
    fn colorspace_convert(&self, img: &ImageBuf, matrix: &[[f32; 3]; 3]) -> ImageBuf {
        cpu_backend::CpuBackend.colorspace_convert(img, matrix)
    }
    fn cctf_encode_srgb(&self, img: &ImageBuf) -> ImageBuf {
        cpu_backend::CpuBackend.cctf_encode_srgb(img)
    }
    fn cctf_decode_srgb(&self, img: &ImageBuf) -> ImageBuf {
        cpu_backend::CpuBackend.cctf_decode_srgb(img)
    }
    fn grain_v2(
        &self,
        img: &ImageBuf,
        params: &crate::GrainV2GpuParams,
    ) -> Option<ImageBuf> {
        // OpticalResolution is a CPU-only compatibility path. All bundled
        // Dehancer profiles use resolution_type=1 (FastBlur).
        (params.resolution_type == 1).then(|| self.grain_v2_gpu(img, params))
    }
    fn gaussian_blur(&self, img: &ImageBuf, sigma: f32) -> ImageBuf {
        if sigma <= 0.0 {
            return img.clone();
        }
        if !crate::gpu_blur_supported(sigma) {
            tracing::info!(
                sigma,
                execution = "cpu",
                "Gaussian blur exceeds GPU FIR support"
            );
            return cpu_backend::CpuBackend.gaussian_blur(img, sigma);
        }
        // For very small sigmas the FIR overhead dominates; CPU path is fine.
        // For practical halation/glare sigmas (1-40 pixels) the GPU is much faster.
        self.gaussian_blur_gpu(img, sigma)
    }
    fn gaussian_blur_multi(&self, img: &ImageBuf, sigmas: &[f32]) -> Vec<ImageBuf> {
        if sigmas.is_empty() {
            return Vec::new();
        }
        if sigmas
            .iter()
            .any(|&sigma| !crate::gpu_blur_supported(sigma))
        {
            tracing::info!(
                execution = "cpu",
                "Gaussian blur batch exceeds GPU FIR support"
            );
            return cpu_backend::CpuBackend.gaussian_blur_multi(img, sigmas);
        }
        self.gaussian_blur_multi_gpu(img, sigmas)
    }
    fn table_lookup(&self, img: &ImageBuf, table_x: &[f32], table_y: &[[f32; 3]]) -> ImageBuf {
        cpu_backend::CpuBackend.table_lookup(img, table_x, table_y)
    }
    fn lut3d_interp(&self, img: &ImageBuf, lut: &Lut3D) -> ImageBuf {
        cpu_backend::CpuBackend.lut3d_interp(img, lut)
    }

    fn scan_spectral(
        &self,
        density_cmy: &ImageBuf,
        channel_density: &[[f64; 3]],
        base_density: &[f64],
        illuminant: &[f64],
        normalization: f64,
        cat: &[[f64; 3]; 3],
        xyz_to_rgb: &[[f64; 3]; 3],
    ) -> ImageBuf {
        // GPU live-preview path collapses CAT and XYZ→RGB into a single
        // matrix — small precision drop acceptable for preview, matches
        // the same trade-off as `hanatos2025_rgb_to_raw`.
        let combined: [[f64; 3]; 3] = {
            let mut out = [[0.0f64; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    out[i][j] = xyz_to_rgb[i][0] * cat[0][j]
                        + xyz_to_rgb[i][1] * cat[1][j]
                        + xyz_to_rgb[i][2] * cat[2][j];
                }
            }
            out
        };
        let xyz_to_rgb = &combined;
        let n_wl = channel_density.len();
        let n_pixels = density_cmy.pixel_count() as u32;

        // Pack uniform params — mat3x3 in WGSL std140 is 3 columns of vec4.
        // GPU shaders are f32-only: cast precision-preserved inputs at the shader boundary.
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            width: u32,
            height: u32,
            n_wavelengths: u32,
            normalization: f32,
            col0: [f32; 4],
            col1: [f32; 4],
            col2: [f32; 4],
            bw: [f32; 4],
        }

        let params = Params {
            width: density_cmy.width,
            height: density_cmy.height,
            n_wavelengths: n_wl as u32,
            normalization: normalization as f32,
            col0: [
                xyz_to_rgb[0][0] as f32,
                xyz_to_rgb[1][0] as f32,
                xyz_to_rgb[2][0] as f32,
                0.0,
            ],
            col1: [
                xyz_to_rgb[0][1] as f32,
                xyz_to_rgb[1][1] as f32,
                xyz_to_rgb[2][1] as f32,
                0.0,
            ],
            col2: [
                xyz_to_rgb[0][2] as f32,
                xyz_to_rgb[1][2] as f32,
                xyz_to_rgb[2][2] as f32,
                0.0,
            ],
            // Standalone scan path carries no B&W remap (the resident chain
            // is the only caller that sets it). Identity / disabled.
            // Per-stage CPU callers consume linear scan RGB; final encoding
            // and clipping happen at the end of the scanning stage.
            bw: [1.0, 0.0, 0.0, 1.0],
        };

        // GPU shaders are f32-only — narrow f64 inputs at the shader boundary.
        // Metal's compiler runs fast-math (no-NaN assumption), so we sanitize CPU-side:
        // any wavelength with a NaN input has its `base_density` bumped to +1000 so
        // `pow(10, -d) ≈ 0`, zeroing that wavelength's contribution exactly the same
        // way Python's `density_to_light` zeros NaN-bearing wavelengths.
        let (cd_flat, mut bd) = sanitize_spectral_inputs(channel_density, base_density, n_wl);
        let illu_f32: Vec<f32> = illuminant.iter().map(|&v| v as f32).collect();
        bd.resize(n_wl, 0.0);
        let input_f32 = scalars_to_f32(&density_cmy.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];

        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/spectral/scan_spectral.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&cd_flat)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&bd)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&illu_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&spektrafilm_math::spectral::CMF_X)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&spektrafilm_math::spectral::CMF_Y)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&spektrafilm_math::spectral::CMF_Z)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            8, // output buffer index
        );

        ImageBuf::from_data(
            density_cmy.width,
            density_cmy.height,
            f32_to_scalars(result),
        )
    }

    fn print_spectral(
        &self,
        density_cmy: &ImageBuf,
        channel_density: &[[f64; 3]],
        base_density: &[f64],
        illuminant: &[f64],
        sensitivity: &[[f64; 3]],
        normalization_factor: f64,
        preflash: [f64; 3],
    ) -> ImageBuf {
        let n_wl = channel_density.len();
        let n_pixels = density_cmy.pixel_count() as u32;

        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            width: u32,
            height: u32,
            n_wavelengths: u32,
            normalization_factor: f32,
            preflash: [f32; 3],
            _pad: f32,
        }

        let params = Params {
            width: density_cmy.width,
            height: density_cmy.height,
            n_wavelengths: n_wl as u32,
            normalization_factor: normalization_factor as f32,
            preflash: [preflash[0] as f32, preflash[1] as f32, preflash[2] as f32],
            _pad: 0.0,
        };

        // GPU shaders are f32-only — narrow f64 inputs at the shader boundary.
        // See `sanitize_spectral_inputs` for the NaN-handling story (Metal fast-math).
        let (cd_flat, mut bd) = sanitize_spectral_inputs(channel_density, base_density, n_wl);
        let sens_flat: Vec<f32> = sensitivity
            .iter()
            .flat_map(|r| r.iter().map(|&v| if v.is_nan() { 0.0 } else { v as f32 }))
            .collect();
        bd.resize(n_wl, 0.0);
        let illu_f32: Vec<f32> = illuminant.iter().map(|&v| v as f32).collect();
        let input_f32 = scalars_to_f32(&density_cmy.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];

        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/spectral/print_spectral.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&cd_flat)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&bd)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&illu_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&sens_flat)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            6, // output buffer index
        );

        ImageBuf::from_data(
            density_cmy.width,
            density_cmy.height,
            f32_to_scalars(result),
        )
    }

    fn hanatos2025_rgb_to_raw(
        &self,
        image: &ImageBuf,
        tc_lut: &spektrafilm_math::spectral::TcLut,
        color_space: &str,
        ref_illuminant: &[f32],
        cat16: bool,
    ) -> ImageBuf {
        // GPU live-preview path collapses the two-step CAT02 adaptation
        // into a single matmul — small visible-spectrum precision drop
        // that's acceptable for preview. The CPU path keeps the two-step
        // for export bit-parity with Python.
        let rgb_to_adapted_xyz = spektrafilm_math::spectral::build_rgb_to_adapted_xyz(
            color_space,
            ref_illuminant,
            cat16,
        );
        let rgb_to_adapted_xyz = &rgb_to_adapted_xyz;
        let n_pixels = image.pixel_count() as u32;
        let lut_size = tc_lut.size as u32;

        // Pack uniform: mat3x3 in std140 = 3 vec4 columns. f32 only on GPU.
        // WGSL columns are read as columns of the matrix; storing row-major as
        // [col[0], col[1], col[2]] where col[j][i] = mat[i][j].
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            width: u32,
            height: u32,
            lut_size: u32,
            _pad: u32,
            col0: [f32; 4],
            col1: [f32; 4],
            col2: [f32; 4],
        }

        let m = rgb_to_adapted_xyz;
        let params = Params {
            width: image.width,
            height: image.height,
            lut_size,
            _pad: 0,
            col0: [m[0][0] as f32, m[1][0] as f32, m[2][0] as f32, 0.0],
            col1: [m[0][1] as f32, m[1][1] as f32, m[2][1] as f32, 0.0],
            col2: [m[0][2] as f32, m[1][2] as f32, m[2][2] as f32, 0.0],
        };

        // TC LUT data is f64 on disk; narrow to f32 at the GPU boundary.
        let tc_lut_f32: Vec<f32> = tc_lut.data.iter().map(|&v| v as f32).collect();
        let input_f32 = scalars_to_f32(&image.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];

        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/spectral/hanatos2025_rgb_to_raw.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&tc_lut_f32)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            3, // output buffer index
        );

        ImageBuf::from_data(image.width, image.height, f32_to_scalars(result))
    }

    fn density_curve_interp(
        &self,
        log_raw: &ImageBuf,
        log_exposure: &[f64],
        density_curves: &[[f64; 3]],
        gamma_factor: f64,
    ) -> ImageBuf {
        let n_pixels = log_raw.pixel_count() as u32;
        let k = log_exposure.len() as u32;

        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            width: u32,
            height: u32,
            k: u32,
            uniform_grid: u32,
            gamma_inv: [f32; 3],
            _pad: f32,
        }
        // Detect uniformly-spaced log_exposure (typical case) to use the fast path.
        let uniform = is_uniform_grid_endpoint(log_exposure);
        let gamma_inv = if gamma_factor.abs() > 1e-12 {
            (1.0 / gamma_factor) as f32
        } else {
            1.0
        };
        let params = Params {
            width: log_raw.width,
            height: log_raw.height,
            k,
            uniform_grid: if uniform { 1 } else { 0 },
            gamma_inv: [gamma_inv; 3],
            _pad: 0.0,
        };

        let log_exp_f32: Vec<f32> = log_exposure.iter().map(|&v| v as f32).collect();
        let curves_f32: Vec<f32> = density_curves
            .iter()
            .flat_map(|r| r.iter().map(|&v| if v.is_nan() { 0.0 } else { v as f32 }))
            .collect();
        let input_f32 = scalars_to_f32(&log_raw.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];

        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/spectral/density_curve_interp.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&log_exp_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&curves_f32)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            4, // output buffer index
        );

        ImageBuf::from_data(log_raw.width, log_raw.height, f32_to_scalars(result))
    }

    fn try_run_film_chain(&self, params: &crate::FilmChainParams<'_>) -> Option<ImageBuf> {
        if !params.gpu_blurs_supported() {
            tracing::info!(
                execution = "per_stage_cpu_blur",
                "resident blur exceeds GPU FIR support"
            );
            return None;
        }
        Some(self.run_film_chain(params))
    }

    fn is_gpu(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "wgpu (f32 preview)"
    }
}

#[cfg(feature = "wgpu-backend")]
mod blur;
#[cfg(feature = "wgpu-backend")]
mod couplers;
#[cfg(feature = "wgpu-backend")]
mod gamut;
#[cfg(feature = "wgpu-backend")]
mod glare;
#[cfg(feature = "wgpu-backend")]
mod grain;
#[cfg(feature = "wgpu-backend")]
mod halation;
#[cfg(feature = "wgpu-backend")]
mod highlight;
#[cfg(feature = "wgpu-backend")]
mod unsharp;
#[cfg(feature = "wgpu-backend")]
use blur::*;
#[cfg(feature = "wgpu-backend")]
use couplers::*;
#[cfg(feature = "wgpu-backend")]
use gamut::*;
#[cfg(feature = "wgpu-backend")]
use glare::*;
#[cfg(feature = "wgpu-backend")]
use grain::*;
#[cfg(feature = "wgpu-backend")]

use halation::*;
#[cfg(feature = "wgpu-backend")]
use highlight::*;
#[cfg(feature = "wgpu-backend")]
use unsharp::*;
