use super::*;

#[test]
fn glare_matches_the_reference_normal_stream() {
    let backend = crate::test_support::adapter();
    let width = 37;
    let height = 19;
    for seed in [0, 42, u32::MAX] {
        for roughness in [0.0, 1e-7, 9e-7, 1.1e-6, 0.5, 2.0] {
            for blur in [0.0, 0.8, 3.5] {
                let (mu, sigma) = spektrafilm_model::glare::lognormal_params(0.1, roughness);
                let gp = spektrafilm_backend::GlareGpuParams {
                    mu: mu as f32,
                    sigma: sigma as f32,
                    blur_px: blur,
                    base_seed: seed,
                    rgb_offset: [0.01; 3],
                };
                let image = backend
                    .allocate_image_pages(width, height, "glare_reference_test")
                    .unwrap();
                let mut scratch = GaussianScratch::new(width, height);
                let state = build_glare_state(
                    &backend.device,
                    &gp,
                    width,
                    height,
                    &image,
                    &mut scratch,
                    &backend,
                )
                .unwrap();
                state.submit(&backend).unwrap();
                let expected = spektrafilm_model::glare::compute_random_glare_amount(
                    width,
                    height,
                    0.1,
                    roughness,
                    blur,
                    u64::from(seed),
                );
                let mut maximum = 0.0_f64;
                for page in &image {
                    let actual = backend
                        .pipe
                        .readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12);
                    for (pixel, channels) in actual.as_chunks::<3>().0.iter().enumerate() {
                        let reference = expected[page.first_pixel as usize + pixel] as f64;
                        for &sample in channels {
                            assert!(sample.is_finite());
                            maximum = maximum.max((f64::from(sample) - reference).abs());
                        }
                    }
                }
                eprintln!("glare seed={seed} roughness={roughness} blur={blur}: max={maximum:.9e}");
                assert!(maximum < 1e-8, "reference glare stream mismatch: {maximum}");
            }
        }
    }
}

struct RestoreLimit(Option<std::ffi::OsString>);
impl Drop for RestoreLimit {
    fn drop(&mut self) {
        // The backend guard serializes every device test in this process.
        unsafe {
            match &self.0 {
                Some(value) => std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", value),
                None => std::env::remove_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"),
            }
        }
    }
}

#[test]
fn glare_stream_survives_odd_chunks_pages_and_repeat_submission() {
    let backend = crate::test_support::adapter();
    let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
    let width = 8193;
    let (mu, sigma) = spektrafilm_model::glare::lognormal_params(0.1, 0.5);
    let gp = spektrafilm_backend::GlareGpuParams {
        mu: mu as f32,
        sigma: sigma as f32,
        blur_px: 0.0,
        base_seed: 42,
        rgb_offset: [0.01; 3],
    };
    let expected =
        spektrafilm_model::glare::compute_random_glare_amount(width, 1, 0.1, 0.5, 0.0, 42);
    let mut previous = None;
    for limit in [65536, 2532, 2520, 2516] {
        unsafe {
            std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", limit.to_string());
        }
        let image = backend
            .allocate_image_pages(width, 1, "glare_chunk_test")
            .unwrap();
        let mut scratch = GaussianScratch::new(width, 1);
        let state = build_glare_state(
            &backend.device,
            &gp,
            width,
            1,
            &image,
            &mut scratch,
            &backend,
        )
        .unwrap();
        for repeat in 0..2 {
            for page in &image {
                backend
                    .queue
                    .write_buffer(&page.buffer, 0, &vec![0; page.n_pixels as usize * 12]);
            }
            state.submit(&backend).unwrap();
            let actual: Vec<f32> = image
                .iter()
                .flat_map(|page| {
                    backend
                        .pipe
                        .readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12)
                })
                .collect();
            let maximum = actual
                .as_chunks::<3>()
                .0
                .iter()
                .zip(&expected)
                .map(|(pixel, value)| (f64::from(pixel[0]) - *value as f64).abs())
                .fold(0.0, f64::max);
            assert!(maximum < 1e-8, "chunk reference error {maximum}");
            if let Some(previous) = &previous {
                assert_eq!(&actual, previous, "chunking or replay shifted the stream");
            }
            eprintln!(
                "glare limit={limit} repeat={repeat}: reference max={maximum:.9e}, exact replay/chunk alignment"
            );
            previous = Some(actual);
        }
    }
    unsafe {
        std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "2515");
    }
    let image = backend
        .allocate_image_pages(1, 1, "glare_limit_test")
        .unwrap();
    let mut scratch = GaussianScratch::new(1, 1);
    match build_glare_state(&backend.device, &gp, 1, 1, &image, &mut scratch, &backend) {
        Ok(_) => panic!("undersized generator binding must fail explicitly"),
        Err(error) => assert!(
            error.contains("glare MT19937 state needs 2516 bytes"),
            "{error}"
        ),
    }
}

