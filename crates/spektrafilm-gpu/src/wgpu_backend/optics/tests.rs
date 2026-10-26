//! Device optical-stage acceptance.
//!
//! Two stages must reproduce their CPU reference at the f32 device
//! boundary:
//!
//! * `optical_diffusion` — the discrete per-channel PSF convolution with
//!   reflect boundary extension and the scatter blend. The taps and the
//!   radius come from the reference resolution, so the shader is checked
//!   against `spektrafilm_model::diffusion::apply_diffusion_filter_um`
//!   end to end: a substituted Gaussian, a truncated kernel, or a different
//!   boundary extension all fail here.
//! * `enlarger_lut_interpolate` — the prepared 3-D PCHIP walk. The lattice,
//!   slopes, and cell hull are prepared by the reference, so the shader is
//!   checked against `spektrafilm_math::pchip3d::pchip_interp`.
//!
//! Both comparisons are f32-device versus f64-reference, so the budget is
//! the f32 rounding of the same arithmetic, not an algorithm tolerance.

use super::*;

#[test]
fn diffusion_reservation_tracks_support_instead_of_hardware_capacity() {
    let modest = super::bounded::workspace_plan(1, 1, 1, 128 << 20, false).unwrap();
    let large = super::bounded::workspace_plan(1, 1, 1, 1 << 30, false).unwrap();
    assert_eq!(large, modest);
    assert!(
        large < 1024,
        "one-pixel identity diffusion must not reserve gigabytes"
    );
    let wider = super::bounded::workspace_plan(64, 48, 9, 1 << 30, true).unwrap();
    assert!(wider > large);
    assert!(wider < 1 << 20);
}

/// A deterministic RGB image with a bright corner block, exercising the
/// PSF core and halo tails plus the reflected image boundaries.
fn hotspot_image(n: u32) -> ImageBuf {
    use spektrafilm_math::precision::from_f64;
    let mut data = Vec::with_capacity((n * n * 3) as usize);
    for y in 0..n {
        for x in 0..n {
            let v = (x.max(y) as f64 / (n as f64 - 1.0)).min(1.0);
            let hot = if x >= n - 12 && y < 12 { 1.0 } else { v * 0.3 };
            data.push(from_f64(hot));
            data.push(from_f64(hot * 0.9));
            data.push(from_f64(hot * 0.8));
        }
    }
    ImageBuf::from_data(n, n, data)
}

/// Worst absolute difference between two f64 views of the same image,
/// and where it happened.
fn max_error(got: &[f64], expect: &[f64]) -> (f64, usize) {
    let mut worst = 0.0f64;
    let mut at = 0usize;
    for (i, (&g, &e)) in got.iter().zip(expect.iter()).enumerate() {
        let diff = (g - e).abs();
        if diff > worst {
            worst = diff;
            at = i;
        }
    }
    (worst, at)
}

#[test]
fn device_diffusion_matches_the_cpu_reference() {
    let backend = crate::test_support::adapter();
    let img = hotspot_image(48);
    // Every family with its full reference-resolved support.
    for family in ["glimmerglass", "black_pro_mist", "pro_mist", "cinebloom"] {
        let filter = spektrafilm_model::diffusion::DiffusionFilter {
            family,
            strength: 0.5,
            spatial_scale: 0.05,
            halo_warmth: 0.0,
            core_intensity: 1.0,
            core_size: 1.0,
            halo_intensity: 1.0,
            halo_size: 1.0,
            bloom_intensity: 1.0,
            bloom_size: 1.0,
        };
        let pixel_size_um = 10.0;
        // The host resolves the exact taps the reference convolves.
        let (psf, radius, scatter) = spektrafilm_model::diffusion::resolve_psf(
            &filter,
            pixel_size_um,
            img.width as usize,
            img.height as usize,
        )
        .unwrap_or_else(|e| panic!("{family}: {e}"));
        let params = spektrafilm_backend::OpticalDiffusionGpuParams {
            psf: &psf,
            radius: radius as u32,
            scatter_fraction: scatter,
        };
        let got = backend
            .optical_diffusion_gpu(&img, &params)
            .expect("the device convolution covers this kernel");
        let expect =
            spektrafilm_model::diffusion::apply_diffusion_filter_um(&img, &filter, pixel_size_um)
                .unwrap_or_else(|e| panic!("{family}: {e}"));
        let expect_f64: Vec<f64> = expect.data.iter().map(|&v| v as f64).collect();
        let got_f64: Vec<f64> = got.data.iter().map(|&v| v as f64).collect();
        let (worst, at) = max_error(&got_f64, &expect_f64);
        // Device summation is binary64, with f32 upload/output boundaries.
        // The CPU FFT and direct sampled sum use the same resolved PSF and
        // reflect extension; this budget does not allow a substituted kernel.
        assert!(
            worst < 5e-5,
            "{family}: max error {worst:.3e} at sample {at} exceeds the f32 \
             convolution budget"
        );
    }
}

