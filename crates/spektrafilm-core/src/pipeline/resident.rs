use super::{Pipeline, select_illuminant};
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;

#[derive(Debug, Clone, Copy)]
enum ResidentFallbackReason {
    WorkflowRoute,
    NegativeToPositiveOutput,
    ScannerReceiver,
    ScannerFilmPoints,
    LangmuirChemistry,
    InputTransferDecoding,
    RequestedSpectralLut,
    ActiveOpticalDiffusion,
    FaithfulGrainDistribution,
    /// Active Dye resolves its film-coordinate field on the CPU; the
    /// resident chain has no equivalent readout.
    FilmGrainV3FieldCpu,
    FilmGrainFotufilmCpu,
    UnsupportedOutputGamut,
    BlurRadiusExceedsBackendSupport,
    MissingResidentFrontPass,
    MallettExecutionParity,
}

#[derive(Debug)]
enum ResidentDecision {
    UseResident,
    PerStage {
        reasons: Vec<ResidentFallbackReason>,
    },
}

impl ResidentDecision {
    fn reasons(reasons: Vec<ResidentFallbackReason>) -> Self {
        if reasons.is_empty() {
            Self::UseResident
        } else {
            Self::PerStage { reasons }
        }
    }
}

impl Pipeline {
    fn resident_decision(&self) -> ResidentDecision {
        let mut reasons = Vec::new();
        // The Mallett shader has no numerically equivalent route through
        // CPU display-domain grain and optics; retain the per-stage path.
        if self.params.settings.rgb_to_raw_method == "mallett2019" {
            reasons.push(ResidentFallbackReason::MallettExecutionParity);
        }
        if !matches!(
            self.params.workflow.route.as_str(),
            "input > film > scan" | "input > film > print > scan"
        ) {
            reasons.push(ResidentFallbackReason::WorkflowRoute);
        }
        // Negative-to-positive interpretation no longer declines for the
        // capture methods (separable/coupled): it runs as an assembled CPU
        // fence on the scanner readout (pipe architecture, capability
        // class — see `scan_tail_cpu_stage` in `try_gpu_resident`). The
        // density reading and Digital Reference replace the acquisition
        // itself, so they still decline.
        if self.params.scanner.scan_output == "negative_to_positive"
            && matches!(
                self.params.scanner.positive_interpretation,
                crate::params::PositiveInterpretation::Density
                    | crate::params::PositiveInterpretation::DigitalReference
            )
        {
            reasons.push(ResidentFallbackReason::NegativeToPositiveOutput);
        }
        // A modeled scanner receiver (LabScanApprox) no longer declines:
        // the receiver capture replaces the GPU spectral integration as
        // the assembled acquisition fence (see `scan_acquisition_cpu_stage`
        // in `try_gpu_resident`). Digital Reference still declines through
        // NegativeToPositiveOutput above (it replaces the acquisition with
        // runtime-prepared spectral assets).
        // Slide points still decline: the positive-film resident chain has
        // a known divergence (first exposed by the scan-tail fence parity
        // probe) and the slide remap's y→0 gain amplifies the f32
        // acquisition perturbation beyond the fence budget. They move to
        // the assembled scanner readout fence once the positive-film chain
        // reaches parity.
        if self.params.io.scan_film
            && self.film.is_positive()
            && (self.params.scanner.black_point != [0.0; 3]
                || self.params.scanner.white_point != [1.0; 3])
        {
            reasons.push(ResidentFallbackReason::ScannerFilmPoints);
        }
        // Non-default Langmuir chemistry no longer declines: DIR runs as
        // an assembled CPU fence (pipe architecture, capability class —
        // see `dir_cpu_stage` in `try_gpu_resident`).
        // Input transfer decoding no longer declines: the resident route
        // decodes the input before upload (pipe architecture, capability
        // class — see `try_gpu_resident`).
        // The scanner and enlarger LUT acquisitions no longer decline: they
        // run as assembled CPU fences that replace the GPU spectral
        // integration (see `scan_acquisition_cpu_stage` and
        // `print_acquisition_cpu_stage` in `try_gpu_resident`).
        // Optical diffusion no longer declines: the enlarger filter runs in
        // the print acquisition fence and the camera filter runs as the
        // chain-head fence on the raw exposure buffer (see
        // `camera_diffusion_cpu_stage` in `try_gpu_resident`).
        // Spektra grain no longer declines: it runs on the host as an
        // assembled fence on the film density buffer (pipe architecture,
        // capability class — see `density_cpu_stage` in `try_gpu_resident`).
        // An active Dye field is resolved on the CPU from the full developed
        // film density; this decision is the capability authority for every
        // resident entry point.
        if crate::stages::grain_v3::active(&self.params) {
            reasons.push(ResidentFallbackReason::FilmGrainV3FieldCpu);
        }
        if crate::stages::grain_fotufilm::active(&self.params) {
            reasons.push(ResidentFallbackReason::FilmGrainFotufilmCpu);
        }
        if self.output_gamut.is_active() && self.output_gamut.gpu_params().is_none() {
            reasons.push(ResidentFallbackReason::UnsupportedOutputGamut);
        }
        if self.tc_lut.is_none() && self.mallett_core.is_none() {
            reasons.push(ResidentFallbackReason::MissingResidentFrontPass);
        }
        ResidentDecision::reasons(reasons)
    }

