//! Experimental probe: does a haloed spatial tile reproduce the full-frame
//! render for the default film chain? This is a research test, not a public
//! API. It deliberately uses the full `Pipeline::process` path per tile so a
//! pass covers expose, develop, print and scan, including halation/DIR/grain.

use super::*;
use crate::profile;
use spektrafilm_backend::cpu_backend::CpuBackend;

fn reflect(index: isize, length: usize) -> usize {
    let period = 2 * length as isize;
    let folded = index.rem_euclid(period);
    if folded < length as isize {
        folded as usize
    } else {
        (period - 1 - folded) as usize
    }
}

fn crop_clip(image: &ImageBuf, x0: isize, y0: isize, width: u32, height: u32) -> ImageBuf {
    let x0 = x0.clamp(0, image.width as isize - width as isize);
    let y0 = y0.clamp(0, image.height as isize - height as isize);
    let mut out = ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            out.set(
                x,
                y,
                image.get((x0 + x as isize) as u32, (y0 + y as isize) as u32),
            );
        }
    }
    out
}

fn crop_reflect(image: &ImageBuf, x0: isize, y0: isize, width: u32, height: u32) -> ImageBuf {
    let mut out = ImageBuf::new(width, height);
    for y in 0..height as isize {
        for x in 0..width as isize {
            let sx = reflect(x0 + x, image.width as usize) as u32;
            let sy = reflect(y0 + y, image.height as usize) as u32;
            out.set(x as u32, y as u32, image.get(sx, sy));
        }
    }
    out
}

fn synthetic(width: u32, height: u32) -> ImageBuf {
    let mut image = ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let fx = x as f64 / (width - 1).max(1) as f64;
            let fy = y as f64 / (height - 1).max(1) as f64;
            let r = 0.04 + 0.80 * fx * (0.35 + 0.65 * fy);
            let g = 0.04 + 0.70 * (1.0 - fx) * (0.25 + 0.75 * fy);
            let b = 0.04 + 0.55 * (0.5 + 0.5 * (fx - fy).sin());
            image.set(x, y, [from_f64(r), from_f64(g), from_f64(b)]);
        }
    }
    // A bright block exercises halation/DIR far from the tile center.
    for y in height / 2 - 8..height / 2 + 8 {
        for x in width / 2 - 16..width / 2 + 16 {
            image.set(x, y, [from_f64(0.98); 3]);
        }
    }
    image
}

fn diff(a: &ImageBuf, b: &ImageBuf) -> (f64, f64) {
    let mut max = 0.0_f64;
    let mut sum = 0.0_f64;
    let mut n = 0usize;
    for (x, y) in a.data.iter().zip(&b.data) {
        let d = (to_f64(*x) - to_f64(*y)).abs();
        max = max.max(d);
        sum += d * d;
        n += 1;
    }
    (max, (sum / n as f64).sqrt())
}

#[test]
#[ignore = "research probe; run explicitly"]
fn haloed_tiles_match_full_frame_default_chain() {
    let data = crate::test_support::data_dir();
    let film = profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
    let print = profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
    let backend = CpuBackend;
    let pitch_um = 3.8_f64; // ~61 MP full frame; representative spatial scale
    let full_w = 512u32;
    let full_h = 512u32;
    let tile_w = 128u32;
    let tile_h = 128u32;
    let image = synthetic(full_w, full_h);

    let make_pipeline = |width: u32, height: u32| {
        let mut params = RuntimeParams::default();
        params.camera.auto_exposure = false;
        params.film_render.grain.active = false;
        params.camera.film_format_mm = (pitch_um * width.max(height) as f64 / 1000.0) as f32;
        params.io.crop = false;
        params.io.upscale_factor = 1.0;
        Pipeline::new_with_spectral(film.clone(), print.clone(), params, &data).unwrap()
    };

    let full_pipeline = make_pipeline(full_w, full_h);
    let full = full_pipeline.process(image.clone(), &backend).unwrap();

    for halo in [32u32, 64, 96, 128, 192, 256] {
        let mut max = 0.0_f64;
        let mut rms = 0.0_f64;
        let mut count = 0usize;
        for ty in (0..full_h).step_by(tile_h as usize) {
            for tx in (0..full_w).step_by(tile_w as usize) {
                let x0 = tx as isize - halo as isize;
                let y0 = ty as isize - halo as isize;
                let tw = tile_w + 2 * halo;
                let th = tile_h + 2 * halo;
                let tile = crop_reflect(&image, x0, y0, tw, th);
                let pipeline = make_pipeline(tw, th);
                let out = pipeline.process(tile, &backend).unwrap();
                let center = crop_reflect(&out, halo as isize, halo as isize, tile_w, tile_h);
                let reference = crop_reflect(&full, tx as isize, ty as isize, tile_w, tile_h);
                let (m, r) = diff(&center, &reference);
                max = max.max(m);
                rms += r * r;
                count += 1;
            }
        }
        let rms = (rms / count as f64).sqrt();
        eprintln!("halo={halo:3} px  max={max:.6e}  rms={rms:.6e}");
    }
}

