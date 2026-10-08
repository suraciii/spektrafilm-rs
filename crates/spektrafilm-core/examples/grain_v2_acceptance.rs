use spektrafilm_core::{
    image_io::{self, BitDepth, SaveOptions},
    params::{GrainEngine, GrainV2FilmType, GrainV2Mode, RuntimeParams},
    pipeline::Pipeline,
    profile,
};
use spektrafilm_gpu::cpu_backend::CpuBackend;
use spektrafilm_math::{image::ImageBuf, precision::from_f64};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let output = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("spektrafilm-grain-v2-acceptance"));
    std::fs::create_dir_all(&output)?;
    let image = ImageBuf::from_data(
        32,
        24,
        (0..32 * 24 * 3)
            .map(|i| from_f64(0.1 + (i % 97) as f64 / 120.))
            .collect(),
    );
    let film = profile::load_profile_by_name(&root, "kodak_portra_400")?;
    let paper = profile::load_profile_by_name(&root, "kodak_portra_endura")?;
    for scan_film in [false, true] {
        for engine in [GrainEngine::V1, GrainEngine::V2] {
            for (mode, film_type) in [
                (GrainV2Mode::Analogue, GrainV2FilmType::Negative),
                (GrainV2Mode::Analogue, GrainV2FilmType::Positive),
                (GrainV2Mode::Noise, GrainV2FilmType::Negative),
                (GrainV2Mode::Noise, GrainV2FilmType::Positive),
            ] {
                if engine == GrainEngine::V1
                    && (mode == GrainV2Mode::Noise || film_type == GrainV2FilmType::Positive)
                {
                    continue;
                }
                let mut params = RuntimeParams::default();
                params.camera.auto_exposure = false;
                params.io.scan_film = scan_film;
                params.film_render.grain.engine = engine;
                params.film_render.grain.select_custom_grain_v2();
                params.film_render.grain.v2_mode = mode;
                params.film_render.grain.v2_film_type = film_type;
                params.validate()?;
                let pipeline =
                    Pipeline::new_with_spectral(film.clone(), paper.clone(), params, &root)?;
                let result = pipeline.process(image.clone(), &CpuBackend)?;
                assert_eq!((result.width, result.height), (32, 24));
                assert!(result.data.iter().all(|v| v.is_finite()));
                let path =
                    output.join(format!("{engine:?}-{mode:?}-{film_type:?}-{scan_film}.tif"));
                image_io::save(
                    &path,
                    &result,
                    SaveOptions {
                        depth: BitDepth::ThirtyTwo,
                        color_space: "sRGB",
                        cctf_encoding: true,
                    },
                    None,
                )?;
                let reloaded = image_io::load(&path)?.image;
                assert_eq!((reloaded.width, reloaded.height), (32, 24));
                let error = result
                    .data
                    .iter()
                    .zip(&reloaded.data)
                    .map(|(a, b)| (*a as f64 - *b as f64).abs())
                    .fold(0.0f64, f64::max);
                assert!(error < 1e-6, "float TIFF roundtrip {error}");
                println!("render/export passed: {}", path.display());
            }
        }
    }
    Ok(())
}
