use super::blur::{BlurJob, DispatchJob};
use super::*;

/// Pre-built DIR coupler passes — owns scratch buffers, kernel buffers,
/// and bind groups so they live for the encoder.
///
/// CPU equivalent: `spektrafilm_model::couplers::apply_density_correction`.
/// Encodes: per-pixel matmul (silver flip + donor Langmuir) → Gaussian
/// blur of the correction → weighted lerp of the 3-Gaussian exponential
/// tail → in-place subtract from log_raw with the receiver Langmuir →
/// re-interpolation of `density_curves_0`.
pub(super) struct DirState {
    _buf_correction: wgpu::Buffer, // raw matmul output (blur source)
    _buf_gaussian: wgpu::Buffer,   // blur scratch / gaussian_part
    _buf_mix: wgpu::Buffer,        // weighted blend → subtract source
    matmul: DispatchJob,
    /// `None` when the spatial blend is disabled (`diffusion_size_px <= 0`),
    /// matching the CPU's `if diffusion_size_pixel > 0.0` gate; the matmul
    /// then writes `buf_mix` directly.
    blur_gaussian: Option<BlurJob>,
    lerp_clear: Option<DispatchJob>, // buf_mix = (1-w) * buf_gaussian
    /// Exponential-tail components in `EXPONENTIAL_GAUSSIAN_FITS` order:
    /// optional blur into `buf_gaussian`, then the `w * amp` accumulate
    /// into `buf_mix` (from the pristine `buf_correction` when the
    /// component σ is non-positive — a degenerate identity blur).
    tail_ops: Vec<(Option<BlurJob>, DispatchJob)>,
    subtract: DispatchJob,        // buf_b -= receiver_langmuir?(buf_mix)
    density_curve_0: DispatchJob, // re-interp density curves
    blur_pipe_h: CachedPipelineRef,
    blur_pipe_v: CachedPipelineRef,
    /// Buffers we own that the bind groups reference (log_exp, curves) —
    /// kept alive for the encoder's lifetime.
    _owned: Vec<wgpu::Buffer>,
}

/// Sanitize an optional Langmuir knee into `(k, mask)` GPU uniforms:
/// `None` (no Langmuir on this side) and non-finite knees both mean
/// "linear" — the CPU short-circuits `k.is_infinite()` and the caller
/// passes `None` when the polarity has no donor/receiver term — so the
/// channel is masked off and the sanitized `k` keeps the
/// unconditionally-evaluated saturation term finite.
fn langmuir_uniforms(k: Option<[f32; 3]>) -> ([f32; 4], [f32; 4]) {
    let mut k_out = [1.0_f32; 4];
    let mut mask = [0.0_f32; 4];
    if let Some(k) = k {
        for c in 0..3 {
            if k[c].is_finite() {
                k_out[c] = k[c];
                mask[c] = 1.0;
            }
        }
    }
    (k_out, mask)
}

