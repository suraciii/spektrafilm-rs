use super::*;
use crate::test_support::data_dir;
use spektrafilm_backend::cpu_backend::CpuBackend;

#[test]
fn polymax_print_updates_match_cold_preparation() {
    let data = data_dir();
    let film = crate::profile::load_profile_by_name(&data, "kodak_doublex").unwrap();
    let paper = crate::profile::load_profile_by_name(&data, "kodak_polymax_fine_art").unwrap();
    let mut params = RuntimeParams::default();
    params.workflow.route = "input > film > print > scan".into();
    params.io.scan_film = false;
    params.io.output_cctf_encoding = false;
    params.io.output_gamut_compress.algorithm = "off".into();
    params.camera.auto_exposure = false;
    params.film_render.grain.active = false;
    params.film_render.halation.active = false;
    params.film_render.dir_couplers.active = false;
    params.print_render.glare.active = false;
    params.scanner.unsharp_mask = [0.; 2];
    params.settings.use_enlarger_lut = false;
    params.settings.use_scanner_lut = false;
    params.enlarger.illuminant = "T".into();
    params = crate::params_builder::digest_params(params, &film, &paper, None, false);
    let pipeline =
        Pipeline::new_with_spectral(film.clone(), paper.clone(), params.clone(), &data).unwrap();
    // Inject neutral negative density so this acceptance isolates paper exposure,
    // development and readout from the camera spectrum adapter and film grain.
    let input = ImageBuf::from_data(
        16,
        1,
        (0..16)
            .flat_map(|i| [from_f64(i as f64 * 0.15); 3])
            .collect(),
    );
    let render = |p: &Pipeline| {
        p.process_with_taps(
            input.clone(),
            &CpuBackend,
            Some(Tap::CmyFilm),
            Some(Tap::RgbOut),
        )
        .unwrap()
    };
    let initial = render(&pipeline);
    // Automatic baseline and every authored-axis combination must survive an
    // incremental update exactly as they do cold preparation.
    for enabled in [false, true] {
        for mask in 0..8 {
            let mut changed = params.clone();
            changed.settings.neutral_print_filters_from_database = enabled;
            changed.neutral_print_filters_protected =
                Some(std::array::from_fn(|axis| mask & (1 << axis) != 0));
            changed.enlarger.c_filter_neutral = 17.;
            changed.enlarger.m_filter_neutral = 23.;
            changed.enlarger.y_filter_neutral = 91.;
            let updated = pipeline
                .clone()
                .with_params_with_backend(changed.clone(), &CpuBackend)
                .unwrap();
            let fresh =
                Pipeline::new_with_spectral(film.clone(), paper.clone(), changed, &data).unwrap();
            assert_eq!(
                updated.print_calibration.as_ref().unwrap().neutral_cmy,
                fresh.print_calibration.as_ref().unwrap().neutral_cmy
            );
            let a = render(&updated);
            let b = render(&fresh);
            for (a, b) in a.data.iter().zip(&b.data) {
                assert!((to_f64(*a) - to_f64(*b)).abs() < 1e-6);
            }
        }
    }
    assert!(initial.data.iter().all(|v| v.is_finite()));
    assert!(
        initial
            .data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| p[0])
            .collect::<Vec<_>>()
            .windows(2)
            .all(|v| v[1] >= v[0])
    );
    params.enlarger.print_exposure = 2.;
    let updated = pipeline
        .clone()
        .with_params_with_backend(params.clone(), &CpuBackend)
        .unwrap();
    let cold = Pipeline::new_with_spectral(film.clone(), paper, params.clone(), &data).unwrap();
    let darker = render(&updated);
    let fresh = render(&cold);
    for (a, b) in darker.data.iter().zip(&fresh.data) {
        assert!((to_f64(*a) - to_f64(*b)).abs() < 1e-6);
    }
    assert!(
        darker
            .data
            .iter()
            .sum::<spektrafilm_math::precision::Scalar>()
            < initial.data.iter().sum()
    );
}
