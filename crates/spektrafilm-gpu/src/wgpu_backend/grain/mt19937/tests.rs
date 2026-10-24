//! Device regression tests for the sequential MT19937 regime.
//!
//! Every case compares the device against the CPU reference
//! `spektrafilm_model::grain::v1::apply_grain_to_density` /
//! `apply_grain_to_density_layers`, which owns the NumPy distribution
//! algorithms. The comparison is exact in the discrete decisions — the same
//! Poisson seeds and the same binomial counts at the same elements — and
//! bounded in the floating field.
//!
//! These tests have not been executed in this environment; they are the
//! deterministic regression cases the owning stage runs.

use super::*;
use spektrafilm_backend::cpu_backend::CpuBackend;
use spektrafilm_math::precision::{from_f64, to_f64};

/// Read a buffer back as f32 values.
fn readback(gpu: &crate::test_support::DeviceBackend, buffer: &wgpu::Buffer, bytes: u64) -> Vec<f32> {
    let target = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mt19937_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &target, 0, bytes);
    gpu.queue.submit(Some(encoder.finish()));
    gpu.read_mapped_f32(&target, bytes)
}

/// Allocate the `[sl][pixel][ch]` f32 output the sampler writes.
fn layer_fields_buffer(
    gpu: &crate::test_support::DeviceBackend,
    width: u32,
    height: u32,
    planes: u32,
) -> (wgpu::Buffer, u64) {
    let bytes = u64::from(width) * u64::from(height) * 3 * 4 * u64::from(planes);
    (
        gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mt19937_layer_fields"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        bytes,
    )
}

/// Upload an `ImageBuf` as the film density CMY buffer.
fn upload_density(
    gpu: &crate::test_support::DeviceBackend,
    image: &spektrafilm_math::image::ImageBuf,
) -> wgpu::Buffer {
    let input: Vec<f32> = image.data.iter().map(|&v| to_f64(v) as f32).collect();
    gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("mt19937_density"),
        contents: bytemuck::cast_slice(&input),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
    })
}

/// Run the sampler's whole two-phase walk and read the layer fields.
fn run_sampler(
    gpu: &crate::test_support::DeviceBackend,
    state: &SpektraMt19937State,
    layer_fields: &wgpu::Buffer,
    layer_bytes: u64,
) -> Vec<f32> {
    state.submit(gpu).unwrap();
    readback(gpu, layer_fields, layer_bytes)
}

/// Sum the device's `[sl][pixel][ch]` fields into one composite CMY image,
/// which is what the composition pass produces. `scale` is the reference's
/// per-path accumulation factor: the composite path scales by
/// `1 / n_sub_layers`, the layered path sums without a scale. The reference's
/// channel total is `sum * scale - density_min` (`apply_grain_to_density`
/// subtracts the floor it added to the input), so the floor is removed here.
fn composed(
    fields: &[f32],
    width: u32,
    height: u32,
    layers: u32,
    scale: f32,
    density_min: [f64; 3],
) -> Vec<[f32; 3]> {
    let n = (width * height) as usize;
    (0..n)
        .map(|i| {
            let mut sum = [0.0f32; 3];
            for sl in 0..layers as usize {
                for ch in 0..3 {
                    sum[ch] += fields[(sl * n + i) * 3 + ch];
                }
            }
            [
                sum[0] * scale - density_min[0] as f32,
                sum[1] * scale - density_min[1] as f32,
                sum[2] * scale - density_min[2] as f32,
            ]
        })
        .collect()
}

