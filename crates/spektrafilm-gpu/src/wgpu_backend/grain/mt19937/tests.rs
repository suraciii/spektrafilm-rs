//! Device regressions for active-layer MT19937 grain.
//!
//! CPU references use the shared curve/RMS preparation and `apply_grain`.
//! Cases preserve whole-plane Poisson-before-binomial ordering, NumPy clump
//! state continuation, and exact realization across dispatch and page seams.

use super::*;
use spektrafilm_backend::cpu_backend::CpuBackend;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};
use spektrafilm_model::grain::v1::{PreparedGrain, apply_grain, prepare_grain};

const PIXEL_SIZE_UM: f64 = 2.0;

fn readback<T: bytemuck::Pod>(
    gpu: &crate::test_support::DeviceBackend,
    buffer: &wgpu::Buffer,
    bytes: u64,
) -> Vec<T> {
    let target = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mt19937_readback"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &target, 0, bytes);
    gpu.queue.submit(Some(encoder.finish()));
    gpu.pipe.read_mapped(&target, bytes)
}

fn storage(gpu: &crate::test_support::DeviceBackend, bytes: u64, label: &str) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Fields are f32 `[layer][pixel][channel]`; clumps are f64 HWC.
fn outputs(
    gpu: &crate::test_support::DeviceBackend,
    width: u32,
    height: u32,
    layers: u32,
) -> (wgpu::Buffer, wgpu::Buffer) {
    let pixels = u64::from(width) * u64::from(height);
    (
        storage(gpu, pixels * 12 * u64::from(layers), "mt19937_layer_fields"),
        storage(gpu, pixels * 24, "mt19937_clumps_f64"),
    )
}

fn upload_density(gpu: &crate::test_support::DeviceBackend, image: &ImageBuf) -> wgpu::Buffer {
    let input: Vec<f32> = image.data.iter().map(|&v| to_f64(v) as f32).collect();
    gpu.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mt19937_density"),
            contents: bytemuck::cast_slice(&input),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        })
}

fn curves(layer_count: u32) -> (Vec<[f64; 3]>, Vec<[[f64; 3]; 3]>) {
    let weights = [0.25, 0.5, 0.25];
    let total: f64 = weights[..layer_count as usize].iter().sum();
    let curve: Vec<[f64; 3]> = [0.0, 1.00000001, 1.00000002, 3.0]
        .into_iter()
        .map(|v| [v; 3])
        .collect();
    let layers = curve
        .iter()
        .enumerate()
        .map(|(knot, _)| {
            std::array::from_fn(|sl| {
                let v = [0.0, 0.2, 1.0, 3.0][knot];
                [if sl < layer_count as usize {
                    v * weights[sl] / total
                } else {
                    0.0
                }; 3]
            })
        })
        .collect();
    (curve, layers)
}

fn prepared(layer_count: u32, monochrome: bool) -> PreparedGrain {
    let (_, layers) = curves(layer_count);
    prepare_grain(
        &layers,
        layer_count,
        PIXEL_SIZE_UM,
        if monochrome {
            [6.0; 3]
        } else {
            [6.0, 8.0, 10.0]
        },
        [2.0, 1.0, 0.5],
        if monochrome {
            [0.03; 3]
        } else {
            [0.03, 0.05, 0.07]
        },
        if monochrome {
            [0.97; 3]
        } else {
            [0.97, 0.99, 0.95]
        },
    )
    .unwrap()
}

fn params(
    base_seed: u64,
    monochrome: bool,
    layer_count: u32,
) -> spektrafilm_backend::GrainV1GpuParams {
    let (curve, layers) = curves(layer_count);
    let prepared = prepared(layer_count, monochrome);
    spektrafilm_backend::GrainV1GpuParams {
        base_seed,
        layer_count,
        density_min: prepared.density_min,
        uniformity: prepared.uniformity,
        density_min_layers: std::array::from_fn(|i| prepared.density_min_layers[i / 3][i % 3]),
        density_max_adj: std::array::from_fn(|i| prepared.density_max_adj[i / 3][i % 3]),
        n_particles: std::array::from_fn(|i| prepared.n_particles[i / 3][i % 3]),
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
        mt19937_regime: true,
    }
}

