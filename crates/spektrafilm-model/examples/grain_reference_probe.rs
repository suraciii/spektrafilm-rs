use spektrafilm_gpu::{GrainV2GpuParams, wgpu_backend::WgpuBackend};
use spektrafilm_math::{
    image::ImageBuf,
    precision::{from_f32, to_f32},
};
use spektrafilm_model::grain::v2::{GrainV2Mode, GrainV2Params, apply_cpu};
use std::{env, error::Error, fs};
fn main() -> Result<(), Box<dyn Error>> {
    // Input and both outputs are native encoded RGB; the input carrier is
    // interleaved RGBA little-endian f32 and alpha is ignored.
    let a: Vec<String> = env::args().collect();
    if a.len() != 8 {
        return Err("usage: grain_reference_probe <encoded-rgba-f32le> <width> <height> <output-prefix> <mode> <profile> <seed>".into());
    }
    let w: u32 = a[2].parse()?;
    let h: u32 = a[3].parse()?;
    let raw = fs::read(&a[1])?;
    if raw.len() != w as usize * h as usize * 16 {
        return Err("input byte count mismatch".into());
    }
    let data: Vec<_> = raw
        .chunks_exact(16)
        .flat_map(|p| {
            (0..3)
                .map(move |c| from_f32(f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap())))
        })
        .collect();
    let input = ImageBuf::from_data(w, h, data);
    let mut p = GrainV2Params::for_profile(a[6].parse()?);
    p.mode = if a[5] == "noise" {
        GrainV2Mode::Noise
    } else {
        GrainV2Mode::Analogue
    };
    p.seed = a[7].parse()?;
    p.resolution_factor = 100.;
    let cpu = apply_cpu(&input, p);
    let save = |path: String, img: &ImageBuf| -> Result<(), Box<dyn Error>> {
        let bytes: Vec<u8> = img
            .data
            .iter()
            .flat_map(|&x| to_f32(x).to_le_bytes())
            .collect();
        fs::write(path, bytes)?;
        Ok(())
    };
    save(format!("{}_cpu_rgb_f32le.bin", a[4]), &cpu)?;
    let gpu = WgpuBackend::new().ok_or("no WGPU adapter")?;
    let gp = GrainV2GpuParams {
        mode: p.mode as u32,
        film_type: p.film_type,
        amount: p.amount,
        shadows: p.shadows,
        midtones: p.midtones,
        highlights: p.highlights,
        raw_scale: p.size,
        cluster_size: p.cluster_size,
        rotation: p.rotation,
        color: p.color,
        resolution_factor: p.resolution_factor,
        seed: p.seed,
        colored: p.colored,
        clustered: p.clustered,
    };
    let out = gpu.grain_v2_gpu(&input, &gp);
    save(format!("{}_gpu_rgb_f32le.bin", a[4]), &out)?;
    let max = cpu
        .data
        .iter()
        .zip(&out.data)
        .map(|(&a, &b)| (to_f32(a) - to_f32(b)).abs())
        .fold(0f32, f32::max);
    println!("CPU/WGPU max_abs={max}");
    Ok(())
}
