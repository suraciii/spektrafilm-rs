use super::*;
use crate::test_support::data_dir;

#[test]
fn film_chemistry_rebuilds_total_and_grain_layers_from_the_fitted_model() {
    let mut film = crate::profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
    let fitted = film.data.density_curves.clone();
    let fitted_layers = film.data.density_curves_layers.clone();
    for active in [false, true] {
        film.data
            .density_curves
            .iter_mut()
            .flatten()
            .for_each(|v| *v = 0.5);
        film.data
            .density_curves_layers
            .iter_mut()
            .flatten()
            .flatten()
            .for_each(|v| *v = 0.1);
        apply_film_chemistry(
            &mut film,
            &crate::params::PrintCurvesMorphParams {
                active,
                ..Default::default()
            },
        )
        .unwrap();
        for (got, want) in film
            .data
            .density_curves
            .iter()
            .flatten()
            .zip(fitted.iter().flatten())
        {
            assert!((got - want).abs() < 1e-12);
        }
        for (got, want) in film
            .data
            .density_curves_layers
            .iter()
            .flatten()
            .flatten()
            .zip(fitted_layers.iter().flatten().flatten())
        {
            assert!((got - want).abs() < 1e-12);
        }
    }
    apply_film_chemistry(
        &mut film,
        &crate::params::PrintCurvesMorphParams {
            gamma_factor: 1.1,
            gamma_factor_fast: 0.9,
            gamma_factor_slow: 1.2,
            developer_exhaustion: 0.3,
            ..Default::default()
        },
    )
    .unwrap();
    // Pinned 28bf883e apply_print_curves_morph_with_layers, Portra 400 sample 128.
    let expected = [
        [0.5911804126249434, 0.5581332496586162, 0.6521190325040326],
        [0.4509958575813556, 0.4580461526112701, 0.5776553146563388],
        [
            0.017750303385221305,
            0.026894456204068887,
            0.034051265456119285,
        ],
    ];
    for channel in 0..3 {
        for (layer, expected_layer) in expected.iter().enumerate() {
            assert!(
                (film.data.density_curves_layers[128][layer][channel] - expected_layer[channel])
                    .abs()
                    < 1e-9
            );
        }
        let sum: f64 = expected.iter().map(|layer| layer[channel]).sum();
        assert!((film.data.density_curves[128][channel] - sum).abs() < 1e-9);
    }
}

/// Keep one prepared fixture per test process. `Pipeline::clone` shares the
/// immutable Fotufilm preparation through `Prepared`'s `Arc`; each caller
/// still receives an independent parameter set, while repeated render tests
/// avoid rebuilding the same population and bank.
fn build() -> Pipeline {
    static SHARED: std::sync::OnceLock<Pipeline> = std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| {
            let dir = data_dir();
            let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
            let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
            Pipeline::new_with_spectral(film, print, RuntimeParams::default(), &dir).unwrap()
        })
        .clone()
}