/// The composite-path control set used by the device cases.
fn composite_params(base_seed: u64, monochrome: bool, n_sub_layers: u32) -> spektrafilm_backend::GrainV1GpuParams {
    // A simple monotone composite curve with three knots per channel, and
    // layer curves that split it. The layered tables are still required by
    // the sampler's uniform layout even though the composite path does not
    // read them, so they carry the same split.
    let curve: Vec<[f64; 3]> = (0..3).map(|i| [i as f64; 3]).collect();
    let layers: Vec<[[f64; 3]; 3]> = curve
        .iter()
        .map(|row| {
            [
                [row[0] * 0.2, row[1] * 0.2, row[2] * 0.2],
                [row[0] * 0.3, row[1] * 0.3, row[2] * 0.3],
                [row[0] * 0.5, row[1] * 0.5, row[2] * 0.5],
            ]
        })
        .collect();
    let pixel_size_um = 12.0;
    let particle_area = [0.2; 3];
    let pixel_area = pixel_size_um * pixel_size_um;
    spektrafilm_backend::GrainV1GpuParams {
        base_seed,
        pixel_size_um,
        particle_area,
        particle_scale_layers: [1.0; 3],
        density_min: [0.03; 3],
        uniformity: [0.97; 3],
        density_min_layers: [0.0; 9],
        density_max_adj: [0.0; 9],
        n_particles: [0.0; 9],
        monochrome,
        lookup_sign: 1.0,
        blur_dye_clouds: 0.0,
        reference_f64: std::mem::size_of::<spektrafilm_math::precision::Scalar>() == 8,
        micro_sigma: 0.0,
        micro_blur_px: 0.0,
        grain_blur: 0.0,
        usm_sigma: 0.0,
        usm_amount: 0.0,
        curve_x: curve.iter().flatten().copied().collect(),
        curve_layers: layers.iter().flatten().flatten().copied().collect(),
        sublayers_active: false,
        n_sub_layers,
        mt19937_regime: true,
        density_max_composite: [2.2; 3],
        n_particles_composite: std::array::from_fn(|_| {
            let count = pixel_area / (0.2 * 1.0);
            if n_sub_layers > 1 { count / f64::from(n_sub_layers) } else { count }
        }),
    }
}

/// The layered-path control set used by the device cases.
fn layered_params(base_seed: u64, monochrome: bool) -> spektrafilm_backend::GrainV1GpuParams {
    let split = [0.25, 0.5, 0.25];
    let curve: Vec<[f64; 3]> = (0..4).map(|i| [i as f64; 3]).collect();
    let layers: Vec<[[f64; 3]; 3]> = curve
        .iter()
        .map(|row| std::array::from_fn(|sl| row.map(|v| v * split[sl])))
        .collect();
    let pixel_size_um = 2.0;
    let particle_area = [0.1, 0.2, 0.3];
    let particle_scale_layers = [2.0, 1.0, 0.5];
    let density_min = [0.03, 0.05, 0.07];
    let uniformity = [0.97, 0.99, 0.95];
    let pixel_area = pixel_size_um * pixel_size_um;
    spektrafilm_backend::GrainV1GpuParams {
        base_seed,
        pixel_size_um,
        particle_area,
        particle_scale_layers,
        density_min,
        uniformity,
        density_min_layers: std::array::from_fn(|i| split[i / 3] * density_min[i % 3]),
        density_max_adj: std::array::from_fn(|i| split[i / 3] * (3.0 + density_min[i % 3])),
        n_particles: std::array::from_fn(|i| {
            pixel_area * split[i / 3] / (particle_area[i % 3] * particle_scale_layers[i / 3])
        }),
        monochrome,
        lookup_sign: 1.0,
        blur_dye_clouds: 0.0,
        reference_f64: std::mem::size_of::<spektrafilm_math::precision::Scalar>() == 8,
        micro_sigma: 0.0,
        micro_blur_px: 0.0,
        grain_blur: 0.0,
        usm_sigma: 0.0,
        usm_amount: 0.0,
        curve_x: curve.iter().flatten().copied().collect(),
        curve_layers: layers.iter().flatten().flatten().copied().collect(),
        sublayers_active: true,
        n_sub_layers: 3,
        mt19937_regime: true,
        density_max_composite: [3.0; 3],
        n_particles_composite: [0.0; 3],
    }
}

/// The MT19937 regime needs native f64. The device lock serializes the
/// software Vulkan driver, which is not safe under concurrent device use.
fn device() -> crate::test_support::DeviceBackend {
    crate::test_support::adapter()
}

