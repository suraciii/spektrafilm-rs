use super::super::Pipeline;
use super::decision::{ResidentDecision, ResidentFallbackReason};

use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::image::ImageBuf;
impl Pipeline {
    pub(in crate::pipeline) fn try_gpu_resident(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
        pixel_size_um: f64,
        ae_ev: f64,
    ) -> Result<Option<ImageBuf>, String> {
        // The upload below is a device operation. A host backend has no
        // retained carrier and must take the per-stage route, exactly as the
        // retained entry does — the chain's own guard below decides for a
        // device, so the host case cannot fail here.
        if !backend.is_gpu() {
            return Ok(None);
        }
        // Compatibility route: the working image is uploaded once, the chain
        // runs on the device, and the result is materialized at this boundary.
        // A device that cannot hold the working image has no resident route;
        // only an explicit GPU request turns that into a failure, and it names
        // the limit so the caller can act on it.
        let input = match self.retain_working_image(image, backend) {
            Ok(input) => input,
            Err(reason) => {
                if backend.explicit_gpu_request() {
                    return Err(match backend.image_binding_limit_reason(image) {
                        Some(limit) => {
                            format!(
                                "explicit GPU request exceeds the device resource limits: {limit}"
                            )
                        }
                        None => reason,
                    });
                }
                return Ok(None);
            }
        };
        match self.try_gpu_resident_retained(&input, backend, pixel_size_um, ae_ev)? {
            Some(retained) => Ok(Some(retained.to_host(backend)?)),
            None => Ok(None),
        }
    }

    /// Upload one working image as the chain's retained input.
    pub(in crate::pipeline) fn retain_working_image(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        let space = spektrafilm_math::colorspace::resolve(&self.params.io.input_color_space)
            .expect("input color space must be validated before filming");
        backend.retain_image(
            image,
            &spektrafilm_backend::RetainedImageMeta {
                width: image.width,
                height: image.height,
                // The upload keeps the samples' stored state; the chain's
                // first device pass decodes the input transfer when the
                // request asks for it.
                domain: if self.params.io.input_cctf_decoding {
                    spektrafilm_backend::SampleDomain::Encoded
                } else {
                    spektrafilm_backend::SampleDomain::Linear
                },
                precision: spektrafilm_backend::Precision::host(),
                space,
                pixel_pitch_um: None,
            },
        )
    }

    /// The production resident chain: consumes a retained input carrier and
    /// returns a retained output carrier, with no host materialization between
    /// the stages.
    pub(in crate::pipeline) fn try_gpu_resident_retained(
        &self,
        input: &spektrafilm_backend::DeviceImage,
        backend: &dyn ComputeBackend,
        pixel_size_um: f64,
        ae_ev: f64,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        self.try_gpu_resident_retained_with_taps(
            input,
            backend,
            pixel_size_um,
            ae_ev,
            spektrafilm_backend::Tap::RgbPre,
            spektrafilm_backend::Tap::RgbOut,
        )
    }

