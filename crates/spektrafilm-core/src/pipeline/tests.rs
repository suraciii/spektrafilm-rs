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

fn deterministic_material_params(pipeline: &Pipeline) -> RuntimeParams {
    let mut params = pipeline.params.clone();
    params.camera.auto_exposure = false;
    params.film_render.grain.active = false;
    params.film_render.halation.active = false;
    params.film_render.dir_couplers.active = false;
    params.film_render.glare.active = false;
    params.print_render.glare.active = false;
    params.enlarger.diffusion_filter.active = false;
    params.enlarger.lens_blur = 0.0;
    params.scanner.lens_blur = 0.0;
    params.scanner.unsharp_mask = [0.0; 2];
    params.debug.deactivate_spatial_effects = true;
    params.debug.deactivate_stochastic_effects = true;
    params
}

fn material_probe() -> ImageBuf {
    ImageBuf::from_data(
        5,
        4,
        (0..60)
            .map(|i| from_f64(0.035 + (i % 31) as f64 / 45.0))
            .collect(),
    )
}

fn fresh_material_pipeline(source: &Pipeline, params: RuntimeParams) -> Pipeline {
    Pipeline::new_with_spectral(
        (*source.original_film).clone(),
        (*source.original_print).clone(),
        params,
        &data_dir(),
    )
    .unwrap()
}

fn assert_material_pixels_close(actual: &ImageBuf, expected: &ImageBuf, case: &str) {
    assert_eq!(
        (actual.width, actual.height),
        (expected.width, expected.height),
        "{case}"
    );
    assert_eq!(actual.data.len(), expected.data.len(), "{case}");
    for (index, (&actual, &expected)) in actual.data.iter().zip(&expected.data).enumerate() {
        let (actual, expected) = (to_f64(actual), to_f64(expected));
        assert!(
            actual.is_finite() && expected.is_finite(),
            "{case}: non-finite pixel {index}"
        );
        assert!(
            (actual - expected).abs() <= 1e-5 * (1.0 + expected.abs()),
            "{case}: pixel {index}: {actual} != {expected}"
        );
    }
}

fn assert_material_pixels_changed(actual: &ImageBuf, baseline: &ImageBuf, case: &str) {
    assert!(
        actual
            .data
            .iter()
            .zip(&baseline.data)
            .any(|(&a, &b)| (to_f64(a) - to_f64(b)).abs() > 1e-7),
        "{case}: edit did not affect rendered pixels"
    );
}

#[test]
fn material_edits_match_fresh_raw_profile_preparation() {
    let shared = build();
    let base = shared
        .clone()
        .with_params(deterministic_material_params(&shared))
        .unwrap();
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = material_probe();
    let baseline = base.process(image.clone(), &backend).unwrap();
    type Edit = fn(&mut RuntimeParams);
    let cases: [(&str, Edit); 5] = [
        ("film base scale", |p| p.film_render.base.scale = 1.7),
        ("film base color", |p| {
            p.film_render.base.yellow = 1.6;
            p.film_render.base.cyan = 0.65;
            p.film_render.base.tilt = 0.12;
        }),
        ("film chemistry gamma", |p| {
            p.film_render.chemistry.gamma_factor = 1.5
        }),
        ("film chemistry exhaustion", |p| {
            p.film_render.chemistry.developer_exhaustion = 0.8
        }),
        ("print base", |p| {
            p.print_render.base.scale = 1.7;
            p.print_render.base.yellow = 1.5;
            p.print_render.base.cyan = 0.7;
        }),
    ];
    for (name, edit) in cases {
        let mut params = base.params.clone();
        edit(&mut params);
        let cached = base.clone().with_params(params).unwrap();
        let fresh = fresh_material_pipeline(&base, cached.params.clone());
        let actual = cached.process(image.clone(), &backend).unwrap();
        let expected = fresh.process(image.clone(), &backend).unwrap();
        assert_material_pixels_close(&actual, &expected, name);
        assert_material_pixels_changed(&actual, &baseline, name);
    }

    let mut params = base.params.clone();
    params.film_render.chemistry.gamma_factor = 1.5;
    params.film_render.chemistry.developer_exhaustion = 0.7;
    let active = base.clone().with_params(params.clone()).unwrap();
    params.film_render.chemistry.active = false;
    let inactive = active.clone().with_params(params).unwrap();
    let fresh = fresh_material_pipeline(&base, inactive.params.clone());
    let inactive_pixels = inactive.process(image.clone(), &backend).unwrap();
    assert_material_pixels_close(
        &inactive_pixels,
        &fresh.process(image.clone(), &backend).unwrap(),
        "film chemistry toggle",
    );
    assert_material_pixels_changed(
        &inactive_pixels,
        &active.process(image, &backend).unwrap(),
        "film chemistry toggle",
    );
}

#[test]
fn film_gamma_recalibrates_print_from_raw_materials() {
    let shared = build();
    let base = shared
        .clone()
        .with_params(deterministic_material_params(&shared))
        .unwrap();
    let mut params = base.params.clone();
    params.film_render.density_curve_gamma = 1.6;
    let updated = base.clone().with_params(params).unwrap();
    let fresh = fresh_material_pipeline(&base, updated.params.clone());
    assert!(updated.print_exposure_factor().is_finite());
    assert!(fresh.print_exposure_factor().is_finite());
    assert!((updated.print_exposure_factor() - fresh.print_exposure_factor()).abs() < 1e-10);
    assert!((updated.print_exposure_factor() - base.print_exposure_factor()).abs() > 1e-8);
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = material_probe();
    let actual = updated.process(image.clone(), &backend).unwrap();
    assert_material_pixels_close(
        &actual,
        &fresh.process(image.clone(), &backend).unwrap(),
        "film gamma",
    );
    assert_material_pixels_changed(
        &actual,
        &base.process(image, &backend).unwrap(),
        "film gamma",
    );
}

#[test]
fn material_edits_round_trip_and_exposure_rebuild_do_not_compound_base() {
    let shared = build();
    let base = shared
        .clone()
        .with_params(deterministic_material_params(&shared))
        .unwrap();
    let mut a = base.params.clone();
    a.film_render.base.scale = 1.4;
    a.film_render.base.yellow = 1.3;
    a.print_render.base.scale = 1.6;
    a.print_render.base.magenta = 0.7;
    let first = base.with_params(a.clone()).unwrap();
    let mut b = first.params.clone();
    b.film_render.base.scale = 0.7;
    b.print_render.base.scale = 0.8;
    let second = first.clone().with_params(b).unwrap();
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = material_probe();
    let first_pixels = first.process(image.clone(), &backend).unwrap();
    let second_pixels = second.process(image.clone(), &backend).unwrap();
    assert_material_pixels_changed(&second_pixels, &first_pixels, "A to B");
    let restored = second.with_params(a).unwrap();
    let restored_pixels = restored.process(image.clone(), &backend).unwrap();
    assert_material_pixels_close(&restored_pixels, &first_pixels, "A to B to A");
    let fresh = fresh_material_pipeline(&first, restored.params.clone());
    assert_material_pixels_close(
        &restored_pixels,
        &fresh.process(image.clone(), &backend).unwrap(),
        "restored fresh",
    );

    let mut exposed = restored.params.clone();
    exposed.camera.exposure_compensation_ev = 0.75;
    let exposed = restored.with_params(exposed).unwrap();
    let fresh = fresh_material_pipeline(&first, exposed.params.clone());
    let exposed_pixels = exposed.process(image.clone(), &backend).unwrap();
    assert_material_pixels_close(
        &exposed_pixels,
        &fresh.process(image, &backend).unwrap(),
        "exposure rebuild",
    );
    assert_material_pixels_changed(&exposed_pixels, &first_pixels, "exposure rebuild");
}