#[test]
fn input_route_preserves_requested_crop_pixels_without_exposure() {
    let mut pipeline = build();
    pipeline.params.workflow.route = "input".into();
    pipeline.params.film_render.grain.active = false;
    pipeline.params.cmyk.active = false;
    pipeline.params.io.input_color_space = "ACES2065-1".into();
    pipeline.params.io.output_color_space = "ACES2065-1".into();
    pipeline.params.io.input_cctf_decoding = false;
    pipeline.params.io.output_cctf_encoding = false;
    pipeline.params.camera.auto_exposure = true;
    pipeline.params.camera.exposure_compensation_ev = 4.0;
    pipeline.params.io.crop = true;
    pipeline.params.io.crop_center = [0.5, 0.5];
    pipeline.params.io.crop_size = [0.5, 0.25];
    let image = ImageBuf::from_data(
        8,
        6,
        (0..8 * 6 * 3)
            .map(|i| from_f64(i as f64 / 100.0 - 0.1))
            .collect(),
    );
    let output = pipeline
        .process(image.clone(), &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    assert_eq!((output.width, output.height), (4, 2));
    for y in 0..2_usize {
        for x in 0..4_usize {
            let source = ((y + 2) * 8 + x + 2) * 3;
            let result = (y * 4 + x) * 3;
            for channel in 0..3 {
                assert!(
                    (to_f64(output.data[result + channel]) - to_f64(image.data[source + channel]))
                        .abs()
                        < 1e-6
                );
            }
        }
    }
    pipeline.params.io.upscale_factor = 2.0;
    let output = pipeline
        .process(image, &spektrafilm_backend::cpu_backend::CpuBackend)
        .unwrap();
    assert_eq!((output.width, output.height), (8, 4));
}
#[test]
fn digital_reference_preserves_formed_negative_and_prepared_identity() {
    let mut baseline = build();
    baseline.params.workflow.route = "input > film > scan".into();
    baseline.params.io.scan_film = true;
    baseline.params.film_render.grain.active = false;
    baseline.params.film_render.halation.active = false;
    baseline.params.film_render.dir_couplers.active = false;
    baseline.params.camera.auto_exposure = false;
    let mut params = baseline.params.clone();
    params.scanner.scan_output = "negative_to_positive".into();
    params.scanner.receiver_profile = crate::params::ReceiverProfile::DigitalReference;
    params.scanner.negative_conversion_method =
        crate::params::NegativeConversionMethod::DigitalReference;
    let reference = baseline.clone().with_params(params).unwrap();
    let image = ImageBuf::from_data(3, 1, vec![from_f64(0.18); 9]);
    let cpu = spektrafilm_backend::cpu_backend::CpuBackend;
    let before = baseline
        .process_with_taps(image.clone(), &cpu, None, Some(Tap::CmyFilm))
        .unwrap();
    let after = reference
        .process_with_taps(image, &cpu, None, Some(Tap::CmyFilm))
        .unwrap();
    assert_eq!(before.data, after.data);
    let identity = reference.digital_reference.as_ref().unwrap().clone();
    let mut params = reference.params.clone();
    params.scanner.reference_exposure_ev = 1.0;
    let adjusted = reference.with_params(params).unwrap();
    assert!(Arc::ptr_eq(
        &identity,
        adjusted.digital_reference.as_ref().unwrap()
    ));
    assert!(
        adjusted
            .reference_provenance()
            .unwrap()
            .receiver_content_identity
            .starts_with("sha256-")
    );
}
#[test]
fn fotufilm_zero_amount_bypasses_geometry_and_cancelled_render_fails() {
    let mut pipeline = build();
    pipeline.params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
    pipeline.params.film_render.grain.active = true;
    pipeline.params.film_render.grain.fotufilm.amount = 0.0;
    pipeline.params.io.upscale_factor = 0.5;
    let image = ImageBuf::new(8, 6);
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let (working, _) = pipeline.rescale_working(&image, &backend).unwrap();
    assert_eq!((working.width, working.height), (4, 3));
    pipeline.params.film_render.grain.fotufilm.amount = 1.0;
    let (working, _) = pipeline.rescale_working(&image, &backend).unwrap();
    assert_eq!((working.width, working.height), (8, 6));
    let cancelled = pipeline.with_cancellation(Some(Arc::new(AtomicBool::new(true))));
    assert!(
        cancelled
            .process(image, &backend)
            .unwrap_err()
            .contains("cancelled")
    );
}
/// A GPU stub whose device cannot represent any route image buffer.
struct OvertaxedGpu {
    explicit: bool,
}

impl spektrafilm_backend::ComputeBackend for OvertaxedGpu {
    fn colorspace_convert(&self, img: &ImageBuf, matrix: &[[f32; 3]; 3]) -> ImageBuf {
        spektrafilm_backend::cpu_backend::CpuBackend.colorspace_convert(img, matrix)
    }
    fn cctf_encode_srgb(&self, img: &ImageBuf) -> ImageBuf {
        spektrafilm_backend::cpu_backend::CpuBackend.cctf_encode_srgb(img)
    }
    fn cctf_decode_srgb(&self, img: &ImageBuf) -> ImageBuf {
        spektrafilm_backend::cpu_backend::CpuBackend.cctf_decode_srgb(img)
    }
    fn gaussian_blur(&self, img: &ImageBuf, sigma: f32) -> ImageBuf {
        spektrafilm_backend::cpu_backend::CpuBackend.gaussian_blur(img, sigma)
    }
    fn table_lookup(&self, img: &ImageBuf, table_x: &[f32], table_y: &[[f32; 3]]) -> ImageBuf {
        spektrafilm_backend::cpu_backend::CpuBackend.table_lookup(img, table_x, table_y)
    }
    fn lut3d_interp(&self, img: &ImageBuf, lut: &spektrafilm_backend::Lut3D) -> ImageBuf {
        spektrafilm_backend::cpu_backend::CpuBackend.lut3d_interp(img, lut)
    }
    fn is_gpu(&self) -> bool {
        true
    }
    fn explicit_gpu_request(&self) -> bool {
        self.explicit
    }
    fn image_binding_limit_reason(&self, _img: &ImageBuf) -> Option<String> {
        Some("the required buffer binding exceeds the synthetic test limit".into())
    }
    fn name(&self) -> &str {
        "overtaxed-test-gpu"
    }
}

#[test]
fn explicit_gpu_request_fails_when_the_device_cannot_bind_the_route_image() {
    let mut pipeline = build();
    pipeline.params.film_render.grain.active = false;
    let err = pipeline
        .process(ImageBuf::new(8, 6), &OvertaxedGpu { explicit: true })
        .unwrap_err();
    assert!(
        err.contains("explicit GPU request exceeds the device resource limits"),
        "{err}"
    );
    // The observation wrapper must not mask the binding limits from the
    // preflight either: every production render binds the backend through it.
    let gpu = OvertaxedGpu { explicit: true };
    let bound = spektrafilm_backend::bind_backend(
        &gpu,
        spektrafilm_backend::telemetry::ObservationContext::new(
            spektrafilm_backend::telemetry::CollectionMode::Off,
        ),
    );
    let err = pipeline
        .process(ImageBuf::new(8, 6), bound.as_ref())
        .unwrap_err();
    assert!(
        err.contains("explicit GPU request exceeds the device resource limits"),
        "{err}"
    );
    // The auto route completes instead: the same stages run their CPU work.
    let out = pipeline
        .process(ImageBuf::new(8, 6), &OvertaxedGpu { explicit: false })
        .unwrap();
    assert_eq!((out.width, out.height), (8, 6));
}

#[test]
fn fotufilm_reference_survives_amount_and_calibration_edits() {
    let base = build();
    let original = base.original_film.clone();
    let mut params = base.params.clone();
    params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
    params.film_render.grain.fotufilm.amount = 0.25;
    let edited = base.with_params(params).unwrap();
    assert!(Arc::ptr_eq(&original, &edited.original_film));
    let mut params = edited.params.clone();
    params.camera.exposure_compensation_ev += 1.0;
    let recalibrated = edited.with_params(params).unwrap();
    assert!(Arc::ptr_eq(&original, &recalibrated.original_film));
}

/// A monochrome stock must reach the silver path: the pipeline renders
/// `kodak_doublex` (five development-time columns, one emulsion value per
/// sample) with grain on and off, and the grain must cover the frame,
/// stay bounded in density and be seed-deterministic. This exercises the
/// single-curve record adaptation and the resolved reference on a real
/// profile; the CLI cannot reach it, because the loaded monochrome
/// profiles carry no Hanatos2025 adaptation window and the spectral front
/// end rejects them on every revision.
#[test]
fn fotufilm_grain_reaches_a_monochrome_film() {
    let dir = data_dir();
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let mut image = ImageBuf::new(32, 24);
    for y in 0..24 {
        for x in 0..32 {
            let exposure = if x < 16 { -1.7 } else { -0.6 };
            image.set(x, y, [from_f64(exposure + y as f64 * 0.002); 3]);
        }
    }
    let mut params = RuntimeParams::default();
    params.camera.film_format_mm = 36.0;
    params.film_render.development_time = Some(6.5);
    params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
    params.film_render.grain.active = true;
    params.film_render.grain.fotufilm.amount = 1.0;
    let pipeline = Pipeline::new(
        crate::profile::load_profile_by_name(&dir, "kodak_doublex").unwrap(),
        crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap(),
        params.clone(),
    );
    let grained = pipeline
        .process_with_taps(
            image.clone(),
            &backend,
            Some(Tap::LogEFilm),
            Some(Tap::CmyFilm),
        )
        .unwrap();
    let repeat = pipeline
        .process_with_taps(
            image.clone(),
            &backend,
            Some(Tap::LogEFilm),
            Some(Tap::CmyFilm),
        )
        .unwrap();
    assert_eq!(grained.data, repeat.data, "grain readout must be seeded");
    let mut bypass = params;
    bypass.film_render.grain.active = false;
    let plain = Pipeline::new(
        crate::profile::load_profile_by_name(&dir, "kodak_doublex").unwrap(),
        crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap(),
        bypass,
    )
    .process_with_taps(image, &backend, Some(Tap::LogEFilm), Some(Tap::CmyFilm))
    .unwrap();
    assert_eq!((grained.width, grained.height), (32, 24));
    let mut changed = 0_usize;
    let mut worst = 0.0_f64;
    let (grained_pixels, _) = grained.data.as_chunks::<3>();
    let (plain_pixels, _) = plain.data.as_chunks::<3>();
    for (pixel, plain_pixel) in grained_pixels.iter().zip(plain_pixels.iter()) {
        assert!(pixel.iter().all(|value| value.is_finite()));
        let delta = (0..3)
            .map(|channel| (to_f64(pixel[channel]) - to_f64(plain_pixel[channel])).abs())
            .fold(0.0_f64, f64::max);
        worst = worst.max(delta);
        if delta > 1e-6 {
            changed += 1;
        }
    }
    let pixels = grained.data.len() / 3;
    assert!(
        changed * 2 > pixels,
        "grain must reach most pixels, changed {changed} of {pixels}"
    );
    assert!(
        worst < 1.0,
        "silver grain must stay within a density unit, worst {worst}"
    );
}

#[test]
fn injected_film_density_does_not_generate_fotufilm_grain() {
    let mut pipeline = build();
    pipeline.params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
    pipeline.params.film_render.grain.active = true;
    pipeline.params.io.scan_film = true;
    pipeline.params.workflow.route = "input > film > scan".to_owned();
    let image = ImageBuf::new(4, 3);
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let active = pipeline
        .process_with_taps(
            image.clone(),
            &backend,
            Some(Tap::CmyFilm),
            Some(Tap::RgbOut),
        )
        .unwrap();
    pipeline.params.film_render.grain.active = false;
    let disabled = pipeline
        .process_with_taps(image, &backend, Some(Tap::CmyFilm), Some(Tap::RgbOut))
        .unwrap();
    assert_eq!(active.data, disabled.data);
}

fn lut_data(pipeline: &Pipeline) -> Vec<f64> {
    pipeline
        .tc_lut()
        .expect("spectral pipeline has a LUT")
        .data
        .clone()
}

/// Changing any sensitivity-affecting spectral control must rebuild the
/// TC LUT (and its calibration) instead of reusing the cached one — the
/// Rust counterpart of the `SpectralLUTService` invalidation keyed on
/// `Hanatos2025SensitivityAdaptation` + filtered sensitivity.
#[test]
fn with_params_rebuilds_lut_on_spectral_change() {
    let base = build();
    let base_lut = lut_data(&base);

    let change = |mutate: fn(&mut RuntimeParams)| {
        let mut params = RuntimeParams::default();
        mutate(&mut params);
        params
    };

    let cases: Vec<(&str, RuntimeParams)> = vec![
        (
            "spectral_gaussian_blur",
            change(|p| p.settings.spectral_gaussian_blur = 8.0),
        ),
        (
            "apply_hanatos2025_adaptation_window",
            change(|p| p.settings.apply_hanatos2025_adaptation_window = false),
        ),
        (
            "apply_hanatos2025_adaptation_surface",
            change(|p| p.settings.apply_hanatos2025_adaptation_surface = true),
        ),
        (
            "camera.filter_uv",
            change(|p| p.camera.filter_uv = [0.5, 415.0, 9.0]),
        ),
        (
            "camera.filter_ir",
            change(|p| p.camera.filter_ir = [0.7, 670.0, 18.0]),
        ),
    ];

    for (name, params) in cases {
        let rebuilt = base.clone().with_params(params).unwrap();
        let lut = lut_data(&rebuilt);
        assert!(
            base_lut.iter().zip(&lut).any(|(a, b)| a != b),
            "{name}: TC LUT was reused after the spectral control changed"
        );
    }
}

/// Non-spectral changes keep the calibrated LUT — `with_params` must not
/// rebuild (and must not corrupt) the front-end for ordinary tweaks.
#[test]
fn with_params_reuses_lut_on_non_spectral_change() {
    let base = build();
    let base_lut = lut_data(&base);

    let mut params = RuntimeParams::default();
    params.scanner.lens_blur = 0.7;
    params.film_render.halation.boost_ev = 0.5;
    let tweaked = base.clone().with_params(params).unwrap();
    assert_eq!(
        lut_data(&tweaked),
        base_lut,
        "non-spectral change must not rebuild the TC LUT"
    );
}
#[test]
fn with_params_rejects_failed_calibration_rebuild() {
    let base = build();
    let mut params = base.params.clone();
    params.settings.rgb_to_raw_method = "unsupported-test-method".into();
    let error = match base.with_params(params) {
        Ok(_) => panic!("invalid rebuild must not keep stale calibration"),
        Err(error) => error,
    };
    assert!(error.contains("unsupported method"), "{error}");
}
#[test]
fn with_params_rejects_invalid_print_morph_update() {
    let base = build();
    let mut params = base.params.clone();
    params.print_render.density_curves_morph.active = true;
    params.print_render.density_curves_morph.gamma_factor = 0.0;
    assert!(base.with_params(params).is_err());
}

#[test]
fn stock_edits_survive_construction_updates_and_spectral_rebuild() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "fujifilm_velvia_100").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let seeded =
        crate::params_builder::digest_params(RuntimeParams::default(), &film, &print, None, true);
    let mut edited = seeded;
    edited.film_render.halation.halation_strength = [0.3, 0.2, 0.1];
    edited.film_render.halation.halation_first_sigma_um = [13.0, 17.0, 23.0];
    edited.film_render.dir_couplers.gamma_samelayer_rgb = [0.9, 0.8, 0.7];
    edited.film_render.dir_couplers.gamma_interlayer_r_to_gb = [0.4, 0.3];
    let params = crate::params_builder::digest_params(edited, &film, &print, None, false);
    let mut pipeline = Pipeline::new_with_spectral(film, print, params.clone(), &dir).unwrap();
    assert_eq!(
        pipeline.params.film_render.halation.halation_strength,
        [0.3, 0.2, 0.1]
    );
    assert_eq!(
        pipeline.params.film_render.halation.halation_first_sigma_um,
        [13.0, 17.0, 23.0]
    );
    assert_eq!(
        pipeline.params.film_render.dir_couplers.gamma_samelayer_rgb,
        [0.9, 0.8, 0.7]
    );
    for rebuild in [false, true] {
        let mut updated = params.clone();
        if rebuild {
            updated.settings.spectral_gaussian_blur = 8.0;
        }
        pipeline = pipeline.with_params(updated).unwrap();
        assert_eq!(
            pipeline.params.film_render.halation.halation_strength,
            [0.3, 0.2, 0.1]
        );
        assert_eq!(
            pipeline.params.film_render.halation.halation_first_sigma_um,
            [13.0, 17.0, 23.0]
        );
        assert_eq!(
            pipeline.params.film_render.dir_couplers.gamma_samelayer_rgb,
            [0.9, 0.8, 0.7]
        );
        assert_eq!(
            pipeline
                .params
                .film_render
                .dir_couplers
                .gamma_interlayer_r_to_gb,
            [0.4, 0.3]
        );
    }
    let mut deactivated = params;
    deactivated.debug.deactivate_spatial_effects = true;
    let deactivated = crate::params_builder::digest_params(
        deactivated,
        &pipeline.film,
        &pipeline.print,
        None,
        false,
    );
    let pipeline = pipeline.with_params(deactivated).unwrap();
    assert_eq!(
        pipeline.params.film_render.halation.halation_first_sigma_um,
        [0.0; 3]
    );
}

