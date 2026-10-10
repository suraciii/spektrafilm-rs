pub mod converting;
mod debug_compare;
pub mod filming;
pub mod printing;
pub mod scanning;

use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::from_f64;

/// Keep a stage's backend view and observation lifetime together.
pub(crate) struct StageObservation<'a> {
    bound: Option<Box<dyn spektrafilm_gpu::ComputeBackend + 'a>>,
    scope: spektrafilm_gpu::telemetry::ObservationScope,
}

impl<'a> StageObservation<'a> {
    pub(crate) fn new(
        backend: &'a dyn spektrafilm_gpu::ComputeBackend,
        name: &'static str,
    ) -> Self {
        let Some(context) = backend
            .observation_context()
            .filter(|context| context.enabled())
        else {
            return Self {
                bound: None,
                scope: Default::default(),
            };
        };
        let scope = context.scope(
            name,
            spektrafilm_gpu::telemetry::ObservationKind::Stage,
            context.purpose(),
        );
        let bound = Some(spektrafilm_gpu::bind_backend(
            backend,
            scope.context().clone(),
        ));
        Self { bound, scope }
    }

    pub(crate) fn cpu(
        backend: &'a dyn spektrafilm_gpu::ComputeBackend,
        name: &'static str,
        reason: spektrafilm_gpu::telemetry::CpuReason,
    ) -> Self {
        let stage = Self::new(backend, name);
        if stage.scope.context().enabled() {
            stage
                .scope
                .context()
                .record_executor(spektrafilm_gpu::telemetry::Executor::Cpu, Some(reason));
        }
        stage
    }

    pub(crate) fn backend(
        &self,
        original: &'a dyn spektrafilm_gpu::ComputeBackend,
    ) -> &dyn spektrafilm_gpu::ComputeBackend {
        self.bound.as_deref().unwrap_or(original)
    }

    pub(crate) fn set_complete(&mut self, complete: bool) {
        self.scope.set_complete(complete);
    }
}

/// Build a `steps × steps² × 3` ImageBuf holding the LUT-input cmy
/// grid. Layout mirrors Python's `_create_lut_3d`:
///
/// ```text
///   reshape(meshgrid(x_r, x_g, x_b, indexing='ij'), (steps², steps, 3))
/// ```
///
/// → pixel at (col=k, row=i*steps+j) carries cmy = (x_r[i], x_g[j], x_b[k]).
/// Running the spectral function on this 2-D image then reshapes back to
/// a `steps × steps × steps × 3` LUT indexed by `((i, j, k), c)`.
fn build_lut_grid(steps: usize, data_min: [f64; 3], data_max: [f64; 3]) -> ImageBuf {
    let mut grid = ImageBuf::new(steps as u32, (steps * steps) as u32);
    let step_inv = (steps - 1) as f64;
    for i in 0..steps {
        let x_r = data_min[0] + (data_max[0] - data_min[0]) * (i as f64) / step_inv;
        for j in 0..steps {
            let x_g = data_min[1] + (data_max[1] - data_min[1]) * (j as f64) / step_inv;
            for k in 0..steps {
                let x_b = data_min[2] + (data_max[2] - data_min[2]) * (k as f64) / step_inv;
                let row = i * steps + j;
                let base = (row * steps + k) * 3;
                grid.data[base] = from_f64(x_r);
                grid.data[base + 1] = from_f64(x_g);
                grid.data[base + 2] = from_f64(x_b);
            }
        }
    }
    grid
}

#[cfg(test)]
mod lut_grid_tests {
    use super::build_lut_grid;
    use spektrafilm_math::precision::from_f64;

