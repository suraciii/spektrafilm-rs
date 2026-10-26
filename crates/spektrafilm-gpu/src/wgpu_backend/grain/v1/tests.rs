use super::*;
use spektrafilm_backend::cpu_backend::CpuBackend;
use spektrafilm_math::precision::{from_f64, to_f64};

#[test]
fn transport_scratch_does_not_scale_with_hardware_binding_maximum() {
    let cap = super::super::super::resources::WORKSPACE_BINDING_BYTES;
    assert_eq!(transport_window_limit(u64::MAX), cap);
    assert_eq!(transport_window_limit(cap * 8), cap);
    assert_eq!(transport_window_limit(4096), 4096);
}

#[test]
fn layered_fast_stats_matches_reference_across_workgroups() {
    let gpu = crate::test_support::adapter();
    assert!(
        gpu.device
            .features()
            .contains(wgpu::Features::SHADER_F64 | wgpu::Features::SHADER_INT64)
    );
    let (width, height) = (257, 3);
    let split = [0.25, 0.5, 0.25];
    let curve: Vec<[f64; 3]> = (0..4).map(|i| [i as f64; 3]).collect();
    let layers: Vec<[[f64; 3]; 3]> = curve
        .iter()
        .map(|row| std::array::from_fn(|sl| row.map(|v| v * split[sl])))
        .collect();
    let image = ImageBuf::from_data(
        width,
        height,
        (0..width * height)
            .flat_map(|i| [0.2, 0.8, 1.5].map(|v| from_f64(v + (i % 13) as f64 * 0.05)))
            .collect(),
    );
    let density_min = [0.03, 0.05, 0.07];
    let uniformity = [0.97, 0.99, 0.95];
    let pixel_size_um = 2.0;
    let particle_area = [0.1, 0.2, 0.3];
    let particle_scale_layers = [2.0, 1.0, 0.5];
    let maxima = std::array::from_fn(|sl| [3.0 * split[sl]; 3]);
    let mut params = GrainV1GpuParams {
        base_seed: 17,
        pixel_size_um,
        particle_area,
        particle_scale_layers,
        density_min,
        uniformity,
        density_min_layers: std::array::from_fn(|i| split[i / 3] * density_min[i % 3]),
        density_max_adj: std::array::from_fn(|i| split[i / 3] * (3.0 + density_min[i % 3])),
        n_particles: std::array::from_fn(|i| {
            pixel_size_um * pixel_size_um * split[i / 3]
                / (particle_area[i % 3] * particle_scale_layers[i / 3])
        }),
        micro_sigma: 0.0,
        monochrome: false,
        lookup_sign: 1.0,
        blur_dye_clouds: 0.0,
        reference_f64: std::mem::size_of::<spektrafilm_math::precision::Scalar>() == 8,
        micro_blur_px: 0.0,
        grain_blur: 0.0,
        usm_sigma: 0.0,
        usm_amount: 0.0,
        curve_x: curve.iter().flatten().copied().collect(),
        curve_layers: layers.iter().flatten().flatten().copied().collect(),
        sublayers_active: true,
        n_sub_layers: 3,
        mt19937_regime: false,
        density_max_composite: [3.0; 3],
        n_particles_composite: [0.0; 3],
    };
    for (seed, monochrome, sign, cloud, micro, micro_blur, grain_blur, usm_sigma, usm_amount) in [
        (17, false, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        (0, true, 1.0, 2.0, 0.2, 0.8, 0.8, 1.2, 1.5),
        (29, false, -1.0, 20.0, 0.3, 3.0, 3.0, 3.0, 1.5),
    ] {
        params.base_seed = seed;
        params.monochrome = monochrome;
        params.lookup_sign = sign;
        params.blur_dye_clouds = cloud;
        let micro_controls = [
            (micro_blur * pixel_size_um) as f32,
            (micro * pixel_size_um / 0.001) as f32,
        ];
        params.micro_sigma = micro_controls[1] as f64 * 0.001 / pixel_size_um;
        params.micro_blur_px = micro_controls[0] as f64 / pixel_size_um;
        params.grain_blur = grain_blur;
        params.usm_sigma = usm_sigma;
        params.usm_amount = usm_amount;
        let mut case_curve = curve.clone();
        let mut case_layers = layers.clone();
        if sign < 0.0 {
            case_curve.reverse();
            case_layers.reverse();
        }
        params.curve_x = case_curve
            .iter()
            .flat_map(|row| row.map(|v| v * f64::from(sign)))
            .collect();
        params.curve_layers = case_layers.iter().flatten().flatten().copied().collect();
        let planes = spektrafilm_model::density_curves::interp_density_cmy_layers(
            &image,
            &case_curve,
            &case_layers,
            sign < 0.0,
        );
        let reference = spektrafilm_model::grain::v1::apply_grain_to_density_layers(
            &planes,
            &maxima,
            width,
            height,
            pixel_size_um,
            particle_area,
            particle_scale_layers,
            density_min,
            uniformity,
            grain_blur,
            cloud,
            micro_controls,
            usm_sigma,
            usm_amount,
            monochrome,
            true,
            params.base_seed,
            &CpuBackend,
        )
        .unwrap();
        let bytes = image.data.len() as u64 * 4;
        let input: Vec<f32> = image.data.iter().map(|&v| to_f64(v) as f32).collect();
        let density = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("spektra_reference_density"),
                contents: bytemuck::cast_slice(&input),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            });
        let state = build_spektra_grain_state(&gpu.device, width, height, &density, &params, &gpu)
            .expect("native f64/i64 grain support");
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spektra_reference_readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        state.submit(&gpu).unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&density, 0, &readback, 0, bytes);
        gpu.queue.submit(Some(encoder.finish()));
        let actual = gpu.read_mapped_f32(&readback, bytes);
        let pages: Vec<_> = input
            .chunks(111)
            .enumerate()
            .map(|(index, samples)| super::super::super::paged::ImagePage {
                first_pixel: index as u32 * 37,
                n_pixels: samples.len() as u32 / 3,
                buffer: gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("grain_physical_seam"),
                        contents: bytemuck::cast_slice(samples),
                        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                    }),
            })
            .collect();
        // Each replay starts from immutable original samples. Chunk changes
        // must preserve global RNG identity, not reseed each transport window.
        for limit in [4096, 540] {
            for page in &pages {
                let first = page.first_pixel as usize * 3;
                gpu.queue.write_buffer(
                    &page.buffer,
                    0,
                    bytemuck::cast_slice(&input[first..first + page.n_pixels as usize * 3]),
                );
            }
            let bounded = build_spektra_grain_state_with_limit(
                &gpu.device,
                width,
                height,
                &pages,
                &params,
                &gpu,
                limit,
            )
            .expect("bounded global-index grain");
            bounded.submit(&gpu).unwrap();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let mut copies = RasterCommands::default();
            copies.copy(&pages, 0, &readback, 0, bytes);
            copies.encode(&mut encoder);
            gpu.queue.submit(Some(encoder.finish()));
            assert_eq!(
                actual,
                gpu.read_mapped_f32(&readback, bytes),
                "storage window {limit} changed global RNG identity"
            );
        }
        for (i, (&actual, &expected)) in actual.iter().zip(&reference.data).enumerate() {
            let expected = to_f64(expected);
            assert!(
                actual.is_finite() && (actual as f64 - expected).abs() < 2e-5,
                "seed {seed}, sign {sign}, sample {i}: device {actual} != reference {expected}"
            );
        }
    }
}
