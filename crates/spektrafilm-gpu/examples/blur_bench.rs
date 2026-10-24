// Measure GPU blur overhead: 1 blur vs N blurs (individually vs fused).
//
// Run with:
//   cargo run --release -p spektrafilm-gpu --example blur_bench

use spektrafilm_backend::ComputeBackend;
use spektrafilm_gpu::wgpu_backend::WgpuBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::from_f32;
use std::time::Instant;

fn make_image(w: u32, h: u32) -> ImageBuf {
    let n = (w as usize) * (h as usize) * 3;
    let data: Vec<_> = (0..n).map(|i| from_f32((i as f32) * 0.001 % 1.0)).collect();
    ImageBuf::from_data(w, h, data)
}

fn time<F: FnOnce()>(label: &str, f: F) {
    let t = Instant::now();
    f();
    let dt = t.elapsed();
    println!("{label:30} {:>8.2} ms", dt.as_secs_f64() * 1000.0);
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();
    let backend = WgpuBackend::new().expect("wgpu");
    let img = make_image(3000, 2000); // 6 MP
    // Three sigmas keeps the combined multi-readback under wgpu's 256 MB
    // single-buffer cap (3 × 72 MB = 216 MB).
    let sigmas = [3.0_f32, 6.0, 12.0];

    // Warm up shader cache + driver.
    backend
        .gaussian_blur(&img, 3.0)
        .expect("single-blur warm-up failed");
    backend
        .gaussian_blur_multi(&img, &sigmas)
        .expect("multi-blur warm-up failed");

    println!("--- 6 MP, sigmas = {:?} ---", sigmas);

    for sigma in sigmas {
        time(&format!("single blur sigma={sigma:.0}"), || {
            backend
                .gaussian_blur(&img, sigma)
                .expect("single-blur benchmark failed");
        });
    }

    time("3 separate blurs (loop)", || {
        for &s in &sigmas {
            backend
                .gaussian_blur(&img, s)
                .expect("separate-blur benchmark failed");
        }
    });

    time("3 fused blurs (multi)", || {
        backend
            .gaussian_blur_multi(&img, &sigmas)
            .expect("fused multi-blur benchmark failed");
    });
}
