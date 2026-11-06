//! Published-curve and single-record regression tests for the bundled paper.
use super::*;
use spektrafilm_backend::cpu_backend::CpuBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};

#[test]
fn polymax_loaded_development_reproduces_published_reflection_density() {
    let data = crate::test_support::data_dir();
    let source: serde_json::Value = serde_json::from_slice(
        &std::fs::read(data.join("profile_sources/kodak_polymax_fine_art.json")).unwrap(),
    )
    .unwrap();
    let mut params = crate::params::RuntimeParams::default();
    params.print_render.density_curve_gamma = 1.0;
    // The exposure origin is recoverable from a stored source vertex and grid.
    let unfiltered = load_profile_by_name(&data, "kodak_polymax_fine_art").unwrap();
    let origin = -1.0 - unfiltered.data.log_exposure[0];
    let (label, stock) = ("unfiltered", "kodak_polymax_fine_art");
    let stored = load_profile_by_name(&data, stock).unwrap();
    assert!(stored.is_bw() && stored.is_paper() && stored.is_print_stock());
    assert_eq!(stored.readout_illuminant(), "D50");
    assert!(stored.data.development_time.is_empty());
    assert!(stored.data.density_curves_model.is_none());
    assert_eq!(
        stored.data.wavelengths,
        (0..81).map(|i| 380. + 5. * i as f64).collect::<Vec<_>>()
    );
    assert!(
        stored
            .data
            .density_curves
            .windows(2)
            .all(|r| r[1][0] >= r[0][0])
    );
    let paper = resolve_for_render(stored, None);
    assert!(
        paper
            .data
            .channel_density
            .iter()
            .all(|r| r == &[1., 0., 0.])
    );
    let points = source["characteristic"][label].as_array().unwrap();
    let input = ImageBuf::from_data(
        points.len() as u32,
        1,
        points
            .iter()
            .flat_map(|p| [from_f64(p[0].as_f64().unwrap() - origin); 3])
            .collect(),
    );
    let developed = crate::stages::printing::develop(&input, &paper, &params, &CpuBackend).unwrap();
    for (pixel, point) in developed.data.as_chunks::<3>().0.iter().zip(points) {
        assert_eq!(pixel[0], pixel[1]);
        assert_eq!(pixel[0], pixel[2]);
        let total = to_f64(pixel[0]) + paper.data.base_density[0];
        assert!(
            (total - point[1].as_f64().unwrap()).abs() < 2e-5,
            "{stock}: {total} vs {point}"
        );
    }
    // More paper exposure must darken the print, with bounded tails.
    let endpoints = ImageBuf::from_data(
        2,
        1,
        vec![
            from_f64(-20.),
            from_f64(-20.),
            from_f64(-20.),
            from_f64(20.),
            from_f64(20.),
            from_f64(20.),
        ],
    );
    let developed =
        crate::stages::printing::develop(&endpoints, &paper, &params, &CpuBackend).unwrap();
    assert!(to_f64(developed.data[0]).abs() < 1e-8);
    assert!(to_f64(developed.data[3]) > 2.);
}

#[test]
fn polymax_color_neutralization_refuses_an_unidentifiable_target() {
    let data = crate::test_support::data_dir();
    let paper = load_profile_by_name(&data, "kodak_polymax_fine_art").unwrap();
    let film = load_profile_by_name(&data, "kodak_doublex").unwrap();
    let params = crate::params::RuntimeParams::default();
    assert!(
        crate::stages::converting::neutralize_filters(&film, &paper, &params, &data, &CpuBackend)
            .is_err()
    );
}
