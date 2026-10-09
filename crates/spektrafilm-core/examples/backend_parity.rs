// Run after integrating geometry and central color semantics.
// cargo run -p spektrafilm-core --example backend_parity --features precision-f64 -- data
use spektrafilm_core::{
    params::{RuntimeParams, Tap},
    pipeline::Pipeline,
    profile,
};
use spektrafilm_gpu::{ComputeBackend, cpu_backend::CpuBackend, wgpu_backend::WgpuBackend};
use spektrafilm_math::{
    image::ImageBuf,
    precision::{from_f64, to_f64},
};
use std::path::PathBuf;

const BUDGET: f64 = 0.005; // Provisional smoke limit, report actual errors independently.

fn difference(a: &ImageBuf, b: &ImageBuf) -> Result<(f64, f64), String> {
    if (a.width, a.height) != (b.width, b.height) {
        return Err(format!(
            "geometry mismatch {}x{} vs {}x{}",
            a.width, a.height, b.width, b.height
        ));
    }
    let mut max = 0.0f64;
    let mut sum = 0.0;
    for (&a, &b) in a.data.iter().zip(&b.data) {
        let d = (to_f64(a) - to_f64(b)).abs();
        if !d.is_finite() {
            return Err("nonfinite output".into());
        }
        max = max.max(d);
        sum += d;
    }
    Ok((max, sum / a.data.len() as f64))
}

