use super::*;
use spektrafilm_backend::cpu_backend::CpuBackend;

fn preset_pipeline(data: &std::path::Path) -> Pipeline {
    let preset = crate::presets::load_selector("built-in:Ilford XP2 Super 400", data).unwrap();
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
    params.film_render.grain.active = false;
    params.film_render.halation.active = false;
    Pipeline::new_with_spectral(resolved.film, resolved.print, params, data).unwrap()
}

#[test]
fn xp2_loaded_curve_preserves_source_density_and_single_record() {
    let data = crate::test_support::data_dir();
    let source: serde_json::Value = serde_json::from_slice(
        &std::fs::read(data.join("profile_sources/ilford_xp2_super_400.json")).unwrap(),
    )
    .unwrap();
    let stored = crate::profile::load_profile_by_name(&data, "ilford_xp2_super_400").unwrap();
    assert!(stored.is_bw() && !stored.is_positive() && stored.is_film());
    assert_eq!(stored.info.usage, "still");
    assert_eq!(stored.data.development_time, [0.0]);
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
                < 3e-5,
            "source density mismatch at {:?}",
            point
        );
    }
    crate::stages::grain_dye::resolve(&film, &params).unwrap();
}

#[test]
fn xp2_builtin_uses_dye_grain_without_authored_rms_and_keeps_print_route() {
    let data = crate::test_support::data_dir();
    let preset = crate::presets::load_selector("built-in:Ilford XP2 Super 400", &data).unwrap();
    let resolved = preset.resolve(&data, &RuntimeParams::default()).unwrap();
    assert_eq!(resolved.film_name, "ilford_xp2_super_400");
    assert_eq!(resolved.print_name, "kodak_polymax_fine_art");
    assert_eq!(
        resolved.params.workflow.route,
        "input > film > print > scan"
    );
    assert_eq!(resolved.params.scanner.scan_output, "direct_scan");
    assert!(resolved.params.film_render.grain.active);
    assert_eq!(
        resolved.params.film_render.grain.engine,
        crate::params::GrainEngine::Dye
    );
    assert_eq!(resolved.params.film_render.grain.rms_granularity, [0.; 3]);
    assert_eq!(resolved.params.film_render.grain.dye_strength, 1.0);
    assert_eq!(resolved.params.film_render.grain.dye_scale, 1.0);
    assert_eq!(resolved.params.film_render.halation.scatter_amount, 0.10);
    assert_eq!(resolved.params.film_render.halation.halation_amount, 0.10);
    assert_eq!(
        resolved.params.film_render.halation.halation_strength,
        [0.03, 0.01, 0.]
    );
    assert!(!resolved.params.settings.use_cat16);
    assert_eq!(resolved.params.enlarger.illuminant, "T");
    assert!(!resolved.params.film_render.chemistry.active);
    assert!(!resolved.params.film_render.dir_couplers.active);
    assert_eq!(resolved.params.scanner.unsharp_mask, [0., 0.]);
    assert_eq!(resolved.params.scanner.lens_blur, 0.);
}

#[test]
fn xp2_pipeline_renders_a_finite_positive() {
    let data = crate::test_support::data_dir();
    let pipeline = preset_pipeline(&data);
    let input = ImageBuf::from_data(
        5,
        1,
        [0.0, 0.25, 0.5, 0.75, 1.0]
            .iter()
            .flat_map(|v| [from_f64(*v); 3])
            .collect(),
    );
    let output = pipeline.process(input, &CpuBackend).unwrap();
    assert!(output.data.iter().all(|v| v.is_finite()));
    assert!(output.data[0] < output.data[output.data.len() - 3]);
}
