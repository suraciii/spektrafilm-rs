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
    let preset = crate::presets::load_selector("built-in:Fujifilm Acros 100 II", data).unwrap();
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
    if !effects {
        params.film_render.grain.active = false;
        params.film_render.halation.active = false;
    }
    Pipeline::new_with_spectral(resolved.film, resolved.print, params, data).unwrap()
}

#[test]
fn acros100ii_loaded_curve_preserves_source_density_and_single_record() {
    let data = crate::test_support::data_dir();
    let source: serde_json::Value = serde_json::from_slice(
        &std::fs::read(data.join("profile_sources/fujifilm_acros_100_ii.json")).unwrap(),
    )
    .unwrap();
    let stored = crate::profile::load_profile_by_name(&data, "fujifilm_acros_100_ii").unwrap();
    assert!(stored.is_bw() && !stored.is_positive() && stored.is_film());
    assert_eq!(stored.info.usage, "still");
    assert_eq!(stored.data.development_time, [10.]);
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

#[test]
fn acros100ii_optical_print_matches_scalar_density_oracle() {
    let data = crate::test_support::data_dir();
    let pipeline = preset_pipeline(&data, false);
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
    assert!(
        output
            .data
            .as_chunks::<3>()
            .0
            .windows(2)
            .all(|w| w[1][0] >= w[0][0])
    );
    assert!(output.data[0] < output.data[output.data.len() - 3]);
}

#[path = "acros100ii_fixture.rs"]
mod spectral_fixture;

#[test]
fn acros100ii_runtime_exposure_matches_independent_d55_integrals() {
    let data = crate::test_support::data_dir();
    let rows = spectral_fixture::SPECTRAL_ORACLE;
    for adapted in [false, true] {
        let preset =
            crate::presets::load_selector("built-in:Fujifilm Acros 100 II", &data).unwrap();
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
                if adapted { 0.0300393783 } else { 0.0 },
                if adapted { 0.0947731099 } else { 0.0 },
            ),
            (
                "holdout",
                &errors[99..123],
                if adapted { 0.0279687054 } else { 0.0372342850 },
                if adapted { 0.0634756293 } else { 0.1159512675 },
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
fn acros100ii_builtin_look_has_fine_texture_without_large_mean_shift() {
    let data = crate::test_support::data_dir();
    let textured = preset_pipeline(&data, true);
    let clean = preset_pipeline(&data, false);
    let input = ImageBuf::from_data(64, 64, vec![from_f64(0.184); 64 * 64 * 3]);
    let run = |pipeline: &Pipeline| {
        pipeline
            .process_with_taps(input.clone(), &CpuBackend, None, Some(Tap::RgbOut))
            .unwrap()
    };
    let textured = run(&textured);
    let clean = run(&clean);
    assert!(textured.data.iter().all(|v| v.is_finite()));
    for channel in 0..3 {
        let residual: Vec<f64> = (0..64 * 64)
            .map(|i| to_f64(textured.data[i * 3 + channel] - clean.data[i * 3 + channel]))
            .collect();
        let mean = residual.iter().sum::<f64>() / residual.len() as f64;
        let variance =
            residual.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / residual.len() as f64;
        assert!(mean.abs() < 0.01, "channel {channel} mean shift {mean}");
        assert!(variance > 1e-12, "channel {channel} texture missing");
    }
}
