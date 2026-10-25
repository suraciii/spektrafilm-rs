use super::*;
use buffers::ChainBuffers;

use super::super::blur::{GaussianScratch, SimpleBlurState};
use super::super::chain_chunks::PointwisePass;
use super::super::couplers::DirState;
use super::super::gamut::GamutState;
use super::super::glare::GlareState;
use super::super::grain::{GrainV2State, SpektraGrainV1State};
use super::super::halation::HalationState;
use super::super::highlight::HighlightBoostState;
use super::super::paged::ImagePage;
use super::super::scan::{ConvertInverseState, ScanDensityState};
use super::super::unsharp::UnsharpState;
use spektrafilm_backend::Tap;

fn tap_index(tap: Tap) -> usize {
    match tap {
        Tap::RgbIn => 0,
        Tap::RgbPre => 1,
        Tap::LogEFilm => 2,
        Tap::CmyFilm => 3,
        Tap::LogEPrint => 4,
        Tap::CmyPrint => 5,
        Tap::RgbOut => 6,
    }
}

fn validate_tap_span(p: &spektrafilm_backend::FilmChainParams<'_>) -> Result<(), String> {
    for tap in [p.inject, p.collect] {
        if p.scan_film && matches!(tap, Tap::LogEPrint | Tap::CmyPrint) {
            return Err(format!(
                "tap {tap:?} is not part of this topology (scan_film = true)"
            ));
        }
    }
    if tap_index(p.collect) < tap_index(p.inject) {
        return Err(format!(
            "no node path reaches tap {:?} from {:?}",
            p.collect, p.inject
        ));
    }
    if p.inject == Tap::RgbIn && p.inject != p.collect {
        return Err(
            "film chain rgb_in must pass through retained metering and geometry before dispatch"
                .into(),
        );
    }
    Ok(())
}

/// Owns all intermediate state produced by the film-chain setup phase.
struct ChainSetup {
    image_width: u32,
    image_height: u32,
    img_bytes: usize,
    buf_a: Vec<ImagePage>,
    buf_b: Vec<ImagePage>,
    /// The rgb→raw front pass (hanatos or mallett). `None` for the
    /// inverse-scan chain head, which has no rgb→raw transform at all.
    front_pipe: Option<PointwisePass>,
    /// Chain-head input transfer decoding on `buf_a`, before the front
    /// transform. `None` when the caller supplied linear samples already.
    input_decode_state: Option<InputDecodeState>,
    convert_inverse_state: Option<ConvertInverseState>,
    scan_readout_state: Option<ScanDensityState>,
    dye_state: Option<super::super::grain::dye::DyePagedState>,
    fotufilm_state: Option<super::super::grain::fotufilm::FotufilmPagedState>,
    dye_print_log: bool,
    dye_collapse: Option<u32>,
    tail_a: Vec<ImagePage>,
    tail_b: Vec<ImagePage>,
    film_density_output: Vec<ImagePage>,
    film_density_width: u32,
    film_density_height: u32,
    print_exposure_output: Vec<ImagePage>,
    print_fine_output: Vec<ImagePage>,
    tail_width: u32,
    tail_height: u32,
    bg_log10: PointwisePass,
    bg_density_film: PointwisePass,
    bg_print: PointwisePass,
    bg_density_print: PointwisePass,
    bg_scan: PointwisePass,
    halation_state: Option<HalationState>,
    highlight_state: Option<HighlightBoostState>,
    camera_lens_blur_state: Option<SimpleBlurState>,
    unsharp_state: Option<UnsharpState>,
    grain_v2_state: Option<GrainV2State>,
    /// Device Spektra v1 grain on the film density, in the density-fence
    /// slot. `None` when the request samples the scipy regime (no per-pixel
    /// mapping) or the device lacks native f64, so the host fence runs.
    grain_v1_state: Option<SpektraGrainV1State>,
    gamut_state: Option<GamutState>,
    glare_state: Option<GlareState>,
    scanner_lens_blur_state: Option<SimpleBlurState>,
    dir_state: Option<DirState>,
}

