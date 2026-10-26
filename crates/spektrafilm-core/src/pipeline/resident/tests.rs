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
    let pipeline = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), backend)
        .expect("prepare calibration on the observed device");
    let context = spektrafilm_backend::telemetry::ObservationContext::new(
        spektrafilm_backend::telemetry::CollectionMode::Summary,
    );
    let observed = spektrafilm_backend::bind_backend(backend, context.clone());
    pipeline
        .process(image, observed.as_ref())
        .expect("observed resident render");
    context.snapshot()
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
    let gpu_out = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(image.clone(), gpu.backend())
        .unwrap();
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
    assert!(
        film.data
            .channel_density
            .iter()
            .any(|row| row.iter().any(|v| v.is_nan())),
        "scanner LUT regression requires the shipped missing spectral rows"
    );
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

    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(img.clone(), gpu.backend())
        .unwrap();
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
    assert!(
        film.data
            .channel_density
            .iter()
            .any(|row| row.iter().any(|v| v.is_nan())),
        "enlarger LUT regression requires the shipped missing spectral rows"
    );
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

    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(img.clone(), gpu.backend())
        .unwrap();
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

    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(img.clone(), gpu.backend())
        .unwrap();
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

    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(img.clone(), gpu.backend())
        .unwrap();
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

    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(image.clone(), gpu.backend())
        .unwrap();
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
    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(image.clone(), gpu.backend())
        .unwrap();
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

    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(img.clone(), gpu.backend())
        .unwrap();
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

    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(img.clone(), gpu.backend())
        .unwrap();
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

