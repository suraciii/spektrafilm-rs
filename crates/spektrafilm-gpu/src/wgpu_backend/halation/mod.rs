use super::*;
use super::blur::{BlurJob, DispatchJob};


/// Pre-built halation passes — owns the per-sigma kernel buffers, params
/// buffers, and bind groups so they live long enough for the encoder.
///
/// The CPU equivalent is `spektrafilm_model::halation::apply_halation_um`.
/// All passes operate on the resident image buffer (passed in as `buf_b`),
/// using `buf_a` as blur intermediate and two newly-allocated buffers
/// `buf_c` / `buf_d` for scatter outputs and the halation accumulator.
#[cfg(feature = "wgpu-backend")]
pub(super) struct HalationState {
    // Owns all the auxiliary buffers + bind groups; only `encode_passes` is
    // called from the hot path.
    buf_c: wgpu::Buffer,
    buf_d: wgpu::Buffer,
    scatter_blurs: Vec<BlurJob>, // [core, tail]
    scatter_mix: DispatchJob,
    halation_blurs: Vec<BlurJob>,          // one per bounce
    halation_accumulate: Vec<DispatchJob>, // one add_scaled per bounce
    halation_final_add: DispatchJob,
    halation_renormalize: Option<DispatchJob>,
    blur_pipe_h: CachedPipelineRef,
    blur_pipe_v: CachedPipelineRef,
}