#[test]
fn composite_mt19937_matches_reference_draw_order() {
    let gpu = device();
    assert!(gpu.device.features().contains(wgpu::Features::SHADER_F64));
    let (width, height) = (37, 11);
    let n_sub_layers = 3u32;
    let params = composite_params(17, false, n_sub_layers);
    let mut image = spektrafilm_math::image::ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let i = f64::from(x + y * width);
            image.set(
                x,
                y,
                [
                    from_f64(0.3 + (i % 7.0) * 0.05),
                    from_f64(0.8 + (i % 5.0) * 0.04),
                    from_f64(1.4 + (i % 3.0) * 0.06),
                ],
            );
        }
    }
    // The reference composite path: every sublayer draws from its own
    // `ch + sl·10` stream over the whole plane, all Poisson first, then all
    // binomial, and the summed layers are scaled by `1 / n_sub_layers`.
    let reference = spektrafilm_model::grain::v1::apply_grain_to_density(
        &image,
        params.pixel_size_um,
        0.2,
        [1.0; 3],
        params.density_min,
        params.density_max_composite,
        params.uniformity,
        0.0,
        n_sub_layers,
        false,
        params.base_seed,
        &CpuBackend,
    )
    .unwrap();
    let density = upload_density(&gpu, &image);
    let (layer_fields, layer_bytes) = layer_fields_buffer(&gpu, width, height, n_sub_layers * 3);
    let state = build_spektra_mt19937_state(
        &gpu.device,
        width,
        height,
        &density,
        &layer_fields,
        &params,
        &gpu,
    )
    .expect("native f64 MT19937 sampling");
    let fields = run_sampler(&gpu, &state, &layer_fields, layer_bytes);
    let expected = composed(
        &fields,
        width,
        height,
        n_sub_layers,
        1.0 / n_sub_layers as f32,
        params.density_min,
    );
    for (i, (actual, expected)) in expected.iter().zip(reference.pixels()).enumerate() {
        for ch in 0..3 {
            let reference_value = to_f64(expected[ch]) as f32;
            assert!(
                (actual[ch] - reference_value).abs() < 2e-5,
                "sample {i} channel {ch}: device {} != reference {reference_value}",
                actual[ch]
            );
        }
    }
}

#[test]
fn composite_monochrome_shares_the_channel_zero_stream() {
    let gpu = device();
    let (width, height) = (23, 9);
    let params = composite_params(0, true, 3);
    let mut image = spektrafilm_math::image::ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            image.set(x, y, [from_f64(0.9); 3]);
        }
    }
    let density = upload_density(&gpu, &image);
    let (layer_fields, layer_bytes) = layer_fields_buffer(&gpu, width, height, 9);
    let state = build_spektra_mt19937_state(
        &gpu.device,
        width,
        height,
        &density,
        &layer_fields,
        &params,
        &gpu,
    )
    .expect("native f64 MT19937 sampling");
    let fields = run_sampler(&gpu, &state, &layer_fields, layer_bytes);
    // Monochrome seeds every channel from stream 0, so the three channels
    // of one pixel must agree exactly.
    let n = (width * height) as usize;
    for i in 0..n {
        let plane = &fields[i * 3..i * 3 + 3];
        assert_eq!(plane[0], plane[1], "sample {i}");
        assert_eq!(plane[0], plane[2], "sample {i}");
    }
}

#[test]
fn composite_seed_changes_the_realization() {
    let gpu = device();
    let (width, height) = (16, 16);
    let mut image = spektrafilm_math::image::ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            image.set(x, y, [from_f64(0.8), from_f64(1.0), from_f64(1.2)]);
        }
    }
    let density = upload_density(&gpu, &image);
    let run = |seed| {
        let params = composite_params(seed, false, 3);
        let (layer_fields, layer_bytes) = layer_fields_buffer(&gpu, width, height, 9);
        let state = build_spektra_mt19937_state(
            &gpu.device,
            width,
            height,
            &density,
            &layer_fields,
            &params,
            &gpu,
        )
        .expect("native f64 MT19937 sampling");
        composed(
            &run_sampler(&gpu, &state, &layer_fields, layer_bytes),
            width,
            height,
            3,
            1.0 / 3.0,
            params.density_min,
        )
    };
    let a = run(0);
    let b = run(1);
    assert_ne!(a, b, "a different seed must produce a different realization");
    let again = run(0);
    assert_eq!(a, again, "the same seed must repeat exactly");
}

