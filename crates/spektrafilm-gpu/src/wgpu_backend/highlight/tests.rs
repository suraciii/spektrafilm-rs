use super::super::{
    glare::build_glare_state, halation::build_halation_state, unsharp::build_unsharp_state,
};
use super::*;

struct RestoreLimit(Option<std::ffi::OsString>);
impl Drop for RestoreLimit {
    fn drop(&mut self) {
        // Device tests hold the adapter's process-wide lock.
        unsafe {
            match &self.0 {
                Some(value) => std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", value),
                None => std::env::remove_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"),
            }
        }
    }
}

fn image(backend: &WgpuBackend, values: &[f32]) -> Vec<ImagePage> {
    let pages = backend
        .allocate_image_pages((values.len() / 3) as u32, 1, "bounded_optical_regression")
        .unwrap();
    for page in &pages {
        let first = page.first_pixel as usize * 3;
        let end = first + page.n_pixels as usize * 3;
        backend
            .queue
            .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&values[first..end]));
    }
    pages
}

fn readback(backend: &WgpuBackend, pages: &[ImagePage]) -> Vec<f32> {
    let mut values = Vec::new();
    for page in pages {
        values.extend(
            backend
                .pipe
                .readback_buffer(&page.buffer, u64::from(page.n_pixels) * 12)
                .unwrap(),
        );
    }
    values
}

#[test]
fn highlight_uses_the_last_chunk_maximum_for_every_chunk() {
    let backend = crate::test_support::adapter();
    let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
    unsafe {
        std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "192");
    }
    let mut values = vec![0.8f32; 17 * 9 * 3];
    *values.last_mut().unwrap() = 12.0;
    let img = image(&backend, &values);
    assert!(
        img.len() > 1,
        "artificial cap must physically page the source"
    );
    let hp = spektrafilm_backend::HighlightBoostGpuParams {
        boost_ev: 1.0,
        boost_range: 0.8,
        protect_ev: 0.0,
    };
    let state = build_highlight_boost_state(&backend.device, &hp, 17 * 9, &img, &backend).unwrap();
    state.submit(&backend).unwrap();
    let got = readback(&backend, &img);
    let a = 28.0f64.powf(1.0 - f64::from(hp.boost_range));
    let raw_x0 = 0.184;
    let span = 1.0 - raw_x0 / 12.0;
    let scale =
        12.0 * (2.0f64.powf(f64::from(hp.boost_ev)) - 1.0) / ((a * span).exp() - a * span - 1.0);
    for (i, (&input, &actual)) in values.iter().zip(&got).enumerate() {
        let dx = (f64::from(input) - raw_x0) / 12.0;
        let expected = f64::from(input) + scale * ((a * dx).exp() - a * dx - 1.0);
        assert!(
            (f64::from(actual) - expected).abs() < 3e-5,
            "sample {i}: {actual} != {expected}"
        );
    }
    assert_eq!(
        got[0], got[48],
        "chunk boundaries must not choose local maxima"
    );
}