pub(super) fn build_dir_state(
    device: &ObservedDevice,
    dp: &crate::DirCouplersGpuParams<'_>,
    width: u32,
    height: u32,
    buf_a: &wgpu::Buffer,
    buf_b: &wgpu::Buffer,
    backend: &WgpuBackend,
) -> DirState {
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
    // buf_correction holds the pristine matmul output — every blur reads
    // it and writes buf_gaussian (reused per component, since each blur
    // completes before the next starts). buf_mix accumulates the weighted
    // blend that ultimately gets subtracted from log_raw.
    let buf_correction = mk_buf("dir_correction");
    let buf_gaussian = mk_buf("dir_gaussian");
    let buf_mix = mk_buf("dir_mix");

    // CPU gates the whole spatial blend on `diffusion_size_pixel > 0`
    // (`compute_exposure_correction`); without it the matmul output feeds
    // the subtract pass directly.
    let diffuse = dp.diffusion_size_px > 0.0;
    let matmul_target = if diffuse { &buf_correction } else { &buf_mix };

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
        donor_k: [f32; 4],
        donor_ref: [f32; 4],
        donor_mask: [f32; 4],
        m_row0: [f32; 4],
        m_row1: [f32; 4],
        m_row2: [f32; 4],
    }
    let m = &dp.couplers_matrix_scaled;
    let (donor_k, donor_mask) = langmuir_uniforms(dp.donor_k);
    let matmul_params = DirMatmulParams {
        n_pixels: n_pixels as u32,
        positive: if dp.is_positive { 1 } else { 0 },
        _pad0: 0,
        _pad1: 0,
        density_max: [dp.density_max[0], dp.density_max[1], dp.density_max[2], 0.0],
        donor_k,
        donor_ref: [dp.donor_ref[0], dp.donor_ref[1], dp.donor_ref[2], 0.0],
        donor_mask,
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
                resource: matmul_target.as_entire_binding(),
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
    let blur_pipe_h =
        backend.cached_pipeline(include_str!("../blur/gaussian_blur_h.wgsl"), blur_layout);
    let blur_pipe_v =
        backend.cached_pipeline(include_str!("../blur/gaussian_blur_v.wgsl"), blur_layout);

    // Each blur reads `buf_correction` (the matmul output, which stays
    // pristine — never the blur destination) and uses `buf_a` as the
    // horizontal mid buffer, free once the matmul has run.
    let make_blur = |sigma: f32, src: &wgpu::Buffer, out: &wgpu::Buffer, label: &str| -> BlurJob {
        blur::make_blur_job(
            device,
            sigma,
            width,
            height,
            src,
            buf_a,
            out,
            label,
            &blur_pipe_h,
            &blur_pipe_v,
        )
    };
    // Gaussian component of the blend: correction → buf_gaussian. Skipped
    // entirely (with the rest of the spatial blend) when the CPU gate is
    // off — `diffuse` is the mirror of `diffusion_size_pixel > 0`.
    let blur_gaussian = diffuse.then(|| {
        make_blur(
            dp.diffusion_size_px,
            &buf_correction,
            &buf_gaussian,
            "dir_blur_gaussian",
        )
    });

    // ── 3. Weighted blend via add_scaled passes ───────────────────────
    // buf_mix = (1-w) * buf_gaussian   (clear_first writes scale*src into dst)
    // buf_mix += (w * amp_k) * tail_k  (one accumulate per fitted component)
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
    let lerp_clear = diffuse.then(|| {
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
        DispatchJob {
            _params_buf: lerp_clear_params,
            pipeline: add_scaled_pipe.clone(),
            bg: lerp_clear_bg,
        }
    });

    // Exponential tail: the CPU's `exponential_filter_channel` is a fixed
    // 3-Gaussian mixture (Python `_EXPONENTIAL_GAUSSIAN_FITS[3]`), so the
    // GPU reproduces it as one blur plus one weighted accumulate per
    // component. A non-positive component σ degrades to an identity blur
    // on the CPU (`gaussian_blur_channel` clones its input), so the
    // accumulate reads the pristine correction directly instead of
    // jumping through the blur.
    let mut tail_ops: Vec<(Option<BlurJob>, DispatchJob)> = Vec::new();
    if diffuse {
        for (amp, ratio) in spektrafilm_math::gaussian::EXPONENTIAL_GAUSSIAN_FITS {
            let sigma = ratio as f32 * dp.diffusion_tail_px;
            let blur = (sigma > 0.0)
                .then(|| make_blur(sigma, &buf_correction, &buf_gaussian, "dir_blur_tail"));
            let src = if blur.is_some() {
                &buf_gaussian
            } else {
                &buf_correction
            };
            let accum_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dir_tail_acc_params"),
                contents: bytemuck::bytes_of(&AddScaledParams {
                    n_pixels: n_pixels as u32,
                    scale: (w as f64 * amp) as f32,
                    clear_first: 0,
                    _pad: 0,
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let accum_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dir_tail_acc_bg"),
                layout: &add_scaled_pipe.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: accum_params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: src.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: buf_mix.as_entire_binding(),
                    },
                ],
            });
            tail_ops.push((
                blur,
                DispatchJob {
                    _params_buf: accum_params,
                    pipeline: add_scaled_pipe.clone(),
                    bg: accum_bg,
                },
            ));
        }
    }

    // ── 4. Subtract: buf_b -= receiver_langmuir?(buf_mix) ────────────
    let subtract_pipe = backend.cached_pipeline(
        include_str!("dir_subtract.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct DirSubtractParams {
        n_pixels: u32,
        _pad0: u32,
        _pad1: u32,
        _pad2: u32,
        receiver_k: [f32; 4],
        receiver_ref: [f32; 4],
        receiver_mask: [f32; 4],
    }
    let (receiver_k, receiver_mask) = langmuir_uniforms(dp.receiver_knee);
    let subtract_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dir_subtract_params"),
        contents: bytemuck::bytes_of(&DirSubtractParams {
            n_pixels: n_pixels as u32,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
            receiver_k,
            receiver_ref: [
                dp.receiver_ref[0],
                dp.receiver_ref[1],
                dp.receiver_ref[2],
                0.0,
            ],
            receiver_mask,
        }),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let subtract_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("dir_subtract_bg"),
        layout: &subtract_pipe.layout,
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
        pipeline: subtract_pipe,
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
    let density_params = DensityParams {
        width,
        height,
        k: dp.log_exposure.len() as u32,
        uniform_grid: if is_uniform_grid_endpoint(dp.log_exposure) {
            1
        } else {
            0
        },
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
        lerp_clear,
        tail_ops,
        subtract,
        density_curve_0,
        blur_pipe_h,
        blur_pipe_v,
        _owned: vec![log_exp_buf, curves_buf],
    }
}