#[test]
fn composite_layer_count_changes_the_realization() {
    // `n_sub_layers` is a composite-path control: the request draws that
    // many planes per channel and scales the sum by `1 / n_sub_layers`.
    let gpu = device();
    let (width, height) = (16, 16);
    let mut image = spektrafilm_math::image::ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            image.set(x, y, [from_f64(0.8), from_f64(1.0), from_f64(1.2)]);
        }
    }
    let density = upload_density(&gpu, &image);
    let run = |n_sub_layers| {
        let params = composite_params(7, false, n_sub_layers);
        let (layer_fields, layer_bytes) = layer_fields_buffer(&gpu, width, height, n_sub_layers * 3);
        let state = build_spektra_mt19937_state(
            &gpu.device,
            width,
            height,
            &density,
            &layer_fields,
            &params,
            &gpu,
        )
        .expect("native f64 MT19937 sampling");
        composed(
            &run_sampler(&gpu, &state, &layer_fields, layer_bytes),
            width,
            height,
            n_sub_layers,
            1.0 / n_sub_layers as f32,
            params.density_min,
        )
    };
    let one = run(1);
    let three = run(3);
    assert_ne!(one, three, "a different composite layer count must change the field");
}

#[test]
fn layered_mt19937_matches_reference_draw_order() {
    let gpu = device();
    let (width, height) = (29, 7);
    let params = layered_params(5, false);
    let split = [0.25, 0.5, 0.25];
    let curve: Vec<[f64; 3]> = (0..4).map(|i| [i as f64; 3]).collect();
    let layers: Vec<[[f64; 3]; 3]> = curve
        .iter()
        .map(|row| std::array::from_fn(|sl| row.map(|v| v * split[sl])))
        .collect();
    let maxima = std::array::from_fn(|sl| [3.0 * split[sl]; 3]);
    let mut image = spektrafilm_math::image::ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let i = f64::from(x + y * width);
            image.set(
                x,
                y,
                [
                    from_f64(0.2 + (i % 11.0) * 0.05),
                    from_f64(0.8 + (i % 7.0) * 0.04),
                    from_f64(1.5 + (i % 5.0) * 0.06),
                ],
            );
        }
    }
    // The reference layered path in the scipy regime: the interpolated
    // sublayer densities feed `layer_particle_model` with
    // `use_fast_stats = false`.
    let planes = spektrafilm_model::density_curves::interp_density_cmy_layers(
        &image, &curve, &layers, false,
    );
    let reference = spektrafilm_model::grain::v1::apply_grain_to_density_layers(
        &planes,
        &maxima,
        width,
        height,
        params.pixel_size_um,
        params.particle_area,
        params.particle_scale_layers,
        params.density_min,
        params.uniformity,
        0.0,
        0.0,
        [0.0; 2],
        0.0,
        0.0,
        false,
        false,
        params.base_seed,
        &CpuBackend,
    )
    .unwrap();
    let density = upload_density(&gpu, &image);
    let (layer_fields, layer_bytes) = layer_fields_buffer(&gpu, width, height, 9);
    let state = build_spektra_mt19937_state(
        &gpu.device,
        width,
        height,
        &density,
        &layer_fields,
        &params,
        &gpu,
    )
    .expect("native f64 MT19937 sampling");
    let fields = run_sampler(&gpu, &state, &layer_fields, layer_bytes);
    // The layered reference sums the three sublayer fields without a scale.
    let expected = composed(&fields, width, height, 3, 1.0, params.density_min);
    for (i, (actual, expected)) in expected.iter().zip(reference.pixels()).enumerate() {
        for ch in 0..3 {
            let reference_value = to_f64(expected[ch]) as f32;
            assert!(
                (actual[ch] - reference_value).abs() < 2e-5,
                "sample {i} channel {ch}: device {} != reference {reference_value}",
                actual[ch]
            );
        }
    }
}

