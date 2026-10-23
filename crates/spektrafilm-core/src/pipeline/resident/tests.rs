use super::*;
use crate::params::RuntimeParams;
use crate::test_support::data_dir;
use spektrafilm_backend::ComputeBackend as _;
use spektrafilm_math::image::ImageBuf;

/// Parameters for the negative-to-positive density-reading acquisition
/// fence. Scanner lens blur is on: the density reading blurs inside the
/// acquisition (transmission domain), so a second blur after the fence
/// would show up.
fn n2p_fence_params() -> RuntimeParams {
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
    params.scanner.lens_blur = 1.0;
    params.scanner.unsharp_mask = [0.0, 0.0];
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
/// snapshot: a check that silently ran on another route or executor
/// would satisfy its budget vacuously.
fn observed_render(
    pipeline: &Pipeline,
    image: ImageBuf,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> spektrafilm_backend::telemetry::ObservationSnapshot {
    let context = spektrafilm_backend::telemetry::ObservationContext::new(
        spektrafilm_backend::telemetry::CollectionMode::Summary,
    );
    let observed = spektrafilm_backend::bind_backend(backend, context.clone());
    pipeline
        .process(image, observed.as_ref())
        .expect("observed resident render");
    context.snapshot()
}

/// Run a render through an observed backend and return the recorded
/// execution path: a fence check that silently ran on the per-stage CPU
/// route would satisfy its budget vacuously.
fn observed_execution_path(
    pipeline: &Pipeline,
    image: ImageBuf,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> spektrafilm_backend::telemetry::ExecutionPath {
    observed_render(pipeline, image, backend)
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
    let gpu = spektrafilm_gpu::test_support::adapter();
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
    let gpu_out = pipeline.process(image.clone(), gpu.backend()).unwrap();
    let cpu_out = pipeline
        .process(image, &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    let (mean, max_diff) = parity_metrics(&gpu_out, &cpu_out);
    assert!(
        mean < 5e-2 && max_diff < 5e-1,
        "positive slide resident parity: mean={mean:.6} max={max_diff:.6}"
    );
}

/// `use_scanner_lut` no longer demotes the route: the PCHIP scanner LUT
/// acquisition runs as an assembled CPU fence that replaces the GPU
/// spectral integration, so a LUT render matches the CPU reference
/// under the standard fence budget.
#[test]
fn scanner_lut_acquisition_fence_matches_cpu() {
    use spektrafilm_math::precision::from_f64;

    let gpu = spektrafilm_gpu::test_support::adapter();
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

    let out_gpu = pipeline.process(img.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(img, &spektrafilm_backend::cpu_backend::CpuBackend)
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

/// `use_enlarger_lut` no longer demotes the print route: the PCHIP
/// enlarger LUT runs as the assembled print acquisition fence that
/// replaces the fused GPU spectral pass, matching the CPU reference
/// under the standard fence budget.
#[test]
fn enlarger_lut_print_fence_matches_cpu() {
    use spektrafilm_math::precision::from_f64;

    let gpu = spektrafilm_gpu::test_support::adapter();
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

    let out_gpu = pipeline.process(img.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(img, &spektrafilm_backend::cpu_backend::CpuBackend)
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

    let gpu = spektrafilm_gpu::test_support::adapter();
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

    let out_gpu = pipeline.process(img.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(img, &spektrafilm_backend::cpu_backend::CpuBackend)
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

/// The default Spektra grain runs on the resident chain as an assembled
/// CPU fence on the film density buffer (pipe architecture, capability
/// class). The fence readback narrows the grain's density conditioning
/// to f32, which perturbs Poisson count decisions at their boundaries;
/// this re-frozen budget (GPU execution design, fence semantics) is
/// one decade wider than the pure-resident budget for exactly that
/// reason. Transfer accounting: 1 readback per fence plus the final
/// readback — 2 + 2 × fence_count total.
#[test]
fn spektra_grain_resident_fence_matches_cpu() {
    use spektrafilm_math::precision::from_f64;

    let gpu = spektrafilm_gpu::test_support::adapter();
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

    let out_gpu = pipeline.process(img.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(img, &spektrafilm_backend::cpu_backend::CpuBackend)
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

/// The supported Dye field is a device pass on the resident chain: the
/// render stays resident and records no host field stage, while the CPU
/// route keeps its own host field (unchanged CPU behavior). Device and
/// host realize one count law but not one draw sequence, so the budget
/// bounds two realizations of the law, not draw identity.
#[test]
fn dye_grain_field_runs_on_the_resident_device_chain() {
    let gpu = spektrafilm_gpu::test_support::adapter();
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
    let snapshot = observed_render(&pipeline, image.clone(), gpu.backend());
    assert_eq!(
        snapshot.execution.path,
        Some(spektrafilm_backend::telemetry::ExecutionPath::GpuResident),
        "backend {} did not keep the resident route",
        gpu.name()
    );
    assert!(
        !snapshot
            .observations
            .iter()
            .any(|stage| stage.name == "grain_v3_field"),
        "the resident Dye field recorded a host stage on {}: {:?}",
        gpu.name(),
        snapshot
            .observations
            .iter()
            .map(|stage| stage.name.as_str())
            .collect::<Vec<_>>()
    );

    let out_gpu = pipeline.process(image.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(image.clone(), &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    assert_eq!(
        (out_gpu.width, out_gpu.height),
        (out_cpu.width, out_cpu.height)
    );
    let (mean, max_diff) = parity_metrics(&out_gpu, &out_cpu);
    assert!(
        mean < 5e-2 && max_diff < 5e-1,
        "device vs host Dye realization on {}: mean={mean:.6} max={max_diff:.6}",
        gpu.name()
    );

    // The field changes the image, and it is the Dye engine rather than
    // the Procedural one: both must be visible. Both baselines run the
    // host field, so they pin the unchanged CPU behavior.
    let mut off = params.clone();
    off.film_render.grain.active = false;
    let baseline = Pipeline::new_with_spectral(film.clone(), print.clone(), off, &dir).unwrap();
    let out_off = baseline
        .process(image.clone(), &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    let (effect, _) = parity_metrics(&out_cpu, &out_off);
    assert!(effect > 1e-3, "dye field had no effect: mean={effect:.6}");

    let mut procedural = params;
    procedural.film_render.grain.engine = crate::params::grain::GrainEngine::Procedural;
    let other = Pipeline::new_with_spectral(film, print, procedural, &dir).unwrap();
    let out_procedural = other
        .process(image, &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    let (branch, _) = parity_metrics(&out_cpu, &out_procedural);
    assert!(
        branch > 1e-3,
        "dye field matches procedural grain: mean={branch:.6}"
    );
}

/// Spatial deactivation must bypass Dye before its field parameters are
/// resolved (spec film-grain/v3 §Controls and resolved inputs), exactly
/// like the per-stage route: no density fence is assembled, so the
/// resident route renders the grain-free reference.
#[test]
fn spatial_deactivation_bypasses_the_dye_density_fence() {
    let gpu = spektrafilm_gpu::test_support::adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = dye_fence_params();
    params.debug.deactivate_spatial_effects = true;
    assert!(
        params.film_render.grain.active,
        "the case must keep grain active so the bypass predicate is the fence gate"
    );
    let pipeline = Pipeline::new_with_spectral(film.clone(), print.clone(), params, &dir).unwrap();
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));
    let image = comparison_image(16);
    let out_gpu = pipeline.process(image.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(image.clone(), &spektrafilm_backend::cpu_backend::CpuBackend)
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
        .process(image, &spektrafilm_backend::cpu_backend::CpuBackend)
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

    let gpu = spektrafilm_gpu::test_support::adapter();
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

    let out_gpu = pipeline.process(img.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(img, &spektrafilm_backend::cpu_backend::CpuBackend)
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

    let gpu = spektrafilm_gpu::test_support::adapter();
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

    let out_gpu = pipeline.process(img.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(img, &spektrafilm_backend::cpu_backend::CpuBackend)
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
    let gpu = spektrafilm_gpu::test_support::adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let pipeline =
        Pipeline::new_with_spectral(film.clone(), print.clone(), n2p_fence_params(), &dir).unwrap();
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));
    let image = n2p_comparison_image(16);
    let out_gpu = pipeline.process(image.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(image.clone(), &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    let (mean, max_diff) = parity_metrics(&out_gpu, &out_cpu);
    assert!(
        mean < 5e-2 && max_diff < 5e-1,
        "density reading fence: mean={mean:.6} max={max_diff:.6}"
    );
    assert_eq!(
        observed_execution_path(&pipeline, image.clone(), gpu.backend()),
        spektrafilm_backend::telemetry::ExecutionPath::GpuResident
    );

    // The reading changes the image (a direct scan of the same negative
    // is the baseline), so the parity above is not vacuous.
    let mut direct = n2p_fence_params();
    direct.scanner.scan_output = "direct_scan".into();
    let baseline = Pipeline::new_with_spectral(film, print, direct, &dir).unwrap();
    let out_direct = baseline
        .process(image, &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    let (effect, _) = parity_metrics(&out_cpu, &out_direct);
    assert!(
        effect > 1e-3,
        "density reading had no effect: mean={effect:.6}"
    );
}

/// F3a acceptance: a second resident render of the same configuration must
/// reuse the static device resources it uploaded for the first one
/// (spectral LUTs, density curves, uniform blocks). The counters are the
/// same `buffer_create_count/bytes` the telemetry report exposes, so the
/// assertion also pins the measurement the plan promised.
#[test]
fn second_resident_render_reuses_static_buffers() {
    use spektrafilm_backend::telemetry::{CollectionMode, ObservationContext};
    let dir = data_dir();
    let pipeline = Pipeline::new_with_spectral(
        crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap(),
        crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap(),
        resident_parity_params(32),
        &dir,
    )
    .unwrap();
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));
    let image = comparison_image(32);
    let render = |backend: &spektrafilm_gpu::wgpu_backend::WgpuBackend, image: ImageBuf| {
        let context = ObservationContext::new(CollectionMode::Summary);
        let observed = spektrafilm_backend::bind_backend(backend, context.clone());
        let start = std::time::Instant::now();
        let out = pipeline
            .process(image, observed.as_ref())
            .expect("resident render");
        let elapsed = start.elapsed();
        let totals = context.snapshot().totals;
        (
            out,
            totals.buffer_create_count.unwrap_or_default(),
            totals.buffer_create_bytes.unwrap_or_default(),
            totals.upload_count.unwrap_or_default(),
            totals.readback_count.unwrap_or_default(),
            elapsed,
        )
    };
    // A fresh backend starts with empty caches: this render is the
    // per-frame baseline every render paid before the static cache existed.
    let fresh = spektrafilm_gpu::test_support::adapter();
    let (first_out, cold_count, cold_bytes, cold_uploads, cold_readbacks, cold_ms) =
        render(&fresh, image.clone());
    let (second_out, warm_count, warm_bytes, warm_uploads, warm_readbacks, warm_ms) =
        render(&fresh, image.clone());
    println!(
        "F3A cold={cold_count}/{cold_bytes}B {cold_ms:?} warm={warm_count}/{warm_bytes}B {warm_ms:?} \
         uploads={cold_uploads}->{warm_uploads} readbacks={cold_readbacks}->{warm_readbacks}"
    );
    assert_eq!(
        first_out.data, second_out.data,
        "caching static resources changed the rendered image"
    );
    assert_eq!(
        warm_readbacks, cold_readbacks,
        "the static cache must not add or remove device readbacks"
    );
    assert!(
        warm_uploads <= cold_uploads,
        "caching static resources must not add uploads: cold={cold_uploads} warm={warm_uploads}"
    );
    assert!(
        warm_count < cold_count,
        "the second render recreated every buffer: cold={cold_count} warm={warm_count}"
    );
    assert!(
        warm_bytes < cold_bytes,
        "the second render re-uploaded the static resources: cold={cold_bytes}B warm={warm_bytes}B"
    );
    // The wall-clock pair is reported, not asserted: on a software adapter
    // a 32x32 render is small enough for scheduling noise to order two runs
    // either way, while the counters above are deterministic.
}

/// A configuration change must not be served the previous configuration's
/// static buffers. The cache identity is the buffer content, so swapping
/// the film stock re-uploads its lookups: the first film's repeat render
/// must still match its own first render bit for bit while the second film
/// renders measurably differently.
#[test]
fn static_cache_does_not_serve_stale_data_after_a_configuration_change() {
    use spektrafilm_backend::telemetry::{CollectionMode, ObservationContext};
    let gpu = spektrafilm_gpu::test_support::adapter();
    let dir = data_dir();
    let build = |film: &str| {
        let pipeline = Pipeline::new_with_spectral(
            crate::profile::load_profile_by_name(&dir, film).unwrap(),
            crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap(),
            resident_parity_params(32),
            &dir,
        )
        .unwrap();
        assert!(matches!(
            pipeline.resident_decision(),
            ResidentDecision::UseResident
        ));
        pipeline
    };
    let portra = build("kodak_portra_400");
    let ektachrome = build("kodak_ektachrome_100");
    let image = comparison_image(32);
    let render = |pipeline: &Pipeline| {
        let context = ObservationContext::new(CollectionMode::Summary);
        let observed = spektrafilm_backend::bind_backend(gpu.backend(), context.clone());
        pipeline
            .process(image.clone(), observed.as_ref())
            .expect("resident render")
    };
    let portra_first = render(&portra);
    let ektachrome_first = render(&ektachrome);
    let portra_second = render(&portra);
    assert_eq!(
        portra_first.data, portra_second.data,
        "the cache served the other film's lookups"
    );
    let (film_difference, _) = parity_metrics(&portra_first, &ektachrome_first);
    assert!(
        film_difference > 1e-3,
        "the two films must render differently for this check to mean anything: {film_difference}"
    );
}

/// The negative-to-positive density reading replaces the acquisition, so
/// it is a fence: an explicit GPU request must fail at assembly instead
/// of running the replacement on the host silently.
#[test]
fn explicit_gpu_request_fails_on_the_n2p_acquisition_fences() {
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let image = n2p_comparison_image(8);
    let pipeline = Pipeline::new_with_spectral(film, print, n2p_fence_params(), &dir).unwrap();
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));
    let err = pipeline
        .process(image, gpu.backend())
        .expect_err("explicit GPU request must fail on the acquisition fence");
    assert!(
        err.contains("explicit GPU request") && err.contains("scanner acquisition"),
        "unexpected error: {err}"
    );
}

/// S3 acceptance, one case per migrated fence: an explicit GPU request
/// must fail at assembly for every capability-missing stage the fences
/// absorbed (pipe architecture "Policy ... demonstrated by test for each
/// S3 migration"), while the automatic route runs them — the per-fence
/// `*_fence_matches_cpu` tests hold that side. The Dye field is not a
/// case here: a supported request is a device pass
/// (`explicit_gpu_request_runs_the_dye_field_on_the_device`), and its
/// host-only form is covered by `experimental_mass_law_keeps_the_host_field`.
#[test]
fn explicit_gpu_request_fails_for_every_fenced_stage() {
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = data_dir();
    let portra = || crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
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
            "negative-to-positive density reading",
            "scanner acquisition",
            portra(),
            n2p_fence_params(),
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
            .process(image.clone(), gpu.backend())
            .expect_err("explicit GPU request must fail on the host fence");
        assert!(
            err.contains("explicit GPU request") && err.contains(expected),
            "{name}: unexpected error: {err}"
        );
    }
}

/// A cropped Dye request leaves the resident raster for the per-stage
/// readout, but that is a routing choice, not a capability gap: the device
/// field serves any readout region, so an explicit GPU request is accepted
/// (the fence policy itself is covered by
/// `explicit_gpu_request_fails_for_every_fenced_stage`, and the host field
/// reason by `experimental_mass_law_keeps_the_host_field`). What refuses
/// this request is the field's own band budget: a four-pixel source gives a
/// film pitch so large that one output row exceeds the cell limit, so the
/// run must fail with that named budget instead of silently demoting to
/// the host field.
#[test]
fn explicit_gpu_request_fails_on_the_field_band_budget() {
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > scan".into();
    params.io.scan_film = true;
    params.camera.auto_exposure = false;
    params.io.output_gamut_compress.algorithm = "off".into();
    params.io.crop = true;
    params.io.crop_size = [0.5, 0.5];
    params.film_render.grain.active = true;
    params.film_render.grain.engine = crate::params::grain::GrainEngine::Dye;
    assert_eq!(
        crate::stages::grain_v3::effective_field_backend(&params, true),
        "gpu"
    );
    let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
    assert!(
        matches!(
            pipeline.resident_decision(),
            ResidentDecision::PerStage { .. }
        ),
        "a cropped Dye request reads out per stage: {:?}",
        pipeline.resident_decision()
    );
    let img = spektrafilm_math::image::ImageBuf::from_data(4, 4, vec![0.5; 48]);
    let err = pipeline
        .process(img, gpu.backend())
        .expect_err("an output row above the band budget must fail by name");
    assert!(
        err.contains("film grain Dye") && err.contains("field cells") && err.contains("limit"),
        "unexpected error: {err}"
    );
}

/// An explicit GPU request with a supported active Dye succeeds on the
/// resident chain: the field is a device pass, so no host stage is left
/// to fail on (pipe architecture fence policy). The capability output
/// agrees that the field resolves on the device.
#[test]
fn explicit_gpu_request_runs_the_dye_field_on_the_device() {
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let params = dye_fence_params();
    assert_eq!(
        crate::stages::grain_v3::effective_field_backend(&params, true),
        "gpu"
    );
    let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));
    let image = comparison_image(16);
    let snapshot = observed_render(&pipeline, image, gpu.backend());
    assert_eq!(
        snapshot.execution.path,
        Some(spektrafilm_backend::telemetry::ExecutionPath::GpuResident),
        "backend {} did not keep the resident route",
        gpu.name()
    );
    assert!(
        !snapshot
            .observations
            .iter()
            .any(|stage| stage.name == "grain_v3_field"),
        "the explicit GPU Dye field recorded a host stage: {:?}",
        snapshot
            .observations
            .iter()
            .map(|stage| stage.name.as_str())
            .collect::<Vec<_>>()
    );
}

/// A cropped Dye request keeps the device field. The resident raster
/// cannot express the readout geometry, so the chain takes the per-stage
/// route — a routing choice, not a capability gap — and an explicit GPU
/// request must render instead of failing before publication. The field
/// stage is recorded without a CPU reason, which is how the host field is
/// labelled (`cpu_field` sets `CpuReason::GrainV3FieldCpu`).
#[test]
fn explicit_gpu_request_runs_a_cropped_dye_field_on_the_device() {
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = dye_fence_params();
    params.io.crop = true;
    params.io.crop_size = [0.5, 0.5];
    assert_eq!(
        crate::stages::grain_v3::effective_field_backend(&params, true),
        "gpu"
    );
    let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
    assert!(
        matches!(
            pipeline.resident_decision(),
            ResidentDecision::PerStage { .. }
        ),
        "a cropped Dye request reads out per stage: {:?}",
        pipeline.resident_decision()
    );
    let snapshot = observed_render(&pipeline, comparison_image(16), gpu.backend());
    assert_eq!(
        snapshot.execution.path,
        Some(spektrafilm_backend::telemetry::ExecutionPath::PerStage),
        "backend {} did not take the per-stage route",
        gpu.name()
    );
    let field = snapshot
        .observations
        .iter()
        .find(|stage| stage.name == "grain_v3_field")
        .expect("the cropped Dye field must record its stage");
    assert_eq!(
        field.cpu_reason, None,
        "the cropped field resolved on the host: {:?}",
        field
    );
}

/// The experimental mass-law selector has no device count law, so its
/// field stays the assembled host fence: the automatic route renders
/// with the published host reason, and an explicit GPU request fails by
/// name instead of silently running host arithmetic. The selector is
/// process-global, so the case runs in a child process (the presets
/// mutation pattern).
#[test]
fn experimental_mass_law_keeps_the_host_field() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "pipeline::resident::tests::experimental_mass_law_child",
            "--nocapture",
        ])
        .env("SPEKTRAFILM_GRAIN_V3_MASS_LAW", "two_class")
        .output()
        .expect("run the mass-law child case");
    assert!(
        output.status.success(),
        "mass-law child case failed:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// Child of `experimental_mass_law_keeps_the_host_field`; skipped unless
/// the selector is set (the suite runs it directly as well).
#[test]
fn experimental_mass_law_child() {
    if std::env::var("SPEKTRAFILM_GRAIN_V3_MASS_LAW").as_deref() != Ok("two_class") {
        eprintln!("skipped: run through the parent case with the mass-law selector set");
        return;
    }
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let params = dye_fence_params();
    // The capability output names the host field and its reason.
    assert_eq!(
        crate::stages::grain_v3::effective_field_backend(&params, true),
        "cpu"
    );
    let note = crate::stages::grain_v3::effective_backend_note(&film, &params, true);
    assert!(
        note.contains("MASS_LAW") && note.contains("resident chain"),
        "{note}"
    );
    let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));
    let image = comparison_image(8);
    {
        // Each adapter owns the device lock for its lifetime, so the two
        // devices in this test cannot be alive at the same time.
        let gpu = spektrafilm_gpu::test_support::explicit_adapter();
        let err = pipeline
            .process(image.clone(), gpu.backend())
            .expect_err("explicit GPU must name the host field");
        assert!(
            err.contains("explicit GPU request") && err.contains("film grain distribution"),
            "{err}"
        );
    }
    // The automatic route keeps the host fence with its published reason.
    let auto = spektrafilm_gpu::test_support::adapter();
    let snapshot = observed_render(&pipeline, image, auto.backend());
    assert_eq!(
        snapshot.execution.path,
        Some(spektrafilm_backend::telemetry::ExecutionPath::GpuResident),
        "backend {} did not keep the resident route",
        auto.name()
    );
    assert!(
        snapshot.observations.iter().any(|stage| {
            stage.name == "grain_v3_field"
                && stage.cpu_reason
                    == Some(spektrafilm_backend::telemetry::CpuReason::GrainV3FieldCpu)
        }),
        "the host field must keep its published reason: {:?}",
        snapshot.observations
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

    let gpu = spektrafilm_gpu::test_support::adapter();
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

    let out_gpu = pipeline.process(img.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(img, &spektrafilm_backend::cpu_backend::CpuBackend)
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

    let gpu = spektrafilm_gpu::test_support::adapter();
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

    let out_gpu = pipeline.process(img.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(img, &spektrafilm_backend::cpu_backend::CpuBackend)
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

    let gpu = spektrafilm_gpu::test_support::adapter();
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

    let out_gpu = pipeline.process(img.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(img, &spektrafilm_backend::cpu_backend::CpuBackend)
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

/// The cmyk appearance runs on the device as the last pipe-tail
/// pass (pipe architecture, S4): the whole post-scan finishing is on
/// the GPU, so GPU and CPU agree under the standard budget with the
/// cmyk active.
#[test]
fn cmyk_pipe_tail_matches_cpu() {
    use spektrafilm_math::precision::from_f64;

    let gpu = spektrafilm_gpu::test_support::adapter();
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
    params.cmyk.active = true;
    params.cmyk.strength = 1.0;

    let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
    let (w, h) = (16u32, 16u32);
    let data: Vec<_> = (0..w * h * 3)
        .map(|i| from_f64(0.05 + 0.9 * ((i * 37) % 256) as f64 / 255.0))
        .collect();
    let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data);

    let out_gpu = pipeline.process(img.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(img, &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();

    let mut max_diff = 0.0f64;
    let mut sum = 0.0f64;
    for (g, c) in out_gpu.data.iter().zip(out_cpu.data.iter()) {
        let d = (*g as f64 - *c as f64).abs();
        max_diff = max_diff.max(d);
        sum += d;
    }
    let mean = sum / out_cpu.data.len() as f64;
    assert!(mean < 5e-2, "cmyk mean divergence: {mean}");
    assert!(max_diff < 5e-1, "cmyk max divergence: {max_diff}");
}

/// Procedural grain, a linear export, and the cmyk tail together.
/// The CPU route applies the cmyk in the encoded domain and decodes
/// afterwards; the resident chain decodes before the cmyk pass, so
/// the two orders differ by the transfer round-trip only (measured mean
/// 7e-6, max 4.9e-4 against the reference on lavapipe) and this
/// combination must stay inside the same budget as the
/// single-dimension tail tests.
#[test]
fn procedural_grain_linear_export_with_cmyk_matches_cpu() {
    use spektrafilm_math::precision::from_f64;

    let gpu = spektrafilm_gpu::test_support::adapter();
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
    params.cmyk.active = true;
    params.cmyk.strength = 1.0;

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

    let out_gpu = pipeline.process(img.clone(), gpu.backend()).unwrap();
    let out_cpu = pipeline
        .process(img.clone(), &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    let (mean, max_diff) = parity_metrics(&out_gpu, &out_cpu);
    assert!(
        mean < 5e-2 && max_diff < 5e-1,
        "procedural linear cmyk tail: mean={mean:.6} max={max_diff:.6}"
    );

    // The cmyk must be observable, so the comparison is not vacuous.
    let mut off = params;
    off.cmyk.active = false;
    let without = Pipeline::new_with_spectral(film, print, off, &dir).unwrap();
    let out_without = without
        .process(img, &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    let (effect, _) = parity_metrics(&out_cpu, &out_without);
    assert!(effect > 1e-3, "cmyk had no effect: mean={effect:.6}");
}

#[test]
fn resident_negative_matches_cpu_per_stage() {
    let gpu = spektrafilm_gpu::test_support::adapter();
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
    let gpu_out = pipeline.process(image.clone(), gpu.backend()).unwrap();
    let cpu_out = pipeline
        .process(image, &spektrafilm_backend::cpu_backend::CpuBackend)
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

    // Dye resolves its field from the full developed density: the
    // delivered raster runs inside the resident chain (device pass, or
    // host fence for a device-unrepresentable request); a cropped
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
