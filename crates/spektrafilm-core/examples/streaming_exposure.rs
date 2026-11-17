//! Streaming multiple-exposure probe: load, expose and drop one shot at a
//! time, accumulate logE in one f64 buffer, then develop/print/scan once.
//!
//! This is a research example for high-resolution memory planning, not a
//! product API. Usage:
//!   streaming_exposure <data> <film> <paper> <out> <ev>:<image>...

use spektrafilm_backend::cpu_backend::CpuBackend;
use spektrafilm_core::{
    image_io::{self, BitDepth, SaveOptions},
    params::{RuntimeParams, Tap},
    pipeline::{ExposureShot, ShotMetering},
    profile,
    runtime::{DigestMode, RuntimePhotoParams},
};
use spektrafilm_math::{image::ImageBuf, precision::from_f64};
use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 5 {
        eprintln!("usage: streaming_exposure <data> <film> <paper> <out> <ev>:<image>...");
        std::process::exit(2);
    }
    let data = PathBuf::from(&args[0]);
    let film = profile::load_profile_by_name(&data, &args[1])?;
    let print = profile::load_profile_by_name(&data, &args[2])?;
    let out = PathBuf::from(&args[3]);
    let mut params = RuntimeParams::default();
    params.io.input_color_space = "sRGB".into();
    params.io.input_cctf_decoding = true;
    params.camera.auto_exposure = false;
    let runtime = RuntimePhotoParams {
        film: film.clone(),
        print: print.clone(),
        params,
        data_dir: data.clone(),
    }
    .into_runtime(DigestMode::ApplyStockSpecifics)?;
    let backend = CpuBackend;

    let mut accum: Option<Vec<f64>> = None;
    let mut shift: Option<f64> = None;
    let mut dims = None;
    for spec in &args[4..] {
        let parts: Vec<&str> = spec.split(':').collect();
        let (ev, path) = match parts.as_slice() {
            [ev, path] => (ev.parse::<f64>()?, *path),
            _ => return Err(format!("expected <ev>:<image>, got {spec:?}").into()),
        };
        let image = image_io::load(Path::new(path))?.image;
        if let Some((w, h)) = dims {
            if image.width != w || image.height != h {
                return Err("streaming shots must have identical dimensions".into());
            }
        } else {
            dims = Some((image.width, image.height));
            accum = Some(vec![0.0; image.width as usize * image.height as usize * 3]);
        }
        let shot = ExposureShot {
            image: &image,
            exposure_ev: ev,
            metering: ShotMetering::Manual,
        };
        let log_e = runtime.process_exposures(&[shot], &backend, Tap::LogEFilm)?;
        let shot_max = log_e
            .data
            .iter()
            .map(|&v| v as f64)
            .fold(f64::NEG_INFINITY, f64::max);
        let effective = match shift {
            None => {
                shift = Some(shot_max);
                shot_max
            }
            Some(current) if shot_max > current => {
                let delta = 10.0_f64.powf(current - shot_max);
                if let Some(values) = accum.as_mut() {
                    values.iter_mut().for_each(|v| *v *= delta);
                }
                shift = Some(shot_max);
                shot_max
            }
            Some(current) => current,
        };
        if let Some(values) = accum.as_mut() {
            for (acc, &value) in values.iter_mut().zip(&log_e.data) {
                *acc += 10.0_f64.powf(value as f64 - effective);
            }
        }
        drop(log_e);
        drop(image);
    }
    let (width, height) = dims.ok_or("no shots")?;
    let shift = shift.ok_or("no shots")?;
    let values = accum.ok_or("no shots")?;
    let mut combined = ImageBuf::new(width, height);
    for (slot, &sum) in combined.data.iter_mut().zip(&values) {
        *slot = from_f64(shift + sum.log10());
    }
    drop(values);
    let out_image =
        runtime.process_with_taps(combined, &backend, Some(Tap::LogEFilm), Some(Tap::RgbOut))?;
    image_io::save(
        &out,
        &out_image,
        SaveOptions {
            depth: BitDepth::ThirtyTwo,
            color_space: "sRGB",
            cctf_encoding: false,
            jpeg_quality: None,
            jpeg_subsampling: None,
            compression: None,
        },
        None,
    )?;
    Ok(())
}