#[test]
fn with_params_recalibrates_print_exposure_for_camera_ev() {
    let dir = data_dir();
    let base = build();
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = ImageBuf::from_data(8, 8, vec![from_f64(0.184); 8 * 8 * 3]);

    for ev in [-2.0f32, -1.0, 0.0, 1.0, 2.0] {
        let mut params = base.params.clone();
        params.camera.exposure_compensation_ev = ev;
        let updated = base.clone().with_params(params.clone()).unwrap();
        let fresh =
            Pipeline::new_with_spectral(base.film.clone(), base.print.clone(), params, &dir)
                .unwrap();

        assert!(
            (updated.print_exposure_factor() - fresh.print_exposure_factor()).abs() < 1e-12,
            "EV {ev}: stale print exposure factor"
        );
        let updated_output = updated.process(image.clone(), &backend).unwrap();
        let fresh_output = fresh.process(image.clone(), &backend).unwrap();
        assert_eq!(updated_output.data.len(), fresh_output.data.len());
        for (actual, expected) in updated_output.data.iter().zip(&fresh_output.data) {
            assert!(
                (*actual as f64 - *expected as f64).abs() < 1e-5,
                "EV {ev}: output mismatch ({actual} vs {expected})"
            );
        }
    }
}

#[test]
fn inactive_capture_illuminant_does_not_change_convert_film_calibration() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut base = RuntimeParams::default();
    base.workflow.route = "input > convert-film > print > scan".into();
    base.camera.capture_illuminant = None;
    let inherited =
        Pipeline::new_with_spectral(film.clone(), print.clone(), base.clone(), &dir).unwrap();
    let mut explicit = base;
    explicit.camera.capture_illuminant = Some("BB4000".into());
    let overridden = Pipeline::new_with_spectral(film, print, explicit, &dir).unwrap();
    assert_eq!(
        inherited.print_exposure_factor(),
        overridden.print_exposure_factor()
    );
    assert_eq!(lut_data(&inherited), lut_data(&overridden));
}

