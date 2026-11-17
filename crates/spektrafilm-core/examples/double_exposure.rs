//! Proof-of-concept multiple-exposure renderer.
//!
//! Run:
//! ```text
//! cargo run -p spektrafilm-core --example double_exposure --features precision-f64 -- \
//!   data kodak_portra_400 kodak_portra_endura out.tif \
//!   -1:a.png -1:b.png        # ev:path
//! ```
//!
//! Each `ev:path` pair is one exposure. `ev` is the physical exposure offset.
//! The example composes at `log_e_film` and develops once, the physically
//! faithful order.

use spektrafilm_backend::cpu_backend::CpuBackend;
use spektrafilm_core::{
    image_io::{self, BitDepth, SaveOptions},
    params::{RuntimeParams, Tap},
    pipeline::ExposureShot,
    profile,
    runtime::{DigestMode, RuntimePhotoParams},
};
use spektrafilm_math::image::ImageBuf;
use std::path::{Path, PathBuf};

fn usage() -> ! {
    eprintln!(
        "usage: double_exposure <data-dir> <film> <paper> <out.tif> <ev>:<image> [<ev>:<image> ...]"
    );
    std::process::exit(2)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 5 {
        usage();
    }
    let data_dir = PathBuf::from(&args[0]);
    let film_name = &args[1];
    let print_name = &args[2];
    let out = PathBuf::from(&args[3]);

    let mut images: Vec<ImageBuf> = Vec::new();
    let mut evs: Vec<f64> = Vec::new();
    for spec in &args[4..] {
        let parts: Vec<&str> = spec.split(':').collect();
        let (ev, path) = match parts.as_slice() {
            [ev, path] => (ev.parse::<f64>()?, *path),
            _ => {
                return Err(format!("expected <ev>:<image>, got {spec:?}").into());
            }
        };
        let loaded = image_io::load(Path::new(path))?;
        images.push(loaded.image);
        evs.push(ev);
    }
    let metering = match std::env::var("POC_METERING").as_deref() {
        Ok("lock-to-first") | Ok("lock_to_first") => {
            spektrafilm_core::pipeline::ShotMetering::LockToFirst
        }
        Ok("manual") => spektrafilm_core::pipeline::ShotMetering::Manual,
        _ => spektrafilm_core::pipeline::ShotMetering::Each,
    };
    let shots: Vec<ExposureShot<'_>> = images
        .iter()
        .zip(&evs)
        .map(|(image, &exposure_ev)| ExposureShot {
            image,
            exposure_ev,
            metering,
        })
        .collect();

    let film = profile::load_profile_by_name(&data_dir, film_name).map_err(|e| e.to_string())?;
    let print = profile::load_profile_by_name(&data_dir, print_name).map_err(|e| e.to_string())?;
    let mut params = RuntimeParams::default();
    params.io.input_color_space = "sRGB".into();
    params.io.input_cctf_decoding = true;
    // The camera meters each shot independently, like a camera in multiple
    // exposure mode. Set POC_AUTO_EXPOSURE=0 for a locked/manual exposure.
    params.camera.auto_exposure = std::env::var("POC_AUTO_EXPOSURE")
        .map(|value| value != "0")
        .unwrap_or(true);
    params.film_render.grain.active = std::env::var("POC_GRAIN")
        .map(|value| value != "0")
        .unwrap_or(true);

    let runtime = RuntimePhotoParams {
        film,
        print,
        params,
        data_dir: data_dir.clone(),
    }
    .into_runtime(DigestMode::ApplyStockSpecifics)
    .map_err(|e| e.to_string())?;
    let collect = match std::env::var("POC_COLLECT").as_deref() {
        Ok("log_e_film") => Tap::LogEFilm,
        Ok("cmy_film") => Tap::CmyFilm,
        _ => Tap::RgbOut,
    };
    let (rendered, report) = runtime.process_exposures_with_report(&shots, &CpuBackend, collect)?;

    let options = SaveOptions {
        depth: BitDepth::ThirtyTwo,
        color_space: "sRGB",
        cctf_encoding: false,
        jpeg_quality: None,
        jpeg_subsampling: None,
        compression: None,
    };
    image_io::save(&out, &rendered, options, None)?;
    if let Ok(raw_path) = std::env::var("POC_RAW_OUT") {
        let bytes: Vec<u8> = rendered
            .data
            .iter()
            .flat_map(|&value| spektrafilm_math::precision::to_f64(value).to_le_bytes())
            .collect();
        std::fs::write(raw_path, bytes)?;
    }
    println!(
        "wrote {} ({}x{}) from {} exposures; total_ev={:.3}; logE=[{:.3}, {:.3}]",
        out.display(),
        rendered.width,
        rendered.height,
        shots.len(),
        report.total_ev,
        report.log_e_range.0,
        report.log_e_range.1
    );
    for shot in &report.shots {
        println!(
            "  shot {}: metered_ev={:.3} exposure_ev={:.3} final_ev={:.3}",
            shot.index, shot.metered_ev, shot.exposure_ev, shot.final_ev
        );
    }
    Ok(())
}
