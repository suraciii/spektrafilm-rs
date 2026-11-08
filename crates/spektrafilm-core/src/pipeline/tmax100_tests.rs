use super::*;
use spektrafilm_backend::cpu_backend::CpuBackend;

fn linear(x: f64, grid: &[f64], rows: &[Vec<f64>]) -> f64 {
    let upper = grid.partition_point(|v| *v <= x);
    if upper == 0 {
        return rows[0][0];
    }
    if upper == grid.len() {
        return rows[upper - 1][0];
    }
    let lo = upper - 1;
    let t = (x - grid[lo]) / (grid[upper] - grid[lo]);
    rows[lo][0] * (1. - t) + rows[upper][0] * t
}

fn preset_pipeline(data: &std::path::Path, effects: bool) -> Pipeline {
    preset_pipeline_flags(data, effects, effects)
}

fn preset_pipeline_flags(data: &std::path::Path, grain: bool, halation: bool) -> Pipeline {
    preset_pipeline_flags_print(data, grain, halation, false)
}

fn preset_pipeline_flags_print(
    data: &std::path::Path,
    grain: bool,
    halation: bool,
    manual_print: bool,
) -> Pipeline {
    let preset = crate::presets::load_selector("built-in:Kodak T-MAX 100", data).unwrap();
    let resolved = preset.resolve(data, &RuntimeParams::default()).unwrap();
    let mut params = resolved.params;
    params.camera.auto_exposure = false;
    params.camera.exposure_compensation_ev = 0.;
    params.io.input_color_space = "sRGB".into();
    params.io.input_cctf_decoding = false;
    params.io.output_color_space = "sRGB".into();
    params.io.output_cctf_encoding = false;
    params.io.output_gamut_compress.algorithm = "off".into();
    params.settings.use_scanner_lut = false;
    params.settings.use_enlarger_lut = false;
    params.settings.use_cat16 = false;
    params.film_render.grain.active = grain;
    params.film_render.halation.active = halation;
    if manual_print {
        params.enlarger.print_exposure_mode = crate::params::PrintExposureMode::Manual;
        params.enlarger.print_exposure = 1.0;
    }
    Pipeline::new_with_spectral(resolved.film, resolved.print, params, data).unwrap()
}

#[test]
fn tmax100_loaded_curve_preserves_source_density_and_single_record() {
    let data = crate::test_support::data_dir();
    let source: serde_json::Value = serde_json::from_slice(
        &std::fs::read(data.join("profile_sources/kodak_tmax_100.json")).unwrap(),
    )
    .unwrap();
    let stored = crate::profile::load_profile_by_name(&data, "kodak_tmax_100").unwrap();
    assert!(stored.is_bw() && !stored.is_positive() && stored.is_film());
    assert_eq!(stored.info.usage, "still");
    assert_eq!(stored.data.development_time, [7.5]);
    assert_eq!(
        stored.data.density_curves_layers.len(),
        stored.data.density_curves.len()
    );
    for (i, layer) in stored.data.density_curves_layers.iter().enumerate() {
        assert_eq!(layer.len(), 1);
        assert!((layer[0][0] - stored.data.density_curves[i][0]).abs() < 1e-12);
    }
    let points = source["characteristic"].as_array().unwrap();
    let origin = points[0][0].as_f64().unwrap() - stored.data.log_exposure[0];
    let film = crate::profile::resolve_for_render(stored, None);
    assert!(film.data.channel_density.iter().all(|v| v == &[1., 0., 0.]));
    let mut params = RuntimeParams::default();
    params.film_render.grain.active = false;
    params.film_render.halation.active = false;
    params.film_render.dir_couplers.active = false;
    params.film_render.chemistry.active = false;
    let input = ImageBuf::from_data(
        points.len() as u32,
        1,
        points
            .iter()
            .flat_map(|p| [from_f64(p[0].as_f64().unwrap() - origin); 3])
            .collect(),
    );
    let density =
        crate::stages::filming::develop(&input, &film, &params, &CpuBackend, 10.).unwrap();
    for (pixel, point) in density.data.as_chunks::<3>().0.iter().zip(points) {
        assert_eq!(pixel[0], pixel[1]);
        assert_eq!(pixel[0], pixel[2]);
        assert!(
            (to_f64(pixel[0]) + film.data.base_density[0] - point[1].as_f64().unwrap()).abs()
                < 2e-5,
            "source density mismatch at {:?}",
            point
        );
    }
    crate::stages::grain_dye::resolve(&film, &params).unwrap();
}

