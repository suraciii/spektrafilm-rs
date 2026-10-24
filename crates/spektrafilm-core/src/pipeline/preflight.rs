//! Central GPU preflight: the deterministic capability decision every public
//! image-processing entry point runs before any output-producing work.
//!
//! The contract ([`specs/gpu-execution/spec.md`]): full-GPU computation means
//! every image-dependent numerical stage executes on the selected device;
//! host orchestration, profile validation, and file I/O do not count. A
//! selected GPU backend must run the complete computation scope on WGPU or
//! fail before output publication — it must never silently run image
//! arithmetic on the CPU. Automatic selection may initially choose a CPU
//! backend; it does not permit a selected GPU to fall back to CPU execution.
//!
//! The resident chain already enforces the fence policy at assembly
//! (`resident::chain`); this module adds the missing boundary that the
//! per-stage and non-resident routes lack. It is deliberately *conservative*:
//! when it cannot prove a stage has a device path, a selected GPU backend
//! fails before image arithmetic rather than risking a silent CPU demotion.
//! It changes no CPU-path behavior and never rejects a CPU backend.
//!
//! Image-stage coverage is separate from preparation ownership. The retained
//! ingress also requires a device-generated TC table owned by the selected
//! backend; CPU-prepared tables cannot be uploaded to satisfy GPU coverage.
//! Bounded profile validation and coefficient resolution remain orchestration.

use super::Pipeline;
use spektrafilm_backend::ComputeBackend;

/// One unsupported image stage the selected GPU backend would have to run on
/// the host. Names reuse the published stage vocabulary so existing error
/// substrings and capability output stay stable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::pipeline) struct UnsupportedStage {
    /// The published stage name (e.g. `"scanner acquisition"`).
    pub name: &'static str,
    /// Why the selected request has no device path for this stage.
    pub reason: String,
}

/// The preflight verdict for one render request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::pipeline) enum Preflight {
    /// Every image stage the request can reach has a device path.
    Covered,
    /// Coverage cannot be proven for these stages; a selected GPU backend must
    /// fail before output publication. The first entry is the most relevant.
    Unsupported(Vec<UnsupportedStage>),
}