#[test]
fn input_gamut_active_toggle_matches_fresh_lut() {
    let base = build();
    let mut updated = base.clone();
    for active in [false, true] {
        let mut params = base.params.clone();
        params.io.input_gamut_compress.active = active;
        let rebuilt = updated.with_params(params.clone()).unwrap();
        let fresh =
            Pipeline::new_with_spectral(base.film.clone(), base.print.clone(), params, &data_dir())
                .unwrap();
        assert_eq!(lut_data(&rebuilt), lut_data(&fresh));
        assert_eq!(
            rebuilt.print_exposure_factor(),
            fresh.print_exposure_factor()
        );
        if !active {
            assert_ne!(lut_data(&rebuilt), lut_data(&base));
        }
        updated = rebuilt;
    }
}

#[test]
fn persistent_taps_and_explicit_precedence_follow_topology() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.debug.lut_mode = true;
    params.io.crop = true;
    params.io.crop_size = [0.5, 0.5];
    params.taps.inject = Some("log_e_film".into());
    params.taps.collect = Some("cmy_film".into());
    let params = crate::params_builder::digest_params(params, &film, &print, None, true);
    let pipeline = Pipeline::new(film, print, params);
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = ImageBuf::from_data(8, 8, vec![from_f64(-1.0); 8 * 8 * 3]);
    let expected = pipeline
        .process_with_taps(
            image.clone(),
            &backend,
            Some(Tap::LogEFilm),
            Some(Tap::CmyFilm),
        )
        .unwrap();
    assert_eq!(
        pipeline.process(image.clone(), &backend).unwrap().data,
        expected.data
    );
    assert_eq!(
        pipeline
            .process_with_taps(image.clone(), &backend, None, None)
            .unwrap()
            .data,
        expected.data
    );
    let unchanged = pipeline
        .process_with_taps(image.clone(), &backend, None, Some(Tap::LogEFilm))
        .unwrap();
    assert_eq!(unchanged.data, image.data);
    let unchanged = pipeline
        .process_with_taps(image.clone(), &backend, Some(Tap::CmyFilm), None)
        .unwrap();
    assert_eq!(unchanged.data, image.data);
    assert_eq!((expected.width, expected.height), (8, 8));
    assert!(
        pipeline
            .process_resident_borrowed(&image, &backend)
            .unwrap()
            .is_none()
    );
    let image = ImageBuf::from_data(8, 8, vec![from_f64(0.184); 8 * 8 * 3]);
    let normal = pipeline
        .process_with_taps(image.clone(), &backend, Some(Tap::RgbIn), Some(Tap::RgbOut))
        .unwrap();
    // `digest_params(..., lut_mode = true)` disables crop so a LUT bake
    // remains a static point transform.
    assert_eq!((normal.width, normal.height), (8, 8));
    let mut params = pipeline.params.clone();
    params.taps = Default::default();
    let default = pipeline
        .with_params(params)
        .unwrap()
        .process(image, &backend)
        .unwrap();
    assert_eq!(normal.data, default.data);
}
#[test]
fn v3_final_crop_preserves_scanner_and_print_boundaries() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_800").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let mut image = ImageBuf::new(96, 64);
    for y in 0..64 {
        for x in 0..96 {
            let exposure = if x < 48 { -1.7 } else { -0.6 };
            image.set(x, y, [from_f64(exposure + y as f64 * 0.002); 3]);
        }
    }
    for (scan_film, output) in [
        (false, "direct_scan"),
        (true, "direct_scan"),
        (true, "negative_to_positive"),
    ] {
        for scale in [1.0, 1.5] {
            let mut params = RuntimeParams::default();
            params.camera.film_format_mm = 36.0;
            params.film_render.grain.engine = crate::params::GrainEngine::Dye;
            params.film_render.dir_couplers.active = false;
            params.film_render.halation.active = false;
            params.workflow.route = if scan_film {
                "input > film > scan"
            } else {
                "input > film > print > scan"
            }
            .into();
            params.io.scan_film = scan_film;
            params.io.upscale_factor = scale;
            params.scanner.scan_output = output.into();
            params.scanner.lens_blur = if scale == 1.0 { 1.5 } else { 3.0 };
            params.scanner.unsharp_mask = [0.7, 0.7];
            params.enlarger.diffusion_filter.active = !scan_film;
            let full_pipeline = Pipeline::new(film.clone(), print.clone(), params.clone());
            let color_ref = full_pipeline.color_reference(&backend);
            // Inject a small developed-exposure field at photographic pitch.
            let render = |pipeline: &Pipeline| {
                pipeline
                    .run_from(
                        image.clone(),
                        Tap::LogEFilm,
                        Tap::RgbOut,
                        &backend,
                        &color_ref,
                        Some((3.1, 0.0)),
                        None,
                        None,
                    )
                    .unwrap()
            };
            let full = render(&full_pipeline);
            params.io.crop = true;
            params.io.crop_center = [0.5, 0.5];
            params.io.crop_size = [0.5, 0.5];
            let cropped = render(&Pipeline::new(film.clone(), print.clone(), params));
            let expected = crate::resizing::crop_image(&full, [0.5, 0.5], [0.5, 0.5]);
            let expected = expected.unwrap();
            assert_eq!(
                (cropped.width, cropped.height),
                (expected.width, expected.height)
            );
            let worst = cropped
                .data
                .iter()
                .zip(expected.data.iter())
                .map(|(&a, &b)| (a as f64 - b as f64).abs())
                .fold(0.0f64, f64::max);
            assert!(
                worst <= 1e-5,
                "{scan_film} {output} scale {scale}: crop edge error {worst}"
            );
        }
    }
}
#[test]
fn fotufilm_dispatch_matrix_follows_development_boundary() {
    #[derive(Clone, Copy)]
    struct Case {
        name: &'static str,
        route: &'static str,
        scan_film: bool,
        inject: Tap,
        collect: Tap,
        develops: bool,
    }

    let cases = [
        Case {
            name: "film-scan full",
            route: "input > film > scan",
            scan_film: true,
            inject: Tap::RgbIn,
            collect: Tap::RgbOut,
            develops: true,
        },
        Case {
            name: "film-scan collect film",
            route: "input > film > scan",
            scan_film: true,
            inject: Tap::RgbIn,
            collect: Tap::CmyFilm,
            develops: true,
        },
        Case {
            name: "film-scan developed injection",
            route: "input > film > scan",
            scan_film: true,
            inject: Tap::CmyFilm,
            collect: Tap::RgbOut,
            develops: false,
        },
        Case {
            name: "film-scan film-log injection",
            route: "input > film > scan",
            scan_film: true,
            inject: Tap::LogEFilm,
            collect: Tap::CmyFilm,
            develops: true,
        },
        Case {
            name: "film-scan passthrough",
            route: "input > film > scan",
            scan_film: true,
            inject: Tap::CmyFilm,
            collect: Tap::CmyFilm,
            develops: false,
        },
        Case {
            name: "film-print full",
            route: "input > film > print > scan",
            scan_film: false,
            inject: Tap::RgbIn,
            collect: Tap::RgbOut,
            develops: true,
        },
        Case {
            name: "film-print collect film",
            route: "input > film > print > scan",
            scan_film: false,
            inject: Tap::RgbIn,
            collect: Tap::CmyFilm,
            develops: true,
        },
        Case {
            name: "film-print collect print log",
            route: "input > film > print > scan",
            scan_film: false,
            inject: Tap::RgbIn,
            collect: Tap::LogEPrint,
            develops: true,
        },
        Case {
            name: "film-print collect print density",
            route: "input > film > print > scan",
            scan_film: false,
            inject: Tap::LogEPrint,
            collect: Tap::CmyPrint,
            develops: false,
        },
        Case {
            name: "film-print print-paper segment",
            route: "input > film > print > scan",
            scan_film: false,
            inject: Tap::CmyFilm,
            collect: Tap::CmyPrint,
            develops: false,
        },
        Case {
            name: "film-print later injection",
            route: "input > film > print > scan",
            scan_film: false,
            inject: Tap::CmyPrint,
            collect: Tap::RgbOut,
            develops: false,
        },
        Case {
            name: "passthrough",
            route: "input",
            scan_film: false,
            inject: Tap::RgbIn,
            collect: Tap::RgbIn,
            develops: false,
        },
        Case {
            name: "convert-film print segment",
            route: "input > convert-film > print > scan",
            scan_film: false,
            inject: Tap::CmyFilm,
            collect: Tap::RgbOut,
            develops: false,
        },
        Case {
            name: "convert-film scan segment",
            route: "input > convert-film > scan",
            scan_film: true,
            inject: Tap::CmyFilm,
            collect: Tap::RgbOut,
            develops: false,
        },
    ];

    // The matrix changes only routing controls. Rebuild parameter views
    // from one prepared pipeline so every case exercises its own route
    // without repeating the same Fotufilm population preparation.
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = ImageBuf::from_data(
        8,
        6,
        (0..8 * 6 * 3)
            .map(|i| from_f64(-1.6 + (i % 17) as f64 * 0.11))
            .collect(),
    );
    let base = Pipeline::new(film, print, RuntimeParams::default());

    for case in cases {
        let mut params = RuntimeParams::default();
        params.workflow.route = case.route.into();
        params.io.scan_film = case.scan_film;
        params.film_render.grain.active = true;
        params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
        params.film_render.grain.fotufilm.amount = 1.0;
        params.random_seed = 17;
        let active = base.clone().with_params(params.clone()).unwrap();
        params.film_render.grain.active = false;
        let disabled = base.clone().with_params(params).unwrap();

        let active_context = spektrafilm_backend::telemetry::ObservationContext::new(
            spektrafilm_backend::telemetry::CollectionMode::Summary,
        );
        let active_backend = spektrafilm_backend::bind_backend(&backend, active_context.clone());
        let active_image = active
            .process_with_taps(
                image.clone(),
                active_backend.as_ref(),
                Some(case.inject),
                Some(case.collect),
            )
            .unwrap_or_else(|error| panic!("{}: {error}", case.name));
        let active_grain_stages = active_context
            .snapshot()
            .observations
            .iter()
            .filter(|observation| observation.name == "grain_fotufilm")
            .count();
        assert_eq!(
            active_grain_stages,
            if case.develops { 1 } else { 0 },
            "{}",
            case.name
        );
        let disabled_image = disabled
            .process_with_taps(
                image.clone(),
                &backend,
                Some(case.inject),
                Some(case.collect),
            )
            .unwrap_or_else(|error| panic!("{} disabled: {error}", case.name));
        if case.develops {
            assert_ne!(active_image.data, disabled_image.data, "{}", case.name);
        } else {
            assert_eq!(active_image.data, disabled_image.data, "{}", case.name);
        }
    }
}