fn image(width: u32, height: u32) -> ImageBuf {
    ImageBuf::from_data(
        width,
        height,
        (0..width * height)
            .flat_map(|i| {
                std::array::from_fn::<_, 3, _>(|ch| {
                    let values = [-0.1f32, 0.0, 0.2, 0.8, 1.0, 1.5, 3.0, 3.1];
                    from_f64(f64::from(values[(i as usize + ch) % values.len()]))
                })
            })
            .collect(),
    )
}

fn reference(image: &ImageBuf, params: &spektrafilm_backend::GrainV1GpuParams) -> ImageBuf {
    let (mut curve, mut layers) = curves(params.layer_count);
    if params.lookup_sign < 0.0 {
        curve.reverse();
        layers.reverse();
    }
    apply_grain(
        image,
        &curve,
        &layers,
        &prepared(params.layer_count, params.monochrome),
        PIXEL_SIZE_UM,
        params.grain_blur,
        params.blur_dye_clouds,
        [
            params.micro_blur_px * PIXEL_SIZE_UM,
            params.micro_sigma * PIXEL_SIZE_UM / 0.001,
        ],
        params.usm_sigma,
        params.usm_amount,
        params.lookup_sign < 0.0,
        params.monochrome,
        false,
        params.base_seed,
        &CpuBackend,
    )
    .unwrap()
}

fn composed(
    fields: &[f32],
    pixels: usize,
    params: &spektrafilm_backend::GrainV1GpuParams,
) -> Vec<f32> {
    (0..pixels * 3)
        .map(|i| {
            let mut sum = 0.0f32;
            for layer in 0..params.layer_count as usize {
                sum += fields[layer * pixels * 3 + i];
            }
            let ch = if params.monochrome { 0 } else { i % 3 };
            (f64::from(sum) - params.density_min[ch]) as f32
        })
        .collect()
}

#[test]
fn working_quotients_match_numpy_float32_rounding() {
    let gpu = crate::test_support::adapter();
    assert!(
        gpu.device
            .features()
            .contains(wgpu::Features::SHADER_F64 | wgpu::Features::SHADER_INT64)
    );
    const SHADER: &str = concat!(
        include_str!("../../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
        include_str!("../work_f32.wgsl"),
        r#"
@group(0) @binding(0) var<storage, read> inputs: array<vec2<f32>>;
@group(0) @binding(1) var<storage, read_write> outputs: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  if gid.x >= arrayLength(&inputs) { return; }
  let pair = inputs[gid.x];
  outputs[gid.x] = sf32_div(pair.x, pair.y);
}
"#
    );
    // Includes the 1.53/3.03 probability whose one-ULP error changed both
    // samplers' sample 15, plus high-density Poisson rates for every plane.
    let mut input = vec![[1.53f32, 3.03f32]];
    for layer_count in 1..=3 {
        let prepared = prepared(layer_count, false);
        for sl in 0..layer_count as usize {
            for ch in 0..3 {
                for net in [0.0, 1.5, 3.0] {
                    let absolute =
                        (f64::from(net as f32) + prepared.density_min_layers[sl][ch]) as f32;
                    let maximum = prepared.density_max_adj[sl][ch] as f32;
                    input.push([absolute, maximum]);
                    let prob = (absolute / maximum).clamp(1e-6f32, 1.0f32 - 1e-6f32);
                    let sat = 1.0f32 - prob * prepared.uniformity[ch] as f32 * (1.0f32 - 1e-6f32);
                    input.push([prepared.n_particles[sl][ch] as f32, sat]);
                }
            }
        }
    }
    let source = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("working_quotients_input"),
            contents: bytemuck::cast_slice(&input),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let bytes = input.len() as u64 * 4;
    let destination = storage(&gpu, bytes, "working_quotients_output");
    let pipe = gpu.cached_pipeline(
        SHADER,
        &[
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let mut commands = RasterCommands::default();
    commands.dispatch(&pipe, &[&source, &destination], 1, true, &gpu);
    commands.submit(&gpu).unwrap();
    let actual = readback::<f32>(&gpu, &destination, bytes);
    for ([numerator, denominator], actual) in input.into_iter().zip(actual) {
        assert_eq!(
            actual.to_bits(),
            (numerator / denominator).to_bits(),
            "working quotient {numerator}/{denominator} must round exactly once to f32"
        );
    }
}

#[test]
fn active_layers_mt19937_match_reference_draw_order() {
    let gpu = crate::test_support::adapter();
    assert!(gpu.device.features().contains(wgpu::Features::SHADER_F64));
    let (width, height) = (37, 11);
    let image = image(width, height);
    let density = upload_density(&gpu, &image);
    for layer_count in 1..=3 {
        let params = params(17, false, layer_count);
        let reference = reference(&image, &params);
        let (fields, clumps) = outputs(&gpu, width, height, layer_count);
        let state = build_spektra_mt19937_state(
            &gpu.device,
            width,
            height,
            &density,
            &fields,
            &clumps,
            &params,
            &gpu,
        )
        .expect("native f64 MT19937 sampling");
        state.submit(&gpu).unwrap();
        let fields = readback::<f32>(&gpu, &fields, fields.size());
        let actual = composed(&fields, (width * height) as usize, &params);
        for (i, (&actual, &expected)) in actual.iter().zip(&reference.data).enumerate() {
            let expected = to_f64(expected);
            assert_eq!(
                actual, expected as f32,
                "layer_count {layer_count}, sample {i}: count conditioning/product or floor differs"
            );
        }
    }
}

#[test]
fn monochrome_shares_channel_zero_fields_and_clumps() {
    let gpu = crate::test_support::adapter();
    let (width, height) = (23, 9);
    let image = ImageBuf::from_data(
        width,
        height,
        vec![from_f64(f64::from(0.9f32)); (width * height * 3) as usize],
    );
    let density = upload_density(&gpu, &image);
    for layer_count in 1..=3 {
        let mut params = params(0, true, layer_count);
        params.micro_sigma = 0.2;
        let (fields, clumps) = outputs(&gpu, width, height, layer_count);
        let state = build_spektra_mt19937_state(
            &gpu.device,
            width,
            height,
            &density,
            &fields,
            &clumps,
            &params,
            &gpu,
        )
        .unwrap();
        state.submit(&gpu).unwrap();
        for pixel in readback::<f32>(&gpu, &fields, fields.size())
            .as_chunks::<3>()
            .0
        {
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[0], pixel[2]);
        }
        for pixel in readback::<f64>(&gpu, &clumps, clumps.size())
            .as_chunks::<3>()
            .0
        {
            assert!(pixel[0].is_finite() && pixel[0] > 0.0);
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[0], pixel[2]);
        }
    }
}