    #[test]
    fn grid_preserves_layout_and_negative_bounds_endpoints() {
        let steps = 3;
        let grid = build_lut_grid(steps, [-2.0, -4.0, -8.0], [2.0, 2.0, 0.0]);
        assert_eq!((grid.width, grid.height), (3, 9));
        assert_eq!(grid.data.len(), steps * steps * steps * 3);

        for (i, r) in [-2.0, 0.0, 2.0].into_iter().enumerate() {
            for (j, g) in [-4.0, -1.0, 2.0].into_iter().enumerate() {
                for (k, b) in [-8.0, -4.0, 0.0].into_iter().enumerate() {
                    let base = ((i * steps + j) * steps + k) * 3;
                    for (channel, expected) in [r, g, b].into_iter().enumerate() {
                        assert_eq!(
                            grid.data[base + channel].to_bits(),
                            from_f64(expected).to_bits(),
                            "grid coordinate ({i}, {j}, {k}), channel {channel}"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use crate::params::RuntimeParams;
    use crate::pipeline::Pipeline;
    use crate::profile;
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::{Scalar, from_f64};
    use std::path::Path;

    fn data_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data")
    }

    #[test]
    fn grain_v2_uses_scan_encoding_and_preserves_linear_export() {
        use crate::params::grain::{GrainEngine, GrainV2Mode};
        use spektrafilm_math::colorspace;
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let paper = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let image = ImageBuf::from_data(
            8,
            6,
            (0..8 * 6 * 3)
                .map(|i| from_f64(0.12 + (i % 29) as f64 / 40.0))
                .collect(),
        );
        for space_name in ["sRGB", "Display P3", "ProPhoto RGB", "ITU-R BT.2020"] {
            let space = colorspace::resolve(space_name).unwrap();
            for scan_film in [false, true] {
                let mut params = RuntimeParams::default();
                params.camera.auto_exposure = false;
                params.film_render.halation.active = false;
                params.film_render.dir_couplers.active = false;
                params.print_render.glare.active = false;
                params.film_render.grain.active = false;
                params.io.scan_film = scan_film;
                params.io.output_color_space = space_name.into();
                params.random_seed = 5489;
                let render = |p: RuntimeParams| {
                    Pipeline::new(film.clone(), paper.clone(), p)
                        .process(image.clone(), &backend)
                        .unwrap()
                };
                let encoded_base = render(params.clone());
                params.film_render.grain.active = true;
                params.film_render.grain.engine = GrainEngine::V2;
                params.film_render.grain.select_custom_grain_v2();
                for mode in [GrainV2Mode::Analogue, GrainV2Mode::Noise] {
                    params.film_render.grain.v2_mode = mode;
                    let mut grain = params.film_render.grain.resolved_grain_v2();
                    grain.seed = 5489;
                    let expected = spektrafilm_model::grain::v2::apply_cpu(&encoded_base, grain);
                    params.io.output_cctf_encoding = true;
                    let encoded = render(params.clone());
                    params.io.output_cctf_encoding = false;
                    let linear = render(params.clone());
                    for ((&actual, &reference), &linear_value) in
                        encoded.data.iter().zip(&expected.data).zip(&linear.data)
                    {
                        assert!(
                            (actual as f64 - reference as f64).abs() < 1e-6,
                            "{space_name} {scan_film} {mode:?}: grain must consume native encoded scan RGB"
                        );
                        let decoded = colorspace::cctf_decode(actual as f64, space.cctf);
                        assert!(
                            (linear_value as f64 - decoded).abs() < 1e-6,
                            "{space_name} {scan_film} {mode:?}: linear export changes grain realization"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_full_pipeline_portra_400_to_endura() {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();

        let mut params = RuntimeParams::default();
        params.film_render.grain.active = false;
        params.film_render.halation.active = false;
        params.film_render.dir_couplers.active = false;
        params.camera.auto_exposure = false;

        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let img = ImageBuf::from_data(8, 8, vec![from_f64(0.184); 8 * 8 * 3]);

        let pipeline = Pipeline::new(film, print, params);
        let result = pipeline.process(img, &backend).unwrap();

        assert_eq!(result.width, 8);
        assert_eq!(result.height, 8);
        let px = result.get(4, 4);
        for c in 0..3 {
            assert!(
                px[c] >= from_f64(0.0) && px[c] <= from_f64(1.0),
                "channel {c} out of range: {}",
                px[c]
            );
        }
        let mean: Scalar =
            result.data.iter().copied().sum::<Scalar>() / result.data.len() as Scalar;
        assert!(mean > from_f64(0.01), "output near-black: mean={mean}");
        assert!(mean < from_f64(0.99), "output near-white: mean={mean}");
    }

    #[test]
    fn convert_film_routes_produce_scan_output() {
        let dir = data_dir();
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        for route in [
            "input > convert-film > scan",
            "input > convert-film > scan-minus-base",
            "input > convert-film > print > scan",
        ] {
            let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
            let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
            let mut params = RuntimeParams::default();
            params.workflow.route = route.into();
            params.camera.auto_exposure = false;
            params.film_render.grain.active = false;
            params.film_render.halation.active = false;
            params.film_render.dir_couplers.active = false;
            params.scanner.unsharp_mask = [0.0, 0.0];
            params.settings.use_scanner_lut = false;
            params.io.input_color_space = "sRGB".into();
            params.io.output_color_space = "sRGB".into();
            let image =
                ImageBuf::from_data(1, 1, vec![from_f64(0.2), from_f64(0.3), from_f64(0.4)]);
            let result = Pipeline::new(film, print, params)
                .process(image, &backend)
                .unwrap();
            assert_eq!((result.width, result.height), (1, 1));
            assert!(result.data.iter().all(|value| value.is_finite()), "{route}");
        }
    }

    #[test]
    fn positive_scan_inverts_negative_capture_on_cpu_path() {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params.io.input_color_space = "sRGB".into();
        params.io.output_color_space = "sRGB".into();
        params.io.output_cctf_encoding = false;
        params.io.output_gamut_compress.algorithm = "off".into();
        params.scanner.scan_output = "positive_scan".into();
        params.scanner.unsharp_mask = [0.0, 0.0];
        params.camera.auto_exposure = false;
        params.film_render.grain.active = false;
        params.film_render.halation.active = false;
        params.film_render.dir_couplers.active = false;
        params.settings.use_scanner_lut = false;
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir).unwrap();
        let mut updated_params = pipeline.params.clone();
        updated_params.io.scan_film = false;
        let pipeline = pipeline.with_params(updated_params).unwrap();
        assert!(pipeline.params.io.scan_film);
        let render = |value| {
            pipeline
                .process(
                    ImageBuf::from_data(1, 1, vec![from_f64(value); 3]),
                    &backend,
                )
                .unwrap()
        };
        let shadow = render(0.1);
        let highlight = render(0.8);
        let shadow_mean = shadow.data.iter().copied().sum::<Scalar>() / from_f64(3.0);
        let highlight_mean = highlight.data.iter().copied().sum::<Scalar>() / from_f64(3.0);
        assert!(shadow.data.iter().all(|value| value.is_finite()));
        assert!(highlight.data.iter().all(|value| value.is_finite()));
        assert!(
            shadow_mean < highlight_mean,
            "positive interpretation must reverse negative polarity: shadow={shadow_mean}, highlight={highlight_mean}"
        );
    }

    #[test]
    fn passthrough_route_skips_simulation() {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.workflow.route = "input".into();
        params.io.input_color_space = "sRGB".into();
        params.io.output_color_space = "sRGB".into();
        params.io.input_cctf_decoding = false;
        params.io.output_cctf_encoding = true;
        let image = ImageBuf::from_data(1, 1, vec![from_f64(0.2), from_f64(0.3), from_f64(0.4)]);
        let output = Pipeline::new(film, print, params)
            .process(image.clone(), &spektrafilm_gpu::cpu_backend::CpuBackend)
            .unwrap();
        assert_eq!(output.width, image.width);
        assert!(output.data.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn test_full_pipeline_with_spectral_lut() {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();

        let mut params = RuntimeParams::default();
        params.film_render.grain.active = false;
        params.film_render.halation.active = false;
        params.film_render.dir_couplers.active = false;
        params.camera.auto_exposure = false;
        params.io.input_color_space = "sRGB".to_string();

        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let img = ImageBuf::from_data(8, 8, vec![from_f64(0.184); 8 * 8 * 3]);

        let pipeline = Pipeline::new_with_spectral(film, print, params, &dir);
        match pipeline {
            Ok(p) => {
                let result = p.process(img, &backend).unwrap();
                let mean: Scalar =
                    result.data.iter().copied().sum::<Scalar>() / result.data.len() as Scalar;
                eprintln!("Spectral pipeline output mean: {mean}");
                let px = result.get(4, 4);
                eprintln!("Spectral pipeline pixel(4,4): {:?}", px);
                assert!(
                    mean > from_f64(0.01),
                    "spectral output near-black: mean={mean}"
                );
                assert!(
                    mean < from_f64(0.99),
                    "spectral output near-white: mean={mean}"
                );
            }
            Err(e) => {
                eprintln!("Spectral LUT not available: {e} — skipping test");
            }
        }
    }

    #[test]
    fn test_film_scan_pipeline() {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = film.clone();

        let mut params = RuntimeParams::default();
        params.io.scan_film = true;
        params.film_render.grain.active = false;
        params.film_render.halation.active = false;
        params.film_render.dir_couplers.active = false;
        params.camera.auto_exposure = false;

        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let img = ImageBuf::from_data(4, 4, vec![from_f64(0.184); 4 * 4 * 3]);

        let pipeline = Pipeline::new(film, print, params);
        let result = pipeline.process(img, &backend).unwrap();
        let mean: Scalar =
            result.data.iter().copied().sum::<Scalar>() / result.data.len() as Scalar;
        assert!(mean > from_f64(0.01), "film scan near-black: mean={mean}");
        assert!(mean < from_f64(0.99), "film scan near-white: mean={mean}");
    }

    #[test]
    fn positive_slide_film_scan_stays_photographic() {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_ektachrome_100").unwrap();
        let print = film.clone();

        let mut params = RuntimeParams::default();
        params.io.scan_film = true;
        params.camera.auto_exposure = false;
        let params = crate::params_builder::digest_params(params, &film, &print, None, true);

        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let mut data = Vec::with_capacity(64 * 64 * 3);
        for y in 0..64 {
            for x in 0..64 {
                data.push(from_f64(x as f64 / 63.0));
                data.push(from_f64(y as f64 / 63.0));
                data.push(from_f64(((x + y) as f64 / 126.0).powf(1.2)));
            }
        }
        let img = ImageBuf::from_data(64, 64, data);

        let pipeline = Pipeline::new_with_spectral(film, print, params.clone(), &dir)
            .unwrap()
            .with_params(params)
            .unwrap();
        let result = pipeline.process(img, &backend).unwrap();
        let mut clipped_low = 0usize;
        let mut clipped_high = 0usize;
        for c in 0..3 {
            let values = result.data.iter().skip(c).step_by(3);
            let (mut min, mut max, mut sum) = (Scalar::INFINITY, Scalar::NEG_INFINITY, 0.0);
            let mut n = 0usize;
            for &v in values {
                min = min.min(v);
                max = max.max(v);
                sum += v;
                if v <= from_f64(0.0) {
                    clipped_low += 1;
                }
                if v >= from_f64(1.0) {
                    clipped_high += 1;
                }
                n += 1;
            }
            let mean = sum / n as Scalar;
            assert!(
                mean > from_f64(0.2) && mean < from_f64(0.8),
                "slide scan channel {c} mean is implausible: {mean} (min={min}, max={max})"
            );
        }
        let total = result.data.len();
        assert!(
            clipped_low < total / 20,
            "slide scan has too many black-clipped samples: {clipped_low}/{total}"
        );
        assert!(
            clipped_high < total / 20,
            "slide scan has too many white-clipped samples: {clipped_high}/{total}"
        );
    }
}

#[cfg(test)]
mod debug_tests {
    use crate::params::RuntimeParams;
    use crate::profile;
    use crate::stages;
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::from_f64;
    use std::path::Path;

    #[test]
    fn debug_pipeline_values() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data");
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();

        let mut params = RuntimeParams::default();
        params.film_render.grain.active = false;
        params.film_render.halation.active = false;
        params.film_render.dir_couplers.active = false;
        params.camera.auto_exposure = false;
        params.io.input_color_space = "sRGB".to_string();

        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let gray = from_f64(0.184);
        let img = ImageBuf::from_data(1, 1, vec![gray, gray, gray]);
        eprintln!("Input: {:?}", img.get(0, 0));

        let ref_illuminant =
            crate::spectral_service::select_illuminant(&film.info.reference_illuminant);
        let log_raw = stages::filming::expose(
            &img,
            &film,
            &params,
            &backend,
            None,
            None,
            &ref_illuminant,
            1.0,
            crate::resizing::pixel_size_um(params.camera.film_format_mm, 1, 1),
            0.0,
        );
        eprintln!("log_raw: {:?}", log_raw.get(0, 0));

        let density_cmy = stages::filming::develop(
            &log_raw,
            &film,
            &params,
            &backend,
            crate::resizing::pixel_size_um(params.camera.film_format_mm, 1, 1),
        );
        eprintln!("density_cmy: {:?}", density_cmy.get(0, 0));

        // Use simplified printing path for debug trace
        let printed =
            stages::printing::process(&density_cmy, &film, &print, &params, &backend).unwrap();
        eprintln!("density_print: {:?}", printed.get(0, 0));
        let density_print = printed;
        let rgb_out = stages::scanning::scan(
            &density_print,
            &print,
            &params,
            &backend,
            &crate::color_reference::ColorReference::identity(),
            &crate::gamut_compression::OutputGamutCompress::identity(),
        );
        eprintln!("rgb_out: {:?}", rgb_out.get(0, 0));
    }
}

/// Focused coverage for issue #8 (0.3.4 scan/glare/effect semantics):
/// direct-film glare disable, print glare effect, halation/DIR presets,
/// preflash in the print B/W references, morph defaults and params
/// validation.
#[cfg(test)]
mod scan_semantics_tests {
    use crate::color_reference::ColorReference;
    use crate::params::RuntimeParams;
    use crate::pipeline::Pipeline;
    use crate::profile;
    use spektrafilm_gpu::cpu_backend::CpuBackend;
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::{from_f64, to_f64};
    use std::path::Path;

    fn data_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data")
    }

    fn quiet_params() -> RuntimeParams {
        // Stochastic/spatial film effects off so the only moving part in
        // each comparison is the control under test.
        let mut params = RuntimeParams::default();
        params.film_render.grain.active = false;
        params.film_render.halation.active = false;
        params.film_render.dir_couplers.active = false;
        params.camera.auto_exposure = false;
        params
    }

    fn flat_image(n: u32) -> ImageBuf {
        ImageBuf::from_data(n, n, vec![from_f64(0.3); (n * n * 3) as usize])
    }

    fn max_diff(a: &ImageBuf, b: &ImageBuf) -> f64 {
        a.data
            .iter()
            .zip(b.data.iter())
            .map(|(x, y)| (to_f64(*x) - to_f64(*y)).abs())
            .fold(0.0, f64::max)
    }

    #[test]
    fn film_render_glare_cannot_change_direct_film_scan() {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let backend = CpuBackend;
        let img = flat_image(8);

        let mut quiet = quiet_params();
        quiet.io.scan_film = true;
        let base = Pipeline::new(film.clone(), film.clone(), quiet.clone())
            .process(img.clone(), &backend)
            .unwrap();

        // Crank film_render.glare far past its normal range — upstream 0.3.4
        // sets `glare = None` on the scan_film path, so the output must be
        // bit-identical.
        quiet.film_render.glare.active = true;
        quiet.film_render.glare.percent = 0.9;
        quiet.film_render.glare.roughness = 1.5;
        quiet.film_render.glare.blur = 2.0;
        let loud = Pipeline::new(film.clone(), film, quiet)
            .process(img, &backend)
            .unwrap();

        assert_eq!(
            max_diff(&base, &loud),
            0.0,
            "film_render.glare leaked into a direct-film scan"
        );
    }

    #[test]
    fn print_glare_changes_print_scan() {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let backend = CpuBackend;
        let img = flat_image(16);

        let mut low = quiet_params();
        low.print_render.glare.active = true;
        low.print_render.glare.percent = 0.01;
        low.print_render.glare.roughness = 0.0;
        low.print_render.glare.blur = 0.0;
        let out_low = Pipeline::new(film.clone(), print.clone(), low.clone())
            .process(img.clone(), &backend)
            .unwrap();

        let mut high = low;
        high.print_render.glare.percent = 0.15;
        let out_high = Pipeline::new(film, print, high)
            .process(img, &backend)
            .unwrap();

        let diff = max_diff(&out_low, &out_high);
        assert!(
            diff > 1e-4,
            "print_render.glare had no effect on the print scan (max diff {diff})"
        );
        // Glare adds illuminant light: the bright-glare render must not be
        // darker anywhere on this flat field.
        let any_darker = out_high
            .data
            .iter()
            .zip(out_low.data.iter())
            .any(|(h, l)| to_f64(*h) < to_f64(*l) - 1e-6);
        assert!(!any_darker, "glare darkened pixels — wrong sign");
    }
    #[test]
    fn magazine_appearance_runs_after_v2_grain_and_preserves_transfer_state() {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let backend = CpuBackend;
        let mut disabled = quiet_params();
        disabled.workflow.route = "input > film > scan".into();
        disabled.io.scan_film = true;
        disabled.film_render.grain.active = true;
        disabled.film_render.grain.engine = crate::params::grain::GrainEngine::V2;
        disabled.io.output_cctf_encoding = true;

        let base = Pipeline::new(film.clone(), film.clone(), disabled.clone())
            .process(flat_image(8), &backend)
            .unwrap();
        let mut active = disabled.clone();
        active.magazine_print_color.active = true;
        active.magazine_print_color.strength = 0.75;
        let output = Pipeline::new(film.clone(), film.clone(), active.clone())
            .process(flat_image(8), &backend)
            .unwrap();
        let mut expected = base.clone();
        let output_space =
            spektrafilm_math::colorspace::resolve(&active.io.output_color_space).unwrap();
        crate::magazine_print_color::apply(
            &mut expected,
            &active.magazine_print_color,
            output_space,
            true,
        );
        assert!(
            max_diff(&output, &expected) < 1e-6,
            "magazine color must run after V2 grain"
        );

        disabled.io.output_cctf_encoding = false;
        active.io.output_cctf_encoding = false;
        let linear_base = Pipeline::new(film.clone(), film.clone(), disabled)
            .process(flat_image(8), &backend)
            .unwrap();
        let linear_output = Pipeline::new(film.clone(), film, active)
            .process(flat_image(8), &backend)
            .unwrap();
        assert!(
            linear_output
                .data
                .iter()
                .all(|value| to_f64(*value).is_finite()),
            "linear magazine output must preserve finite headroom"
        );
        assert!(max_diff(&linear_output, &linear_base) > 1e-5);
    }

    #[test]
    fn finished_rgb_route_applies_magazine_appearance() {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let backend = CpuBackend;
        let mut disabled = quiet_params();
        disabled.workflow.route = "input".into();
        disabled.io.output_cctf_encoding = true;

        let base = Pipeline::new(film.clone(), film.clone(), disabled.clone())
            .process(flat_image(8), &backend)
            .unwrap();
        let mut active = disabled.clone();
        active.magazine_print_color.active = true;
        active.magazine_print_color.strength = 0.75;
        let output = Pipeline::new(film.clone(), film, active.clone())
            .process(flat_image(8), &backend)
            .unwrap();
        let mut expected = base.clone();
        let output_space =
            spektrafilm_math::colorspace::resolve(&active.io.output_color_space).unwrap();
        crate::magazine_print_color::apply(
            &mut expected,
            &active.magazine_print_color,
            output_space,
            true,
        );
        assert!(
            max_diff(&output, &expected) < 1e-6,
            "Finished RGB must run the shared magazine finishing"
        );
        assert!(max_diff(&output, &base) > 1e-5);
    }

    #[test]
    fn finished_rgb_route_ignores_resolved_profiles() {
        let dir = data_dir();
        let portra = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let gold = profile::load_profile_by_name(&dir, "kodak_gold_200").unwrap();
        let endura = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let supra = profile::load_profile_by_name(&dir, "kodak_supra_endura").unwrap();
        let backend = CpuBackend;
        let mut params = quiet_params();
        params.workflow.route = "input".into();
        params.io.output_cctf_encoding = true;
        params.magazine_print_color.active = true;
        params.magazine_print_color.strength = 1.0;

        let portra_output = Pipeline::new(portra, endura, params.clone())
            .process(flat_image(8), &backend)
            .unwrap();
        let gold_output = Pipeline::new(gold, supra, params)
            .process(flat_image(8), &backend)
            .unwrap();
        assert_eq!(
            max_diff(&portra_output, &gold_output),
            0.0,
            "Finished RGB output must not depend on the resolved film or print profile"
        );
    }

    #[test]
    fn preflash_shifts_print_black_white_references() {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = quiet_params();
        params.scanner.white_correction = true;
        params.scanner.black_correction = true;
        let pipeline =
            Pipeline::new_with_spectral(film, print, params, &dir).expect("spectral pipeline");
        let zero = ColorReference::compute(
            &pipeline.film,
            &pipeline.print,
            &pipeline.params,
            pipeline.print_illuminant_slice(),
            pipeline.print_exposure_factor(),
            [0.0; 3],
        );
        let flashed = ColorReference::compute(
            &pipeline.film,
            &pipeline.print,
            &pipeline.params,
            pipeline.print_illuminant_slice(),
            pipeline.print_exposure_factor(),
            [0.05; 3],
        );
        let (m0, q0) = zero.xyz_remap().expect("print path builds a remap");
        let (m1, q1) = flashed.xyz_remap().expect("print path builds a remap");
        assert!(
            (m0 - m1).abs() > 1e-9 || (q0 - q1).abs() > 1e-9,
            "preflash must flow into the B/W reference log-raws (m {m0} vs {m1})"
        );
    }

    #[test]
    fn params_validate_rejects_only_effective_bad_families() {
        assert!(RuntimeParams::default().validate().is_ok());

        let mut params = RuntimeParams::default();
        params.camera.diffusion_filter.active = true;
        params.camera.diffusion_filter.strength = 0.5;
        params.camera.diffusion_filter.filter_family = "black_promist".into();
        assert!(params.validate().is_err());

        // Ineffective filters never reach the family check (Python's
        // early return in apply_diffusion_filter_um).
        params.camera.diffusion_filter.strength = 0.0;
        assert!(params.validate().is_ok());
        params.camera.diffusion_filter.strength = 0.5;
        params.camera.diffusion_filter.spatial_scale = 0.0;
        assert!(params.validate().is_ok());

        let mut params = RuntimeParams::default();
        params.enlarger.diffusion_filter.active = true;
        params.enlarger.diffusion_filter.filter_family = "golden_glow".into();
        assert!(params.validate().is_err());
    }

    #[test]
    fn new_with_spectral_rejects_bad_family_before_building() {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.camera.diffusion_filter.active = true;
        params.camera.diffusion_filter.filter_family = "nope".into();
        let err = Pipeline::new_with_spectral(film, print, params, &dir)
            .err()
            .expect("effective unknown filter must fail");
        assert!(err.contains("unknown diffusion filter family"));
    }
}