/// The device acquisition includes transmission-domain blur before density
/// interpretation, rather than blurring the interpreted positive image.
#[test]
fn n2p_device_density_reading_matches_cpu() {
    let gpu = spektrafilm_gpu::test_support::adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut pipeline =
        Pipeline::new_with_spectral(film.clone(), print.clone(), n2p_fence_params(), &dir).unwrap();
    pipeline.conversion_tones = [
        crate::stages::scanning::ChannelTone {
            brightness_ev: -0.3,
            contrast: 0.55,
            dmax: 1.1,
        },
        crate::stages::scanning::ChannelTone {
            brightness_ev: 0.8,
            contrast: 0.85,
            dmax: 1.7,
        },
        crate::stages::scanning::ChannelTone {
            brightness_ev: 1.2,
            contrast: 1.1,
            dmax: 2.1,
        },
    ];
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));
    let image = n2p_comparison_image(16);
    let mut device_pipeline = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap();
    device_pipeline.conversion_tones = pipeline.conversion_tones;
    let out_gpu = device_pipeline
        .process(image.clone(), gpu.backend())
        .unwrap();
    let out_cpu = pipeline
        .process(image.clone(), &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    let (mean, max_diff) = parity_metrics(&out_gpu, &out_cpu);
    eprintln!("standard setup CPU/resident output parity mean={mean:.9} max={max_diff:.9}");
    assert!(
        mean < 5e-3 && max_diff < 5e-2,
        "device density reading: mean={mean:.6} max={max_diff:.6}"
    );
    let snapshot = observed_render(&device_pipeline, image.clone(), gpu.backend());
    assert_eq!(
        snapshot.execution.path,
        Some(spektrafilm_backend::telemetry::ExecutionPath::GpuResident)
    );
    assert_eq!(
        snapshot
            .observations
            .iter()
            .filter(|stage| stage.name == "standard_setup")
            .count(),
        1,
        "resident acquisition must measure the complete frame exactly once"
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
    let fresh = spektrafilm_gpu::test_support::adapter();
    let dir = data_dir();
    let pipeline = Pipeline::new_with_spectral_backend(
        crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap(),
        crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap(),
        resident_parity_params(32),
        &dir,
        fresh.backend(),
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
        let pipeline = Pipeline::new_with_spectral_backend(
            crate::profile::load_profile_by_name(&dir, film).unwrap(),
            crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap(),
            resident_parity_params(32),
            &dir,
            gpu.backend(),
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

/// Device acquisition retains its entire numerical body, including optics.
#[test]
fn explicit_gpu_request_runs_n2p_density_acquisition() {
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = data_dir();
    let pipeline = Pipeline::new_with_spectral(
        crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap(),
        crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap(),
        n2p_fence_params(),
        &dir,
    )
    .unwrap();
    let image = n2p_comparison_image(8);
    let output = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(image.clone(), gpu.backend())
        .unwrap();
    let reference = pipeline
        .process(image.clone(), &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    let (mean, max) = parity_metrics(&output, &reference);
    assert!(mean < 5e-3 && max < 5e-2, "n2p mean={mean} max={max}");
    let snapshot = observed_render(&pipeline, image, gpu.backend());
    assert_eq!(
        snapshot.execution.path,
        Some(spektrafilm_backend::telemetry::ExecutionPath::GpuResident)
    );
    assert!(
        !snapshot
            .observations
            .iter()
            .any(|stage| stage.executor == Some(spektrafilm_backend::telemetry::Executor::Cpu)),
        "selected GPU density acquisition executed host image arithmetic: {:?}",
        snapshot.observations
    );
}

/// Conversion must enter the inverse-density head, not expose/develop again.
#[test]
fn explicit_gpu_convert_routes_match_cpu() {
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    assert!(
        film.data
            .channel_density
            .iter()
            .any(|row| row.iter().any(|v| v.is_nan())),
        "the shipped film must retain missing spectral rows for inverse-scan regression"
    );
    let image = comparison_image(8);
    let mut outputs = Vec::new();
    for route in [
        "input > convert-film > scan",
        "input > convert-film > scan-minus-base",
        "input > convert-film > print > scan",
    ] {
        let mut params = n2p_fence_params();
        params.workflow.route = route.into();
        params.io.scan_film = false;
        params.scanner.scan_output = "direct_scan".into();
        let reference_pipeline = Pipeline::new_with_spectral_backend(
            film.clone(),
            print.clone(),
            params.clone(),
            &dir,
            &spektrafilm_backend::cpu_backend::CpuBackend,
        )
        .unwrap();
        let pipeline = Pipeline::new_with_spectral_backend(
            film.clone(),
            print.clone(),
            params,
            &dir,
            gpu.backend(),
        )
        .unwrap();
        let output = pipeline
            .clone()
            .with_params_with_backend(pipeline.params.clone(), gpu.backend())
            .unwrap()
            .process(image.clone(), gpu.backend())
            .unwrap();
        let reference = reference_pipeline
            .process(image.clone(), &spektrafilm_backend::cpu_backend::CpuBackend)
            .unwrap();
        let density_reference = crate::stages::converting::process(
            &image,
            &reference_pipeline.film,
            &reference_pipeline.params,
        )
        .unwrap();
        let density = pipeline
            .process_with_taps(
                image.clone(),
                gpu.backend(),
                Some(crate::params::Tap::RgbPre),
                Some(crate::params::Tap::CmyFilm),
            )
            .unwrap();
        let (density_mean, density_max) = parity_metrics(&density, &density_reference);
        assert!(
            density_mean < 5e-3 && density_max < 5e-2,
            "{route} inverse density: mean={density_mean} max={density_max}"
        );
        assert!(
            density_reference.data.iter().any(|&value| value > 0.1),
            "missing spectral rows must not collapse the inverse density to zero"
        );
        let (mean, max) = parity_metrics(&output, &reference);
        assert!(mean < 5e-3 && max < 5e-2, "{route}: mean={mean} max={max}");
        if route == "input > convert-film > scan" {
            let mut physical_density = density_reference.clone();
            for value in &mut physical_density.data {
                *value = (*value as f32) as _;
            }
            let mut scanner_reference = reference_pipeline.clone();
            scanner_reference.params.io.scan_film = true;
            scanner_reference.params.workflow.route = "input > film > scan".into();
            scanner_reference.params.io.output_cctf_encoding = false;
            let mut scanner_device = pipeline.clone();
            scanner_device.params.io.output_cctf_encoding = false;
            let physical_reference = scanner_reference
                .process_with_taps(
                    physical_density.clone(),
                    &spektrafilm_backend::cpu_backend::CpuBackend,
                    Some(crate::params::Tap::CmyFilm),
                    Some(crate::params::Tap::RgbOut),
                )
                .unwrap();
            let acquired = scanner_device
                .process_with_taps(
                    physical_density,
                    gpu.backend(),
                    Some(crate::params::Tap::CmyFilm),
                    Some(crate::params::Tap::RgbOut),
                )
                .unwrap();
            let (_, acquisition_max) = parity_metrics(&acquired, &physical_reference);
            assert!(
                acquisition_max < 1e-5,
                "identical physical-f32 scanner acquisition max absolute linear destination RGB error={acquisition_max}"
            );
        }
        let snapshot = observed_render(&pipeline, image.clone(), gpu.backend());
        assert_eq!(
            snapshot.execution.path,
            Some(spektrafilm_backend::telemetry::ExecutionPath::GpuResident)
        );
        assert!(
            !snapshot
                .observations
                .iter()
                .any(|stage| stage.executor == Some(spektrafilm_backend::telemetry::Executor::Cpu)),
            "{route} executed host image arithmetic: {:?}",
            snapshot.observations
        );
        outputs.push(output);
    }
    assert!(
        parity_metrics(&outputs[0], &outputs[1]).1 > 1e-5,
        "base toggle must affect acquisition"
    );
    assert!(
        parity_metrics(&outputs[0], &outputs[2]).1 > 1e-3,
        "printing must affect conversion"
    );
}

/// The enclosing Dye readout retains scanner context, then crops on device.
#[test]
fn explicit_gpu_deferred_crop_dye_route_matches_cpu() {
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
    params.scanner.unsharp_mask = [0.7, 0.7];
    params.film_render.grain.active = true;
    params.film_render.grain.engine = crate::params::grain::GrainEngine::Dye;
    // Field realization and the geometry-aware tail both stay on device.
    assert_eq!(
        crate::stages::grain_v3::effective_field_backend(&params, true),
        "gpu"
    );
    let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
    assert!(
        matches!(pipeline.resident_decision(), ResidentDecision::UseResident),
        "a deferred-crop Dye request keeps the resident route: {:?}",
        pipeline.resident_decision()
    );
    let img = spektrafilm_math::image::ImageBuf::from_data(4, 4, vec![0.5; 48]);
    let output = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(img.clone(), gpu.backend())
        .unwrap();
    let reference = pipeline
        .process(img, &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    assert_eq!(
        (output.width, output.height),
        (reference.width, reference.height)
    );
    let (mean, max) = parity_metrics(&output, &reference);
    assert!(
        mean < 5e-3 && max < 5e-2,
        "deferred crop: mean={mean} max={max}"
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

/// The scipy MT19937 regime is Spektra's default (`use_fast_stats` off) and
/// the composite path never leaves it, so the resident chain must run grain
/// on the device rather than fall back to the assembled host fence. The
/// device sampler reproduces the reference draw order (all Poisson first,
/// then all binomial) for both the layered and composite regimes, so the
/// whole stage resolves on the device and agrees with the CPU reference.
#[test]
fn spektra_mt19937_grain_runs_on_the_device() {
    use spektrafilm_math::precision::from_f64;

    for sublayers_active in [true, false] {
        let gpu = spektrafilm_gpu::test_support::adapter();
        let dir = data_dir();
        let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.io.output_color_space = "sRGB".into();
        params.io.output_cctf_encoding = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.camera.auto_exposure = false;
        params.settings.use_fast_stats = false;
        params.film_render.grain.active = true;
        params.film_render.grain.engine = crate::params::grain::GrainEngine::Spektra;
        params.film_render.grain.sublayers_active = sublayers_active;

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        let (w, h) = (16u32, 16u32);
        let data: Vec<_> = (0..w * h * 3)
            .map(|i| from_f64(0.05 + 0.5 * ((i * 37) % 256) as f64 / 255.0))
            .collect();
        let img = spektrafilm_math::image::ImageBuf::from_data(w, h, data.clone());

        let snapshot = observed_render(&pipeline, img.clone(), gpu.backend());
        assert!(
            !snapshot
                .observations
                .iter()
                .any(|stage| stage.name == "grain_v1"),
            "the MT19937 regime fell back to the host fence (sublayers_active={sublayers_active}): {:?}",
            snapshot
                .observations
                .iter()
                .map(|stage| stage.name.as_str())
                .collect::<Vec<_>>()
        );

        let out_gpu = pipeline
            .clone()
            .with_params_with_backend(pipeline.params.clone(), gpu.backend())
            .unwrap()
            .process(img.clone(), gpu.backend())
            .unwrap();
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
            "MT19937 grain mean divergence (sublayers_active={sublayers_active}): {mean}"
        );
        assert!(
            max_diff < 5e-1,
            "MT19937 grain max divergence (sublayers_active={sublayers_active}): {max_diff}"
        );
    }
}

/// Deferred crop and downstream scanner optics are resident device stages.
#[test]
fn deferred_crop_dye_with_downstream_optics_keeps_the_resident_route() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = dye_fence_params();
    params.io.crop = true;
    params.io.crop_size = [0.5, 0.5];
    params.scanner.unsharp_mask = [0.7, 0.7];
    assert_eq!(
        crate::stages::grain_v3::effective_field_backend(&params, true),
        "gpu"
    );
    let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
    assert!(
        matches!(pipeline.resident_decision(), ResidentDecision::UseResident),
        "a deferred-crop Dye request keeps the resident route: {:?}",
        pipeline.resident_decision()
    );
}

/// The experimental mass-law selector has no device count law. Both explicit
/// and automatically selected GPU requests reject it before host arithmetic;
/// CPU remains the supported executor. The selector is process-global, so
/// this case runs in a child process.
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
        "mass-law child case failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
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
            .clone()
            .with_params_with_backend(pipeline.params.clone(), gpu.backend())
            .unwrap()
            .process(image.clone(), gpu.backend())
            .expect_err("explicit GPU must name the host field");
        assert!(
            err.contains("selected GPU") && err.contains("film grain distribution"),
            "{err}"
        );
    }
    // Selection policy is independent of the unsupported count law: an
    // automatically selected GPU must also fail instead of entering a fence.
    let auto = spektrafilm_gpu::test_support::adapter();
    let err = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), auto.backend())
        .unwrap()
        .process(image, auto.backend())
        .expect_err("automatically selected GPU must name the host field");
    assert!(err.contains("film grain distribution"), "{err}");
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

    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(img.clone(), gpu.backend())
        .unwrap();
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

    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(img.clone(), gpu.backend())
        .unwrap();
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

    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(img.clone(), gpu.backend())
        .unwrap();
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

    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(img.clone(), gpu.backend())
        .unwrap();
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

    let out_gpu = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(img.clone(), gpu.backend())
        .unwrap();
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
    let gpu_out = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(image.clone(), gpu.backend())
        .unwrap();
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
    // Mallett no longer declines: its front pass is a device matmul like
    // hanatos2025's, and the display-domain grain and optics stages that
    // follow it are the same device nodes on either upsampler. The CPU
    // per-stage route applies the same Procedural grain, so the two agree.
    pipeline.params.settings.rgb_to_raw_method = "mallett2019".into();
    assert!(
        matches!(pipeline.resident_decision(), ResidentDecision::UseResident),
        "Mallett2019 must not decline the resident route"
    );
    pipeline.params.settings.rgb_to_raw_method = "hanatos2025".into();

    pipeline.params.film_render.grain.engine = crate::params::grain::GrainEngine::Spektra;
    // Spektra formation is a resident device stage.
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));

    // Dye keeps the resident route across crop/upscale geometry and retains
    // enclosing film context until downstream optics complete on device.
    pipeline.params.film_render.grain.engine = crate::params::grain::GrainEngine::Dye;
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));
    // Print glare is a spatial stage after the field, so a crop that keeps
    // film context around it defers the crop past the optics. With no such
    // stage, the field reads the full source and returns the requested crop
    // grid on its readout carrier. The default unsharp mask is active, so
    // this step clears it: the crop must not keep the whole-route fallback
    // on its own.
    pipeline.params.print_render.glare.active = false;
    pipeline.params.scanner.unsharp_mask = [0.0; 2];
    pipeline.params.io.crop = true;
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));
    // Scanner optics retain the enclosing field context and crop afterward.
    pipeline.params.scanner.unsharp_mask = [0.7, 0.7];
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));
    pipeline.params.scanner.unsharp_mask = [0.0; 2];
    pipeline.params.io.crop = false;
    pipeline.params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
    pipeline.params.film_render.grain.fotufilm.amount = 1.0;
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));
    pipeline.params.film_render.grain.fotufilm.amount = 0.0;
    assert!(matches!(
        pipeline.resident_decision(),
        ResidentDecision::UseResident
    ));
}