#[test]
fn fotufilm_dispatch_with_crop_and_optics_runs_once() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > print > scan".into();
    params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
    params.film_render.grain.active = true;
    params.io.crop = true;
    params.io.crop_size = [0.5, 0.5];
    params.enlarger.lens_blur = 1.25;
    params.scanner.lens_blur = 1.0;
    params.random_seed = 41;
    let pipeline = Pipeline::new(film.clone(), print.clone(), params.clone());
    params.film_render.grain.active = false;
    let disabled = Pipeline::new(film, print, params);
    let image = ImageBuf::from_data(12, 8, vec![from_f64(-0.8); 12 * 8 * 3]);
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let mut timings = std::collections::BTreeMap::new();
    let active = pipeline
        .process_with_taps_timed(
            image.clone(),
            &backend,
            Some(Tap::RgbIn),
            Some(Tap::RgbOut),
            Some(&mut timings),
            None,
        )
        .unwrap();
    assert!(timings.contains_key("grain_fotufilm_readout"));
    let quiet = disabled
        .process_with_taps(image, &backend, Some(Tap::RgbIn), Some(Tap::RgbOut))
        .unwrap();
    assert_ne!(active.data, quiet.data);
    assert!(active.width < 12 && active.height < 8);
}

#[test]
fn fotufilm_lut_mode_matches_grain_disabled_and_prepares_no_bank() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.debug.lut_mode = true;
    params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
    params.film_render.grain.active = true;
    params.film_render.grain.fotufilm.amount = 1.0;
    let digested = crate::params_builder::digest_params(params.clone(), &film, &print, None, true);
    assert!(!crate::stages::grain_fotufilm::active(&digested));
    let active = Pipeline::new(film.clone(), print.clone(), digested);
    params.film_render.grain.active = false;
    let disabled_params = crate::params_builder::digest_params(params, &film, &print, None, true);
    let disabled = Pipeline::new(film, print, disabled_params);
    let image = ImageBuf::from_data(4, 4, vec![from_f64(0.18); 4 * 4 * 3]);
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    assert_eq!(
        active.process(image.clone(), &backend).unwrap().data,
        disabled.process(image, &backend).unwrap().data
    );
}