#[test]
fn glare_cancellation_stops_submission_and_allows_reference_replay() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let gpu = crate::test_support::adapter();
    let flag = Arc::new(AtomicBool::new(false));
    let mut backend = gpu.derived_explicit();
    backend.pipe = backend.pipe.with_cancellation(Some(flag.clone()));
    backend.cancellation = Some(flag.clone());
    let (mu, sigma) = spektrafilm_model::glare::lognormal_params(0.1, 0.5);
    let gp = spektrafilm_backend::GlareGpuParams {
        mu: mu as f32,
        sigma: sigma as f32,
        blur_px: 0.0,
        base_seed: 42,
        rgb_offset: [0.01; 3],
    };
    let image = backend
        .allocate_image_pages(8193, 1, "glare_cancellation_test")
        .unwrap();
    let mut scratch = GaussianScratch::new(8193, 1);
    let state = build_glare_state(
        &backend.device,
        &gp,
        8193,
        1,
        &image,
        &mut scratch,
        &backend,
    )
    .unwrap();
    flag.store(true, Ordering::Relaxed);
    assert_eq!(state.submit(&backend).unwrap_err(), "operation cancelled");
    assert!(
        build_glare_state(
            &backend.device,
            &gp,
            8193,
            1,
            &image,
            &mut scratch,
            &backend
        )
        .is_err()
    );
    flag.store(false, Ordering::Relaxed);
    state.submit(&backend).unwrap();
    let expected =
        spektrafilm_model::glare::compute_random_glare_amount(8193, 1, 0.1, 0.5, 0.0, 42);
    for page in &image {
        let actual = backend
            .pipe
            .readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12);
        for (index, pixel) in actual.as_chunks::<3>().0.iter().enumerate() {
            let reference = expected[page.first_pixel as usize + index] as f64;
            assert!(
                pixel
                    .iter()
                    .all(|&sample| (f64::from(sample) - reference).abs() < 1e-8)
            );
        }
    }
}

#[test]
fn glare_large_fields_keep_reference_error_bounded() {
    let backend = crate::test_support::adapter();
    let width = 1024;
    let height = 1024;
    for seed in [0, 42, 1729, u32::MAX] {
        let (mu, sigma) = spektrafilm_model::glare::lognormal_params(0.1, 0.5);
        let gp = spektrafilm_backend::GlareGpuParams {
            mu: mu as f32,
            sigma: sigma as f32,
            blur_px: 0.0,
            base_seed: seed,
            rgb_offset: [0.01; 3],
        };
        let image = backend
            .allocate_image_pages(width, height, "glare_large_test")
            .unwrap();
        let mut scratch = GaussianScratch::new(width, height);
        let state = build_glare_state(
            &backend.device,
            &gp,
            width,
            height,
            &image,
            &mut scratch,
            &backend,
        )
        .unwrap();
        state.submit(&backend).unwrap();
        let expected = spektrafilm_model::glare::compute_random_glare_amount(
            width,
            height,
            0.1,
            0.5,
            0.0,
            u64::from(seed),
        );
        let mut maximum = 0.0_f64;
        let mut first_over = None;
        let mut samples = Vec::with_capacity((width * height) as usize);
        for page in &image {
            let actual = backend
                .pipe
                .readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12);
            for (pixel, channels) in actual.as_chunks::<3>().0.iter().enumerate() {
                let index = page.first_pixel as usize + pixel;
                let error = (f64::from(channels[0]) - expected[index] as f64).abs();
                assert!(channels.iter().all(|v| v.is_finite()));
                maximum = maximum.max(error);
                samples.push(f64::from(channels[0]));
                if error >= 1e-8 && first_over.is_none() {
                    first_over = Some((index, error, channels[0], expected[index] as f64));
                }
            }
        }
        eprintln!(
            "large glare seed={seed} count={}: max={maximum:.9e} first_over={first_over:?}",
            width * height
        );
        assert!(maximum < 1e-8, "large glare error {maximum} seed={seed}");
        let n = samples.len() as f64;
        let mean = samples.iter().sum::<f64>() / n;
        let variance = samples.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        let lag = samples
            .windows(2)
            .map(|p| (p[0] - mean) * (p[1] - mean))
            .sum::<f64>()
            / (n - 1.0)
            / variance;
        // Analytic lognormal moments, independent of stream alignment.
        let target_mean = f64::from(0.1f32) / 100.0;
        let target_variance = (target_mean * 0.5).powi(2);
        assert!((mean / target_mean - 1.0).abs() < 0.005);
        assert!((variance / target_variance - 1.0).abs() < 0.02);
        assert!(lag.abs() < 0.01);
        // A mean-preserving constant-noise replacement fails the same variance check.
        assert!((0.0 / target_variance - 1.0_f64).abs() >= 0.02);
        eprintln!(
            "glare moments seed={seed}: mean={mean:.12e} variance={variance:.12e} lag1={lag:.9e}"
        );
    }
}