/// Params for the Mallett2019 resident parity check: the reference print
/// route with the Mallett front pass, the default display-domain Procedural
/// grain, and every device-heavy optional stage at its default.
fn mallett_parity_params() -> RuntimeParams {
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > print > scan".into();
    params.settings.rgb_to_raw_method = "mallett2019".into();
    params.camera.auto_exposure = false;
    params.film_render.grain.active = true;
    params.film_render.grain.engine = crate::params::grain::GrainEngine::Procedural;
    params.io.output_gamut_compress.algorithm = "off".into();
    params
}

/// An explicit GPU request with the Mallett2019 upsampler runs end to end on
/// the device: the front pass is a device matmul, and the display-domain
/// Procedural grain, halation, DIR, and viewing glare that follow it are the
/// same resident device nodes the hanatos2025 route uses.
///
/// This pins the route decision that the old `MallettExecutionParity`
/// decline blocked. That decline rested on an unmeasured claim that the
/// Mallett shader had "no numerically equivalent route through CPU
/// display-domain grain and optics": the CPU per-stage route had silently
/// dropped Procedural grain for Mallett (`stages/scanning.rs`'s
/// `grain_v2_active` carried a `rgb_to_raw_method != "mallett2019"`
/// conjunct from a merge resolution with no spec citation), so a resident
/// render would have applied grain the CPU reference did not. Removing the
/// CPU drop makes the two routes one algorithm, and the parity below is the
/// measured evidence.
#[test]
fn explicit_gpu_request_runs_mallett_on_the_resident_device_chain() {
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let pipeline = Pipeline::new_with_spectral(film, print, mallett_parity_params(), &dir).unwrap();
    assert!(
        matches!(pipeline.resident_decision(), ResidentDecision::UseResident),
        "mallett2019 must not decline the resident route"
    );
    // The Procedural grain must actually be active on both routes, so the
    // parity below is not a grain-free comparison that passes vacuously.
    let grain = crate::telemetry::RenderConfiguration::from_runtime(
        &pipeline.params,
        &pipeline.film,
        &pipeline.print,
    )
    .grain;
    assert!(
        grain.active,
        "the Mallett render's Procedural grain must be active for the parity to mean anything"
    );

    let image = comparison_image(16);
    let snapshot = observed_render(&pipeline, image, gpu.backend());
    assert_eq!(
        snapshot.execution.path,
        Some(spektrafilm_backend::telemetry::ExecutionPath::GpuResident),
        "backend {} did not keep the resident route for mallett2019",
        gpu.name()
    );
    assert!(
        !snapshot
            .observations
            .iter()
            .any(|stage| stage.name == "grain_v2"),
        "the explicit GPU Mallett render recorded a host grain stage: {:?}",
        snapshot
            .observations
            .iter()
            .map(|stage| stage.name.as_str())
            .collect::<Vec<_>>()
    );
}