#[test]
fn layered_monochrome_shares_the_channel_zero_stream() {
    let gpu = device();
    let (width, height) = (19, 5);
    let mut params = layered_params(0, true);
    // Monochrome shares the RNG stream, not the control set, so the three
    // channels coincide only when their controls do. Collapse the
    // channel-varying controls onto channel 0 before asserting equality;
    // `composite_monochrome_shares_the_channel_zero_stream` is already
    // symmetric.
    params.particle_area = [params.particle_area[0]; 3];
    params.density_min = [params.density_min[0]; 3];
    params.uniformity = [params.uniformity[0]; 3];
    for sl in 0..3 {
        for ch in 1..3 {
            params.density_min_layers[sl * 3 + ch] = params.density_min_layers[sl * 3];
            params.density_max_adj[sl * 3 + ch] = params.density_max_adj[sl * 3];
            params.n_particles[sl * 3 + ch] = params.n_particles[sl * 3];
        }
    }
    let mut image = spektrafilm_math::image::ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            image.set(x, y, [from_f64(0.9); 3]);
        }
    }
    let density = upload_density(&gpu, &image);
    let (layer_fields, layer_bytes) = layer_fields_buffer(&gpu, width, height, 9);
    let state = build_spektra_mt19937_state(
        &gpu.device,
        width,
        height,
        &density,
        &layer_fields,
        &params,
        &gpu,
    )
    .expect("native f64 MT19937 sampling");
    let fields = run_sampler(&gpu, &state, &layer_fields, layer_bytes);
    let n = (width * height) as usize;
    for i in 0..n {
        let plane = &fields[i * 3..i * 3 + 3];
        assert_eq!(plane[0], plane[1], "sample {i}");
        assert_eq!(plane[0], plane[2], "sample {i}");
    }
}

#[test]
fn segmented_dispatch_preserves_the_realization() {
    // Compare production segmented submissions with a single command buffer
    // over freshly initialized generator state, retaining actual field parity.
    let gpu = device();
    let (width, height) = (32, 32);
    let params = composite_params(11, false, 3);
    let mut image = spektrafilm_math::image::ImageBuf::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let i = f64::from(x + y * width);
            image.set(
                x,
                y,
                [
                    from_f64(0.4 + (i % 9.0) * 0.05),
                    from_f64(0.9 + (i % 6.0) * 0.03),
                    from_f64(1.3 + (i % 4.0) * 0.07),
                ],
            );
        }
    }
    let density = upload_density(&gpu, &image);
    let (layer_fields, layer_bytes) = layer_fields_buffer(&gpu, width, height, 9);
    let state = build_spektra_mt19937_state(
        &gpu.device,
        width,
        height,
        &density,
        &layer_fields,
        &params,
        &gpu,
    )
    .expect("native f64 MT19937 sampling");
    let single = run_sampler(&gpu, &state, &layer_fields, layer_bytes);

    let state = build_spektra_mt19937_state(&gpu.device, width, height, &density, &layer_fields, &params, &gpu)
        .expect("fresh native f64 MT19937 sampling");
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    for step in &state.steps { step.encode(&mut encoder); }
    gpu.queue.submit(Some(encoder.finish()));
    let split = readback(&gpu, &layer_fields, layer_bytes);
    assert_eq!(single, split, "a submission boundary must not change the realization");
}


