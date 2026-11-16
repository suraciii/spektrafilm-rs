//! Research probe: retained per-shot logE and device logE composition on GPU.
//! Usage: gpu_exposure_probe <data> <film> <paper> <a.tif> <b.tif> <out.tif>

use spektrafilm_backend::ComputeBackend;
use spektrafilm_core::{
    image_io::{self, BitDepth, SaveOptions},
    params::{RuntimeParams, Tap},
    pipeline::{ExposureShot, Pipeline, RetainedRender, ShotMetering},
    profile,
};
use spektrafilm_gpu::wgpu_backend::WgpuBackend;
use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        eprintln!("usage: gpu_exposure_probe <data> <film> <paper> <a.tif> <b.tif> <out.tif>");
        std::process::exit(2);
    }
    let data = PathBuf::from(&args[0]);
    let film = profile::load_profile_by_name(&data, &args[1])?;
    let print = profile::load_profile_by_name(&data, &args[2])?;
    let a = image_io::load(Path::new(&args[3]))?.image;
    let b = image_io::load(Path::new(&args[4]))?.image;
    let out = PathBuf::from(&args[5]);
    let gpu = WgpuBackend::new_explicit().ok_or("GPU unavailable")?;
    let cpu = spektrafilm_backend::cpu_backend::CpuBackend;
    let mut params = RuntimeParams::default();
    params.io.input_color_space = "sRGB".into();
    params.io.input_cctf_decoding = true;
    params.camera.auto_exposure = false;
    let pipeline = Pipeline::new_with_spectral_backend(film, print, params, &data, &gpu)?;
    let ev = -1.0;
    let mut accumulator = None;
    for image in [&a, &b] {
        let mut shot = pipeline.clone();
        shot.params.camera.exposure_compensation_ev = ev as f32;
        let retained = shot
            .process_retained_with_taps(
                image,
                RetainedRender::FULL,
                &gpu,
                Some(Tap::RgbIn),
                Some(Tap::LogEFilm),
            )?
            .ok_or("GPU declined retained logE")?;
        eprintln!(
            "retained logE {}x{} domain={:?}",
            retained.width(),
            retained.height(),
            retained.domain()
        );
        eprintln!("compose start");
        accumulator = Some(
            gpu.log_e_accumulate(accumulator.as_ref(), &retained, 1.0)?
                .ok_or("GPU declined logE accumulation")?,
        );
        eprintln!("compose done");
    }
    eprintln!("finalize start");
    let composite = gpu
        .log_e_finalize(accumulator.as_ref().expect("at least one shot"))?
        .ok_or("GPU declined logE finalize")?;
    eprintln!("finalize done; readback start");
    if std::env::var_os("GPU_RETAINED_TAIL").is_some() {
        let tail_tap = match std::env::var("GPU_TAIL_TAP").as_deref() {
            Ok("cmy_film") => Tap::CmyFilm,
            Ok("cmy_print") => Tap::CmyPrint,
            _ => Tap::RgbOut,
        };
        eprintln!("retained tail start tap={tail_tap:?}");
        let retained = pipeline.process_device_retained_with_taps(
            &composite,
            RetainedRender::FULL,
            &gpu,
            Some(Tap::LogEFilm),
            Some(tail_tap),
        )?;
        let retained = retained.to_host(&gpu)?;
        eprintln!("retained tail done");
        image_io::save(
            &out.with_extension("retained.tif"),
            &retained,
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
    }
    let combined = composite.to_host(&gpu)?;
    let cpu_pipeline = Pipeline::new_with_spectral(
        profile::load_profile_by_name(&data, &args[1])?,
        profile::load_profile_by_name(&data, &args[2])?,
        {
            let mut params = RuntimeParams::default();
            params.io.input_color_space = "sRGB".into();
            params.io.input_cctf_decoding = true;
            params.camera.auto_exposure = false;
            params
        },
        &data,
    )?;
    let mut cpu_acc = vec![0.0_f64; combined.data.len()];
    for image in [&a, &b] {
        let shot = ExposureShot {
            image,
            exposure_ev: ev,
            weight: 1.0,
            metering: ShotMetering::Manual,
        };
        let log_e = cpu_pipeline.process_exposures(&[shot], &cpu, Tap::LogEFilm)?;
        for (acc, &value) in cpu_acc.iter_mut().zip(&log_e.data) {
            *acc += 10.0_f64.powf(value as f64 + 1.0);
        }
    }
    let mut max = 0.0_f64;
    let mut rms = 0.0_f64;
    for (i, (&gpu_value, &acc)) in combined.data.iter().zip(&cpu_acc).enumerate() {
        let cpu_value = acc.log10() - 1.0;
        let d = (gpu_value as f64 - cpu_value).abs();
        max = max.max(d);
        rms += d * d;
        let _ = i;
    }
    eprintln!(
        "device compose vs CPU logE: max={max:.6e} rms={:.6e}",
        (rms / combined.data.len() as f64).sqrt()
    );
    eprintln!("readback done; final chain start");
    let shots = [
        ExposureShot {
            image: &a,
            exposure_ev: ev,
            weight: 1.0,
            metering: ShotMetering::Manual,
        },
        ExposureShot {
            image: &b,
            exposure_ev: ev,
            weight: 1.0,
            metering: ShotMetering::Manual,
        },
    ];
    eprintln!("core GPU path start");
    let (gpu_rendered, _report) = pipeline.process_exposures_gpu(&shots, &gpu, Tap::RgbOut)?;
    eprintln!("core GPU path done");
    image_io::save(
        &out.with_extension("gpu.tif"),
        &gpu_rendered,
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
    let rendered =
        cpu_pipeline.process_with_taps(combined, &cpu, Some(Tap::LogEFilm), Some(Tap::RgbOut))?;
    image_io::save(
        &out,
        &rendered,
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