#[test]
fn glare_without_native_f64_runs_the_generator_and_rejects_the_gaussian() {
    // The MT19937 normal transform stays f32-only; the Gaussian owns binary64
    // coefficients and state, so a blurred glare request fails explicitly.
    // Acquire the common device guard before opening a second, restricted device.
    let gpu = crate::test_support::adapter();
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..Default::default()
    });
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("restricted test adapter");
    let (raw_device, raw_queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("glare_f32_only_test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
        },
        None,
    ))
    .expect("f32-only test device");
    assert!(
        !raw_device
            .features()
            .intersects(wgpu::Features::SHADER_F64 | wgpu::Features::SHADER_INT64)
    );
    let resources = std::sync::Arc::new(crate::wgpu_backend::resources::DeviceResources::new(
        raw_device.clone(),
        false,
    ));
    let mut backend = gpu.derived_explicit();
    backend.device = ObservedDevice {
        raw: raw_device,
        resources: resources.clone(),
        operation: None,
        context: Default::default(),
        batch: None,
    };
    backend.queue = ObservedQueue {
        raw: raw_queue,
        resources,
        operation: None,
        context: Default::default(),
    };
    backend.pipeline_cache = std::sync::Arc::new(PipelineCache::default());
    backend.pipe = GpuPipe::new(
        backend.device.clone(),
        backend.queue.clone(),
        backend.pipeline_cache.clone(),
    );
    let (mu, sigma) = spektrafilm_model::glare::lognormal_params(0.1, 0.5);
    for blur in [0.0, 0.8, 3.5] {
        let gp = spektrafilm_backend::GlareGpuParams {
            mu: mu as f32,
            sigma: sigma as f32,
            blur_px: blur,
            base_seed: 42,
            rgb_offset: [0.01; 3],
        };
        let image = backend
            .allocate_image_pages(37, 19, "glare_f32_only_image")
            .unwrap();
        let mut scratch = GaussianScratch::new(37, 19);
        let state = build_glare_state(&backend.device, &gp, 37, 19, &image, &mut scratch, &backend);
        if blur > 0.0 {
            // The Gaussian owns binary64 coefficients and state; a device without
            // native f64 fails explicitly rather than approximating the law.
            let Err(error) = state else {
                panic!("blurred glare must not run without native f64");
            };
            assert_eq!(
                error,
                "glare Gaussian: grain Gaussian requires native shader f64 arithmetic"
            );
            eprintln!(
                "f32-only glare features={:?} blur={blur} rejected",
                backend.device.features()
            );
            continue;
        }
        let state = state.unwrap();
        state.submit(&backend).unwrap();
        let expected =
            spektrafilm_model::glare::compute_random_glare_amount(37, 19, 0.1, 0.5, blur, 42);
        let mut maximum = 0.0_f64;
        for page in &image {
            let actual = backend
                .pipe
                .readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12);
            for (pixel, channels) in actual.as_chunks::<3>().0.iter().enumerate() {
                for &sample in channels {
                    assert!(sample.is_finite());
                    maximum = maximum.max(
                        (f64::from(sample) - expected[page.first_pixel as usize + pixel] as f64)
                            .abs(),
                    );
                }
            }
        }
        assert!(maximum < 1e-8, "restricted device glare error {maximum}");
        eprintln!(
            "f32-only glare features={:?} blur={blur} max={maximum:.9e}",
            backend.device.features()
        );
    }
}