#[test]
fn device_diffusion_is_energy_conserving_on_a_uniform_field() {
    use spektrafilm_math::precision::from_f64;
    let backend = crate::test_support::adapter();
    let n = 48u32;
    let img = ImageBuf::from_data(n, n, vec![from_f64(0.37); (n * n * 3) as usize]);
    let filter = spektrafilm_model::diffusion::DiffusionFilter {
        family: "black_pro_mist",
        strength: 1.0,
        spatial_scale: 0.02,
        halo_warmth: 0.0,
        core_intensity: 1.0,
        core_size: 1.0,
        halo_intensity: 1.0,
        halo_size: 1.0,
        bloom_intensity: 1.0,
        bloom_size: 1.0,
    };
    let (psf, radius, scatter) =
        spektrafilm_model::diffusion::resolve_psf(&filter, 10.0, n as usize, n as usize)
            .expect("effective filter");
    let params = spektrafilm_backend::OpticalDiffusionGpuParams {
        psf: &psf,
        radius: radius as u32,
        scatter_fraction: scatter,
    };
    let got = backend
        .optical_diffusion_gpu(&img, &params)
        .expect("the device convolution covers this kernel");
    let worst = got
        .data
        .iter()
        .map(|v| ((*v as f64) - 0.37).abs())
        .fold(0.0f64, f64::max);
    // (1-p_s)·c + p_s·(K∗c) = c for a per-channel PSF summing to one. The
    // reference test uses 2e-3 to allow host truncation loss; the device
    // evaluates the same taps, so it must agree with the reference to f32.
    assert!(
        worst < 5e-5,
        "uniform field drifted by {worst:.3e} — the scatter blend is not energy conserving"
    );
}