fn main() -> Result<(), String> {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "data".into()));
    let gpu = WgpuBackend::new().ok_or("WGSL adapter unavailable; no GPU pass recorded")?;
    let cpu = CpuBackend;
    println!(
        "backend={} cpu={} reference=3bb2c2d provisional_max_abs_budget={BUDGET}",
        gpu.name(),
        cpu.name()
    );
    let film =
        profile::load_profile_by_name(&dir, "kodak_portra_400").map_err(|e| e.to_string())?;
    let print =
        profile::load_profile_by_name(&dir, "kodak_portra_endura").map_err(|e| e.to_string())?;
    let data = (0..48 * 32)
        .flat_map(|i| {
            let x = i % 48;
            let y = i / 48;
            let v = 0.0001 + 0.8 * x as f64 / 47.0;
            let rgb = if x >= 36 && y < 8 {
                [4.0, 2.0, 1.0]
            } else if y > 24 {
                [v, 0.01, 0.002]
            } else {
                [v, v * 0.8, v * 0.6]
            };
            rgb.into_iter().map(from_f64)
        })
        .collect();
    let input = ImageBuf::from_data(48, 32, data);
    let mut base = RuntimeParams::default();
    base.camera.auto_exposure = false;
    base.io.input_color_space = "sRGB".into();
    base.io.output_gamut_compress.algorithm = "off".into();
    base.film_render.grain.active = false;
    base.film_render.halation.active = false;
    base.film_render.dir_couplers.active = false;
    base.print_render.glare.active = false;
    base.settings.use_enlarger_lut = false;
    base.settings.use_scanner_lut = false;
    base.scanner.unsharp_mask = [0.0, 0.0];
    let mut baseline: Option<ImageBuf> = None;
    for space in [
        "sRGB",
        "Display P3",
        "Adobe RGB (1998)",
        "ProPhoto RGB",
        "ITU-R BT.2020",
        "ACES2065-1",
        "ACEScg",
    ] {
        for encoded in [false, true] {
            for case in [
                "plain",
                "scan",
                "geometry",
                "controls",
                "diffusion",
                "decode",
                "spectral_lut",
                "wide_blur",
            ] {
                let mut p = base.clone();
                p.io.output_color_space = space.into();
                p.io.output_cctf_encoding = encoded;
                let resident_expected =
                    !matches!(case, "diffusion" | "decode" | "spectral_lut" | "wide_blur");
                match case {
                    "scan" => p.io.scan_film = true,
                    "geometry" => {
                        p.io.crop = true;
                        p.io.crop_center = [0.62, 0.45];
                        p.io.crop_size = [0.6, 0.7];
                        p.io.upscale_factor = 0.75;
                    }
                    "controls" => {
                        p.camera.filter_uv = [0.8, 430.0, 12.0];
                        p.camera.filter_ir = [0.6, 630.0, 20.0];
                        p.camera.lens_blur_um = 700.0;
                        p.scanner.lens_blur = 0.7;
                        p.scanner.unsharp_mask = [0.7, 0.3];
                        p.film_render.halation.boost_ev = 0.5;
                        p.enlarger.preflash_exposure = 0.02;
                        p.enlarger.y_filter_shift = 8.0;
                        p.enlarger.m_filter_shift = -5.0;
                    }
                    "diffusion" => {
                        p.camera.diffusion_filter.active = true;
                        p.camera.diffusion_filter.spatial_scale = 2.0;
                        p.camera.diffusion_filter.strength = 1.0;
                    }
                    "decode" => p.io.input_cctf_decoding = true,
                    "spectral_lut" => {
                        p.settings.use_enlarger_lut = true;
                        p.settings.use_scanner_lut = true;
                    }
                    "wide_blur" => p.scanner.lens_blur = 90.0,
                    _ => {}
                }
                let pipeline = Pipeline::new_with_spectral(film.clone(), print.clone(), p, &dir)?;
                let reference = pipeline.process_with_taps(input.clone(), &cpu, None, None)?;
                // Split immediately before scanning to force stage dispatch while
                // preserving full-input metering and physical pitch upstream.
                let scan_input = if pipeline.params.io.scan_film {
                    Tap::CmyFilm
                } else {
                    Tap::CmyPrint
                };
                let densities = pipeline.process_with_taps(
                    input.clone(),
                    &gpu,
                    Some(Tap::RgbIn),
                    Some(scan_input),
                )?;
                let stages = pipeline.process_with_taps(
                    densities,
                    &gpu,
                    Some(scan_input),
                    Some(Tap::RgbOut),
                )?;
                let routed = pipeline.process(input.clone(), &gpu)?;
                let resident = pipeline.process_resident_borrowed(&input, &gpu)?;
                if resident.is_some() != resident_expected {
                    return Err(format!(
                        "{space}/{encoded}/{case}: unexpected resident route"
                    ));
                }
                for (label, output) in [("stages", &stages), ("routed", &routed)] {
                    let (max, mean) = difference(&reference, output)?;
                    let range = output
                        .data
                        .iter()
                        .map(|&v| to_f64(v))
                        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
                            (lo.min(v), hi.max(v))
                        });
                    println!(
                        "space={space:?} encoded={encoded} case={case} path={label} geometry={}x{} range={range:?} max_abs={max:.9e} mean_abs={mean:.9e} resident={resident_expected}",
                        output.width, output.height
                    );
                    if max > BUDGET {
                        return Err(format!(
                            "{space}/{encoded}/{case}/{label}: {max} exceeds provisional {BUDGET}"
                        ));
                    }
                    if range.1 - range.0 < 1e-6 {
                        return Err(format!("{case}: blank output"));
                    }
                }
                if let Some(resident) = resident {
                    let (max, mean) = difference(&stages, &resident)?;
                    println!("resident_vs_stages max_abs={max:.9e} mean_abs={mean:.9e}");
                    if max > BUDGET {
                        return Err(format!("{case}: resident drift {max}"));
                    }
                }
                if space == "sRGB" && !encoded {
                    if case == "plain" {
                        baseline = Some(reference);
                    } else if matches!(case, "controls" | "diffusion" | "decode") {
                        let (max, _) = difference(baseline.as_ref().unwrap(), &reference)?;
                        if max < 1e-6 {
                            return Err(format!("{case}: controls produced no visible change"));
                        }
                        println!("control_change case={case} max_abs={max:.9e}");
                    }
                }
            }
        }
    }
    for layered in [false, true] {
        let mut p = base.clone();
        p.film_render.grain.active = true;
        p.film_render.grain.sublayers_active = layered;
        let pipeline = Pipeline::new_with_spectral(film.clone(), print.clone(), p, &dir)?;
        if pipeline.process_resident_borrowed(&input, &gpu)?.is_some() {
            return Err(format!(
                "grain layered={layered}: expected faithful CPU sampler fallback"
            ));
        }
        let reference = pipeline.process_with_taps(input.clone(), &cpu, None, None)?;
        let routed = pipeline.process(input.clone(), &gpu)?;
        let (max, mean) = difference(&reference, &routed)?;
        println!(
            "grain layered={layered} execution=cpu_sampler max_abs={max:.9e} mean_abs={mean:.9e}"
        );
        if max > BUDGET {
            return Err(format!("grain CPU fallback drift {max}"));
        }
    }
    Ok(())
}