#[test]
fn seed_and_active_layer_count_change_the_realization() {
    let gpu = crate::test_support::adapter();
    let (width, height) = (16, 16);
    let density = upload_density(&gpu, &image(width, height));
    let run = |seed, layer_count| {
        let params = params(seed, false, layer_count);
        let (fields, clumps) = outputs(&gpu, width, height, layer_count);
        let state = build_spektra_mt19937_state(
            &gpu.device,
            width,
            height,
            &density,
            &fields,
            &clumps,
            &params,
            &gpu,
        )
        .unwrap();
        state.submit(&gpu).unwrap();
        composed(
            &readback::<f32>(&gpu, &fields, fields.size()),
            (width * height) as usize,
            &params,
        )
    };
    let one = run(7, 1);
    let two = run(7, 2);
    let three = run(7, 3);
    assert_ne!(one, two, "active layer count must affect the field");
    assert_ne!(two, three, "active layer count must affect the field");
    assert_ne!(three, run(8, 3), "seed must affect the field");
    assert_eq!(three, run(7, 3), "same seed must repeat exactly");
}

#[test]
fn clumping_continues_the_final_active_plane_numpy_state() {
    let gpu = crate::test_support::adapter();
    let (width, height) = (29, 7);
    for layer_count in 1..=3 {
        for monochrome in [false, true] {
            let image = if monochrome {
                ImageBuf::from_data(
                    width,
                    height,
                    vec![from_f64(f64::from(0.9f32)); (width * height * 3) as usize],
                )
            } else {
                image(width, height)
            };
            let mut params = params(5, monochrome, layer_count);
            let particle_reference = reference(&image, &params);
            // Nonzero clumping exposes continuation of the final sampler stream,
            // including the cached normal variate across the odd pixel count.
            params.micro_sigma = 0.2;
            let reference = reference(&image, &params);
            let density = upload_density(&gpu, &image);
            // A one-count field difference can masquerade as a continued-normal
            // failure. Check the unclumped realization exactly before comparing
            // the composed clumping pass with its transcendental error budget.
            let (fields, clumps) = outputs(&gpu, width, height, layer_count);
            let sampler = build_spektra_mt19937_state(
                &gpu.device,
                width,
                height,
                &density,
                &fields,
                &clumps,
                &params,
                &gpu,
            )
            .unwrap();
            sampler.submit(&gpu).unwrap();
            let fields = readback::<f32>(&gpu, &fields, fields.size());
            let particles = composed(&fields, (width * height) as usize, &params);
            for (i, (&actual, &expected)) in
                particles.iter().zip(&particle_reference.data).enumerate()
            {
                assert_eq!(
                    actual,
                    to_f64(expected) as f32,
                    "layer_count {layer_count}, monochrome {monochrome}, sample {i}: particle count before clumping differs"
                );
            }
            let state = super::super::v1::build_spektra_grain_state(
                &gpu.device,
                width,
                height,
                &density,
                &params,
                &gpu,
            )
            .unwrap();
            state.submit(&gpu).unwrap();
            let actual = readback::<f32>(&gpu, &density, density.size());
            for (i, (&actual, &expected)) in actual.iter().zip(&reference.data).enumerate() {
                let expected = to_f64(expected);
                assert!(
                    actual.is_finite() && (f64::from(actual) - expected).abs() < 2e-5,
                    "layer_count {layer_count}, monochrome {monochrome}, sample {i}: {actual} != {expected}"
                );
            }
        }
    }
}