#[test]
fn monochrome_development_time_update_uses_raw_family() {
    let shared = build();
    let film = crate::profile::load_profile_by_name(&data_dir(), "kodak_doublex").unwrap();
    let mut params = deterministic_material_params(&shared);
    params.workflow.route = "input > film > scan".into();
    params.film_render.development_time = Some(4.0);
    let first =
        Pipeline::new_with_spectral(film, (*shared.original_print).clone(), params, &data_dir())
            .unwrap();
    assert_eq!(first.film.data.development_time, vec![4.0]);
    let mut params = first.params.clone();
    params.film_render.development_time = Some(12.0);
    let updated = first.clone().with_params(params).unwrap();
    let fresh = fresh_material_pipeline(&first, updated.params.clone());
    assert_eq!(
        updated.original_film.data.development_time,
        vec![4.0, 5.0, 6.5, 9.0, 12.0]
    );
    assert_eq!(updated.film.data.development_time, vec![12.0]);
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = material_probe();
    let actual = updated.process(image.clone(), &backend).unwrap();
    assert_material_pixels_close(
        &actual,
        &fresh.process(image.clone(), &backend).unwrap(),
        "BW development time",
    );
    assert_material_pixels_changed(
        &actual,
        &first.process(image, &backend).unwrap(),
        "BW development time",
    );
}

