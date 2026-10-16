use super::*;
use buffers::ChainBuffers;

use super::super::blur::SimpleBlurState;
use super::super::cache::CachedPipelineRef;
use super::super::couplers::DirState;
use super::super::gamut::GamutState;
use super::super::glare::GlareState;
use super::super::grain::GrainV2State;
use super::super::halation::HalationState;
use super::super::highlight::HighlightBoostState;
use super::super::unsharp::UnsharpState;

/// Owns all intermediate state produced by the film-chain setup phase.
struct ChainSetup<'a> {
    image: &'a ImageBuf,
    n_pixels: u32,
    img_bytes: usize,
    buf_a: wgpu::Buffer,
    buf_b: wgpu::Buffer,
    #[allow(dead_code)]
    mappable: bool,
    #[allow(dead_code)]
    chain_buffers: ChainBuffers,
    front_pipe: (CachedPipelineRef, wgpu::BindGroup, Option<wgpu::Buffer>),
    bg_log10: (CachedPipelineRef, wgpu::BindGroup, wgpu::Buffer),
    bg_density_film: wgpu::BindGroup,
    bg_print: wgpu::BindGroup,
    bg_density_print: wgpu::BindGroup,
    bg_scan: wgpu::BindGroup,
    readback: Option<wgpu::Buffer>,
    halation_state: Option<HalationState>,
    highlight_state: Option<HighlightBoostState>,
    dye_status: Option<wgpu::Buffer>,
    camera_lens_blur_state: Option<SimpleBlurState>,
    unsharp_state: Option<UnsharpState>,
    grain_v2_state: Option<GrainV2State>,
    gamut_state: Option<GamutState>,
    glare_state: Option<GlareState>,
    scanner_lens_blur_state: Option<SimpleBlurState>,
    dir_state: Option<DirState>,
    density_pipe: CachedPipelineRef,
    print_pipe: CachedPipelineRef,
    scan_pipe: CachedPipelineRef,
}