impl DirState {
    pub(super) fn encode_passes(
        &self,
        encoder: &mut ObservedEncoder,
        n_pixels: u32,
        wg_xy: (u32, u32),
    ) {
        let dispatch_linear = |enc: &mut ObservedEncoder, job: &DispatchJob| {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dir_linear"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&job.pipeline.pipeline);
            pass.set_bind_group(0, &job.bg, &[]);
            dispatch_linear(&mut pass, n_pixels);
        };
        let dispatch_blur = |enc: &mut ObservedEncoder, job: &BlurJob| {
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
        if let Some(job) = self.blur_gaussian.as_ref() {
            dispatch_blur(encoder, job);
        }
        if let Some(job) = self.lerp_clear.as_ref() {
            dispatch_linear(encoder, job); // buf_mix = (1-w) * buf_gaussian
        }
        for (blur, accum) in &self.tail_ops {
            if let Some(blur) = blur.as_ref() {
                dispatch_blur(encoder, blur); // buf_correction → buf_gaussian
            }
            dispatch_linear(encoder, accum); // buf_mix += w*amp_k * component
        }
        dispatch_linear(encoder, &self.subtract); // buf_b -= receiver?(buf_mix)
        dispatch_linear(encoder, &self.density_curve_0); // buf_b → buf_a (corrected)
    }
}

#[cfg(all(test, feature = "wgpu-backend"))]
mod tests {
    use super::super::observation::TEST_DEVICE_LOCK;
    use super::*;

    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct MatmulParams {
        n_pixels: u32,
        positive: u32,
        _pad0: u32,
        _pad1: u32,
        density_max: [f32; 4],
        donor_k: [f32; 4],
        donor_ref: [f32; 4],
        donor_mask: [f32; 4],
        m_row0: [f32; 4],
        m_row1: [f32; 4],
        m_row2: [f32; 4],
    }

    #[test]
    fn dir_matmul_shader_matches_reference_math() {
        let _guard = TEST_DEVICE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(b) = WgpuBackend::new() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        let density: Vec<f32> = vec![2.511903, 2.7011967, 0.26266688, 0.34336, 0.07575, 0.05212];
        let params = MatmulParams {
            n_pixels: 2,
            positive: 1,
            _pad0: 0,
            _pad1: 0,
            density_max: [2.511998, 2.8034478, 2.810454, 0.0],
            donor_k: [1.0; 4],
            donor_ref: [0.0; 4],
            donor_mask: [0.0; 4],
            m_row0: [0.341, 0.355, 0.305, 0.0],
            m_row1: [0.154, 0.324, 0.358, 0.0],
            m_row2: [0.171, 0.225, 0.273, 0.0],
        };
        let input = b.pipe.upload_image(&density, 2);
        let out = b.pipe.dispatch_device(
            include_str!("dir_matmul.wgsl"),
            &input,
            &[GpuBuffer::uniform(bytemuck::bytes_of(&params))],
            2,
            "dir_matmul_test",
        );
        let got = b.pipe.readback_image(&out);
        // silver = dmax - density; correction = M^T silver.
        let dmax = [2.511998f32, 2.8034478, 2.810454];
        let mut expected = Vec::new();
        for px in 0..2 {
            let s: Vec<f32> = (0..3).map(|c| dmax[c] - density[px * 3 + c]).collect();
            for m in 0..3 {
                expected.push(
                    s[0] * [0.341, 0.355, 0.305][m]
                        + s[1] * [0.154, 0.324, 0.358][m]
                        + s[2] * [0.171, 0.225, 0.273][m],
                );
            }
        }
        eprintln!("matmul got={got:?} expected={expected:?}");
        for (g, e) in got.iter().zip(expected.iter()) {
            assert!((g - e).abs() < 1e-5, "got {got:?} expected {expected:?}");
        }
    }

