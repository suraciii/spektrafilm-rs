use super::*;

mod logic_tests {
    use super::*;

    #[test]
    fn transport_scratch_does_not_scale_with_hardware_binding_maximum() {
        let cap = crate::wgpu_backend::resources::WORKSPACE_BINDING_BYTES;
        assert_eq!(transport_window_limit(u64::MAX), cap);
        assert_eq!(transport_window_limit(cap * 8), cap);
        assert_eq!(transport_window_limit(4096), 4096);
    }
}

#[test]
fn pcg_is_deterministic_across_storage_windows() {
    let gpu = crate::test_support::adapter();
    assert!(
        gpu.device
            .features()
            .contains(wgpu::Features::SHADER_F64 | wgpu::Features::SHADER_INT64)
    );
    let (width, height) = (33, 4);
    let curve: Vec<[f64; 3]> = [[0.0; 3], [0.3; 3], [1.0; 3]].to_vec();
    let layers: Vec<[[f64; 3]; 3]> = curve.iter().map(|row| [*row, [0.0; 3], [0.0; 3]]).collect();
    let prepared = spektrafilm_model::grain::spektra::prepare_grain(
        &layers,
        1,
        2.0,
        [6.0, 8.0, 10.0],
        [2.0, 1.0, 0.5],
        [0.03, 0.05, 0.07],
        [0.97, 0.99, 0.95],
    )
    .unwrap();
    let params = SpektraGrainGpuParams {
        base_seed: 42,
        layer_count: 1,
        density_min: prepared.density_min,
        uniformity: prepared.uniformity,
        density_min_layers: std::array::from_fn(|i| prepared.density_min_layers[i / 3][i % 3]),
        density_max_adj: std::array::from_fn(|i| prepared.density_max_adj[i / 3][i % 3]),
        n_particles: std::array::from_fn(|i| prepared.n_particles[i / 3][i % 3]),
        monochrome: false,
        lookup_sign: 1.0,
        blur_dye_clouds: 0.0,
        reference_f64: std::mem::size_of::<spektrafilm_math::precision::Scalar>() == 8,
        micro_sigma: 0.2,
        micro_blur_px: 1.0,
        grain_blur: 0.0,
        usm_sigma: 0.0,
        usm_amount: 0.0,
        curve_x: curve.iter().flatten().copied().collect(),
        curve_layers: layers.iter().flatten().flatten().copied().collect(),
    };
    let input: Vec<f32> = (0..width * height * 3)
        .map(|i| 0.4 + 0.1 * (i % 5) as f32)
        .collect();
    let bytes = input.len() as u64 * 4;
    let density = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("spektra_pcg_density"),
            contents: bytemuck::cast_slice(&input),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });
    let state = build_spektra_grain_state(&gpu.device, width, height, &density, &params, &gpu)
        .expect("statistical PCG state");
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("spektra_pcg_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    state.submit(&gpu).unwrap();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&density, 0, &readback, 0, bytes);
    gpu.queue.submit(Some(encoder.finish()));
    let actual = gpu.read_mapped_f32(&readback, bytes).unwrap();
    assert!(actual.iter().all(|value| value.is_finite()));
    assert!(
        actual
            .iter()
            .zip(&input)
            .any(|(actual, input)| (actual - input).abs() > 1e-6),
        "the PCG path must actually sample"
    );

    gpu.queue
        .write_buffer(&density, 0, bytemuck::cast_slice(&input));
    // 1024 bytes yields 42-pixel chunks for this 132-pixel image, so the
    // bounded build spans four storage windows rather than repeating one.
    let bounded = build_spektra_grain_state_with_limit(
        &gpu.device,
        width,
        height,
        &density,
        &params,
        &gpu,
        1024,
    )
    .expect("bounded statistical PCG state");
    bounded.submit(&gpu).unwrap();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&density, 0, &readback, 0, bytes);
    gpu.queue.submit(Some(encoder.finish()));
    assert_eq!(
        actual,
        gpu.read_mapped_f32(&readback, bytes).unwrap(),
        "storage windows changed the per-pixel PCG realization"
    );

    gpu.queue
        .write_buffer(&density, 0, bytemuck::cast_slice(&input));
    let mut high_seed = params.clone();
    high_seed.base_seed += 1u64 << 32;
    let high = build_spektra_grain_state(&gpu.device, width, height, &density, &high_seed, &gpu)
        .expect("high seed statistical PCG state");
    high.submit(&gpu).unwrap();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&density, 0, &readback, 0, bytes);
    gpu.queue.submit(Some(encoder.finish()));
    assert_ne!(
        actual,
        gpu.read_mapped_f32(&readback, bytes).unwrap(),
        "the PCG stream must consume the full request seed"
    );
}

#[test]
fn spektra_reports_the_missing_f64_and_int64_capability() {
    let gpu = crate::test_support::restricted_adapter();
    let params = SpektraGrainGpuParams {
        base_seed: 0,
        layer_count: 1,
        density_min: [0.0; 3],
        uniformity: [0.0; 3],
        density_min_layers: [0.0; 9],
        density_max_adj: [0.0; 9],
        n_particles: [0.0; 9],
        monochrome: false,
        lookup_sign: 1.0,
        blur_dye_clouds: 0.0,
        reference_f64: true,
        micro_sigma: 0.0,
        micro_blur_px: 0.0,
        grain_blur: 0.0,
        usm_sigma: 0.0,
        usm_amount: 0.0,
        curve_x: Vec::new(),
        curve_layers: Vec::new(),
    };
    let density = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("restricted_spektra_density"),
            contents: bytemuck::cast_slice(&[0.0f32; 3]),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
    let error = match build_spektra_grain_state(&gpu.device, 1, 1, &density, &params, &gpu) {
        Ok(_) => panic!("restricted device must not build the Spektra grain state"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        "Spektra grain requires device binary64 arithmetic and 64-bit integers"
    );
}