#[cfg(feature = "wgpu-backend")]
pub(super) fn build_halation_state(
    device: &wgpu::Device,
    hp: &crate::HalationGpuParams,
    width: u32,
    height: u32,
    buf_a: &wgpu::Buffer,
    buf_b: &wgpu::Buffer,
    backend: &WgpuBackend,
) -> HalationState {
    use wgpu::util::DeviceExt;
    let n_pixels = (width as usize) * (height as usize);
    let img_bytes = (n_pixels * 3 * 4) as u64;

    let mk_buf = |label: &str| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: img_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    };
    let buf_c = mk_buf("halation_c"); // scatter core / blur output
    let buf_d = mk_buf("halation_d"); // scatter tail / accumulator

    let blur_layout = &[
        wgpu::BufferBindingType::Uniform,
        wgpu::BufferBindingType::Storage { read_only: true },
        wgpu::BufferBindingType::Storage { read_only: true },
        wgpu::BufferBindingType::Storage { read_only: false },
    ];
    let blur_pipe_h = backend.cached_pipeline(
        include_str!("../blur/gaussian_blur_h.wgsl"),
        blur_layout,
    );
    let blur_pipe_v = backend.cached_pipeline(
        include_str!("../blur/gaussian_blur_v.wgsl"),
        blur_layout,
    );

    // Helper: build a single H/V blur job from source → output, using buf_a
    // (the freed RGB upload) as blur scratch (mid).
    let make_blur_job =
        |sigma: f32, src: &wgpu::Buffer, out: &wgpu::Buffer, label: &str| -> BlurJob {
            let sigma = sigma.max(0.01);
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
                label: Some(&format!("halation_blur_kernel_{label}")),
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
                label: Some(&format!("halation_blur_params_{label}")),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bg_h = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("halation_blur_h_{label}")),
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
                        resource: buf_a.as_entire_binding(),
                    }, // mid
                ],
            });
            let bg_v = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("halation_blur_v_{label}")),
                layout: &blur_pipe_v.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buf_a.as_entire_binding(),
                    }, // mid
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
        };

    // ── Scatter blurs (core → buf_c, tail → buf_d) ─────────────────────
    let scatter_blurs = vec![
        make_blur_job(hp.scatter_core_px, buf_b, &buf_c, "scatter_core"),
        make_blur_job(hp.scatter_tail_px, buf_b, &buf_d, "scatter_tail"),
    ];

    // ── Scatter mix: result = (1-sa)*result + sa*((1-atw)*core + atw*tail)
    let scatter_mix_pipe = backend.cached_pipeline(
        include_str!("scatter_mix.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct ScatterMixParams {
        n_pixels: u32,
        scatter_amount: f32,
        _pad0: u32,
        _pad1: u32,
        // vec4<f32>: per-channel tail weights, .w padding for 16-byte alignment
        tail_weight: [f32; 4],
    }
    let scatter_mix_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("halation_scatter_mix_params"),
        contents: bytemuck::bytes_of(&ScatterMixParams {
            n_pixels: n_pixels as u32,
            scatter_amount: hp.scatter_amount,
            _pad0: 0,
            _pad1: 0,
            tail_weight: [
                hp.scatter_tail_weight[0],
                hp.scatter_tail_weight[1],
                hp.scatter_tail_weight[2],
                0.0,
            ],
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let scatter_mix_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("halation_scatter_mix_bg"),
        layout: &scatter_mix_pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: scatter_mix_params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buf_c.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buf_d.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buf_b.as_entire_binding(),
            },
        ],
    });
    let scatter_mix_job = DispatchJob {
        _params_buf: scatter_mix_params,
        pipeline: scatter_mix_pipe,
        bg: scatter_mix_bg,
    };

    // ── Halation bounces ──────────────────────────────────────────────
    let n = hp.halation_n_bounces as usize;
    // Pre-compute normalized decay weights.
    let mut decay = vec![0.0f32; n];
    for k in 0..n {
        decay[k] = hp.halation_bounce_decay.powi(k as i32);
    }
    let decay_sum: f32 = decay.iter().sum();
    if decay_sum > 0.0 {
        for d in &mut decay {
            *d /= decay_sum;
        }
    }

    // Each bounce blurs buf_b → buf_c at sigma_k, then accumulates into
    // buf_d. First bounce sets `clear_first` so we don't need a separate
    // zero pass.
    let add_scaled_pipe = backend.cached_pipeline(
        include_str!("add_scaled.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct AddScaledParams {
        n_pixels: u32,
        scale: f32,
        clear_first: u32,
        _pad: u32,
    }

    let mut halation_blurs = Vec::with_capacity(n);
    let mut halation_accumulate = Vec::with_capacity(n);
    for (k, &wk) in decay.iter().enumerate() {
        let sigma_k = hp.halation_first_sigma_px * ((k as f32) + 1.0).sqrt();
        halation_blurs.push(make_blur_job(
            sigma_k,
            buf_b,
            &buf_c,
            &format!("bounce_{k}"),
        ));

        let acc_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("halation_acc_params_{k}")),
            contents: bytemuck::bytes_of(&AddScaledParams {
                n_pixels: n_pixels as u32,
                scale: wk,
                clear_first: if k == 0 { 1 } else { 0 },
                _pad: 0,
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let acc_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("halation_acc_bg_{k}")),
            layout: &add_scaled_pipe.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: acc_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_c.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buf_d.as_entire_binding(),
                },
            ],
        });
        halation_accumulate.push(DispatchJob {
            _params_buf: acc_params,
            pipeline: add_scaled_pipe.clone(),
            bg: acc_bg,
        });
    }

    // Final add: result[c] += a_tot[c] * accumulator[c]. Per-channel
    // because halation_strength varies dramatically across channels
    // (Portra zeros out blue entirely). Uses the per-channel variant
    // of add_scaled.
    let final_add_pipe = backend.cached_pipeline(
        include_str!("add_scaled_per_channel.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct AddScaledPerChannelParams {
        n_pixels: u32,
        _pad0: u32,
        _pad1: u32,
        _pad2: u32,
        scale: [f32; 4],
    }
    let final_add_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("halation_final_add_params"),
        contents: bytemuck::bytes_of(&AddScaledPerChannelParams {
            n_pixels: n_pixels as u32,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
            scale: [
                hp.halation_a_tot[0],
                hp.halation_a_tot[1],
                hp.halation_a_tot[2],
                0.0,
            ],
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let final_add_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("halation_final_add_bg"),
        layout: &final_add_pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: final_add_params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buf_d.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buf_b.as_entire_binding(),
            },
        ],
    });
    let halation_final_add = DispatchJob {
        _params_buf: final_add_params,
        pipeline: final_add_pipe,
        bg: final_add_bg,
    };

    // Per-channel renormalize: `result[c] /= 1 + a_tot[c]`. Uses the
    // pre-computed inverse factor passed in `inv_factor.xyz`.
    let halation_renormalize = if hp.halation_renormalize {
        let renorm_pipe = backend.cached_pipeline(
            include_str!("halation_renormalize.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct RenormParams {
            n_pixels: u32,
            _pad: u32,
            // f32x4: .xyz used, .w padding to align next vec4. Two u32
            // header fields above push this to offset 16, matching the
            // WGSL std140 layout.
            _gap: [u32; 2],
            inv_factor: [f32; 4],
        }
        let inv = [
            1.0 / (1.0 + hp.halation_a_tot[0]),
            1.0 / (1.0 + hp.halation_a_tot[1]),
            1.0 / (1.0 + hp.halation_a_tot[2]),
            0.0,
        ];
        let renorm_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("halation_renorm_params"),
            contents: bytemuck::bytes_of(&RenormParams {
                n_pixels: n_pixels as u32,
                _pad: 0,
                _gap: [0, 0],
                inv_factor: inv,
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let renorm_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("halation_renorm_bg"),
            layout: &renorm_pipe.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: renorm_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buf_b.as_entire_binding(),
                },
            ],
        });
        Some(DispatchJob {
            _params_buf: renorm_params,
            pipeline: renorm_pipe,
            bg: renorm_bg,
        })
    } else {
        None
    };

    HalationState {
        buf_c,
        buf_d,
        scatter_blurs,
        scatter_mix: scatter_mix_job,
        halation_blurs,
        halation_accumulate,
        halation_final_add,
        halation_renormalize,
        blur_pipe_h,
        blur_pipe_v,
    }
}