fn assert_scalar_print_oracle(data: &std::path::Path, manual_print: bool) {
    let pipeline = preset_pipeline_flags_print(data, false, false, manual_print);
    let exposures: Vec<f64> = (0..33).map(|i| -2. + i as f64 * 0.1).collect();
    let input = ImageBuf::from_data(
        33,
        1,
        exposures.iter().flat_map(|x| [from_f64(*x); 3]).collect(),
    );
    let stage = |tap| {
        pipeline
            .process_with_taps(input.clone(), &CpuBackend, Some(Tap::LogEFilm), Some(tap))
            .unwrap()
    };
    let negative = stage(Tap::CmyFilm);
    let log_print = stage(Tap::LogEPrint);
    let paper = stage(Tap::CmyPrint);
    let output = stage(Tap::RgbOut);
    let lamp_response: f64 = pipeline
        .print_illuminant_slice()
        .iter()
        .zip(&pipeline.print.data.log_sensitivity)
        .map(|(lamp, sensitivity)| lamp * 10f64.powf(sensitivity[0]))
        .sum();
    for (i, &exposure) in exposures.iter().enumerate() {
        let nd = linear(
            exposure,
            &pipeline.film.data.log_exposure,
            &pipeline.film.data.density_curves,
        );
        let total = nd + pipeline.film.data.base_density[0];
        let log_h = (lamp_response * pipeline.print_exposure_factor()).log10() - total;
        let pd = linear(
            log_h,
            &pipeline.print.data.log_exposure,
            &pipeline.print.data.density_curves,
        );
        let relative_reflectance = 10f64.powf(-pd);
        assert!((to_f64(negative.data[i * 3]) - nd).abs() < 2e-5);
        assert!((to_f64(log_print.data[i * 3]) - log_h).abs() < 2e-5);
        assert!((to_f64(paper.data[i * 3]) - pd).abs() < 2e-5);
        for channel in 0..3 {
            assert!((to_f64(output.data[i * 3 + channel]) - relative_reflectance).abs() < 2e-4);
        }
    }
    if !manual_print {
        assert!(
            output
                .data
                .as_chunks::<3>()
                .0
                .windows(2)
                .all(|w| w[1][0] >= w[0][0])
        );
    }
    assert!(output.data[0] < output.data[output.data.len() - 3]);
}

#[test]
fn tmax100_auto_print_exposure_matches_scalar_density_oracle() {
    assert_scalar_print_oracle(&crate::test_support::data_dir(), false);
}

#[test]
fn tmax100_manual_print_exposure_matches_scalar_density_oracle() {
    assert_scalar_print_oracle(&crate::test_support::data_dir(), true);
}

#[test]
fn tmax100_grain_stage_matches_rms8_at_48um() {
    let data = crate::test_support::data_dir();
    let stored = crate::profile::load_profile_by_name(&data, "kodak_tmax_100").unwrap();
    let film = crate::profile::resolve_for_render(stored, None);
    let norm =
        spektrafilm_model::density_curves::normalize_density_curves_f64(&film.density_curves_f64());
    let preset = crate::presets::load_selector("built-in:Kodak T-MAX 100", &data).unwrap();
    let resolved = preset.resolve(&data, &RuntimeParams::default()).unwrap();
    let mut grain = resolved.params.film_render.grain;
    grain.active = true;
    grain.engine = crate::params::GrainEngine::Spektra;
    grain.rms_granularity = [9.; 3];
    grain.density_min = [0.03; 3];
    grain.uniformity = [0.97; 3];
    grain.particle_scale_sublayers = [1.0, 0.4, 0.25];
    grain.blur = 0.0;
    grain.blur_dye_clouds_um = 0.0;
    grain.micro_structure = [0.0; 2];
    grain.mult_usm_amount = 0.0;
    grain.monochrome = true;
    let image = ImageBuf::from_data(256, 256, vec![from_f64(1.0); 256 * 256 * 3]);
    let output = crate::stages::filming::apply_spektra_grain(
        image,
        &film,
        &grain,
        &norm,
        0,
        48.0,
        &CpuBackend,
    )
    .unwrap();
    let values: Vec<f64> = output
        .data
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| to_f64(p[0]))
        .collect();
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let sigma =
        (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt();
    println!("grain stage 48um: mean={mean:.9} sigma={sigma:.9}");
    assert!((mean - 1.0).abs() < 1e-3, "mean {mean}");
    assert!((sigma - 7.98e-3).abs() < 0.10e-3, "sigma {sigma}");
}

#[path = "tmax100_fixture.rs"]
mod spectral_fixture;