/// Build a deterministic LUT-shaped lattice and prepare it with the
/// reference, then interpolate a probe set of coordinates on the device.
#[test]
fn device_enlarger_lut_matches_the_pchip_reference() {
    use spektrafilm_math::precision::{from_f64, to_f64};
    let backend = crate::test_support::adapter();
    let size = 9usize;
    // A smooth monotone-ish field per channel with a deliberate local
    // extremum, so the PCHIP limiter and the cell hull clamp both engage.
    let mut lut = vec![0.0f64; size * size * size * 3];
    for i in 0..size {
        for j in 0..size {
            for k in 0..size {
                let base = ((i * size + j) * size + k) * 3;
                let x = i as f64 / (size - 1) as f64;
                let y = j as f64 / (size - 1) as f64;
                let z = k as f64 / (size - 1) as f64;
                lut[base] = (x * 1.7 - 0.3 * y + 0.05 * z * z).clamp(-0.2, 1.2);
                lut[base + 1] = (0.4 * y + 0.6 * z - 0.1 * x * x).clamp(-0.2, 1.2);
                lut[base + 2] = (z * z - 0.25 * x * y).clamp(-0.2, 1.2);
            }
        }
    }
    let prepared = spektrafilm_math::pchip3d::prepare_pchip_3d(lut, size);
    // The enlarger LUT domain is [-density_min, max(density_curves)] per
    // channel; a representative narrow domain keeps the probe dense.
    let data_min = [-0.05, -0.04, -0.03];
    let data_max = [1.1, 0.9, 1.0];

    // Probe every lattice node plus interior fractions and out-of-domain
    // coordinates, so the clamping and the last-interval branch are covered.
    let mut coords: Vec<[f64; 3]> = Vec::new();
    let step = 1.0 / (size - 1) as f64;
    for i in 0..size {
        for j in 0..size {
            for k in 0..size {
                coords.push([i as f64 * step, j as f64 * step, k as f64 * step]);
            }
        }
    }
    for i in 0..size - 1 {
        for j in 0..size - 1 {
            coords.push([
                (i as f64 + 0.37) * step,
                (j as f64 + 0.71) * step,
                (i as f64 + 0.13) * step,
            ]);
        }
    }
    coords.push([-0.5, -0.5, -0.5]);
    coords.push([1.5, 1.5, 1.5]);

    let mut data = Vec::with_capacity(coords.len() * 3);
    for c in &coords {
        for ch in 0..3 {
            data.push(from_f64(
                data_min[ch] + (data_max[ch] - data_min[ch]) * c[ch],
            ));
        }
    }
    let img = ImageBuf::from_data(coords.len() as u32, 1, data);

    let params = spektrafilm_backend::EnlargerLutGpuParams {
        lut: &prepared.lut,
        slope_x: &prepared.slope_x,
        slope_y: &prepared.slope_y,
        slope_z: &prepared.slope_z,
        cell_min: &prepared.cell_min,
        cell_max: &prepared.cell_max,
        size: prepared.size as u32,
        data_min,
        data_scale: [
            (size - 1) as f64 / (data_max[0] - data_min[0]),
            (size - 1) as f64 / (data_max[1] - data_min[1]),
            (size - 1) as f64 / (data_max[2] - data_min[2]),
        ],
    };
    let got = backend
        .enlarger_lut_gpu(&img, &params)
        .expect("the device LUT covers this lattice");

    let mut worst = 0.0f64;
    let mut worst_at = [0.0f64; 3];
    for (p, coord) in coords.iter().enumerate() {
        let scale = (size - 1) as f64;
        // `coord` is the normalized domain fraction, so the axis coordinate
        // is the fraction scaled onto the lattice — exactly what the device
        // derives from the raw value it received.
        let axis = [coord[0] * scale, coord[1] * scale, coord[2] * scale];
        let expect = spektrafilm_math::pchip3d::pchip_interp(&prepared, axis[0], axis[1], axis[2]);
        for (ch, &expected) in expect.iter().enumerate() {
            let diff = (expected - to_f64(got.data[p * 3 + ch])).abs();
            if diff > worst {
                worst = diff;
                worst_at = *coord;
            }
        }
    }
    // The lattice and slopes are the reference's; the device performs the
    // same Hermite walk in f32. The budget is that f32 rounding, amplified
    // by the cubic basis and the slope magnitudes (order 1 here).
    assert!(
        worst < 1e-4,
        "max error {worst:.3e} at {worst_at:?} exceeds the f32 PCHIP budget"
    );
}