#[test]
fn optical_chunks_preserve_glare_rng_and_reference_blur_across_seams() {
    let backend = crate::test_support::adapter();
    let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
    let width = 37;
    let height = 19;
    let mut values: Vec<f32> = (0..width * height * 3)
        .map(|i| 0.03 + (i % 29) as f32 * 0.01)
        .collect();
    // Impulses straddle the 16-pixel optical and 211-pixel glare binding
    // boundaries. Channel widths exercise FIR and IIR without averaging sigmas.
    values[15 * 3] = 8.0;
    values[16 * 3 + 1] = 5.0;
    values[210 * 3] = 8.0;
    values[211 * 3 + 1] = 5.0;
    values[(width * height * 3 - 1) as usize] = 11.0;
    let hp = spektrafilm_backend::HalationGpuParams {
        scatter_amount: 0.35,
        scatter_core_px: [0.8, 2.4, 4.0],
        scatter_tail_px: [1.1, 3.2, 5.0],
        scatter_tail_weight: [0.1, 0.2, 0.3],
        source_compression: 0.7,
        halation_a_tot: [0.2, 0.1, 0.04],
        halation_first_sigma_px: [1.0, 2.5, 4.0],
        halation_n_bounces: 2,
        halation_bounce_decay: 0.6,
        halation_renormalize: true,
    };
    let run = |cap: &str, stage: u32| {
        unsafe {
            std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", cap);
        }
        let img = image(&backend, &values);
        let mut gaussian_scratch = super::super::blur::GaussianScratch::new(width, height);
        match stage {
            0 => {
                let state =
                    build_halation_state(&hp, width, height, &img, &mut gaussian_scratch, &backend)
                        .unwrap();
                state.submit(&backend).unwrap();
            }
            1 | 2 => {
                let gp = spektrafilm_backend::GlareGpuParams {
                    mu: -2.0,
                    sigma: 0.6,
                    blur_px: if stage == 1 { 0.0 } else { 3.5 },
                    base_seed: 0x12ab34cd,
                    rgb_offset: [0.7, 0.9, 1.1],
                };
                let state = build_glare_state(
                    &backend.device,
                    &gp,
                    width,
                    height,
                    &img,
                    &mut gaussian_scratch,
                    &backend,
                )
                .unwrap();
                state.submit(&backend).unwrap();
            }
            _ => {
                let up = spektrafilm_backend::UnsharpGpuParams {
                    sigma_px: 2.5,
                    amount: 0.8,
                };
                let state = build_unsharp_state(
                    &backend.device,
                    &up,
                    width,
                    height,
                    &img,
                    &mut gaussian_scratch,
                    &backend,
                )
                .unwrap();
                state.submit(&backend).unwrap();
                return readback(&backend, state.destination());
            }
        }
        readback(&backend, &img)
    };
    for stage in 0..4 {
        let full = run("65536", stage);
        // The sequential glare generator needs its 2516-byte MT slot.
        let bounded = run(
            if stage == 1 || stage == 2 {
                "2532"
            } else {
                "192"
            },
            stage,
        );
        for (i, (&expected, &actual)) in full.iter().zip(&bounded).enumerate() {
            assert!(
                actual.is_finite() && (actual - expected).abs() < 5e-5,
                "stage {stage}, sample {i}: bounded {actual} != full {expected}"
            );
            if stage == 1 {
                assert_eq!(actual, expected, "global RNG index changed at sample {i}");
            }
        }
    }

    // Retain all stage states at once, as chain setup does. Sequential reuse
    // must be numerically identical to independent working sets, including
    // scatter tails, compressed-source bounces, seeded glare, and USM combine.
    let run_sequence = |reuse: bool| {
        unsafe {
            std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "2532");
        }
        let img = image(&backend, &values);
        let mut shared = super::super::blur::GaussianScratch::new(width, height);
        let mut glare_scratch = super::super::blur::GaussianScratch::new(width, height);
        let mut unsharp_scratch = super::super::blur::GaussianScratch::new(width, height);
        let halation =
            build_halation_state(&hp, width, height, &img, &mut shared, &backend).unwrap();
        let gp = spektrafilm_backend::GlareGpuParams {
            mu: -2.0,
            sigma: 0.6,
            blur_px: 3.5,
            base_seed: 0x12ab34cd,
            rgb_offset: [0.7, 0.9, 1.1],
        };
        let glare = build_glare_state(
            &backend.device,
            &gp,
            width,
            height,
            &img,
            if reuse {
                &mut shared
            } else {
                &mut glare_scratch
            },
            &backend,
        )
        .unwrap();
        let up = spektrafilm_backend::UnsharpGpuParams {
            sigma_px: 2.5,
            amount: 0.8,
        };
        let unsharp = build_unsharp_state(
            &backend.device,
            &up,
            width,
            height,
            &img,
            if reuse {
                &mut shared
            } else {
                &mut unsharp_scratch
            },
            &backend,
        )
        .unwrap();
        // Jobs, not the setup helper, must retain all referenced GPU buffers.
        drop(shared);
        drop(glare_scratch);
        drop(unsharp_scratch);
        let unsharp_destination = unsharp.destination().to_vec();
        halation.submit(&backend).unwrap();
        glare.submit(&backend).unwrap();
        unsharp.submit(&backend).unwrap();
        readback(&backend, &unsharp_destination)
    };
    assert_eq!(run_sequence(true), run_sequence(false));
}

#[test]
fn unsharp_combine_chunks_at_binding_capacity_not_alias_cap() {
    use spektrafilm_backend::telemetry::{CollectionMode, ObservationContext};

    let backend = crate::test_support::adapter();
    let mut backend = backend.derived_explicit();
    let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
    // A 4 MiB binding cap holds 349,525 items, five times the 65,536-item
    // alias-scratch cap the in-place combine used to chunk at.
    const LIMIT: u64 = 4 << 20;
    unsafe {
        std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", LIMIT.to_string());
    }
    let width = 2048;
    let height = 2000;
    let pixels = u64::from(width) * u64::from(height);
    let img = backend
        .allocate_image_pages(width, height, "unsharp_no_alias_test")
        .unwrap();
    for page in &img {
        let first = page.first_pixel as usize * 3;
        let count = page.n_pixels as usize * 3;
        let values: Vec<f32> = (0..count)
            .map(|i| 0.05 + ((first + i) % 23) as f32 * 0.01)
            .collect();
        backend
            .queue
            .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&values));
    }
    let up = spektrafilm_backend::UnsharpGpuParams {
        sigma_px: 2.5,
        amount: 0.8,
    };
    let mut scratch = super::super::blur::GaussianScratch::new(width, height);
    let state = build_unsharp_state(
        &backend.device,
        &up,
        width,
        height,
        &img,
        &mut scratch,
        &backend,
    )
    .unwrap();
    let context = ObservationContext::new(CollectionMode::Summary);
    backend.context_mut(context.clone());
    state.submit(&backend).unwrap();
    let submits = context.snapshot().totals.submit_count.unwrap();
    let alias_chunks = pixels.div_ceil(65_536);
    // Distinct source/blur/destination bindings raise the combine chunk size to
    // the binding cap; the blur adds a few bounded batches. The in-place layout
    // needed one submission per alias chunk, far above this.
    assert!(
        submits < alias_chunks,
        "unsharp still chunks at the alias cap: {submits} >= {alias_chunks} submissions"
    );
}
