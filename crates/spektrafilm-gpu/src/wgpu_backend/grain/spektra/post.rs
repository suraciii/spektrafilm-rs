use super::super::super::{CachedPipelineRef, WgpuBackend};
use super::super::raster::{RasterCommands, RasterStorage, paged_transport, transport, uniform};
use super::blur::{GrainBlurState, build_grain_blur_paged};
use spektrafilm_backend::SpektraGrainGpuParams;

const SHARPEN_SHADER: &str = concat!(
    include_str!("../../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("post.wgsl")
);

/// Full-source finishing: all sharpened mass is reduced before any normalized
/// output is written. Storage windows never define a normalization domain.
pub(in crate::wgpu_backend) struct GrainPostState {
    grain_blur: Option<GrainBlurState>,
    before_usm: RasterCommands,
    usm_blur: Option<GrainBlurState>,
    after: RasterCommands,
}

pub(in crate::wgpu_backend) fn build_grain_post(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    density: &(impl RasterStorage + ?Sized),
    params: &SpektraGrainGpuParams,
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
    params: &SpektraGrainGpuParams,
    limit: u64,
) -> Result<GrainPostState, String> {
    if !backend
        .device
        .features()
        .contains(wgpu::Features::SHADER_F64)
    {
        return Err("grain finishing requires native shader f64 arithmetic".into());
    }
    let window_limit = super::transport_window_limit(limit);
    let n = width
        .checked_mul(height)
        .filter(|&n| n > 0 && n <= u32::MAX / 3)
        .ok_or("grain post dimensions are not representable")?;
    let bytes = u64::from(n) * 12;
    let chunk = (window_limit / 12).min(u64::from(n)) as u32;
    if chunk == 0 {
        return Err("grain post minimum RGB binding needs 12 bytes".into());
    }
    let source_window = transport(backend, u64::from(chunk) * 12, "grain_post_source_window")?;
    let output_window = transport(backend, u64::from(chunk) * 12, "grain_post_output_window")?;
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
        minimum: [f64; 4],
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
                        params.density_min[0],
                        params.density_min[if params.monochrome { 0 } else { 1 }],
                        params.density_min[if params.monochrome { 0 } else { 2 }],
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
    let blur_active = params.grain_blur > 0.0;
    let usm = params.usm_sigma > 0.0 && params.usm_amount > 0.0;
    let mut grain_blur = None;
    let density = density.pages();
    let mut source = density.clone();
    if blur_active {
        let filtered = paged_transport(backend, bytes, "grain_post_filtered_transport")?;
        grain_blur = Some(build_grain_blur_paged(
            backend,
            width,
            height,
            &density,
            &filtered,
            [params.grain_blur; 3],
            0,
            0,
        )?);
        source = filtered;
    }
    let mut usm_blur = None;
    let mut before_usm = RasterCommands::default();
    if usm {
        if window_limit < 192 {
            return Err("grain USM pairwise accumulators require a 192-byte binding".into());
        }
        // Upstream blurs D=max(image,0), not the unclamped source. Reuse the
        // existing windows; source is job-owned and no longer needed unclamped.
        for first in (0..n).step_by(chunk as usize) {
            let count = chunk.min(n - first);
            let p = uniform(
                backend,
                &FloorParams {
                    dimensions: [count, 1, 0, 0],
                    minimum: [0.0; 4],
                },
            );
            before_usm.copy(
                &source,
                u64::from(first) * 12,
                &source_window,
                0,
                u64::from(count) * 12,
            );
            before_usm.dispatch(
                &floor_pipe,
                &[&p, &source_window, &output_window],
                count,
                false,
                backend,
            );
            before_usm.copy(
                &output_window,
                0,
                &source,
                u64::from(first) * 12,
                u64::from(count) * 12,
            );
        }
        let blurred = paged_transport(backend, bytes, "grain_usm_blurred_transport")?;
        usm_blur = Some(build_grain_blur_paged(
            backend,
            width,
            height,
            &source,
            &blurred,
            [params.usm_sigma; 3],
            0,
            0,
        )?);
        let sharpened = paged_transport(backend, bytes, "grain_usm_sharpened_transport")?;
        let blur_window = transport(backend, u64::from(chunk) * 12, "grain_usm_blur_window")?;
        let sharpen_pipe = backend.cached_pipeline(
            SHARPEN_SHADER,
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct SharpenParams {
            dimensions: [u32; 4],
            controls: [f32; 4],
        }
        for first in (0..n).step_by(chunk as usize) {
            let count = chunk.min(n - first);
            let p = uniform(
                backend,
                &SharpenParams {
                    dimensions: [count, 0, 0, 0],
                    controls: [
                        params.usm_amount as f32,
                        (1.0 + params.usm_amount) as f32,
                        0.0,
                        0.0,
                    ],
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
                &[&p, &source_window, &blur_window, &output_window],
                count,
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
        let pair = transport(backend, 24, "grain_usm_mass")?;
        let lanes = transport(backend, 192, "grain_usm_pairwise_lanes")?;
        let stack = paged_transport(
            backend,
            u64::from(n.ilog2() + 1) * 24,
            "grain_usm_pairwise_stack",
        )?;
        let reduce_pipe = backend.cached_pipeline(
            include_str!("post_reduce.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        MassReduction {
            backend,
            commands: &mut after,
            source: &source,
            sharpened: &sharpened,
            source_window: &source_window,
            sharpen_window: &blur_window,
            lanes: &lanes,
            pair: &pair,
            stack: &stack,
            pipeline: &reduce_pipe,
            chunk,
        }
        .emit(0, n, 0);
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
    } else {
        floor(&mut after, &source, &density);
    }
    Ok(GrainPostState {
        grain_blur,
        before_usm,
        usm_blur,
        after,
    })
}

/// Emit the global NumPy pairwise tree, not a tree over transport chunks.
/// Only logarithmically many left-child results are retained; a leaf's eight
/// accumulators carry across arbitrarily placed storage windows.
struct MassReduction<'a> {
    backend: &'a WgpuBackend,
    commands: &'a mut RasterCommands,
    source: &'a dyn RasterStorage,
    sharpened: &'a dyn RasterStorage,
    source_window: &'a wgpu::Buffer,
    sharpen_window: &'a wgpu::Buffer,
    lanes: &'a wgpu::Buffer,
    pair: &'a wgpu::Buffer,
    stack: &'a dyn RasterStorage,
    pipeline: &'a CachedPipelineRef,
    chunk: u32,
}

impl MassReduction<'_> {
    fn emit(&mut self, first: u32, count: u32, depth: u32) {
        if count > 128 {
            let midpoint = (count / 2) & !7;
            self.emit(first, midpoint, depth + 1);
            self.commands
                .copy(self.pair, 0, self.stack, u64::from(depth) * 24, 24);
            self.emit(first + midpoint, count - midpoint, depth + 1);
            self.commands
                .copy(self.stack, u64::from(depth) * 24, self.source_window, 0, 24);
            let p = uniform(self.backend, &[0u32, 0, 0, 1]);
            self.commands.dispatch(
                self.pipeline,
                &[
                    &p,
                    self.source_window,
                    self.sharpen_window,
                    self.lanes,
                    self.pair,
                ],
                3,
                false,
                self.backend,
            );
        } else {
            for offset in (0..count).step_by(self.chunk as usize) {
                let size = self.chunk.min(count - offset);
                self.commands.copy(
                    self.source,
                    u64::from(first + offset) * 12,
                    self.source_window,
                    0,
                    u64::from(size) * 12,
                );
                self.commands.copy(
                    self.sharpened,
                    u64::from(first + offset) * 12,
                    self.sharpen_window,
                    0,
                    u64::from(size) * 12,
                );
                let p = uniform(self.backend, &[size, offset, count, 0]);
                self.commands.dispatch(
                    self.pipeline,
                    &[
                        &p,
                        self.source_window,
                        self.sharpen_window,
                        self.lanes,
                        self.pair,
                    ],
                    3,
                    false,
                    self.backend,
                );
            }
        }
    }
}

impl GrainPostState {
    pub(in crate::wgpu_backend) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        if let Some(blur) = &self.grain_blur {
            blur.submit(backend)?;
        }
        self.before_usm.submit(backend)?;
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
    // output mass must return to its input mass. The original array reductions,
    // ratio and output products are f32; the relative allowance is 1e-5.
    const MAX_RELATIVE_MASS_ERROR: f64 = 1e-5;

    fn params() -> SpektraGrainGpuParams {
        SpektraGrainGpuParams {
            base_seed: 7,
            layer_count: 3,
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
            mt19937_regime: false,
        }
    }

    fn run(
        gpu: &WgpuBackend,
        width: u32,
        height: u32,
        input: &[f32],
        p: &SpektraGrainGpuParams,
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
        let mut p = params();
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
        for limit in [192, 1560] {
            gpu.queue
                .write_buffer(&density, 0, bytemuck::cast_slice(&input));
            let state =
                build_grain_post_with_limit(&gpu, width, height, &density, &p, limit).unwrap();
            state.submit(&gpu).unwrap();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&density, 0, &readback, 0, bytes);
            gpu.queue.submit(Some(encoder.finish()));
            assert_eq!(
                expected,
                gpu.read_mapped_f32(&readback, bytes),
                "window {limit} changed complete-source USM mass"
            );
        }
    }

    fn sum_f32(values: &[f32]) -> f32 {
        if values.len() < 8 {
            values.iter().fold(-0.0, |sum, &value| sum + value)
        } else if values.len() <= 128 {
            let mut sums: [f32; 8] = values[..8].try_into().unwrap();
            let end = values.len() - values.len() % 8;
            for chunk in values[8..end].as_chunks::<8>().0 {
                for index in 0..8 {
                    sums[index] += chunk[index];
                }
            }
            let mut sum = ((sums[0] + sums[1]) + (sums[2] + sums[3]))
                + ((sums[4] + sums[5]) + (sums[6] + sums[7]));
            for &value in &values[end..] {
                sum += value;
            }
            sum
        } else {
            let midpoint = (values.len() / 2) & !7;
            sum_f32(&values[..midpoint]) + sum_f32(&values[midpoint..])
        }
    }

    fn reference_unsharp(image: &ImageBuf, sigma: f64, amount: f64) -> ImageBuf {
        let mut nonnegative = image.clone();
        for value in &mut nonnegative.data {
            *value = value.max(from_f32(0.0));
        }
        let image = &nonnegative;
        let blurred = spektrafilm_math::gaussian::gaussian_blur(image, sigma);
        let mut out = image.clone();
        let mut sums = [0.0f32; 3];
        let mut out_sums = [0.0f32; 3];
        for (i, px) in image.pixels().enumerate() {
            let out_px = &mut out.data[i * 3..i * 3 + 3];
            let blur_px = &blurred.data[i * 3..i * 3 + 3];
            for ch in 0..3 {
                let density = to_f32(px[ch]).max(0.0);
                let blur_density = to_f32(blur_px[ch]).max(1e-6);
                let sharpened = if density <= 0.0 {
                    0.0
                } else {
                    density.max(1e-6).powf((1.0 + amount) as f32) / blur_density.powf(amount as f32)
                };
                out_px[ch] = from_f32(sharpened);
            }
        }
        for ch in 0..3 {
            sums[ch] = sum_f32(
                &image
                    .pixels()
                    .map(|px| to_f32(px[ch]).max(0.0))
                    .collect::<Vec<_>>(),
            );
            out_sums[ch] = sum_f32(&out.pixels().map(|px| to_f32(px[ch])).collect::<Vec<_>>());
        }
        for px in out.pixels_mut() {
            for ch in 0..3 {
                if out_sums[ch] > 0.0 {
                    px[ch] = from_f32(to_f32(px[ch]) * (sums[ch] / out_sums[ch]));
                }
            }
        }
        out
    }

    /// The layered reference tail after composition: optional `grain_blur`,
    /// then multiplicative USM, then the floor subtraction.
    fn reference_layered(
        input: &[f32],
        width: u32,
        height: u32,
        p: &SpektraGrainGpuParams,
    ) -> Vec<f32> {
        let mut image =
            ImageBuf::from_data(width, height, input.iter().map(|&v| from_f32(v)).collect());
        if p.grain_blur > 0.0 {
            image = spektrafilm_math::gaussian::gaussian_blur(&image, p.grain_blur);
        }
        if p.usm_sigma > 0.0 && p.usm_amount > 0.0 {
            image = reference_unsharp(&image, p.usm_sigma, p.usm_amount);
        }
        for px in image.pixels_mut() {
            for (channel, sample) in px.iter_mut().enumerate() {
                *sample = from_f64(
                    to_f64(*sample) - p.density_min[if p.monochrome { 0 } else { channel }],
                );
            }
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
        p: &SpektraGrainGpuParams,
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
        let mut p = params();
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
        let mut p = params();
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
        let p = params();
        let actual = run(&gpu, width, height, &input, &p);
        let expected: Vec<f32> = input
            .iter()
            .enumerate()
            .map(|(i, &v)| (f64::from(v) - p.density_min[i % 3]) as f32)
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
        let mut p = params();
        p.usm_sigma = 1.1;
        p.usm_amount = 1.4;
        let actual = run(&gpu, width, height, &input, &p);
        // `out_sums[c] == 0` takes the reference's no-rescale branch, then the
        // floor is removed in f64, then narrowed to f32.
        let expected: Vec<f32> = (0..(width * height) as usize * 3)
            .map(|i| (0.0f64 - p.density_min[i % 3]) as f32)
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn positive_subepsilon_density_uses_clamped_usm_numerator() {
        let gpu = test_support::adapter();
        let (width, height) = (7, 1);
        let input: Vec<f32> = [0.0, 1e-9, 3e-8, 5e-7, 9e-7, 2e-8, 0.0]
            .into_iter()
            .flat_map(|value| [value; 3])
            .collect();
        let mut p = params();
        p.density_min = [0.0; 3];
        p.usm_sigma = 0.7;
        p.usm_amount = 1.5;
        let expected = reference_layered(&input, width, height, &p);
        let actual = run(&gpu, width, height, &input, &p);
        for (index, (&got, &want)) in actual.iter().zip(&expected).enumerate() {
            assert!(
                (f64::from(got) - f64::from(want)).abs() <= f64::from(want.abs()) * 2e-5 + 1e-15,
                "subepsilon sample {index}: {got} != {want}"
            );
        }
        assert_eq!(actual[0], 0.0);
        assert_eq!(
            actual[3], actual[6],
            "positive values below epsilon share the clamped numerator"
        );
    }

    #[test]
    fn monochrome_finishing_replicates_channel_zero_floor_with_and_without_usm() {
        let gpu = test_support::adapter();
        let (width, height) = (17, 1);
        let input: Vec<f32> = (0..width)
            .flat_map(|i| [0.2 + i as f32 * 0.03; 3])
            .collect();
        let mut p = params();
        p.monochrome = true;
        p.density_min = [0.03, 0.5, 0.9];
        for sharpen in [false, true] {
            p.usm_sigma = if sharpen { 0.7 } else { 0.0 };
            p.usm_amount = if sharpen { 1.5 } else { 0.0 };
            let expected = reference_layered(&input, width, height, &p);
            check_parity(
                &gpu,
                width,
                height,
                &input,
                &p,
                &expected,
                "monochrome floor",
            );
            for pixel in run(&gpu, width, height, &input, &p).as_chunks::<3>().0 {
                assert_eq!(pixel[0], pixel[1]);
                assert_eq!(pixel[0], pixel[2]);
            }
        }
    }

    #[test]
    fn density_floor_is_subtracted_before_binary32_narrowing() {
        let gpu = test_support::adapter();
        let mut p = params();
        p.density_min = [1.0 + 2.0f64.powi(-25); 3];
        let actual = run(&gpu, 1, 1, &[1.0; 3], &p);
        assert_eq!(actual, vec![-2.0f32.powi(-25); 3]);
    }

    #[test]
    fn unsharp_adds_one_before_weak_scalar_exponent_narrowing() {
        let gpu = test_support::adapter();
        let mut p = params();
        p.density_min = [0.0; 3];
        p.usm_sigma = 0.7;
        p.usm_amount = 0.5 + 2.0f64.powi(-24) + 2.0f64.powi(-27);
        assert_ne!((1.0 + p.usm_amount) as f32, 1.0f32 + p.usm_amount as f32);
        let input: Vec<f32> = [1e-5, 1e-4, 1e-3, 0.01, 0.1, 1.0]
            .into_iter()
            .flat_map(|value| [value; 3])
            .collect();
        let expected = reference_layered(&input, 6, 1, &p);
        let actual = run(&gpu, 6, 1, &input, &p);
        for (index, (&got, &want)) in actual.iter().zip(&expected).enumerate() {
            assert!(
                (f64::from(got) - f64::from(want)).abs() <= f64::from(want.abs()) * 4e-7 + 1e-16,
                "weak-scalar exponent sample {index}: {got} != {want}"
            );
        }
    }

    #[test]
    fn bounded_mass_reduction_matches_global_pairwise_tree_bit_for_bit() {
        let gpu = test_support::adapter();
        let pipeline = gpu.cached_pipeline(
            include_str!("post_reduce.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        for n in [1u32, 7, 8, 31, 128, 129, 256, 4096] {
            let values: Vec<f32> = (0..n)
                .flat_map(|index| {
                    [
                        if index % 8 == 0 { 1.0 } else { 1e-7 },
                        (index % 7) as f32 * 0.3 + 1e-8,
                        if index % 129 == 0 { 1000.0 } else { 1e-5 },
                    ]
                })
                .collect();
            let sharpened: Vec<f32> = values.iter().map(|&value| value * value).collect();
            let image = gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("pairwise_mass_source"),
                    contents: bytemuck::cast_slice(&values),
                    usage: wgpu::BufferUsages::COPY_SRC,
                });
            let output = gpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("pairwise_mass_sharpened"),
                    contents: bytemuck::cast_slice(&sharpened),
                    usage: wgpu::BufferUsages::COPY_SRC,
                });
            for window in [192u64, 1560] {
                let source_window = transport(&gpu, window, "pairwise_test_source_window").unwrap();
                let sharpen_window =
                    transport(&gpu, window, "pairwise_test_sharpen_window").unwrap();
                let lanes = transport(&gpu, 192, "pairwise_test_lanes").unwrap();
                let pair = transport(&gpu, 24, "pairwise_test_mass").unwrap();
                let stack =
                    paged_transport(&gpu, u64::from(n.ilog2() + 1) * 24, "pairwise_test_stack")
                        .unwrap();
                let mut commands = RasterCommands::default();
                MassReduction {
                    backend: &gpu,
                    commands: &mut commands,
                    source: &image,
                    sharpened: &output,
                    source_window: &source_window,
                    sharpen_window: &sharpen_window,
                    lanes: &lanes,
                    pair: &pair,
                    stack: &stack,
                    pipeline: &pipeline,
                    chunk: (window / 12) as u32,
                }
                .emit(0, n, 0);
                commands.submit(&gpu).unwrap();
                let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("pairwise_test_readback"),
                    size: 24,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                encoder.copy_buffer_to_buffer(&pair, 0, &readback, 0, 24);
                gpu.queue.submit(Some(encoder.finish()));
                let actual: Vec<f32> = gpu.pipe.read_mapped(&readback, 24);
                for (plane, input) in [&values, &sharpened].into_iter().enumerate() {
                    for channel in 0..3 {
                        let expected = sum_f32(
                            &input
                                .as_chunks::<3>()
                                .0
                                .iter()
                                .map(|pixel| pixel[channel])
                                .collect::<Vec<_>>(),
                        );
                        assert_eq!(
                            actual[plane * 3 + channel].to_bits(),
                            expected.to_bits(),
                            "pairwise mass n={n}, window={window}, plane={plane}, channel={channel}"
                        );
                    }
                }
            }
        }
    }
}