impl Preflight {
    /// The actionable error for a selected GPU backend, naming the complete
    /// computation scope and the unsupported stages. `Ok` when covered.
    pub(in crate::pipeline) fn selected_gpu_error(&self) -> Result<(), String> {
        let Preflight::Unsupported(stages) = self else {
            return Ok(());
        };
        Err(format!(
            "selected GPU cannot run the complete computation scope on WGPU \
             (unsupported stages: {}); retry with an explicit CPU request to use the \
             per-stage route",
            stages
                .iter()
                .map(|stage| stage.name)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

impl Pipeline {
    /// The conservative device-coverage decision for this request and backend.
    ///
    /// A CPU backend is never rejected; the CPU path is a complete, legitimate
    /// execution, so it is `Covered`. Every selected GPU backend acts on the
    /// `Unsupported` verdict, regardless of selection intent.
    pub(in crate::pipeline) fn preflight(&self, backend: &dyn ComputeBackend) -> Preflight {
        if !backend.is_gpu() {
            return Preflight::Covered;
        }
        let mut stages = self.unsupported_device_stages();
        stages.dedup_by(|a, b| a.name == b.name);
        if stages.is_empty() {
            Preflight::Covered
        } else {
            Preflight::Unsupported(stages)
        }
    }

    /// Fail a selected GPU backend that cannot cover the computation scope
    /// before any image arithmetic runs; a no-op for a CPU backend.
    ///
    /// Initial automatic CPU selection remains a valid execution route.
    pub(in crate::pipeline) fn ensure_gpu_coverage(
        &self,
        backend: &dyn ComputeBackend,
    ) -> Result<(), String> {
        if !backend.is_gpu() {
            return Ok(());
        }
        self.preflight(backend).selected_gpu_error()
    }

    /// The image stages this request would have to execute on the host, in
    /// execution order. Each clause reuses the exact gate the producing code
    /// uses, so the preflight never contradicts what actually runs, and the
    /// resident chain's own assembly guard remains the authority for the
    /// stages it already covers.
    fn unsupported_device_stages(&self) -> Vec<UnsupportedStage> {
        let mut stages = Vec::new();
        let p = &self.params;

        // The input-only route's terminal conversion and the convert-film
        // route's density solver both run on the device now, so neither is a
        // gap. The convert route's remaining stages are the printing and
        // scanning passes the clauses below already classify.
        let route = p.workflow.route_enum();

        // Full-source scanner auto-setup is a device reduction over the
        // complete source: the selected backend measures and interprets the
        // finite population on device and returns only bounded controls, so
        // neither an active policy nor an attached setup is a host stage.

        // The resident front pass (RGB→raw upsampler) is what the whole
        // resident route depends on; without it no device path exists.
        // Input-only uses terminal conversion, and convert-film starts with
        // the inverse scanner solve, so neither needs an RGB-to-raw head.
        if self.tc_lut.is_none()
            && self.mallett_core.is_none()
            && !route.is_convert_film()
            && route != crate::WorkflowRoute::Input
        {
            stages.push(UnsupportedStage {
                name: "missing resident front pass",
                reason: "no RGB-to-raw front pass was resolved for the resident route".into(),
            });
        }

        // Input transfer decoding runs on the device: the resident chain
        // decodes as its first pass, and the input-only route folds the decode
        // into its terminal conversion. A route that never enters the resident
        // chain and never runs the input-route conversion is not reachable.
        let _ = p.io.input_cctf_decoding;

        // Full-source auto-exposure metering runs on the device: the meter is
        // a reduction over the reference's <=256 px preview, and the exposure
        // multiply folds into the resident front-pass matrix (or the per-stage
        // `expose`), so no host image arithmetic remains on this stage.
        let _ = p.camera.auto_exposure;

        // Grain: Spektra resolves on the device in both sampling regimes
        // whenever the film profile supplies the sublayer density curves the
        // pass reads; Fotufilm population formation also runs on the device.
        // Dye is covered unless its selected experimental mass law has no
        // device field path; the published host-field reason names that gap.
        let grain = &p.film_render.grain;
        // The input route runs no film stage, so it has no grain to distribute.
        let grain_reachable = route != crate::WorkflowRoute::Input;
        let dye_gap = crate::stages::grain_v3::active(p)
            && crate::stages::grain_v3::published_host_field_reason(p).is_some();
        // Both Spektra sampling regimes are device passes; the request keeps
        // the host fence only when the film profile cannot supply the sublayer
        // density curves the pass reads (`stages::filming::spektra_device_gap`).
        let spektra_gap =
            if grain.active && grain.engine == crate::params::grain::GrainEngine::Spektra {
                crate::stages::filming::spektra_device_gap(&self.film).map(str::to_owned)
            } else {
                None
            };
        if grain_reachable && (dye_gap || spektra_gap.is_some()) {
            let reason = if dye_gap {
                crate::stages::grain_v3::published_host_field_reason(p).unwrap_or_default()
            } else {
                spektra_gap.unwrap_or_default()
            };
            stages.push(UnsupportedStage {
                name: "film grain distribution",
                reason,
            });
        }
        // A Fotufilm request is device-covered: the population formation and
        // the residual readout run on the device, so no host stage remains to
        // report. `specs/gpu-execution/spec.md` §Computation scope names
        // "Fotufilm population formation and readout" in full-GPU scope.

        // Dye crop tails and optical probes are device stages. Keep the
        // geometry authority's remaining coverage decision, rather than
        // permitting a per-stage host route when it cannot assemble them.
        if crate::stages::grain_v3::active(p)
            && !crate::stages::grain_v3::resident_fence_supported(&self.film, p)
        {
            stages.push(UnsupportedStage {
                name: "film grain route",
                reason: "the Dye readout geometry has no complete resident device route".into(),
            });
        }

        // Output gamut compression needs a device path; without one it is a
        // host stage.
        if self.output_gamut.is_active() && self.output_gamut.gpu_params().is_none() {
            stages.push(UnsupportedStage {
                name: "output gamut compression",
                reason: "the active output gamut has no device path".into(),
            });
        }

        // The scanner acquisition and the scanner readout interpretation are
        // device-covered for every selected method: the modeled receiver
        // capture, the PCHIP scanner LUT (which the luminance remap also
        // selects), the Digital Reference acquisition, the density reading,
        // the slide points, the separable inverse, the coupled inverse, and
        // the LabScan finish all run on the device, each with the host
        // reference available only to an initially selected CPU backend.

        // A resident Gaussian uses the reference FIR/IIR pair with no radius
        // cap, so only the discrete PSF convolution has a device bound.

        stages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::RuntimeParams;
    use crate::test_support::data_dir;

    /// A GPU backend that claims explicit-GPU selection without a real device,
    /// so the classification and the error path are exercised deterministically
    /// on any machine. Every image method delegates to the CPU implementation;
    /// only the capability flags matter to the preflight.
    struct FakeExplicitGpu;
    impl ComputeBackend for FakeExplicitGpu {
        fn colorspace_convert(&self,
        image: &spektrafilm_math::image::ImageBuf,
        matrix: &[[f32; 3]; 3],) -> Result<spektrafilm_math::image::ImageBuf, String> { spektrafilm_backend::cpu_backend::CpuBackend.colorspace_convert(image, matrix) }
        fn cctf_encode_srgb(&self,
        image: &spektrafilm_math::image::ImageBuf,) -> Result<spektrafilm_math::image::ImageBuf, String> { spektrafilm_backend::cpu_backend::CpuBackend.cctf_encode_srgb(image) }
        fn cctf_decode_srgb(&self,
        image: &spektrafilm_math::image::ImageBuf,) -> Result<spektrafilm_math::image::ImageBuf, String> { spektrafilm_backend::cpu_backend::CpuBackend.cctf_decode_srgb(image) }
        fn gaussian_blur(&self,
        image: &spektrafilm_math::image::ImageBuf,
        sigma: f32,) -> Result<spektrafilm_math::image::ImageBuf, String> { spektrafilm_backend::cpu_backend::CpuBackend.gaussian_blur(image, sigma) }
        fn table_lookup(&self,
        image: &spektrafilm_math::image::ImageBuf,
        table_x: &[f32],
        table_y: &[[f32; 3]],) -> Result<spektrafilm_math::image::ImageBuf, String> { spektrafilm_backend::cpu_backend::CpuBackend.table_lookup(image, table_x, table_y) }
        fn lut3d_interp(&self,
        image: &spektrafilm_math::image::ImageBuf,
        lut: &spektrafilm_backend::Lut3D,) -> Result<spektrafilm_math::image::ImageBuf, String> { spektrafilm_backend::cpu_backend::CpuBackend.lut3d_interp(image, lut) }
        fn is_gpu(&self) -> bool {
            true
        }
        fn explicit_gpu_request(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            "fake-explicit-gpu"
        }
    }

    fn portra() -> crate::profile::Profile {
        crate::profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap()
    }
    fn endura() -> crate::profile::Profile {
        crate::profile::load_profile_by_name(&data_dir(), "kodak_portra_endura").unwrap()
    }

    /// A covered device route: a resident film-scan with grain, output
    /// gamut, transfer and auto-exposure all off, so no image stage lacks a
    /// device path. Camera diffusion and the enlarger print acquisition are
    /// device-covered, so they stay at their defaults.
    fn covered_params() -> RuntimeParams {
        let mut p = RuntimeParams::default();
        p.workflow.route = "input > film > scan".into();
        p.io.scan_film = true;
        p.camera.auto_exposure = false;
        p.io.input_cctf_decoding = false;
        p.io.output_gamut_compress.algorithm = "off".into();
        p.film_render.grain.active = false;
        p
    }

    fn covered_pipeline() -> Pipeline {
        let mut p = covered_params();
        p.io.output_cctf_encoding = false;
        Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap()
    }

    #[test]
    fn a_covered_device_route_passes_the_explicit_gpu_preflight() {
        let pipeline = covered_pipeline();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
        assert!(pipeline.ensure_gpu_coverage(&FakeExplicitGpu).is_ok());
    }

    #[test]
    fn a_cpu_backend_is_never_rejected_even_with_a_gap() {
        // An unresolved front pass is a device gap, but an initially selected
        // CPU backend is a complete, legitimate execution and must pass.
        let pipeline = Pipeline::new(portra(), endura(), RuntimeParams::default());
        assert_eq!(
            pipeline.preflight(&spektrafilm_backend::cpu_backend::CpuBackend),
            Preflight::Covered
        );
        assert!(
            pipeline
                .ensure_gpu_coverage(&spektrafilm_backend::cpu_backend::CpuBackend)
                .is_ok()
        );
    }

    #[test]
    fn the_error_names_the_scope_and_the_unsupported_stage() {
        // No resident front pass was prepared, so GPU coverage must fail.
        let pipeline = Pipeline::new(portra(), endura(), covered_params());
        let err = pipeline
            .ensure_gpu_coverage(&FakeExplicitGpu)
            .expect_err("an explicit GPU request with a host fence must fail");
        assert!(
            err.contains("complete computation scope"),
            "error must name the scope: {err}"
        );
        assert!(
            err.contains("missing resident front pass"),
            "error must name the unsupported stage: {err}"
        );
    }

    /// Both Spektra sampling regimes resolve on the device, so an active
    /// Spektra grain is not a host fence: an explicit GPU request stays
    /// covered whether or not the request selects the fast-statistics regime.
    #[test]
    fn spektra_grain_is_not_a_device_gap() {
        for use_fast_stats in [false, true] {
            let mut p = covered_params();
            p.settings.use_fast_stats = use_fast_stats;
            p.film_render.grain.active = true;
            p.film_render.grain.engine = crate::params::grain::GrainEngine::Spektra;
            let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
            assert_eq!(
                pipeline.preflight(&FakeExplicitGpu),
                Preflight::Covered,
                "Spektra grain (use_fast_stats={use_fast_stats}) must not be a host fence"
            );
            assert!(pipeline.ensure_gpu_coverage(&FakeExplicitGpu).is_ok());
        }
    }

    #[test]
    fn an_input_only_route_is_not_a_device_gap() {
        // The route's terminal conversion is a device pass now.
        let mut p = covered_params();
        p.workflow.route = "input".into();
        p.io.scan_film = false;
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
        let unprepared = Pipeline::new(portra(), endura(), pipeline.params.clone());
        assert_eq!(unprepared.preflight(&FakeExplicitGpu), Preflight::Covered);
    }

    #[test]
    fn a_convert_film_route_is_not_a_device_gap() {
        // The convert-film density solver runs on the device now, so the
        // route's own stages are covered; the shared clauses below decide
        // whatever else the request selects.
        let mut p = covered_params();
        p.workflow.route = "input > convert-film > scan".into();
        p.io.scan_film = false;
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
    }

    #[test]
    fn a_convert_film_route_with_fotufilm_is_device_covered() {
        // Both inverse scanning and Fotufilm population formation have
        // device implementations; neither is a host fence.
        let mut p = covered_params();
        p.workflow.route = "input > convert-film > scan".into();
        p.io.scan_film = false;
        p.film_render.grain.active = true;
        p.film_render.grain.engine = crate::params::grain::GrainEngine::Fotufilm;
        p.film_render.grain.fotufilm.amount = 1.0;
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
        assert!(pipeline.ensure_gpu_coverage(&FakeExplicitGpu).is_ok());
    }

    #[test]
    fn a_missing_front_pass_is_a_device_gap() {
        // `Pipeline::new` resolves no TC LUT or Mallett core, so no device
        // front pass exists at all.
        let pipeline = Pipeline::new(portra(), endura(), covered_params());
        let err = pipeline
            .ensure_gpu_coverage(&FakeExplicitGpu)
            .expect_err("a request without a front pass cannot run on the device");
        assert!(
            err.contains("missing resident front pass"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn output_mix_below_one_is_not_a_device_gap() {
        // The final mix is a device blend now, so a partial mix stays covered.
        let mut p = covered_params();
        p.output_mix.amount = 0.5;
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
    }

    #[test]
    fn fotufilm_population_formation_is_device_covered() {
        let mut p = covered_params();
        p.film_render.grain.active = true;
        p.film_render.grain.engine = crate::params::grain::GrainEngine::Fotufilm;
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
        assert!(pipeline.ensure_gpu_coverage(&FakeExplicitGpu).is_ok());
    }

    /// The Mallett2019 front pass is a device matmul like the hanatos2025
    /// front pass, so an explicit GPU request with it is covered: the
    /// display-domain grain and optics stages follow the same device
    /// nodes whichever upsampler produced the raw exposure.
    #[test]
    fn the_mallett_front_pass_is_not_a_device_gap() {
        let mut p = covered_params();
        p.settings.rgb_to_raw_method = "mallett2019".into();
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
        assert!(pipeline.ensure_gpu_coverage(&FakeExplicitGpu).is_ok());
    }

    #[test]
    fn device_covered_scanner_stages_are_not_gaps() {
        // The neutral scan model v2 resolves the scanner acquisition's tone
        // constants on the host, and the acquisition itself is a device
        // spectral pass, so a negative interpretation stays device-covered.
        type ScannerStageCase = (&'static str, fn(&mut RuntimeParams));
        let cases: &[ScannerStageCase] = &[
            ("scanner LUT", |p: &mut RuntimeParams| {
                p.settings.use_scanner_lut = true;
            }),
            ("negative interpretation", |p: &mut RuntimeParams| {
                p.scanner.scan_output = "negative_to_positive".into();
            }),
            ("interpretation tone style", |p: &mut RuntimeParams| {
                p.scanner.scan_output = "negative_to_positive".into();
                p.scanner.conversion_style = "neutral".into();
            }),
        ];
        for (name, mutate) in cases {
            let mut p = covered_params();
            mutate(&mut p);
            let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
            assert_eq!(
                pipeline.preflight(&FakeExplicitGpu),
                Preflight::Covered,
                "{name} must stay device-covered"
            );
        }
    }

    #[test]
    fn auto_exposure_is_device_covered() {
        // Full-source metering is a device reduction over the reference's
        // <=256 px preview, and the exposure multiply folds into the resident
        // front-pass matrix, so an active auto-exposure is no longer a gap.
        let mut p = covered_params();
        p.camera.auto_exposure = true;
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
        assert!(pipeline.ensure_gpu_coverage(&FakeExplicitGpu).is_ok());
    }

    #[test]
    fn a_cropped_dye_route_with_downstream_optics_is_device_covered() {
        // The device reads an enclosing full-film lattice through spatial
        // optics, then applies its post-optics crop tail on the device.
        let mut p = covered_params();
        p.io.scan_film = false;
        p.workflow.route = "input > film > print > scan".into();
        p.camera.film_format_mm = 8.0;
        p.film_render.grain.active = true;
        p.film_render.grain.engine = crate::params::grain::GrainEngine::Dye;
        p.film_render.grain.v3_dye_support_um = 8.0;
        p.io.crop = true;
        p.io.crop_size = [0.5, 0.5];
        p.scanner.unsharp_mask = [0.7, 0.7];
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
        assert!(pipeline.ensure_gpu_coverage(&FakeExplicitGpu).is_ok());
    }

    #[test]
    fn a_cropped_dye_route_without_downstream_optics_is_device_covered() {
        // A film scan with a crop-only readout is device-covered as well as
        // the enclosing-lattice route with downstream optics above.
        let mut p = covered_params();
        p.camera.film_format_mm = 8.0;
        p.scanner.unsharp_mask = [0.0; 2];
        p.film_render.grain.active = true;
        p.film_render.grain.engine = crate::params::grain::GrainEngine::Dye;
        p.film_render.grain.v3_dye_support_um = 8.0;
        p.io.crop = true;
        p.io.crop_size = [0.5, 0.5];
        p.io.upscale_factor = 1.5;
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
    }

    #[test]
    fn an_inert_enlarger_control_on_a_film_scan_is_not_a_device_gap() {
        // A film scan skips the print stage, so the enlarger control is inert.
        let mut p = covered_params();
        p.settings.use_enlarger_lut = true;
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
    }

    #[test]
    fn the_camera_diffusion_filter_is_device_covered() {
        // The device convolves the exact discrete PSF, so an active camera
        // diffusion filter is not a device gap.
        let mut p = covered_params();
        p.camera.diffusion_filter.active = true;
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
    }

    #[test]
    fn the_enlarger_print_acquisition_is_device_covered() {
        // The enlarger LUT and the enlarger diffusion filter both have
        // device paths on the print route.
        let mut p = covered_params();
        p.io.scan_film = false;
        p.workflow.route = "input > film > print > scan".into();
        p.settings.use_enlarger_lut = true;
        p.enlarger.diffusion_filter.active = true;
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
    }

    #[test]
    fn a_device_covered_crop_is_not_a_device_gap() {
        // The retained route applies crop and upscale on the device; a
        // non-Dye grain with no other host stage must stay covered.
        let mut p = covered_params();
        p.io.crop = true;
        p.io.crop_size = [0.5, 0.5];
        p.io.upscale_factor = 2.0;
        let pipeline = Pipeline::new_with_spectral(portra(), endura(), p, &data_dir()).unwrap();
        assert_eq!(pipeline.preflight(&FakeExplicitGpu), Preflight::Covered);
    }

    #[test]
    fn an_automatically_selected_gpu_rejects_missing_device_coverage() {
        // Automatic intent does not authorize CPU work once GPU selection
        // has succeeded. Initial CPU selection remains a separate case.
        struct AutomaticGpu;
        impl ComputeBackend for AutomaticGpu {
            fn colorspace_convert(&self,
            image: &spektrafilm_math::image::ImageBuf,
            matrix: &[[f32; 3]; 3],) -> Result<spektrafilm_math::image::ImageBuf, String> { spektrafilm_backend::cpu_backend::CpuBackend.colorspace_convert(image, matrix) }
            fn cctf_encode_srgb(&self,
            image: &spektrafilm_math::image::ImageBuf,) -> Result<spektrafilm_math::image::ImageBuf, String> { spektrafilm_backend::cpu_backend::CpuBackend.cctf_encode_srgb(image) }
            fn cctf_decode_srgb(&self,
            image: &spektrafilm_math::image::ImageBuf,) -> Result<spektrafilm_math::image::ImageBuf, String> { spektrafilm_backend::cpu_backend::CpuBackend.cctf_decode_srgb(image) }
            fn gaussian_blur(&self,
            image: &spektrafilm_math::image::ImageBuf,
            sigma: f32,) -> Result<spektrafilm_math::image::ImageBuf, String> { spektrafilm_backend::cpu_backend::CpuBackend.gaussian_blur(image, sigma) }
            fn table_lookup(&self,
            image: &spektrafilm_math::image::ImageBuf,
            table_x: &[f32],
            table_y: &[[f32; 3]],) -> Result<spektrafilm_math::image::ImageBuf, String> { spektrafilm_backend::cpu_backend::CpuBackend.table_lookup(image, table_x, table_y) }
            fn lut3d_interp(&self,
            image: &spektrafilm_math::image::ImageBuf,
            lut: &spektrafilm_backend::Lut3D,) -> Result<spektrafilm_math::image::ImageBuf, String> { spektrafilm_backend::cpu_backend::CpuBackend.lut3d_interp(image, lut) }
            fn is_gpu(&self) -> bool {
                true
            }
            fn name(&self) -> &str {
                "fake-auto-gpu"
            }
        }
        let pipeline = Pipeline::new(portra(), endura(), covered_params());
        assert!(matches!(
            pipeline.preflight(&AutomaticGpu),
            Preflight::Unsupported(_)
        ));
        let error = pipeline.ensure_gpu_coverage(&AutomaticGpu).unwrap_err();
        assert!(error.contains("missing resident front pass"), "{error}");
    }
}