    /// Prepare the resident front pass with exposure folded into its f64 matrix.
    fn resident_front_pass(
        &self,
        color_ref: &crate::color_reference::ColorReference,
        ae_ev: f64,
    ) -> Option<spektrafilm_gpu::FrontPass<'_>> {
        // Bake the exposure scale (auto-exposure × manual EV compensation)
        // into the front-pass matrix. Both upsamplers (hanatos and mallett)
        // are homogeneous in the input RGB, so scaling the matrix is
        // equivalent to scaling the input — saves a separate "scale" compute
        // pass at the head of the chain. Auto-exposure metering itself stays
        // on CPU (~30 ms at 6 MP after the per-row rayon parallelization);
        // the result is a single float that's cheap to roll into the matrix.
        let mut exposure_scale_f64 = 1.0f64;
        if self.params.camera.auto_exposure {
            exposure_scale_f64 *= 2.0f64.powf(ae_ev);
        }
        if self.params.camera.exposure_compensation_ev != 0.0 {
            exposure_scale_f64 *= 2.0f64.powf(self.params.camera.exposure_compensation_ev as f64);
        }
        // B&W/slide filming exposure correction is a linear scale on the raw
        // film exposure (CPU: `raw *= factor` before log10). Since both
        // upsamplers are homogeneous in the input RGB, folding it into the
        // exposure scale is equivalent. 1.0 (no-op) on every path except a
        // corrected slide scan.
        exposure_scale_f64 *= color_ref.filming_exposure_correction;
        let fold_exposure = |m: &mut [[f64; 3]; 3]| {
            if (exposure_scale_f64 - 1.0).abs() > 1e-9 {
                for row in m.iter_mut() {
                    for v in row.iter_mut() {
                        *v *= exposure_scale_f64;
                    }
                }
            }
        };