    /// Execute exactly the selected topology span over an existing carrier.
    pub(in crate::pipeline) fn try_gpu_resident_retained_with_taps(
        &self,
        input: &spektrafilm_backend::DeviceImage,
        backend: &dyn ComputeBackend,
        pixel_size_um: f64,
        ae_ev: f64,
        inject: spektrafilm_backend::Tap,
        collect: spektrafilm_backend::Tap,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        use spektrafilm_backend::Tap;
        let order = self.tap_order();
        let ip = order.iter().position(|tap| *tap == inject).ok_or_else(|| {
            format!(
                "tap {inject:?} is not part of this topology (scan_film = {})",
                self.params.io.scan_film
            )
        })?;
        let cp = order
            .iter()
            .position(|tap| *tap == collect)
            .ok_or_else(|| {
                format!(
                    "tap {collect:?} is not part of this topology (scan_film = {})",
                    self.params.io.scan_film
                )
            })?;
        if cp < ip {
            return Err(format!(
                "no node path reaches tap {collect:?} from {inject:?}"
            ));
        }
        if inject == collect {
            return Ok(Some(input.clone()));
        }
        let includes = |tap| {
            order
                .iter()
                .position(|candidate| *candidate == tap)
                .is_some_and(|position| ip <= position && position < cp)
        };
        let expose = includes(Tap::RgbPre);
        let develop = includes(Tap::LogEFilm);
        let scan = collect == Tap::RgbOut;
        if !backend.is_gpu() {
            return Ok(None);
        }
        let decision = match self.resident_decision() {
            ResidentDecision::PerStage { mut reasons } => {
                reasons.retain(|reason| match reason {
                    ResidentFallbackReason::FilmGrainV3FieldCpu => develop,
                    ResidentFallbackReason::MissingResidentFrontPass => expose,
                    ResidentFallbackReason::UnsupportedOutputGamut => scan,
                    _ => true,
                });
                if reasons.is_empty() {
                    ResidentDecision::UseResident
                } else {
                    ResidentDecision::PerStage { reasons }
                }
            }
            decision => decision,
        };
        match decision {
            ResidentDecision::UseResident => {}
            ResidentDecision::PerStage { reasons } => {
                if let Some(context) = backend.observation_context() {
                    for reason in &reasons {
                        use spektrafilm_backend::telemetry::ResidentDeclineReason as R;
                        let reason = match reason {
                            ResidentFallbackReason::WorkflowRoute => R::WorkflowRoute,

                            ResidentFallbackReason::FilmGrainV3FieldCpu => R::GrainV3FieldCpu,
                            ResidentFallbackReason::UnsupportedOutputGamut => {
                                R::UnsupportedOutputGamut
                            }
                            ResidentFallbackReason::BlurRadiusExceedsBackendSupport => {
                                R::BlurRadiusExceedsBackendSupport
                            }
                            ResidentFallbackReason::MissingResidentFrontPass => {
                                R::MissingResidentFrontPass
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
        // Input transfer decoding is the chain's first device pass (pipe
        // architecture: input preparation). The front pass assumes linear RGB
        // exactly like `expose` produces for the per-stage route, so the
        // uploaded samples are decoded on the device instead of on the host.
        // Same expression as `expose`; it runs only on the resident arm so a
        // declined route never decodes twice (the per-stage `expose` performs
        // its own decode).
        let convert_route = self.params.workflow.route_enum().is_convert_film();
        let scan_film = if convert_route {
            self.params.workflow.route_enum() != crate::WorkflowRoute::ConvertFilmPrintScan
        } else {
            self.params.io.scan_film
        };
        let input_decode =
            (expose && self.params.io.input_cctf_decoding && !convert_route).then(|| {
                let input_space =
                    spektrafilm_math::colorspace::resolve(&self.params.io.input_color_space)
                        .expect("input color space must be validated before filming");
                spektrafilm_backend::InputDecodeGpuParams {
                    cctf_kind: spektrafilm_backend::cctf_kind(input_space.cctf),
                }
            });
        // Keep the inverse solver record on the stack for the entire backend
        // submission; the front pass borrows it without an allocation.
        let convert_params = if expose && convert_route {
            self.convert_converter
                .as_ref()
                .map(|converter| converter.gpu_params())
        } else {
            None
        };
        // A suffix injection has no front transform. The inactive matrix is
        // only a packed setup record; the typed span omits its dispatch.
        let front = if expose {
            self.resident_front_pass(ae_ev, convert_params.as_ref())
        } else {
            Some(spektrafilm_backend::FrontPass::Mallett2019 {
                matrix: [[0.0; 3]; 3],
            })
        };
        let Some(front) = front else {
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
        let scan_profile = if scan_film { &self.film } else { &self.print };
        let include_base =
            self.params.workflow.route_enum() != crate::WorkflowRoute::ConvertFilmScanMinusBase;
        let prepared =
            crate::chain_prep::PreparedChain::for_scan(scan_profile, &self.params, include_base);
        let scan_context = &prepared.scan;
        let scan_illu: &[f64] = &scan_context.illuminant;
        let scan_norm = scan_context.normalization;
        let scan_xyz_to_rgb = prepared.scan_xyz_to_rgb;

        // GPU shaders take f32 — narrow the working-geometry pitch once.
        let pix_um = pixel_size_um as f32;

        // Keep the two print exposure halves separate: preflash is added
        // before the inner floor/log, while print exposure multiplies the
        // recovered linear raw before the outer floor/log.
        let print_exposure_scale = self.params.enlarger.print_exposure as f64;
        let print_norm_factor = self.print_exposure_factor;

        // Halation in the resident chain — only built when the halation
        // stage is active. Preserve channel-specific profile controls.
        let halation = if expose && self.params.film_render.halation.active {
            let h = &self.params.film_render.halation;
            Some(spektrafilm_backend::HalationGpuParams {
                scatter_amount: h.scatter_amount as f32,
                scatter_core_px: std::array::from_fn(|c| {
                    (h.scatter_core_um[c] * h.scatter_spatial_scale / pix_um as f64) as f32
                }),
                scatter_tail_px: std::array::from_fn(|c| {
                    (h.scatter_tail_um[c] * h.scatter_spatial_scale / pix_um as f64) as f32
                }),
                scatter_tail_weight: [
                    h.scatter_tail_weight[0] as f32,
                    h.scatter_tail_weight[1] as f32,
                    h.scatter_tail_weight[2] as f32,
                ],
                source_compression: h.source_compression as f32,
                halation_a_tot: std::array::from_fn(|c| {
                    (h.halation_strength[c] * h.halation_amount) as f32
                }),
                halation_first_sigma_px: std::array::from_fn(|c| {
                    (h.halation_first_sigma_um[c] * h.halation_spatial_scale / pix_um as f64) as f32
                }),
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
        let dir_inputs = if develop && dir.active {
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
        let glare = (scan && !scan_film)
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
                    base_seed: self.params.random_seed as u32,
                    rgb_offset: offset_rgb,
                }
            });

        // Unsharp mask: scanner.unsharp_mask = [sigma, amount].
        let [usm_sigma, usm_amount] = self.params.scanner.unsharp_mask;
        let unsharp = if scan && usm_sigma > 0.0 && usm_amount > 0.0 {
            Some(spektrafilm_backend::UnsharpGpuParams {
                sigma_px: usm_sigma as f32,
                amount: usm_amount as f32,
            })
        } else {
            None
        };

        let camera_lens_blur_px = if expose && self.params.camera.lens_blur_um > 0.0 {
            Some(self.params.camera.lens_blur_um / pix_um)
        } else {
            None
        };

        // The density reading replaces the scan acquisition itself; it runs
        // inside the assembled acquisition fence below. The resident route
        // never supplies a captured image, so the density reading is exactly
        // `n2p` (the `captured_rgb.is_none()` conjunct in `stages::scanning`
        // is always true here).
        let density_reading = scan && self.params.scanner.scan_output == "negative_to_positive";
        let scan_readout = density_reading
            .then(|| {
                crate::stages::scanning::density_reading_gpu_params(
                    scan_profile,
                    &self.conversion_tones,
                    &self.params,
                )
            })
            .transpose()?;

        // The acquisition replacement blurs the transmitted light inside the
        // acquisition (density domain), so the post-acquisition scanner lens
        // blur must not run again.
        let scanner_lens_blur_px =
            if scan && self.params.scanner.lens_blur > 0.0 && !density_reading {
                Some(self.params.scanner.lens_blur)
            } else {
                None
            };

        let hboost = &self.params.film_render.halation;
        let highlight_boost = if expose && hboost.boost_ev != 0.0 {
            Some(spektrafilm_backend::HighlightBoostGpuParams {
                boost_ev: hboost.boost_ev,
                boost_range: hboost.boost_range,
                protect_ev: hboost.protect_ev,
            })
        } else {
            None
        };
        let grain_v2 = if scan
            && self.params.film_render.grain.active
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
        // Recovered convert-film CMY is already developed; neither optical
        // engine may generate a new field or apply its readout geometry there.
        let dye_readout =
            if develop && !convert_route && crate::stages::grain_v3::active(&self.params) {
                crate::stages::grain_v3::validate_supported(&self.film, &self.params)?;
                let spatial_context = (scan
                    && crate::stages::grain_v3::scanner_spatial_context(&self.params))
                    || (collect != Tap::CmyFilm
                        && crate::stages::grain_v3::print_spatial_context(&self.params));
                Some(crate::stages::grain_v3::readout_plan(
                    input.width(),
                    input.height(),
                    &self.params,
                    pixel_size_um,
                    spatial_context,
                )?)
            } else {
                None
            };
        let grain_dye = dye_readout
            .as_ref()
            .map(|plan| {
                crate::stages::grain_v3::resident_device_params_for_grid(
                    &self.film,
                    &self.params,
                    pixel_size_um,
                    plan.grid,
                )
            })
            .transpose()?;
        let fotufilm_readout =
            if develop && !convert_route && crate::stages::grain_fotufilm::active(&self.params) {
                let spatial_context = (scan
                    && crate::stages::grain_v3::scanner_spatial_context(&self.params))
                    || (collect != Tap::CmyFilm
                        && crate::stages::grain_v3::print_spatial_context(&self.params));
                Some(crate::stages::grain_v3::readout_plan(
                    input.width(),
                    input.height(),
                    &self.params,
                    pixel_size_um,
                    spatial_context,
                )?)
            } else {
                None
            };
        let grain_fotufilm = fotufilm_readout
            .as_ref()
            .map(|plan| {
                self.fotufilm_prepared.resident_device_params(
                    input.width(),
                    input.height(),
                    &self.film,
                    &self.params,
                    pixel_size_um,
                    plan.grid,
                    &|| self.check_cancelled(),
                )
            })
            .transpose()?;
        // Spektra realizes either upstream sampling regime on the device.
        // Preparation errors propagate instead of becoming a host fallback.
        let grain_v1 = if develop
            && !convert_route
            && self.params.film_render.grain.active
            && matches!(
                self.params.film_render.grain.engine,
                crate::params::grain::GrainEngine::Spektra
            ) {
            Some(crate::stages::filming::spektra_grain_device_params(
                &self.film,
                &self.params.film_render.grain,
                film_curves_norm,
                self.params.settings.use_fast_stats,
                self.params.random_seed,
                pixel_size_um,
            )?)
        } else {
            None
        };
        // Host grain runs as an assembled fence on the film density buffer:
        // the resident chain keeps device residency for every other stage,
        // transferring 2 + 2 × fence count times total. Spektra resolves its
        // field from the normalized density; Dye reads the same buffer with
        // the identity readout geometry only when its field has no device
        // path (`stages::grain_v3::resident_device_params`).
        let density_cpu_stage = if !develop || convert_route {
            None
        } else {
            self.density_cpu_stage(
                film_curves_norm,
                backend,
                pixel_size_um,
                grain_dye.is_some(),
                grain_v1.is_some(),
            )
        };

        let scan_output_space =
            spektrafilm_math::colorspace::resolve(&self.params.io.output_color_space)
                .expect("validated output colour space");
        let scan_channel_density: &[[f64; 3]] = if scan_film {
            &film_channel_density
        } else {
            &print_channel_density
        };
        let scan_base_zero = vec![0.0; scan_profile.data.base_density.len()];
        let scan_base_density: &[f64] = if !include_base {
            &scan_base_zero
        } else if scan_film {
            film_base_density
        } else {
            print_base_density
        };
        let scanner_lut =
            (scan && !density_reading && self.params.settings.use_scanner_lut).then(|| {
                let (minimum, maximum) =
                    crate::stages::scanning::scanner_lut_bounds(scan_profile, &self.params);
                spektrafilm_backend::ScannerSpectralLutGpuParams {
                    channel_density: scan_channel_density,
                    base_density: scan_base_density,
                    illuminant: scan_illu,
                    normalization: scan_norm,
                    cat: &scan_context.adapt,
                    xyz_to_rgb: &scan_context.base_xyz_to_rgb,
                    minimum,
                    maximum,
                    size: self.params.settings.lut_resolution as u32,
                }
            });
        let enlarger_lut = (includes(Tap::CmyFilm)
            && !scan_film
            && self.params.settings.use_enlarger_lut)
            .then(|| {
                let (minimum, maximum) =
                    crate::stages::printing::enlarger_lut_bounds(&self.film, &self.params);
                spektrafilm_backend::EnlargerSpectralLutGpuParams {
                    channel_density: &film_channel_density,
                    base_density: film_base_density,
                    illuminant: &self.print_illuminant,
                    sensitivity: &print_sens,
                    normalization: print_norm_factor,
                    preflash: self.preflash_raw,
                    minimum,
                    maximum,
                    size: self.params.settings.lut_resolution as u32,
                }
            });
        // Scanner LUT acquisition is a device operation, not a host fence.
        let scan_acquisition_cpu_stage = if !scan || density_reading || scanner_lut.is_some() {
            None
        } else {
            self.scan_acquisition_cpu_stage(
                false,
                scan_context,
                scan_profile,
                &self.params,
                scan_channel_density,
                scan_base_density,
                backend,
            )
        };

        // Camera optical diffusion and the enlarger print acquisition run on
        // the device: the chain's fused passes convolve the same discrete PSF
        // and evaluate the same PCHIP lattice, so neither is assembled as a
        // host fence.
        // Output transfer (pipe architecture, S4): encode the native
        // destination when the caller requested encoding and the
        // Procedural-grain pass hasn't already emitted encoded samples, or
        // restore linear when it has and no encoding was requested. This
        // moved the former post-readback host encode/decode onto the device.
        let grain_v2_active = scan && self.grain_emits_encoded();
        let cctf_kind = spektrafilm_backend::cctf_kind(scan_output_space.cctf);
        let identity_f32: [[f32; 3]; 3] = std::array::from_fn(|i| {
            std::array::from_fn(|j| scan_output_space.rgb_to_rgb_identity()[i][j] as f32)
        });
        let output_transfer = if scan && !grain_v2_active && self.params.io.output_cctf_encoding {
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
        // CMYK runs after the encoded grain domain when Procedural grain is
        // active, even for a linear output; the final decode is appended
        // after CMYK to match the CPU scanner order exactly.
        let cmyk_encoded = grain_v2_active || self.params.io.output_cctf_encoding;
        let cmyk = if scan && self.params.cmyk.active && self.params.cmyk.strength > 0.0 {
            let (to_lab, from_lab) =
                crate::cmyk::CmykTransform::new(scan_output_space).gpu_matrices();
            let f32m = |m: &[[f64; 3]; 3]| {
                std::array::from_fn(|i| std::array::from_fn(|j| m[i][j] as f32))
            };
            Some(spektrafilm_backend::CmykGpuParams {
                strength: self.params.cmyk.strength.clamp(0.0, 1.0) as f32,
                encoded: cmyk_encoded,
                cctf_kind,
                to_lab: f32m(&to_lab),
                from_lab: f32m(&from_lab),
            })
        } else {
            None
        };

        let prepare_diffusion = |filter: &crate::params::diffusion::DiffusionFilterParams,
                                 enabled: bool,
                                 width: u32,
                                 height: u32,
                                 pitch: f64|
         -> Result<_, String> {
            if !enabled || !filter.active {
                return Ok(None);
            }
            spektrafilm_model::diffusion::validate_family(&filter.filter_family)?;
            Ok(spektrafilm_model::diffusion::resolve_psf(
                &filter.to_model(),
                pitch,
                width as usize,
                height as usize,
            )
            .ok())
        };
        let camera_diffusion = prepare_diffusion(
            &self.params.camera.diffusion_filter,
            expose,
            input.width(),
            input.height(),
            pixel_size_um,
        )?;
        let print_grid = dye_readout
            .as_ref()
            .or(fotufilm_readout.as_ref())
            .map(|plan| plan.grid);
        let (print_width, print_height, print_pitch) =
            print_grid.map_or((input.width(), input.height(), pixel_size_um), |grid| {
                (
                    grid.width,
                    grid.height,
                    grid.pitch_um[usize::from(input.height() > input.width())],
                )
            });
        let enlarger_diffusion = prepare_diffusion(
            &self.params.enlarger.diffusion_filter,
            includes(Tap::CmyFilm) && !scan_film,
            print_width,
            print_height,
            print_pitch,
        )?;

        let params = spektrafilm_backend::FilmChainParams {
            scan_preparation_key: if density_reading
                && inject == Tap::CmyFilm
                && collect == Tap::RgbOut
            {
                input.payload()
            } else {
                None
            },
            inject,
            collect,
            image_width: input.width(),
            image_height: input.height(),
            pixel_pitch_um: pixel_size_um,
            front,
            film_log_exposure: film_log_exp,
            film_density_curves_normalized: film_curves_norm,
            film_gamma: self.params.film_render.density_curve_gamma as f64,
            film_channel_density: &film_channel_density,
            film_base_density: if include_base {
                film_base_density
            } else {
                &scan_base_zero
            },
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
            enlarger_lut,
            scanner_lut,
            scan_illuminant: scan_illu,
            scan_normalization: scan_norm,
            scan_xyz_to_rgb: &scan_xyz_to_rgb,
            scan_cat: &scan_context.adapt,
            scan_base_xyz_to_rgb: &scan_context.base_xyz_to_rgb,
            scan_output_space,
            scan_film,
            halation,
            dir_couplers,
            glare,
            gamut: if scan {
                self.output_gamut.gpu_params()
            } else {
                None
            },
            unsharp,
            grain_v2,
            camera_lens_blur_px,
            camera_diffusion: camera_diffusion.as_ref().map(|resolved| {
                spektrafilm_backend::OpticalDiffusionGpuParams {
                    psf: &resolved.0,
                    radius: resolved.1 as u32,
                    scatter_fraction: resolved.2,
                }
            }),
            enlarger_diffusion: enlarger_diffusion.as_ref().map(|resolved| {
                spektrafilm_backend::OpticalDiffusionGpuParams {
                    psf: &resolved.0,
                    radius: resolved.1 as u32,
                    scatter_fraction: resolved.2,
                }
            }),
            scanner_lens_blur_px,
            highlight_boost,
            grain_dye,
            grain_dye_crop: dye_readout
                .as_ref()
                .or(fotufilm_readout.as_ref())
                .and_then(|plan| plan.deferred_crop)
                .map(|r| [r.x as u32, r.y as u32, r.width as u32, r.height as u32]),
            grain_fotufilm,
            grain_v1,
            density_cpu_stage,
            scan_acquisition_cpu_stage,
            scan_readout: scan_readout.as_ref(),
            output_transfer,
            cmyk,
            input_decode,
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
        // A selected GPU cannot execute host image-stage fences, including
        // automatically selected adapters and diagnostic spans.
        if backend.is_gpu() {
            let mut host_stages: Vec<&str> = Vec::new();
            if params.density_cpu_stage.is_some() {
                host_stages.push("film grain distribution");
            }
            if params.scan_acquisition_cpu_stage.is_some() {
                host_stages.push("scanner acquisition");
            }
            if !host_stages.is_empty() {
                return Err(format!(
                    "selected GPU request cannot run the complete computation scope on WGPU \
                     (host-executed stages: {}); retry with an explicit CPU request to use the \
                     per-stage route",
                    host_stages.join(", ")
                ));
            }
        }
        let resident = crate::stages::StageObservation::new(backend, "film_chain");
        let result = match resident
            .backend(backend)
            .try_run_film_chain_retained(input, &params)
        {
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
            && matches!(
                self.params.film_render.grain.engine,
                crate::params::grain::GrainEngine::Procedural
            )
    }
}