#[test]
fn segmented_dispatch_preserves_fields_and_clump_continuation() {
    let gpu = crate::test_support::adapter();
    let (width, height) = (37, 29);
    let density = upload_density(&gpu, &image(width, height));
    let mut params = params(11, false, 3);
    params.micro_sigma = 0.2;
    let (fields, clumps) = outputs(&gpu, width, height, 3);
    let state = build_spektra_mt19937_state_with_limit(
        &gpu.device,
        width,
        height,
        &density,
        &fields,
        &clumps,
        &params,
        &gpu,
        4096,
    )
    .unwrap();
    state.submit(&gpu).unwrap();
    let expected_fields = readback::<f32>(&gpu, &fields, fields.size());
    let expected_clumps = readback::<f64>(&gpu, &clumps, clumps.size());
    let state = build_spektra_mt19937_state_with_limit(
        &gpu.device,
        width,
        height,
        &density,
        &fields,
        &clumps,
        &params,
        &gpu,
        4096,
    )
    .unwrap();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    for step in &state.steps {
        step.encode(&mut encoder);
    }
    gpu.queue.submit(Some(encoder.finish()));
    assert_eq!(
        expected_fields,
        readback::<f32>(&gpu, &fields, fields.size())
    );
    assert_eq!(
        expected_clumps,
        readback::<f64>(&gpu, &clumps, clumps.size())
    );
}

#[test]
fn builder_rejects_without_native_f64() {
    let gpu = crate::test_support::adapter();
    if gpu
        .device
        .features()
        .contains(wgpu::Features::SHADER_F64 | wgpu::Features::SHADER_INT64)
    {
        return;
    }
    let params = params(1, false, 3);
    let density = storage(&gpu, 12, "decline_density");
    let (fields, clumps) = outputs(&gpu, 1, 1, 3);
    let error =
        build_spektra_mt19937_state(&gpu.device, 1, 1, &density, &fields, &clumps, &params, &gpu)
            .err()
            .expect("a device without native f64 must decline");
    assert!(error.contains("native f64"), "got: {error}");
    let mut invalid = params;
    invalid.layer_count = 0;
    invalid.curve_layers.clear();
    let error = build_spektra_mt19937_state(
        &gpu.device,
        0,
        0,
        &density,
        &fields,
        &clumps,
        &invalid,
        &gpu,
    )
    .err()
    .expect("missing shader capabilities must take precedence over invalid inputs");
    assert!(error.contains("native f64"), "got: {error}");
}

mod logic_tests {
    use super::*;

