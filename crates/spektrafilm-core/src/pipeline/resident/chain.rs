use super::super::Pipeline;
use super::decision::{ResidentDecision, ResidentFallbackReason};
use crate::stages::StageObservation;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::image::ImageBuf;

pub(super) type CpuStageFn<'a> = Box<dyn Fn(&ImageBuf) -> Result<ImageBuf, String> + 'a>;

impl Pipeline {
    pub(in crate::pipeline) fn try_gpu_resident(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
        color_ref: &crate::color_reference::ColorReference,
        pixel_size_um: f64,
        ae_ev: f64,
    ) -> Result<Option<ImageBuf>, String> {
        if !backend.is_gpu() {
            return Ok(None);
        }
        match self.resident_decision() {
            ResidentDecision::UseResident => {}
            ResidentDecision::PerStage { reasons } => {
                if let Some(context) = backend.observation_context() {
                    for reason in &reasons {
                        use spektrafilm_backend::telemetry::ResidentDeclineReason as R;
                        let reason = match reason {
                            ResidentFallbackReason::WorkflowRoute => R::WorkflowRoute,
                            ResidentFallbackReason::FilmGrainV3FieldCpu => R::GrainV3FieldCpu,
                            ResidentFallbackReason::FilmGrainFotufilmCpu => R::FotufilmFilmCpu,
                            ResidentFallbackReason::UnsupportedOutputGamut => {
                                R::UnsupportedOutputGamut
                            }
                            ResidentFallbackReason::BlurRadiusExceedsBackendSupport => {
                                R::BlurRadiusExceedsBackendSupport
                            }
                            ResidentFallbackReason::MissingResidentFrontPass => {
                                R::MissingResidentFrontPass
                            }
                            ResidentFallbackReason::MallettExecutionParity => {
                                R::MallettExecutionParity
                            }
                        };
                        context.decline_resident(reason);
                    }
                }
                // An explicitly requested GPU backend must not silently run
                // capability-missing stages on the CPU (pipe architecture
                // fence policy): fail at assembly instead of demoting the
                // route. Frontend-class reasons select a route; they do not
                // demote one and stay permitted.
                if backend.explicit_gpu_request() {
                    // A readout geometry the resident chain cannot express
                    // selects the per-stage route; the device realizes the
                    // field there, so that decline is route selection rather
                    // than a capability gap. A request the device cannot
                    // realize at all stays capability-missing.
                    let host_field =
                        crate::stages::grain_v3::published_host_field_reason(&self.params)
                            .is_some();
                    let capability_missing = reasons.iter().any(|r| match r {
                        ResidentFallbackReason::WorkflowRoute
                        | ResidentFallbackReason::MissingResidentFrontPass
                        | ResidentFallbackReason::MallettExecutionParity
                        | ResidentFallbackReason::UnsupportedOutputGamut => false,
                        ResidentFallbackReason::FilmGrainV3FieldCpu => host_field,
                        _ => true,
                    });
                    if capability_missing {
                        return Err(format!(
                            "explicit GPU request cannot run the complete computation scope \
                             on WGPU (capability-missing stages: {:?}); retry with an \
                             explicit CPU request to use the per-stage route",
                            reasons
                        ));
                    }
                }
                tracing::info!(
                    target: "spektrafilm_core::pipeline",
                    backend = backend.name(),
                    execution = "per_stage_cpu",
                    fallback_reasons = ?reasons,
                    "using per-stage path because resident GPU execution is unavailable"
                );
                return Ok(None);
            }
        }
        // Input transfer decoding is shared preprocessing for the resident
        // route (pipe architecture, capability class): the front pass
        // assumes linear RGB exactly like `expose` produces for the
        // per-stage route. Same expression as `expose`. It runs only on the
        // resident arm so a declined route never decodes twice (the
        // per-stage `expose` performs its own decode).
        let decoded;
        let image = if self.params.io.input_cctf_decoding {
            let _decode = StageObservation::cpu(
                backend,
                "input_transfer",
                spektrafilm_backend::telemetry::CpuReason::InputTransferDecoding,
            );
            let input_space =
                spektrafilm_math::colorspace::resolve(&self.params.io.input_color_space)
                    .expect("input color space must be validated before filming");
            let mut rgb = image.clone();
            use rayon::prelude::*;
            use spektrafilm_math::precision::{from_f64, to_f64};
            rgb.data.par_iter_mut().for_each(|v| {
                *v = from_f64(spektrafilm_math::colorspace::cctf_decode(
                    to_f64(*v),
                    input_space.cctf,
                ));
            });
            decoded = rgb;
            &decoded
        } else {
            image
        };
        // A corrected scan — slide points, the B&W/slide luminance remap,
        // the negative-to-positive density reading, or Digital Reference —
        // keeps the resident route: the scanner readout fence applies the
        // interpretation after the acquisition, matching the per-stage
        // order, and the acquisition itself runs on the host whenever the
        // CPU route replaces it (PCHIP scanner LUT, receiver capture,
        // density reading, Digital Reference).
        let Some(front) = self.resident_front_pass(color_ref, ae_ev) else {
            if backend.explicit_gpu_request() {
                return Err(
                    "explicit GPU request cannot prepare the resident front pass on WGPU; retry with an explicit CPU request"
                        .into(),
                );
            }
            return Ok(None);
        };

        let film_curves = crate::chain_prep::FilmCurves::prepare(&self.film);
        let film_log_exp = film_curves.log_exposure;
        let film_curves_norm = &film_curves.normalized;
        let film_channel_density = crate::chain_prep::channel_density(&self.film);
        let film_base_density = &self.film.data.base_density;

        let print_sens = self.print.sensitivity_f64();
        let print_curves = crate::chain_prep::PrintCurves::prepare(&self.print, &self.params)
            .expect("print density-curve model was validated at pipeline construction");
        let print_channel_density = crate::chain_prep::channel_density(&self.print);
        let print_base_density = &self.print.data.base_density;

        // Scanning: viewing illuminant + normalization + combined XYZ→RGB
        // matrix. For scan_film we scan the developed film directly, so the
        // viewing illuminant and dye-density wavelength count come from the
        // film, not the print (mirrors the CPU scanning stage, which is
        // handed `self.film` as its profile when scan_film).
        let scan_profile = if self.params.io.scan_film {
            &self.film
        } else {
            &self.print
        };
        let prepared = crate::chain_prep::PreparedChain::for_scan(scan_profile, &self.params, None);
        let scan_context = &prepared.scan;
        let viewing_illu: &[f64] = &scan_context.illuminant;
        let scan_norm = scan_context.normalization;
        let scan_xyz_to_rgb = prepared.scan_xyz_to_rgb;

        // GPU shaders take f32 — narrow the working-geometry pitch once.
        let pix_um = pixel_size_um as f32;

        // Keep the two print exposure halves separate: preflash is added
        // before the inner floor/log, while print exposure and BW correction
        // multiply the recovered linear raw before the outer floor/log.
        let print_exposure_scale =
            self.params.enlarger.print_exposure as f64 * color_ref.printing_exposure_correction;
        let print_norm_factor = self.print_exposure_factor;

        // Halation in the resident chain — only built when the halation
        // stage is active. Mirrors `apply_halation_um`: averages the
        // per-channel µm sigmas, converts to pixel space, and passes the
        // resulting scalars to the shaders.
        let halation = if self.params.film_render.halation.active {
            let h = &self.params.film_render.halation;
            // GPU shaders take f32 — narrow at the boundary.
            let avg_f64 = |a: [f64; 3]| (a[0] + a[1] + a[2]) / 3.0;
            let strength_avg = (avg_f64(h.halation_strength) * h.halation_amount) as f32;
            let a_tot = [
                (h.halation_strength[0] * h.halation_amount) as f32,
                (h.halation_strength[1] * h.halation_amount) as f32,
                (h.halation_strength[2] * h.halation_amount) as f32,
            ];
            Some(spektrafilm_backend::HalationGpuParams {
                scatter_amount: h.scatter_amount as f32,
                scatter_core_px: (avg_f64(h.scatter_core_um) * h.scatter_spatial_scale
                    / pix_um as f64) as f32,
                scatter_tail_px: (avg_f64(h.scatter_tail_um) * h.scatter_spatial_scale
                    / pix_um as f64) as f32,
                scatter_tail_weight: [
                    h.scatter_tail_weight[0] as f32,
                    h.scatter_tail_weight[1] as f32,
                    h.scatter_tail_weight[2] as f32,
                ],
                source_compression: h.source_compression as f32,
                halation_amount: h.halation_amount as f32,
                halation_strength_avg: strength_avg,
                halation_a_tot: a_tot,
                halation_first_sigma_px: (avg_f64(h.halation_first_sigma_um)
                    * h.halation_spatial_scale
                    / pix_um as f64) as f32,
                halation_n_bounces: h.halation_n_bounces,
                halation_bounce_decay: h.halation_bounce_decay as f32,
                halation_renormalize: h.halation_renormalize,
            })
        } else {
            None
        };

        // DIR couplers in the resident chain. Mirrors CPU
        // `apply_density_correction`: build the scaled couplers matrix and
        // pre-compute the "density curves before DIR" once. The shader
        // re-interpolates these against `log_raw - correction`.
        // Held in this binding so `&density_curves_0_f64` outlives the
        // backend call.
        let dir = &self.params.film_render.dir_couplers;
        let dir_inputs = if dir.active {
            let matrix = crate::chain_prep::dir_matrix(dir);
            let prepared = spektrafilm_model::couplers::prepare_dir(
                &film_curves.raw,
                film_log_exp,
                &matrix,
                dir.amount,
                self.film.is_positive(),
                dir.langmuir_donor_k_rgb,
                dir.langmuir_receiver_k_rgb,
            );
            Some((
                prepared,
                pixel_size_um,
                dir.diffusion_size_um,
                dir.diffusion_tail_um,
                dir.diffusion_tail_weight,
                self.film.is_positive(),
                self.params.film_render.density_curve_gamma as f64,
            ))
        } else {
            None
        };
        let dir_couplers = dir_inputs.as_ref().map(|d| {
            // GPU shader path is f32 — narrow at the boundary.
            let m = d.0.matrix_scaled;
            let matrix_f32: [[f32; 3]; 3] = [
                [m[0][0] as f32, m[0][1] as f32, m[0][2] as f32],
                [m[1][0] as f32, m[1][1] as f32, m[1][2] as f32],
                [m[2][0] as f32, m[2][1] as f32, m[2][2] as f32],
            ];
            let dm = d.0.density_max;
            let narrow = |v: [f64; 3]| [v[0] as f32, v[1] as f32, v[2] as f32];
            spektrafilm_backend::DirCouplersGpuParams {
                couplers_matrix_scaled: matrix_f32,
                density_max: [dm[0] as f32, dm[1] as f32, dm[2] as f32],
                is_positive: d.5,
                diffusion_size_px: (d.2 / d.1) as f32,
                diffusion_tail_px: (d.3 / d.1) as f32,
                diffusion_tail_weight: d.4 as f32,
                // The reversal donor stays linear (the saturation sits on
                // the receiver side), mirroring `apply_density_correction`.
                donor_k: (!d.5).then(|| narrow(d.0.donor_k)),
                donor_ref: narrow(d.0.donor_ref),
                receiver_knee: d.0.receiver_knee.map(narrow),
                receiver_ref: narrow(d.0.receiver_ref.unwrap_or([0.0; 3])),
                density_curves_0: &d.0.curves_0,
                log_exposure: film_log_exp,
                gamma_factor: d.6,
            }
        });
        // Glare in the resident chain — applied after scan_spectral on the
        // final RGB buffer. Mirrors the CPU lognormal + blur + add. Python
        // 0.3.4 disables viewing glare entirely on the `io.scan_film` path
        // (`glare = None`; `film_render.glare` is never read upstream) —
        // only `print_render.glare` reaches the print scan.
        let glare = (!self.params.io.scan_film)
            .then_some(&self.params.print_render.glare)
            .filter(|g| g.active && g.percent > 0.0)
            .map(|g| {
                // LogNormal parameters shared with `compute_random_glare_amount`.
                let (mu, sigma) =
                    spektrafilm_model::glare::lognormal_params(g.percent, g.roughness);
                // glare_rgb_offset = (XYZ→RGB) · illuminant_xyz / 100.
                let glare_rgb_offset = crate::chain_prep::glare_rgb_offset_f64(scan_context);
                let offset_rgb = [
                    (glare_rgb_offset[0] / 100.0) as f32,
                    (glare_rgb_offset[1] / 100.0) as f32,
                    (glare_rgb_offset[2] / 100.0) as f32,
                ];
                spektrafilm_backend::GlareGpuParams {
                    mu: mu as f32,
                    sigma: sigma as f32,
                    blur_px: g.blur,
                    base_seed: self.params.random_seed.wrapping_add(42) as u32,
                    rgb_offset: offset_rgb,
                }
            });

        // Unsharp mask: scanner.unsharp_mask = [sigma, amount].
        let [usm_sigma, usm_amount] = self.params.scanner.unsharp_mask;
        let unsharp = if usm_sigma > 0.0 && usm_amount > 0.0 {
            Some(spektrafilm_backend::UnsharpGpuParams {
                sigma_px: usm_sigma as f32,
                amount: usm_amount as f32,
            })
        } else {
            None
        };

        let camera_lens_blur_px = if self.params.camera.lens_blur_um > 0.0 {
            Some(self.params.camera.lens_blur_um / pix_um)
        } else {
            None
        };

        // The density reading and Digital Reference replace the scan
        // acquisition itself; both run inside the assembled acquisition
        // fence below. The resident route never supplies a captured image,
        // so the density reading is exactly `n2p && Density` (the
        // `captured_rgb.is_none()` conjunct in `stages::scanning` is always
        // true here).
        let n2p_requested = self.params.scanner.scan_output == "negative_to_positive";
        let reference_mode = self.params.scanner.negative_conversion_method
            == crate::params::NegativeConversionMethod::DigitalReference;
        let density_reading = n2p_requested
            && self.params.scanner.negative_conversion_method
                == crate::params::NegativeConversionMethod::Density;

        // Both acquisition replacements blur the transmitted light inside
        // the acquisition (density domain and optical capture domain), so
        // the post-acquisition scanner lens blur must not run again.
        let scanner_lens_blur_px =
            if self.params.scanner.lens_blur > 0.0 && !reference_mode && !density_reading {
                Some(self.params.scanner.lens_blur)
            } else {
                None
            };

        let hboost = &self.params.film_render.halation;
        let highlight_boost = if hboost.boost_ev != 0.0 {
            Some(spektrafilm_backend::HighlightBoostGpuParams {
                boost_ev: hboost.boost_ev,
                boost_range: hboost.boost_range,
                protect_ev: hboost.protect_ev,
            })
        } else {
            None
        };
        let grain_v2 = if self.params.film_render.grain.active
            && matches!(
                self.params.film_render.grain.engine,
                crate::params::grain::GrainEngine::Procedural
            ) {
            Some(self.params.film_render.grain.gpu_params(
                self.params.random_seed,
                self.params.debug.deactivate_spatial_effects,
            ))
        } else {
            None
        };
        // Film-grain Dye resolves its field on the device whenever the
        // request is device-representable: the pass runs on the film density
        // buffer in the density-fence position and no host arithmetic
        // touches the field. A request the device cannot realize (the
        // experimental mass-law selector) keeps the host field as the
        // assembled density fence below, so an explicit GPU request fails by
        // name at assembly instead of silently running host arithmetic.
        let grain_dye = if crate::stages::grain_v3::active(&self.params) {
            match crate::stages::grain_v3::resident_device_params(
                &self.film,
                &self.params,
                image,
                pixel_size_um,
            ) {
                Ok(params) => Some(params),
                Err(reason) => {
                    tracing::info!(
                        target: "spektrafilm_core::pipeline",
                        %reason,
                        "film grain Dye: the field falls back to the assembled host fence"
                    );
                    None
                }
            }
        } else {
            None
        };
        // Host grain runs as an assembled fence on the film density buffer:
        // the resident chain keeps device residency for every other stage,
        // transferring 2 + 2 × fence count times total. Spektra resolves its
        // field from the normalized density; Dye reads the same buffer with
        // the identity readout geometry only when its field has no device
        // path (`stages::grain_v3::resident_device_params`).
        let density_cpu_stage = self.density_cpu_stage(
            film_curves_norm,
            backend,
            pixel_size_um,
            grain_dye.is_some(),
        );

        // Scanner readout interpretation (slide points, faithful B&W/slide
        // luminance remap, negative-to-positive conversion, LabScan finish)
        // runs on the host as an assembled fence on the acquisition buffer
        // — after scanner lens blur and glare, before output gamut (the CPU
        // scanning order; pipe architecture, capability class). The capture
        // methods (separable/coupled) interpret a captured image; the
        // density reading and Digital Reference replace the acquisition
        // itself in the acquisition fence below.
        let scan_output_space =
            spektrafilm_math::colorspace::resolve(&self.params.io.output_color_space)
                .expect("validated output colour space");
        let scan_channel_density: &[[f64; 3]] = if self.params.io.scan_film {
            &film_channel_density
        } else {
            &print_channel_density
        };
        let scan_base_density: &[f64] = if self.params.io.scan_film {
            film_base_density
        } else {
            print_base_density
        };
        // The per-stage route rebuilds the luminance remap from the
        // receiver's own dye captures before any scan work
        // (`stages::scanning`), so both scanner fences must see that
        // reference rather than the plain profile one. The density reading
        // and Digital Reference skip the rebuild: they replace the
        // acquisition and carry their own calibration.
        let receiver_reference = if density_reading || reference_mode {
            None
        } else {
            scan_context.receiver.as_ref().map(|receiver| {
                receiver.correction(
                    scan_profile,
                    &self.params,
                    color_ref,
                    scan_channel_density,
                    scan_base_density,
                )
            })
        };
        let color_ref = receiver_reference.as_ref().unwrap_or(color_ref);
        // Per-frame scanner setup (`conversion_brightness_ev`/`balance_ev`/
        // `contrast`) is part of the scan parameters the per-stage route
        // hands to the scan stage, so the scanner fences run the same ones.
        let scan_params_cow = self.scan_params();
        let scan_params: &crate::params::RuntimeParams = scan_params_cow.as_ref();
        // Only the capture methods (separable/coupled) interpret the
        // acquired image; the density reading and Digital Reference replace
        // the acquisition in `scan_acquisition_cpu_stage`.
        let n2p_capture_method = n2p_requested
            && matches!(
                self.params.scanner.negative_conversion_method,
                crate::params::NegativeConversionMethod::Separable
                    | crate::params::NegativeConversionMethod::Coupled
            );
        // A fenced CPU stage executes inside the resident chain, so it must
        // keep the executor observations the per-stage route publishes
        // (telemetry metrics: an actual CPU image stage is identified from
        // executor observations). Fence closures pass this observed CPU
        // backend to the shared stage functions so the stage's own stage
        // markers stay live.
        // `use_scanner_lut` selects the PCHIP scanner LUT acquisition, a
        // modeled receiver selects the receiver capture, and the luminance
        // correction selects the same LUT (`stages::scanning`); all three
        // replace the GPU spectral integration on the CPU route, so the
        // resident chain runs the same CPU acquisition as an assembled
        // fence (pipe architecture, capability class).
        let scan_acquisition_cpu_stage = self.scan_acquisition_cpu_stage(
            reference_mode,
            density_reading,
            n2p_requested,
            scan_context,
            scan_profile,
            scan_params,
            scan_channel_density,
            scan_base_density,
            color_ref,
            backend,
        );

        // Camera optical diffusion runs on the host as an assembled fence on
        // the raw exposure buffer — after highlight reconstruction, before
        // the camera lens blur (the CPU filming order; pipe architecture,
        // capability class). The faithful discrete-PSF convolution matches
        // the CPU reference exactly.
        let camera_diffusion_cpu_stage = self.camera_diffusion_cpu_stage(backend, pixel_size_um);
        // The PCHIP enlarger LUT and the enlarger diffusion filter run on
        // the host as an assembled fence that replaces the fused print
        // spectral pass (pipe architecture, capability class): the shared
        // `expose_calibrated` already owns the LUT branch, the linear-domain
        // diffusion, and the outer log, so the fence emits the same domain
        // the fused shader produces and the GPU print density pass continues.
        let edf = &self.params.enlarger.diffusion_filter;
        let print_fence_requested =
            self.params.settings.use_enlarger_lut || (edf.active && edf.strength > 0.0);
        let print_acquisition_cpu_stage = self.print_acquisition_cpu_stage(
            print_fence_requested,
            color_ref,
            backend,
            pixel_size_um,
        );
        // Slide points are read after the acquisition, so a positive scan
        // with non-default points needs the readout fence even when the
        // luminance remap itself is off.
        let slide_points_active = self.params.io.scan_film
            && self.film.is_positive()
            && (self.params.scanner.black_point != [0.0; 3]
                || self.params.scanner.white_point != [1.0; 3]);
        let scan_tail_cpu_stage = self.scan_tail_cpu_stage(
            n2p_capture_method,
            reference_mode,
            density_reading,
            slide_points_active,
            n2p_requested,
            scan_context,
            scan_profile,
            scan_params,
            scan_channel_density,
            scan_base_density,
            scan_output_space,
            color_ref,
            backend,
        );
        // Output transfer (pipe architecture, S4): encode the native
        // destination when the caller requested encoding and the
        // Procedural-grain pass hasn't already emitted encoded samples, or
        // restore linear when it has and no encoding was requested. This
        // moved the former post-readback host encode/decode onto the device.
        let grain_v2_active = self.grain_emits_encoded();
        let cctf_kind = match scan_output_space.cctf {
            spektrafilm_math::colorspace::Cctf::Linear => 0,
            spektrafilm_math::colorspace::Cctf::Acescct => 1,
            spektrafilm_math::colorspace::Cctf::Srgb => 2,
            spektrafilm_math::colorspace::Cctf::ProPhoto => 3,
            spektrafilm_math::colorspace::Cctf::Rec2020 => 4,
            spektrafilm_math::colorspace::Cctf::AdobeRgb1998 => 5,
            spektrafilm_math::colorspace::Cctf::Gamma2_6 => 6,
        };
        let identity_f32: [[f32; 3]; 3] = std::array::from_fn(|i| {
            std::array::from_fn(|j| scan_output_space.rgb_to_rgb_identity()[i][j] as f32)
        });
        let output_transfer = if !grain_v2_active && self.params.io.output_cctf_encoding {
            Some(spektrafilm_backend::OutputTransferGpuParams {
                decode: false,
                cctf_kind,
                matrix: identity_f32,
            })
        } else if grain_v2_active && !self.params.io.output_cctf_encoding {
            Some(spektrafilm_backend::OutputTransferGpuParams {
                decode: true,
                cctf_kind,
                matrix: identity_f32,
            })
        } else {
            None
        };
        // Magazine print appearance (pipe architecture, S4): the last host
        // finishing moves onto the device right after the transfer tail.
        // The buffer state there is encoded exactly when the caller
        // requested output encoding.
        let magazine = if self.params.magazine_print_color.active
            && self.params.magazine_print_color.strength > 0.0
        {
            let (to_lab, from_lab) =
                crate::magazine_print_color::MagazineTransform::new(scan_output_space)
                    .gpu_matrices();
            let (tone_in, tone_out, tone_slopes) = crate::magazine_print_color::gpu_tone_curve();
            let f32m = |m: &[[f64; 3]; 3]| {
                std::array::from_fn(|i| std::array::from_fn(|j| m[i][j] as f32))
            };
            let f32a = |a: &[f64; 10]| std::array::from_fn(|i| a[i] as f32);
            Some(spektrafilm_backend::MagazineGpuParams {
                strength: self.params.magazine_print_color.strength.clamp(0.0, 1.0) as f32,
                encoded: self.params.io.output_cctf_encoding,
                cctf_kind,
                to_lab: f32m(&to_lab),
                from_lab: f32m(&from_lab),
                tone_in: f32a(&tone_in),
                tone_out: f32a(&tone_out),
                tone_slopes: f32a(&tone_slopes),
            })
        } else {
            None
        };

        let params = spektrafilm_backend::FilmChainParams {
            image,
            front,
            film_log_exposure: film_log_exp,
            film_density_curves_normalized: film_curves_norm,
            film_gamma: self.params.film_render.density_curve_gamma as f64,
            film_channel_density: &film_channel_density,
            film_base_density,
            print_illuminant: &self.print_illuminant,
            print_sensitivity: &print_sens,
            print_normalization_factor: print_norm_factor,
            print_exposure_scale,
            print_log_exposure: print_curves.log_exposure,
            print_density_curves: &print_curves.density,
            print_gamma: print_curves.gamma,
            print_channel_density: &print_channel_density,
            print_base_density,
            preflash: self.preflash_raw,
            viewing_illuminant: viewing_illu,
            scan_normalization: scan_norm,
            scan_xyz_to_rgb: &scan_xyz_to_rgb,
            scan_output_space,
            white_black_xyz_remap: if scan_tail_cpu_stage.is_some() {
                None
            } else {
                color_ref.xyz_remap()
            },
            scan_film: self.params.io.scan_film,
            halation,
            dir_couplers,
            glare,
            gamut: self.output_gamut.gpu_params(),
            unsharp,
            grain_v2,
            camera_lens_blur_px,
            scanner_lens_blur_px,
            highlight_boost,
            grain_dye,
            density_cpu_stage,
            scan_tail_cpu_stage,
            scan_acquisition_cpu_stage,
            print_acquisition_cpu_stage,
            camera_diffusion_cpu_stage,
            output_transfer,
            magazine,
        };
        if !params.gpu_blurs_supported() {
            if let Some(context) = backend.observation_context() {
                context.decline_resident(
                    spektrafilm_backend::telemetry::ResidentDeclineReason::BlurRadiusExceedsBackendSupport,
                );
            }
            tracing::info!(
                target: "spektrafilm_core::pipeline",
                backend = backend.name(),
                execution = "per_stage_cpu",
                fallback_reasons = ?[ResidentFallbackReason::BlurRadiusExceedsBackendSupport],
                "using per-stage path because a resident FIR blur exceeds backend support"
            );
            if backend.explicit_gpu_request() {
                return Err(
                    "explicit GPU request exceeds the backend FIR blur support; retry with an explicit CPU request"
                        .into(),
                );
            }
            return Ok(None);
        }
        // Pipe architecture fence policy: a fence is a transfer optimization
        // for automatic selection, so an explicit GPU request must fail at
        // assembly when any capability-missing stage would execute on the
        // host (spec gpu-execution §Backend selection, pipe architecture
        // "Policy"). The pre-fence decision rejected exactly these stages;
        // the fences must not let an explicit request run them silently.
        if backend.explicit_gpu_request() {
            let mut host_stages: Vec<&str> = Vec::new();
            if self.params.io.input_cctf_decoding {
                host_stages.push("input transfer decoding");
            }
            if params.camera_diffusion_cpu_stage.is_some() {
                host_stages.push("camera optical diffusion");
            }
            if params.density_cpu_stage.is_some() {
                host_stages.push("film grain distribution");
            }
            if params.scan_acquisition_cpu_stage.is_some() {
                host_stages.push("scanner acquisition");
            }
            if params.scan_tail_cpu_stage.is_some() {
                host_stages.push("scanner readout interpretation");
            }
            if params.print_acquisition_cpu_stage.is_some() {
                host_stages.push("print acquisition");
            }
            if !host_stages.is_empty() {
                return Err(format!(
                    "explicit GPU request cannot run the complete computation scope on WGPU \
                     (host-executed stages: {}); retry with an explicit CPU request to use the \
                     per-stage route",
                    host_stages.join(", ")
                ));
            }
        }
        let resident = crate::stages::StageObservation::new(backend, "film_chain");
        let result = match resident.backend(backend).try_run_film_chain(&params) {
            Ok(result) => result,
            Err(error) => {
                // A CPU fence could not calibrate its output, or the chain
                // refused to allocate inside the device limits. The per-stage
                // route reports the same fence failure to the caller, so
                // surface it instead of falling back (spec: fail with an
                // error before output publication).
                tracing::error!(
                    target: "spektrafilm_core::pipeline",
                    %error,
                    "resident chain failed"
                );
                return Err(error);
            }
        };
        if let Some(context) = backend.observation_context() {
            if result.is_some() {
                context.set_path(spektrafilm_backend::telemetry::ExecutionPath::GpuResident);
            } else {
                context.decline_resident(
                    spektrafilm_backend::telemetry::ResidentDeclineReason::BackendNoResidentSupport,
                );
            }
        }
        Ok(result)
    }

    /// Whether the Procedural-grain pass emits encoded samples on the
    /// resident chain (it encodes before grain; the pipe-tail transfer
    /// decodes afterwards when the caller requested linear output).
    fn grain_emits_encoded(&self) -> bool {
        self.params.film_render.grain.active
            && self.params.settings.rgb_to_raw_method != "mallett2019"
            && matches!(
                self.params.film_render.grain.engine,
                crate::params::grain::GrainEngine::Procedural
            )
    }
}
