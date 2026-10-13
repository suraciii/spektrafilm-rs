use super::ObservedEncoder;
use super::{
    ImageBuf, WgpuBackend, build_dir_state, build_gamut_state, build_glare_state,
    build_grain_v2_state, build_halation_state, build_highlight_boost_state,
    build_simple_blur_state, build_unsharp_state, dispatch_linear, f32_to_scalars,
    is_uniform_grid_endpoint, sanitize_spectral_inputs, scalars_to_f32,
};

impl WgpuBackend {
    pub fn run_film_chain(&self, p: &crate::FilmChainParams<'_>) -> ImageBuf {
        if !self.device.context.enabled() {
            return self.run_film_chain_inner(p);
        }
        let (backend, _batch) = self.observed_batch("film_chain");
        backend.run_film_chain_inner(p)
    }
    /// GPU-resident pipeline: runs the front pass (hanatos LUT lookup or
    /// mallett matmul), highlight boost, camera lens blur, halation,
    /// density curves, DIR, print spectral, scan spectral, glare,
    /// gamut compression, scanner lens blur,
    /// and unsharp as a single command buffer with ping-pong image storage.
    /// Only one upload at the start and one readback at the end.
    fn run_film_chain_inner(&self, p: &crate::FilmChainParams<'_>) -> ImageBuf {
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
            self.device
                .upload(img_bytes as u64, crate::telemetry::UploadCategory::Image);
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
            exposure_scale: f32,
            _pad: [f32; 3],
            preflash: [f32; 3],
            _tail_pad: f32,
        }
        let print_params = PrintParams {
            width: image.width,
            height: image.height,
            n_wavelengths: film_channel_density.len() as u32,
            normalization_factor: p.print_normalization_factor as f32,
            exposure_scale: p.print_exposure_scale as f32,
            _pad: [0.0; 3],
            preflash: [preflash[0] as f32, preflash[1] as f32, preflash[2] as f32],
            _tail_pad: 0.0,
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
            include_str!(
                "../../../spektrafilm-shaders/wgsl/spectral/print_spectral_calibrated.wgsl"
            ),
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
        let mut halation_state = p.halation.as_ref().map(|hp| {
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
        let mut highlight_state = p
            .highlight_boost
            .as_ref()
            .map(|hp| build_highlight_boost_state(&self.device, hp, n_pixels, &buf_b, self));

        let mut camera_lens_blur_state = p.camera_lens_blur_px.and_then(|sigma| {
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
        let mut unsharp_state = p.unsharp.as_ref().map(|up| {
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
        // Encode the destination space before Grain Procedural preparation; all
        // resolution and grain passes stay in this resident command buffer.
        let mut grain_v2_state = p.grain_v2.as_ref().map(|gp| {
            build_grain_v2_state(
                &self.device,
                gp,
                image.width,
                image.height,
                &buf_b,
                Some(p.scan_output_space),
                self,
            )
        });

        // ── Output gamut compression state ───────────────────────────────
        // Single per-pixel dispatch in place on buf_b, after glare and
        // before unsharp (the CPU scanning order). The C_max table is
        // baked once at pipeline construction; only the upload happens here.
        let mut gamut_state = p
            .gamut
            .as_ref()
            .map(|gp| build_gamut_state(&self.device, gp, &buf_b, n_pixels, self));

        // ── Glare state ──────────────────────────────────────────────────
        // Applied in place on buf_b (the scan_spectral output) just before
        // readback. Generates per-pixel lognormal noise into a scratch
        // buffer, optionally blurs it, then adds `g * rgb_offset[c]` to
        // the image. Uses buf_a (free after scan_spectral consumed
        // density_cmy) and one fresh scratch buffer.
        let mut glare_state = p.glare.as_ref().map(|gp| {
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

        let mut scanner_lens_blur_state = p.scanner_lens_blur_px.and_then(|sigma| {
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
        let mut dir_state = p.dir_couplers.as_ref().map(|dp| {
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

        // ── Assemble the stage list, then run it on the pipe ────────────
        // Consecutive GPU nodes share one command buffer and one submission,
        // exactly like the former fused inline sequence. Each closure owns
        // the bind groups and state it encodes; buffers ride along as
        // Arc-cloned handles (wgpu resources are cheap clones).
        let wg_xy = (image.width.div_ceil(16), image.height.div_ceil(16));
        let mut stages: Vec<super::pipe::ChainStage<'_>> = Vec::new();
        let pass = |name: &'static str,
                    pipeline: wgpu::ComputePipeline,
                    bg: wgpu::BindGroup,
                    n: u32|
         -> super::pipe::ChainStage<'static> {
            super::pipe::ChainStage::Gpu {
                name,
                encode: Box::new(move |encoder| {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some(name),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &bg, &[]);
                    dispatch_linear(&mut pass, n);
                }),
            }
        };

        // 1. Front pass (hanatos or mallett): buf_a (rgb in) → buf_b (raw)
        stages.push(pass(
            "front_transform",
            front_pipe.pipeline.clone(),
            bg_front,
            n_pixels,
        ));
        // 1a. Highlight boost on raw, before optical scatter.
        if let Some(hs) = highlight_state.take() {
            stages.push(super::pipe::ChainStage::Gpu {
                name: "highlight_boost",
                encode: Box::new(move |encoder| hs.encode_passes(encoder)),
            });
        }
        // 1a'. Capability fence (pipe architecture, S3): the camera
        //     diffusion filter's discrete PSF convolution runs on the host
        //     over the linear raw — after highlight reconstruction, before
        //     the camera lens blur (the CPU filming order).
        if let Some(cpu) = p.camera_diffusion_cpu_stage.as_ref() {
            let buf_b = buf_b.clone();
            let (w, h) = (image.width, image.height);
            let img_bytes_u64 = img_bytes as u64;
            stages.push(super::pipe::ChainStage::Cpu {
                name: "camera_diffusion_cpu_stage",
                fence: Box::new(move |pipe| {
                    let host = pipe.readback_buffer(&buf_b, img_bytes_u64);
                    let diffused = cpu(&ImageBuf::from_data(
                        w,
                        h,
                        crate::gpu_helpers::f32_to_scalars(host),
                    ));
                    let out_f32 = crate::gpu_helpers::scalars_to_f32(&diffused.data);
                    pipe.write_image_buffer(&buf_b, &out_f32);
                }),
            });
        }
        // 1b. Camera lens blur on raw before halation.
        if let Some(bs) = camera_lens_blur_state.take() {
            let buf_b = buf_b.clone();
            stages.push(super::pipe::ChainStage::Gpu {
                name: "camera_lens_blur",
                encode: Box::new(move |encoder| bs.encode_passes(encoder, wg_xy, &buf_b)),
            });
        }
        // 1c. Halation in-place on buf_b (blur scratch in buf_a, scatter
        //     outputs and accumulator in buf_c / buf_d).
        if let Some(hs) = halation_state.take() {
            stages.push(super::pipe::ChainStage::Gpu {
                name: "halation",
                encode: Box::new(move |encoder| hs.encode_passes(encoder, n_pixels, wg_xy)),
            });
        }
        // 2. log10 in-place on buf_b: raw → log_raw (3 channels per thread).
        {
            let (log10_pipe, log10_bg, _keepalive) = bg_log10;
            stages.push(pass(
                "log_exposure",
                log10_pipe.pipeline,
                log10_bg,
                n_pixels,
            ));
        }
        // 3. Density curve (film, normalized): buf_b (log_raw) → buf_a (density_cmy)
        stages.push(pass(
            "film_density",
            density_pipe.pipeline.clone(),
            bg_density_film,
            n_pixels,
        ));
        // 3b. DIR couplers: corrects log_raw in buf_b and re-interpolates
        //     the density curve back into buf_a. Non-default Langmuir
        //     chemistry runs as an assembled CPU fence instead (pipe
        //     architecture, capability class), reading both buffers and
        //     writing the corrected density back in place.
        if let Some(cpu) = p.dir_cpu_stage.as_ref() {
            let buf_a = buf_a.clone();
            let buf_b = buf_b.clone();
            let (w, h) = (image.width, image.height);
            let img_bytes_u64 = img_bytes as u64;
            stages.push(super::pipe::ChainStage::Cpu {
                name: "dir_couplers_cpu",
                fence: Box::new(move |pipe| {
                    let density = pipe.readback_buffer(&buf_a, img_bytes_u64);
                    let log_raw = pipe.readback_buffer(&buf_b, img_bytes_u64);
                    let corrected = cpu(
                        &ImageBuf::from_data(w, h, crate::gpu_helpers::f32_to_scalars(density)),
                        &ImageBuf::from_data(w, h, crate::gpu_helpers::f32_to_scalars(log_raw)),
                    );
                    let out_f32 = crate::gpu_helpers::scalars_to_f32(&corrected.data);
                    pipe.write_image_buffer(&buf_a, &out_f32);
                }),
            });
        } else if let Some(ds) = dir_state.take() {
            stages.push(super::pipe::ChainStage::Gpu {
                name: "dir_couplers",
                encode: Box::new(move |encoder| ds.encode_passes(encoder, n_pixels, wg_xy)),
            });
        }
        // 3a. Capability fence (pipe architecture, S3): an unported grain
        //     engine runs on the host over the developed film density —
        //     after DIR, matching the CPU develop order — and the result
        //     is written back in place so the pre-bound print bind groups
        //     keep addressing the same buffer.
        if let Some(cpu) = p.density_cpu_stage.as_ref() {
            let buf_a = buf_a.clone();
            let (w, h) = (image.width, image.height);
            let img_bytes_u64 = img_bytes as u64;
            stages.push(super::pipe::ChainStage::Cpu {
                name: "density_cpu_stage",
                fence: Box::new(move |pipe| {
                    let host = pipe.readback_buffer(&buf_a, img_bytes_u64);
                    let grained = cpu(&ImageBuf::from_data(
                        w,
                        h,
                        crate::gpu_helpers::f32_to_scalars(host),
                    ));
                    let out_f32 = crate::gpu_helpers::scalars_to_f32(&grained.data);
                    pipe.write_image_buffer(&buf_a, &out_f32);
                }),
            });
        }
        // 4 + 5. Printing: print_spectral (buf_a → buf_b) then the print
        //     density curve (buf_b → buf_a). Skipped for scan_film — the
        //     scan pass consumes the film density already in buf_a. The
        //     PCHIP enlarger LUT and the enlarger diffusion filter run as
        //     an assembled CPU fence that replaces the fused spectral pass
        //     (pipe architecture, capability class); the print density
        //     curve pass consumes the fence output unchanged.
        if !p.scan_film {
            if let Some(cpu) = p.print_acquisition_cpu_stage.as_ref() {
                let buf_a = buf_a.clone();
                let buf_b = buf_b.clone();
                let (w, h) = (image.width, image.height);
                let img_bytes_u64 = img_bytes as u64;
                stages.push(super::pipe::ChainStage::Cpu {
                    name: "print_acquisition_cpu_stage",
                    fence: Box::new(move |pipe| {
                        let density = pipe.readback_buffer(&buf_a, img_bytes_u64);
                        let log_raw = cpu(&ImageBuf::from_data(
                            w,
                            h,
                            crate::gpu_helpers::f32_to_scalars(density),
                        ));
                        let out_f32 = crate::gpu_helpers::scalars_to_f32(&log_raw.data);
                        pipe.write_image_buffer(&buf_b, &out_f32);
                    }),
                });
            } else {
                stages.push(pass(
                    "print_spectral",
                    print_pipe.pipeline.clone(),
                    bg_print,
                    n_pixels,
                ));
            }
            stages.push(pass(
                "print_density",
                density_pipe.pipeline,
                bg_density_print,
                n_pixels,
            ));
        }
        // 6. Scan acquisition: the PCHIP scanner LUT (the slide luminance
        //     remap or `use_scanner_lut` forces it on the CPU route for
        //     Python parity) runs as an assembled CPU fence that replaces
        //     the direct spectral integration (pipe architecture,
        //     capability class): buf_a (density) → fence → buf_b (RGB).
        if let Some(cpu) = p.scan_acquisition_cpu_stage.as_ref() {
            let buf_a = buf_a.clone();
            let buf_b = buf_b.clone();
            let (w, h) = (image.width, image.height);
            let img_bytes_u64 = img_bytes as u64;
            stages.push(super::pipe::ChainStage::Cpu {
                name: "scan_acquisition_cpu_stage",
                fence: Box::new(move |pipe| {
                    let density = pipe.readback_buffer(&buf_a, img_bytes_u64);
                    let captured = cpu(&ImageBuf::from_data(
                        w,
                        h,
                        crate::gpu_helpers::f32_to_scalars(density),
                    ));
                    let out_f32 = crate::gpu_helpers::scalars_to_f32(&captured.data);
                    pipe.write_image_buffer(&buf_b, &out_f32);
                }),
            });
        } else {
            stages.push(pass("scan_spectral", scan_pipe.pipeline, bg_scan, n_pixels));
        }
        // 6a. Scanner readout optics on the linear acquisition, before
        //     glare, gamut correction, and unsharp mask.
        if let Some(bs) = scanner_lens_blur_state.take() {
            let buf_b = buf_b.clone();
            stages.push(super::pipe::ChainStage::Gpu {
                name: "scanner_lens_blur",
                encode: Box::new(move |encoder| bs.encode_passes(encoder, wg_xy, &buf_b)),
            });
        }
        // 6b. Glare (in place on buf_b).
        if let Some(gs) = glare_state.take() {
            stages.push(super::pipe::ChainStage::Gpu {
                name: "glare",
                encode: Box::new(move |encoder| gs.encode_passes(encoder, n_pixels, wg_xy)),
            });
        }
        // 6b'. Capability fence (pipe architecture, S3): the scanner
        //     readout interpretation — slide points, the faithful B&W/slide
        //     luminance remap, negative-to-positive conversion, and the
        //     LabScan finish — runs on the host over the acquisition RGB
        //     after scanner lens blur and glare, before output gamut
        //     compression and unsharp (the CPU scanning order).
        if let Some(cpu) = p.scan_tail_cpu_stage.as_ref() {
            let buf_b = buf_b.clone();
            let (w, h) = (image.width, image.height);
            let img_bytes_u64 = img_bytes as u64;
            stages.push(super::pipe::ChainStage::Cpu {
                name: "scan_tail_cpu_stage",
                fence: Box::new(move |pipe| {
                    let host = pipe.readback_buffer(&buf_b, img_bytes_u64);
                    let interpreted = cpu(&ImageBuf::from_data(
                        w,
                        h,
                        crate::gpu_helpers::f32_to_scalars(host),
                    ));
                    let out_f32 = crate::gpu_helpers::scalars_to_f32(&interpreted.data);
                    pipe.write_image_buffer(&buf_b, &out_f32);
                }),
            });
        }
        // 6c. Output gamut compression (in place on buf_b) — after glare,
        //     before unsharp, mirroring the CPU scanning order.
        if let Some(gs) = gamut_state.take() {
            stages.push(super::pipe::ChainStage::Gpu {
                name: "output_gamut",
                encode: Box::new(move |encoder| gs.encode_passes(encoder, n_pixels)),
            });
        }
        // 6d. Unsharp mask — writes the final image back to buf_b so the
        //     readback path below is unchanged.
        if let Some(us) = unsharp_state.take() {
            let buf_b = buf_b.clone();
            let img_bytes_u64 = img_bytes as u64;
            stages.push(super::pipe::ChainStage::Gpu {
                name: "unsharp_mask",
                encode: Box::new(move |encoder| {
                    us.encode_passes(encoder, n_pixels, wg_xy, &buf_b, img_bytes_u64)
                }),
            });
        }
        // 6e. Encode native destination RGB, then apply Grain Procedural.
        if let Some(gs) = grain_v2_state.take() {
            let buf_b = buf_b.clone();
            stages.push(super::pipe::ChainStage::Gpu {
                name: "grain_v2",
                encode: Box::new(move |encoder| gs.encode_pass(encoder, n_pixels, &buf_b)),
            });
        }
        // 6f. Output transfer (pipe architecture, S4): encode the native
        //     destination RGB (same-space roundtrip matrix + the space's
        //     encoding transfer), or restore linear for a Procedural-grain
        //     output when the caller requested no encoding. Runs in place
        //     on buf_b after every scanner effect, so the readback below
        //     observes the finalized samples.
        if let Some(tp) = p.output_transfer.as_ref() {
            let buf_b = buf_b.clone();
            #[repr(C)]
            #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
            struct TransferParams {
                n_pixels: u32,
                mode: u32,
                cctf_kind: u32,
                _pad: u32,
                col0: [f32; 4],
                col1: [f32; 4],
                col2: [f32; 4],
            }
            let m = tp.matrix;
            let transfer_params = TransferParams {
                n_pixels,
                mode: if tp.decode { 1 } else { 0 },
                cctf_kind: tp.cctf_kind,
                _pad: 0,
                col0: [m[0][0], m[1][0], m[2][0], 0.0],
                col1: [m[0][1], m[1][1], m[2][1], 0.0],
                col2: [m[0][2], m[1][2], m[2][2], 0.0],
            };
            let params_buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("cctf_transfer_params"),
                    contents: bytemuck::bytes_of(&transfer_params),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let pipe = self.cached_pipeline(
                include_str!("../../../spektrafilm-shaders/wgsl/cctf_transfer.wgsl"),
                &[
                    wgpu::BufferBindingType::Uniform,
                    wgpu::BufferBindingType::Storage { read_only: false },
                ],
            );
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bg_cctf_transfer"),
                layout: &pipe.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buf_b.as_entire_binding(),
                    },
                ],
            });
            stages.push(super::pipe::ChainStage::Gpu {
                name: "output_transfer",
                encode: Box::new(move |encoder| {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("output_transfer"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&pipe.pipeline);
                    pass.set_bind_group(0, &bg, &[]);
                    dispatch_linear(&mut pass, n_pixels);
                }),
            });
        }
        // Zero-copy path: when buf_b is mappable, skip the blit and map it
        // directly below. Otherwise stage it into the MAP_READ readback
        // buffer, still inside the single submission.
        if let Some(rb) = readback.as_ref() {
            let rb = rb.clone();
            let buf_b = buf_b.clone();
            let img_bytes_u64 = img_bytes as u64;
            stages.push(super::pipe::ChainStage::Gpu {
                name: "readback_copy",
                encode: Box::new(move |encoder| {
                    encoder.copy_buffer_to_buffer(&buf_b, 0, &rb, 0, img_bytes_u64)
                }),
            });
        }
        // Everything since function entry: CPU-side param prep, buffer
        // creation/uploads, and stage assembly.
        let cpu_setup_ms = t_start.elapsed().as_secs_f32() * 1000.0;
        self.pipe.run_chain(stages);

        // Single sync point at the end. Map buf_b directly on the zero-copy
        // path, or the staging buffer otherwise.
        let map_target = readback.as_ref().unwrap_or(&buf_b);
        let out_f32 = self.read_mapped_f32(map_target, img_bytes as u64);
        let gpu_wait_ms = t_start.elapsed().as_secs_f32() * 1000.0 - cpu_setup_ms;

        let out = ImageBuf::from_data(image.width, image.height, f32_to_scalars(out_f32));
        tracing::debug!(
            target: "spektrafilm_gpu::wgpu_backend",
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
}