#[test]
fn tmax100_runtime_exposure_matches_independent_d55_integrals() {
    let data = crate::test_support::data_dir();
    let rows = spectral_fixture::SPECTRAL_ORACLE;
    for adapted in [false, true] {
        let preset = crate::presets::load_selector("built-in:Kodak T-MAX 100", &data).unwrap();
        let resolved = preset.resolve(&data, &RuntimeParams::default()).unwrap();
        let mut params = resolved.params;
        params.camera.auto_exposure = false;
        params.camera.exposure_compensation_ev = 0.;
        params.io.input_color_space = "sRGB".into();
        params.io.input_cctf_decoding = false;
        params.settings.use_cat16 = false;
        params.settings.apply_hanatos2025_adaptation_surface = adapted;
        params.settings.apply_hanatos2025_adaptation_window = true;
        params.film_render.grain.active = false;
        params.film_render.halation.active = false;
        let pipeline =
            Pipeline::new_with_spectral(resolved.film, resolved.print, params, &data).unwrap();
        let image = ImageBuf::from_data(
            rows.len() as u32,
            1,
            rows.iter()
                .flat_map(|r| r[..3].iter().map(|x| from_f64(*x)))
                .collect(),
        );
        let output = pipeline
            .process_with_taps(image, &CpuBackend, None, Some(Tap::LogEFilm))
            .unwrap();
        let errors: Vec<f64> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let actual = to_f64(output.data[3 * i]);
                for channel in 1..3 {
                    assert!((to_f64(output.data[3 * i + channel]) - actual).abs() < 1e-6);
                }
                (actual - r[3]) / std::f64::consts::LOG10_2
            })
            .collect();
        for (name, values, expected_rmse, expected_max) in [
            (
                "training",
                &errors[..99],
                if adapted { 0.08869662954598162 } else { 0.0 },
                if adapted { 0.2631400984730351 } else { 0.0 },
            ),
            (
                "holdout",
                &errors[99..123],
                if adapted {
                    0.057507501165709615
                } else {
                    0.11040140858600368
                },
                if adapted {
                    0.13977007522250878
                } else {
                    0.32966232857672295
                },
            ),
        ] {
            let rmse = (values.iter().map(|e| e * e).sum::<f64>() / values.len() as f64).sqrt();
            let max = values.iter().map(|e| e.abs()).fold(0., f64::max);
            println!("adapted={adapted} {name}: RMSE={rmse:.9} max={max:.9} stops");
            if adapted || name == "holdout" {
                assert!((rmse - expected_rmse).abs() < 0.0001, "{name}: {rmse}");
                assert!((max - expected_max).abs() < 0.0001, "{name}: {max}");
            }
        }
        println!("adapted={adapted} neutral error={} stops", errors[123]);
        assert!(errors[123].abs() < 0.0001);
    }
}

#[test]
fn tmax100_builtin_look_has_fine_texture_without_large_mean_shift() {
    let data = crate::test_support::data_dir();
    let textured = preset_pipeline_flags(&data, true, true);
    let grain_only = preset_pipeline_flags(&data, true, false);
    let halation_only = preset_pipeline_flags(&data, false, true);
    let clean = preset_pipeline(&data, false);
    let input = ImageBuf::from_data(64, 64, vec![from_f64(0.5); 64 * 64 * 3]);
    let run = |pipeline: &Pipeline| {
        pipeline
            .process_with_taps(input.clone(), &CpuBackend, None, Some(Tap::RgbOut))
            .unwrap()
    };
    let textured = run(&textured);
    let grain_only = run(&grain_only);
    let halation_only = run(&halation_only);
    let clean = run(&clean);
    assert!(textured.data.iter().all(|v| v.is_finite()));
    for (label, variant) in [("grain_only", &grain_only), ("both", &textured)] {
        for channel in 0..3 {
            let residual: Vec<f64> = (0..64 * 64)
                .map(|i| to_f64(variant.data[i * 3 + channel] - clean.data[i * 3 + channel]))
                .collect();
            let mean = residual.iter().sum::<f64>() / residual.len() as f64;
            let variance =
                residual.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / residual.len() as f64;
            assert!(
                mean.abs() < 0.002,
                "{label} channel {channel} mean shift {mean}"
            );
            assert!(
                variance > 1e-6,
                "{label} channel {channel} texture too weak {variance}"
            );
        }
    }
    // A flat field must be a no-op for scatter/halation; otherwise a mean
    // shift is an optics artifact rather than grain.
    let max_diff = halation_only
        .data
        .iter()
        .zip(&clean.data)
        .map(|(a, b)| (to_f64(*a) - to_f64(*b)).abs())
        .fold(0., f64::max);
    // Auto print exposure can move a flat field by a few 1e-6; this is a
    // numerical/optics check, not a densitometric no-op guarantee.
    assert!(
        max_diff < 1e-5,
        "flat-field halation/scatter changed output: {max_diff}"
    );
}
