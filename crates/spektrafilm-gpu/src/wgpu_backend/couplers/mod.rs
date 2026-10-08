use super::*;
use super::blur::{BlurJob, DispatchJob};

/// Pre-built DIR coupler passes — owns scratch buffers, kernel buffers,
/// and bind groups so they live for the encoder.
///
/// CPU equivalent: `spektrafilm_model::couplers::apply_density_correction`.
/// Encodes per-pixel matmul → two Gaussian blurs of the correction →
/// weighted lerp via `add_scaled` → in-place subtract from log_raw via
/// `add_scaled(scale=-1)` → re-interpolation of `density_curves_0`.
#[cfg(feature = "wgpu-backend")]
pub(super) struct DirState {
    _buf_correction: wgpu::Buffer, // matmul output → tail_part (after blur2)
    _buf_gaussian: wgpu::Buffer,   // gaussian_part output (after blur1)
    _buf_mix: wgpu::Buffer,        // weighted lerp output → final correction
    matmul: DispatchJob,
    blur_gaussian: BlurJob,
    blur_tail: BlurJob,
    lerp_clear: DispatchJob,      // buf_mix = (1-w) * buf_gaussian
    lerp_accumulate: DispatchJob, // buf_mix += w * buf_correction (tail_part)
    subtract: DispatchJob,        // buf_b -= buf_mix
    density_curve_0: DispatchJob, // re-interp density curves
    blur_pipe_h: CachedPipelineRef,
    blur_pipe_v: CachedPipelineRef,
    /// Buffers we own that the bind groups reference (log_exp, curves) —
    /// kept alive for the encoder's lifetime.
    _owned: Vec<wgpu::Buffer>,
}