/// A bound smaller than the lattice forces device source-halo bricks. The
/// oracle prepares log-XYZ (not RGB), and uses noncommuting colour matrices.
#[test]
fn scanner_generated_lut_matches_log_xyz_reference_with_bounded_aliasing() {
    use spektrafilm_backend::{ComputeBackend, ScannerSpectralLutGpuParams};
    use spektrafilm_math::{
        pchip3d::{pchip_interp, prepare_pchip_3d},
        precision::from_f64,
    };
    let backend = crate::test_support::adapter();
    struct RestoreLimit(Option<std::ffi::OsString>);
    impl Drop for RestoreLimit {
        fn drop(&mut self) {
            // Device tests serialize through the adapter's process-wide lock.
            unsafe {
                match &self.0 {
                    Some(value) => std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", value),
                    None => std::env::remove_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"),
                }
            }
        }
    }
    let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
    unsafe {
        std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "6000");
    }
    let size = 7usize;
    let minimum = [-0.2, -0.1, 0.0];
    let maximum = [2.8, 2.2, 3.1];
    let channel_density: Vec<[f64; 3]> = (0..7)
        .map(|i| {
            let t = i as f64 / 6.0;
            [0.1 + 1.4 * t, 1.3 - 0.9 * t, 0.2 + 0.8 * t * t]
        })
        .collect();
    // A short base is wholly ignored by scan_log_xyz_cpu, not partially used.
    let base_density = [0.6];
    let illuminant = [0.7, 1.3, 0.5, 1.7, 0.9, 0.3, 1.1];
    let cat = [[1.1, 0.07, -0.03], [-0.02, 0.91, 0.08], [0.04, -0.06, 1.04]];
    let rgb = [[1.3, -0.2, 0.1], [0.2, 0.8, -0.1], [-0.1, 0.15, 0.95]];
    let params = ScannerSpectralLutGpuParams {
        channel_density: &channel_density,
        base_density: &base_density,
        illuminant: &illuminant,
        normalization: 0.13,
        cat: &cat,
        xyz_to_rgb: &rgb,
        minimum,
        maximum,
        size: size as u32,
    };
    let mut grid = ImageBuf::new(size as u32, (size * size) as u32);
    for x in 0..size {
        for y in 0..size {
            for z in 0..size {
                for (c, q) in [x, y, z].into_iter().enumerate() {
                    grid.data[((x * size + y) * size + z) * 3 + c] = from_f64(
                        minimum[c] + (maximum[c] - minimum[c]) * q as f64 / (size - 1) as f64,
                    );
                }
            }
        }
    }
    let prepared = prepare_pchip_3d(
        spektrafilm_backend::cpu_backend::scan_log_xyz_cpu(
            &grid,
            &channel_density,
            &base_density,
            &illuminant,
            params.normalization,
        ),
        size,
    );
    let probes = [
        [-0.7, -0.4, -0.3],
        minimum,
        maximum,
        [3.2, 2.7, 3.6],
        [0.51, 1.14, 1.33],
        [1.4, 0.7, 2.1],
        [2.29, 1.81, 0.83],
        [0.3, 0.2, 0.4],
        [1.8, 1.3, 2.6],
    ];
    let image = ImageBuf::from_data(3, 3, probes.into_iter().flatten().map(from_f64).collect());
    let got = backend
        .scanner_spectral_lut(&image, &params)
        .unwrap()
        .unwrap();
    assert_eq!((got.width, got.height), (3, 3));
    let input = super::super::scalars_to_f32(&image.data);
    let retained = backend
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scanner_lut_alias_regression"),
            contents: bytemuck::cast_slice(&input),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });
    backend
        .spectral_lut_transport(&retained, &retained, 9, |source, output, count| {
            backend.execute_scanner_lut_resident(source, output, count, &params)
        })
        .unwrap();
    let alias = backend.pipe.readback_buffer(&retained, 9 * 12);
    for (pixel, source) in image.data.as_chunks::<3>().0.iter().enumerate() {
        let q: [f64; 3] = std::array::from_fn(|c| {
            (source[c] as f64 - minimum[c]) * (size - 1) as f64 / (maximum[c] - minimum[c])
        });
        let log_xyz = pchip_interp(&prepared, q[0], q[1], q[2]);
        let xyz = log_xyz.map(|v| 10.0f64.powf(v));
        let adapted: [f64; 3] =
            std::array::from_fn(|c| cat[c][0] * xyz[0] + cat[c][1] * xyz[1] + cat[c][2] * xyz[2]);
        for c in 0..3 {
            let expected = rgb[c][0] * adapted[0] + rgb[c][1] * adapted[1] + rgb[c][2] * adapted[2];
            let actual = got.data[pixel * 3 + c] as f64;
            // Binary64 lattice, slopes, Hermite, and colour reconstruction
            // cross only the device's f32 image input/output boundaries.
            assert!(
                (actual - expected).abs() <= 3e-5 * expected.abs().max(1e-8),
                "scanner LUT pixel {pixel} channel {c}: {actual} != {expected}"
            );
            assert_eq!(
                alias[pixel * 3 + c],
                actual as f32,
                "aliasing changed the retained result"
            );
        }
    }
    unsafe {
        std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", "5183");
    }
    let error = backend.scanner_spectral_lut(&image, &params).unwrap_err();
    assert!(
        error.contains("minimum halo working set needs 5184 bytes"),
        "{error}"
    );
}