#[cfg(feature = "wgpu-backend")]
impl HalationState {
    pub(super) fn encode_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        n_pixels: u32,
        wg_xy: (u32, u32),
    ) {
        let _ = (&self.buf_c, &self.buf_d); // owned, just keepalive
        let dispatch_blur = |enc: &mut wgpu::CommandEncoder, job: &BlurJob| {
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("halation_blur_h"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.blur_pipe_h.pipeline);
                pass.set_bind_group(0, &job.bg_h, &[]);
                pass.dispatch_workgroups(wg_xy.0, wg_xy.1, 1);
            }
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("halation_blur_v"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.blur_pipe_v.pipeline);
                pass.set_bind_group(0, &job.bg_v, &[]);
                pass.dispatch_workgroups(wg_xy.0, wg_xy.1, 1);
            }
        };
        let dispatch_linear = |enc: &mut wgpu::CommandEncoder, job: &DispatchJob| {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("halation_linear"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&job.pipeline.pipeline);
            pass.set_bind_group(0, &job.bg, &[]);
            dispatch_linear(&mut pass, n_pixels);
        };

        // Scatter
        if !self.scatter_blurs.is_empty() {
            for j in &self.scatter_blurs {
                dispatch_blur(encoder, j);
            }
            dispatch_linear(encoder, &self.scatter_mix);
        }

        // Halation bounces
        for (blur_job, acc_job) in self
            .halation_blurs
            .iter()
            .zip(self.halation_accumulate.iter())
        {
            dispatch_blur(encoder, blur_job);
            dispatch_linear(encoder, acc_job);
        }
        if !self.halation_blurs.is_empty() {
            dispatch_linear(encoder, &self.halation_final_add);
            if let Some(rn) = &self.halation_renormalize {
                dispatch_linear(encoder, rn);
            }
        }
    }
}