#[cfg(feature = "wgpu-backend")]
pub(super) fn build_dir_state(
    device: &wgpu::Device,
    dp: &crate::DirCouplersGpuParams<'_>,
    width: u32,
    height: u32,
    buf_a: &wgpu::Buffer,
    buf_b: &wgpu::Buffer,
    backend: &WgpuBackend,
) -> DirState {
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
    // buf_correction holds the matmul output, then is overwritten by the
    // tail blur (in-place via the V pass of a separable blur — H and V
    // are separate compute passes so wgpu does NOT see the input/output
    // as aliasing). buf_gaussian holds the first blur's output.
    // buf_mix holds the weighted lerp result that ultimately gets
    // subtracted from log_raw.
    let buf_correction = mk_buf("dir_correction");
    let buf_gaussian = mk_buf("dir_gaussian");
    let buf_mix = mk_buf("dir_mix");

    // ── 1. dir_matmul ─────────────────────────────────────────────────
    let matmul_pipe = backend.cached_pipeline(
        include_str!("dir_matmul.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct DirMatmulParams {
        n_pixels: u32,
        positive: u32,
        _pad0: u32,
        _pad1: u32,
        density_max: [f32; 4],
        m_row0: [f32; 4],
        m_row1: [f32; 4],
        m_row2: [f32; 4],
    }
    let m = &dp.couplers_matrix_scaled;
    let matmul_params = DirMatmulParams {
        n_pixels: n_pixels as u32,
        positive: if dp.is_positive { 1 } else { 0 },
        _pad0: 0,
        _pad1: 0,
        density_max: [dp.density_max[0], dp.density_max[1], dp.density_max[2], 0.0],
        m_row0: [m[0][0], m[0][1], m[0][2], 0.0],
        m_row1: [m[1][0], m[1][1], m[1][2], 0.0],
        m_row2: [m[2][0], m[2][1], m[2][2], 0.0],
    };
    let matmul_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dir_matmul_params"),
        contents: bytemuck::bytes_of(&matmul_params),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let matmul_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("dir_matmul_bg"),
        layout: &matmul_pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: matmul_params_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buf_a.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buf_correction.as_entire_binding(),
            },
        ],
    });
    let matmul = DispatchJob {
        _params_buf: matmul_params_buf,
        pipeline: matmul_pipe,
        bg: matmul_bg,
    };

    // ── 2. Two Gaussian blurs of the correction ───────────────────────
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

    // Blur reads `buf_correction`, uses `buf_a` as mid (free after matmul),
    // writes to `output`. Two blurs: gaussian → buf_correction (overwrite),
    // tail → buf_acc. Wait — we can't blur buf_correction in-place because
    // the V pass reads mid, and our first blur uses buf_a as mid, output to
    // … hmm we'd need a 3rd buffer or sequential reuse.
    //
    // Sequential reuse: blur1 reads buf_correction → writes mid into buf_a
    // → writes output into buf_acc. Then buf_correction is still untouched
    // and ready for blur2. Blur2 reads buf_correction → mid into buf_a →
    // output into… we already wrote buf_acc. Solution: swap roles. The
    // gaussian part can go into buf_acc, tail into buf_correction
    // (overwriting the matmul output — fine, we don't need it after both
    // blurs are done).
    //
    // After blur2 buf_acc = gaussian_part, buf_correction = tail_part.
    // The weighted lerp writes the result into buf_acc (`buf_acc *=
    // (1-w)` then `buf_acc += w * buf_correction`).
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
                label: Some(&format!("dir_blur_kernel_{label}")),
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
                label: Some(&format!("dir_blur_params_{label}")),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bg_h = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("dir_blur_h_{label}")),
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
                label: Some(&format!("dir_blur_v_{label}")),
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
    // Blur1: correction → buf_gaussian (gaussian_part)
    let blur_gaussian = make_blur_job(
        dp.diffusion_size_px,
        &buf_correction,
        &buf_gaussian,
        "gaussian",
    );
    // Blur2: correction → correction (in place; H writes mid via buf_a,
    // V reads mid and writes buf_correction in a separate compute pass
    // so wgpu does not see input/output aliasing).
    let blur_tail = make_blur_job(
        dp.diffusion_tail_px,
        &buf_correction,
        &buf_correction,
        "tail",
    );

    // ── 3. Weighted lerp via two add_scaled passes ────────────────────
    // buf_mix = (1-w) * buf_gaussian   (clear_first writes scale*src into dst)
    // buf_mix += w * buf_correction  (tail_part)
    let add_scaled_pipe = backend.cached_pipeline(
        include_str!("../halation/add_scaled.wgsl"),
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
    let w = dp.diffusion_tail_weight;
    let lerp_clear_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dir_lerp_clear_params"),
        contents: bytemuck::bytes_of(&AddScaledParams {
            n_pixels: n_pixels as u32,
            scale: 1.0 - w,
            clear_first: 1,
            _pad: 0,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    // src = buf_gaussian (read), dst = buf_mix (write). With clear_first=1
    // this is `buf_mix = (1-w) * buf_gaussian`. Different buffers, no
    // aliasing.
    let lerp_clear_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("dir_lerp_clear_bg"),
        layout: &add_scaled_pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: lerp_clear_params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buf_gaussian.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buf_mix.as_entire_binding(),
            },
        ],
    });
    let lerp_clear = DispatchJob {
        _params_buf: lerp_clear_params,
        pipeline: add_scaled_pipe.clone(),
        bg: lerp_clear_bg,
    };

    let lerp_acc_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dir_lerp_acc_params"),
        contents: bytemuck::bytes_of(&AddScaledParams {
            n_pixels: n_pixels as u32,
            scale: w,
            clear_first: 0,
            _pad: 0,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let lerp_acc_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("dir_lerp_acc_bg"),
        layout: &add_scaled_pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: lerp_acc_params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buf_correction.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buf_mix.as_entire_binding(),
            },
        ],
    });
    let lerp_accumulate = DispatchJob {
        _params_buf: lerp_acc_params,
        pipeline: add_scaled_pipe.clone(),
        bg: lerp_acc_bg,
    };

    // ── 4. Subtract: buf_b -= buf_mix ────────────────────────────────
    let subtract_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dir_subtract_params"),
        contents: bytemuck::bytes_of(&AddScaledParams {
            n_pixels: n_pixels as u32,
            scale: -1.0,
            clear_first: 0,
            _pad: 0,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let subtract_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("dir_subtract_bg"),
        layout: &add_scaled_pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: subtract_params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buf_mix.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buf_b.as_entire_binding(),
            },
        ],
    });
    let subtract = DispatchJob {
        _params_buf: subtract_params,
        pipeline: add_scaled_pipe,
        bg: subtract_bg,
    };

    // ── 5. Final density curve re-interp using density_curves_0 ───────
    let density_pipe = backend.cached_pipeline(
        include_str!("../../../../spektrafilm-shaders/wgsl/spectral/density_curve_interp.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let log_exp_f32: Vec<f32> = dp.log_exposure.iter().map(|&v| v as f32).collect();
    let curves_f32: Vec<f32> = dp
        .density_curves_0
        .iter()
        .flat_map(|r| r.iter().map(|&v| if v.is_nan() { 0.0 } else { v as f32 }))
        .collect();
    let log_exp_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dir_density_log_exp"),
        contents: bytemuck::cast_slice(&log_exp_f32),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let curves_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dir_density_curves_0"),
        contents: bytemuck::cast_slice(&curves_f32),
        usage: wgpu::BufferUsages::STORAGE,
    });
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
    let density_params = DensityParams {
        width,
        height,
        k: dp.log_exposure.len() as u32,
        uniform_grid: if is_uniform_grid_endpoint(dp.log_exposure) { 1 } else { 0 },
        gamma_inv: [(1.0 / dp.gamma_factor) as f32; 3],
        _pad: 0.0,
    };
    let density_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dir_density_params"),
        contents: bytemuck::bytes_of(&density_params),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let density_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("dir_density_bg"),
        layout: &density_pipe.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: density_params_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buf_b.as_entire_binding(),
            }, // input: corrected log_raw
            wgpu::BindGroupEntry {
                binding: 2,
                resource: log_exp_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: curves_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buf_a.as_entire_binding(),
            }, // output: corrected density_cmy
        ],
    });
    // Keep log_exp_buf / curves_buf alive via the DispatchJob's
    // _params_buf isn't ideal — pack them into the state instead.
    let density_curve_0 = DispatchJob {
        _params_buf: density_params_buf,
        pipeline: density_pipe,
        bg: density_bg,
    };

    // The two storage buffers (log_exp, curves) need to outlive the
    // encoder. Stuff them somewhere — extend DirState with a small
    // owner Vec.
    DirState {
        _buf_correction: buf_correction,
        _buf_gaussian: buf_gaussian,
        _buf_mix: buf_mix,
        matmul,
        blur_gaussian,
        blur_tail,
        lerp_clear,
        lerp_accumulate,
        subtract,
        density_curve_0,
        blur_pipe_h,
        blur_pipe_v,
        _owned: vec![log_exp_buf, curves_buf],
    }
}

