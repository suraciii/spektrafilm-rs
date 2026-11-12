use super::*;
use spektrafilm_backend::cpu_backend::CpuBackend;

#[path = "astia100f_fixture.rs"]
mod fixture;

fn candidate() -> Pipeline {
    let data = crate::test_support::data_dir();
    let preset = crate::presets::load_selector("built-in:Fujifilm Astia 100F", &data).unwrap();
    let resolved = preset.resolve(&data, &RuntimeParams::default()).unwrap();
    let mut params = resolved.params;
    // Material oracles isolate density and spectral readout from spatial look effects.
    params.film_render.grain.active = false;
    params.film_render.halation.active = false;
    params.camera.auto_exposure = false;
    params.camera.exposure_compensation_ev = 0.;
    params.io.input_color_space = "sRGB".into();
    params.io.output_color_space = "sRGB".into();
    params.io.input_cctf_decoding = false;
    params.io.output_cctf_encoding = false;
    params.io.output_gamut_compress.algorithm = "off".into();
    params.io.input_gamut_compress.active = false;
    params.settings.use_scanner_lut = false;
    Pipeline::new_with_spectral(resolved.film, resolved.print, params, &data).unwrap()
}

#[test]
fn astia100f_loaded_density_reconstructs_published_status_a() {
    let pipeline = candidate();
    let data = crate::test_support::data_dir();
    let source: serde_json::Value = serde_json::from_slice(
        &std::fs::read(data.join("profile_sources/fujifilm_astia_100f.json")).unwrap(),
    )
    .unwrap();
    assert!(pipeline.film.is_positive());
    assert!(pipeline.film.data.density_curves_model.is_none());
    let curves = source["coordinates"]["characteristic"].as_array().unwrap();
    let origin = curves[0][0][0].as_f64().unwrap() - pipeline.film.data.log_exposure[0];
    for (channel, curve) in curves.iter().enumerate() {
        let points = curve.as_array().unwrap();
        let image = ImageBuf::from_data(
            points.len() as u32,
            1,
            points
                .iter()
                .flat_map(|p| [from_f64(p[0].as_f64().unwrap() - origin); 3])
                .collect(),
        );
        let developed = pipeline
            .process_with_taps(image, &CpuBackend, Some(Tap::LogEFilm), Some(Tap::CmyFilm))
            .unwrap();
        let mut maximum = 0.0f64;
        for (i, point) in points.iter().enumerate() {
            let a = &developed.data[i * 3..i * 3 + 3];
            let transmission: f64 = fixture::STATUS_A_WEIGHTS
                .iter()
                .enumerate()
                .map(|(j, w)| {
                    let density = pipeline.film.data.base_density[j]
                        + (0..3)
                            .map(|c| to_f64(a[c]) * pipeline.film.data.channel_density[j][c])
                            .sum::<f64>();
                    10f64.powf(-density) * w[channel]
                })
                .sum();
            maximum = maximum.max((-transmission.log10() - point[1].as_f64().unwrap()).abs());
        }
        println!("ASTIA Status A channel {channel}: max density error {maximum:.9}");
        assert!(maximum < 0.005);
    }
    crate::stages::grain_dye::resolve(&pipeline.film, &pipeline.params).unwrap();
}

#[test]
fn astia100f_runtime_exposure_matches_independent_spectral_integrals() {
    let rows = fixture::SPECTRAL_ORACLE;
    for adapted in [false, true] {
        let mut pipeline = candidate();
        pipeline
            .params
            .settings
            .apply_hanatos2025_adaptation_surface = adapted;
        // Rebuild: the adapter is prepared from the surface switch.
        let data = crate::test_support::data_dir();
        let pipeline = Pipeline::new_with_spectral(
            (*pipeline.film).clone(),
            (*pipeline.print).clone(),
            pipeline.params,
            &data,
        )
        .unwrap();
        let image = ImageBuf::from_data(
            rows.len() as u32,
            1,
            rows.iter()
                .flat_map(|r| r[..3].iter().map(|v| from_f64(*v)))
                .collect(),
        );
        let output = pipeline
            .process_with_taps(image, &CpuBackend, None, Some(Tap::LogEFilm))
            .unwrap();
        for c in 0..3 {
            for (label, start, end) in [("training", 0, 99), ("holdout", 99, 123)] {
                let errors: Vec<f64> = (start..end)
                    .map(|i| {
                        (to_f64(output.data[i * 3 + c]) - rows[i][c + 3])
                            / std::f64::consts::LOG10_2
                    })
                    .collect();
                let rmse = (errors.iter().map(|v| v * v).sum::<f64>() / errors.len() as f64).sqrt();
                let maximum = errors.iter().map(|v| v.abs()).fold(0., f64::max);
                println!(
                    "ASTIA adapted={adapted} {label} channel={c}: RMSE={rmse:.9} max={maximum:.9} stops"
                );
                // Numerical agreement with the Python adapter, not physical accuracy.
                if label == "holdout" {
                    let expected_rmse = if adapted {
                        [0.153172381, 0.051918646, 0.017811951]
                    } else {
                        [0.183348058, 0.072863641, 0.025671162]
                    };
                    let expected_max = if adapted {
                        [0.537527007, 0.117151475, 0.038056322]
                    } else {
                        [0.390907302, 0.192581582, 0.060899687]
                    };
                    assert!((rmse - expected_rmse[c]).abs() < 0.0002);
                    assert!((maximum - expected_max[c]).abs() < 0.0002);
                }
            }
            assert!(to_f64(output.data[123 * 3 + c]).abs() < 0.0001);
        }
    }
}