    #[test]
    fn builder_rejects_inconsistent_curve_tables() {
        let mut params = params(1, false, 3);
        assert!(validate_inputs(1, 1, &params).is_ok());
        params.curve_layers.clear();
        let error = validate_inputs(1, 1, &params).unwrap_err();
        assert!(error.contains("curve tables"), "got: {error}");
    }

    #[test]
    fn builder_rejects_invalid_active_layer_counts() {
        let mut params = params(1, false, 3);
        for layer_count in 1..=3 {
            params.layer_count = layer_count;
            assert!(validate_inputs(1, 1, &params).is_ok());
        }
        for layer_count in [0, 4] {
            params.layer_count = layer_count;
            assert!(
                validate_inputs(1, 1, &params).is_err(),
                "layer_count {layer_count} must be rejected"
            );
        }
    }

    #[test]
    fn builder_rejects_zero_pixel_count() {
        let params = params(1, false, 3);
        for (width, height) in [(0, 0), (0, 1), (1, 0)] {
            let error = validate_inputs(width, height, &params).unwrap_err();
            assert!(error.contains("zero"), "got: {error}");
        }
        assert!(validate_inputs(u32::MAX, 2, &params).is_err());
        assert!(validate_inputs(u32::MAX / 3 + 1, 1, &params).is_err());
        assert!(validate_inputs(u32::MAX / 3, 1, &params).is_ok());
    }

    #[test]
    fn validation_reports_pixel_then_curve_then_layer_errors() {
        let mut params = params(1, false, 3);
        params.curve_layers.clear();
        params.layer_count = 0;
        assert!(validate_inputs(0, 1, &params).unwrap_err().contains("zero"));
        assert!(
            validate_inputs(1, 1, &params)
                .unwrap_err()
                .contains("curve tables")
        );
    }
}

fn physical_pages(
    gpu: &crate::test_support::DeviceBackend,
    bytes: u64,
) -> Vec<super::super::super::paged::ImagePage> {
    // RasterStorage offsets use 12-byte units even for binary64 clumps.
    // Odd page lengths intentionally split some f64 samples between pages.
    let pixels = (bytes / 12) as u32;
    (0..pixels)
        .step_by(37)
        .map(|first| {
            let count = 37.min(pixels - first);
            super::super::super::paged::ImagePage {
                first_pixel: first,
                n_pixels: count,
                buffer: storage(gpu, u64::from(count) * 12, "mt_physical_seam"),
            }
        })
        .collect()
}

#[test]
fn storage_windows_preserve_poisson_phase_mt_carry_and_clumps() {
    let gpu = crate::test_support::adapter();
    let (width, height) = (257, 3);
    let density = upload_density(&gpu, &image(width, height));
    for layer_count in 1..=3 {
        for monochrome in [false, true] {
            let mut params = params(19, monochrome, layer_count);
            params.micro_sigma = 0.2;
            let (fields, clumps) = outputs(&gpu, width, height, layer_count);
            let bounded_fields = physical_pages(&gpu, fields.size());
            let bounded_clumps = physical_pages(&gpu, clumps.size());
            let normal = build_spektra_mt19937_state(
                &gpu.device,
                width,
                height,
                &density,
                &fields,
                &clumps,
                &params,
                &gpu,
            )
            .unwrap();
            let split = build_spektra_mt19937_state_with_limit(
                &gpu.device,
                width,
                height,
                &density,
                &bounded_fields,
                &bounded_clumps,
                &params,
                &gpu,
                4096,
            )
            .unwrap();
            assert!(split.steps.len() > normal.steps.len());
            normal.submit(&gpu).unwrap();
            let expected_fields = readback::<f32>(&gpu, &fields, fields.size());
            let expected_clumps = readback::<f64>(&gpu, &clumps, clumps.size());
            split.submit(&gpu).unwrap();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let mut copies = RasterCommands::default();
            copies.copy(&bounded_fields, 0, &fields, 0, fields.size());
            copies.copy(&bounded_clumps, 0, &clumps, 0, clumps.size());
            copies.encode(&mut encoder);
            gpu.queue.submit(Some(encoder.finish()));
            assert_eq!(
                expected_fields,
                readback::<f32>(&gpu, &fields, fields.size())
            );
            assert_eq!(
                expected_clumps,
                readback::<f64>(&gpu, &clumps, clumps.size())
            );
        }
    }
}