    #[test]
    fn langmuir_uniforms_masks_absent_and_non_finite_knees() {
        // No knee on this side (the caller passes `None`): the shader must
        // not apply the saturation, so every channel is masked off.
        let (k, mask) = langmuir_uniforms(None);
        assert_eq!(mask, [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(k, [1.0, 1.0, 1.0, 1.0]);
        // `K = inf` means "linear" on the CPU (`k.is_infinite()`); the
        // channel is masked off and the uniform stays finite so the
        // unconditionally-evaluated ratio cannot divide by infinity.
        let (k, mask) = langmuir_uniforms(Some([f64::INFINITY as f32, 1.5, 3.0]));
        assert_eq!(mask, [0.0, 1.0, 1.0, 0.0]);
        assert_eq!(k, [1.0, 1.5, 3.0, 1.0]);
        let (k, mask) = langmuir_uniforms(Some([0.0, 2.0, 4.0]));
        assert_eq!(mask, [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(k, [0.0, 2.0, 4.0, 1.0]);
    }

    #[test]
    fn dir_matmul_shader_applies_donor_langmuir_on_masked_channels() {
        let _guard = TEST_DEVICE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(b) = WgpuBackend::new() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        // Negative film at the acceptor, where the donor langmuir runs.
        let density: Vec<f32> = vec![0.4, 1.6, 2.4, 0.9, 0.2, 1.1];
        let donor_k = [1.0f32, 2.0, 1.0, 1.0];
        let donor_ref = [0.5f32, 1.0, 0.5, 0.0];
        // Channel 1 sits behind a non-finite knee: linear there.
        let donor_mask = [1.0f32, 0.0, 1.0, 0.0];
        let params = MatmulParams {
            n_pixels: 2,
            positive: 0,
            _pad0: 0,
            _pad1: 0,
            density_max: [1.0; 4],
            donor_k,
            donor_ref,
            donor_mask,
            m_row0: [0.341, 0.355, 0.305, 0.0],
            m_row1: [0.154, 0.324, 0.358, 0.0],
            m_row2: [0.171, 0.225, 0.273, 0.0],
        };
        let input = b.pipe.upload_image(&density, 2);
        let out = b.pipe.dispatch_device(
            include_str!("dir_matmul.wgsl"),
            &input,
            &[GpuBuffer::uniform(bytemuck::bytes_of(&params))],
            2,
            "dir_matmul_donor_test",
        );
        let got = b.pipe.readback_image(&out);
        let mut expected = Vec::new();
        for px in 0..2 {
            // silver = density, then g(v) = v*(k+ref)/(k+v) per masked channel.
            let s: Vec<f32> = (0..3)
                .map(|c| {
                    let v = density[px * 3 + c];
                    if donor_mask[c] > 0.5 {
                        v * (donor_k[c] + donor_ref[c]) / (donor_k[c] + v)
                    } else {
                        v
                    }
                })
                .collect();
            for m in 0..3 {
                expected.push(
                    s[0] * [0.341, 0.355, 0.305][m]
                        + s[1] * [0.154, 0.324, 0.358][m]
                        + s[2] * [0.171, 0.225, 0.273][m],
                );
            }
        }
        for (g, e) in got.iter().zip(expected.iter()) {
            assert!((g - e).abs() < 1e-5, "got {got:?} expected {expected:?}");
        }
    }
}