#[test]
fn fotufilm_seed_and_amount_determinism() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    // Seed and amount are readout controls; keep the prepared pipeline
    // shared while each parameter view remains independently rebuilt.
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > scan".into();
    params.io.scan_film = true;
    params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
    params.film_render.grain.active = true;
    params.film_render.grain.fotufilm.amount = 1.0;
    params.random_seed = 91;
    let image = ImageBuf::from_data(
        8,
        6,
        (0..8 * 6 * 3)
            .map(|i| from_f64(-1.1 + (i % 13) as f64 * 0.15))
            .collect(),
    );
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let base = Pipeline::new(film, print, params.clone());
    let render = |p: Pipeline| p.process(image.clone(), &backend).unwrap().data;
    let first = render(base.clone());
    let repeat = render(base.clone());
    assert_eq!(first, repeat);
    params.random_seed += 1;
    let changed = render(base.clone().with_params(params.clone()).unwrap());
    assert_ne!(first, changed);
    params.random_seed = 91;
    params.film_render.grain.fotufilm.amount = 0.65;
    let amount_first = render(base.clone().with_params(params.clone()).unwrap());
    let amount_repeat = render(base.with_params(params).unwrap());
    assert_eq!(amount_first, amount_repeat);
}
#[test]
fn output_mix_interpolates_complete_output_in_actual_linear_transport() {
    use spektrafilm_math::colorspace::{cctf_decode, cctf_encode, resolve};
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = ImageBuf::from_data(
        3,
        1,
        [-0.1, 0.2, 1.4, 0.1, 0.4, 0.8, 1.5, 0.8, 0.3]
            .map(from_f64)
            .to_vec(),
    );
    for encoded in [false, true] {
        let mut pipeline = build();
        pipeline.params.camera.auto_exposure = false;
        pipeline.params.film_render.grain.active = true;
        pipeline.params.film_render.grain.engine = crate::params::GrainEngine::Procedural;
        pipeline.params.random_seed = 37;
        pipeline.params.cmyk.active = true;
        pipeline.params.io.output_color_space = "sRGB".into();
        pipeline.params.io.output_cctf_encoding = encoded;
        let complete = pipeline.process(image.clone(), &backend).unwrap();
        let complete_storage = complete.data.as_ptr();
        let exact = pipeline
            .compose_output_mix(complete, None, &backend, None)
            .unwrap();
        assert_eq!(exact.data.as_ptr(), complete_storage);
        pipeline.params.output_mix.amount = 0.0;
        let source = pipeline.process(image.clone(), &backend).unwrap();
        pipeline.params.output_mix.amount = 0.35;
        let mixed = pipeline.process(image.clone(), &backend).unwrap();
        let space = resolve("sRGB").unwrap();
        for ((source, result), actual) in source.data.iter().zip(&exact.data).zip(&mixed.data) {
            let linear = |value| {
                if encoded {
                    cctf_decode(to_f64(value), space.cctf)
                } else {
                    to_f64(value)
                }
            };
            let expected = 0.65 * linear(*source) + 0.35 * linear(*result);
            let expected = if encoded {
                cctf_encode(expected, space.cctf)
            } else {
                expected
            };
            assert!((to_f64(*actual) - expected).abs() < 3e-6);
        }
    }
}

#[test]
fn output_mix_zero_decodes_before_resampling_and_ignores_virtual_exposure() {
    use spektrafilm_math::colorspace::{cctf_decode, conversion_matrix, convert_rgb, resolve};
    let mut pipeline = build();
    pipeline.params.workflow.route = "input > film > scan".into();
    pipeline.params.io.scan_film = true;
    pipeline.params.film_render.grain.active = false;
    pipeline.params.io.input_color_space = "sRGB".into();
    pipeline.params.io.output_color_space = "ACES2065-1".into();
    pipeline.params.io.input_cctf_decoding = true;
    pipeline.params.io.output_cctf_encoding = false;
    pipeline.params.io.crop = true;
    pipeline.params.io.crop_center = [0.5, 0.5];
    pipeline.params.io.crop_size = [0.75, 0.5];
    pipeline.params.io.upscale_factor = 1.5;
    pipeline.params.camera.auto_exposure = true;
    pipeline.params.camera.exposure_compensation_ev = 4.0;
    pipeline.params.output_mix.amount = 0.0;
    let image = ImageBuf::from_data(
        8,
        6,
        (0..8 * 6 * 3)
            .map(|i| from_f64(i as f64 / 80.0 - 0.15))
            .collect(),
    );
    let input = resolve("sRGB").unwrap();
    let output = resolve("ACES2065-1").unwrap();
    let mut decoded = image.clone();
    for value in &mut decoded.data {
        *value = from_f64(cctf_decode(to_f64(*value), input.cctf));
    }
    let (expected, _) = crate::resizing::crop_and_rescale(
        &decoded,
        &pipeline.params.io,
        pipeline.params.camera.film_format_mm,
    )
    .unwrap();
    let mut expected = expected.into_owned();
    let matrix = conversion_matrix(input, output);
    for pixel in expected.data.as_chunks_mut::<3>().0 {
        let converted = convert_rgb(
            [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])],
            input,
            false,
            output,
            false,
            &matrix,
        );
        for c in 0..3 {
            pixel[c] = from_f64(converted[c]);
        }
    }
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let actual = pipeline.process(image, &backend).unwrap();
    assert_eq!((actual.width, actual.height), (9, 6));
    assert_eq!(actual.data, expected.data);
}

#[test]
fn output_mix_does_not_touch_density_taps_and_rejects_missing_final_source() {
    let mut pipeline = build();
    pipeline.params.film_render.grain.active = false;
    pipeline.params.camera.auto_exposure = false;
    let image = ImageBuf::from_data(2, 2, vec![from_f64(0.2); 12]);
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let baseline = pipeline
        .process_with_taps(
            image.clone(),
            &backend,
            Some(Tap::RgbIn),
            Some(Tap::CmyFilm),
        )
        .unwrap();
    pipeline.params.output_mix.amount = 0.0;
    let diagnostic = pipeline
        .process_with_taps(
            image.clone(),
            &backend,
            Some(Tap::RgbIn),
            Some(Tap::CmyFilm),
        )
        .unwrap();
    assert_eq!(baseline.data, diagnostic.data);
    for tap in [Tap::RgbPre, Tap::CmyFilm, Tap::RgbOut] {
        let error = pipeline
            .process_with_taps(image.clone(), &backend, Some(tap), Some(Tap::RgbOut))
            .unwrap_err();
        assert!(error.contains("requires rgb_in injection"));
    }
}