        // Front pass: hanatos TC LUT lookup, or the mallett 3×3 matmul
        // (`core · M_cs`, same fold as the CPU `expose` dispatch).
        let front = if let Some(tc_lut) = self.tc_lut.as_ref() {
            let ref_illuminant = select_illuminant(&self.front_illuminant);
            let mut rgb_to_adapted = spektrafilm_math::spectral::build_rgb_to_adapted_xyz(
                &self.params.io.input_color_space,
                &ref_illuminant,
                self.params.settings.use_cat16,
            );
            fold_exposure(&mut rgb_to_adapted);
            spektrafilm_gpu::FrontPass::Hanatos2025 {
                tc_lut,
                rgb_to_adapted_xyz: rgb_to_adapted,
            }
        } else {
            let core = self.mallett_core.as_ref()?;
            let mut matrix = crate::mallett::film_matrix(core, &self.params.io.input_color_space);
            fold_exposure(&mut matrix);
            spektrafilm_gpu::FrontPass::Mallett2019 { matrix }
        };
        Some(front)
    }

    /// Try the GPU-resident fast path. Builds all the per-stage data and
    /// hands it to the backend's `try_run_film_chain`. The pipe tail runs
    /// the output transfer and magazine passes, so the readback already
    /// carries the finalized export state. Pitch and metered EV come from
    /// the complete input before crop/rescale.
    pub(super) fn try_gpu_resident(
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
        // Input transfer decoding is shared preprocessing for the resident
        // route (pipe architecture, capability class): the front pass
        // assumes linear RGB exactly like `expose` produces for the
        // per-stage route. Same expression as `expose`, decoded once here
        // so the per-stage `expose` never double-decodes.
        let decoded;
        let image = if self.params.io.input_cctf_decoding {
            let _decode = super::stages::StageObservation::cpu(
                backend,
                "input_transfer",
                spektrafilm_gpu::telemetry::CpuReason::InputTransferDecoding,
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
        match self.resident_decision() {
            ResidentDecision::UseResident => {}
            ResidentDecision::PerStage { reasons } => {
                if let Some(context) = backend.observation_context() {
                    for reason in &reasons {
                        use spektrafilm_gpu::telemetry::ResidentDeclineReason as R;
                        let reason = match reason {
                            ResidentFallbackReason::WorkflowRoute => R::WorkflowRoute,
                            ResidentFallbackReason::NegativeToPositiveOutput => {
                                R::NegativeToPositiveOutput
                            }
                            ResidentFallbackReason::ScannerFilmPoints => R::ScannerFilmPoints,
                            ResidentFallbackReason::ScannerReceiver => R::ScannerReceiver,
                            ResidentFallbackReason::LangmuirChemistry => R::LangmuirChemistry,
                            ResidentFallbackReason::InputTransferDecoding => {
                                R::InputTransferDecoding
                            }
                            ResidentFallbackReason::RequestedSpectralLut => R::RequestedSpectralLut,
                            ResidentFallbackReason::ActiveOpticalDiffusion => {
                                R::ActiveOpticalDiffusion
                            }
                            ResidentFallbackReason::FaithfulGrainDistribution => {
                                R::FaithfulGrainDistribution
                            }
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
                    let capability_missing = reasons.iter().any(|r| {
                        !matches!(
                            r,
                            ResidentFallbackReason::WorkflowRoute
                                | ResidentFallbackReason::MissingResidentFrontPass
                                | ResidentFallbackReason::MallettExecutionParity
                                | ResidentFallbackReason::UnsupportedOutputGamut
                        )
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
        // The resident scan shader applies its nonlinear luminance remap
        // during spectral acquisition, and the CPU route forces the PCHIP
        // scanner LUT for the same condition. Corrected scans require the
        // faithful per-stage path until the positive-film resident chain
        // reaches parity (see the ScannerFilmPoints note above).
        if color_ref.has_remap() {
            if backend.explicit_gpu_request() {
                return Err(
                    "explicit GPU request cannot run the scanner luminance correction on WGPU \
                     (it must follow linear optical blur and glare on the faithful per-stage path); \
                     retry with an explicit CPU request"
                        .into(),
                );
            }
            if let Some(context) = backend.observation_context() {
                context.decline_resident(
                    spektrafilm_gpu::telemetry::ResidentDeclineReason::ScannerFilmPoints,
                );
            }
            tracing::info!(
                target: "spektrafilm_core::pipeline",
                reason = "scanner luminance correction must follow linear optical blur and glare",
                "using faithful per-stage scanner correction"
            );
            return Ok(None);
        }
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
            Some(spektrafilm_gpu::HalationGpuParams {
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
        let dir_langmuir_default = {
            let coefficients = if self.film.is_positive() {
                &dir.langmuir_receiver_k_rgb
            } else {
                &dir.langmuir_donor_k_rgb
            };
            coefficients.iter().all(|k| (k - 1.0).abs() <= f64::EPSILON)
        };
        let dir_inputs = if dir.active && dir_langmuir_default {
            let matrix = crate::chain_prep::dir_matrix(dir);
            let prepared = spektrafilm_model::couplers::prepare_dir(
                &film_curves.raw,
                film_log_exp,
                &matrix,
                dir.amount,
                self.film.is_positive(),
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
            spektrafilm_gpu::DirCouplersGpuParams {
                couplers_matrix_scaled: matrix_f32,
                density_max: [dm[0] as f32, dm[1] as f32, dm[2] as f32],
                is_positive: d.5,
                diffusion_size_px: (d.2 / d.1) as f32,
                diffusion_tail_px: (d.3 / d.1) as f32,
                diffusion_tail_weight: d.4 as f32,
                density_curves_0: &d.0.curves_0,
                log_exposure: film_log_exp,
                gamma_factor: d.6,
            }
        });
        // Non-default Langmuir chemistry runs DIR as an assembled CPU fence
        // over the density and log-raw buffers (pipe architecture,
        // capability class): the GPU shader has no Langmuir path.
        let dir_cpu_stage: Option<Box<dyn Fn(&ImageBuf, &ImageBuf) -> ImageBuf + '_>> =
            if dir.active && !dir_langmuir_default {
                let dir = &self.params.film_render.dir_couplers;
                let matrix = crate::chain_prep::dir_matrix(dir);
                let curves_raw = &film_curves.raw;
                let log_exp = film_log_exp;
                let pix_um = pixel_size_um;
                let is_positive = self.film.is_positive();
                let gamma = self.params.film_render.density_curve_gamma as f64;
                let seed_backend = backend;
                let closure: Box<dyn Fn(&ImageBuf, &ImageBuf) -> ImageBuf + '_> =
                    Box::new(move |density: &ImageBuf, log_raw: &ImageBuf| {
                        let observation =
                            crate::stages::StageObservation::new(seed_backend, "dir_couplers");
                        let observed = observation.backend(seed_backend);
                        spektrafilm_model::couplers::apply_density_correction(
                            density,
                            log_raw,
                            pix_um as f32,
                            log_exp,
                            curves_raw,
                            &matrix,
                            dir.amount,
                            dir.diffusion_size_um,
                            dir.diffusion_tail_um,
                            dir.diffusion_tail_weight,
                            is_positive,
                            gamma as f32,
                            dir.langmuir_donor_k_rgb,
                            dir.langmuir_receiver_k_rgb,
                            observed,
                        )
                    });
                Some(closure)
            } else {
                None
            };

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
                spektrafilm_gpu::GlareGpuParams {
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
            Some(spektrafilm_gpu::UnsharpGpuParams {
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

        let scanner_lens_blur_px = if self.params.scanner.lens_blur > 0.0 {
            Some(self.params.scanner.lens_blur)
        } else {
            None
        };

        let hboost = &self.params.film_render.halation;
        let highlight_boost = if hboost.boost_ev != 0.0 {
            Some(spektrafilm_gpu::HighlightBoostGpuParams {
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
        // Spektra grain runs on the host as an assembled fence on the film
        // density buffer: the resident chain keeps device residency for
        // every other stage, transferring 2 + 2 × 1 times total.
        let density_cpu_stage = if self.params.film_render.grain.active
            && matches!(
                self.params.film_render.grain.engine,
                crate::params::grain::GrainEngine::Spektra
            ) {
            let film = &self.film;
            let grain = &self.params.film_render.grain;
            let norm_curves = film_curves_norm;
            let use_fast_stats = self.params.settings.use_fast_stats;
            let random_seed = self.params.random_seed;
            let seed_backend = backend;
            let closure: Box<dyn Fn(&ImageBuf) -> ImageBuf + '_> = Box::new(move |density| {
                let observation = crate::stages::StageObservation::new(seed_backend, "grain_v1");
                let observed = observation.backend(seed_backend);
                crate::stages::filming::apply_spektra_grain(
                    density.clone(),
                    film,
                    grain,
                    norm_curves,
                    use_fast_stats,
                    random_seed,
                    pixel_size_um,
                    observed,
                )
            });
            Some(closure)
        } else {
            None
        };

        // Scanner readout interpretation (slide points, faithful B&W/slide
        // luminance remap, negative-to-positive conversion, LabScan finish)
        // runs on the host as an assembled fence on the acquisition buffer
        // — after scanner lens blur and glare, before output gamut (the CPU
        // scanning order; pipe architecture, capability class). The capture
        // methods (separable/density/coupled) interpret a captured image;
        // Digital Reference replaces the acquisition itself and still
        // declines above.
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
        let n2p_requested = self.params.scanner.scan_output == "negative_to_positive";
        // Only the capture methods (separable/coupled) interpret the
        // acquired image; the density reading and Digital Reference replace
        // the acquisition and decline in `resident_decision`.
        let n2p_capture_method = n2p_requested
            && matches!(
                self.params.scanner.positive_interpretation,
                crate::params::PositiveInterpretation::Separable
                    | crate::params::PositiveInterpretation::Coupled
            );
        // `use_scanner_lut` selects the PCHIP scanner LUT acquisition and a
        // modeled receiver selects the receiver capture; both replace the
        // GPU spectral integration on the CPU route, and the resident chain
        // runs the same CPU acquisition as an assembled fence (pipe
        // architecture, capability class). The luminance-remap condition
        // never reaches here — it declines above until the positive-film
        // chain reaches parity (issue #163).
        let scan_acquisition_cpu_stage: Option<Box<dyn Fn(&ImageBuf) -> ImageBuf + '_>> =
            if scan_context.receiver.is_some() || self.params.settings.use_scanner_lut {
                let profile = scan_profile;
                let params = &self.params;
                let color_ref = color_ref;
                let channel_density = scan_channel_density;
                let base_density = scan_base_density;
                let defer_correction = !n2p_requested;
                let closure: Box<dyn Fn(&ImageBuf) -> ImageBuf + '_> =
                    Box::new(move |density: &ImageBuf| {
                        crate::stages::scanning::scan_spectral_rgb(
                            density,
                            profile,
                            params,
                            &spektrafilm_gpu::cpu_backend::CpuBackend,
                            color_ref,
                            channel_density,
                            base_density,
                            scan_context,
                            defer_correction,
                        )
                    });
                Some(closure)
            } else {
                None
            };

        // Camera optical diffusion runs on the host as an assembled fence on
        // the raw exposure buffer — after highlight reconstruction, before
        // the camera lens blur (the CPU filming order; pipe architecture,
        // capability class). The faithful discrete-PSF convolution matches
        // the CPU reference exactly.
        let camera_diffusion_cpu_stage: Option<Box<dyn Fn(&ImageBuf) -> ImageBuf + '_>> = if self
            .params
            .camera
            .diffusion_filter
            .active
        {
            let dm = self.params.camera.diffusion_filter.to_model();
            let closure: Box<dyn Fn(&ImageBuf) -> ImageBuf + '_> =
                Box::new(move |raw: &ImageBuf| {
                    spektrafilm_model::diffusion::apply_diffusion_filter_um(raw, &dm, pixel_size_um)
                        .expect("camera diffusion filter family validated at pipeline entry")
                });
            Some(closure)
        } else {
            None
        };
        // The PCHIP enlarger LUT and the enlarger diffusion filter run on
        // the host as an assembled fence that replaces the fused print
        // spectral pass (pipe architecture, capability class): the shared
        // `expose_calibrated` already owns the LUT branch, the linear-domain
        // diffusion, and the outer log, so the fence emits the same domain
        // the fused shader produces and the GPU print density pass continues.
        let edf = &self.params.enlarger.diffusion_filter;
        let print_fence_requested =
            self.params.settings.use_enlarger_lut || (edf.active && edf.strength > 0.0);
        let print_acquisition_cpu_stage: Option<Box<dyn Fn(&ImageBuf) -> ImageBuf + '_>> =
            if print_fence_requested && !self.params.io.scan_film {
                let film = &self.film;
                let print = &self.print;
                let params = &self.params;
                let illuminant = &self.print_illuminant;
                let exposure_factor = self.print_exposure_factor;
                let preflash = self.preflash_raw;
                let bw = color_ref.printing_exposure_correction;
                let closure: Box<dyn Fn(&ImageBuf) -> ImageBuf + '_> =
                    Box::new(move |density: &ImageBuf| {
                        crate::stages::printing::expose_calibrated(
                            density,
                            film,
                            print,
                            params,
                            &spektrafilm_gpu::cpu_backend::CpuBackend,
                            illuminant,
                            exposure_factor,
                            preflash,
                            bw,
                            pixel_size_um,
                        )
                    });
                Some(closure)
            } else {
                None
            };
        let scan_tail_cpu_stage: Option<Box<dyn Fn(&ImageBuf) -> ImageBuf + '_>> =
            if n2p_capture_method {
                let profile = scan_profile;
                let params = &self.params;
                let color_ref = color_ref;
                let channel_density = scan_channel_density;
                let base_density = scan_base_density;
                let n2p = n2p_requested;
                let seed_backend = backend;
                let closure: Box<dyn Fn(&ImageBuf) -> ImageBuf + '_> =
                    Box::new(move |rgb: &ImageBuf| {
                        let observation =
                            crate::stages::StageObservation::new(seed_backend, "scan_tail");
                        let observed = observation.backend(seed_backend);
                        // Negative-to-positive binds its calibration
                        // captures to the CPU backend, exactly like the
                        // per-stage route.
                        static CPU_BACKEND: spektrafilm_gpu::cpu_backend::CpuBackend =
                            spektrafilm_gpu::cpu_backend::CpuBackend;
                        let interp_backend: &dyn spektrafilm_gpu::ComputeBackend =
                            if n2p { &CPU_BACKEND } else { observed };
                        let mut image = rgb.clone();
                        crate::stages::scanning::apply_scan_readout_interpretation(
                            &mut image,
                            profile,
                            params,
                            interp_backend,
                            color_ref,
                            channel_density,
                            base_density,
                            scan_context,
                            scan_output_space,
                            n2p,
                            false,
                            false,
                        )
                        .expect(
                            "scanner readout calibration failed on the resident fence; the \
                             per-stage route reports the same failure to the caller",
                        );
                        image
                    });
                Some(closure)
            } else {
                None
            };
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
            Some(spektrafilm_gpu::OutputTransferGpuParams {
                decode: false,
                cctf_kind,
                matrix: identity_f32,
            })
        } else if grain_v2_active && !self.params.io.output_cctf_encoding {
            Some(spektrafilm_gpu::OutputTransferGpuParams {
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
                crate::magazine_print_color::MagazineTransform::new(&scan_output_space)
                    .gpu_matrices();
            let (tone_in, tone_out, tone_slopes) = crate::magazine_print_color::gpu_tone_curve();
            let f32m = |m: &[[f64; 3]; 3]| {
                std::array::from_fn(|i| std::array::from_fn(|j| m[i][j] as f32))
            };
            let f32a = |a: &[f64; 10]| std::array::from_fn(|i| a[i] as f32);
            Some(spektrafilm_gpu::MagazineGpuParams {
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

        let params = spektrafilm_gpu::FilmChainParams {
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
            bw_xyz_remap: if scan_tail_cpu_stage.is_some() {
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
            density_cpu_stage,
            dir_cpu_stage,
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
                    spektrafilm_gpu::telemetry::ResidentDeclineReason::BlurRadiusExceedsBackendSupport,
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
        let resident = super::stages::StageObservation::new(backend, "film_chain");
        let result = resident.backend(backend).try_run_film_chain(&params);
        if let Some(context) = backend.observation_context() {
            if result.is_some() {
                context.set_path(spektrafilm_gpu::telemetry::ExecutionPath::GpuResident);
            } else {
                context.decline_resident(
                    spektrafilm_gpu::telemetry::ResidentDeclineReason::BackendNoResidentSupport,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::RuntimeParams;
    use crate::test_support::data_dir;

    #[test]
    fn grain_engine_controls_resident_fallback() {
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.settings.use_enlarger_lut = false;
        params.settings.use_scanner_lut = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.grain.active = true;
        params.film_render.grain.engine = crate::params::grain::GrainEngine::Procedural;
        let mut pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();

        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        pipeline.params.settings.rgb_to_raw_method = "mallett2019".into();
        let ResidentDecision::PerStage { reasons } = pipeline.resident_decision() else {
            panic!("Mallett must remain on the per-stage path");
        };
        assert!(
            reasons
                .iter()
                .any(|reason| { matches!(reason, ResidentFallbackReason::MallettExecutionParity) })
        );
        pipeline.params.settings.rgb_to_raw_method = "hanatos2025".into();

        pipeline.params.film_render.grain.engine = crate::params::grain::GrainEngine::Spektra;
        // Spektra no longer declines: it runs on the resident chain as an
        // assembled CPU fence on the film density buffer (pipe
        // architecture, capability class).
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));

        // Dye resolves its field on the CPU from the full developed density,
        // so no resident entry point may run it.
        pipeline.params.film_render.grain.engine = crate::params::grain::GrainEngine::Dye;
        let ResidentDecision::PerStage { reasons } = pipeline.resident_decision() else {
            panic!("Dye grain must remain on the per-stage path");
        };
        assert!(
            reasons
                .iter()
                .any(|reason| matches!(reason, ResidentFallbackReason::FilmGrainV3FieldCpu))
        );
        pipeline.params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
        pipeline.params.film_render.grain.fotufilm.amount = 1.0;
        let ResidentDecision::PerStage { reasons } = pipeline.resident_decision() else {
            panic!("Fotufilm grain must remain on the per-stage path");
        };
        assert!(
            reasons
                .iter()
                .any(|reason| matches!(reason, ResidentFallbackReason::FilmGrainFotufilmCpu))
        );
        pipeline.params.film_render.grain.fotufilm.amount = 0.0;
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
    }

    #[test]
    fn active_slide_points_decline_resident_but_defaults_do_not() {
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_ektachrome_100").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.settings.use_scanner_lut = false;
        params.settings.use_enlarger_lut = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.grain.active = false;
        let mut pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        // Slide points stay on the faithful per-stage path until the
        // positive-film resident chain reaches parity (the scan-tail fence
        // machinery is in place; see `scan_tail_cpu_stage`).
        pipeline.params.scanner.black_point[0] = 0.1;
        let ResidentDecision::PerStage { reasons } = pipeline.resident_decision() else {
            panic!("active slide points must use the faithful per-stage route");
        };
        assert!(
            reasons
                .iter()
                .any(|r| matches!(r, ResidentFallbackReason::ScannerFilmPoints))
        );
        pipeline.params.io.scan_film = false;
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
    }

    /// `use_scanner_lut` no longer demotes the route: the PCHIP scanner LUT
    /// acquisition runs as an assembled CPU fence that replaces the GPU
    /// spectral integration, so a LUT render matches the CPU reference
    /// under the standard fence budget.
    #[test]
    fn scanner_lut_acquisition_fence_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.settings.use_scanner_lut = true;
        params.settings.use_enlarger_lut = false;
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.grain.active = false;

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.05 + 0.5 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

        let out_gpu = pipeline.process(img.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();

        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        let mean = sum / out_cpu.data.len() as f64;
        assert!(mean < 5e-2, "scanner LUT fence mean divergence: {mean}");
        assert!(
            max_diff < 5e-1,
            "scanner LUT fence max divergence: {max_diff}"
        );
    }

    /// A modeled scanner receiver (LabScanApprox) no longer demotes the
    /// route: the receiver capture runs as the assembled acquisition fence
    /// replacing the GPU spectral integration, matching the CPU reference
    /// under the standard fence budget.
    #[test]
    fn scanner_receiver_acquisition_fence_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.receiver_profile = crate::params::ReceiverProfile::LabScanApprox;
        params.settings.use_scanner_lut = false;
        params.settings.use_enlarger_lut = false;
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.grain.active = false;

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.05 + 0.5 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

        let out_gpu = pipeline.process(img.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();

        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        let mean = sum / out_cpu.data.len() as f64;
        assert!(mean < 5e-2, "receiver fence mean divergence: {mean}");
        assert!(max_diff < 5e-1, "receiver fence max divergence: {max_diff}");
    }

    /// `use_enlarger_lut` no longer demotes the print route: the PCHIP
    /// enlarger LUT runs as the assembled print acquisition fence that
    /// replaces the fused GPU spectral pass, matching the CPU reference
    /// under the standard fence budget.
    #[test]
    fn enlarger_lut_print_fence_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > print > scan".into();
        params.io.scan_film = false;
        params.settings.use_scanner_lut = false;
        params.settings.use_enlarger_lut = true;
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.grain.active = false;

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.05 + 0.5 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

        let out_gpu = pipeline.process(img.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();

        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        let mean = sum / out_cpu.data.len() as f64;
        // The print route's plain resident baseline already sits at ~0.067
        // mean (never parity-frozen before this slice); the fence must stay
        // inside that route budget.
        assert!(mean < 1e-1, "enlarger LUT fence mean divergence: {mean}");
        assert!(
            max_diff < 5e-1,
            "enlarger LUT fence max divergence: {max_diff}"
        );
    }

    /// The enlarger diffusion filter runs in the same print acquisition
    /// fence (linear-domain PSF convolution between the inner and outer
    /// logarithms), so an active filter no longer demotes the print route.
    #[test]
    fn enlarger_diffusion_print_fence_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > print > scan".into();
        params.io.scan_film = false;
        params.enlarger.diffusion_filter.active = true;
        params.enlarger.diffusion_filter.strength = 0.5;
        params.enlarger.diffusion_filter.spatial_scale = 4.0;
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.grain.active = false;

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.05 + 0.5 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

        let out_gpu = pipeline.process(img.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();

        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        let mean = sum / out_cpu.data.len() as f64;
        // Print-route resident baseline budget (see the enlarger LUT test).
        assert!(
            mean < 1e-1,
            "enlarger diffusion fence mean divergence: {mean}"
        );
        assert!(
            max_diff < 5e-1,
            "enlarger diffusion fence max divergence: {max_diff}"
        );
    }

    /// Negative-to-positive capture interpretation (separable inverse) runs
    /// as the same scanner readout fence; the density reading and Digital
    /// Reference still decline because they replace the acquisition.
    #[test]
    fn negative_to_positive_capture_fence_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.settings.use_scanner_lut = false;
        params.settings.use_enlarger_lut = false;
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.grain.active = false;
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.positive_interpretation = crate::params::PositiveInterpretation::Separable;

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.05 + 0.5 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

        let out_gpu = pipeline.process(img.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();

        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        let mean = sum / out_cpu.data.len() as f64;
        assert!(
            mean < 5e-2,
            "negative-to-positive fence mean divergence: {mean}"
        );
        assert!(
            max_diff < 5e-1,
            "negative-to-positive fence max divergence: {max_diff}"
        );
    }

    /// The default Spektra grain runs on the resident chain as an assembled
    /// CPU fence on the film density buffer (pipe architecture, capability
    /// class). The fence readback narrows the grain's density conditioning
    /// to f32, which perturbs Poisson count decisions at their boundaries;
    /// this re-frozen budget (pipe-architecture spec, fence-error mode) is
    /// one decade wider than the pure-resident budget for exactly that
    /// reason. Transfer accounting: 1 readback per fence plus the final
    /// readback — 2 + 2 × fence_count total.
    #[test]
    fn spektra_grain_resident_fence_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        // Defaults keep Spektra grain active — the fence scenario.
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.05 + 0.5 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

        let out_gpu = pipeline.process(img.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();

        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        let mean = sum / out_cpu.data.len() as f64;
        assert!(mean < 5e-2, "fence mean divergence: {mean}");
        assert!(max_diff < 5e-1, "fence max divergence: {max_diff}");
    }

    /// Input transfer decoding no longer demotes to per-stage: the resident
    /// route decodes the input before upload with the same expression as
    /// `expose`, so a decoded-input render must match the CPU reference
    /// under the same Spektra-fence budget (the default engine still runs
    /// as the density fence).
    #[test]
    fn input_transfer_decoding_resident_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.camera.auto_exposure = false;
        params.io.input_cctf_decoding = true;
        params.io.input_color_space = "sRGB".into();
        params.io.output_gamut_compress.algorithm = "off".into();

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.02 + 0.96 * ((i * 31) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

        let out_gpu = pipeline.process(img.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();

        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        let mean = sum / out_cpu.data.len() as f64;
        assert!(mean < 5e-2, "decoded-input mean divergence: {mean}");
        assert!(max_diff < 5e-1, "decoded-input max divergence: {max_diff}");
    }

    /// Non-default Langmuir chemistry no longer demotes to per-stage: DIR
    /// runs as an assembled CPU fence over the density and log-raw buffers.
    /// The fence narrows both buffer inputs to f32 (GPU chain precision);
    /// the budget matches the Spektra-fence budget.
    #[test]
    fn langmuir_dir_resident_fence_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.camera.auto_exposure = false;
        params.film_render.grain.active = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.dir_couplers.active = true;
        // Negative film uses the donor coefficients.
        params.film_render.dir_couplers.langmuir_donor_k_rgb = [1.3, 0.85, 1.1];

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.05 + 0.5 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

        let out_gpu = pipeline.process(img.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();

        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        let mean = sum / out_cpu.data.len() as f64;
        assert!(mean < 5e-2, "langmuir fence mean divergence: {mean}");
        assert!(max_diff < 5e-1, "langmuir fence max divergence: {max_diff}");
    }

    /// An explicitly requested GPU backend must fail at assembly when a
    /// capability-missing stage would otherwise demote the route to CPU
    /// silently (pipe architecture fence policy). The negative-to-positive
    /// density reading replaces the acquisition itself and has no fence, so
    /// requesting it with an explicit backend errors before publication.
    #[test]
    fn explicit_gpu_request_fails_on_capability_gap() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new_explicit() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.positive_interpretation = crate::params::PositiveInterpretation::Density;

        // Capability gap: the density reading replaces the acquisition.
        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        let img = spektrafilm_math::image::ImageBuf::from_data(4, 4, vec![0.5_f32.into(); 48]);
        let err = pipeline
            .process(img, &gpu)
            .expect_err("explicit GPU request must fail on a capability gap");
        assert!(
            err.contains("explicit GPU request"),
            "unexpected error: {err}"
        );

        // The automatic route with the same configuration still succeeds on
        // the per-stage path.
        let Some(auto_gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            return;
        };
        let mut params2 = RuntimeParams::default();
        params2.camera.auto_exposure = false;
        params2.io.output_gamut_compress.algorithm = "off".into();
        params2.workflow.route = "input > film > scan".into();
        params2.io.scan_film = true;
        params2.scanner.scan_output = "negative_to_positive".into();
        params2.scanner.positive_interpretation = crate::params::PositiveInterpretation::Density;
        let pipeline2 = Pipeline::new_with_spectral(
            crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap(),
            crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap(),
            params2,
            &dir,
        )
        .unwrap();
        let img2 = spektrafilm_math::image::ImageBuf::from_data(4, 4, vec![0.5_f32.into(); 48]);
        assert!(pipeline2.process(img2, &auto_gpu).is_ok());

        // The capture methods (separable) interpret the acquisition on the
        // resident fence, so the same explicit backend must succeed — this
        // also guards the whole-route CPU fallback in `process_with_taps`
        // from swallowing the fence path again.
        let mut params3 = RuntimeParams::default();
        params3.workflow.route = "input > film > scan".into();
        params3.io.scan_film = true;
        params3.camera.auto_exposure = false;
        params3.io.output_gamut_compress.algorithm = "off".into();
        params3.scanner.scan_output = "negative_to_positive".into();
        params3.scanner.positive_interpretation = crate::params::PositiveInterpretation::Separable;
        let pipeline3 = Pipeline::new_with_spectral(
            crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap(),
            crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap(),
            params3,
            &dir,
        )
        .unwrap();
        let img3 = spektrafilm_math::image::ImageBuf::from_data(4, 4, vec![0.5_f32.into(); 48]);
        assert!(pipeline3.process(img3, &gpu).is_ok());
    }

    /// Camera optical diffusion runs on the resident chain as an assembled
    /// CPU fence on the raw exposure buffer — after highlight
    /// reconstruction, before the camera lens blur — with the exact
    /// discrete-PSF convolution, so an active filter no longer demotes the
    /// route.
    #[test]
    fn camera_diffusion_fence_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.camera.diffusion_filter.active = true;
        params.camera.diffusion_filter.strength = 0.5;
        params.camera.diffusion_filter.spatial_scale = 4.0;
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.grain.active = false;

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.05 + 0.5 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

        let out_gpu = pipeline.process(img.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();

        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        let mean = sum / out_cpu.data.len() as f64;
        assert!(
            mean < 5e-2,
            "camera diffusion fence mean divergence: {mean}"
        );
        assert!(
            max_diff < 5e-1,
            "camera diffusion fence max divergence: {max_diff}"
        );
    }

    /// The output transfer encode runs on the device as the pipe-tail pass
    /// (pipe architecture, S4): a render that requests encoded output has
    /// no post-readback host encode, so GPU and CPU agree under the
    /// standard budget.
    #[test]
    fn output_transfer_encode_pipe_tail_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.io.output_color_space = "sRGB".into();
        params.io.output_cctf_encoding = true;
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.grain.active = false;

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.05 + 0.5 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

        let out_gpu = pipeline.process(img.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();

        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        let mean = sum / out_cpu.data.len() as f64;
        assert!(mean < 5e-2, "output transfer mean divergence: {mean}");
        assert!(
            max_diff < 5e-1,
            "output transfer max divergence: {max_diff}"
        );
    }

    /// The decode half of the pipe-tail transfer: Procedural grain emits
    /// encoded samples and the tail restores linear output when the caller
    /// requested no encoding.
    #[test]
    fn output_transfer_decode_pipe_tail_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.io.output_color_space = "sRGB".into();
        params.io.output_cctf_encoding = false;
        params.film_render.grain.active = true;
        params.film_render.grain.engine = crate::params::GrainEngine::Procedural;
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.05 + 0.5 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

        let out_gpu = pipeline.process(img.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();

        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        let mean = sum / out_cpu.data.len() as f64;
        assert!(mean < 5e-2, "transfer decode mean divergence: {mean}");
        assert!(
            max_diff < 5e-1,
            "transfer decode max divergence: {max_diff}"
        );
    }

    /// The magazine appearance runs on the device as the last pipe-tail
    /// pass (pipe architecture, S4): the whole post-scan finishing is on
    /// the GPU, so GPU and CPU agree under the standard budget with the
    /// magazine active.
    #[test]
    fn magazine_pipe_tail_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.io.output_color_space = "sRGB".into();
        params.io.output_cctf_encoding = true;
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.grain.active = false;
        params.magazine_print_color.active = true;
        params.magazine_print_color.strength = 1.0;

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.05 + 0.9 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

        let out_gpu = pipeline.process(img.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();

        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        let mean = sum / out_cpu.data.len() as f64;
        assert!(mean < 5e-2, "magazine mean divergence: {mean}");
        assert!(max_diff < 5e-1, "magazine max divergence: {max_diff}");
    }
}