#[test]
fn astia100f_positive_readout_ignores_paper_and_rejects_negative_inversion() {
    let pipeline = candidate();
    let exposure: Vec<f64> = (0..33).map(|i| -2.5 + i as f64 * 0.15).collect();
    let input = ImageBuf::from_data(
        33,
        1,
        exposure.iter().flat_map(|x| [from_f64(*x); 3]).collect(),
    );
    let run = |p: &Pipeline| {
        p.process_with_taps(
            input.clone(),
            &CpuBackend,
            Some(Tap::LogEFilm),
            Some(Tap::RgbOut),
        )
        .unwrap()
    };
    let image = run(&pipeline);
    assert!(image.data.iter().all(|v| v.is_finite()));
    // RGB matrix coefficients can be negative: individual output channels
    // need not be monotone even when every spectral transmittance increases.
    let luminance: Vec<f64> = image
        .data
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| 0.2126 * to_f64(p[0]) + 0.7152 * to_f64(p[1]) + 0.0722 * to_f64(p[2]))
        .collect();
    assert!(luminance.windows(2).all(|p| p[1] >= p[0] - 1e-7));
    for c in 0..3 {
        assert!((to_f64(image.data[96 + c]) - 1.).abs() < 0.005);
        assert!(image.data[c] < from_f64(0.01));
    }
    let data = crate::test_support::data_dir();
    let paper = crate::profile::load_profile_by_name(&data, "kodak_polymax_fine_art").unwrap();
    let other = Pipeline::new_with_spectral(
        (*pipeline.film).clone(),
        paper,
        pipeline.params.clone(),
        &data,
    )
    .unwrap();
    assert_eq!(image.data, run(&other).data);
    let mut params = pipeline.params.clone();
    params.scanner.scan_output = "negative_to_positive".into();
    assert!(
        Pipeline::new_with_spectral(
            (*pipeline.film).clone(),
            (*pipeline.print).clone(),
            params,
            &data
        )
        .is_err()
    );
}

#[test]
fn astia100f_direct_readout_matches_independent_d50_spectral_oracle() {
    let pipeline = candidate();
    let rows = fixture::SCAN_ORACLE;
    let input = ImageBuf::from_data(
        rows.len() as u32,
        1,
        rows.iter()
            .flat_map(|row| row[..3].iter().map(|v| from_f64(*v)))
            .collect(),
    );
    let output = pipeline
        .process_with_taps(input, &CpuBackend, Some(Tap::LogEFilm), Some(Tap::RgbOut))
        .unwrap();
    for (i, row) in rows.iter().enumerate() {
        for c in 0..3 {
            let error = (to_f64(output.data[i * 3 + c]) - row[c + 3]).abs();
            assert!(error < 2e-5, "sample {i} channel {c}: {error}");
        }
    }
}

#[test]
fn astia100f_builtin_scan_has_fine_texture_without_large_mean_shift() {
    let data = crate::test_support::data_dir();
    let preset = crate::presets::load_selector("built-in:Fujifilm Astia 100F", &data).unwrap();
    let resolved = preset.resolve(&data, &RuntimeParams::default()).unwrap();
    let mut params = resolved.params;
    params.camera.auto_exposure = false;
    params.io.input_color_space = "sRGB".into();
    params.io.input_cctf_decoding = false;
    params.io.output_color_space = "sRGB".into();
    params.io.output_cctf_encoding = false;
    params.settings.use_scanner_lut = false;
    assert!(params.film_render.grain.active && params.film_render.halation.active);
    let look = Pipeline::new_with_spectral(
        resolved.film.clone(),
        resolved.print.clone(),
        params.clone(),
        &data,
    )
    .unwrap();
    params.film_render.grain.active = false;
    params.film_render.halation.active = false;
    let clean = Pipeline::new_with_spectral(resolved.film, resolved.print, params, &data).unwrap();
    let input = ImageBuf::from_data(64, 64, vec![from_f64(0.184); 64 * 64 * 3]);
    let run = |p: &Pipeline| {
        p.process_with_taps(input.clone(), &CpuBackend, None, Some(Tap::RgbOut))
            .unwrap()
    };
    let textured = run(&look);
    let reference = run(&clean);
    assert!(textured.data.iter().all(|v| v.is_finite()));
    for c in 0..3 {
        let residual: Vec<f64> = (0..64 * 64)
            .map(|i| to_f64(textured.data[i * 3 + c] - reference.data[i * 3 + c]))
            .collect();
        let mean = residual.iter().sum::<f64>() / residual.len() as f64;
        let variance =
            residual.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / residual.len() as f64;
        assert!(mean.abs() < 0.01, "channel {c} mean shift {mean}");
        assert!(variance > 1e-12, "channel {c} texture missing");
    }
}