#[test]
fn output_mix_source_endpoint_never_reads_processed_samples() {
    let mut pipeline = build();
    pipeline.params.output_mix.amount = 0.0;
    pipeline.params.io.output_cctf_encoding = false;
    let source = ImageBuf::from_data(1, 1, [-0.4, 0.2, 1.8].map(from_f64).to_vec());
    let storage = source.data.as_ptr();
    let expected = source.data.clone();
    let poisoned = ImageBuf::from_data(1, 1, vec![from_f64(f64::NAN); 3]);
    let output = pipeline
        .compose_output_mix(
            poisoned,
            Some(source),
            &spektrafilm_backend::cpu_backend::CpuBackend,
            None,
        )
        .unwrap();
    assert_eq!(output.data.as_ptr(), storage);
    assert_eq!(output.data, expected);
}

#[test]
fn output_mix_source_matches_deferred_field_crop_rounding() {
    let mut pipeline = build();
    let source = ImageBuf::from_data(
        7,
        5,
        (0..7 * 5 * 3).map(|i| from_f64(i as f64 / 50.0)).collect(),
    );
    pipeline.params.io.crop = true;
    pipeline.params.io.crop_size = [0.7, 0.4];
    pipeline.params.io.input_cctf_decoding = false;
    for engine in [
        crate::params::GrainEngine::Dye,
        crate::params::GrainEngine::Fotufilm,
    ] {
        pipeline.params.film_render.grain.engine = engine;
        pipeline.params.film_render.grain.active = true;
        for scale in [0.5, 1.5, 2.1] {
            pipeline.params.io.upscale_factor = scale;
            let geometry =
                crate::stages::grain_v3::geometry(&source, &pipeline.params, 2.0).unwrap();
            let prepared = pipeline.prepare_output_mix_source(&source).unwrap();
            assert_eq!(
                (prepared.width, prepared.height),
                (geometry.width, geometry.height)
            );
            let (full, crop) = geometry.full_film_grid(&source).unwrap();
            let cropped =
                crate::stages::grain_v3::crop_output(ImageBuf::new(full.width, full.height), crop);
            assert_eq!(
                (prepared.width, prepared.height),
                (cropped.width, cropped.height)
            );
        }
    }
}

#[test]
fn output_mix_input_route_keeps_cmyk_only_in_result_branch() {
    let mut pipeline = build();
    pipeline.params.workflow.route = "input".into();
    pipeline.params.film_render.grain.active = false;
    pipeline.params.cmyk.active = true;
    pipeline.params.io.input_color_space = "ACES2065-1".into();
    pipeline.params.io.output_color_space = "ACES2065-1".into();
    pipeline.params.io.input_cctf_decoding = false;
    pipeline.params.io.output_cctf_encoding = false;
    let image = ImageBuf::from_data(1, 1, [-0.1, 0.4, 1.4].map(from_f64).to_vec());
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let complete = pipeline.process(image.clone(), &backend).unwrap();
    pipeline.params.output_mix.amount = 0.0;
    let source = pipeline.process(image.clone(), &backend).unwrap();
    assert!(source.data[0] < from_f64(0.0));
    assert!(source.data[2] > from_f64(1.0));
    assert_ne!(complete.data, source.data);
    pipeline.params.output_mix.amount = 0.5;
    let mixed = pipeline.process(image, &backend).unwrap();
    for ((s, r), actual) in source.data.iter().zip(&complete.data).zip(&mixed.data) {
        assert_eq!(*actual, from_f64(0.5 * to_f64(*s) + 0.5 * to_f64(*r)));
    }
}

// --- Native stage evidence capture -----------------------------------------

fn capture_manifest(dir: &std::path::Path) -> serde_json::Value {
    let text = std::fs::read_to_string(dir.join("stage-evidence.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn stage<'a>(manifest: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    manifest["stages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == id)
        .unwrap()
}

#[test]
fn stage_capture_preserves_output_and_publishes_the_executed_boundaries() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.camera.auto_exposure = true;
    params.io.crop = true;
    params.io.crop_size = [0.5, 0.25];
    let pipeline = Pipeline::new(film, print, params);
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = ImageBuf::from_data(
        8,
        6,
        (0..8 * 6 * 3)
            .map(|i| from_f64((i % 7) as f64 * 0.1))
            .collect(),
    );
    let expected = pipeline.process(image.clone(), &backend).unwrap();

    let bundle = tempfile::tempdir().unwrap();
    let mut capture = crate::stage_evidence::StageCapture::new(bundle.path());
    let captured = pipeline
        .process_with_taps_timed(
            image.clone(),
            &backend,
            None,
            None,
            None,
            Some(&mut capture),
        )
        .unwrap();
    assert_eq!(captured.data, expected.data, "capture changed the render");
    capture.publish().unwrap();

    let manifest = capture_manifest(bundle.path());
    assert_eq!(manifest["execution_status"], "completed");
    for id in [
        "rgb_in",
        "log_e_film",
        "cmy_film",
        "log_e_print",
        "cmy_print",
        "rgb_out",
    ] {
        assert_eq!(stage(&manifest, id)["availability"], "available", "{id}");
    }
    // Metering runs on the full source, before crop/resize; the intermediate
    // grid is the cropped working geometry.
    assert_eq!(
        manifest["exposure"]["source_dimensions"],
        serde_json::json!([8, 6])
    );
    assert_eq!(stage(&manifest, "rgb_in")["layout"]["width"], 8);
    assert_ne!(stage(&manifest, "log_e_film")["layout"]["width"], 8);
    assert_eq!(
        manifest["print_normalization"]["value"],
        pipeline.print_exposure_factor()
    );
    // The rgb_out buffer is byte-identical to the raw-out dump of the same
    // output (pre-saving pipeline output).
    let mut expected_bytes = Vec::new();
    for &value in &expected.data {
        expected_bytes.extend_from_slice(&to_f64(value).to_le_bytes());
    }
    assert_eq!(
        std::fs::read(bundle.path().join("rgb_out.f64")).unwrap(),
        expected_bytes
    );
}

