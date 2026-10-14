use super::Pipeline;

mod chain;
mod decision;
mod front_pass;

pub(crate) use decision::{ResidentDecision, ResidentFallbackReason};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::RuntimeParams;
    use crate::test_support::data_dir;
    use spektrafilm_math::image::ImageBuf;

    /// Parameters for the negative-to-positive acquisition-replacement
    /// fences. Scanner lens blur is on: both interpretations blur inside
    /// the acquisition, so a second blur after the fence would show up.
    fn n2p_fence_params(interpretation: crate::params::NegativeConversionMethod) -> RuntimeParams {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.settings.use_scanner_lut = false;
        params.settings.use_enlarger_lut = false;
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.io.output_cctf_encoding = false;
        params.film_render.grain.active = false;
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.negative_conversion_method = interpretation;
        params.scanner.lens_blur = 1.0;
        params.scanner.unsharp_mask = [0.0, 0.0];
        if interpretation == crate::params::NegativeConversionMethod::DigitalReference {
            params.scanner.receiver_profile = crate::params::ReceiverProfile::DigitalReference;
            params.scanner.auto_setup = crate::params::ScannerAutoSetup::Off;
            params.scanner.finish = crate::params::ScannerFinish::Neutral;
            params.io.output_color_space = "sRGB".into();
        }
        params
    }

    /// A deterministic negative-like scan scene for the n2p fence checks.
    fn n2p_comparison_image(n: u32) -> ImageBuf {
        use spektrafilm_math::precision::from_f64;
        let data: Vec<_> = (0..n * n * 3)
            .map(|i| from_f64(0.05 + 0.5 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        ImageBuf::from_data(n, n, data)
    }

    /// Run a render through an observed backend and return the recorded
    /// execution path: a fence check that silently ran on the per-stage CPU
    /// route would satisfy its budget vacuously.
    fn observed_execution_path(
        pipeline: &Pipeline,
        image: ImageBuf,
        backend: &dyn spektrafilm_gpu::ComputeBackend,
    ) -> spektrafilm_gpu::telemetry::ExecutionPath {
        let context = spektrafilm_gpu::telemetry::ObservationContext::new(
            spektrafilm_gpu::telemetry::CollectionMode::Summary,
        );
        let observed = spektrafilm_gpu::bind_backend(backend, context.clone());
        pipeline
            .process(image, observed.as_ref())
            .expect("observed resident render");
        context
            .snapshot()
            .execution
            .path
            .expect("resident renders record their execution path")
    }

    /// Params for the film-render resident parity checks: the slide-style
    /// `input > film > scan` route with every optional device-heavy stage
    /// off, so the comparison isolates the film develop chain (density
    /// curve, DIR couplers, scanner readout).
    fn resident_parity_params(n: u32) -> RuntimeParams {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.settings.use_scanner_lut = false;
        params.settings.use_enlarger_lut = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.grain.active = false;
        params.io.output_cctf_encoding = false;
        // Keep the diffusion kernels inside the FIR domain at this working
        // resolution so the GPU's truncated kernel and the CPU's matched
        // path agree without the IIR approximation entering the budget.
        if n > 64 {
            params.film_render.dir_couplers.diffusion_size_um = 20.0;
            params.film_render.dir_couplers.diffusion_tail_um = 200.0;
        }
        params
    }

    fn comparison_image(n: u32) -> ImageBuf {
        use spektrafilm_math::precision::from_f64;
        let data: Vec<_> = (0..n * n * 3)
            .map(|i| from_f64(0.05 + 0.9 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        ImageBuf::from_data(n, n, data)
    }

    /// Mean and max absolute deviation between the two outputs.
    fn parity_metrics(gpu: &ImageBuf, cpu: &ImageBuf) -> (f64, f64) {
        let mut max_diff = 0.0f64;
        let mut sum = 0.0f64;
        for (g, c) in gpu.data.iter().zip(cpu.data.iter()) {
            let d = (*g as f64 - *c as f64).abs();
            max_diff = max_diff.max(d);
            sum += d;
        }
        (sum / cpu.data.len() as f64, max_diff)
    }

    /// Resident chain parity for a positive slide film. Pins the DIR
    /// coupler chemistry: the Langmuir-aware "curves before DIR", the
    /// reversal receiver saturation after the diffusion blend, and the
    /// 3-Gaussian exponential tail (a linear or single-Gaussian DIR stage
    /// misses by orders of magnitude on this image).
    #[test]
    fn resident_positive_slide_matches_cpu_per_stage() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_ektachrome_100").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let pipeline =
            Pipeline::new_with_spectral(film, print, resident_parity_params(16), &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let image = comparison_image(16);
        let gpu_out = pipeline.process(image.clone(), &gpu).unwrap();
        let cpu_out = pipeline
            .process(image, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (mean, max_diff) = parity_metrics(&gpu_out, &cpu_out);
        assert!(
            mean < 5e-2 && max_diff < 5e-1,
            "positive slide resident parity: mean={mean:.6} max={max_diff:.6}"
        );
    }

    /// Resident chain parity for a negative film at a working resolution
    /// where the DIR diffusion is active — the donor-side Langmuir and the
    /// Slide points stay on the resident route: the scanner readout fence
    /// applies them after the acquisition, matching the per-stage order.
    #[test]
    fn active_slide_points_stay_resident_and_match_cpu() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_ektachrome_100").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = resident_parity_params(16);
        params.scanner.black_point = [0.05, 0.0, 0.0];
        params.scanner.white_point = [0.9, 1.0, 1.0];
        let mut base_params = params.clone();
        base_params.scanner.black_point = [0.0; 3];
        base_params.scanner.white_point = [1.0; 3];
        let pipeline =
            Pipeline::new_with_spectral(film.clone(), print.clone(), params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        // The points must be observable: the same pipeline without them
        // produces a different image, so the parity below is not vacuous.
        let baseline = Pipeline::new_with_spectral(film, print, base_params, &dir).unwrap();
        let image = comparison_image(16);
        let gpu_out = pipeline.process(image.clone(), &gpu).unwrap();
        let cpu_out = pipeline
            .process(image.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let cpu_base = baseline
            .process(image, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (effect, _) = parity_metrics(&cpu_out, &cpu_base);
        assert!(effect > 1e-3, "slide points had no effect: {effect:.6}");
        let (mean, max_diff) = parity_metrics(&gpu_out, &cpu_out);
        assert!(
            mean < 5e-2 && max_diff < 5e-1,
            "slide points resident parity: mean={mean:.6} max={max_diff:.6}"
        );
    }

    /// The B&W/slide luminance remap keeps the resident route: the readout
    /// fence applies `clip(m·Y+q, 0, 1)` after the acquisition and the
    /// acquisition itself runs the CPU's PCHIP scanner LUT. The dark input
    /// stresses the remap's `y → 0` gain, which the earlier per-stage
    /// fallback existed to avoid.
    #[test]
    fn luminance_correction_matches_cpu_on_the_resident_route() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_ektachrome_100").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = resident_parity_params(16);
        params.scanner.white_correction = true;
        params.scanner.black_correction = true;
        params.scanner.black_point = [0.05, 0.0, 0.0];
        params.scanner.white_point = [0.9, 1.0, 1.0];
        let mut base_params = params.clone();
        base_params.scanner.white_correction = false;
        base_params.scanner.black_correction = false;
        base_params.scanner.black_point = [0.0; 3];
        base_params.scanner.white_point = [1.0; 3];
        let pipeline =
            Pipeline::new_with_spectral(film.clone(), print.clone(), params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        // The correction must be observable: without it the same pipeline
        // produces a different image, so the parity below is not vacuous.
        let baseline = Pipeline::new_with_spectral(film, print, base_params, &dir).unwrap();
        let image = comparison_image(16);
        let gpu_out = pipeline.process(image.clone(), &gpu).unwrap();
        let cpu_out = pipeline
            .process(image.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let cpu_base = baseline
            .process(image, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (effect, _) = parity_metrics(&cpu_out, &cpu_base);
        assert!(
            effect > 1e-3,
            "luminance correction had no effect: {effect:.6}"
        );
        let (mean, max_diff) = parity_metrics(&gpu_out, &cpu_out);
        assert!(
            mean < 5e-2 && max_diff < 5e-1,
            "luminance correction resident parity: mean={mean:.6} max={max_diff:.6}"
        );
    }

    /// The print route builds its own luminance remap; the same readout
    /// fence carries it, under the print-route budget.
    #[test]
    fn luminance_correction_matches_cpu_on_the_print_route() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = resident_parity_params(16);
        params.io.scan_film = false;
        params.workflow.route = "input > film > print > scan".into();
        params.scanner.white_correction = true;
        params.scanner.black_correction = true;
        params.scanner.black_point = [0.05, 0.0, 0.0];
        params.scanner.white_point = [0.9, 1.0, 1.0];
        let mut base_params = params.clone();
        base_params.scanner.white_correction = false;
        base_params.scanner.black_correction = false;
        base_params.scanner.black_point = [0.0; 3];
        base_params.scanner.white_point = [1.0; 3];
        let pipeline =
            Pipeline::new_with_spectral(film.clone(), print.clone(), params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let baseline = Pipeline::new_with_spectral(film, print, base_params, &dir).unwrap();
        let image = comparison_image(16);
        let gpu_out = pipeline.process(image.clone(), &gpu).unwrap();
        let cpu_out = pipeline
            .process(image.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let cpu_base = baseline
            .process(image, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (effect, _) = parity_metrics(&cpu_out, &cpu_base);
        assert!(
            effect > 1e-3,
            "print luminance correction had no effect: {effect:.6}"
        );
        let (mean, max_diff) = parity_metrics(&gpu_out, &cpu_out);
        assert!(
            mean < 1e-1 && max_diff < 5e-1,
            "print luminance correction resident parity: mean={mean:.6} max={max_diff:.6}"
        );
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
    /// A modeled receiver rebuilds the luminance remap from its own dye
    /// captures before the readout fence; the fence must see that rebuilt
    /// reference, exactly like the per-stage route (`stages::scanning`).
    #[test]
    fn scanner_receiver_luminance_correction_matches_cpu() {
        use spektrafilm_math::precision::from_f64;

        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_ektachrome_100").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.receiver_profile = crate::params::ReceiverProfile::LabScanApprox;
        params.scanner.white_correction = true;
        params.scanner.black_correction = true;
        params.scanner.black_point = [0.05, 0.0, 0.0];
        params.scanner.white_point = [0.9, 1.0, 1.0];
        params.settings.use_scanner_lut = false;
        params.settings.use_enlarger_lut = false;
        params.camera.auto_exposure = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.film_render.grain.active = false;

        let pipeline =
            Pipeline::new_with_spectral(film.clone(), print.clone(), params, &dir).unwrap();
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
            .process(img.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (mean, max_diff) = parity_metrics(&out_gpu, &out_cpu);
        assert!(
            mean < 5e-2 && max_diff < 5e-1,
            "receiver correction fence: mean={mean:.6} max={max_diff:.6}"
        );

        // The correction must still change the image, otherwise the
        // comparison above is vacuous.
        let mut plain = RuntimeParams::default();
        plain.workflow.route = "input > film > scan".into();
        plain.io.scan_film = true;
        plain.scanner.receiver_profile = crate::params::ReceiverProfile::LabScanApprox;
        plain.settings.use_scanner_lut = false;
        plain.settings.use_enlarger_lut = false;
        plain.camera.auto_exposure = false;
        plain.io.output_gamut_compress.algorithm = "off".into();
        plain.film_render.grain.active = false;
        let uncorrected =
            Pipeline::new_with_spectral(film.clone(), print.clone(), plain, &dir).unwrap();
        let out_plain = uncorrected
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (_, uncorrected_max) = parity_metrics(&out_cpu, &out_plain);
        assert!(
            uncorrected_max > 1e-3,
            "luminance correction had no effect: max={uncorrected_max:.6}"
        );
    }

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
    /// Reference replace the acquisition in the acquisition fence (see
    /// `n2p_density_reading_fence_matches_cpu` and
    /// `digital_reference_fence_matches_cpu`).
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
        params.scanner.negative_conversion_method =
            crate::params::NegativeConversionMethod::Separable;

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

    /// Params for the Dye fence parity checks: the reference print route with
    /// every optional device-heavy stage off, the Dye engine active, and a
    /// small format so the readout pitch stays near the field's support and
    /// the resolved grain stays visible.
    fn dye_fence_params() -> RuntimeParams {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > print > scan".into();
        params.io.scan_film = false;
        params.settings.use_scanner_lut = false;
        params.settings.use_enlarger_lut = false;
        params.camera.auto_exposure = false;
        params.camera.film_format_mm = 8.0;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.io.output_cctf_encoding = false;
        params.film_render.grain.active = true;
        params.film_render.grain.engine = crate::params::grain::GrainEngine::Dye;
        params.film_render.grain.v3_dye_support_um = 8.0;
        params
    }

    /// The Dye field resolves on the CPU from the full developed density; the
    /// resident chain runs it as the assembled density fence when the request
    /// resolves to the delivered raster.
    #[test]
    fn dye_grain_density_fence_matches_cpu() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let params = dye_fence_params();
        let pipeline =
            Pipeline::new_with_spectral(film.clone(), print.clone(), params.clone(), &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let image = comparison_image(64);
        let out_gpu = pipeline.process(image.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(image.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        assert_eq!(
            (out_gpu.width, out_gpu.height),
            (out_cpu.width, out_cpu.height)
        );
        let (mean, max_diff) = parity_metrics(&out_gpu, &out_cpu);
        assert!(
            mean < 5e-2 && max_diff < 5e-1,
            "dye density fence: mean={mean:.6} max={max_diff:.6}"
        );
        assert_eq!(
            observed_execution_path(&pipeline, image.clone(), &gpu),
            spektrafilm_gpu::telemetry::ExecutionPath::GpuResident
        );

        // The field changes the image, and it is the Dye engine rather than
        // the Procedural one: both must be visible.
        let mut off = params.clone();
        off.film_render.grain.active = false;
        let baseline = Pipeline::new_with_spectral(film.clone(), print.clone(), off, &dir).unwrap();
        let out_off = baseline
            .process(image.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (effect, _) = parity_metrics(&out_cpu, &out_off);
        assert!(effect > 1e-3, "dye field had no effect: mean={effect:.6}");

        let mut procedural = params;
        procedural.film_render.grain.engine = crate::params::grain::GrainEngine::Procedural;
        let other = Pipeline::new_with_spectral(film, print, procedural, &dir).unwrap();
        let out_procedural = other
            .process(image, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (branch, _) = parity_metrics(&out_cpu, &out_procedural);
        assert!(
            branch > 1e-3,
            "dye field matches procedural grain: mean={branch:.6}"
        );
    }

    /// Spatial deactivation must bypass Dye before its field parameters are
    /// resolved (spec film-grain/v3 §Controls and compatibility), exactly
    /// like the per-stage route: no density fence is assembled, so the
    /// resident route renders the grain-free reference.
    #[test]
    fn spatial_deactivation_bypasses_the_dye_density_fence() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = dye_fence_params();
        params.debug.deactivate_spatial_effects = true;
        assert!(
            params.film_render.grain.active,
            "the case must keep grain active so the bypass predicate is the fence gate"
        );
        let pipeline =
            Pipeline::new_with_spectral(film.clone(), print.clone(), params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let image = comparison_image(16);
        let out_gpu = pipeline.process(image.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(image.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (mean, max_diff) = parity_metrics(&out_gpu, &out_cpu);
        assert!(
            mean < 5e-2 && max_diff < 5e-1,
            "bypassed dye fence: mean={mean:.6} max={max_diff:.6}"
        );

        // The bypass must be observable, and the parity budget alone cannot
        // show it: at this resolution the field's own effect (measured mean
        // 0.005) sits inside the 5e-2 fence budget. With spatial effects
        // enabled the same configuration resolves the field and changes the
        // image, so the resident output must stay far closer to the bypassed
        // reference than to the field-applied one.
        let mut enabled = dye_fence_params();
        enabled.debug.deactivate_spatial_effects = false;
        let with_field = Pipeline::new_with_spectral(film, print, enabled, &dir).unwrap();
        let field_cpu = with_field
            .process(image, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (effect, _) = parity_metrics(&out_cpu, &field_cpu);
        let (residual, _) = parity_metrics(&out_gpu, &out_cpu);
        assert!(
            effect > 10.0 * residual,
            "the resident route ran the bypassed field: field effect mean={effect:.6}, \
             gpu-vs-reference residual mean={residual:.6}"
        );
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

    /// Non-default Langmuir chemistry stays on the device: the DIR node
    /// applies the donor saturation with the caller's coefficients, so a
    /// negative film with custom knees must match the CPU reference without
    /// any fence or per-stage demotion.
    #[test]
    fn langmuir_dir_non_default_donor_matches_cpu() {
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
        assert!(mean < 5e-2, "langmuir DIR mean divergence: {mean}");
        assert!(max_diff < 5e-1, "langmuir DIR max divergence: {max_diff}");
    }

    /// The density reading replaces the acquisition, so the resident chain
    /// runs it as the assembled acquisition fence (including the
    /// transmission-domain blur inside the acquisition).
    #[test]
    fn n2p_density_reading_fence_matches_cpu() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let pipeline = Pipeline::new_with_spectral(
            film.clone(),
            print.clone(),
            n2p_fence_params(crate::params::NegativeConversionMethod::Density),
            &dir,
        )
        .unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let image = n2p_comparison_image(16);
        let out_gpu = pipeline.process(image.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(image.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (mean, max_diff) = parity_metrics(&out_gpu, &out_cpu);
        assert!(
            mean < 5e-2 && max_diff < 5e-1,
            "density reading fence: mean={mean:.6} max={max_diff:.6}"
        );
        assert_eq!(
            observed_execution_path(&pipeline, image.clone(), &gpu),
            spektrafilm_gpu::telemetry::ExecutionPath::GpuResident
        );

        // The reading changes the image (a direct scan of the same negative
        // is the baseline), and it is the density reading rather than a
        // capture interpretation: both must be visible.
        let mut direct = n2p_fence_params(crate::params::NegativeConversionMethod::Density);
        direct.scanner.scan_output = "direct_scan".into();
        let baseline =
            Pipeline::new_with_spectral(film.clone(), print.clone(), direct, &dir).unwrap();
        let out_direct = baseline
            .process(image.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (effect, _) = parity_metrics(&out_cpu, &out_direct);
        assert!(
            effect > 1e-3,
            "density reading had no effect: mean={effect:.6}"
        );

        let separable = Pipeline::new_with_spectral(
            film,
            print,
            n2p_fence_params(crate::params::NegativeConversionMethod::Separable),
            &dir,
        )
        .unwrap();
        let out_separable = separable
            .process(image, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (branch, _) = parity_metrics(&out_cpu, &out_separable);
        assert!(
            branch > 1e-3,
            "density reading matches a capture interpretation: mean={branch:.6}"
        );
    }

    /// Digital Reference replaces the acquisition with runtime-prepared
    /// spectral assets; the resident chain runs it as the same fence.
    #[test]
    fn digital_reference_fence_matches_cpu() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let pipeline = Pipeline::new_with_spectral(
            film.clone(),
            print.clone(),
            n2p_fence_params(crate::params::NegativeConversionMethod::DigitalReference),
            &dir,
        )
        .unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let image = n2p_comparison_image(16);
        let out_gpu = pipeline.process(image.clone(), &gpu).unwrap();
        let out_cpu = pipeline
            .process(image.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (mean, max_diff) = parity_metrics(&out_gpu, &out_cpu);
        assert!(
            mean < 5e-2 && max_diff < 5e-1,
            "digital reference fence: mean={mean:.6} max={max_diff:.6}"
        );
        assert_eq!(
            observed_execution_path(&pipeline, image.clone(), &gpu),
            spektrafilm_gpu::telemetry::ExecutionPath::GpuResident
        );

        // Reference Exposure scales the response, so a one-stop variant must
        // move the image; that also proves the reference conversion ran
        // rather than a capture interpretation.
        let mut shifted =
            n2p_fence_params(crate::params::NegativeConversionMethod::DigitalReference);
        shifted.scanner.reference_exposure_ev += 1.0;
        let brighter =
            Pipeline::new_with_spectral(film.clone(), print.clone(), shifted, &dir).unwrap();
        let out_brighter = brighter
            .process(image.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (effect, _) = parity_metrics(&out_cpu, &out_brighter);
        assert!(
            effect > 1e-3,
            "reference exposure had no effect: mean={effect:.6}"
        );

        let density = Pipeline::new_with_spectral(
            film,
            print,
            n2p_fence_params(crate::params::NegativeConversionMethod::Density),
            &dir,
        )
        .unwrap();
        let out_density = density
            .process(image, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (branch, _) = parity_metrics(&out_cpu, &out_density);
        assert!(
            branch > 1e-3,
            "digital reference matches the density reading: mean={branch:.6}"
        );
    }

    /// The scanner luminance correction runs on the readout fence, so an
    /// explicit GPU request must fail at assembly instead of running the
    /// fence on the host silently (pipe architecture fence policy). The
    /// automatic route keeps running it, which
    /// `luminance_correction_matches_cpu_on_the_resident_route` demonstrates.
    #[test]
    fn explicit_gpu_request_fails_on_the_scanner_correction_fence() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new_explicit() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_ektachrome_100").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = resident_parity_params(8);
        params.scanner.white_correction = true;
        params.scanner.black_correction = true;
        params.scanner.black_point = [0.05, 0.0, 0.0];
        params.scanner.white_point = [0.9, 1.0, 1.0];
        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let err = pipeline
            .process(comparison_image(8), &gpu)
            .expect_err("explicit GPU request must fail on the readout fence");
        assert!(
            err.contains("explicit GPU request") && err.contains("scanner readout interpretation"),
            "unexpected error: {err}"
        );
    }

    /// The negative-to-positive acquisition replacements are fences, so an
    /// explicit GPU request must fail at assembly for every interpretation
    /// instead of running the replacement on the host silently.
    #[test]
    fn explicit_gpu_request_fails_on_the_n2p_acquisition_fences() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new_explicit() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let image = n2p_comparison_image(8);
        for interpretation in [
            crate::params::NegativeConversionMethod::Density,
            crate::params::NegativeConversionMethod::DigitalReference,
        ] {
            let pipeline = Pipeline::new_with_spectral(
                film.clone(),
                print.clone(),
                n2p_fence_params(interpretation),
                &dir,
            )
            .unwrap();
            assert!(matches!(
                pipeline.resident_decision(),
                ResidentDecision::UseResident
            ));
            let err = pipeline
                .process(image.clone(), &gpu)
                .expect_err("explicit GPU request must fail on the acquisition fence");
            assert!(
                err.contains("explicit GPU request") && err.contains("scanner acquisition"),
                "{interpretation:?}: unexpected error: {err}"
            );
        }
    }

    /// S3 acceptance, one case per migrated fence: an explicit GPU request
    /// must fail at assembly for every capability-missing stage the fences
    /// absorbed (pipe architecture "Policy ... demonstrated by test for each
    /// S3 migration"), while the automatic route runs them — the per-fence
    /// `*_fence_matches_cpu` tests hold that side.
    #[test]
    fn explicit_gpu_request_fails_for_every_fenced_stage() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new_explicit() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let portra = || crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let ektachrome =
            || crate::profile::load_profile_by_name(&dir, "kodak_ektachrome_100").unwrap();
        let cases: Vec<(&str, &str, crate::profile::Profile, RuntimeParams)> = vec![
            (
                "input transfer decoding",
                "input transfer decoding",
                portra(),
                {
                    let mut params = RuntimeParams::default();
                    params.camera.auto_exposure = false;
                    params.io.input_cctf_decoding = true;
                    params.io.input_color_space = "sRGB".into();
                    params.io.output_gamut_compress.algorithm = "off".into();
                    params
                },
            ),
            (
                "spektra grain distribution",
                "film grain distribution",
                portra(),
                {
                    let mut params = resident_parity_params(8);
                    params.film_render.grain.active = true;
                    params
                },
            ),
            (
                "dye grain field",
                "film grain distribution",
                portra(),
                dye_fence_params(),
            ),
            (
                "camera optical diffusion",
                "camera optical diffusion",
                portra(),
                {
                    let mut params = resident_parity_params(8);
                    params.camera.diffusion_filter.active = true;
                    params.camera.diffusion_filter.strength = 0.5;
                    params.camera.diffusion_filter.spatial_scale = 4.0;
                    params
                },
            ),
            ("enlarger LUT acquisition", "print acquisition", portra(), {
                let mut params = dye_fence_params();
                params.film_render.grain.active = false;
                params.settings.use_enlarger_lut = true;
                params
            }),
            (
                "enlarger diffusion acquisition",
                "print acquisition",
                portra(),
                {
                    let mut params = dye_fence_params();
                    params.film_render.grain.active = false;
                    params.enlarger.diffusion_filter.active = true;
                    params.enlarger.diffusion_filter.strength = 0.5;
                    params
                },
            ),
            (
                "scanner LUT acquisition",
                "scanner acquisition",
                portra(),
                {
                    let mut params = resident_parity_params(8);
                    params.settings.use_scanner_lut = true;
                    params
                },
            ),
            (
                "scanner receiver acquisition",
                "scanner acquisition",
                portra(),
                {
                    let mut params = resident_parity_params(8);
                    params.scanner.receiver_profile = crate::params::ReceiverProfile::LabScanApprox;
                    params
                },
            ),
            (
                "negative-to-positive capture",
                "scanner readout interpretation",
                portra(),
                {
                    let mut params = resident_parity_params(8);
                    params.scanner.scan_output = "negative_to_positive".into();
                    params.scanner.negative_conversion_method =
                        crate::params::NegativeConversionMethod::Separable;
                    params
                },
            ),
            (
                "active slide points",
                "scanner readout interpretation",
                ektachrome(),
                {
                    let mut params = resident_parity_params(8);
                    params.scanner.black_point = [0.05, 0.0, 0.0];
                    params.scanner.white_point = [0.9, 1.0, 1.0];
                    params
                },
            ),
        ];
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let image = comparison_image(8);
        for (name, expected, film, params) in cases {
            let pipeline = Pipeline::new_with_spectral(film, print.clone(), params, &dir).unwrap();
            assert!(
                matches!(pipeline.resident_decision(), ResidentDecision::UseResident),
                "{name}: the case must keep the resident route, not decline"
            );
            let err = pipeline
                .process(image.clone(), &gpu)
                .expect_err("explicit GPU request must fail on the host fence");
            assert!(
                err.contains("explicit GPU request") && err.contains(expected),
                "{name}: unexpected error: {err}"
            );
        }
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
        // Capability gap: a cropped Dye request resolves its field on a
        // readout region the resident chain's fixed-size density buffer
        // cannot hold, so no resident fence exists for it.
        params.io.crop = true;
        params.io.crop_size = [0.5, 0.5];
        params.film_render.grain.active = true;
        params.film_render.grain.engine = crate::params::grain::GrainEngine::Dye;
        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        let img = spektrafilm_math::image::ImageBuf::from_data(4, 4, vec![0.5; 48]);
        let err = pipeline
            .process(img, &gpu)
            .expect_err("explicit GPU request must fail on a capability gap");
        assert!(
            err.contains("explicit GPU request"),
            "unexpected error: {err}"
        );
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

    /// Procedural grain, a linear export, and the magazine tail together.
    /// The CPU route applies the magazine in the encoded domain and decodes
    /// afterwards; the resident chain decodes before the magazine pass, so
    /// the two orders differ by the transfer round-trip only (measured mean
    /// 7e-6, max 4.9e-4 against the reference on lavapipe) and this
    /// combination must stay inside the same budget as the
    /// single-dimension tail tests.
    #[test]
    fn procedural_grain_linear_export_with_magazine_matches_cpu() {
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
        params.magazine_print_color.active = true;
        params.magazine_print_color.strength = 1.0;

        let pipeline =
            Pipeline::new_with_spectral(film.clone(), print.clone(), params.clone(), &dir).unwrap();
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
            .process(img.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (mean, max_diff) = parity_metrics(&out_gpu, &out_cpu);
        assert!(
            mean < 5e-2 && max_diff < 5e-1,
            "procedural linear magazine tail: mean={mean:.6} max={max_diff:.6}"
        );

        // The magazine must be observable, so the comparison is not vacuous.
        let mut off = params;
        off.magazine_print_color.active = false;
        let without = Pipeline::new_with_spectral(film, print, off, &dir).unwrap();
        let out_without = without
            .process(img, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (effect, _) = parity_metrics(&out_cpu, &out_without);
        assert!(effect > 1e-3, "magazine had no effect: mean={effect:.6}");
    }

    #[test]
    fn resident_negative_matches_cpu_per_stage() {
        let Some(gpu) = spektrafilm_gpu::wgpu_backend::WgpuBackend::new() else {
            eprintln!("no GPU adapter available; skipping");
            return;
        };
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = resident_parity_params(16);
        params.film_render.dir_couplers.diffusion_tail_weight = 0.5;
        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        let image = comparison_image(16);
        let gpu_out = pipeline.process(image.clone(), &gpu).unwrap();
        let cpu_out = pipeline
            .process(image, &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        let (mean, max_diff) = parity_metrics(&gpu_out, &cpu_out);
        assert!(
            mean < 5e-2 && max_diff < 5e-1,
            "negative resident parity: mean={mean:.6} max={max_diff:.6}"
        );
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

        // Dye resolves its field on the CPU from the full developed density.
        // The delivered raster runs as the assembled density fence; a cropped
        // request keeps the whole-route fallback.
        pipeline.params.film_render.grain.engine = crate::params::grain::GrainEngine::Dye;
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        pipeline.params.io.crop = true;
        let ResidentDecision::PerStage { reasons } = pipeline.resident_decision() else {
            panic!("a cropped Dye request must remain on the per-stage path");
        };
        assert!(
            reasons
                .iter()
                .any(|reason| matches!(reason, ResidentFallbackReason::FilmGrainV3FieldCpu))
        );
        pipeline.params.io.crop = false;
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
}