impl WgpuBackend {
    /// Phase 1: validate dye resources, allocate buffers, pre-compile
    /// pipelines, build bind groups, and construct per-stage state.
    fn setup_chain(
        &self,
        p: &spektrafilm_backend::FilmChainParams<'_>,
        input: &spektrafilm_backend::DeviceImage,
    ) -> Result<ChainSetup, String> {
        // The working raster geometry is the carrier's, not a host image's:
        // after retained input geometry the chain never sees host samples.
        let image_width = p.image_width;
        let image_height = p.image_height;
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

        // Bounded Dye resources are validated before the chain allocates
        // anything (pipe architecture: resource validation precedes chain
        // allocations), so a request the device cannot hold is a named
        // failure instead of an allocation trap.
        if input.width() != image_width || input.height() != image_height {
            return Err(format!(
                "film chain input carrier is {}x{} but the resolved working geometry is \
                 {image_width}x{image_height}",
                input.width(),
                input.height()
            ));
        }
        if let Some(dye) = p.grain_dye.as_ref() {
            self.validate_dye_resources(image_width, image_height, dye)?;
        }

        let n_pixels = image_width * image_height;
        let img_bytes = n_pixels as usize * 3 * 4;

        let buf_a = self.allocate_image_pages(image_width, image_height, "chain_work_a")?;
        if let Ok(payload) = self.buffer_payload(input) {
            if payload.bytes != img_bytes as u64 {
                return Err(
                    "film chain retained input footprint does not match geometry".to_owned(),
                );
            }
            // Borrowed retained input is immutable, including a singleton page.
            let mut encoder = self.device.create_command_encoder(&Default::default());
            for page in &buf_a {
                self.copy_samples_to(
                    &mut encoder,
                    payload,
                    u64::from(page.first_pixel) * 3,
                    u64::from(page.n_pixels) * 3,
                    &page.buffer,
                    0,
                )?;
            }
            self.queue.submit(Some(encoder.finish()));
        } else if let Some(image) = input.host_image() {
            for page in &buf_a {
                let first = page.first_pixel as usize * 3;
                let values = scalars_to_f32(&image.data[first..first + page.n_pixels as usize * 3]);
                self.queue
                    .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&values));
            }
        } else {
            return Err("film chain input is not available on this device".to_owned());
        }
        let mut buf_b = self.allocate_image_pages(image_width, image_height, "chain_work_b")?;
        let mut buf_a = buf_a;
        if matches!(p.inject, Tap::LogEFilm | Tap::LogEPrint) {
            // Bind the supplied retained exposure in the slot consumed by
            // the next node, without a transfer or a host round trip.
            std::mem::swap(&mut buf_a, &mut buf_b);
        }
        let dye_state = p
            .grain_dye
            .as_ref()
            .map(|dye| {
                let source = super::device_image::BufferPayload::from_pages(
                    self.backend_id(),
                    buf_a.clone(),
                    image_width,
                    image_height,
                );
                let response = if p.grain_dye_cell_spectral {
                    Some(if p.scan_film {
                        spektrafilm_backend::DyeResponseGpuParams::Scan {
                            channel_density: p.film_channel_density,
                            base_density: p.film_base_density,
                            illuminant: p.scan_illuminant,
                            normalization: p.scan_normalization,
                            xyz_to_rgb: p.scan_xyz_to_rgb,
                        }
                    } else {
                        spektrafilm_backend::DyeResponseGpuParams::Print {
                            channel_density: p.film_channel_density,
                            base_density: p.film_base_density,
                            illuminant: p.print_illuminant,
                            sensitivity: p.print_sensitivity,
                            normalization_factor: p.print_normalization_factor,
                            exposure_scale: p.print_exposure_scale,
                            preflash: p.preflash,
                        }
                    })
                } else {
                    None
                };
                self.prepare_dye_paged(image_width, image_height, &source, dye, response.as_ref())
            })
            .transpose()?;
        let fotufilm_state = p
            .grain_fotufilm
            .as_ref()
            .map(|params| {
                let source = super::device_image::BufferPayload::from_pages(
                    self.backend_id(),
                    buf_a.clone(),
                    image_width,
                    image_height,
                );
                self.prepare_fotufilm_paged(image_width, image_height, &source, params)
            })
            .transpose()?;
        let (print_width, print_height) = p
            .grain_dye
            .as_ref()
            .map(|dye| (dye.grid.width, dye.grid.height))
            .or_else(|| {
                p.grain_fotufilm
                    .as_ref()
                    .map(|grain| (grain.width, grain.height))
            })
            .unwrap_or((image_width, image_height));
        let print_input = if p.inject == Tap::CmyFilm {
            buf_a.clone()
        } else if let Some(state) = dye_state.as_ref() {
            state.output_pages()
        } else if fotufilm_state.is_some() {
            self.allocate_image_pages(print_width, print_height, "field_density")?
        } else {
            buf_a.clone()
        };
        let tail_width = print_width / p.grain_dye_aperture.unwrap_or(1);
        let tail_height = print_height / p.grain_dye_aperture.unwrap_or(1);
        let tail_buf =
            |label: &str, width: u32, height: u32| self.allocate_image_pages(width, height, label);
        // Source work pages are no longer read after field formation. Reuse
        // them in queue order only when the tail has the same logical extent.
        let same_tail = (tail_width, tail_height) == (image_width, image_height);
        let tail_a = if same_tail {
            buf_a.clone()
        } else {
            tail_buf("field_tail_density", tail_width, tail_height)?
        };
        let tail_b = if same_tail {
            buf_b.clone()
        } else {
            tail_buf("field_tail_rgb", tail_width, tail_height)?
        };
        let print_output = if p.grain_dye_aperture.is_some() {
            tail_buf("field_fine_exposure", print_width, print_height)?
        } else {
            tail_b.clone()
        };
        let dye_print_log = p.grain_dye_cell_spectral && !p.scan_film;
        let collapse = p.grain_dye_aperture;
        let print_density_input = if p.inject == Tap::LogEPrint {
            &buf_b
        } else {
            &tail_b
        };

        // Static (LUT) buffers — configuration-invariant, so they are served
        // from the pipe's cross-frame cache: uploaded once per configuration
        // instead of once per frame (pipe architecture F3a).
        let mk_storage = |label: &str, bytes: &[u8]| {
            self.pipe
                .static_buffer(label, bytes, wgpu::BufferUsages::STORAGE)
        };
        let mk_uniform = |label: &str, bytes: &[u8]| {
            self.pipe.static_buffer(
                label,
                bytes,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_SRC,
            )
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
            front_params_buf,
            film_density_params_buf,
            print_density_params_buf,
        } = buffers::upload(
            tail_width,
            tail_height,
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
            p,
            &mk_storage,
            &mk_uniform,
        );
        let print_params_buf = {
            let bytes = [
                print_width,
                print_height,
                p.film_channel_density.len() as u32,
                (p.print_normalization_factor as f32).to_bits(),
                (p.print_exposure_scale as f32).to_bits(),
                u32::from(p.enlarger_diffusion.is_some()),
                0,
                0,
                (p.preflash[0] as f32).to_bits(),
                (p.preflash[1] as f32).to_bits(),
                (p.preflash[2] as f32).to_bits(),
                0,
            ];
            mk_uniform("dye_print_params", bytemuck::cast_slice(&bytes))
        };

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

        // ── Build bind groups (per dispatch, but no buffer creation) ─────
        // Front pass: hanatos (params + rgb_in + tc_lut + raw_out) or mallett
        // (params + rgb_in + raw_out). The tc_lut buffer rides along in the
        // tuple to outlive the encoder on the hanatos arm.
        let front_pipe = match &p.front {
            spektrafilm_backend::FrontPass::Hanatos2025 { generated, .. } => {
                let generated = generated.ok_or_else(||
                    "selected GPU requires a device-prepared front spectral table; prepare calibration with the selected backend".to_owned())?;
                if generated.values.backend_id() != self.backend_id() {
                    return Err("front spectral table belongs to another device; rebuild calibration with the selected backend".to_owned());
                }
                let tc_lut_buf = generated
                    .values
                    .as_any()
                    .downcast_ref::<super::super::preparation::PreparationTable>()
                    .ok_or_else(|| "front spectral table has an invalid device payload".to_owned())?
                    .buffer
                    .clone();
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
                Some(self.chain_pointwise(
                    pipe,
                    &[&front_params_buf, &buf_a, &tc_lut_buf, &buf_b],
                    &[(1, false), (3, true)],
                    n_pixels,
                    true,
                    "front_transform",
                )?)
            }
            spektrafilm_backend::FrontPass::Mallett2019 { .. } => {
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
                Some(self.chain_pointwise(
                    pipe,
                    &[&front_params_buf, &buf_a, &buf_b],
                    &[(1, false), (2, true)],
                    n_pixels,
                    true,
                    "front_transform",
                )?)
            }
            spektrafilm_backend::FrontPass::ScanInverse(_) => None,
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
            self.chain_pointwise(
                pipe,
                &[&log10_params_buf, &buf_b],
                &[(1, true)],
                n_pixels,
                false,
                "log_exposure",
            )?
        };

        let bg_density_film = self.chain_pointwise(
            density_pipe.clone(),
            &[
                &film_density_params_buf,
                &buf_b,
                &film_log_exp_buf,
                &film_curves_buf,
                &buf_a,
            ],
            &[(1, false), (4, true)],
            n_pixels,
            true,
            "film_density",
        )?;
        let bg_print = self.chain_pointwise(
            print_pipe.clone(),
            &[
                &print_params_buf,
                &print_input,
                &film_cd_buf,
                &film_bd_buf,
                &print_illu_buf,
                &print_sens_buf,
                &print_output,
            ],
            &[(1, false), (6, true)],
            print_width * print_height,
            true,
            "print_spectral",
        )?;
        let bg_density_print = self.chain_pointwise(
            density_pipe.clone(),
            &[
                &print_density_params_buf,
                print_density_input,
                &print_log_exp_buf,
                &print_curves_buf,
                &tail_a,
            ],
            &[(1, false), (4, true)],
            tail_width * tail_height,
            true,
            "print_density",
        )?;
        let scan_input = if p.scan_film { &print_input } else { &tail_a };
        let bg_scan =
            self.build_scan_direct_state(p, scan_input, &tail_b, tail_width, tail_height)?;

        // All optical Gaussians execute sequentially on this operation's queue.
        // Size their two distinct working rasters to the largest active geometry;
        // a grain readout can make the scanner tail larger than the source.
        let source_gaussian = p
            .halation
            .as_ref()
            .is_some_and(super::super::halation::uses_gaussian)
            || p.camera_lens_blur_px.is_some_and(|sigma| sigma > 0.0)
            || p.dir_couplers
                .as_ref()
                .is_some_and(|dp| dp.diffusion_size_px > 0.0);
        let tail_gaussian = p.unsharp.is_some()
            || p.glare.as_ref().is_some_and(|gp| gp.blur_px > 0.0)
            || p.scanner_lens_blur_px.is_some_and(|sigma| sigma > 0.0);
        let gaussian_geometry = if source_gaussian
            && (!tail_gaussian
                || u64::from(image_width) * u64::from(image_height)
                    > u64::from(tail_width) * u64::from(tail_height))
        {
            (image_width, image_height)
        } else {
            (tail_width, tail_height)
        };
        let mut gaussian_scratch = GaussianScratch::new(gaussian_geometry.0, gaussian_geometry.1);

        // Device limits fail an active optical stage rather than omitting it.
        let halation_state = p
            .halation
            .as_ref()
            .map(|hp| {
                build_halation_state(
                    hp,
                    image_width,
                    image_height,
                    &buf_b,
                    &mut gaussian_scratch,
                    self,
                )
            })
            .transpose()?;

        // ── Highlight boost state ────────────────────────────────────────
        // Runs on raw film exposure immediately after the front pass.
        let highlight_state = p
            .highlight_boost
            .as_ref()
            .map(|hp| build_highlight_boost_state(&self.device, hp, n_pixels, &buf_b, self))
            .transpose()?;

        // The blur jobs retain handles to their shared operation-owned pages.
        let camera_lens_blur_state = p
            .camera_lens_blur_px
            .filter(|&sigma| sigma > 0.0)
            .map(|sigma| {
                let buffers = gaussian_scratch.buffers(self, image_width, image_height)?;
                build_simple_blur_state(
                    self,
                    sigma,
                    image_width,
                    image_height,
                    &buf_b,
                    &buffers.output,
                    &buffers.intermediate,
                    "camera_lens",
                )
            })
            .transpose()?;

        // ── Unsharp mask state ───────────────────────────────────────────
        // Complete the Gaussian before the bounded combine reads the distinct
        // blur output and writes the original scan raster.
        let unsharp_state = p
            .unsharp
            .as_ref()
            .map(|up| {
                build_unsharp_state(
                    &self.device,
                    up,
                    tail_width,
                    tail_height,
                    &tail_b,
                    &mut gaussian_scratch,
                    self,
                )
            })
            .transpose()?;
        // Encode the destination space before Grain Procedural preparation; all
        // resolution and grain passes stay in this resident command buffer.
        let grain_v2_state = p
            .grain_v2
            .as_ref()
            .map(|gp| {
                build_grain_v2_state(
                    &self.device,
                    gp,
                    tail_width,
                    tail_height,
                    &tail_b,
                    Some(p.scan_output_space),
                    self,
                )
            })
            .transpose()?;

        // Grain setup reports unsupported arithmetic and resource limits
        // before dispatch; absence must not silently omit an active field.
        let grain_v1_state = p
            .grain_v1
            .as_ref()
            .map(|gp| {
                build_spektra_grain_state(&self.device, image_width, image_height, &buf_a, gp, self)
            })
            .transpose()?;

        // ── Output gamut compression state ───────────────────────────────
        // Single per-pixel dispatch in place on buf_b, after glare and
        // before unsharp (the CPU scanning order). The C_max table is
        // baked once at pipeline construction; only the upload happens here.
        // The JzAzBz mode needs native shader binary64; a device without it
        // is a named failure at assembly, not a host fallback.
        let gamut_state = p
            .gamut
            .as_ref()
            .map(|gp| build_gamut_state(&self.device, gp, &tail_b, tail_width * tail_height, self))
            .transpose()?;

        // ── Glare state ──────────────────────────────────────────────────
        // Applied in place on buf_b (the scan_spectral output) just before
        // readback. Generates per-pixel lognormal noise into a scratch
        // buffer, optionally blurs it, then adds `g * rgb_offset[c]` to
        // the image. Its blur output and intermediate reuse the operation's
        // Gaussian pages only after earlier optical stages finish using them.
        let glare_state = p
            .glare
            .as_ref()
            .map(|gp| {
                build_glare_state(
                    &self.device,
                    gp,
                    tail_width,
                    tail_height,
                    &tail_b,
                    &mut gaussian_scratch,
                    self,
                )
            })
            .transpose()?;

        let scanner_lens_blur_state = p
            .scanner_lens_blur_px
            .filter(|&sigma| sigma > 0.0)
            .map(|sigma| {
                let buffers = gaussian_scratch.buffers(self, tail_width, tail_height)?;
                build_simple_blur_state(
                    self,
                    sigma,
                    tail_width,
                    tail_height,
                    &tail_b,
                    &buffers.output,
                    &buffers.intermediate,
                    "scanner_lens",
                )
            })
            .transpose()?;

        // ── DIR couplers state ────────────────────────────────────────────
        // Allocated lazily when the DIR stage is active. Reads buf_a
        // (density_cmy from film density curve) and buf_b (log_raw),
        // produces a corrected buf_a via re-interpolation of
        // `density_curves_0` against `log_raw - correction`. Stage accumulators
        // remain distinct from the shared Gaussian output and intermediate.
        let dir_state = p
            .dir_couplers
            .as_ref()
            .map(|dp| {
                build_dir_state(
                    &self.device,
                    dp,
                    image_width,
                    image_height,
                    &buf_a,
                    &buf_b,
                    &mut gaussian_scratch,
                    self,
                )
            })
            .transpose()?;

        // ── Input transfer decoding state ────────────────────────────────
        // Chain-head pass in place on buf_a (the uploaded input), before the
        // front transform. Keeping it here is what lets the caller upload the
        // input exactly as read and do no host image arithmetic to prepare it.
        let input_decode_state = p
            .input_decode
            .as_ref()
            .map(|dp| build_input_decode_state(&self.device, dp, &buf_a, n_pixels, self))
            .transpose()?;
        let convert_inverse_state = match &p.front {
            spektrafilm_backend::FrontPass::ScanInverse(params) => {
                Some(self.build_convert_inverse_state(params, &buf_a, &buf_b, n_pixels)?)
            }
            _ => None,
        };
        let scan_readout_state = p
            .scan_readout
            .map(|params| {
                self.build_scan_density_state(
                    params,
                    if p.scan_film { &print_input } else { &tail_a },
                    &tail_b,
                    tail_width,
                    tail_height,
                    p.scan_preparation_key,
                )
            })
            .transpose()?;

        let print_exposure_output = print_density_input.clone();
        Ok(ChainSetup {
            image_width,
            image_height,
            img_bytes,
            dye_state,
            fotufilm_state,
            dye_print_log,
            dye_collapse: collapse,
            tail_a,
            tail_b,
            tail_width,
            tail_height,
            film_density_output: print_input,
            film_density_width: print_width,
            film_density_height: print_height,
            print_exposure_output,
            print_fine_output: print_output,
            buf_a,
            buf_b,
            front_pipe,
            convert_inverse_state,
            scan_readout_state,
            input_decode_state,
            bg_log10,
            bg_density_film,
            bg_print,
            bg_density_print,
            bg_scan,
            halation_state,
            highlight_state,
            camera_lens_blur_state,
            unsharp_state,
            grain_v2_state,
            grain_v1_state,
            gamut_state,
            glare_state,
            scanner_lens_blur_state,
            dir_state,
        })
    }

    /// Execute a typed topology span over retained device buffers. Diagnostic
    /// boundaries select the already-computed buffer, never a CPU fence or a
    /// second rendering. Only the caller may materialize the chosen artifact.
    pub(super) fn run_film_chain_inner(
        &self,
        p: &spektrafilm_backend::FilmChainParams<'_>,
        input: &spektrafilm_backend::DeviceImage,
        setup_context: &spektrafilm_backend::telemetry::ObservationContext,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.check_cancelled()?;
        let t_start = std::time::Instant::now();
        validate_tap_span(p)?;
        if input.is_retained() {
            self.buffer_payload(input)?;
        }
        if p.inject == p.collect {
            return Ok(input.clone());
        }
        let includes =
            |tap| tap_index(p.inject) <= tap_index(tap) && tap_index(tap) < tap_index(p.collect);
        if (includes(Tap::LogEFilm) && p.density_cpu_stage.is_some())
            || (p.collect == Tap::RgbOut && p.scan_acquisition_cpu_stage.is_some())
        {
            return Err("selected GPU film chain cannot execute a CPU image-stage fence; retry with an explicit CPU request".into());
        }
        let setup = self.setup_chain(p, input)?;
        let ChainSetup {
            image_width,
            image_height,
            img_bytes,
            ref buf_a,
            ref buf_b,
            ref front_pipe,
            ref input_decode_state,
            ref convert_inverse_state,
            ref scan_readout_state,
            dye_state,
            fotufilm_state,
            ref dye_print_log,
            ref dye_collapse,
            ref tail_a,
            ref tail_b,
            tail_width,
            tail_height,
            ref film_density_output,
            film_density_width,
            film_density_height,
            ref print_exposure_output,
            ref print_fine_output,
            ref bg_log10,
            ref bg_density_film,
            ref bg_print,
            ref bg_density_print,
            ref bg_scan,
            ref halation_state,
            ref highlight_state,
            ref camera_lens_blur_state,
            ref unsharp_state,
            ref grain_v2_state,
            ref grain_v1_state,
            ref gamut_state,
            ref glare_state,
            ref scanner_lens_blur_state,
            ref dir_state,
            ..
        } = setup;
        // ── Assemble the stage list, then run it on the pipe ────────────
        // GPU nodes use ordered submissions while sharing resident buffers.
        // Keeping node workloads separate avoids a whole-frame watchdog job.
        // Each closure owns its bind groups and state; buffers use cheap handles.
        let mut stages: Vec<super::super::pipe::ChainStage<'_>> = Vec::new();
        let mut boundaries = [0usize; 7];

        // 0. Input transfer decoding on the uploaded samples, before the
        //    front transform assumes linear RGB.
        if let Some(ds) = input_decode_state.as_ref() {
            stages.push(super::super::pipe::ChainStage::Device {
                name: "input_decode",
                execute: Box::new(move || ds.submit(self)),
            });
        }
        if let Some(inverse) = convert_inverse_state.as_ref() {
            let buf_a = buf_a.clone();
            let buf_b = buf_b.clone();
            stages.push(super::super::pipe::ChainStage::Device {
                name: "convert_inverse_solver",
                execute: Box::new(move || {
                    inverse.submit(self)?;
                    let mut copy = super::super::grain::raster::RasterCommands::default();
                    copy.copy(&buf_b, 0, &buf_a, 0, img_bytes as u64);
                    copy.submit(self)
                }),
            });
        } else if let Some(front) = front_pipe.as_ref() {
            stages.push(super::super::pipe::ChainStage::Device {
                name: "front_transform",
                execute: Box::new(move || front.submit(self)),
            });
            // 1a. Highlight boost on raw, before optical scatter.
            if let Some(hs) = highlight_state.as_ref() {
                stages.push(super::super::pipe::ChainStage::Device {
                    name: "highlight_boost",
                    execute: Box::new(move || hs.submit(self)),
                });
            }
            if let Some(diffusion) = p.camera_diffusion.as_ref() {
                let image = buf_b.clone();
                stages.push(super::super::pipe::ChainStage::Device {
                    name: "camera_diffusion",
                    execute: Box::new(move || {
                        self.diffusion_transport(
                            image_width,
                            image_height,
                            &image,
                            &image,
                            diffusion,
                            None,
                        )
                    }),
                });
            }
            // 1a'. Camera optical diffusion runs on the device in this position —
            //     after highlight reconstruction, before the camera lens blur
            //     (the CPU filming order) — so there is no host fence here. The
            //     per-stage route reaches the same device convolution through
            //     `stages::filming::expose`.
            // 1b. Camera lens blur on raw before halation.
            if let Some(bs) = camera_lens_blur_state.as_ref() {
                let buf_b = buf_b.clone();
                stages.push(super::super::pipe::ChainStage::Device {
                    name: "camera_lens_blur",
                    execute: Box::new(move || bs.submit(self, &buf_b)),
                });
            }
            // 1c. Halation in-place on buf_b; its own working buffers hold
            //     the scatter results and the multi-bounce accumulator.
            if let Some(hs) = halation_state.as_ref() {
                stages.push(super::super::pipe::ChainStage::Device {
                    name: "halation",
                    execute: Box::new(move || hs.submit(self)),
                });
            }
            // 2. log10 in-place on buf_b: raw → log_raw (3 channels per thread).
            stages.push(super::super::pipe::ChainStage::Device {
                name: "log_exposure",
                execute: Box::new(move || bg_log10.submit(self)),
            });
        }
        boundaries[tap_index(Tap::LogEFilm)] = stages.len();
        if convert_inverse_state.is_none() {
            // 3. Density curve (film, normalized): buf_b (log_raw) → buf_a (density_cmy)
            stages.push(super::super::pipe::ChainStage::Device {
                name: "film_density",
                execute: Box::new(move || bg_density_film.submit(self)),
            });
            // 3b. DIR couplers: corrects log_raw in buf_b and re-interpolates
            //     the density curve back into buf_a.
            if let Some(ds) = dir_state.as_ref() {
                stages.push(super::super::pipe::ChainStage::Device {
                    name: "dir_couplers",
                    execute: Box::new(move || ds.submit_passes(self)),
                });
            }
        }
        // 3c'. Spektra v1 grain: the device field on the developed density in
        //     buf_a, in the same density-fence position (after the film
        //     density curve and DIR, before printing). The pass runs in place
        //     on the pre-bound buffer and reuses the shared FIR kernels for
        //     the final blur, so the print bind groups keep addressing it.
        //     A request the device cannot realize faithfully keeps the host
        //     fence above instead — never a different noise law.
        if let Some(gs) = grain_v1_state.as_ref() {
            stages.push(super::super::pipe::ChainStage::Device {
                name: super::super::grain::GRAIN_V1_STAGE_NAME,
                execute: Box::new(move || gs.submit(self)),
            });
        }
        // 3d. Dye forms its field after development and DIR. Downstream
        // consumers already bind its output pages; only the optional scanner
        // cell response needs a separate RGB destination. Check the bounded
        // status after execution and before publishing any carrier.
        let dye_status = dye_state.as_ref().map(|state| state.status_buffer());
        if let Some(dye_state) = dye_state {
            let scan_destination =
                (p.grain_dye_cell_spectral && p.scan_film).then(|| tail_b.clone());
            stages.push(super::super::pipe::ChainStage::Device {
                name: "grain_dye",
                execute: Box::new(move || {
                    dye_state.submit(self)?;
                    if let Some(scan) = scan_destination {
                        let payload = super::device_image::BufferPayload::from_pages(
                            self.backend_id(),
                            dye_state.output_pages(),
                            dye_state.width,
                            dye_state.height,
                        );
                        let mut copy = super::super::grain::raster::RasterCommands::default();
                        copy.copy(&payload.pages, 0, &scan, 0, payload.bytes);
                        copy.submit(self)?;
                    }
                    Ok(())
                }),
            });
        }
        if let Some(state) = fotufilm_state {
            let destination = film_density_output.clone();
            stages.push(super::super::pipe::ChainStage::Device {
                name: "grain_fotufilm",
                execute: Box::new(move || {
                    state.submit(self)?;
                    let payload = super::device_image::BufferPayload::from_pages(
                        self.backend_id(),
                        state.output_pages(),
                        state.width,
                        state.height,
                    );
                    let mut copy = super::super::grain::raster::RasterCommands::default();
                    copy.copy(&payload.pages, 0, &destination, 0, payload.bytes);
                    copy.submit(self)
                }),
            });
        }
        boundaries[tap_index(Tap::CmyFilm)] = stages.len();
        let (image_width, image_height) = (tail_width, tail_height);
        let n_pixels = image_width * image_height;
        let buf_a = tail_a;
        let buf_b = tail_b;
        // 4 + 5. Printing: print_spectral (buf_a → buf_b) then the print
        //     density curve (buf_b → buf_a). Skipped for scan_film — the
        //     scan pass consumes the film density already in buf_a. The
        //     enlarger LUT and the enlarger diffusion filter run on the
        //     device through the same dispatches the per-stage route uses,
        //     so the fused spectral pass stays the chain's only print
        //     acquisition.
        if !p.scan_film {
            if *dye_print_log {
                let source = film_density_output.clone();
                let output = print_fine_output.clone();
                let (width, height) = (film_density_width, film_density_height);
                stages.push(super::super::pipe::ChainStage::Device {
                    name: "dye_print_response_log",
                    execute: Box::new(move || {
                        let source = super::device_image::BufferPayload::from_pages(
                            self.backend_id(),
                            source,
                            width,
                            height,
                        );
                        let pages = self.dye_log10_pages(&source)?;
                        let payload = super::device_image::BufferPayload::from_pages(
                            self.backend_id(),
                            pages,
                            width,
                            height,
                        );
                        let mut copy = super::super::grain::raster::RasterCommands::default();
                        copy.copy(&payload.pages, 0, &output, 0, payload.bytes);
                        copy.submit(self)
                    }),
                });
            } else {
                if let Some(lut) = p.enlarger_lut.as_ref() {
                    let source = film_density_output.clone();
                    let output = print_fine_output.clone();
                    let count = film_density_width * film_density_height;
                    stages.push(super::super::pipe::ChainStage::Device {
                        name: "enlarger_lut",
                        execute: Box::new(move || {
                            self.spectral_lut_transport(
                                &source,
                                &output,
                                count,
                                |source, output, n| {
                                    self.execute_enlarger_lut_resident(source, output, n, lut)
                                },
                            )
                        }),
                    });
                    if p.enlarger_diffusion.is_none() {
                        let roundtrip = self.print_lut_roundtrip(
                            print_fine_output,
                            count,
                            p.print_exposure_scale,
                        )?;
                        stages.push(super::super::pipe::ChainStage::Device {
                            name: "print_lut_roundtrip",
                            execute: Box::new(move || roundtrip.submit(self)),
                        });
                    }
                } else {
                    stages.push(super::super::pipe::ChainStage::Device {
                        name: "print_spectral",
                        execute: Box::new(move || bg_print.submit(self)),
                    });
                }
            }
            if let Some(diffusion) = p.enlarger_diffusion.as_ref() {
                let image = print_fine_output.clone();
                stages.push(super::super::pipe::ChainStage::Device {
                    name: "enlarger_diffusion",
                    execute: Box::new(move || {
                        self.diffusion_transport(
                            film_density_width,
                            film_density_height,
                            &image,
                            &image,
                            diffusion,
                            Some(p.print_exposure_scale),
                        )
                    }),
                });
            }
            if let Some(factor) = dye_collapse {
                let source = print_fine_output.clone();
                let output = buf_b.clone();
                let (width, height) = (film_density_width, film_density_height);
                let factor = *factor;
                stages.push(super::super::pipe::ChainStage::Device {
                    name: "dye_aperture_collapse",
                    execute: Box::new(move || {
                        let source = super::device_image::BufferPayload::from_pages(
                            self.backend_id(),
                            source,
                            width,
                            height,
                        );
                        let (pages, width, height) =
                            self.dye_aperture_collapse_pages(&source, factor)?;
                        let payload = super::device_image::BufferPayload::from_pages(
                            self.backend_id(),
                            pages,
                            width,
                            height,
                        );
                        let mut copy = super::super::grain::raster::RasterCommands::default();
                        copy.copy(&payload.pages, 0, &output, 0, payload.bytes);
                        copy.submit(self)
                    }),
                });
            }
            boundaries[tap_index(Tap::LogEPrint)] = stages.len();
            stages.push(super::super::pipe::ChainStage::Device {
                name: "print_density",
                execute: Box::new(move || bg_density_print.submit(self)),
            });
        }
        boundaries[tap_index(Tap::CmyPrint)] = stages.len();
        // 6. Resident scan acquisition replaces density with linear RGB:
        // cell readout, calibrated negative reconstruction, scanner LUT, or
        // direct spectral integration. Standard setup measures the complete
        // post-optics frame only when its immutable reconstruction is absent.
        if p.grain_dye_cell_spectral && p.scan_film {
            // The cell response already is linear scanner RGB.
        } else if let Some(readout) = scan_readout_state.as_ref() {
            stages.push(super::super::pipe::ChainStage::Device {
                name: "scanner_optics",
                execute: Box::new(move || readout.submit_optics(self)),
            });
            if readout.has_setup() {
                stages.push(super::super::pipe::ChainStage::Device {
                    name: "standard_setup",
                    execute: Box::new(move || readout.submit_setup_observed(self, setup_context)),
                });
            }
            stages.push(super::super::pipe::ChainStage::Device {
                name: "scanner_density",
                execute: Box::new(move || readout.submit_inverse(self)),
            });
        } else if let Some(lut) = p.scanner_lut.as_ref() {
            let source = if p.scan_film {
                film_density_output.clone()
            } else {
                buf_a.clone()
            };
            let output = buf_b.clone();
            stages.push(super::super::pipe::ChainStage::Device {
                name: "scanner_lut",
                execute: Box::new(move || {
                    self.spectral_lut_transport(&source, &output, n_pixels, |source, output, n| {
                        self.execute_scanner_lut_resident(source, output, n, lut)
                    })
                }),
            });
        } else {
            stages.push(super::super::pipe::ChainStage::Device {
                name: "scan_spectral",
                execute: Box::new(move || bg_scan.submit(self)),
            });
        }
        // 6a. Scanner readout optics on the linear acquisition, before
        //     glare, gamut correction, and unsharp mask.
        if let Some(bs) = scanner_lens_blur_state.as_ref() {
            let buf_b = buf_b.clone();
            stages.push(super::super::pipe::ChainStage::Device {
                name: "scanner_lens_blur",
                execute: Box::new(move || bs.submit(self, &buf_b)),
            });
        }
        // 6b. Glare (in place on buf_b).
        if let Some(gs) = glare_state.as_ref() {
            stages.push(super::super::pipe::ChainStage::Device {
                name: "glare",
                execute: Box::new(move || gs.submit(self)),
            });
        }
        // 6c. Output gamut compression (in place on buf_b) — after glare,
        //     before unsharp, mirroring the CPU scanning order.
        if let Some(gs) = gamut_state.as_ref() {
            stages.push(super::super::pipe::ChainStage::Device {
                name: "output_gamut",
                execute: Box::new(move || gs.submit(self)),
            });
        }
        // 6d. Unsharp mask — writes the final image back to buf_b so the
        //     readback path below is unchanged.
        if let Some(us) = unsharp_state.as_ref() {
            stages.push(super::super::pipe::ChainStage::Device {
                name: "unsharp_mask",
                execute: Box::new(move || us.submit(self)),
            });
        }
        // 6e. Encode native destination RGB, then apply Grain Procedural.
        if let Some(gs) = grain_v2_state.as_ref() {
            let buf_b = buf_b.clone();
            stages.push(super::super::pipe::ChainStage::Device {
                name: "grain_v2",
                execute: Box::new(move || gs.submit(self, &buf_b)),
            });
        }
        // 6f. Output transfer is split around the terminal appearance. Encode
        // before CMYK; decode is appended after CMYK for linear output.
        let output_transfer_stage = |tp: &spektrafilm_backend::OutputTransferGpuParams,
                                     image: &Vec<ImagePage>|
         -> Result<super::super::pipe::ChainStage<'_>, String> {
            let transfer = self.chain_output_transfer(image, n_pixels, tp)?;
            Ok(super::super::pipe::ChainStage::Device {
                name: "output_transfer",
                execute: Box::new(move || transfer.submit(self)),
            })
        };
        if let Some(tp) = p.output_transfer.as_ref().filter(|tp| !tp.decode) {
            stages.push(output_transfer_stage(tp, buf_b)?);
        }
        // 6g. CMYK ink style (pipe architecture, S4): the last host
        //     finishing moves onto the device. Runs after the output
        //     transfer, so the buffer state is encoded exactly when the
        //     caller requested output encoding.
        if let Some(cp) = p.cmyk.as_ref() {
            let cmyk = self.chain_cmyk(buf_b, n_pixels, cp)?;
            stages.push(super::super::pipe::ChainStage::Device {
                name: "cmyk",
                execute: Box::new(move || cmyk.submit(self)),
            });
        }
        if let Some(tp) = p.output_transfer.as_ref().filter(|tp| tp.decode) {
            stages.push(output_transfer_stage(tp, buf_b)?);
        }
        boundaries[tap_index(Tap::RgbOut)] = stages.len();
        let start = boundaries[tap_index(p.inject)];
        let stop = boundaries[tap_index(p.collect)];
        stages.truncate(stop);
        stages.drain(..start);
        let (selected, width, height) = match p.collect {
            Tap::LogEFilm => (setup.buf_b.clone(), p.image_width, p.image_height),
            Tap::CmyFilm => (
                film_density_output.clone(),
                film_density_width,
                film_density_height,
            ),
            Tap::LogEPrint => (print_exposure_output.clone(), image_width, image_height),
            Tap::CmyPrint => (buf_a.clone(), image_width, image_height),
            Tap::RgbOut => (buf_b.clone(), image_width, image_height),
            Tap::RgbIn | Tap::RgbPre => unreachable!("equal boundary returned before setup"),
        };
        let final_output = (selected, width, height);
        // The chain ends on the device. The output buffer is returned as a
        // retained carrier, so the route's saving conversion and viewer read
        // it without a download and reupload; only the compatibility wrapper
        // materializes it.
        let cpu_setup_ms = t_start.elapsed().as_secs_f32() * 1000.0;
        self.pipe.run_chain(stages)?;
        if p.collect == Tap::RgbOut
            && let Some(readout) = scan_readout_state.as_ref()
        {
            readout.check(
                self,
                p.scan_readout.ok_or("scanner density parameters missing")?,
                p.scan_preparation_key,
                width,
                height,
            )?;
        }
        // The Dye pass publishes only a bounded flag; it is checked after the
        // run and before the output is published (pipe architecture), so an
        // out-of-bounds field fails instead of escaping.
        if start < boundaries[tap_index(Tap::CmyFilm)]
            && stop >= boundaries[tap_index(Tap::CmyFilm)]
            && let Some(status) = dye_status.as_ref()
        {
            self.check_dye_status(status)?;
        }
        tracing::debug!(
            target: "spektrafilm_gpu::wgpu_backend",
            cpu_setup_ms = format!("{cpu_setup_ms:.1}"),
            "film chain timings"
        );
        // The buffer carries the output transfer exactly when the assembled
        // pipe tail left encoded samples: the encode pass sets it, the decode
        // pass clears it, and without either the Procedural-grain pass decided
        // it before grain.
        let encoded = match p.output_transfer.as_ref() {
            Some(transfer) => !transfer.decode,
            None => p.grain_v2.is_some(),
        };
        let domain = match p.collect {
            Tap::LogEFilm | Tap::LogEPrint => spektrafilm_backend::SampleDomain::LogExposure,
            Tap::CmyFilm | Tap::CmyPrint => spektrafilm_backend::SampleDomain::Density,
            Tap::RgbOut if encoded => spektrafilm_backend::SampleDomain::Encoded,
            _ => spektrafilm_backend::SampleDomain::Linear,
        };
        let pixel_pitch_um = p
            .grain_dye
            .as_ref()
            .filter(|_| tap_index(p.collect) >= tap_index(Tap::CmyFilm))
            .map_or(p.pixel_pitch_um, |dye| {
                let axis = usize::from(p.image_height > p.image_width);
                dye.grid.pitch_um[axis] * f64::from(p.grain_dye_aperture.unwrap_or(1))
            });
        let pixel_pitch_um = p
            .grain_fotufilm
            .as_ref()
            .filter(|_| tap_index(p.collect) >= tap_index(Tap::CmyFilm))
            .map_or(pixel_pitch_um, |grain| {
                let axis = usize::from(p.image_height > p.image_width);
                grain.pitch_um[axis] * grain.size.clamp(0.5, 4.0)
            });
        let payload = super::device_image::BufferPayload::from_pages(
            self.backend_id(),
            final_output.0,
            final_output.1,
            final_output.2,
        );
        let (pages, width, height) = if let Some(rect) = p.grain_dye_crop {
            (self.dye_crop_pages(&payload, rect)?, rect[2], rect[3])
        } else {
            // These pages belong to this invocation, not a reusable scratch
            // pool. The retained carrier owns them after the chain is dropped.
            (payload.pages, final_output.1, final_output.2)
        };
        Ok(self.retain_image_pages(
            pages,
            width,
            height,
            domain,
            if p.collect == Tap::RgbOut {
                p.scan_output_space
            } else {
                input.space()
            },
            Some(pixel_pitch_um),
        ))
    }
}