#[test]
fn stage_capture_input_route_marks_film_and_print_stages_not_applicable() {
    let mut pipeline = build();
    pipeline.params.workflow.route = "input".into();
    pipeline.params.film_render.grain.active = false;
    pipeline.params.cmyk.active = false;
    pipeline.params.io.input_color_space = "ACES2065-1".into();
    pipeline.params.io.output_color_space = "ACES2065-1".into();
    pipeline.params.io.input_cctf_decoding = false;
    pipeline.params.io.output_cctf_encoding = false;
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = ImageBuf::from_data(8, 6, vec![from_f64(0.2); 8 * 6 * 3]);
    let bundle = tempfile::tempdir().unwrap();
    let mut capture = crate::stage_evidence::StageCapture::new(bundle.path());
    pipeline
        .process_with_taps_timed(image, &backend, None, None, None, Some(&mut capture))
        .unwrap();
    capture.publish().unwrap();

    let manifest = capture_manifest(bundle.path());
    assert_eq!(manifest["execution_status"], "completed");
    for id in ["log_e_film", "cmy_film"] {
        assert_eq!(
            stage(&manifest, id)["reason"],
            "no_film_stage_on_route",
            "{id}"
        );
    }
    for id in ["log_e_print", "cmy_print"] {
        assert_eq!(
            stage(&manifest, id)["reason"],
            "no_print_stage_on_route",
            "{id}"
        );
    }
    assert_eq!(manifest["exposure"]["availability"], "not_applicable");
    assert_eq!(
        manifest["exposure"]["reason"],
        "no_exposure_application_on_route"
    );
    assert_eq!(
        manifest["print_normalization"]["availability"],
        "not_applicable"
    );
}

#[test]
fn stage_capture_scan_route_skips_print_stages_without_failing() {
    let mut pipeline = build();
    pipeline.params.workflow.route = "input > film > scan".into();
    pipeline.params.io.scan_film = true;
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = ImageBuf::from_data(8, 6, vec![from_f64(0.2); 8 * 6 * 3]);
    let bundle = tempfile::tempdir().unwrap();
    let mut capture = crate::stage_evidence::StageCapture::new(bundle.path());
    pipeline
        .process_with_taps_timed(image, &backend, None, None, None, Some(&mut capture))
        .unwrap();
    capture.publish().unwrap();

    let manifest = capture_manifest(bundle.path());
    assert_eq!(manifest["execution_status"], "completed");
    for id in ["log_e_print", "cmy_print"] {
        assert_eq!(
            stage(&manifest, id)["availability"],
            "not_applicable",
            "{id}"
        );
        assert_eq!(
            stage(&manifest, id)["reason"],
            "no_print_stage_on_route",
            "{id}"
        );
    }
    for id in ["rgb_in", "log_e_film", "cmy_film", "rgb_out"] {
        assert_eq!(stage(&manifest, id)["availability"], "available", "{id}");
    }
}

#[test]
fn stage_capture_declares_film_frame_geometry_for_the_expanded_grain_grid() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > print > scan".into();
    params.film_render.grain.engine = crate::params::GrainEngine::Fotufilm;
    params.film_render.grain.active = true;
    params.io.crop = true;
    params.io.crop_size = [0.5, 0.5];
    params.enlarger.lens_blur = 1.25;
    params.scanner.lens_blur = 1.0;
    params.random_seed = 41;
    let pipeline = Pipeline::new(film, print, params);
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = ImageBuf::from_data(12, 8, vec![from_f64(0.2); 12 * 8 * 3]);
    let bundle = tempfile::tempdir().unwrap();
    let mut capture = crate::stage_evidence::StageCapture::new(bundle.path());
    let output = pipeline
        .process_with_taps_timed(image, &backend, None, None, None, Some(&mut capture))
        .unwrap();
    capture.publish().unwrap();

    let manifest = capture_manifest(bundle.path());
    assert_eq!(manifest["execution_status"], "completed");
    // The delivered crop is extracted after the spatial effects run on the
    // full-film frame: intermediates must say so instead of inheriting the
    // output grid silently.
    assert_eq!(
        stage(&manifest, "log_e_film")["geometry"]["coordinate_space"],
        "working_pixels"
    );
    for id in ["cmy_film", "log_e_print", "cmy_print"] {
        assert_eq!(
            stage(&manifest, id)["geometry"]["coordinate_space"],
            "film_frame_pixels",
            "{id}"
        );
    }
    assert_eq!(
        stage(&manifest, "rgb_out")["geometry"]["coordinate_space"],
        "output_pixels"
    );
    assert_eq!(stage(&manifest, "rgb_out")["layout"]["width"], output.width);
    assert!(
        stage(&manifest, "cmy_film")["layout"]["width"]
            .as_u64()
            .unwrap()
            > stage(&manifest, "rgb_out")["layout"]["width"]
                .as_u64()
                .unwrap(),
        "the film-frame grid must not be the delivered grid"
    );
}

/// CMYK ink style is optional finishing, not a route. The conversion
/// routes are base workflows too, so they must run the shared terminal
/// appearance exactly once, after their scanner-domain effects, with the
/// same identity behavior as every other route.
#[test]
fn convert_routes_apply_the_shared_terminal_cmyk_appearance() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = ImageBuf::from_data(
        8,
        8,
        (0..8 * 8 * 3)
            .map(|i| {
                let channel = i % 3;
                from_f64(0.25 + 0.4 * channel as f64 / 2.0)
            })
            .collect(),
    );
    let output_space =
        spektrafilm_math::colorspace::resolve("sRGB").expect("registered output space");
    let full_strength = crate::params::CmykParams {
        active: true,
        strength: 1.0,
    };
    let render = |params: RuntimeParams| {
        Pipeline::new_with_spectral(film.clone(), print.clone(), params, &dir)
            .unwrap()
            .process(image.clone(), &backend)
            .unwrap()
    };
    for route in [
        "input > convert-film > print > scan",
        "input > convert-film > scan",
        "input > convert-film > scan-minus-base",
    ] {
        let mut disabled = RuntimeParams::default();
        disabled.workflow.route = route.into();
        disabled.io.scan_film = !route.contains("print");
        disabled.camera.auto_exposure = false;
        disabled.film_render.grain.active = false;
        disabled.film_render.halation.active = false;
        disabled.film_render.dir_couplers.active = false;
        disabled.io.output_cctf_encoding = true;
        disabled.random_seed = 5;
        let base = Pipeline::new_with_spectral(film.clone(), print.clone(), disabled.clone(), &dir)
            .unwrap()
            .process(image.clone(), &backend)
            .unwrap();

        // Disabled and active strength 0 are exact identities on every route.
        for params in [
            crate::params::CmykParams {
                active: false,
                strength: 1.0,
            },
            crate::params::CmykParams {
                active: true,
                strength: 0.0,
            },
        ] {
            let mut toggled = disabled.clone();
            toggled.cmyk = params;
            let output = render(toggled);
            assert_eq!(
                output.data, base.data,
                "{route}: inactive and strength-0 cmyk must be exact identities"
            );
        }

        // Active strength 1 applies the shared terminal appearance once,
        // matching the reference transform over the completed RGB result.
        let mut active = disabled.clone();
        active.cmyk = full_strength.clone();
        let full = render(active);
        assert_ne!(
            full.data, base.data,
            "{route}: active strength 1 must apply the appearance"
        );
        let mut reference = base.clone();
        crate::cmyk::apply(&mut reference, &full_strength, output_space, true);
        let diff = full
            .data
            .iter()
            .zip(reference.data.iter())
            .map(|(a, b)| (to_f64(*a) - to_f64(*b)).abs())
            .fold(0.0, f64::max);
        assert!(
            diff < 1e-6,
            "{route}: the terminal appearance must match the shared reference exactly once \
             (max diff {diff})"
        );
    }
}