#[test]
fn edited_material_stage_round_trip_preserves_sources_and_continuation() {
    use crate::stage_execution::{StageResult, continue_stage};
    use sha2::{Digest, Sha256};
    let shared = build();
    let base = shared
        .clone()
        .with_params(deterministic_material_params(&shared))
        .unwrap();
    let mut params = base.params.clone();
    params.camera.exposure_compensation_ev = 0.5;
    params.film_render.base.scale = 1.4;
    params.film_render.base.yellow = 1.25;
    params.film_render.density_curve_gamma = 1.3;
    params.print_render.base.scale = 1.6;
    params.print_render.base.cyan = 0.8;
    let full = base.clone().with_params(params).unwrap();
    assert!(Arc::ptr_eq(&full.original_film, &base.original_film));
    assert!(Arc::ptr_eq(&full.original_print, &base.original_print));
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = material_probe();
    let mut input_digest = Sha256::new();
    for value in &image.data {
        input_digest.update(value.to_le_bytes());
    }
    let expected = fresh_material_pipeline(&base, full.params.clone())
        .process(image.clone(), &backend)
        .unwrap();
    let mut params = full.params.clone();
    params.workflow.route = "input > film".into();
    params.film_render.density_curve_gamma = base.params.film_render.density_curve_gamma;
    let producing = full.clone().with_params(params).unwrap();
    let prior_midgray = producing.midgray_density().to_vec();
    let mut params = producing.params.clone();
    params.film_render.density_curve_gamma = full.params.film_render.density_curve_gamma;
    let producing = producing.with_params(params).unwrap();
    let fresh = fresh_material_pipeline(&base, producing.params.clone());
    assert!(
        producing
            .midgray_density()
            .iter()
            .zip(&prior_midgray)
            .any(|(a, b)| a.is_finite() && b.is_finite() && (a - b).abs() > 1e-8)
    );
    for (actual, expected) in [
        (producing.midgray_density(), fresh.midgray_density()),
        (
            producing.compensated_midgray_density(),
            fresh.compensated_midgray_density(),
        ),
    ] {
        assert_eq!(actual.len(), expected.len());
        for (&a, &b) in actual.iter().zip(expected) {
            assert!(a == b || (a.is_nan() && b.is_nan()), "{a} != {b}");
        }
    }
    let mut film = producing.process_stage(image, &backend, 42).unwrap();
    film.header.provenance.input_identity = format!("{:x}", input_digest.finalize());
    film.header.provenance.backend = backend.name().to_owned();
    assert!(
        film.header
            .film
            .matches_source(&full.original_film)
            .unwrap()
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("film.sfstage");
    crate::stage_artifact::write(&path, &film).unwrap();
    let film = crate::stage_artifact::read(&path).unwrap();
    assert!(
        film.header
            .film
            .matches_source(&full.original_film)
            .unwrap()
    );
    let mut continuation = film.header.baseline.clone();
    continuation.workflow.route = "film > print".into();
    let StageResult::Material(print) = continue_stage(
        film,
        continuation,
        Some((*full.original_print).clone()),
        &data_dir(),
        &backend,
        42,
    )
    .unwrap() else {
        panic!("expected completed print")
    };
    assert!(
        print
            .header
            .film
            .matches_source(&full.original_film)
            .unwrap()
    );
    assert!(
        print
            .header
            .print
            .as_ref()
            .unwrap()
            .matches_source(&full.original_print)
            .unwrap()
    );
    assert!(
        print
            .header
            .original_print
            .as_ref()
            .unwrap()
            .matches_source(&full.original_print)
            .unwrap()
    );
    let path = dir.path().join("print.sfstage");
    crate::stage_artifact::write(&path, &print).unwrap();
    let print = crate::stage_artifact::read(&path).unwrap();
    assert!(
        print
            .header
            .print
            .as_ref()
            .unwrap()
            .matches_source(&full.original_print)
            .unwrap()
    );
    let mut continuation = print.header.baseline.clone();
    continuation.workflow.route = "print > scan".into();
    let StageResult::Image(actual) =
        continue_stage(print, continuation, None, &data_dir(), &backend, 42).unwrap()
    else {
        panic!("expected scanned image")
    };
    assert_material_pixels_close(&actual, &expected, "persisted material continuation");
}

#[test]
fn input_preparation_and_route_changes_preserve_pixels_and_material_calibration() {
    let base = build();
    let mut params = base.params.clone();
    params.workflow.route = "input".into();
    params.film_render.grain.active = false;
    params.print_render.glare.active = false;
    params.camera.auto_exposure = false;
    params.cmyk.active = false;
    params.io.input_color_space = "ACES2065-1".into();
    params.io.output_color_space = "ACES2065-1".into();
    params.io.input_cctf_decoding = false;
    params.io.output_cctf_encoding = false;
    let image = ImageBuf::from_data(
        4,
        3,
        (0..36).map(|i| from_f64(0.05 + i as f64 / 100.0)).collect(),
    );
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let input = Pipeline::new_with_spectral(
        (*base.original_film).clone(),
        (*base.original_print).clone(),
        params.clone(),
        &data_dir(),
    )
    .unwrap();
    assert!(input.spectral.is_none());
    let output = input.process(image.clone(), &backend).unwrap();
    for (actual, expected) in output.data.iter().zip(&image.data) {
        assert!((to_f64(*actual) - to_f64(*expected)).abs() < 1e-6);
    }

    params.camera.filter_uv = [0.5, 415.0, 9.0];
    let input = input.with_params(params.clone()).unwrap();
    assert!(input.spectral.is_none());
    params.workflow.route = base.params.workflow.route.clone();
    let film = input.with_params(params.clone()).unwrap();
    assert!(film.tc_lut().is_some());
    let fresh = Pipeline::new_with_spectral(
        (*base.original_film).clone(),
        (*base.original_print).clone(),
        params.clone(),
        &data_dir(),
    )
    .unwrap();
    assert_eq!(
        film.process(image.clone(), &backend).unwrap().data,
        fresh.process(image.clone(), &backend).unwrap().data,
    );
    params.workflow.route = "input".into();
    let input = film.with_params(params).unwrap();
    assert!(input.spectral.is_none());
    assert_eq!(input.process(image, &backend).unwrap().data, output.data);
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
/// A selected GPU whose minimum retained allocation exceeds its resources.
struct OvertaxedGpu {
    explicit: bool,
}

impl spektrafilm_backend::ComputeBackend for OvertaxedGpu {
    fn colorspace_convert(
        &self,
        img: &ImageBuf,
        matrix: &[[f32; 3]; 3],
    ) -> Result<ImageBuf, String> {
        spektrafilm_backend::cpu_backend::CpuBackend.colorspace_convert(img, matrix)
    }
    fn cctf_encode_srgb(&self, img: &ImageBuf) -> Result<ImageBuf, String> {
        spektrafilm_backend::cpu_backend::CpuBackend.cctf_encode_srgb(img)
    }
    fn cctf_decode_srgb(&self, img: &ImageBuf) -> Result<ImageBuf, String> {
        spektrafilm_backend::cpu_backend::CpuBackend.cctf_decode_srgb(img)
    }
    fn gaussian_blur(&self, img: &ImageBuf, sigma: f32) -> Result<ImageBuf, String> {
        spektrafilm_backend::cpu_backend::CpuBackend.gaussian_blur(img, sigma)
    }
    fn table_lookup(
        &self,
        img: &ImageBuf,
        table_x: &[f32],
        table_y: &[[f32; 3]],
    ) -> Result<ImageBuf, String> {
        spektrafilm_backend::cpu_backend::CpuBackend.table_lookup(img, table_x, table_y)
    }
    fn lut3d_interp(
        &self,
        img: &ImageBuf,
        lut: &spektrafilm_backend::Lut3D,
    ) -> Result<ImageBuf, String> {
        spektrafilm_backend::cpu_backend::CpuBackend.lut3d_interp(img, lut)
    }
    fn is_gpu(&self) -> bool {
        true
    }
    fn explicit_gpu_request(&self) -> bool {
        self.explicit
    }
    fn retain_image(
        &self,
        _image: &ImageBuf,
        _meta: &spektrafilm_backend::RetainedImageMeta,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        let selection = if self.explicit {
            "explicit GPU request"
        } else {
            "selected GPU"
        };
        Err(format!(
            "{selection} exceeds the device resource limits: minimum retained allocation exceeds the synthetic test limit"
        ))
    }
    fn name(&self) -> &str {
        "overtaxed-test-gpu"
    }
}

#[test]
fn selected_gpu_fails_when_the_device_cannot_allocate_retained_input() {
    let base = build();
    let mut pipeline = Pipeline::new((*base.film).clone(), (*base.print).clone(), base.params);
    pipeline.params.film_render.grain.active = false;
    let err = pipeline
        .process(ImageBuf::new(8, 6), &OvertaxedGpu { explicit: true })
        .unwrap_err();
    assert!(
        err.contains("explicit GPU request exceeds the device resource limits"),
        "{err}"
    );
    // Observation wrapping must preserve retained resource errors as well.
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
    // Automatic intent must not demote a backend already selected as GPU.
    let err = pipeline
        .process(ImageBuf::new(8, 6), &OvertaxedGpu { explicit: false })
        .unwrap_err();
    assert!(
        err.contains("selected GPU exceeds the device resource limits"),
        "{err}"
    );
    let automatic = OvertaxedGpu { explicit: false };
    let observed = spektrafilm_backend::bind_backend(
        &automatic,
        spektrafilm_backend::telemetry::ObservationContext::new(
            spektrafilm_backend::telemetry::CollectionMode::Off,
        ),
    );
    let err = pipeline
        .process(ImageBuf::new(8, 6), observed.as_ref())
        .unwrap_err();
    assert!(
        err.contains("selected GPU exceeds the device resource limits"),
        "{err}"
    );
}

/// An automatically selected device that declines optional image hooks.
/// Required image methods count any accidental host delegation separately.
#[derive(Default)]
struct DecliningSelectedGpu {
    host_calls: std::sync::atomic::AtomicUsize,
    meter_calls: std::sync::atomic::AtomicUsize,
    mix_calls: std::sync::atomic::AtomicUsize,
    conversion_calls: std::sync::atomic::AtomicUsize,
}

impl ComputeBackend for DecliningSelectedGpu {
    fn colorspace_convert(
        &self,
        image: &ImageBuf,
        matrix: &[[f32; 3]; 3],
    ) -> Result<ImageBuf, String> {
        self.host_calls.fetch_add(1, Ordering::Relaxed);
        spektrafilm_backend::cpu_backend::CpuBackend.colorspace_convert(image, matrix)
    }
    fn cctf_encode_srgb(&self, image: &ImageBuf) -> Result<ImageBuf, String> {
        self.host_calls.fetch_add(1, Ordering::Relaxed);
        spektrafilm_backend::cpu_backend::CpuBackend.cctf_encode_srgb(image)
    }
    fn cctf_decode_srgb(&self, image: &ImageBuf) -> Result<ImageBuf, String> {
        self.host_calls.fetch_add(1, Ordering::Relaxed);
        spektrafilm_backend::cpu_backend::CpuBackend.cctf_decode_srgb(image)
    }
    fn gaussian_blur(&self, image: &ImageBuf, sigma: f32) -> Result<ImageBuf, String> {
        self.host_calls.fetch_add(1, Ordering::Relaxed);
        spektrafilm_backend::cpu_backend::CpuBackend.gaussian_blur(image, sigma)
    }
    fn table_lookup(
        &self,
        image: &ImageBuf,
        x: &[f32],
        y: &[[f32; 3]],
    ) -> Result<ImageBuf, String> {
        self.host_calls.fetch_add(1, Ordering::Relaxed);
        spektrafilm_backend::cpu_backend::CpuBackend.table_lookup(image, x, y)
    }
    fn lut3d_interp(
        &self,
        image: &ImageBuf,
        lut: &spektrafilm_backend::Lut3D,
    ) -> Result<ImageBuf, String> {
        self.host_calls.fetch_add(1, Ordering::Relaxed);
        spektrafilm_backend::cpu_backend::CpuBackend.lut3d_interp(image, lut)
    }
    fn meter_autoexposure(
        &self,
        _: &ImageBuf,
        _: &spektrafilm_backend::MeterAutoexposureParams,
    ) -> Result<Option<f64>, String> {
        self.meter_calls.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }
    fn mix_output(
        &self,
        _: &ImageBuf,
        _: &ImageBuf,
        _: &spektrafilm_backend::OutputMixGpuParams,
    ) -> Result<Option<ImageBuf>, String> {
        self.mix_calls.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }
    fn convert_colorspace(
        &self,
        _: &ImageBuf,
        _: &spektrafilm_backend::ConvertColorspaceGpuParams,
    ) -> Result<Option<ImageBuf>, String> {
        self.conversion_calls.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }
    fn is_gpu(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "declining-automatically-selected-gpu"
    }
}

#[test]
fn selected_automatic_gpu_meter_decline_never_returns_host_measurement() {
    let mut pipeline = build();
    pipeline.params.camera.auto_exposure = true;
    let backend = DecliningSelectedGpu::default();
    assert!(!backend.explicit_gpu_request());
    let error = pipeline
        .meter_autoexposure(&ImageBuf::new(8, 6), &backend)
        .unwrap_err();
    assert!(
        error.contains("full-source auto-exposure metering"),
        "{error}"
    );
    assert_eq!(backend.meter_calls.load(Ordering::Relaxed), 1);
    assert_eq!(backend.host_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn selected_automatic_gpu_mix_decline_never_returns_host_composition() {
    let mut pipeline = build();
    let backend = DecliningSelectedGpu::default();
    // The source endpoint and an interior blend both require the device hook.
    for amount in [0.0, 0.5] {
        pipeline.params.output_mix.amount = amount;
        let error = pipeline
            .compose_output_mix(
                ImageBuf::new(8, 6),
                Some(ImageBuf::new(8, 6)),
                &backend,
                None,
            )
            .unwrap_err();
        assert!(error.contains("output_mix composition"), "{error}");
    }
    assert_eq!(backend.mix_calls.load(Ordering::Relaxed), 2);
    assert_eq!(backend.host_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn selected_gpu_mix_source_rejects_host_preparation_before_color_resolution() {
    let mut pipeline = build();
    // A device-route rejection must precede any host source preparation.
    pipeline.params.io.input_color_space = "unresolved-host-source".into();
    let backend = DecliningSelectedGpu::default();
    let error = pipeline
        .prepare_output_mix_source(&ImageBuf::new(8, 6), &backend)
        .unwrap_err();
    assert!(
        error.contains("output_mix source requires the retained device route"),
        "{error}"
    );
    assert_eq!(backend.host_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn selected_automatic_gpu_conversion_decline_never_returns_host_pixels() {
    let mut pipeline = build();
    pipeline.params.workflow.route = "input".into();
    // The route forms no film record, so Dye has nothing to condition on; this case
    // exercises the conversion decline, not grain.
    pipeline.params.film_render.grain.active = false;
    pipeline.params.io.crop = false;
    pipeline.params.io.upscale_factor = 1.0;
    let backend = DecliningSelectedGpu::default();
    // Exercise the optional per-stage hook independently of the retained
    // production ingress, which rejects a missing retained implementation.
    let error = pipeline
        .process_full(ImageBuf::new(8, 6), &backend, None, None)
        .unwrap_err();
    assert!(error.contains("input route conversion"), "{error}");
    assert_eq!(backend.conversion_calls.load(Ordering::Relaxed), 1);
    assert_eq!(backend.host_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn selected_automatic_gpu_retention_failure_does_not_enter_host_stages() {
    let base = build();
    let mut pipeline = Pipeline::new((*base.film).clone(), (*base.print).clone(), base.params);
    pipeline.params.workflow.route = "input".into();
    pipeline.params.film_render.grain.active = false;
    let backend = DecliningSelectedGpu::default();
    let error = pipeline.process(ImageBuf::new(8, 6), &backend).unwrap_err();
    assert!(error.contains("does not retain device images"), "{error}");
    assert_eq!(backend.host_calls.load(Ordering::Relaxed), 0);
    assert_eq!(backend.conversion_calls.load(Ordering::Relaxed), 0);
    let error = pipeline
        .process_resident_borrowed(&ImageBuf::new(8, 6), &backend)
        .unwrap_err();
    assert!(error.contains("does not retain device images"), "{error}");
    assert_eq!(backend.host_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn public_gpu_diagnostic_retention_errors_never_enter_host_tap_stages() {
    let base = build();
    let mut pipeline = Pipeline::new((*base.film).clone(), (*base.print).clone(), base.params);
    pipeline.params.film_render.grain.active = false;
    pipeline.params.camera.auto_exposure = true;
    let backend = DecliningSelectedGpu::default();
    for (inject, collect) in [
        (Tap::RgbIn, Tap::RgbPre),
        (Tap::RgbPre, Tap::LogEFilm),
        (Tap::LogEFilm, Tap::CmyFilm),
        (Tap::CmyFilm, Tap::RgbOut),
        (Tap::LogEPrint, Tap::CmyPrint),
        (Tap::RgbOut, Tap::RgbOut),
    ] {
        let error = pipeline
            .process_with_taps(
                ImageBuf::from_data(2, 2, vec![from_f64(0.18); 12]),
                &backend,
                Some(inject),
                Some(collect),
            )
            .unwrap_err();
        assert!(
            error.contains("does not retain device images"),
            "{inject:?}->{collect:?}: {error}"
        );
        pipeline.params.taps.inject = Some(inject.name().into());
        pipeline.params.taps.collect = Some(collect.name().into());
        let error = pipeline
            .process_resident_borrowed(
                &ImageBuf::from_data(2, 2, vec![from_f64(0.18); 12]),
                &backend,
            )
            .unwrap_err();
        assert!(
            error.contains("does not retain device images"),
            "borrowed {inject:?}->{collect:?}: {error}"
        );
    }
    assert_eq!(backend.host_calls.load(Ordering::Relaxed), 0);
    assert_eq!(backend.meter_calls.load(Ordering::Relaxed), 0);
    assert_eq!(backend.conversion_calls.load(Ordering::Relaxed), 0);
    assert_eq!(backend.mix_calls.load(Ordering::Relaxed), 0);
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

/// `with_params` must refresh the resolved conversion tone when the style
/// changes: the tone is resolved once per pipeline, never re-read per frame.
///
/// No style ships with the product, so the test data directory is overlaid
/// with a user-authored `scan-styles/warm-probe.json`.
#[test]
fn with_params_resolves_a_changed_conversion_style() {
    let data = data_dir();
    let overlay = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(&data).unwrap() {
        let entry = entry.unwrap();
        std::os::unix::fs::symlink(entry.path(), overlay.path().join(entry.file_name())).unwrap();
    }
    let styles = overlay.path().join("scan-styles");
    std::fs::create_dir(&styles).unwrap();
    std::fs::write(
        styles.join("warm-probe.json"),
        r#"{"channels":[
            {"brightness_ev":0.9281766618,"contrast":0.73551400058216643,"dmax":1.5625841465572565},
            {"brightness_ev":0.8781766618,"contrast":0.73551400058216643,"dmax":1.5625841465572565},
            {"brightness_ev":0.8281766618,"contrast":0.73551400058216643,"dmax":1.5625841465572565}
        ]}"#,
    )
    .unwrap();

    let film = crate::profile::load_profile_by_name(overlay.path(), "kodak_portra_400").unwrap();
    let print =
        crate::profile::load_profile_by_name(overlay.path(), "kodak_portra_endura").unwrap();
    let base =
        Pipeline::new_with_spectral(film, print, RuntimeParams::default(), overlay.path()).unwrap();
    let mut params = base.params.clone();
    params.workflow.route = "input > film > scan".into();
    params.io.scan_film = true;
    params.scanner.scan_output = "negative_to_positive".into();
    params.scanner.conversion_style = "warm-probe".into();
    let edited = base.with_params(params).unwrap();
    let warm = crate::stages::scanning::style::resolve("warm-probe", Some(overlay.path())).unwrap();
    assert_eq!(
        edited.conversion_tones[0].brightness_ev,
        warm[0].brightness_ev
    );

    // Back to neutral restores the frozen reference tone.
    let mut params = edited.params.clone();
    params.scanner.conversion_style = "neutral".into();
    let neutral = edited.with_params(params).unwrap();
    assert_eq!(
        neutral.conversion_tones[0].brightness_ev,
        crate::stages::scanning::style::STANDARD[0].brightness_ev
    );
}

/// A monochrome stock must reach the silver path: the pipeline renders
/// `kodak_doublex` (five development-time columns, one emulsion value per
/// sample) with grain on and off, and the grain must cover the frame,
/// stay bounded in density and be seed-deterministic. This exercises the
/// single-curve record adaptation and the resolved reference on a real
/// profile through the film-density boundary.
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
fn preparation_components_share_only_compatible_results() {
    let shared = build();
    let base = shared
        .clone()
        .with_params(deterministic_material_params(&shared))
        .unwrap();
    let mut params = base.params.clone();
    params.io.output_gamut_compress.knee[0] = 0.8;
    let output = base.clone().with_params(params).unwrap();
    assert!(Arc::ptr_eq(&base.film, &output.film));
    assert!(Arc::ptr_eq(&base.print, &output.print));
    assert!(Arc::ptr_eq(
        base.spectral.as_ref().unwrap(),
        output.spectral.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        base.gray.as_ref().unwrap(),
        output.gray.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        base.print_calibration.as_ref().unwrap(),
        output.print_calibration.as_ref().unwrap()
    ));
    assert!(!Arc::ptr_eq(&base.output_gamut, &output.output_gamut));

    let mut params = output.params.clone();
    params.enlarger.m_filter_shift += 3.0;
    let print = output.clone().with_params(params).unwrap();
    assert!(Arc::ptr_eq(
        output.gray.as_ref().unwrap(),
        print.gray.as_ref().unwrap()
    ));
    assert!(!Arc::ptr_eq(
        output.print_calibration.as_ref().unwrap(),
        print.print_calibration.as_ref().unwrap()
    ));

    let mut params = print.params.clone();
    params.film_render.base.scale *= 1.1;
    let material = print.clone().with_params(params).unwrap();
    assert!(Arc::ptr_eq(
        print.spectral.as_ref().unwrap(),
        material.spectral.as_ref().unwrap()
    ));
    assert!(!Arc::ptr_eq(&print.film, &material.film));
    assert!(Arc::ptr_eq(&print.print, &material.print));
    assert!(!Arc::ptr_eq(
        print.gray.as_ref().unwrap(),
        material.gray.as_ref().unwrap()
    ));
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = material_probe();
    for (name, updated) in [
        ("output", &output),
        ("print", &print),
        ("material", &material),
    ] {
        let cold = fresh_material_pipeline(&base, updated.params.clone());
        let actual = updated.process(image.clone(), &backend).unwrap();
        let expected = cold.process(image.clone(), &backend).unwrap();
        assert_material_pixels_close(&actual, &expected, name);
    }
    let mut params = base.params.clone();
    params.settings.rgb_to_raw_method = "mallett2019".into();
    let mallett = base.clone().with_params(params).unwrap();
    let mut params = mallett.params.clone();
    params.settings.apply_hanatos2025_adaptation_window =
        !params.settings.apply_hanatos2025_adaptation_window;
    params.settings.apply_hanatos2025_adaptation_surface =
        !params.settings.apply_hanatos2025_adaptation_surface;
    params.settings.spectral_gaussian_blur = 8.0;
    params.io.input_gamut_compress.algorithm = "oklch".into();
    params.io.input_gamut_compress.knee[0] = 0.7;
    let mallett_edited = mallett.clone().with_params(params).unwrap();
    assert!(Arc::ptr_eq(
        mallett.spectral.as_ref().unwrap(),
        mallett_edited.spectral.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        mallett.gray.as_ref().unwrap(),
        mallett_edited.gray.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        mallett.print_calibration.as_ref().unwrap(),
        mallett_edited.print_calibration.as_ref().unwrap()
    ));
    assert_eq!(
        mallett.process(image.clone(), &backend).unwrap().data,
        mallett_edited
            .process(image.clone(), &backend)
            .unwrap()
            .data,
    );

    let mut params = deterministic_material_params(&shared);
    params.film_render.base.active = false;
    params.print_render.base.active = false;
    params.film_render.chemistry.active = false;
    let base = shared.with_params(params).unwrap();
    let mut params = base.params.clone();
    params.film_render.base.scale = 1.7;
    params.print_render.base.cyan = 0.7;
    params.film_render.chemistry.gamma_factor = 1.5;
    let inactive = base.clone().with_params(params.clone()).unwrap();
    assert!(Arc::ptr_eq(&base.film, &inactive.film));
    assert!(Arc::ptr_eq(&base.print, &inactive.print));
    assert!(Arc::ptr_eq(
        base.gray.as_ref().unwrap(),
        inactive.gray.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        base.print_calibration.as_ref().unwrap(),
        inactive.print_calibration.as_ref().unwrap()
    ));
    assert_material_pixels_close(
        &inactive.process(image.clone(), &backend).unwrap(),
        &base.process(image.clone(), &backend).unwrap(),
        "inactive material controls",
    );
    let fresh_inactive = fresh_material_pipeline(&base, inactive.params.clone());
    assert_material_pixels_close(
        &inactive.process(image.clone(), &backend).unwrap(),
        &fresh_inactive.process(image.clone(), &backend).unwrap(),
        "inactive material controls fresh",
    );

    params.film_render.base.active = true;
    params.print_render.base.active = true;
    params.film_render.chemistry.active = true;
    let active = inactive.clone().with_params(params).unwrap();
    assert!(!Arc::ptr_eq(&inactive.film, &active.film));
    assert!(!Arc::ptr_eq(&inactive.print, &active.print));
    let fresh = fresh_material_pipeline(&base, active.params.clone());
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = material_probe();
    assert_material_pixels_close(
        &active.process(image.clone(), &backend).unwrap(),
        &fresh.process(image, &backend).unwrap(),
        "re-enabled material controls",
    );
}

#[test]
fn inactive_camera_filters_keep_preparation_and_pixels() {
    let shared = build();
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = material_probe();
    for method in ["hanatos2025", "mallett2019"] {
        for amplitudes in [[0.0, 0.0], [0.5, 0.0], [0.0, 0.5]] {
            let mut params = deterministic_material_params(&shared);
            params.settings.rgb_to_raw_method = method.into();
            params.camera.filter_uv[0] = amplitudes[0];
            params.camera.filter_ir[0] = amplitudes[1];
            let base = shared.clone().with_params(params).unwrap();
            let mut params = base.params.clone();
            if amplitudes[0] == 0.0 {
                params.camera.filter_uv[1] = 450.0;
                params.camera.filter_uv[2] = 11.0;
            }
            if amplitudes[1] == 0.0 {
                params.camera.filter_ir[1] = 620.0;
                params.camera.filter_ir[2] = 29.0;
            }
            let updated = base.clone().with_params(params).unwrap();
            assert!(
                Arc::ptr_eq(
                    base.spectral.as_ref().unwrap(),
                    updated.spectral.as_ref().unwrap()
                ),
                "{method}: {amplitudes:?}"
            );
            assert_eq!(
                base.process(image.clone(), &backend).unwrap().data,
                updated.process(image.clone(), &backend).unwrap().data
            );
        }
    }
}

#[test]
fn cpu_cat16_updates_only_dependent_references() {
    let shared = build();
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = material_probe();
    for method in ["hanatos2025", "mallett2019"] {
        let mut params = deterministic_material_params(&shared);
        params.settings.rgb_to_raw_method = method.into();
        let base = shared.clone().with_params(params).unwrap();
        let mut params = base.params.clone();
        params.settings.use_cat16 = !params.settings.use_cat16;
        let updated = base.clone().with_params(params).unwrap();
        assert!(
            Arc::ptr_eq(
                base.spectral.as_ref().unwrap(),
                updated.spectral.as_ref().unwrap()
            ),
            "{method}"
        );
        assert_eq!(
            Arc::ptr_eq(base.gray.as_ref().unwrap(), updated.gray.as_ref().unwrap()),
            method == "mallett2019"
        );
        let cold = fresh_material_pipeline(&base, updated.params.clone());
        assert_material_pixels_close(
            &updated.process(image.clone(), &backend).unwrap(),
            &cold.process(image.clone(), &backend).unwrap(),
            method,
        );
    }
}

#[test]
fn model_less_film_ignores_chemistry_for_reuse() {
    let shared = build();
    let mut film = (*shared.original_film).clone();
    film.data.density_curves_model = None;
    let params = deterministic_material_params(&shared);
    let base =
        Pipeline::new_with_spectral(film, (*shared.original_print).clone(), params, &data_dir())
            .unwrap();
    let mut params = base.params.clone();
    params.film_render.chemistry.gamma_factor = 1.5;
    params.film_render.chemistry.active = !params.film_render.chemistry.active;
    let updated = base.clone().with_params(params).unwrap();
    assert!(Arc::ptr_eq(&base.film, &updated.film));
    assert!(Arc::ptr_eq(
        base.gray.as_ref().unwrap(),
        updated.gray.as_ref().unwrap()
    ));
}

#[test]
fn input_only_skips_print_model_and_neutral_database() {
    let shared = build();
    let mut print = (*shared.original_print).clone();
    print.data.density_curves_model.as_mut().unwrap().model_type = "invalid-model".into();
    let mut params = deterministic_material_params(&shared);
    params.workflow.route = "input".into();
    params.settings.neutral_print_filters_from_database = true;
    let empty = tempfile::tempdir().unwrap();
    std::fs::create_dir(empty.path().join("filters")).unwrap();
    std::fs::write(
        empty.path().join("filters/neutral_print_filters.json"),
        "not json",
    )
    .unwrap();
    let input = Pipeline::new_with_spectral(
        (*shared.original_film).clone(),
        print,
        params.clone(),
        empty.path(),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&input.print, &input.original_print));
    assert!(input.spectral.is_none());
    assert!(input.gray.is_none());
    assert!(input.print_calibration.is_none());
    params.print_render.base.scale = 1.5;
    let updated = input.clone().with_params(params.clone()).unwrap();
    assert!(Arc::ptr_eq(&input.print, &updated.print));
    params.workflow.route = "input > film > print > scan".into();
    let error = match updated.with_params(params) {
        Ok(_) => panic!("print route must validate the print model"),
        Err(error) => error,
    };
    assert!(
        error.contains("invalid print density-curve model"),
        "{error}"
    );
}

#[test]
fn updated_bw_scan_validates_new_development_time() {
    let shared = build();
    let mut film = crate::profile::load_profile_by_name(&data_dir(), "kodak_doublex").unwrap();
    film.data.density_curves_model = None;
    film.data.density_curves[0][0] = f64::NAN;
    let mut params = deterministic_material_params(&shared);
    params.workflow.route = "input > film > scan".into();
    params.scanner.scan_output = "direct_scan".into();
    params.film_render.development_time = Some(4.0);
    let first =
        Pipeline::new_with_spectral(film, (*shared.original_print).clone(), params, &data_dir())
            .unwrap();
    assert!(!first.film.data.density_curves[0][0].is_finite());
    let mut params = first.params.clone();
    params.film_render.development_time = Some(12.0);
    params.scanner.scan_output = "negative_to_positive".into();
    let updated = first.with_params(params.clone()).unwrap();
    let cold = fresh_material_pipeline(&updated, params);
    assert_eq!(
        updated.film.data.density_curves,
        cold.film.data.density_curves
    );
}

#[test]
fn scan_checks_resolved_curves_before_chemistry_on_cold_and_warm_paths() {
    let shared = build();
    let mut film = (*shared.original_film).clone();
    film.data.density_curves[0][0] = f64::NAN;
    let mut params = deterministic_material_params(&shared);
    params.workflow.route = "input > film > scan".into();
    params.scanner.scan_output = "direct_scan".into();
    let first = Pipeline::new_with_spectral(
        film.clone(),
        (*shared.original_print).clone(),
        params.clone(),
        &data_dir(),
    )
    .unwrap();
    assert!(first.film.data.density_curves[0][0].is_finite());
    params.scanner.scan_output = "negative_to_positive".into();
    let warm_error = match first.with_params(params.clone()) {
        Ok(_) => panic!("raw non-finite curve must reject the update"),
        Err(error) => error,
    };
    let cold_error = match Pipeline::new_with_spectral(
        film,
        (*shared.original_print).clone(),
        params,
        &data_dir(),
    ) {
        Ok(_) => panic!("raw non-finite curve must reject construction"),
        Err(error) => error,
    };
    assert!(
        warm_error.contains("requires finite density curves"),
        "{warm_error}"
    );
    assert_eq!(warm_error, cold_error);
}

#[test]
fn positive_scan_rejection_precedes_invalid_film_model() {
    let shared = build();
    let mut film =
        crate::profile::load_profile_by_name(&data_dir(), "fujifilm_velvia_100").unwrap();
    film.data.density_curves_model.as_mut().unwrap().model_type = "invalid-model".into();
    let mut params = deterministic_material_params(&shared);
    params.workflow.route = "input > film > scan".into();
    params.scanner.scan_output = "negative_to_positive".into();
    let error = match Pipeline::new_with_spectral(
        film,
        (*shared.original_print).clone(),
        params,
        &data_dir(),
    ) {
        Ok(_) => panic!("positive film must be rejected"),
        Err(error) => error,
    };
    assert!(
        error.contains("requires a negative filming-stage film profile"),
        "{error}"
    );
}

#[test]
fn rejected_candidate_preserves_preparation_and_pixels() {
    let shared = build();
    let mut initial = deterministic_material_params(&shared);
    initial.workflow.route = "input > film > scan".into();
    initial.scanner.scan_output = "negative_to_positive".into();
    let pipeline = shared.clone().with_params(initial).unwrap();
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let image = material_probe();
    let before = pipeline.process(image.clone(), &backend).unwrap();
    let film = Arc::clone(&pipeline.film);
    let print = Arc::clone(&pipeline.print);
    let spectral = pipeline.spectral.clone().unwrap();
    let gray = pipeline.gray.clone().unwrap();
    let print_calibration = pipeline.print_calibration.clone().unwrap();
    let mut params = pipeline.params.clone();
    params.film_render.base.scale = 1.5;
    params.scanner.conversion_style = "missing-preparation-regression-style".into();
    let error = match pipeline.prepare_parameter_update(params, &backend) {
        Ok(_) => panic!("missing style must reject the candidate"),
        Err(error) => error,
    };
    assert!(error.contains("scanner.conversion_style"), "{error}");
    assert!(Arc::ptr_eq(&film, &pipeline.film));
    assert!(Arc::ptr_eq(&print, &pipeline.print));
    assert!(Arc::ptr_eq(&spectral, pipeline.spectral.as_ref().unwrap()));
    assert!(Arc::ptr_eq(&gray, pipeline.gray.as_ref().unwrap()));
    assert!(Arc::ptr_eq(
        &print_calibration,
        pipeline.print_calibration.as_ref().unwrap()
    ));
    assert_ne!(pipeline.params.film_render.base.scale, 1.5);
    let after = pipeline.process(image, &backend).unwrap();
    assert_eq!(before.data, after.data);
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
        let fresh = Pipeline::new_with_spectral(
            (*base.original_film).clone(),
            (*base.original_print).clone(),
            params,
            &dir,
        )
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
    let updated = inherited.clone().with_params(explicit.clone()).unwrap();
    assert!(Arc::ptr_eq(
        inherited.spectral.as_ref().unwrap(),
        updated.spectral.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        inherited.gray.as_ref().unwrap(),
        updated.gray.as_ref().unwrap()
    ));
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
        let fresh = Pipeline::new_with_spectral(
            (*base.original_film).clone(),
            (*base.original_print).clone(),
            params,
            &data_dir(),
        )
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
fn dye_final_crop_preserves_scanner_and_print_boundaries() {
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
            // Inject a small developed-exposure field at photographic pitch.
            let render = |pipeline: &Pipeline| {
                pipeline
                    .run_from(
                        image.clone(),
                        Tap::LogEFilm,
                        Tap::RgbOut,
                        &backend,
                        Some((3.1, 0.0)),
                        None,
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
    // The matrix changes the route for every case, and a route change rebuilds the
    // spectral pipeline from the calibration data. A `Pipeline::new` base has no data
    // directory and cannot rebuild, so this case table needs the prepared constructor.
    let base = Pipeline::new_with_spectral(film, print, RuntimeParams::default(), &dir).unwrap();

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
                crate::stages::grain_dye::geometry(&source, &pipeline.params, 2.0).unwrap();
            let prepared = pipeline
                .prepare_output_mix_source(&source, &spektrafilm_backend::cpu_backend::CpuBackend)
                .unwrap();
            assert_eq!(
                (prepared.width, prepared.height),
                (geometry.width, geometry.height)
            );
            let (full, crop) = geometry.full_film_grid(&source).unwrap();
            let cropped =
                crate::stages::grain_dye::crop_output(ImageBuf::new(full.width, full.height), crop);
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
    // Dye realizes the field on the film lattice and crops at readout, so the film
    // density boundary is the full frame rather than the cropped working raster.
    assert_eq!(stage(&manifest, "log_e_film")["layout"]["width"], 8);
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

#[test]
fn standard_setup_reads_the_frame_without_rewriting_density_evidence() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > scan".into();
    params.io.scan_film = true;
    params.scanner.scan_output = "negative_to_positive".into();
    params.camera.auto_exposure = false;
    params.film_render.dir_couplers.active = false;
    params.debug.deactivate_spatial_effects = true;
    params.debug.deactivate_stochastic_effects = true;
    params.scanner.unsharp_mask = [0.0; 2];
    params.io.output_cctf_encoding = false;
    params.io.output_gamut_compress.algorithm = "off".into();
    let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let make = |ev: f64| {
        ImageBuf::from_data(
            8,
            1,
            (-3..5)
                .flat_map(|i| [from_f64((i as f64 * 0.3 + ev) * std::f64::consts::LOG10_2); 3])
                .collect(),
        )
    };
    let mut means = Vec::new();
    for ev in [-1.0, 0.0, 1.0] {
        let input = make(ev);
        let original = input.data.clone();
        let density = pipeline
            .process_with_taps(
                input.clone(),
                &backend,
                Some(Tap::LogEFilm),
                Some(Tap::CmyFilm),
            )
            .unwrap();
        let rgb = pipeline
            .process_with_taps(
                input.clone(),
                &backend,
                Some(Tap::LogEFilm),
                Some(Tap::RgbOut),
            )
            .unwrap();
        let replay = pipeline
            .process_with_taps(
                density.clone(),
                &backend,
                Some(Tap::CmyFilm),
                Some(Tap::RgbOut),
            )
            .unwrap();
        assert_eq!(input.data, original);
        assert_eq!(
            rgb.data, replay.data,
            "density tap must reproduce the same full-frame setup"
        );
        assert!(rgb.data.iter().all(|v| v.is_finite()));
        means.push(rgb.data.iter().map(|v| to_f64(*v)).sum::<f64>() / rgb.data.len() as f64);
        let repeated = pipeline
            .process_with_taps(input, &backend, Some(Tap::LogEFilm), Some(Tap::RgbOut))
            .unwrap();
        assert_eq!(rgb.data, repeated.data);
    }
    assert!(means.windows(2).all(|w| w[0] < w[1]), "{means:?}");
}

#[test]
fn completed_film_and_print_splits_preserve_exact_cpu_math_and_source_geometry() {
    use crate::params::GrainEngine;
    use crate::stage_execution::{StageResult, continue_stage};
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let input = ImageBuf::from_data(
        16,
        12,
        (0..16 * 12 * 3)
            .map(|i| from_f64(0.03 + (i % 53) as f64 / 90.0))
            .collect(),
    );
    for engine in [
        GrainEngine::Spektra,
        GrainEngine::Procedural,
        GrainEngine::Dye,
        GrainEngine::Fotufilm,
    ] {
        let mut full = build();
        full.params.camera.auto_exposure = true;
        full.params.camera.film_format_mm = if engine == GrainEngine::Dye {
            8.0
        } else {
            35.0
        };
        full.params.random_seed = 42;
        full.params.film_render.grain.active = true;
        full.params.film_render.grain.engine = engine;
        full.params.film_render.grain.rms_granularity = [6.0, 8.0, 10.0];
        full.params.io.crop = true;
        full.params.io.crop_size = [0.75, 0.625];
        full.params.io.upscale_factor = 0.5;
        full.params.scanner.lens_blur = 0.0;
        full.params.scanner.unsharp_mask = [0.0; 2];
        full.params.print_render.glare.active = false;
        full.params.enlarger.diffusion_filter.active = false;
        full.params.output_mix.amount = 0.35;
        let expected = full.process(input.clone(), &backend).unwrap();
        let mut film_pipeline = full.clone();
        film_pipeline.params.workflow.route = "input > film".into();
        let film = film_pipeline
            .process_stage(input.clone(), &backend, 42)
            .unwrap();
        let frozen = film.density.data.clone();
        let mut continuation = film.header.baseline.clone();
        continuation.workflow.route = "film > print > scan".into();
        let result = continue_stage(
            film.clone(),
            continuation,
            Some((*full.original_print).clone()),
            &data_dir(),
            &backend,
            42,
        )
        .unwrap();
        let StageResult::Image(actual) = result else {
            panic!("expected scanned image")
        };
        assert_eq!(actual.data, expected.data, "film split {engine:?}");
        assert_eq!(
            film.density.data, frozen,
            "stored grain must not be regenerated"
        );
        let mut print_pipeline = full.clone();
        print_pipeline.params.workflow.route = "input > film > print".into();
        let print = print_pipeline
            .process_stage(input.clone(), &backend, 42)
            .unwrap();
        let mut continuation = print.header.baseline.clone();
        continuation.workflow.route = "print > scan".into();
        let StageResult::Image(actual) =
            continue_stage(print, continuation, None, &data_dir(), &backend, 42).unwrap()
        else {
            panic!("expected scanned image")
        };
        assert_eq!(actual.data, expected.data, "print split {engine:?}");
    }
}

#[test]
fn negative_scan_split_keeps_standard_frame_setup_and_fixed_style_compensation() {
    use crate::stage_execution::{StageResult, continue_stage};
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let input = ImageBuf::from_data(
        9,
        5,
        (0..9 * 5 * 3)
            .map(|i| from_f64(0.02 + (i % 19) as f64 / 40.0))
            .collect(),
    );
    let directory = tempfile::tempdir().unwrap();
    let styles = directory.path().join("scan-styles");
    std::fs::create_dir(&styles).unwrap();
    std::fs::write(styles.join("fixed-probe.json"), r#"{"channels":[{"brightness_ev":0.9281766618,"contrast":0.73551400058216643,"dmax":1.5625841465572565},{"brightness_ev":0.8781766618,"contrast":0.73551400058216643,"dmax":1.5625841465572565},{"brightness_ev":0.8281766618,"contrast":0.73551400058216643,"dmax":1.5625841465572565}]}"#).unwrap();
    for style in ["standard", "fixed-probe"] {
        let mut full = build();
        full.params.workflow.route = "input > film > scan".into();
        full.params.io.scan_film = true;
        full.params.film_render.grain.active = false;
        full.params.camera.auto_exposure = false;
        full.params.camera.exposure_compensation_ev = 0.75;
        full.params.scanner.scan_output = "negative_to_positive".into();
        full.params.scanner.conversion_style = style.into();
        full.conversion_tones =
            crate::stages::scanning::style::resolve(style, Some(directory.path())).unwrap();
        let expected = full.process(input.clone(), &backend).unwrap();
        let mut producing = full.clone();
        producing.params.workflow.route = "input > film".into();
        producing.params.scanner.scan_output = "direct_scan".into();
        let film = producing.process_stage(input.clone(), &backend, 7).unwrap();
        let mut params = film.header.baseline.clone();
        params.workflow.route = "film > scan".into();
        params.scanner.scan_output = "negative_to_positive".into();
        let StageResult::Image(actual) =
            continue_stage(film, params, None, directory.path(), &backend, 7).unwrap()
        else {
            panic!("expected scan")
        };
        assert_eq!(actual.data, expected.data, "scan style {style}");
    }
}

#[test]
fn completed_density_rejects_camera_reinterpretation_but_allows_future_procedural_grain() {
    use crate::stage_execution::{StageResult, continue_stage};
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let mut producing = build();
    producing.params.workflow.route = "input > film".into();
    producing.params.film_render.grain.active = false;
    let input = ImageBuf::from_data(4, 3, vec![from_f64(0.2); 36]);
    let film = producing
        .process_stage(input.clone(), &backend, 12)
        .unwrap();
    let mut params = film.header.baseline.clone();
    params.workflow.route = "film > scan".into();
    params.camera.exposure_compensation_ev += 1.0;
    let error = crate::stage_execution::validate_continuation(&film.header, &params).unwrap_err();
    assert!(error.contains("camera.exposure_compensation_ev"));
    params.camera.exposure_compensation_ev = film.header.baseline.camera.exposure_compensation_ev;
    params.film_render.grain.engine = crate::params::GrainEngine::Procedural;
    params.film_render.grain.active = true;
    let mut full = producing.clone();
    full.params = params.clone();
    full.params.workflow.route = "input > film > scan".into();
    full.params.io.scan_film = true;
    full.params.random_seed = 99;
    let expected = full.process(input.clone(), &backend).unwrap();
    full.params.film_render.grain.active = false;
    let without_grain = full.process(input, &backend).unwrap();
    let StageResult::Image(actual) =
        continue_stage(film, params, None, &data_dir(), &backend, 99).unwrap()
    else {
        panic!("expected scan")
    };
    assert_eq!(
        actual.data, expected.data,
        "post-scan Procedural must run exactly once with the new seed"
    );
    assert_ne!(
        actual.data, without_grain.data,
        "enabling Procedural must visibly change the scanned samples"
    );
}

/// Procedural composites in the scanner's encoded output, so the negative
/// route must render it, not merely accept the parameters. Comparing against a
/// grain-free render catches a route that silently skips the stage.
#[test]
fn negative_to_positive_renders_procedural_grain() {
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let input = ImageBuf::from_data(
        48,
        32,
        (0..48 * 32)
            .flat_map(|i| {
                let v = 0.30 + 0.004 * f64::from(i % 48);
                [from_f64(v), from_f64(v * 0.95), from_f64(v * 0.9)]
            })
            .collect(),
    );
    let render = |active: bool| {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
        params.camera.auto_exposure = false;
        params.film_render.halation.active = false;
        params.film_render.dir_couplers.active = false;
        params.film_render.grain.active = active;
        params.film_render.grain.engine = crate::params::GrainEngine::Procedural;
        params.random_seed = 4242;
        Pipeline::new_with_spectral(film.clone(), print.clone(), params, &dir)
            .expect("negative_to_positive must accept Procedural")
            .process(input.clone(), &backend)
            .unwrap()
    };
    let with_grain = render(true);
    let without_grain = render(false);
    assert_ne!(
        with_grain.data, without_grain.data,
        "Procedural must change the rendered samples on the negative route"
    );
}

#[test]
fn continuing_print_reinitializes_replaced_material_and_retains_film_seed_lineage() {
    use crate::stage_execution::{StageResult, continue_stage};
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let input = ImageBuf::from_data(
        5,
        3,
        (0..45)
            .map(|i| from_f64(0.04 + (i % 11) as f64 / 30.0))
            .collect(),
    );
    let mut producing = build();
    producing.params.workflow.route = "input > film".into();
    producing.params.film_render.grain.active = false;
    let film = producing
        .process_stage(input.clone(), &backend, 42)
        .unwrap();
    let replacement = crate::profile::load_profile_by_name(&data_dir(), "kodak_2393").unwrap();
    let mut params = film.header.baseline.clone();
    params.workflow.route = "film > print".into();
    let StageResult::Material(print) = continue_stage(
        film.clone(),
        params,
        Some(replacement.clone()),
        &data_dir(),
        &backend,
        99,
    )
    .unwrap() else {
        panic!("expected print")
    };
    assert_eq!(print.header.seed, 99);
    assert_eq!(print.header.lineage.len(), 1);
    assert_eq!(print.header.lineage[0].seed, 42);
    assert_eq!(print.header.provenance.inherited_operations, ["film"]);
    assert_eq!(
        print
            .header
            .print
            .as_ref()
            .unwrap()
            .profile
            .info
            .stock
            .as_deref(),
        Some("kodak_2393")
    );
    let mut full_params = producing.params.clone();
    full_params.workflow.route = "input > film > print > scan".into();
    full_params.random_seed = 99;
    let full = Pipeline::new_with_spectral(
        (*producing.original_film).clone(),
        replacement,
        full_params,
        &data_dir(),
    )
    .unwrap();
    let expected = full.process(input, &backend).unwrap();
    let mut params = print.header.baseline.clone();
    params.workflow.route = "print > scan".into();
    let StageResult::Image(actual) =
        continue_stage(*print, params, None, &data_dir(), &backend, 99).unwrap()
    else {
        panic!("expected scan")
    };
    assert_eq!(actual.data, expected.data);
    let mut no_source = film.header.baseline.clone();
    no_source.workflow.route = "film > scan".into();
    no_source.output_mix.amount = 0.5;
    assert!(
        crate::stage_execution::validate_continuation(&film.header, &no_source)
            .unwrap_err()
            .contains("prepared-source")
    );
}

#[test]
fn highlight_boost_requires_enabled_halation() {
    let dir = data_dir();
    let film = crate::profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = crate::profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let backend = spektrafilm_backend::cpu_backend::CpuBackend;
    let input = ImageBuf::from_data(
        32,
        24,
        (0..32 * 24)
            .flat_map(|i| {
                let x = i % 32;
                let v = if x >= 24 { 12.0 } else { 0.4 + x as f64 * 0.02 };
                [from_f64(v), from_f64(v * 0.9), from_f64(v * 0.8)]
            })
            .collect(),
    );
    let render = |halation: bool, boost_ev: f32| {
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
        params.camera.auto_exposure = false;
        params.film_render.grain.active = false;
        params.film_render.dir_couplers.active = false;
        params.film_render.halation.active = halation;
        params.film_render.halation.boost_ev = boost_ev;
        params.random_seed = 4242;
        Pipeline::new_with_spectral(film.clone(), print.clone(), params, &dir)
            .unwrap()
            .process(input.clone(), &backend)
            .unwrap()
    };

    let disabled_zero = render(false, 0.0);
    let disabled_boost = render(false, 2.0);
    assert_eq!(
        disabled_zero.data, disabled_boost.data,
        "highlight boost must not run while halation is disabled"
    );

    let enabled_zero = render(true, 0.0);
    let enabled_boost = render(true, 2.0);
    assert_ne!(
        enabled_zero.data, enabled_boost.data,
        "highlight boost must change the render while halation is enabled"
    );
}