#[test]
#[ignore = "research probe; run explicitly"]
fn haloed_tiles_match_full_frame_by_tap() {
    let data = crate::test_support::data_dir();
    let film = profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
    let print = profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
    let backend = CpuBackend;
    let pitch_um = 3.8_f64;
    let full_w = 512u32;
    let full_h = 512u32;
    let tile_w = 128u32;
    let tile_h = 128u32;
    let halo = 256u32;
    let image = synthetic(full_w, full_h);
    let make_pipeline = |width: u32, height: u32| {
        let mut params = RuntimeParams::default();
        params.camera.auto_exposure = false;
        params.film_render.grain.active = false;
        params.print_render.glare.active = false;
        params.scanner.unsharp_mask = [0.0, 0.0];
        params.camera.film_format_mm = (pitch_um * width.max(height) as f64 / 1000.0) as f32;
        params.io.crop = false;
        params.io.upscale_factor = 1.0;
        Pipeline::new_with_spectral(film.clone(), print.clone(), params, &data).unwrap()
    };
    let full_pipeline = make_pipeline(full_w, full_h);
    let tiles = [(0u32, 0u32), (256, 0), (0, 256), (256, 256)];
    for tap in [Tap::LogEFilm, Tap::CmyFilm, Tap::CmyPrint, Tap::RgbOut] {
        let full = full_pipeline
            .process_with_taps(image.clone(), &backend, Some(Tap::RgbIn), Some(tap))
            .unwrap();
        let mut max = 0.0_f64;
        let mut rms = 0.0_f64;
        for &(tx, ty) in &tiles {
            let x0 = (tx as isize - halo as isize).clamp(0, full_w as isize - tile_w as isize);
            let y0 = (ty as isize - halo as isize).clamp(0, full_h as isize - tile_h as isize);
            let x1 = (tx as isize + tile_w as isize + halo as isize).min(full_w as isize);
            let y1 = (ty as isize + tile_h as isize + halo as isize).min(full_h as isize);
            let tw = (x1 - x0) as u32;
            let th = (y1 - y0) as u32;
            let tile = crop_clip(&image, x0, y0, tw, th);
            let pipeline = make_pipeline(tw, th);
            let out = pipeline
                .process_with_taps(tile, &backend, Some(Tap::RgbIn), Some(tap))
                .unwrap();
            let cx = tx as isize - x0;
            let cy = ty as isize - y0;
            let center = crop_clip(&out, cx, cy, tile_w, tile_h);
            let reference = crop_clip(&full, tx as isize, ty as isize, tile_w, tile_h);
            let (m, r) = diff(&center, &reference);
            max = max.max(m);
            rms = rms.max(r);
        }
        eprintln!("tap={tap:?} max={max:.6e} rms_max={rms:.6e}");
    }
}

#[test]
#[ignore = "research probe; run explicitly"]
fn logefilm_mismatch_source_probe() {
    let data = crate::test_support::data_dir();
    let film = profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
    let print = profile::load_profile_by_name(&data, "kodak_portra_endura").unwrap();
    let backend = CpuBackend;
    let pitch_um = 3.8_f64;
    let full_w = 512u32;
    let full_h = 512u32;
    let halo = 256u32;
    let tile_w = 128u32;
    let tile_h = 128u32;
    let image = synthetic(full_w, full_h);
    let make_pipeline = |width: u32, height: u32, halation: bool| {
        let mut params = RuntimeParams::default();
        params.camera.auto_exposure = false;
        params.film_render.grain.active = false;
        params.film_render.halation.active = halation;
        params.camera.film_format_mm = (pitch_um * width.max(height) as f64 / 1000.0) as f32;
        params.io.crop = false;
        params.io.upscale_factor = 1.0;
        Pipeline::new_with_spectral(film.clone(), print.clone(), params, &data).unwrap()
    };
    for halation in [false, true] {
        let full_pipeline = make_pipeline(full_w, full_h, halation);
        let full = full_pipeline
            .process_with_taps(
                image.clone(),
                &backend,
                Some(Tap::RgbIn),
                Some(Tap::LogEFilm),
            )
            .unwrap();
        let (_, fp) = full_pipeline
            .rescale_working(&image.clone(), &backend)
            .unwrap();
        eprintln!(
            "halation={halation} full_dim={}x{} pitch={fp}",
            full.width, full.height
        );
        for (tx, ty) in [(0u32, 0u32), (128, 128), (256, 256), (384, 384)] {
            let tw = tile_w + 2 * halo;
            let th = tile_h + 2 * halo;
            let tile = crop_reflect(
                &image,
                tx as isize - halo as isize,
                ty as isize - halo as isize,
                tw,
                th,
            );
            let tile_pipeline = make_pipeline(tw, th, halation);
            let (_, tp) = tile_pipeline
                .rescale_working(&tile.clone(), &backend)
                .unwrap();
            let out = tile_pipeline
                .process_with_taps(tile, &backend, Some(Tap::RgbIn), Some(Tap::LogEFilm))
                .unwrap();
            let center = crop_reflect(&out, halo as isize, halo as isize, tile_w, tile_h);
            let reference = crop_reflect(&full, tx as isize, ty as isize, tile_w, tile_h);
            let (m, r) = diff(&center, &reference);
            eprintln!("  tile=({tx},{ty}) tile={tw}x{th} pitch={tp} max={m:.6e} rms={r:.6e}");
        }
    }
}
