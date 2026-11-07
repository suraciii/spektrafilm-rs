//! Still 400TX is not the existing 7266 reversal. Test the actual loaded response.
use super::*;
use spektrafilm_backend::cpu_backend::CpuBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};

#[test]
fn trix400_loaded_curve_preserves_source_density_and_single_silver_record() {
    let data = crate::test_support::data_dir();
    let source: serde_json::Value = serde_json::from_slice(
        &std::fs::read(data.join("profile_sources/kodak_trix_400.json")).unwrap(),
    )
    .unwrap();
    let stored = load_profile_by_name(&data, "kodak_trix_400").unwrap();
    assert!(stored.is_bw() && !stored.is_positive() && stored.is_film());
    assert_eq!(stored.info.usage, "still");
    assert_eq!(stored.data.development_time, [6.]);
    let reversal = load_profile_by_name(&data, "kodak_trix").unwrap();
    assert!(reversal.is_positive());
    let points = source["characteristic"].as_array().unwrap();
    let origin = points[0][0].as_f64().unwrap() - stored.data.log_exposure[0];
    let film = resolve_for_render(stored, None);
    assert!(film.data.channel_density.iter().all(|v| v == &[1., 0., 0.]));
    let mut params = crate::params::RuntimeParams::default();
    params.film_render.grain.active = false;
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
    let density = crate::stages::filming::develop(&input, &film, &params, &CpuBackend, 6.).unwrap();
    for (pixel, point) in density.data.as_chunks::<3>().0.iter().zip(points) {
        assert_eq!(pixel[0], pixel[1]);
        assert_eq!(pixel[0], pixel[2]);
        assert!(
            (to_f64(pixel[0]) + film.data.base_density[0] - point[1].as_f64().unwrap()).abs()
                < 2e-5
        );
    }
    // Default grain consumers can prepare the aggregate record without fake layers.
    crate::stages::grain_dye::resolve(&film, &params).unwrap();
}
