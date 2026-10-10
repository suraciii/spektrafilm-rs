use super::{Pipeline, select_illuminant};
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::image::ImageBuf;

#[derive(Debug, Clone, Copy)]
enum ResidentFallbackReason {
    WorkflowRoute,
    PositiveScanOutput,
    LangmuirChemistry,
    InputTransferDecoding,
    RequestedSpectralLut,
    ActiveOpticalDiffusion,
    FaithfulGrainDistribution,
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
        if self.params.scanner.scan_output == "positive_scan" {
            reasons.push(ResidentFallbackReason::PositiveScanOutput);
        }
        let dir = &self.params.film_render.dir_couplers;
        let coefficients = if self.film.is_positive() {
            &dir.langmuir_receiver_k_rgb
        } else {
            &dir.langmuir_donor_k_rgb
        };
        if dir.active && coefficients.iter().any(|k| (k - 1.0).abs() > f64::EPSILON) {
            reasons.push(ResidentFallbackReason::LangmuirChemistry);
        }
        if self.params.io.input_cctf_decoding {
            reasons.push(ResidentFallbackReason::InputTransferDecoding);
        }
        if self.params.settings.use_scanner_lut
            || (!self.params.io.scan_film && self.params.settings.use_enlarger_lut)
        {
            reasons.push(ResidentFallbackReason::RequestedSpectralLut);
        }
        let diffusion_effective = |df: &crate::params::diffusion::DiffusionFilterParams| {
            df.active && df.strength > 0.0 && df.spatial_scale > 0.0
        };
        if diffusion_effective(&self.params.camera.diffusion_filter)
            || (!self.params.io.scan_film
                && diffusion_effective(&self.params.enlarger.diffusion_filter))
        {
            reasons.push(ResidentFallbackReason::ActiveOpticalDiffusion);
        }
        if self.params.film_render.grain.active
            && matches!(
                self.params.film_render.grain.engine,
                crate::params::grain::GrainEngine::V1
            )
        {
            reasons.push(ResidentFallbackReason::FaithfulGrainDistribution);
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
    /// hands it to the backend's `try_run_film_chain`. Grain V2 returns native
    /// encoded RGB; other paths return linear RGB. `apply_post_scan` finalizes export.
    /// Pitch and metered EV come from the complete input before crop/rescale.
    pub(super) fn try_gpu_resident(
        &self,
        image: &ImageBuf,
        backend: &dyn ComputeBackend,
        color_ref: &crate::color_reference::ColorReference,
        pixel_size_um: f64,
        ae_ev: f64,
    ) -> Option<ImageBuf> {
        if !backend.is_gpu() {
            return None;
        }
        match self.resident_decision() {
            ResidentDecision::UseResident => {}
            ResidentDecision::PerStage { reasons } => {
                if let Some(context) = backend.observation_context() {
                    for reason in &reasons {
                        use spektrafilm_gpu::telemetry::ResidentDeclineReason as R;
                        let reason = match reason {
                            ResidentFallbackReason::WorkflowRoute => R::WorkflowRoute,
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
                tracing::info!(
                    target: "spektrafilm_core::pipeline",
                    backend = backend.name(),
                    execution = "per_stage_cpu",
                    fallback_reasons = ?reasons,
                    "using per-stage path because resident GPU execution is unavailable"
                );
                return None;
            }
        }
        let front = self.resident_front_pass(color_ref, ae_ev)?;

        let film_curves = crate::chain_prep::FilmCurves::prepare(&self.film);
        let film_log_exp = film_curves.log_exposure;
        let film_curves_norm = &film_curves.normalized;
        let film_channel_density = crate::chain_prep::channel_density(&self.film);
        let film_base_density = &self.film.data.base_density;

        let print_sens = crate::chain_prep::print_sensitivity(&self.print);
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

        // print_exposure_factor × print_exposure, with the B&W printing
        // exposure correction folded in (CPU applies it as a further raw
        // multiply; print_spectral applies the whole product as one
        // normalization). 1.0 except on a corrected print scan.
        let print_exposure_scale =
            self.params.enlarger.print_exposure as f64 * color_ref.printing_exposure_correction;
        let print_norm_factor = self.print_exposure_factor * print_exposure_scale;

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
        let dir_inputs = if self.params.film_render.dir_couplers.active {
            let dir = &self.params.film_render.dir_couplers;
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
                log_exposure: &film_log_exp,
                gamma_factor: d.6,
            }
        });

        // Glare in the resident chain — applied after scan_spectral on the
        // final RGB buffer. Mirrors the CPU lognormal + blur + add. Python
        // 0.3.4 disables viewing glare entirely on the `io.scan_film` path
        // (`glare = None`; `film_render.glare` is never read upstream) —
        // only `print_render.glare` reaches the print scan.
        let glare = (!self.params.io.scan_film)
            .then(|| &self.params.print_render.glare)
            .filter(|g| g.active && g.percent > 0.0)
            .map(|g| {
                // LogNormal parameters shared with `compute_random_glare_amount`.
                let (mu, sigma) =
                    spektrafilm_model::glare::lognormal_params(g.percent, g.roughness);
                // glare_rgb_offset = (XYZ→RGB) · illuminant_xyz / 100.
                let glare_rgb_offset = crate::chain_prep::glare_rgb_offset_f64(&scan_context);
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
                boost_ev: hboost.boost_ev as f32,
                boost_range: hboost.boost_range as f32,
                protect_ev: hboost.protect_ev as f32,
            })
        } else {
            None
        };
        let grain_v2 = if self.params.film_render.grain.active
            && matches!(
                self.params.film_render.grain.engine,
                crate::params::grain::GrainEngine::V2
            ) {
            Some(self.params.film_render.grain.gpu_params(
                self.params.random_seed,
                self.params.debug.deactivate_spatial_effects,
            ))
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
            print_log_exposure: print_curves.log_exposure,
            print_density_curves: &print_curves.density,
            print_gamma: print_curves.gamma,
            print_channel_density: &print_channel_density,
            print_base_density,
            preflash: self.preflash_raw,
            viewing_illuminant: &viewing_illu,
            scan_normalization: scan_norm,
            scan_xyz_to_rgb: &scan_xyz_to_rgb,
            scan_output_space: spektrafilm_math::colorspace::resolve(
                &self.params.io.output_color_space,
            )
            .expect("validated output colour space"),
            bw_xyz_remap: color_ref.xyz_remap(),
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
        };
        if !params.gpu_blurs_supported() {
            if let Some(context) = backend.observation_context() {
                context.decline_resident(spektrafilm_gpu::telemetry::ResidentDeclineReason::BlurRadiusExceedsBackendSupport);
            }
            tracing::info!(
                target: "spektrafilm_core::pipeline",
                backend = backend.name(),
                execution = "per_stage_cpu",
                fallback_reasons = ?[ResidentFallbackReason::BlurRadiusExceedsBackendSupport],
                "using per-stage path because a resident FIR blur exceeds backend support"
            );
            return None;
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
        result
    }

    /// Finalize the export transfer without applying the same-space matrix twice.
    pub(super) fn apply_post_scan(&self, mut rgb: ImageBuf) -> ImageBuf {
        use rayon::prelude::*;
        use spektrafilm_math::precision::from_f64;
        let grain_v2_active = self.params.film_render.grain.active
            && self.params.settings.rgb_to_raw_method != "mallett2019"
            && matches!(
                self.params.film_render.grain.engine,
                crate::params::grain::GrainEngine::V2
            );
        if grain_v2_active {
            if !self.params.io.output_cctf_encoding {
                let space =
                    spektrafilm_math::colorspace::resolve(&self.params.io.output_color_space)
                        .expect("validated output colour space");
                rgb.data.par_iter_mut().for_each(|value| {
                    *value = from_f64(spektrafilm_math::colorspace::cctf_decode(
                        *value as f64,
                        space.cctf,
                    ));
                });
            }
            return rgb;
        }
        if self.params.io.output_cctf_encoding {
            let space = spektrafilm_math::colorspace::resolve(&self.params.io.output_color_space)
                .expect("validated output colour space");
            rgb.data.par_chunks_exact_mut(3).for_each(|px| {
                let out = spektrafilm_math::colorspace::encode_rgb(
                    [px[0] as f64, px[1] as f64, px[2] as f64],
                    space,
                );
                for c in 0..3 {
                    px[c] = from_f64(out[c]);
                }
            });
        }
        rgb
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::RuntimeParams;

    fn data_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data")
    }

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
        params.film_render.grain.engine = crate::params::grain::GrainEngine::V2;
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

        pipeline.params.film_render.grain.engine = crate::params::grain::GrainEngine::V1;
        let ResidentDecision::PerStage { reasons } = pipeline.resident_decision() else {
            panic!("V1 grain must remain on the per-stage path");
        };
        assert!(
            reasons.iter().any(|reason| {
                matches!(reason, ResidentFallbackReason::FaithfulGrainDistribution)
            })
        );
    }
}