#[test]
fn device_diffusion_preserves_global_support_across_tiny_bindings_and_print_domains() {
    let backend = crate::test_support::adapter();
    struct RestoreLimit(Option<std::ffi::OsString>);
    impl Drop for RestoreLimit {
        fn drop(&mut self) {
            unsafe {
                match &self.0 {
                    Some(value) => std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", value),
                    None => std::env::remove_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"),
                }
            }
        }
    }
    let _restore = RestoreLimit(std::env::var_os("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES"));
    let radius = 5usize;
    let k = 2 * radius + 1;
    let psf: [Vec<f64>; 3] = std::array::from_fn(|c| {
        let mut taps: Vec<_> = (0..k * k)
            .map(|i| {
                let x = (i % k) as f64 - radius as f64;
                let y = (i / k) as f64 - radius as f64;
                (-(x * x + y * y) / (4.0 + c as f64 * 2.0)).exp()
            })
            .collect();
        let total: f64 = taps.iter().sum();
        for tap in &mut taps {
            *tap /= total;
        }
        taps
    });
    let params = spektrafilm_backend::OpticalDiffusionGpuParams {
        psf: &psf,
        radius: radius as u32,
        scatter_fraction: 0.43,
    };
    let image = hotspot_image(17);
    let samples: Vec<f32> = image
        .data
        .iter()
        .enumerate()
        .map(|(i, &v)| if i % 29 == 0 { -0.25 } else { v as f32 })
        .collect();
    let input = backend.pipe.static_buffer(
        "diffusion_test_source",
        bytemuck::cast_slice(&samples),
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let source = super::super::film_chain::device_image::BufferPayload::from_buffer(
        backend.backend_id(),
        input,
        17,
        17,
    );
    for scale in [None, Some(1.7)] {
        let linear: Vec<f64> = samples
            .iter()
            .map(|&v| match scale {
                Some(scale) => (10.0f64.powf(f64::from(v)) * scale) as f32 as f64,
                None => f64::from(v),
            })
            .collect();
        let channels: [Vec<f64>; 3] =
            std::array::from_fn(|c| linear.iter().skip(c).step_by(3).copied().collect());
        let blurred: [Vec<f64>; 3] = std::array::from_fn(|c| {
            spektrafilm_math::fft_conv::convolve2d_reflect(&channels[c], 17, 17, &psf[c], k)
        });
        let expected: Vec<f64> = (0..samples.len())
            .map(|i| {
                let mixed = ((1.0 - params.scatter_fraction) * linear[i]
                    + params.scatter_fraction * blurred[i % 3][i / 3])
                    as f32;
                if scale.is_some() {
                    (f64::from(mixed).max(0.0) + 1e-10).log10() as f32 as f64
                } else {
                    f64::from(mixed)
                }
            })
            .collect();
        for limit in [12000, 192] {
            unsafe {
                std::env::set_var("SPEKTRAFILM_GPU_BINDING_LIMIT_BYTES", limit.to_string());
            }
            let pages = backend.diffusion_pages(&source, &params, scale).unwrap();
            if limit == 192 {
                assert!(pages.len() > 1);
            }
            let retained = backend.retain_image_pages(
                pages,
                17,
                17,
                spektrafilm_backend::SampleDomain::Linear,
                &spektrafilm_math::colorspace::SRGB,
                None,
            );
            let got = backend.materialize_buffer(&retained).unwrap();
            let got: Vec<_> = got.data.iter().map(|&v| v as f64).collect();
            let (worst, at) = max_error(&got, &expected);
            assert!(
                worst < 5e-5,
                "limit={limit}, print_scale={scale:?}: max error {worst} at {at}"
            );
        }
    }
}
