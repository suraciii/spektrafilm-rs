//! Full-path CPU vs GPU multiple-exposure visual gate.
//!
//! Runs on the selected WGPU adapter. In CI use:
//! `SPEKTRAFILM_GPU_SOFTWARE=1 cargo test -p spektrafilm-core --test multiple_exposure_gpu_visual`.
//! A missing adapter skips this test; the software adapter is the reference
//! lane for machines whose hardware driver is unstable.
//!
//! This small synthetic fixture is a *smoke* visual gate; the large-image
//! fixtures use the stricter PSNR/p99 gate documented in gpu.md.

use spektrafilm_backend::cpu_backend::CpuBackend;
use spektrafilm_core::params::{RuntimeParams, Tap};
use spektrafilm_core::pipeline::{ExposureShot, Pipeline, ShotMetering};
use spektrafilm_core::profile;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::from_f64;

fn synthetic(width: u32, height: u32, phase: f64) -> ImageBuf {
    let mut image = ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let fx = x as f64 / (width - 1).max(1) as f64;
            let fy = y as f64 / (height - 1).max(1) as f64;
            image.set(
                x,
                y,
                [
                    from_f64((0.03 + 0.80 * fx * (0.3 + 0.7 * fy) + phase).clamp(0.0, 1.0)),
                    from_f64((0.03 + 0.65 * (1.0 - fx) * (0.2 + 0.8 * fy)).clamp(0.0, 1.0)),
                    from_f64((0.03 + 0.5 * (0.5 + 0.5 * (fx - fy).sin())).clamp(0.0, 1.0)),
                ],
            );
        }
    }
    image
}

fn blur3(image: &ImageBuf) -> Vec<f64> {
    let (w, h) = (image.width as isize, image.height as isize);
    let mut out = vec![0.0; image.data.len()];
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                let mut sum = 0.0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let yy = (y + dy).clamp(0, h - 1);
                        let xx = (x + dx).clamp(0, w - 1);
                        let i = ((yy * w + xx) * 3 + c) as usize;
                        sum += image.data[i] as f64;
                    }
                }
                out[((y * w + x) * 3 + c) as usize] = sum / 9.0;
            }
        }
    }
    out
}

fn metrics(cpu: &ImageBuf, gpu: &ImageBuf) -> (f64, f64, f64, f64, f64) {
    assert_eq!(cpu.data.len(), gpu.data.len());
    let mut max = 0.0_f64;
    let mut sq = 0.0_f64;
    let mut diffs = Vec::with_capacity(cpu.data.len());
    for (&a, &b) in cpu.data.iter().zip(&gpu.data) {
        let d = (a as f64 - b as f64).abs();
        max = max.max(d);
        sq += d * d;
        diffs.push(d);
    }
    diffs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p99 = diffs[(diffs.len() - 1) * 99 / 100];
    let mse = sq / cpu.data.len() as f64;
    let psnr = if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (1.0 / mse).log10()
    };
    let cb = blur3(cpu);
    let gb = blur3(gpu);
    let low_mse = cb
        .iter()
        .zip(&gb)
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f64>()
        / cb.len() as f64;
    let lowpass_psnr = if low_mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (1.0 / low_mse).log10()
    };
    let mut cpu_hp = 0.0;
    let mut gpu_hp = 0.0;
    for i in 0..cpu.data.len() {
        cpu_hp += (cpu.data[i] as f64 - cb[i]).powi(2);
        gpu_hp += (gpu.data[i] as f64 - gb[i]).powi(2);
    }
    let cpu_std = (cpu_hp / cpu.data.len() as f64).sqrt();
    let gpu_std = (gpu_hp / gpu.data.len() as f64).sqrt();
    let grain_ratio = if cpu_std == 0.0 {
        f64::NAN
    } else {
        gpu_std / cpu_std
    };
    (max, p99, psnr, lowpass_psnr, grain_ratio)
}

#[test]
fn fully_resident_gpu_matches_cpu_within_visual_gate() {
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.io.input_color_space = "sRGB".into();
    params.io.input_cctf_decoding = false;
    params.camera.auto_exposure = false;
    params.camera.film_format_mm = 3.8912;
    let cpu =
        Pipeline::new_with_spectral(film.clone(), print.clone(), params.clone(), &dir).unwrap();
    let gpu_pipeline =
        Pipeline::new_with_spectral_backend(film, print, params, &dir, gpu.backend()).unwrap();
    let a = synthetic(1024, 1024, 0.0);
    let b = synthetic(1024, 1024, 0.08);
    let shots = [
        ExposureShot {
            image: &a,
            exposure_ev: -1.0,
            weight: 1.0,
            metering: ShotMetering::Manual,
        },
        ExposureShot {
            image: &b,
            exposure_ev: -1.0,
            weight: 1.0,
            metering: ShotMetering::Manual,
        },
    ];
    let cpu_out = cpu
        .process_exposures(&shots, &CpuBackend, Tap::RgbOut)
        .unwrap();
    let gpu_out = gpu_pipeline
        .process_exposures_gpu(&shots, gpu.backend(), Tap::RgbOut)
        .unwrap()
        .0;
    let (max, p99, psnr, lowpass_psnr, grain_ratio) = metrics(&cpu_out, &gpu_out);
    eprintln!(
        "GPU visual gate: max={max:.4e} p99={p99:.4e} pixel_psnr={psnr:.2} dB \
         lowpass_psnr={lowpass_psnr:.2} dB grain_ratio={grain_ratio:.4}"
    );
    // Independent stochastic grain is expected; compare structure and grain
    // energy rather than requiring identical noise pixels.
    assert!(psnr >= 50.0, "pixel PSNR {psnr:.2} dB below 50 dB");
    assert!(
        lowpass_psnr >= 55.0,
        "low-pass PSNR {lowpass_psnr:.2} dB below 55 dB"
    );
    assert!(p99 <= 0.02, "p99 {p99:.4e} above 0.02");
    assert!(max <= 0.25, "max {max:.4e} above 0.25");
    assert!(
        (0.90..=1.10).contains(&grain_ratio),
        "grain std ratio {grain_ratio:.4} outside [0.90, 1.10]"
    );
}

#[test]
fn gpu_rejects_all_zero_weights_like_cpu() {
    let gpu = spektrafilm_gpu::test_support::explicit_adapter();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
    let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
    let mut params = RuntimeParams::default();
    params.io.input_color_space = "sRGB".into();
    params.io.input_cctf_decoding = false;
    params.camera.auto_exposure = false;
    params.camera.film_format_mm = 3.8912;
    let pipeline =
        Pipeline::new_with_spectral_backend(film, print, params, &dir, gpu.backend()).unwrap();
    let a = synthetic(64, 64, 0.0);
    let b = synthetic(64, 64, 0.08);
    let shots = [
        ExposureShot {
            image: &a,
            exposure_ev: -1.0,
            weight: 0.0,
            metering: ShotMetering::Manual,
        },
        ExposureShot {
            image: &b,
            exposure_ev: -1.0,
            weight: 0.0,
            metering: ShotMetering::Manual,
        },
    ];
    let error = pipeline
        .process_exposures_gpu(&shots, gpu.backend(), Tap::RgbOut)
        .expect_err("all-zero weights must be rejected");
    assert!(
        error.contains("positive finite total weight"),
        "unexpected GPU error: {error}"
    );
}