impl WgpuBackend {
    /// Phase 1: validate dye resources, allocate buffers, pre-compile
    /// pipelines, build bind groups, and construct per-stage state.
    fn setup_chain<'a>(&self, p: &'a crate::FilmChainParams<'a>) -> Result<ChainSetup<'a>, String> {
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

        // Bounded Dye resources are validated before the chain allocates
        // anything (pipe architecture: resource validation precedes chain
        // allocations), so a request the device cannot hold is a named
        // failure instead of an allocation trap.
        if let Some(dye) = p.grain_dye.as_ref() {
            self.validate_dye_resources(image.width, image.height, dye)?;
        }

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

        // Static (LUT) buffers — configuration-invariant, so they are served
        // from the pipe's cross-frame cache: uploaded once per configuration
        // instead of once per frame (pipe architecture F3a).
        let mk_storage = |label: &str, bytes: &[u8]| {
            self.pipe
                .static_buffer(label, bytes, wgpu::BufferUsages::STORAGE)
        };
        let mk_uniform = |label: &str, bytes: &[u8]| {
            self.pipe
                .static_buffer(label, bytes, wgpu::BufferUsages::UNIFORM)
        };

        let ChainBuffers {
            film_log_exp_buf,
            film_curves_buf,
            film_cd_buf,
            film_bd_buf,
            print_illu_buf,
            print_sens_buf,
            print_log_exp_buf,
            print_curves_buf,
            scan_cd_buf,
            scan_bd_buf,
            view_illu_buf,
            cmf_x_buf,
            cmf_y_buf,
            cmf_z_buf,
            front_params_buf,
            film_density_params_buf,
            print_params_buf,
            print_density_params_buf,

            scan_params_buf,
        } = buffers::upload(
            image,
            film_log_exposure,
            film_density_curves_normalized,
            film_gamma,
            film_channel_density,
            film_base_density,
            print_illuminant,
            print_sensitivity,
            print_log_exposure,
            print_density_curves,
            print_gamma,
            preflash,
            print_channel_density,
            print_base_density,
            viewing_illuminant,
            scan_normalization,
            scan_xyz_to_rgb,
            p,
            &mk_storage,
            &mk_uniform,
        );

        // ── Pre-compile pipelines (cached after first call) ──────────────
        // Each shader's bindings layout is fixed and known here.
        let density_pipe = self.cached_pipeline(
            include_str!("../../../../spektrafilm-shaders/wgsl/spectral/density_curve_interp.wgsl"),
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
                "../../../../spektrafilm-shaders/wgsl/spectral/print_spectral_calibrated.wgsl"
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
            include_str!("../../../../spektrafilm-shaders/wgsl/spectral/scan_spectral.wgsl"),
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
                        "../../../../spektrafilm-shaders/wgsl/spectral/hanatos2025_rgb_to_raw.wgsl"
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
                        "../../../../spektrafilm-shaders/wgsl/spectral/mallett_rgb_to_raw.wgsl"
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
                include_str!("../../../../spektrafilm-shaders/wgsl/spectral/log10_inplace.wgsl"),
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
        // Encode the destination space before Grain Procedural preparation; all
        // resolution and grain passes stay in this resident command buffer.
        let grain_v2_state = p.grain_v2.as_ref().map(|gp| {
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

        let dye_status = if let Some(dye) = p.grain_dye.as_ref() {
            let dye_state = self.prepare_dye(image.width, image.height, &buf_a, dye)?;
            Some(dye_state.status_buffer())
        } else {
            None
        };

        Ok(ChainSetup {
            image,
            n_pixels,
            img_bytes,
            buf_a,
            buf_b,
            mappable,
            chain_buffers: ChainBuffers {
                film_log_exp_buf,
                film_curves_buf,
                film_cd_buf,
                film_bd_buf,
                print_illu_buf,
                print_sens_buf,
                print_log_exp_buf,
                print_curves_buf,
                scan_cd_buf,
                scan_bd_buf,
                view_illu_buf,
                cmf_x_buf,
                cmf_y_buf,
                cmf_z_buf,
                front_params_buf,
                film_density_params_buf,
                print_params_buf,
                print_density_params_buf,
                scan_params_buf,
            },
            front_pipe: (front_pipe, bg_front, _front_tc_lut),
            bg_log10,
            bg_density_film,
            bg_print,
            bg_density_print,
            bg_scan,
            readback,
            halation_state,
            highlight_state,
            dye_status,
            camera_lens_blur_state,
            unsharp_state,
            grain_v2_state,
            gamut_state,
            glare_state,
            scanner_lens_blur_state,
            dir_state,
            density_pipe,
            print_pipe,
            scan_pipe,
        })
    }

    /// GPU-resident pipeline: runs the front pass (hanatos LUT lookup or
    /// mallett matmul), highlight boost, camera lens blur, halation,
    /// density curves, DIR, print spectral, scan spectral, glare,
    /// gamut compression, scanner lens blur,
    /// and unsharp as one assembled stage list over ping-pong image
    /// storage. Consecutive device passes share a command buffer; an
    /// assembled CPU fence (grain distribution, acquisition replacement,
    /// readout interpretation, optical diffusion, output transfer,
    /// magazine) flushes the submission, runs the host stage, and the
    /// remaining passes continue on the device.
    pub(super) fn run_film_chain_inner(
        &self,
        p: &crate::FilmChainParams<'_>,
    ) -> Result<ImageBuf, String> {
        let t_start = std::time::Instant::now();
        let setup = self.setup_chain(p)?;
        let ChainSetup {
            image,
            n_pixels,
            img_bytes,
            ref buf_a,
            ref buf_b,
            ref front_pipe,
            ref bg_log10,
            ref bg_density_film,
            ref bg_print,
            ref bg_density_print,
            ref bg_scan,
            ref readback,
            ref halation_state,
            ref highlight_state,
            dye_status: _,
            ref camera_lens_blur_state,
            ref unsharp_state,
            ref grain_v2_state,
            ref gamut_state,
            ref glare_state,
            ref scanner_lens_blur_state,
            ref dir_state,
            ref density_pipe,
            ref print_pipe,
            ref scan_pipe,
            ..
        } = setup;
        let (front_pipe, bg_front, _front_tc_lut) = front_pipe.clone();
        // ── Assemble the stage list, then run it on the pipe ────────────
        // GPU nodes use ordered submissions while sharing resident buffers.
        // Keeping node workloads separate avoids a whole-frame watchdog job.
        // Each closure owns its bind groups and state; buffers use cheap handles.
        let wg_xy = (image.width.div_ceil(16), image.height.div_ceil(16));
        let mut stages: Vec<super::super::pipe::ChainStage<'_>> = Vec::new();
        let pass = |name: &'static str,
                    pipeline: wgpu::ComputePipeline,
                    bg: wgpu::BindGroup,
                    n: u32|
         -> super::super::pipe::ChainStage<'static> {
            super::super::pipe::ChainStage::Gpu {
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
        if let Some(hs) = highlight_state.as_ref() {
            stages.push(super::super::pipe::ChainStage::Gpu {
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
            stages.push(super::super::pipe::ChainStage::Cpu {
                name: "camera_diffusion_cpu_stage",
                fence: Box::new(move |pipe| {
                    let host = pipe.readback_buffer(&buf_b, img_bytes_u64);
                    let diffused = cpu(&ImageBuf::from_data(
                        w,
                        h,
                        crate::gpu_helpers::f32_to_scalars(host),
                    ))?;
                    let out_f32 = crate::gpu_helpers::scalars_to_f32(&diffused.data);
                    pipe.write_image_buffer(&buf_b, &out_f32);
                    Ok(())
                }),
            });
        }
        // 1b. Camera lens blur on raw before halation.
        if let Some(bs) = camera_lens_blur_state.as_ref() {
            let buf_b = buf_b.clone();
            stages.push(super::super::pipe::ChainStage::Gpu {
                name: "camera_lens_blur",
                encode: Box::new(move |encoder| bs.encode_passes(encoder, wg_xy, &buf_b)),
            });
        }
        // 1c. Halation in-place on buf_b (blur scratch in buf_a, scatter
        //     outputs and accumulator in buf_c / buf_d).
        if let Some(hs) = halation_state.as_ref() {
            stages.push(super::super::pipe::ChainStage::Gpu {
                name: "halation",
                encode: Box::new(move |encoder| hs.encode_passes(encoder, n_pixels, wg_xy)),
            });
        }
        // 2. log10 in-place on buf_b: raw → log_raw (3 channels per thread).
        {
            let (log10_pipe, log10_bg, _keepalive) = bg_log10.clone();
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
            bg_density_film.clone(),
            n_pixels,
        ));
        // 3b. DIR couplers: corrects log_raw in buf_b and re-interpolates
        //     the density curve back into buf_a.
        if let Some(ds) = dir_state.as_ref() {
            stages.push(super::super::pipe::ChainStage::Gpu {
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
            stages.push(super::super::pipe::ChainStage::Cpu {
                name: "density_cpu_stage",
                fence: Box::new(move |pipe| {
                    let host = pipe.readback_buffer(&buf_a, img_bytes_u64);
                    let grained = cpu(&ImageBuf::from_data(
                        w,
                        h,
                        crate::gpu_helpers::f32_to_scalars(host),
                    ))?;
                    let out_f32 = crate::gpu_helpers::scalars_to_f32(&grained.data);
                    pipe.write_image_buffer(&buf_a, &out_f32);
                    Ok(())
                }),
            });
        }
        // 3c. Film-grain Dye: the device field on the developed density in
        //     buf_a, in the density-fence position (after the film density
        //     curve and DIR, before printing). The pass resolves the
        //     conditional field and writes its readout back into the same
        //     buffer, so the pre-bound print bind groups keep addressing it.
        //     Its bounded status is checked after the run and before the
        //     output is published below.
        if let Some(dye) = p.grain_dye.as_ref() {
            let dye_state = self.prepare_dye(image.width, image.height, buf_a, dye)?;
            dye_state.append_stages(&mut stages, buf_a);
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
                stages.push(super::super::pipe::ChainStage::Cpu {
                    name: "print_acquisition_cpu_stage",
                    fence: Box::new(move |pipe| {
                        let density = pipe.readback_buffer(&buf_a, img_bytes_u64);
                        let log_raw = cpu(&ImageBuf::from_data(
                            w,
                            h,
                            crate::gpu_helpers::f32_to_scalars(density),
                        ))?;
                        let out_f32 = crate::gpu_helpers::scalars_to_f32(&log_raw.data);
                        pipe.write_image_buffer(&buf_b, &out_f32);
                        Ok(())
                    }),
                });
            } else {
                stages.push(pass(
                    "print_spectral",
                    print_pipe.pipeline.clone(),
                    bg_print.clone(),
                    n_pixels,
                ));
            }
            stages.push(pass(
                "print_density",
                density_pipe.pipeline.clone(),
                bg_density_print.clone(),
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
            stages.push(super::super::pipe::ChainStage::Cpu {
                name: "scan_acquisition_cpu_stage",
                fence: Box::new(move |pipe| {
                    let density = pipe.readback_buffer(&buf_a, img_bytes_u64);
                    let captured = cpu(&ImageBuf::from_data(
                        w,
                        h,
                        crate::gpu_helpers::f32_to_scalars(density),
                    ))?;
                    let out_f32 = crate::gpu_helpers::scalars_to_f32(&captured.data);
                    pipe.write_image_buffer(&buf_b, &out_f32);
                    Ok(())
                }),
            });
        } else {
            stages.push(pass(
                "scan_spectral",
                scan_pipe.pipeline.clone(),
                bg_scan.clone(),
                n_pixels,
            ));
        }
        // 6a. Scanner readout optics on the linear acquisition, before
        //     glare, gamut correction, and unsharp mask.
        if let Some(bs) = scanner_lens_blur_state.as_ref() {
            let buf_b = buf_b.clone();
            stages.push(super::super::pipe::ChainStage::Gpu {
                name: "scanner_lens_blur",
                encode: Box::new(move |encoder| bs.encode_passes(encoder, wg_xy, &buf_b)),
            });
        }
        // 6b. Glare (in place on buf_b).
        if let Some(gs) = glare_state.as_ref() {
            stages.push(super::super::pipe::ChainStage::Gpu {
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
            stages.push(super::super::pipe::ChainStage::Cpu {
                name: "scan_tail_cpu_stage",
                fence: Box::new(move |pipe| {
                    let host = pipe.readback_buffer(&buf_b, img_bytes_u64);
                    let interpreted = cpu(&ImageBuf::from_data(
                        w,
                        h,
                        crate::gpu_helpers::f32_to_scalars(host),
                    ))?;
                    let out_f32 = crate::gpu_helpers::scalars_to_f32(&interpreted.data);
                    pipe.write_image_buffer(&buf_b, &out_f32);
                    Ok(())
                }),
            });
        }
        // 6c. Output gamut compression (in place on buf_b) — after glare,
        //     before unsharp, mirroring the CPU scanning order.
        if let Some(gs) = gamut_state.as_ref() {
            stages.push(super::super::pipe::ChainStage::Gpu {
                name: "output_gamut",
                encode: Box::new(move |encoder| gs.encode_passes(encoder, n_pixels)),
            });
        }
        // 6d. Unsharp mask — writes the final image back to buf_b so the
        //     readback path below is unchanged.
        if let Some(us) = unsharp_state.as_ref() {
            let buf_b = buf_b.clone();
            let img_bytes_u64 = img_bytes as u64;
            stages.push(super::super::pipe::ChainStage::Gpu {
                name: "unsharp_mask",
                encode: Box::new(move |encoder| {
                    us.encode_passes(encoder, n_pixels, wg_xy, &buf_b, img_bytes_u64)
                }),
            });
        }
        // 6e. Encode native destination RGB, then apply Grain Procedural.
        if let Some(gs) = grain_v2_state.as_ref() {
            let buf_b = buf_b.clone();
            stages.push(super::super::pipe::ChainStage::Gpu {
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
                concat!(
                    include_str!("../../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                    include_str!("../../../../spektrafilm-shaders/wgsl/cctf_transfer.wgsl")
                ),
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
            stages.push(super::super::pipe::ChainStage::Gpu {
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
        // 6g. Magazine print appearance (pipe architecture, S4): the last
        //     host finishing moves onto the device. Runs after the output
        //     transfer, so the buffer state is encoded exactly when the
        //     caller requested output encoding.
        if let Some(mp) = p.magazine.as_ref() {
            let buf_b = buf_b.clone();
            #[repr(C)]
            #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
            struct MagazineParams {
                n_pixels: u32,
                cctf_kind: u32,
                encoded: u32,
                _pad0: u32,
                strength_pad: [f32; 4],
                to_col0: [f32; 4],
                to_col1: [f32; 4],
                to_col2: [f32; 4],
                from_col0: [f32; 4],
                from_col1: [f32; 4],
                from_col2: [f32; 4],
                tin0: [f32; 4],
                tin1: [f32; 4],
                tin2: [f32; 4],
                tout0: [f32; 4],
                tout1: [f32; 4],
                tout2: [f32; 4],
                slo0: [f32; 4],
                slo1: [f32; 4],
                slo2: [f32; 4],
            }
            let pack10 = |v: &[f32; 10]| {
                [
                    [v[0], v[1], v[2], v[3]],
                    [v[4], v[5], v[6], v[7]],
                    [v[8], v[9], 0.0, 0.0],
                ]
            };
            let (tin, tout, slo) = (
                pack10(&mp.tone_in),
                pack10(&mp.tone_out),
                pack10(&mp.tone_slopes),
            );
            let magazine_params = MagazineParams {
                n_pixels,
                cctf_kind: mp.cctf_kind,
                encoded: u32::from(mp.encoded),
                _pad0: 0,
                strength_pad: [mp.strength, 0.0, 0.0, 0.0],
                to_col0: [mp.to_lab[0][0], mp.to_lab[1][0], mp.to_lab[2][0], 0.0],
                to_col1: [mp.to_lab[0][1], mp.to_lab[1][1], mp.to_lab[2][1], 0.0],
                to_col2: [mp.to_lab[0][2], mp.to_lab[1][2], mp.to_lab[2][2], 0.0],
                from_col0: [mp.from_lab[0][0], mp.from_lab[1][0], mp.from_lab[2][0], 0.0],
                from_col1: [mp.from_lab[0][1], mp.from_lab[1][1], mp.from_lab[2][1], 0.0],
                from_col2: [mp.from_lab[0][2], mp.from_lab[1][2], mp.from_lab[2][2], 0.0],
                tin0: tin[0],
                tin1: tin[1],
                tin2: tin[2],
                tout0: tout[0],
                tout1: tout[1],
                tout2: tout[2],
                slo0: slo[0],
                slo1: slo[1],
                slo2: slo[2],
            };
            let params_buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("magazine_params"),
                    contents: bytemuck::bytes_of(&magazine_params),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let pipe = self.cached_pipeline(
                concat!(
                    include_str!("../../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                    include_str!("../../../../spektrafilm-shaders/wgsl/magazine.wgsl")
                ),
                &[
                    wgpu::BufferBindingType::Uniform,
                    wgpu::BufferBindingType::Storage { read_only: false },
                ],
            );
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bg_magazine"),
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
            stages.push(super::super::pipe::ChainStage::Gpu {
                name: "magazine",
                encode: Box::new(move |encoder| {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("magazine"),
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
        // buffer in the last ordered device submission.
        if let Some(rb) = readback.as_ref() {
            let rb = rb.clone();
            let buf_b = buf_b.clone();
            let img_bytes_u64 = img_bytes as u64;
            stages.push(super::super::pipe::ChainStage::Gpu {
                name: "readback_copy",
                encode: Box::new(move |encoder| {
                    encoder.copy_buffer_to_buffer(&buf_b, 0, &rb, 0, img_bytes_u64)
                }),
            });
        }
        // Everything since function entry: CPU-side param prep, buffer
        // creation/uploads, and stage assembly.
        let cpu_setup_ms = t_start.elapsed().as_secs_f32() * 1000.0;
        self.pipe.run_chain(stages)?;
        // The Dye pass publishes only a bounded flag; it is checked after the
        // run and before the output is read back and published (pipe
        // architecture), so an out-of-bounds field fails instead of escaping.
        if let Some(status) = setup.dye_status.as_ref() {
            self.check_dye_status(status)?;
        }

        // Single sync point at the end. Map buf_b directly on the zero-copy
        // path, or the staging buffer otherwise.
        let map_target = readback.as_ref().unwrap_or(buf_b);
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
        Ok(out)
    }
}

// ── Bounded-execution preflight (pure, device-free) ──────────────────────────
//
// The chain allocates the image buffers straight from `n_pixels` and binds
// every buffer with `as_entire_binding()`, so an oversized request would
// otherwise surface as an uncaptured wgpu-core validation error. These
// helpers derive the required byte sizes arithmetically so the resident
// chain entry can compare them with the device limits before any allocation.

/// Conservative bound for the small fixed-size buffers the chain creates
/// besides the image and lookup buffers: the std140 param uniforms
/// (largest is the 272-byte `MagazineParams`), the FIR kernels (≤ 513
/// taps under `MAX_BLUR_RADIUS`), and the grain resolution-weight sets.
///
/// The grain weights are the only config-sized entry in this bound. Validated
/// configuration keeps them far below it: `film_render.grain.v2_size` is
/// limited to `[1, 48]` (`spektrafilm-core`'s parameter validation), which
/// bounds the radius and leaves the weight set under a kilobyte. A caller
/// that builds `FilmChainParams` programmatically can exceed the bound; the
/// preflight then under-reports, which is no worse than the validation error
/// it replaces.
const MAX_FIXED_AUX_BYTES: u64 = 4 * 1024;

/// Device buffer sizes the resident chain will allocate, derived from the
/// parameters without touching a device.
pub(in crate::wgpu_backend) struct RequiredChainBuffers {
    /// One image-sized buffer: the `img_a`/`img_b` ping-pong pair, the
    /// readback buffer, and every stage scratch buffer (blur, halation,
    /// DIR, glare, unsharp, grain) is created at this size or smaller. The
    /// Dye field's bounded band scratch is not derived here: it is planned
    /// against the device limits by `validate_dye_resources`, which runs
    /// before the chain allocates anything.
    pub(super) image_bytes: u64,
    /// Largest static buffer: the f32-narrowed lookup tables (spectral
    /// densities, density curves, front-pass LUT, gamut `C_max`) plus the
    /// fixed-size param uniforms and kernels.
    pub(super) static_bytes: u64,
}

impl RequiredChainBuffers {
    /// The largest single buffer the chain will create.
    pub(in crate::wgpu_backend) fn largest(&self) -> u64 {
        self.image_bytes.max(self.static_bytes)
    }
}

/// Derives the required device buffer sizes from the chain parameters.
/// Sizes mirror `run_film_chain_inner`: every lookup value is narrowed to
/// one f32 on upload, so `n` f64 samples become `4n` bytes and an
/// `[[f64; 3]]` table of `n` rows becomes `12n`.
pub(in crate::wgpu_backend) fn required_chain_buffers(
    p: &crate::FilmChainParams<'_>,
) -> RequiredChainBuffers {
    let f32s = |count: usize| (count as u64).saturating_mul(4);
    let table = |rows: usize| f32s(rows).saturating_mul(3);
    let image_bytes = (u64::from(p.image.width) * u64::from(p.image.height)).saturating_mul(3 * 4);
    // `scan_cd`/`scan_bd` feed from the film or the print spectral data,
    // matching the `scan_film` selection in `run_film_chain_inner`.
    let scan_wavelengths = if p.scan_film {
        p.film_channel_density.len()
    } else {
        p.print_channel_density.len()
    };
    let tc_lut_bytes = match &p.front {
        crate::FrontPass::Hanatos2025 { tc_lut, .. } => f32s(tc_lut.data.len()),
        crate::FrontPass::Mallett2019 { .. } => 0,
    };
    let dir_log_bytes = p.dir_couplers.map_or(0, |dp| f32s(dp.log_exposure.len()));
    let dir_curves_bytes = p
        .dir_couplers
        .map_or(0, |dp| table(dp.density_curves_0.len()));
    let cmax_bytes = p.gamut.map_or(0, |gp| f32s(gp.cmax.len()));
    let static_bytes = [
        f32s(p.film_log_exposure.len()),
        table(p.film_density_curves_normalized.len()),
        table(p.film_channel_density.len()),
        f32s(p.film_channel_density.len()),
        f32s(p.print_illuminant.len()),
        table(p.print_sensitivity.len()),
        f32s(p.print_log_exposure.len()),
        table(p.print_density_curves.len()),
        table(scan_wavelengths),
        f32s(scan_wavelengths),
        f32s(p.viewing_illuminant.len()),
        f32s(spektrafilm_math::spectral::N_WAVELENGTHS), // cmf_x/y/z
        tc_lut_bytes,
        dir_log_bytes,
        dir_curves_bytes,
        cmax_bytes,
        MAX_FIXED_AUX_BYTES,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    RequiredChainBuffers {
        image_bytes,
        static_bytes,
    }
}

/// Compares one required buffer size with the device limits. Every chain
/// binding is `as_entire_binding()`, so the size is checked against both
/// the absolute buffer limit and the storage binding limit; returns a
/// human-readable reason on violation.
pub(in crate::wgpu_backend) fn required_buffer_exceeds_limits(
    required_bytes: u64,
    limits: &wgpu::Limits,
) -> Option<String> {
    if required_bytes > limits.max_buffer_size {
        return Some(format!(
            "required buffer of {required_bytes} bytes exceeds max_buffer_size ({} bytes)",
            limits.max_buffer_size
        ));
    }
    if required_bytes > u64::from(limits.max_storage_buffer_binding_size) {
        return Some(format!(
            "required buffer of {required_bytes} bytes exceeds max_storage_buffer_binding_size \
             ({} bytes)",
            limits.max_storage_buffer_binding_size
        ));
    }
    None
}

/// The failure an explicit GPU request gets when the chain exceeds the
/// device resource limits: identified, with the CPU route named.
pub(in crate::wgpu_backend) fn explicit_resource_error(reason: &str) -> String {
    format!(
        "explicit GPU request exceeds the device resource limits ({reason}); retry with an \
         explicit CPU request"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FilmChainParams, FrontPass};
    use spektrafilm_math::image::ImageBuf;

    const MAT3: [[f64; 3]; 3] = [[0.0; 3]; 3];

    /// Minimal chain params: empty lookup tables, no optional stages. The
    /// image is borrowed so oversized geometries cost no allocation.
    fn params(image: &ImageBuf) -> FilmChainParams<'_> {
        FilmChainParams {
            image,
            front: FrontPass::Mallett2019 { matrix: MAT3 },
            film_log_exposure: &[],
            film_density_curves_normalized: &[],
            film_gamma: 1.0,
            film_channel_density: &[],
            film_base_density: &[],
            print_illuminant: &[],
            print_sensitivity: &[],
            print_normalization_factor: 1.0,
            print_exposure_scale: 1.0,
            print_log_exposure: &[],
            print_density_curves: &[],
            print_gamma: 1.0,
            print_channel_density: &[],
            print_base_density: &[],
            preflash: [0.0; 3],
            viewing_illuminant: &[],
            scan_normalization: 1.0,
            scan_xyz_to_rgb: &MAT3,
            white_black_xyz_remap: None,
            scan_film: false,
            halation: None,
            dir_couplers: None,
            glare: None,
            gamut: None,
            unsharp: None,
            grain_v2: None,
            scan_output_space: &spektrafilm_math::colorspace::SRGB,
            camera_lens_blur_px: None,
            scanner_lens_blur_px: None,
            highlight_boost: None,
            grain_dye: None,
            density_cpu_stage: None,
            scan_tail_cpu_stage: None,
            scan_acquisition_cpu_stage: None,
            print_acquisition_cpu_stage: None,
            camera_diffusion_cpu_stage: None,
            output_transfer: None,
            magazine: None,
        }
    }

    fn synthetic_limits(
        max_buffer_size: u64,
        max_storage_buffer_binding_size: u32,
    ) -> wgpu::Limits {
        wgpu::Limits {
            max_buffer_size,
            max_storage_buffer_binding_size,
            ..wgpu::Limits::default()
        }
    }

    #[test]
    fn required_buffer_exceeds_limits_accepts_a_comfortable_bound() {
        let limits = synthetic_limits(1 << 30, 1 << 28);
        assert_eq!(required_buffer_exceeds_limits(1 << 20, &limits), None);
    }

    #[test]
    fn required_buffer_exceeds_limits_rejects_an_oversized_image() {
        // 20000×20000 → 4.8 GB of image buffers; a synthetic tiny
        // `max_buffer_size` must reject that before any allocation.
        let image = ImageBuf {
            width: 20_000,
            height: 20_000,
            data: vec![],
        };
        let required = required_chain_buffers(&params(&image));
        assert_eq!(required.image_bytes, 20_000u64 * 20_000 * 12);
        assert_eq!(required.largest(), required.image_bytes);
        let limits = synthetic_limits(1 << 20, u32::MAX);
        let reason =
            required_buffer_exceeds_limits(required.largest(), &limits).expect("oversized image");
        assert!(reason.contains("max_buffer_size"));
        assert!(reason.contains(&required.largest().to_string()));
    }

    #[test]
    fn required_buffer_exceeds_limits_rejects_a_tighter_binding_limit() {
        // 512×512 → 3 MiB of image buffers: fine for the buffer limit but
        // over a 1 MiB storage binding limit.
        let image = ImageBuf {
            width: 512,
            height: 512,
            data: vec![],
        };
        let required = required_chain_buffers(&params(&image));
        assert!(required.largest() > (1 << 20));
        let limits = synthetic_limits(u64::MAX, 1 << 20);
        let reason = required_buffer_exceeds_limits(required.largest(), &limits)
            .expect("tight binding limit");
        assert!(reason.contains("max_storage_buffer_binding_size"));
    }

    #[test]
    fn explicit_resource_error_carries_the_required_wording() {
        let image = ImageBuf {
            width: 20_000,
            height: 20_000,
            data: vec![],
        };
        let required = required_chain_buffers(&params(&image));
        let limits = synthetic_limits(1 << 20, u32::MAX);
        let reason = required_buffer_exceeds_limits(required.largest(), &limits).unwrap();
        let message = explicit_resource_error(&reason);
        assert!(message.contains("explicit GPU request"));
        assert!(message.contains("resource"));
        assert!(message.contains("retry with an explicit CPU request"));
        // The offending quantity rides along in the reason.
        assert!(message.contains(&required.largest().to_string()));
    }

    /// The resident entry must decline before any allocation when a required
    /// buffer exceeds the device limits: the automatic route falls back to the
    /// per-stage path, and an explicit request fails with the resource reason
    /// instead of an uncaptured wgpu-core validation error.
    #[test]
    fn oversized_geometry_declines_the_automatic_route_and_fails_an_explicit_request() {
        use crate::ComputeBackend;
        let _lock = crate::wgpu_backend::observation::TEST_DEVICE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // 20k x 20k x 12 B = 4.8 GB declared: beyond any device limit and never
        // allocated, because the preflight runs before the first allocation.
        let image = ImageBuf {
            width: 20_000,
            height: 20_000,
            data: vec![],
        };
        let params = params(&image);
        let (automatic, explicit) = (
            crate::test_support::adapter(),
            crate::test_support::explicit_adapter(),
        );
        assert!(
            automatic
                .try_run_film_chain(&params)
                .expect("the automatic route declines, it does not error")
                .is_none(),
            "the automatic route must decline instead of allocating 4.8 GB"
        );
        let error = explicit
            .try_run_film_chain(&params)
            .expect_err("an explicit request must fail");
        assert!(error.contains("explicit GPU request"), "{error}");
        assert!(error.contains("resource"), "{error}");
        assert!(
            error.contains("retry with an explicit CPU request"),
            "{error}"
        );
    }

    #[test]
    fn required_chain_buffers_mirror_the_chain_arithmetic() {
        let image = ImageBuf {
            width: 8,
            height: 4,
            data: vec![],
        };
        let log_exp = [0.0f64; 5];
        let tc_lut = spektrafilm_math::spectral::TcLut {
            size: 16,
            channels: 3,
            data: vec![0.0; 2048],
        };
        let p = FilmChainParams {
            front: FrontPass::Hanatos2025 {
                tc_lut: &tc_lut,
                rgb_to_adapted_xyz: MAT3,
            },
            film_log_exposure: &log_exp,
            ..params(&image)
        };
        let required = required_chain_buffers(&p);
        assert_eq!(required.image_bytes, 8u64 * 4 * 12);
        // A non-empty `[[f64; 3]]` table narrows to 12 bytes per row and
        // outgrows the fixed-aux bound, so the table path itself is pinned.
        let density_rows = [[0.0f64; 3]; 400];
        let table_params = FilmChainParams {
            film_channel_density: &density_rows,
            ..params(&image)
        };
        assert_eq!(
            required_chain_buffers(&table_params).static_bytes,
            400u64 * 12
        );
        // The 2048-entry f64 LUT narrows to 2048 f32 = 8 KiB, beating both
        // the 5-point log exposure (20 bytes) and the fixed aux bound.
        assert_eq!(required.static_bytes, 2048u64 * 4);
    }
}
