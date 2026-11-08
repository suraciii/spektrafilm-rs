use super::*;
use spektrafilm_backend::cpu_backend::CpuBackend;

#[path = "agfa100_fixture.rs"]
mod fixture;

fn candidate() -> Pipeline {
    let data = crate::test_support::data_dir();
    let preset = crate::presets::load_selector("built-in:Agfachrome RSX II 100", &data).unwrap();
    let resolved = preset.resolve(&data, &RuntimeParams::default()).unwrap();
    let mut params = resolved.params;
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
    params.settings.apply_hanatos2025_adaptation_surface = false;
    params.settings.apply_hanatos2025_adaptation_window = false;
    Pipeline::new_with_spectral(resolved.film, resolved.print, params, &data).unwrap()
}

#[test]
fn agfa100_loaded_density_reconstructs_published_status_a() {
    let pipeline = candidate();
    let data = crate::test_support::data_dir();
    let source: serde_json::Value = serde_json::from_slice(
        &std::fs::read(data.join("profile_sources/agfachrome_rsx_ii_100.json")).unwrap(),
    )
    .unwrap();
    assert!(pipeline.film.is_positive());
    assert!(pipeline.film.data.density_curves_model.is_none());
    let origin = source["coordinates"]["characteristic_green"][0][0]
        .as_f64()
        .unwrap()
        - pipeline.film.data.log_exposure[0];
    for (source_name, target_channel) in [("green", 1usize), ("red", 0), ("blue", 2)] {
        let points = source["coordinates"][format!("characteristic_{source_name}")]
            .as_array()
            .unwrap();
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
                    10f64.powf(-density) * w[target_channel]
                })
                .sum();
            let predicted = -transmission.log10();
            maximum = maximum.max((predicted - point[1].as_f64().unwrap()).abs());
        }
        println!("AGFA Status A {source_name}: max density error {maximum:.9}");
        assert!(maximum < 0.01, "{source_name}: {maximum}");
    }
    crate::stages::grain_dye::resolve(&pipeline.film, &pipeline.params).unwrap();
}

#[test]
fn agfa100_positive_readout_ignores_paper_and_rejects_negative_inversion() {
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
    let luminance: Vec<f64> = image
        .data
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| 0.2126 * to_f64(p[0]) + 0.7152 * to_f64(p[1]) + 0.0722 * to_f64(p[2]))
        .collect();
    assert!(luminance.windows(2).all(|p| p[1] >= p[0] - 1e-7));
    for c in 0..3 {
        assert!((to_f64(image.data[96 + c]) - 1.).abs() < 0.01);
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
fn agfa100_direct_readout_matches_independent_d50_spectral_oracle() {
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
fn agfa100_grain_stage_matches_rms10_at_48um() {
    let data = crate::test_support::data_dir();
    let stored = crate::profile::load_profile_by_name(&data, "agfachrome_rsx_ii_100").unwrap();
    let film = crate::profile::resolve_for_render(stored, None);
    let norm =
        spektrafilm_model::density_curves::normalize_density_curves_f64(&film.density_curves_f64());
    let preset = crate::presets::load_selector("built-in:Agfachrome RSX II 100", &data).unwrap();
    let resolved = preset.resolve(&data, &RuntimeParams::default()).unwrap();
    let mut grain = resolved.params.film_render.grain;
    grain.active = true;
    grain.engine = crate::params::GrainEngine::Spektra;
    grain.rms_granularity = [11.78; 3];
    grain.density_min = [0.03; 3];
    grain.uniformity = [0.97; 3];
    grain.particle_scale_sublayers = [1.0, 0.4, 0.25];
    grain.blur = 0.0;
    grain.blur_dye_clouds_um = 0.0;
    grain.micro_structure = [0.0; 2];
    grain.mult_usm_amount = 0.0;
    grain.monochrome = false;
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
    println!("AGFA grain stage 48um: mean={mean:.9} sigma={sigma:.9}");
    assert!((mean - 1.0).abs() < 1e-3, "mean {mean}");
    assert!((sigma - 10.0e-3).abs() < 1.5e-3, "sigma {sigma}");
}

#[test]
fn agfa100_channel_density_uses_runtime_cmy_order() {
    let data = crate::test_support::data_dir();
    let film = crate::profile::load_profile_by_name(&data, "agfachrome_rsx_ii_100").unwrap();
    let wavelengths = film.data.wavelengths.clone();
    // Runtime channel 0 is cyan (absorbs red), 1 magenta (green), 2 yellow
    // (blue). A Y/M/C storage order is the bug that produced blue skin.
    for (channel, expected_nm) in [(0usize, 660.0), (1, 545.0), (2, 445.0)] {
        let peak = film
            .data
            .channel_density
            .iter()
            .enumerate()
            .max_by(|a, b| a.1[channel].total_cmp(&b.1[channel]))
            .map(|(index, _)| wavelengths[index])
            .unwrap();
        assert!(
            (peak - expected_nm).abs() <= 5.0,
            "channel {channel}: peak {peak} nm, expected {expected_nm} nm"
        );
    }
}