#[test]
fn builder_rejects_without_native_f64() {
    // A device without SHADER_F64 cannot condition the count laws in the
    // reference precision, so the sampler must name the limitation instead
    // of narrowing the arithmetic. On a device that has the feature this
    // case is vacuous and asserts nothing.
    let gpu = device();
    if gpu.device.features().contains(wgpu::Features::SHADER_F64) {
        return;
    }
    let params = composite_params(1, false, 3);
    let density = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("decline_density"),
        size: 12,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let (layer_fields, _) = layer_fields_buffer(&gpu, 1, 1, 9);
    let error = build_spektra_mt19937_state(
        &gpu.device, 1, 1, &density, &layer_fields, &params, &gpu,
    )
    .err().expect("a device without native f64 must decline");
    assert!(
        error.contains("native f64"),
        "the decline must name the limitation, got: {error}"
    );
}

#[test]
fn builder_rejects_inconsistent_curve_tables() {
    let gpu = device();
    let mut params = composite_params(1, false, 3);
    params.curve_layers.clear();
    let density = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("reject_density"),
        size: 12,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let (layer_fields, _) = layer_fields_buffer(&gpu, 1, 1, 9);
    let error = build_spektra_mt19937_state(
        &gpu.device, 1, 1, &density, &layer_fields, &params, &gpu,
    )
    .err().expect("a mismatched layer table must be rejected");
    assert!(error.contains("curve tables"), "got: {error}");
}

#[test]
fn builder_rejects_nonpositive_composite_density_maximum() {
    let gpu = device();
    let mut params = composite_params(1, false, 3);
    params.density_max_composite = [0.0, 2.2, 2.2];
    let density = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("reject_max_density"),
        size: 12,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let (layer_fields, _) = layer_fields_buffer(&gpu, 1, 1, 9);
    let error = build_spektra_mt19937_state(
        &gpu.device, 1, 1, &density, &layer_fields, &params, &gpu,
    )
    .err().expect("a zero composite density maximum must be rejected");
    assert!(error.contains("density maximum"), "got: {error}");
}

#[test]
fn builder_rejects_zero_pixel_count() {
    let gpu = device();
    let params = composite_params(1, false, 3);
    let density = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("reject_zero_density"),
        size: 12,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let (layer_fields, _) = layer_fields_buffer(&gpu, 1, 1, 9);
    let error = build_spektra_mt19937_state(
        &gpu.device, 0, 0, &density, &layer_fields, &params, &gpu,
    )
    .err().expect("an empty image must be rejected");
    assert!(error.contains("nonzero"), "got: {error}");
}

#[test]
fn storage_windows_preserve_complete_poisson_phase_and_mt_carry() {
    let gpu = device();
    let (width, height) = (257, 3);
    let image = spektrafilm_math::image::ImageBuf::from_data(width, height, (0..width * height).flat_map(|i| [0.1, 0.4, 0.8].map(|v| from_f64(v + f64::from(i % 17) * 0.025))).collect());
    let density = upload_density(&gpu, &image);
    for params in [composite_params(19, false, 4), layered_params(0, true)] {
        let layers = if params.sublayers_active { 3 } else { params.n_sub_layers };
        let (whole, bytes) = layer_fields_buffer(&gpu, width, height, layers);
        let bounded: Vec<_> = (0..width * height * layers).step_by(37).map(|first| {
            let count = 37.min(width * height * layers - first);
            super::super::super::paged::ImagePage { first_pixel: first, n_pixels: count,
                buffer: gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("mt_physical_seam"), size: u64::from(count) * 12, usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }) }
        }).collect();
        let normal = build_spektra_mt19937_state(&gpu.device, width, height, &density, &whole, &params, &gpu).unwrap();
        let split = build_spektra_mt19937_state_with_limit(&gpu.device, width, height, &density, &bounded, &params, &gpu, 4096).unwrap();
        assert!(split.steps.len() > normal.steps.len());
        let expected = run_sampler(&gpu, &normal, &whole, bytes);
        split.submit(&gpu).unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let mut copies = RasterCommands::default();
        copies.copy(&bounded, 0, &whole, 0, bytes);
        copies.encode(&mut encoder);
        gpu.queue.submit(Some(encoder.finish()));
        assert_eq!(expected, readback(&gpu, &whole, bytes));
    }
}
