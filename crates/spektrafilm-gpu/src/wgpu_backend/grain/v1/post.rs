use super::super::super::WgpuBackend;
use super::super::raster::{RasterCommands, RasterStorage, paged_transport, transport, uniform};
use super::blur::{GrainBlurState, build_grain_blur_paged};
use spektrafilm_backend::GrainV1GpuParams;

const SHARPEN_SHADER: &str = concat!(
    include_str!("../../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("post.wgsl")
);

/// Full-source finishing: all sharpened mass is reduced before any normalized
/// output is written. Storage windows never define a normalization domain.
pub(in crate::wgpu_backend) struct GrainPostState {
    before: RasterCommands,
    grain_blur: Option<GrainBlurState>,
    usm_blur: Option<GrainBlurState>,
    after: RasterCommands,
}

pub(in crate::wgpu_backend) fn build_grain_post(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    density: &(impl RasterStorage + ?Sized),
    params: &GrainV1GpuParams,
) -> Result<GrainPostState, String> {
    build_grain_post_with_limit(
        backend,
        width,
        height,
        density,
        params,
        super::super::super::bounded::storage_binding_limit(backend)?,
    )
}

fn build_grain_post_with_limit(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    density: &(impl RasterStorage + ?Sized),
    params: &GrainV1GpuParams,
    limit: u64,
) -> Result<GrainPostState, String> {
    let n = width
        .checked_mul(height)
        .filter(|&n| n > 0 && n <= u32::MAX / 3)
        .ok_or("grain post dimensions are not representable")?;
    let bytes = u64::from(n) * 12;
    let chunk = (limit / 12).min(u64::from(n)) as u32;
    if chunk == 0 {
        return Err("grain post minimum RGB binding needs 12 bytes".into());
    }
    let source_window = transport(backend, u64::from(chunk) * 12, "grain_post_source_window")?;
    let output_window = transport(backend, u64::from(chunk) * 12, "grain_post_output_window")?;
    let mut before = RasterCommands::default();
    let mut after = RasterCommands::default();
    let floor_pipe = backend.cached_pipeline(
        include_str!("post_floor_copy.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct FloorParams {
        dimensions: [u32; 4],
        minimum: [f32; 4],
    }
    let floor = |commands: &mut RasterCommands,
                 source: &dyn RasterStorage,
                 destination: &dyn RasterStorage| {
        for first in (0..n).step_by(chunk as usize) {
            let count = chunk.min(n - first);
            let p = uniform(
                backend,
                &FloorParams {
                    dimensions: [count, 0, 0, 0],
                    minimum: [
                        params.density_min[0] as f32,
                        params.density_min[1] as f32,
                        params.density_min[2] as f32,
                        0.0,
                    ],
                },
            );
            commands.copy(
                source,
                u64::from(first) * 12,
                &source_window,
                0,
                u64::from(count) * 12,
            );
            commands.dispatch(
                &floor_pipe,
                &[&p, &source_window, &output_window],
                count,
                false,
                backend,
            );
            commands.copy(
                &output_window,
                0,
                destination,
                u64::from(first) * 12,
                u64::from(count) * 12,
            );
        }
    };
    let layered = params.sublayers_active;
    let blur_active = if layered {
        params.grain_blur > 0.0
    } else {
        params.grain_blur > 0.4
    };
    let usm = layered && !(params.usm_sigma <= 0.0 || params.usm_amount <= 0.0);
    let mut grain_blur = None;
    let density = density.pages();
    let mut source = density.clone();
    if blur_active {
        let filtered = paged_transport(backend, bytes, "grain_post_filtered_transport")?;
        if !layered {
            floor(&mut before, &density, &filtered);
        }
        grain_blur = Some(if layered {
            build_grain_blur_paged(
                backend,
                width,
                height,
                &density,
                &filtered,
                [f64::from(params.grain_blur); 3],
                0,
                0,
            )?
        } else {
            build_grain_blur_paged(
                backend,
                width,
                height,
                &filtered,
                &density,
                [f64::from(params.grain_blur); 3],
                0,
                0,
            )?
        });
        if layered {
            source = filtered;
        }
    }
    let mut usm_blur = None;
    if usm {
        if !backend
            .device
            .features()
            .contains(wgpu::Features::SHADER_F64)
        {
            return Err("grain multiplicative USM requires native shader f64".into());
        }
        if limit < 48 {
            return Err("grain USM minimum mass binding needs 48 bytes".into());
        }
        let blurred = paged_transport(backend, bytes, "grain_usm_blurred_transport")?;
        usm_blur = Some(build_grain_blur_paged(
            backend,
            width,
            height,
            &source,
            &blurred,
            [f64::from(params.usm_sigma); 3],
            0,
            0,
        )?);
        let sharpened = paged_transport(backend, bytes, "grain_usm_sharpened_transport")?;
        let blur_window = transport(backend, u64::from(chunk) * 12, "grain_usm_blur_window")?;
        let lane_capacity = (chunk / 32).max(1);
        let pair = transport(
            backend,
            u64::from(lane_capacity) * 48,
            "grain_usm_lane_mass",
        )?;
        let first_items = n.div_ceil(8192) * 256;
        let mut partials = paged_transport(
            backend,
            u64::from(first_items) * 48,
            "grain_usm_mass_transport",
        )?;
        let sharpen_pipe = backend.cached_pipeline(
            SHARPEN_SHADER,
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct SharpenParams {
            dimensions: [u32; 4],
            controls: [f32; 4],
        }
        // Preserve the original 32-pixel lane sums and 256-lane reduction
        // tree. Only a window smaller than one lane needs carried state.
        let lane_batch = (chunk / 32).max(1);
        for lane in (0..n.div_ceil(32)).step_by(lane_batch as usize) {
            let begin = lane * 32;
            let end = n.min(begin + lane_batch * 32);
            for first in (begin..end).step_by(chunk as usize) {
                let count = chunk.min(end - first);
                let p = uniform(
                    backend,
                    &SharpenParams {
                        dimensions: [count, u32::from(first != begin), 0, 0],
                        controls: [params.usm_amount, 0.0, 0.0, 0.0],
                    },
                );
                after.copy(
                    &source,
                    u64::from(first) * 12,
                    &source_window,
                    0,
                    u64::from(count) * 12,
                );
                after.copy(
                    &blurred,
                    u64::from(first) * 12,
                    &blur_window,
                    0,
                    u64::from(count) * 12,
                );
                after.dispatch(
                    &sharpen_pipe,
                    &[&p, &source_window, &blur_window, &output_window, &pair],
                    count.div_ceil(32) * 3,
                    false,
                    backend,
                );
                after.copy(
                    &output_window,
                    0,
                    &sharpened,
                    u64::from(first) * 12,
                    u64::from(count) * 12,
                );
            }
            after.copy(
                &pair,
                0,
                &partials,
                u64::from(lane) * 48,
                u64::from((end - begin).div_ceil(32)) * 48,
            );
        }
        let reduce_pipe = backend.cached_pipeline(
            include_str!("post_reduce.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let mass_items = (limit / 48).min(256) as u32;
        let mass_window = transport(backend, u64::from(mass_items) * 48, "grain_usm_mass_window")?;
        let mut items = first_items;
        while items > 1 {
            let groups = items.div_ceil(256);
            let reduced = paged_transport(
                backend,
                u64::from(groups) * 48,
                "grain_usm_reduced_transport",
            )?;
            for group in 0..groups {
                let begin = group * 256;
                let end = items.min(begin + 256);
                for first in (begin..end).step_by(mass_items as usize) {
                    let count = mass_items.min(end - first);
                    let p = uniform(backend, &[count, u32::from(first != begin), 0u32, 0]);
                    after.copy(
                        &partials,
                        u64::from(first) * 48,
                        &mass_window,
                        0,
                        u64::from(count) * 48,
                    );
                    after.dispatch(&reduce_pipe, &[&p, &mass_window, &pair], 3, false, backend);
                }
                after.copy(&pair, 0, &reduced, u64::from(group) * 48, 48);
            }
            partials = reduced;
            items = groups;
        }
        let apply_pipe = backend.cached_pipeline(
            include_str!("post_apply.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        for first in (0..n).step_by(chunk as usize) {
            let count = chunk.min(n - first);
            let p = uniform(backend, &[count, 0u32, 0, 0]);
            after.copy(
                &sharpened,
                u64::from(first) * 12,
                &source_window,
                0,
                u64::from(count) * 12,
            );
            after.copy(&partials, 0, &pair, 0, 48);
            after.dispatch(
                &apply_pipe,
                &[&p, &source_window, &pair, &output_window],
                count,
                false,
                backend,
            );
            after.copy(
                &output_window,
                0,
                &density,
                u64::from(first) * 12,
                u64::from(count) * 12,
            );
        }
        floor(&mut after, &density, &density);
    } else if layered || !blur_active {
        floor(&mut after, &source, &density);
    }
    Ok(GrainPostState {
        before,
        grain_blur,
        usm_blur,
        after,
    })
}

impl GrainPostState {
    pub(in crate::wgpu_backend) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.before.submit(backend)?;
        if let Some(blur) = &self.grain_blur {
            blur.submit(backend)?;
        }
        if let Some(blur) = &self.usm_blur {
            blur.submit(backend)?;
        }
        self.after.submit(backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use spektrafilm_math::image::ImageBuf;
    use spektrafilm_math::precision::{from_f32, from_f64, to_f32, to_f64};

    // Numerical budget: max absolute sample error in density units over every
    // sample of each synthetic wedge. Sources: f32 sample storage (one ulp at
    // density 4 is 2.4e-7), the f64 elementary functions (measured 1.81 eps
    // relative through `f64_math.wgsl`; the USM path evaluates two of them),
    // and the sibling Gaussian's tap-order f32 accumulation, which uses the
    // reference weights and stays below 1e-6 relative of a few-density result.
    // 2e-5 is ~20x the largest term and matches the sampler acceptance budget,
    // so a regression of a whole stage cannot hide inside it.
    const MAX_SAMPLE_ERROR: f64 = 2e-5;
    // Mass conservation is consumer-visible: after the ratio, each channel's
    // output mass must return to its input mass. f64 reductions are exact to
    // rounding and the f32 output stores add at most one ulp per sample, so
    // the relative bound is 1e-5.
    const MAX_RELATIVE_MASS_ERROR: f64 = 1e-5;

    fn params(layered: bool) -> GrainV1GpuParams {
        GrainV1GpuParams {
            base_seed: 7,
            pixel_size_um: 2.0,
            particle_area: [0.1; 3],
            particle_scale_layers: [1.0; 3],
            density_min: [0.03, 0.05, 0.07],
            uniformity: [0.98; 3],
            density_min_layers: [0.01; 9],
            density_max_adj: [1.0; 9],
            n_particles: [100.0; 9],
            monochrome: false,
            lookup_sign: 1.0,
            blur_dye_clouds: 0.0,
            reference_f64: std::mem::size_of::<spektrafilm_math::precision::Scalar>() == 8,
            micro_sigma: 0.0,
            micro_blur_px: 0.0,
            grain_blur: 0.0,
            usm_sigma: 0.0,
            usm_amount: 0.0,
            curve_x: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            curve_layers: vec![0.0; 18],
            sublayers_active: layered,
            n_sub_layers: 3,
            mt19937_regime: false,
            density_max_composite: [3.0; 3],
            n_particles_composite: [100.0; 3],
        }
    }

    fn run(
        gpu: &WgpuBackend,
        width: u32,
        height: u32,
        input: &[f32],
        p: &GrainV1GpuParams,
    ) -> Vec<f32> {
        let bytes = input.len() as u64 * 4;
        let density = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grain_post_input"),
                contents: bytemuck::cast_slice(input),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            });
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grain_post_readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let state = build_grain_post(gpu, width, height, &density, p).expect("grain post builds");
        state.submit(gpu).unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&density, 0, &readback, 0, bytes);
        gpu.queue.submit(Some(encoder.finish()));
        gpu.read_mapped_f32(&readback, bytes)
    }

    #[test]
    fn storage_windows_keep_complete_source_usm_mass() {
        let gpu = test_support::adapter();
        let (width, height) = (257, 3);
        let input = wedge(width, height);
        let mut p = params(true);
        p.usm_sigma = 1.2;
        p.usm_amount = 1.5;
        let expected = run(&gpu, width, height, &input, &p);
        let bytes = input.len() as u64 * 4;
        let density = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grain_usm_boundary_input"),
                contents: bytemuck::cast_slice(&input),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            });
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grain_usm_boundary_readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let state = build_grain_post_with_limit(&gpu, width, height, &density, &p, 192).unwrap();
        state.submit(&gpu).unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&density, 0, &readback, 0, bytes);
        gpu.queue.submit(Some(encoder.finish()));
        assert_eq!(
            expected,
            gpu.read_mapped_f32(&readback, bytes),
            "window-local USM mass changed the image"
        );
    }

    fn reference_unsharp(image: &ImageBuf, sigma: f32, amount: f32) -> ImageBuf {
        let blurred = spektrafilm_math::gaussian::gaussian_blur(image, f64::from(sigma));
        let mut out = image.clone();
        let mut sums = [0.0f64; 3];
        let mut out_sums = [0.0f64; 3];
        for (i, px) in image.pixels().enumerate() {
            let out_px = &mut out.data[i * 3..i * 3 + 3];
            let blur_px = &blurred.data[i * 3..i * 3 + 3];
            for ch in 0..3 {
                let density = to_f64(px[ch]).max(0.0);
                let blur_density = to_f64(blur_px[ch]).max(1e-6);
                let sharpened = if density <= 0.0 {
                    0.0
                } else {
                    density.powf(1.0 + f64::from(amount)) / blur_density.powf(f64::from(amount))
                };
                out_px[ch] = from_f64(sharpened);
                sums[ch] += density;
                out_sums[ch] += sharpened;
            }
        }
        for px in out.pixels_mut() {
            for ch in 0..3 {
                if out_sums[ch] > 0.0 {
                    px[ch] = from_f64(to_f64(px[ch]) * sums[ch] / out_sums[ch]);
                }
            }
        }
        out
    }

    /// The layered reference tail after composition: optional `grain_blur`,
    /// then multiplicative USM, then the floor subtraction.
    fn reference_layered(input: &[f32], width: u32, height: u32, p: &GrainV1GpuParams) -> Vec<f32> {
        let mut image =
            ImageBuf::from_data(width, height, input.iter().map(|&v| from_f32(v)).collect());
        if p.grain_blur > 0.0 {
            image = spektrafilm_math::gaussian::gaussian_blur(&image, f64::from(p.grain_blur));
        }
        if p.usm_sigma > 0.0 && p.usm_amount > 0.0 {
            image = reference_unsharp(&image, p.usm_sigma, p.usm_amount);
        }
        for px in image.pixels_mut() {
            for (sample, &density_min) in px.iter_mut().zip(&p.density_min) {
                *sample -= from_f64(density_min);
            }
        }
        image.data.iter().map(|&v| to_f32(v)).collect()
    }

    /// The composite reference tail: floor subtraction first, then the final
    /// blur only above 0.4, and no multiplicative USM.
    fn reference_composite(
        input: &[f32],
        width: u32,
        height: u32,
        p: &GrainV1GpuParams,
    ) -> Vec<f32> {
        let mut image =
            ImageBuf::from_data(width, height, input.iter().map(|&v| from_f32(v)).collect());
        for px in image.pixels_mut() {
            for (sample, &density_min) in px.iter_mut().zip(&p.density_min) {
                *sample -= from_f64(density_min);
            }
        }
        if p.grain_blur > 0.4 {
            image = spektrafilm_math::gaussian::gaussian_blur(&image, f64::from(p.grain_blur));
        }
        image.data.iter().map(|&v| to_f32(v)).collect()
    }

    fn wedge(width: u32, height: u32) -> Vec<f32> {
        (0..width * height)
            .flat_map(|i| {
                [0.25, 0.9, 1.7].map(|base| {
                    let v = base + (i % 17) as f32 * 0.07;
                    if i % 29 == 0 {
                        0.0
                    } else if i % 31 == 0 {
                        -0.2
                    } else {
                        v
                    }
                })
            })
            .collect()
    }

    fn check_parity(
        gpu: &WgpuBackend,
        width: u32,
        height: u32,
        input: &[f32],
        p: &GrainV1GpuParams,
        expected: &[f32],
        label: &str,
    ) {
        let actual = run(gpu, width, height, input, p);
        assert_eq!(actual.len(), expected.len());
        for (i, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
            assert!(
                actual.is_finite(),
                "{label}: sample {i} is not finite ({actual})"
            );
            let error = (f64::from(actual) - f64::from(expected)).abs();
            assert!(
                error <= MAX_SAMPLE_ERROR,
                "{label}: sample {i} device {actual} reference {expected} error {error} exceeds {MAX_SAMPLE_ERROR}"
            );
        }
    }

    #[test]
    fn layered_usm_matches_reference_and_conserves_mass() {
        let gpu = test_support::adapter();
        let (width, height) = (257, 3);
        let input = wedge(width, height);
        let mut p = params(true);
        p.grain_blur = 0.0;
        p.usm_sigma = 1.1;
        p.usm_amount = 1.4;
        let expected = reference_layered(&input, width, height, &p);
        // The population must separate the USM stage from a plain floor
        // subtraction, otherwise a silently omitted stage would pass parity.
        let mut plain = p.clone();
        plain.usm_sigma = 0.0;
        plain.usm_amount = 0.0;
        let floor_only = reference_layered(&input, width, height, &plain);
        assert!(
            expected
                .iter()
                .zip(&floor_only)
                .any(|(a, b)| (f64::from(*a) - f64::from(*b)).abs() > 1e-3),
            "USM case must move samples well beyond the comparison budget"
        );
        check_parity(&gpu, width, height, &input, &p, &expected, "layered usm");
        let actual = run(&gpu, width, height, &input, &p);

        // Consumer-visible mass: adding the removed floor back must recover the
        // input channel mass after the multiplicative correction.
        for ch in 0..3 {
            let input_mass: f64 = (0..(width * height) as usize)
                .map(|i| f64::from(input[i * 3 + ch].max(0.0)))
                .sum();
            let output_mass: f64 = (0..(width * height) as usize)
                .map(|i| f64::from(actual[i * 3 + ch]) + p.density_min[ch])
                .sum();
            let relative = (output_mass - input_mass).abs() / input_mass.max(f64::MIN_POSITIVE);
            assert!(
                relative <= MAX_RELATIVE_MASS_ERROR,
                "channel {ch} mass {output_mass} differs from input mass {input_mass} by {relative}"
            );
        }
    }

    #[test]
    fn layered_blur_then_usm_matches_reference() {
        let gpu = test_support::adapter();
        let (width, height) = (257, 3);
        let input = wedge(width, height);
        let mut p = params(true);
        p.grain_blur = 0.8;
        p.usm_sigma = 2.0;
        p.usm_amount = 0.9;
        let expected = reference_layered(&input, width, height, &p);
        check_parity(
            &gpu,
            width,
            height,
            &input,
            &p,
            &expected,
            "layered blur+usm",
        );
    }

    #[test]
    fn layered_without_usm_subtracts_floor_exactly() {
        let gpu = test_support::adapter();
        let (width, height) = (257, 3);
        let input = wedge(width, height);
        let p = params(true);
        let actual = run(&gpu, width, height, &input, &p);
        let expected: Vec<f32> = input
            .iter()
            .enumerate()
            .map(|(i, &v)| v - p.density_min[i % 3] as f32)
            .collect();
        assert_eq!(
            actual, expected,
            "disabled USM must still remove the density floor"
        );
    }

    #[test]
    fn zero_density_usm_keeps_zero_and_subtracts_floor() {
        let gpu = test_support::adapter();
        let (width, height) = (257, 3);
        let input = vec![0.0f32; (width * height) as usize * 3];
        let mut p = params(true);
        p.usm_sigma = 1.1;
        p.usm_amount = 1.4;
        let actual = run(&gpu, width, height, &input, &p);
        // `out_sums[c] == 0` takes the reference's no-rescale branch, then the
        // floor is removed: f32(0 - f32(density_min)).
        let expected: Vec<f32> = (0..(width * height) as usize * 3)
            .map(|i| 0.0f32 - p.density_min[i % 3] as f32)
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn composite_threshold_and_floor_order_match_reference() {
        let gpu = test_support::adapter();
        let (width, height) = (257, 3);
        let input = wedge(width, height);

        // Below the composite gate (> 0.4) the blur does not run and USM is
        // never part of the composite path.
        let mut p = params(false);
        p.grain_blur = 0.2;
        p.usm_sigma = 1.1;
        p.usm_amount = 1.4;
        let expected = reference_composite(&input, width, height, &p);
        check_parity(
            &gpu,
            width,
            height,
            &input,
            &p,
            &expected,
            "composite floor only",
        );

        // Above the gate the floor is removed before the final blur.
        p.grain_blur = 0.6;
        let expected = reference_composite(&input, width, height, &p);
        check_parity(
            &gpu,
            width,
            height,
            &input,
            &p,
            &expected,
            "composite floor then blur",
        );
    }
}