/// Mallett2019 resident parity with the default display-domain Procedural
/// grain active: the device render must agree with the CPU reference under
/// the standard device budget, exactly as the hanatos2025 route does.
#[test]
fn mallett_resident_matches_cpu_per_stage() {
    let gpu = spektrafilm_gpu::test_support::adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let pipeline = Pipeline::new_with_spectral(film, print, mallett_parity_params(), &dir).unwrap();
    let image = comparison_image(16);
    let gpu_out = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(image.clone(), gpu.backend())
        .unwrap();
    let cpu_out = pipeline
        .process(image, &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    let (mean, max_diff) = parity_metrics(&gpu_out, &cpu_out);
    assert!(
        mean < 5e-3 && max_diff < 5e-2,
        "mallett resident parity: mean={mean:.6} max={max_diff:.6}"
    );
}

/// The JzAzBz output gamut compression runs inside the resident chain on
/// the device, not as a per-stage host hook. The pass executes the
/// perceptual transform in binary64, so the same render must agree with
/// the CPU reference under the standard device budget — and the request
/// must be resident, not demoted to the per-stage route by a missing
/// device path.
#[test]
fn jzazbz_output_gamut_resident_matches_cpu() {
    let gpu = spektrafilm_gpu::test_support::adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_ektachrome_100").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = resident_parity_params(16);
    params.io.output_gamut_compress.algorithm = "jzazbz".into();
    let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
    // jzazbz has a device path, so the request stays resident.
    assert!(
        matches!(pipeline.resident_decision(), ResidentDecision::UseResident),
        "jzazbz has a device path and must not demote the resident route"
    );
    // The algorithm must be observable: with it off, the same image
    // compresses differently, so the parity below is not vacuous.
    let mut base_params = resident_parity_params(16);
    base_params.io.output_gamut_compress.algorithm = "off".into();
    let baseline = Pipeline::new_with_spectral(
        crate::profile::load_profile_by_name(&dir, "kodak_ektachrome_100").unwrap(),
        crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap(),
        base_params,
        &dir,
    )
    .unwrap();
    let image = comparison_image(16);
    let gpu_out = pipeline
        .clone()
        .with_params_with_backend(pipeline.params.clone(), gpu.backend())
        .unwrap()
        .process(image.clone(), gpu.backend())
        .unwrap();
    let cpu_out = pipeline
        .process(image.clone(), &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    let cpu_base = baseline
        .process(image, &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    let (effect, _) = parity_metrics(&cpu_out, &cpu_base);
    assert!(
        effect > 1e-3,
        "jzazbz compression had no effect: {effect:.6}"
    );
    let (mean, max_diff) = parity_metrics(&gpu_out, &cpu_out);
    assert!(
        mean < 5e-2 && max_diff < 5e-1,
        "jzazbz resident parity: mean={mean:.6} max={max_diff:.6}"
    );
}

/// The passthrough still has a processed branch (CMYK), and must use the
/// same final linear-light mix as film routes, in either output transport.
#[test]
fn retained_input_output_mix_matches_cpu_with_cmyk() {
    use spektrafilm_backend::cpu_backend::CpuBackend;
    use spektrafilm_math::colorspace::{cctf_decode, cctf_encode, resolve};
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let image = comparison_image(8);
    for encoded in [false, true] {
        let mut params = resident_parity_params(8);
        params.random_seed = 37;
        params.workflow.route = "input".into();
        params.io.input_color_space = "Display P3".into();
        params.io.input_cctf_decoding = true;
        params.io.output_color_space = "sRGB".into();
        params.io.output_cctf_encoding = encoded;
        params.io.crop = true;
        params.io.crop_size = [0.5, 0.75];
        params.io.upscale_factor = 1.5;
        params.cmyk.active = true;
        params.cmyk.strength = 1.0;
        let cpu =
            Pipeline::new_with_spectral(film.clone(), print.clone(), params.clone(), &dir).unwrap();
        let device = cpu
            .clone()
            .with_params_with_backend(params, gpu.backend())
            .unwrap();
        let full_cpu = cpu.process(image.clone(), &CpuBackend).unwrap();
        let source_cpu = cpu.prepare_output_mix_source(&image, &CpuBackend).unwrap();
        let space = resolve("sRGB").unwrap();
        let linear = |v: f64| {
            if encoded {
                cctf_decode(v, space.cctf)
            } else {
                v
            }
        };
        assert!(
            full_cpu
                .data
                .iter()
                .zip(&source_cpu.data)
                .any(|(a, b)| (linear(*a as f64) - *b as f64).abs() > 1e-3),
            "CMYK must distinguish the processed branch from the source"
        );
        for amount in [0.0, 0.35, 1.0] {
            let mut cpu = cpu.clone();
            cpu.params.output_mix.amount = amount;
            let mut device = device.clone();
            device.params.output_mix.amount = amount;
            let expected = cpu.process(image.clone(), &CpuBackend).unwrap();
            let actual = device
                .process_retained(&image, crate::pipeline::RetainedRender::FULL, gpu.backend())
                .unwrap()
                .unwrap()
                .to_host(gpu.backend())
                .unwrap();
            assert_eq!((actual.width, actual.height), (6, 9));
            assert_eq!(actual.data.len(), expected.data.len());
            for (index, ((actual, expected), (source, full))) in actual
                .data
                .iter()
                .zip(&expected.data)
                .zip(source_cpu.data.iter().zip(&full_cpu.data))
                .enumerate()
            {
                let mixed = (1.0 - amount) * *source as f64 + amount * linear(*full as f64);
                let mixed = if encoded {
                    cctf_encode(mixed, space.cctf)
                } else {
                    mixed
                };
                assert!(
                    (*expected as f64 - mixed).abs() < 3e-6,
                    "CPU mix sample {index}"
                );
                // Existing retained input color-conversion sample budget.
                assert!(
                    (*actual as f64 - *expected as f64).abs() < 2e-5,
                    "encoded={encoded} amount={amount} sample={index}: GPU {actual}, CPU {expected}"
                );
            }
        }
    }
}

/// The optical readout changes the result lattice, not the conditioning
/// source. Its mix source must independently replay all delivered geometry.
#[test]
fn retained_optical_grain_output_mix_replays_complete_geometry() {
    use crate::params::GrainEngine;
    use spektrafilm_backend::cpu_backend::CpuBackend;
    use spektrafilm_math::precision::from_f64;
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let image = ImageBuf::from_data(
        16,
        12,
        (0..16 * 12 * 3)
            .map(|i| from_f64(0.05 + 0.8 * ((i * 37) % 101) as f64 / 100.0))
            .collect(),
    );
    for engine in [GrainEngine::Dye, GrainEngine::Fotufilm] {
        let mut params = dye_fence_params();
        params.random_seed = 37;
        params.film_render.grain.engine = engine;
        params.camera.film_format_mm = 0.16;
        params.film_render.grain.fotufilm.amount = 1.0;
        params.io.input_cctf_decoding = true;
        params.io.crop = true;
        params.io.crop_center = [0.6, 0.4];
        params.io.crop_size = [0.5, 0.5];
        params.io.upscale_factor = 1.5;
        // Isolate optical grain readout and its enclosing scanner field from
        // unrelated full-source camera diffusion and viewing glare.
        params.film_render.halation.active = false;
        params.film_render.dir_couplers.active = false;
        params.print_render.glare.active = false;
        params.scanner.lens_blur = 0.6;
        params.scanner.unsharp_mask = [0.0, 0.0];
        params.enlarger.diffusion_filter.active = false;
        let cpu =
            Pipeline::new_with_spectral(film.clone(), print.clone(), params.clone(), &dir).unwrap();
        let device = cpu
            .clone()
            .with_params_with_backend(params, gpu.backend())
            .unwrap();
        // Clones retain the same immutable fitted population and bank for
        // full and preview geometry; raster size is not population identity.
        for preview in [None, Some(12)] {
            let oriented = image.rotated_quarter_turns(1);
            let working = preview.map_or_else(
                || oriented.clone(),
                |max| crate::params_builder::resize_for_preview(&oriented, max),
            );
            let source = cpu
                .prepare_output_mix_source(&working, &CpuBackend)
                .unwrap();
            let full_cpu = cpu.process(working.clone(), &CpuBackend).unwrap();
            let request = crate::pipeline::RetainedRender {
                rotation_quarter_turns: 1,
                preview_max_size: preview,
            };
            let full_gpu = device
                .process_retained(&image, request, gpu.backend())
                .unwrap()
                .unwrap()
                .to_host(gpu.backend())
                .unwrap();
            assert_eq!(
                (full_gpu.width, full_gpu.height),
                (source.width, source.height)
            );
            for amount in [0.0, 0.35] {
                let mut cpu = cpu.clone();
                cpu.params.output_mix.amount = amount;
                let mut device = device.clone();
                device.params.output_mix.amount = amount;
                let expected = cpu.process(working.clone(), &CpuBackend).unwrap();
                let actual = device
                    .process_retained(&image, request, gpu.backend())
                    .unwrap()
                    .unwrap()
                    .to_host(gpu.backend())
                    .unwrap();
                assert_eq!(
                    (actual.width, actual.height),
                    (expected.width, expected.height)
                );
                assert_eq!(actual.data.len(), source.data.len());
                for (index, ((actual, source), (gpu_full, cpu_full))) in actual
                    .data
                    .iter()
                    .zip(&source.data)
                    .zip(full_gpu.data.iter().zip(&full_cpu.data))
                    .enumerate()
                {
                    let mixed = (1.0 - amount) * *source as f64 + amount * *gpu_full as f64;
                    assert!(
                        (*actual as f64 - mixed).abs() < 2e-5,
                        "{engine:?} preview={preview:?} amount={amount} sample={index}: geometry/mix {actual} != {mixed}"
                    );
                    let cpu_mixed = (1.0 - amount) * *source as f64 + amount * *cpu_full as f64;
                    assert!((expected.data[index] as f64 - cpu_mixed).abs() < 3e-6);
                }
                let (mean, max) = parity_metrics(&actual, &expected);
                if engine == GrainEngine::Dye && amount != 0.0 {
                    // CPU FastStatsRng uses SplitMix64/53-bit uniforms;
                    // production dye_count.wgsl uses coordinate-mixed 32-bit
                    // words/24-bit uniforms. Equal seeds do not pair draws.
                    // specs/gpu-execution/design.md §Stochastic alignment
                    // explicitly permits Dye's independent random words;
                    // its law/formation certification remains separate.
                    // This is a measurement, not numerical Dye acceptance.
                    eprintln!(
                        "unpaired Dye realization measurement preview={preview:?} amount={amount}: mean={mean}, max={max}; not an acceptance gate"
                    );
                } else {
                    assert!(
                        mean < 5e-3 && max < 5e-2,
                        "{engine:?} preview={preview:?} amount={amount}: mean={mean}, max={max}"
                    );
                }
            }
        }
    }
}

/// Inverse scan already recovers developed CMY. Fotufilm selection is inert
/// there; active Dye rejects these routes before rendering on both executors.
#[test]
fn retained_convert_film_never_generates_dye_or_fotufilm_grain() {
    use crate::params::GrainEngine;
    use spektrafilm_backend::cpu_backend::CpuBackend;
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let image = n2p_comparison_image(8);
    for route in [
        "input > convert-film > scan",
        "input > convert-film > scan-minus-base",
        "input > convert-film > print > scan",
    ] {
        for engine in [GrainEngine::Dye, GrainEngine::Fotufilm] {
            let mut params = resident_parity_params(8);
            params.random_seed = 37;
            params.workflow.route = route.into();
            params.film_render.grain.engine = engine;
            params.film_render.grain.active = false;
            params.film_render.grain.fotufilm.amount = 1.0;
            params.film_render.grain.v3_dye_support_um = 8.0;
            params.scanner.scan_output = "direct_scan".into();
            params.scanner.lens_blur = 0.0;
            params.scanner.unsharp_mask = [0.0, 0.0];
            params.enlarger.diffusion_filter.active = false;
            params.io.crop = true;
            params.io.crop_center = [0.65, 0.35];
            params.io.crop_size = [0.5, 0.75];
            params.io.upscale_factor = 1.5;
            let cpu =
                Pipeline::new_with_spectral(film.clone(), print.clone(), params.clone(), &dir)
                    .unwrap();
            let device = cpu
                .clone()
                .with_params_with_backend(params.clone(), gpu.backend())
                .unwrap();
            let cpu_off = cpu.process(image.clone(), &CpuBackend).unwrap();
            let gpu_off = device
                .process_retained(&image, crate::pipeline::RetainedRender::FULL, gpu.backend())
                .unwrap()
                .unwrap()
                .to_host(gpu.backend())
                .unwrap();
            assert_eq!((cpu_off.width, cpu_off.height), (6, 9));
            assert_eq!((gpu_off.width, gpu_off.height), (6, 9));
            assert_eq!(gpu_off.data.len(), cpu_off.data.len());
            let (mean, max) = parity_metrics(&gpu_off, &cpu_off);
            assert!(
                mean < 5e-3 && max < 5e-2,
                "inactive {route} {engine:?}: mean={mean}, max={max}"
            );
            params.film_render.grain.active = true;
            let cpu = cpu.with_params(params.clone()).unwrap();
            let device = device
                .with_params_with_backend(params, gpu.backend())
                .unwrap();
            if engine == GrainEngine::Dye {
                let expected = crate::stages::grain_v3::validate_supported(&cpu.film, &cpu.params)
                    .unwrap_err();
                assert!(
                    expected.contains("film grain Dye: unsupported condition"),
                    "{expected}"
                );
                assert_eq!(
                    cpu.process(image.clone(), &CpuBackend).unwrap_err(),
                    expected
                );
                let context = spektrafilm_backend::telemetry::ObservationContext::new(
                    spektrafilm_backend::telemetry::CollectionMode::Summary,
                );
                let observed = spektrafilm_backend::bind_backend(gpu.backend(), context.clone());
                assert_eq!(
                    device
                        .process_retained(
                            &image,
                            crate::pipeline::RetainedRender::FULL,
                            observed.as_ref()
                        )
                        .unwrap_err(),
                    expected
                );
                assert!(
                    context.snapshot().observations.is_empty(),
                    "unsupported Dye must fail before device work or host fallback"
                );
                continue;
            }
            let cpu_on = cpu.process(image.clone(), &CpuBackend).unwrap();
            let gpu_on = device
                .process_retained(&image, crate::pipeline::RetainedRender::FULL, gpu.backend())
                .unwrap()
                .unwrap()
                .to_host(gpu.backend())
                .unwrap();
            assert_eq!((cpu_on.width, cpu_on.height), (6, 9));
            assert_eq!((gpu_on.width, gpu_on.height), (6, 9));
            assert_eq!(
                cpu_on.data, cpu_off.data,
                "CPU {route} {engine:?} redeveloped density"
            );
            assert_eq!((gpu_on.width, gpu_on.height), (cpu_on.width, cpu_on.height));
            assert_eq!(
                gpu_on.data, gpu_off.data,
                "GPU {route} {engine:?} redeveloped density"
            );
            assert_eq!(gpu_on.data.len(), cpu_on.data.len());
            let (mean, max) = parity_metrics(&gpu_on, &cpu_on);
            assert!(
                mean < 5e-3 && max < 5e-2,
                "{route} {engine:?}: mean={mean}, max={max}"
            );
        }
    }
}

#[test]
fn retained_inactive_optical_grain_keeps_ordinary_crop_and_rescale() {
    use crate::params::GrainEngine;
    use spektrafilm_backend::cpu_backend::CpuBackend;
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let image = comparison_image(16);
    for engine in [GrainEngine::Dye, GrainEngine::Fotufilm] {
        for scale in [0.5, 1.5] {
            let mut params = resident_parity_params(16);
            params.random_seed = 37;
            params.camera.auto_exposure = false;
            params.film_render.grain.engine = engine;
            params.film_render.grain.active = true;
            params.debug.deactivate_spatial_effects = engine == GrainEngine::Dye;
            params.film_render.grain.fotufilm.amount = 0.0;
            params.io.crop = true;
            params.io.crop_size = [0.5, 0.25];
            params.io.upscale_factor = scale;
            let cpu =
                Pipeline::new_with_spectral(film.clone(), print.clone(), params.clone(), &dir)
                    .unwrap();
            let device = cpu
                .clone()
                .with_params_with_backend(params, gpu.backend())
                .unwrap();
            let expected = cpu.process(image.clone(), &CpuBackend).unwrap();
            let actual = device
                .process_retained(&image, crate::pipeline::RetainedRender::FULL, gpu.backend())
                .unwrap()
                .unwrap()
                .to_host(gpu.backend())
                .unwrap();
            assert_eq!(
                (actual.width, actual.height),
                ((8.0 * scale) as u32, (4.0 * scale) as u32)
            );
            assert_eq!(
                (actual.width, actual.height),
                (expected.width, expected.height)
            );
            assert_eq!(actual.data.len(), expected.data.len());
            let (mean, max) = parity_metrics(&actual, &expected);
            assert!(
                mean < 5e-3 && max < 5e-2,
                "{engine:?} scale={scale}: mean={mean}, max={max}"
            );
        }
    }
}

/// Compare identical developed print density so this regression owns glare
/// parameter/seed composition, not front-end film or print conditioning.
#[test]
fn resident_print_glare_preserves_the_reference_seed_and_stream() {
    let gpu = spektrafilm_gpu::test_support::adapter();
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_gold_200").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_supra_endura").unwrap();
    let mut params = resident_parity_params(19);
    params.workflow.route = "input > film > print > scan".into();
    params.io.scan_film = false;
    params.settings.preview_mode = false;
    params.camera.auto_exposure = false;
    params.scanner.unsharp_mask = [0.0, 0.0];
    params.print_render.glare.active = true;
    params.print_render.glare.percent = 0.1;
    params.print_render.glare.roughness = 0.5;
    params.print_render.glare.blur = 0.8;
    let mut pipeline =
        Pipeline::new_with_spectral_backend(film, print, params, &dir, gpu.backend()).unwrap();
    let image = comparison_image(19);
    let render = |pipeline: &Pipeline, backend: &dyn spektrafilm_backend::ComputeBackend| {
        pipeline
            .process_with_taps(
                image.clone(),
                backend,
                Some(crate::params::Tap::CmyPrint),
                Some(crate::params::Tap::RgbOut),
            )
            .unwrap()
    };
    for seed in [0, 42, u64::from(u32::MAX), (1u64 << 32) + 42] {
        pipeline.params.random_seed = seed;
        let cpu = render(&pipeline, &spektrafilm_backend::cpu_backend::CpuBackend);
        let device = render(&pipeline, gpu.backend());
        let (_, maximum) = parity_metrics(&device, &cpu);
        assert!(
            maximum < 1e-5,
            "print glare seed={seed}: linear RGB max={maximum}"
        );
        let mut off = pipeline.clone();
        off.params.print_render.glare.active = false;
        let reference_off = render(&off, &spektrafilm_backend::cpu_backend::CpuBackend);
        assert!(
            parity_metrics(&cpu, &reference_off).1 > 1e-4,
            "glare must be observable"
        );
        eprintln!("integrated print glare seed={seed}: linear RGB max={maximum:.9e}");
    }
}