#[cfg(feature = "wgpu-backend")]
impl DirState {
    pub(super) fn encode_passes(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        n_pixels: u32,
        wg_xy: (u32, u32),
    ) {
        let dispatch_linear = |enc: &mut wgpu::CommandEncoder, job: &DispatchJob| {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dir_linear"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&job.pipeline.pipeline);
            pass.set_bind_group(0, &job.bg, &[]);
            dispatch_linear(&mut pass, n_pixels);
        };
        let dispatch_blur = |enc: &mut wgpu::CommandEncoder, job: &BlurJob| {
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("dir_blur_h"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.blur_pipe_h.pipeline);
                pass.set_bind_group(0, &job.bg_h, &[]);
                pass.dispatch_workgroups(wg_xy.0, wg_xy.1, 1);
            }
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("dir_blur_v"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.blur_pipe_v.pipeline);
                pass.set_bind_group(0, &job.bg_v, &[]);
                pass.dispatch_workgroups(wg_xy.0, wg_xy.1, 1);
            }
        };

        dispatch_linear(encoder, &self.matmul);
        dispatch_blur(encoder, &self.blur_gaussian); // buf_correction → buf_acc
        dispatch_blur(encoder, &self.blur_tail); // buf_correction → buf_correction (overwrite)
        dispatch_linear(encoder, &self.lerp_clear); // buf_acc *= (1-w)
        dispatch_linear(encoder, &self.lerp_accumulate); // buf_acc += w * buf_correction
        dispatch_linear(encoder, &self.subtract); // buf_b -= buf_acc
        dispatch_linear(encoder, &self.density_curve_0); // buf_b → buf_a (corrected)
    }
}