// ── Bounded-execution preflight (pure, device-free) ──────────────────────────
//
// Complete rasters are copy transports. Individual numerical passes bind
// bounded windows; immutable coefficients retain their own binding minimum.

/// Conservative bound for the small fixed-size buffers the chain creates
/// besides the image and lookup buffers: the std140 param uniforms
/// (largest is the 128-byte `CmykParams`), the FIR kernels (≤ 513
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
    /// Largest static buffer: the f32-narrowed lookup tables (spectral
    /// densities, density curves, front-pass LUT, gamut `C_max`) plus the
    /// fixed-size param uniforms and kernels.
    pub(in crate::wgpu_backend) static_bytes: u64,
}

/// Derives the required device buffer sizes from the chain parameters.
/// Sizes mirror `run_film_chain_inner`: every lookup value is narrowed to
/// one f32 on upload, so `n` f64 samples become `4n` bytes and an
/// `[[f64; 3]]` table of `n` rows becomes `12n`.
pub(in crate::wgpu_backend) fn required_chain_buffers(
    p: &spektrafilm_backend::FilmChainParams<'_>,
) -> RequiredChainBuffers {
    let f32s = |count: usize| (count as u64).saturating_mul(4);
    let table = |rows: usize| f32s(rows).saturating_mul(3);
    // `scan_cd`/`scan_bd` feed from the film or the print spectral data,
    // matching the `scan_film` selection in `run_film_chain_inner`.
    let scan_wavelengths = if p.scan_film {
        p.film_channel_density.len()
    } else {
        p.print_channel_density.len()
    };
    let tc_lut_bytes = match &p.front {
        spektrafilm_backend::FrontPass::Hanatos2025 { tc_lut, .. } => (tc_lut.size as u64)
            .saturating_mul(tc_lut.size as u64)
            .saturating_mul(12),
        spektrafilm_backend::FrontPass::Mallett2019 { .. }
        | spektrafilm_backend::FrontPass::ScanInverse(_) => 0,
    };
    let dir_log_bytes = p.dir_couplers.map_or(0, |dp| f32s(dp.log_exposure.len()));
    let dir_curves_bytes = p
        .dir_couplers
        .map_or(0, |dp| table(dp.density_curves_0.len()));
    let cmax_bytes = p.gamut.map_or(0, |gp| f32s(gp.cmax.len()));
    // Spektra v1's device pass uploads the composite curve x column
    // (`knots` × 3) and the layer curve tensor (`knots` × 9) as f64 storage.
    let grain_v1_curve_bytes = p.grain_v1.as_ref().map_or(0, |gp| {
        gp.curve_x.len() as u64 * 8 + gp.curve_layers.len() as u64 * 8
    });
    let scan_axes_bytes = p
        .scan_readout
        .map_or(0, |p| (p.density.len().max(p.exposure.len()) as u64) * 8);
    let convert_table_bytes = match &p.front {
        spektrafilm_backend::FrontPass::ScanInverse(p) => {
            (p.dye.len().max(p.cmfs.len()) as u64) * 3 * 8
        }
        _ => 0,
    };
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
        f32s(p.scan_illuminant.len()),
        f32s(spektrafilm_math::spectral::N_WAVELENGTHS), // cmf_x/y/z
        tc_lut_bytes,
        dir_log_bytes,
        dir_curves_bytes,
        cmax_bytes,
        grain_v1_curve_bytes,
        scan_axes_bytes,
        convert_table_bytes,
        MAX_FIXED_AUX_BYTES,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    RequiredChainBuffers { static_bytes }
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

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::{ComputeBackend, FilmChainParams, FrontPass};
    use spektrafilm_backend::{Precision, RetainedImageMeta, SampleDomain};
    use spektrafilm_math::colorspace;
    use spektrafilm_math::image::ImageBuf;

    const MAT3: [[f64; 3]; 3] = [[0.0; 3]; 3];

    /// Minimal chain params: empty lookup tables, no optional stages. The
    /// geometry is declared directly so oversized sizes cost no allocation.
    fn params<'a>(width: u32, height: u32) -> FilmChainParams<'a> {
        FilmChainParams {
            inject: Tap::RgbPre,
            collect: Tap::RgbOut,
            enlarger_lut: None,
            scanner_lut: None,
            image_width: width,
            image_height: height,
            pixel_pitch_um: 1.0,
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
            grain_fotufilm: None,
            print_gamma: 1.0,
            print_channel_density: &[],
            print_base_density: &[],
            preflash: [0.0; 3],
            scan_illuminant: &[],
            scan_normalization: 1.0,
            scan_xyz_to_rgb: &MAT3,
            scan_cat: &MAT3,
            scan_base_xyz_to_rgb: &MAT3,
            scan_film: false,
            halation: None,
            dir_couplers: None,
            glare: None,
            gamut: None,
            unsharp: None,
            grain_v2: None,
            grain_v1: None,
            scan_output_space: &spektrafilm_math::colorspace::SRGB,
            camera_lens_blur_px: None,
            camera_diffusion: None,
            enlarger_diffusion: None,
            scanner_lens_blur_px: None,
            highlight_boost: None,
            grain_dye: None,
            grain_dye_cell_spectral: false,
            grain_dye_aperture: None,
            grain_dye_crop: None,
            density_cpu_stage: None,
            scan_acquisition_cpu_stage: None,
            scan_readout: None,
            scan_preparation_key: None,
            output_transfer: None,
            cmyk: None,
            input_decode: None,
        }
    }

    #[test]
    fn lens_scratch_is_lazy_and_active_states_retain_their_working_pages() {
        use spektrafilm_backend::telemetry::{CollectionMode, ObservationContext};

        let gpu = crate::test_support::explicit_adapter();
        let knots = [-1.0, 0.0, 1.0];
        let curves = [[0.0; 3], [1.0; 3], [2.0; 3]];
        let spectral = [[0.0; 3]; 81];
        let base = [0.0; 81];
        let light = [1.0; 81];
        let mut p = params(256, 8);
        p.film_log_exposure = &knots;
        p.film_density_curves_normalized = &curves;
        p.print_log_exposure = &knots;
        p.print_density_curves = &curves;
        p.film_channel_density = &spectral;
        p.print_channel_density = &spectral;
        p.film_base_density = &base;
        p.print_base_density = &base;
        p.print_illuminant = &light;
        p.scan_illuminant = &light;
        p.print_sensitivity = &spectral;
        let mut host = ImageBuf::new(p.image_width, p.image_height);
        let center = (4 * p.image_width as usize + 128) * 3;
        host.data[center..center + 3].copy_from_slice(&[0.25 as _, 0.5 as _, 1.0 as _]);
        let input = gpu
            .retain_image(
                &host,
                &RetainedImageMeta {
                    width: p.image_width,
                    height: p.image_height,
                    domain: SampleDomain::Linear,
                    precision: Precision::host(),
                    space: &colorspace::SRGB,
                    pixel_pitch_um: None,
                },
            )
            .unwrap();
        // Warm coefficient resources, so the budget measures raster working
        // storage rather than the small immutable calibration tables.
        drop(gpu.setup_chain(&p, &input).unwrap());
        let raster_bytes = u64::from(p.image_width) * u64::from(p.image_height) * 12;
        for inactive in [None, Some(0.0), Some(-1.0)] {
            p.camera_lens_blur_px = inactive;
            p.scanner_lens_blur_px = inactive;
            let context = ObservationContext::new(CollectionMode::Summary);
            let mut observed = gpu.derived_explicit();
            observed.context_mut(context.clone());
            let _setup = observed.setup_chain(&p, &input).unwrap();
            // Two chain rasters plus eleven pointwise windows; inactive
            // camera/scanner optics may not add full-raster scratch buffers.
            assert!(
                context.snapshot().totals.buffer_create_bytes.unwrap() <= 13 * raster_bytes + 1024
            );
        }

        p.camera_lens_blur_px = Some(1.0);
        p.scanner_lens_blur_px = Some(2.0);
        let setup = gpu.setup_chain(&p, &input).unwrap();
        for page in &setup.buf_b {
            let first = page.first_pixel as usize * 3;
            let values = scalars_to_f32(&host.data[first..first + page.n_pixels as usize * 3]);
            gpu.queue
                .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&values));
        }
        // setup_chain's local scratch owners have already gone out of scope.
        // Both optical states must retain the pages and preserve active output.
        setup
            .camera_lens_blur_state
            .as_ref()
            .unwrap()
            .submit(&gpu, &setup.buf_b)
            .unwrap();
        setup
            .scanner_lens_blur_state
            .as_ref()
            .unwrap()
            .submit(&gpu, &setup.tail_b)
            .unwrap();
        let output = gpu.retain_image_pages(
            setup.tail_b.clone(),
            p.image_width,
            p.image_height,
            SampleDomain::Linear,
            &colorspace::SRGB,
            None,
        );
        let expected = gpu.gaussian_blur_gpu(&host, 1.0).unwrap();
        let expected = gpu.gaussian_blur_gpu(&expected, 2.0).unwrap();
        for (actual, expected) in output
            .to_host(gpu.backend())
            .unwrap()
            .data
            .iter()
            .zip(&expected.data)
        {
            assert!((*actual as f64 - *expected as f64).abs() < 2e-6);
        }
    }

    #[test]
    fn retained_tap_segments_preserve_exposure_and_development_boundaries() {
        use spektrafilm_backend::ComputeBackend;
        let backend = crate::test_support::adapter();
        let knots = [-1.0, 0.0, 1.0];
        let curves = [[0.0; 3], [1.0; 3], [2.0; 3]];
        let spectral = [[0.0; 3]; 81];
        let base = [0.0; 81];
        let light = [1.0; 81];
        let mut p = params(1, 1);
        p.front = FrontPass::Mallett2019 {
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        p.film_log_exposure = &knots;
        p.film_density_curves_normalized = &curves;
        p.print_log_exposure = &knots;
        p.print_density_curves = &curves;
        p.film_channel_density = &spectral;
        p.print_channel_density = &spectral;
        p.film_base_density = &base;
        p.print_base_density = &base;
        p.print_illuminant = &light;
        p.scan_illuminant = &light;
        p.print_sensitivity = &spectral;
        p.collect = Tap::LogEFilm;
        let source = ImageBuf::from_data(1, 1, f32_to_scalars(vec![0.1, 1.0, 10.0]));
        let input = backend
            .retain_image(
                &source,
                &RetainedImageMeta {
                    width: 1,
                    height: 1,
                    domain: SampleDomain::Linear,
                    precision: Precision::host(),
                    space: &colorspace::SRGB,
                    pixel_pitch_um: Some(1.0),
                },
            )
            .unwrap();
        let repeated = ImageBuf::from_data(3, 1, source.data.repeat(3));
        let split = (0..3)
            .map(|i| {
                let samples: Vec<f32> = repeated.data[i * 3..i * 3 + 3]
                    .iter()
                    .map(|&v| v as f32)
                    .collect();
                super::super::super::paged::ImagePage {
                    first_pixel: i as u32,
                    n_pixels: 1,
                    buffer: backend
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("split_source_pixel"),
                            contents: bytemuck::cast_slice(&samples),
                            usage: wgpu::BufferUsages::STORAGE
                                | wgpu::BufferUsages::COPY_SRC
                                | wgpu::BufferUsages::COPY_DST,
                        }),
                }
            })
            .collect();
        let paged = backend.retain_image_pages(
            split,
            3,
            1,
            SampleDomain::Linear,
            &colorspace::SRGB,
            Some(1.0),
        );
        let contiguous = backend
            .retain_image(
                &repeated,
                &RetainedImageMeta {
                    width: 3,
                    height: 1,
                    domain: SampleDomain::Linear,
                    precision: Precision::host(),
                    space: &colorspace::SRGB,
                    pixel_pitch_um: Some(1.0),
                },
            )
            .unwrap();
        p.image_width = 3;
        for carrier in [&contiguous, &paged] {
            p.collect = Tap::CmyFilm;
            let first = backend.run_film_chain_retained(carrier, &p).unwrap();
            let second = backend.run_film_chain_retained(carrier, &p).unwrap();
            assert_eq!(
                first.to_host(backend.backend()).unwrap().data,
                second.to_host(backend.backend()).unwrap().data
            );
            assert_eq!(
                carrier.to_host(backend.backend()).unwrap().data,
                repeated.data
            );
            let mixed = backend
                .mix_output_images(
                    &first,
                    carrier,
                    &spektrafilm_backend::OutputMixGpuParams {
                        amount: 0.0,
                        encoded: false,
                        cctf_kind: 0,
                    },
                )
                .unwrap();
            assert_eq!(
                mixed.to_host(backend.backend()).unwrap().data,
                repeated.data
            );
            p.density_cpu_stage = Some(Box::new(|_| panic!("forbidden CPU image stage")));
            assert!(backend.run_film_chain_retained(carrier, &p).is_err());
            assert_eq!(
                carrier.to_host(backend.backend()).unwrap().data,
                repeated.data
            );
            p.density_cpu_stage = None;
        }
        p.image_width = 1;
        p.collect = Tap::LogEFilm;
        let mut identity_params = params(1, 1);
        identity_params.inject = Tap::CmyFilm;
        identity_params.collect = Tap::CmyFilm;
        let identity = backend
            .run_film_chain_retained(&input, &identity_params)
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(
            input.payload().unwrap(),
            identity.payload().unwrap()
        ));
        assert_eq!(identity.domain(), input.domain());
        assert_eq!(identity.pixel_pitch_um(), input.pixel_pitch_um());
        assert_eq!((identity.width(), identity.height()), (1, 1));
        let exposure = backend.run_film_chain_retained(&input, &p).unwrap();
        assert_eq!(exposure.domain(), SampleDomain::LogExposure);
        for (actual, expected) in exposure
            .to_host(backend.backend())
            .unwrap()
            .data
            .iter()
            .zip([-1.0, 0.0, 1.0])
        {
            assert!((*actual as f64 - expected).abs() < 2e-6);
        }
        p.inject = Tap::LogEFilm;
        p.collect = Tap::CmyFilm;
        // Re-running the front/log passes would produce the wrong density;
        // running the scanner tail would produce zeros with this scan matrix.
        let density = backend.run_film_chain_retained(&exposure, &p).unwrap();
        assert_eq!(density.domain(), SampleDomain::Density);
        for (actual, expected) in density
            .to_host(backend.backend())
            .unwrap()
            .data
            .iter()
            .zip([0.0, 1.0, 2.0])
        {
            assert!((*actual as f64 - expected).abs() < 2e-6);
        }
        p.inject = Tap::LogEPrint;
        p.collect = Tap::CmyPrint;
        let paper = backend.run_film_chain_retained(&exposure, &p).unwrap();
        assert_eq!(paper.domain(), SampleDomain::Density);
        for (actual, expected) in paper
            .to_host(backend.backend())
            .unwrap()
            .data
            .iter()
            .zip([0.0, 1.0, 2.0])
        {
            assert!((*actual as f64 - expected).abs() < 2e-6);
        }
        p.inject = Tap::LogEFilm;
        p.collect = Tap::CmyFilm;
        p.density_cpu_stage = Some(Box::new(|_| {
            panic!("GPU diagnostics must not run CPU image arithmetic")
        }));
        let error = backend.run_film_chain_retained(&exposure, &p).unwrap_err();
        assert!(error.contains("CPU image-stage fence"), "{error}");
    }

    #[test]
    fn dye_carriers_preserve_segmented_routes_crops_and_prior_outputs() {
        use spektrafilm_backend::dye_params::{
            DyeChannelParts, DyeChannelTablesGpu, DyeGpuParams, DyeReadoutGrid,
        };

        let gpu = crate::test_support::adapter();
        let knots = [-1.0, 0.0, 1.0];
        let curves = [[0.0; 3], [1.0; 3], [2.0; 3]];
        let spectral = [[0.1, 0.2, 0.3]; 81];
        let base = [0.0; 81];
        let light = [1.0; 81];
        let sensitivity = [[0.01, 0.02, 0.03]; 81];
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let chain_params = |width, height| {
            let mut p = params(width, height);
            p.film_log_exposure = &knots;
            p.film_density_curves_normalized = &curves;
            p.print_log_exposure = &knots;
            p.print_density_curves = &curves;
            p.film_channel_density = &spectral;
            p.print_channel_density = &spectral;
            p.film_base_density = &base;
            p.print_base_density = &base;
            p.print_illuminant = &light;
            p.scan_illuminant = &light;
            p.print_sensitivity = &sensitivity;
            p.scan_xyz_to_rgb = &identity;
            p.scan_cat = &identity;
            p.scan_base_xyz_to_rgb = &identity;
            p
        };
        let retain = |image: &ImageBuf, domain| {
            gpu.retain_image(
                image,
                &RetainedImageMeta {
                    width: image.width,
                    height: image.height,
                    domain,
                    precision: Precision::host(),
                    space: &colorspace::SRGB,
                    pixel_pitch_um: Some(1.0),
                },
            )
            .unwrap()
        };
        let host = ImageBuf::from_data(
            9,
            7,
            (0..9 * 7 * 3)
                .map(|i| spektrafilm_math::precision::from_f64(-0.75 + f64::from(i % 17) / 64.0))
                .collect(),
        );
        let input = retain(&host, SampleDomain::LogExposure);
        let mut p = chain_params(9, 7);
        p.inject = Tap::LogEFilm;
        p.collect = Tap::CmyFilm;
        let density = gpu
            .run_film_chain_retained(&input, &p)
            .unwrap()
            .to_host(gpu.backend())
            .unwrap();
        let dye_knots = [0.0, 1.0];
        let inverse = [1.0];
        let zero = [0.0; 2];
        for (width, height) in [(9, 7), (11, 9)] {
            let dye = DyeGpuParams {
                tables: DyeChannelTablesGpu::pack(std::array::from_fn(|_| DyeChannelParts {
                    min: 0.1,
                    cap_total: 1.0,
                    cap: [1.0, 0.0, 0.0],
                    frac: [1.0, 0.0, 0.0],
                    area: [0.2; 3],
                    uniformity: 0.0,
                    lookup_sign: 1.0,
                    knots: &dye_knots,
                    inv_dx: &inverse,
                    layers: [&dye_knots, &zero, &zero],
                })),
                cell_um: 1.0,
                taps: vec![0.25, 0.5, 0.25],
                source_pitch_um: 1.0,
                grid: DyeReadoutGrid {
                    origin_um: [-0.25, 0.375],
                    pitch_um: [0.875, 1.125],
                    width,
                    height,
                },
                seed: 0x1234_5678_9abc_def0,
                channel_correlation: 0.3,
                expectation: false,
            };
            let expected_field = gpu.dye_field(&density, &dye).unwrap();
            let mut downstream = chain_params(width, height);
            downstream.inject = Tap::CmyFilm;
            let field = retain(&expected_field, SampleDomain::Density);
            let expected = gpu
                .run_film_chain_retained(&field, &downstream)
                .unwrap()
                .to_host(gpu.backend())
                .unwrap();
            downstream.grain_dye = Some(dye.clone());
            let resumed = gpu.run_film_chain_retained(&field, &downstream).unwrap();
            assert_eq!(resumed.to_host(gpu.backend()).unwrap().data, expected.data);
            p.grain_dye = Some(dye);
            p.collect = Tap::RgbOut;
            let result = gpu.run_film_chain_retained(&input, &p).unwrap();
            assert_eq!(result.to_host(gpu.backend()).unwrap().data, expected.data);

            p.collect = Tap::CmyFilm;
            let field_result = gpu.run_film_chain_retained(&input, &p).unwrap();
            assert_eq!(field_result.domain(), SampleDomain::Density);
            assert_eq!(
                field_result.to_host(gpu.backend()).unwrap().data,
                expected_field.data
            );
            p.collect = Tap::RgbOut;
            p.grain_dye_crop = Some([1, 1, width - 2, height - 2]);
            let crop = gpu.run_film_chain_retained(&input, &p).unwrap();
            let expected_crop: Vec<_> = (1..height - 1)
                .flat_map(|y| {
                    let first = (y * width + 1) as usize * 3;
                    expected.data[first..first + (width - 2) as usize * 3]
                        .iter()
                        .copied()
                })
                .collect();
            assert_eq!((crop.width(), crop.height()), (width - 2, height - 2));
            assert_eq!(crop.to_host(gpu.backend()).unwrap().data, expected_crop);

            p.grain_dye_crop = None;
            p.grain_dye.as_mut().unwrap().seed += 1;
            let next = gpu.run_film_chain_retained(&input, &p).unwrap();
            assert_ne!(next.to_host(gpu.backend()).unwrap().data, expected.data);
            assert_eq!(result.to_host(gpu.backend()).unwrap().data, expected.data);
            assert_eq!(
                field_result.to_host(gpu.backend()).unwrap().data,
                expected_field.data
            );
            assert_eq!(crop.to_host(gpu.backend()).unwrap().data, expected_crop);
            assert_eq!(input.to_host(gpu.backend()).unwrap().data, host.data);
        }
    }

    #[test]
    fn cancelled_identity_tap_discards_carrier_without_poisoning_reuse() {
        use spektrafilm_backend::ComputeBackend;
        use std::sync::{Arc, atomic::AtomicBool};
        let gpu = crate::test_support::explicit_adapter();
        let image = ImageBuf::from_data(1, 1, f32_to_scalars(vec![0.125, 0.5, 0.75]));
        let source = gpu
            .retain_image(
                &image,
                &RetainedImageMeta {
                    width: 1,
                    height: 1,
                    domain: SampleDomain::Linear,
                    precision: Precision::F32,
                    space: &colorspace::SRGB,
                    pixel_pitch_um: None,
                },
            )
            .unwrap();
        let mut p = params(1, 1);
        p.collect = p.inject;
        let mut cancelled = gpu.derived_explicit();
        cancelled.pipe = gpu
            .pipe
            .with_cancellation(Some(Arc::new(AtomicBool::new(true))));
        assert_eq!(
            cancelled.run_film_chain_retained(&source, &p).unwrap_err(),
            "operation cancelled"
        );
        let healthy = gpu.run_film_chain_retained(&source, &p).unwrap();
        assert!(Arc::ptr_eq(
            source.payload().unwrap(),
            healthy.payload().unwrap()
        ));
        assert_eq!(
            scalars_to_f32(&healthy.to_host(gpu.backend()).unwrap().data).as_ref(),
            &[0.125, 0.5, 0.75]
        );
    }

    #[test]
    fn diagnostic_taps_reject_absent_or_reversed_topology_boundaries() {
        let mut p = params(1, 1);
        p.inject = Tap::CmyFilm;
        p.collect = Tap::LogEFilm;
        assert!(validate_tap_span(&p).unwrap_err().contains("no node path"));
        p.scan_film = true;
        p.inject = Tap::LogEPrint;
        p.collect = Tap::LogEPrint;
        assert!(
            validate_tap_span(&p)
                .unwrap_err()
                .contains("not part of this topology")
        );
    }

    #[test]
    fn required_chain_buffers_mirror_the_chain_arithmetic() {
        const W: u32 = 8;
        const H: u32 = 4;
        let log_exp = [0.0f64; 5];
        let tc_lut = spektrafilm_math::spectral::TcLut {
            size: 16,
            channels: 3,
            data: vec![0.0; 16 * 16 * 3],
        };
        let p = FilmChainParams {
            front: FrontPass::Hanatos2025 {
                tc_lut: &tc_lut,
                generated: None,
                rgb_to_adapted_xyz: MAT3,
            },
            film_log_exposure: &log_exp,
            ..params(W, H)
        };
        let required = required_chain_buffers(&p);
        // A non-empty `[[f64; 3]]` table narrows to 12 bytes per row and
        // outgrows the fixed-aux bound, so the table path itself is pinned.
        let density_rows = [[0.0f64; 3]; 400];
        let table_params = FilmChainParams {
            film_channel_density: &density_rows,
            ..params(W, H)
        };
        assert_eq!(
            required_chain_buffers(&table_params).static_bytes,
            400u64 * 12
        );
        // Resource planning follows declared lattice geometry even when a
        // generated retained table has no host sample vector.
        assert_eq!(required.static_bytes, MAX_FIXED_AUX_BYTES);
    }
}
