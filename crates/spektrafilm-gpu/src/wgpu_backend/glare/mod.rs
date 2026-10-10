use super::blur::{BlurJob, DispatchJob};
use super::*;

/// Pre-built glare pass — owns the noise-gen dispatch, optional blur,
/// and the additive RGB apply.
///
/// CPU equivalent:
/// `spektrafilm_model::glare::{compute_random_glare_amount,
/// add_glare_with_amount}`. Operates on buf_b (the RGB output of
/// scan_spectral) in place; the lognormal noise is held in a fresh
/// scratch buffer, blurred via the existing 3-channel separable
/// Gaussian.
#[cfg(feature = "wgpu-backend")]
pub(super) struct GlareState {
    _scratch: wgpu::Buffer,
    gen_dispatch: DispatchJob,
    blur: Option<BlurJob>,
    apply_dispatch: DispatchJob,
    blur_pipe_h: CachedPipelineRef,
    blur_pipe_v: CachedPipelineRef,
}

#[cfg(feature = "wgpu-backend")]
pub(super) fn build_glare_state(
    device: &ObservedDevice,
    gp: &crate::GlareGpuParams,
    width: u32,
    height: u32,
    buf_a: &wgpu::Buffer,
    buf_b: &wgpu::Buffer,
    backend: &WgpuBackend,
) -> GlareState {
    let n_pixels = (width as usize) * (height as usize);
    let img_bytes = (n_pixels * 3 * 4) as u64;

    let scratch = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("glare_scratch"),
        size: img_bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    // ── Noise generation ──────────────────────────────────────────────
    let gen_pipe = backend.cached_pipeline(
        include_str!("glare_gen.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct GenParams {
        n_pixels: u32,
        base_seed: u32,
        mu: f32,
        sigma: f32,
    }
    let gen_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("glare_gen_params"),
        contents: bytemuck::bytes_of(&GenParams {
            n_pixels: n_pixels as u32,
            base_seed: gp.base_seed,
            mu: gp.mu,
            sigma: gp.sigma,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let gen_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("glare_gen_bg"),
        layout: &gen_pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: gen_params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: scratch.as_entire_binding(),
            },
        ],
    });
    let gen_dispatch = DispatchJob {
        _params_buf: gen_params,
        pipeline: gen_pipe,
        bg: gen_bg,
    };

    // ── Optional blur (separable, in place on scratch via buf_a mid) ──
    let blur_layout = &[
        wgpu::BufferBindingType::Uniform,
        wgpu::BufferBindingType::Storage { read_only: true },
        wgpu::BufferBindingType::Storage { read_only: true },
        wgpu::BufferBindingType::Storage { read_only: false },
    ];
    let blur_pipe_h =
        backend.cached_pipeline(include_str!("../blur/gaussian_blur_h.wgsl"), blur_layout);
    let blur_pipe_v =
        backend.cached_pipeline(include_str!("../blur/gaussian_blur_v.wgsl"), blur_layout);
    let blur = if gp.blur_px > 0.0 {
        let sigma = gp.blur_px.max(0.01);
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

        let kernel_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("glare_blur_kernel"),
            contents: bytemuck::cast_slice(&kernel_f32),
            usage: wgpu::BufferUsages::STORAGE,
        });
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct BlurParams {
            width: u32,
            height: u32,
            radius: u32,
            _pad: u32,
        }
        let params = BlurParams {
            width,
            height,
            radius,
            _pad: 0,
        };
        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("glare_blur_params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bg_h = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glare_blur_h_bg"),
            layout: &blur_pipe_h.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: scratch.as_entire_binding(),
                }, // src (read)
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: kernel_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buf_a.as_entire_binding(),
                }, // mid
            ],
        });
        let bg_v = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glare_blur_v_bg"),
            layout: &blur_pipe_v.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_a.as_entire_binding(),
                }, // mid (read)
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: kernel_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: scratch.as_entire_binding(),
                }, // back into scratch (separate pass — no aliasing)
            ],
        });
        Some(BlurJob {
            _kernel_buf: kernel_buf,
            _params_buf: params_buf,
            bg_h,
            bg_v,
        })
    } else {
        None
    };

    // ── Apply: image[c] += glare_amount * rgb_offset[c] ───────────────
    let apply_pipe = backend.cached_pipeline(
        include_str!("glare_apply.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct ApplyParams {
        n_pixels: u32,
        _pad0: u32,
        _pad1: u32,
        _pad2: u32,
        offset: [f32; 4],
    }
    let apply_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("glare_apply_params"),
        contents: bytemuck::bytes_of(&ApplyParams {
            n_pixels: n_pixels as u32,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
            offset: [gp.rgb_offset[0], gp.rgb_offset[1], gp.rgb_offset[2], 0.0],
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let apply_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("glare_apply_bg"),
        layout: &apply_pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: apply_params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: scratch.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buf_b.as_entire_binding(),
            },
        ],
    });
    let apply_dispatch = DispatchJob {
        _params_buf: apply_params,
        pipeline: apply_pipe,
        bg: apply_bg,
    };

    GlareState {
        _scratch: scratch,
        gen_dispatch,
        blur,
        apply_dispatch,
        blur_pipe_h,
        blur_pipe_v,
    }
}

#[cfg(feature = "wgpu-backend")]
impl GlareState {
    pub(super) fn encode_passes(
        &self,
        encoder: &mut ObservedEncoder,
        n_pixels: u32,
        wg_xy: (u32, u32),
    ) {
        // 1. Noise generation.
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("glare_gen"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.gen_dispatch.pipeline.pipeline);
            pass.set_bind_group(0, &self.gen_dispatch.bg, &[]);
            dispatch_linear(&mut pass, n_pixels);
        }
        // 2. Optional blur.
        if let Some(b) = &self.blur {
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("glare_blur_h"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.blur_pipe_h.pipeline);
                pass.set_bind_group(0, &b.bg_h, &[]);
                pass.dispatch_workgroups(wg_xy.0, wg_xy.1, 1);
            }
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("glare_blur_v"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.blur_pipe_v.pipeline);
                pass.set_bind_group(0, &b.bg_v, &[]);
                pass.dispatch_workgroups(wg_xy.0, wg_xy.1, 1);
            }
        }
        // 3. Apply onto buf_b.
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("glare_apply"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.apply_dispatch.pipeline.pipeline);
            pass.set_bind_group(0, &self.apply_dispatch.bg, &[]);
            dispatch_linear(&mut pass, n_pixels);
        }
    }
}
