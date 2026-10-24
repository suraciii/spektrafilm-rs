//! Lensfun's forward rectilinear modifier on device-resident oriented RGB.
use super::*;
use spektrafilm_backend::RawLensParams;

// A local ABI wrapper avoids making backend metadata depend on bytemuck.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct LensUniform {
    dimensions: [u32; 4],
    flags: [u32; 4],
    normalization: [f32; 4],
    center: [f32; 4],
    distortion: [f32; 4],
    tca: [f32; 4],
    tca_tail: [f32; 4],
    vignette: [f32; 4],
    mapping: [f32; 4],
}
impl From<&RawLensParams> for LensUniform {
    fn from(p: &RawLensParams) -> Self {
        Self {
            dimensions: p.dimensions, flags: p.flags, normalization: p.normalization,
            center: p.center, distortion: p.distortion, tca: p.tca, tca_tail: p.tca_tail,
            vignette: p.vignette, mapping: p.mapping,
        }
    }
}
impl WgpuBackend {
    /// Encode the returned passes in order, after orientation and before CAT02
    /// adaptation/tint. Input and output must be distinct width*height*3 f32
    /// buffers at the prepared dimensions. Bound resources live with dispatches.
    pub(super) fn prepare_lens_passes(
        &self, params: &RawLensParams, input: &RawBuffer, output: &RawBuffer,
    ) -> Vec<RawDispatch> {
        let width = params.dimensions[0];
        let height = params.dimensions[1];
        let uniform = RawBuffer::upload(&self.device, "raw_lens_params",
            bytemuck::bytes_of(&LensUniform::from(params)), wgpu::BufferUsages::UNIFORM);
        let axes = RawBuffer::zeros(&self.device, "raw_lens_axes", (u64::from(width) + u64::from(height)) * 4);
        let vignette = RawBuffer::zeros(&self.device, "raw_lens_vignette", u64::from(width) * u64::from(height) * 12);
        let storage_in = wgpu::BufferBindingType::Storage { read_only: true };
        let storage_out = wgpu::BufferBindingType::Storage { read_only: false };
        vec![
            self.raw_dispatch(AXES, &[&uniform, &axes], &[wgpu::BufferBindingType::Uniform, storage_out], 2, "raw_lens_axes"),
            self.raw_dispatch(VIGNETTE, &[&uniform, &axes, input, &vignette],
                &[wgpu::BufferBindingType::Uniform, storage_in, storage_in, storage_out], height, "raw_lens_vignette"),
            self.raw_dispatch(RESAMPLE, &[&uniform, &axes, &vignette, output],
                &[wgpu::BufferBindingType::Uniform, storage_in, storage_in, storage_out], width * height, "raw_lens_resample"),
        ]
    }
}
const AXES: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_lens_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_lens_axes.wgsl"),
);
const VIGNETTE: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_lens_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_lens_vignette.wgsl"),
);
const RESAMPLE: &str = concat!(
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_lens_common.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_lens_mapping.wgsl"),
    include_str!("../../../../spektrafilm-shaders/wgsl/raw/raw_lens_resample.wgsl"),
);

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(width: u32, height: u32) -> RawLensParams {
        RawLensParams {
            dimensions: [width, height, 0, 0], flags: [1, 0, 0, 0],
            normalization: [1.0, 0.0, 1.0, 0.0], center: [0.0; 4],
            distortion: [0.0; 4], tca: [1.0, 1.0, 0.0, 0.0], tca_tail: [0.0; 4],
            vignette: [0.0; 4], mapping: [1.0, 1.0, 1.0, 1.0],
        }
    }
    fn apply(params: &RawLensParams, source: &[f32]) -> Vec<f32> {
        let gpu = crate::test_support::adapter();
        let input = RawBuffer::upload(&gpu.device, "lens_test_input", bytemuck::cast_slice(source), wgpu::BufferUsages::STORAGE);
        let output = RawBuffer::zeros(&gpu.device, "lens_test_output", source.len() as u64 * 4);
        let passes = gpu.prepare_lens_passes(params, &input, &output);
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("lens_regression") });
        for pass in &passes { pass.encode(&mut encoder, "lens_regression"); }
        gpu.queue.submit(Some(encoder.finish()));
        gpu.pipe.readback_buffer_as::<f32>(&output.buffer, source.len() as u64 * 4)
    }
    #[test]
    fn channel_tca_precedes_nearest_edge_bilinear_resampling() {
        let mut params = plan(3, 2);
        params.dimensions[3] = 1;
        params.tca = [0.5, 2.0, 0.0, 0.0];
        let source = [0.0, 10.0, 20.0, 2.0, 12.0, 22.0, 4.0, 14.0, 24.0,
                      6.0, 16.0, 26.0, 8.0, 18.0, 28.0, 10.0, 20.0, 30.0];
        let result = apply(&params, &source);
        let expected = [0.0, 10.0, 20.0, 1.0, 12.0, 24.0, 2.0, 14.0, 24.0,
                        3.0, 16.0, 26.0, 4.0, 18.0, 30.0, 5.0, 20.0, 30.0];
        assert_eq!(result, expected);
    }
    #[test]
    fn vignette_runs_on_source_pixels_before_geometry() {
        let mut params = plan(3, 1);
        params.flags[2] = 1;
        params.vignette = [1.0, 0.0, 0.0, 1.0];
        params.mapping[3] = 0.5;
        let result = apply(&params, &[1.0; 9]);
        // De-vignetting source is [1, 1/2, 1/5]; sampling x=[0,.5,1].
        let expected = [1.0, 1.0, 1.0, 0.75, 0.75, 0.75, 0.5, 0.5, 0.5];
        assert_eq!(result, expected);
    }
    #[test]
    fn full_frame_axes_round_every_addition_instead_of_multiplying_index() {
        let mut params = plan(4096, 1);
        let step = 0.000_491_314_271_9_f64;
        params.normalization[0] = step as f32;
        params.normalization[1] = (step - f64::from(step as f32)) as f32;
        params.normalization[2] = 1000.0;
        let source: Vec<f32> = (0..4096).flat_map(|x| [x as f32; 3]).collect();
        let result = apply(&params, &source);
        let mut x = 0.0f32;
        for i in 0..4096 {
            let expected = (f64::from(x) * 1000.0) as f32;
            assert!((result[i * 3] - expected).abs() <= 0.0003, "pixel {i}");
            x = (f64::from(x) + step) as f32;
        }
        assert!((x - (4096.0 * step) as f32).abs() > 1e-5);
    }

    #[test]
    fn forward_distortion_models_follow_lensfun_polynomials() {
        let source: Vec<f32> = (0..64).flat_map(|x| [x as f32; 3]).collect();
        for model in 1..=3 {
            let mut params = plan(64, 1);
            params.dimensions[2] = model;
            params.distortion = [0.002, 0.00003, 0.0004, 0.0];
            params.normalization[0] = 0.125;
            params.normalization[2] = 8.0;
            let result = apply(&params, &source);
            for i in 0..64 {
                let x = i as f32 * 0.125;
                let r2 = x * x;
                let [a, b, c, _] = params.distortion;
                let polynomial = match model {
                    1 => (1.0 - a) + a * r2,
                    2 => (1.0 + f64::from(a * r2) + f64::from(b * r2 * r2)) as f32,
                    _ => {
                        let d = (1.0 - f64::from(a) - f64::from(b) - f64::from(c)) as f32;
                        a * r2 * x + b * r2 + c * x + d
                    }
                };
                let expected = (x * polynomial * 8.0).clamp(0.0, 63.0);
                assert!((result[i * 3] - expected).abs() < 2e-5, "model {model}, pixel {i}");
            }
        }
    }

    #[test]
    fn combined_native_simd_preserves_channel_lanes_unaligned_rows_and_scalar_tails() {
        // a=c=0 in PTLens makes native SSE's approximate sqrt irrelevant.
        // An affine source makes each sampled coordinate observable, including
        // independent x/y lane scales, through the real resampling consumer.
        for width in [6u32, 7] {
            for model in [1u32, 3] {
                let mut params = plan(width, 3);
                params.flags[3] = 1;
                params.dimensions[2] = model;
                params.normalization = [0.25, 0.0, 4.0, 0.0];
                params.distortion[if model == 1 { 0 } else { 1 }] = 0.125;
                let source: Vec<f32> = (0..3).flat_map(|y| {
                    (0..width).flat_map(move |x| [x as f32 + 10.0 * y as f32; 3])
                }).collect();
                let result = apply(&params, &source);
                for row in 0..3u32 {
                    for col in 0..width {
                        for channel in 0..3u32 {
                            let pair = col * 3 + channel;
                            let simd = (row * width) % 2 == 0 && pair < width * 3 / 4 * 4;
                            let first = pair / 4 * 4 + pair % 2 * 2;
                            let sx = if simd { first / 3 } else { col };
                            let sy = if simd { (first + 1) / 3 } else { col };
                            let scale = |x: u32| {
                                let x = x as f32 * 0.25;
                                let y = row as f32 * 0.25;
                                0.875 + 0.125 * (x * x + y * y)
                            };
                            let x = (col as f32 * scale(sx)).clamp(0.0, (width - 1) as f32);
                            let y = (row as f32 * scale(sy)).clamp(0.0, 2.0);
                            let index = ((row * width + col) * 3 + channel) as usize;
                            assert!((result[index] - (x + 10.0 * y)).abs() < 2e-5,
                                "width {width}, model {model}, ({col},{row}), channel {channel}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_lensfun_projection_has_the_rectilinear_inverse_mapping() {
        let width = 5;
        let height = 3;
        let source: Vec<f32> = (0..height).flat_map(|y|
            (0..width).flat_map(move |x| [x as f32, y as f32, (x + y) as f32])).collect();
        for kind in 2..=8 {
            let mut params = plan(width, height);
            params.flags = [kind, 1, 0, 0];
            params.normalization = [0.25, 0.0, 4.0, 0.0];
            params.center = [0.5, 0.0, 0.25, 0.0];
            let result = apply(&params, &source);
            for y in 0..height {
                for x in 0..width {
                    let px = f64::from(x) * 0.25 - 0.5;
                    let py = f64::from(y) * 0.25 - 0.25;
                    let radius = px.hypot(py);
                    let theta = radius.atan();
                    let radial = match kind {
                        2 => if radius == 0.0 { 1.0 } else { theta / radius },
                        5 => if radius == 0.0 { 1.0 } else { theta.sin() / radius },
                        6 => if radius == 0.0 { 1.0 } else { 2.0 * (theta / 2.0).tan() / radius },
                        7 => if radius == 0.0 { 1.0 } else { 2.0 * (theta / 2.0).sin() / radius },
                        8 => if radius == 0.0 { 1.47 * 0.713 } else { 1.47 * (theta * 0.713).sin() / radius },
                        _ => 1.0,
                    };
                    let (sx, sy) = match kind {
                        3 => (px.atan(), py * px.atan().cos()),
                        4 => (px.atan2(1.0), py.atan2((1.0 + px * px).sqrt())),
                        _ => (px * radial, py * radial),
                    };
                    let expected_x = ((sx + 0.5) * 4.0).clamp(0.0, f64::from(width - 1));
                    let expected_y = ((sy + 0.25) * 4.0).clamp(0.0, f64::from(height - 1));
                    let index = ((y * width + x) * 3) as usize;
                    assert!((f64::from(result[index]) - expected_x).abs() < 3e-5, "projection {kind} x");
                    assert!((f64::from(result[index + 1]) - expected_y).abs() < 3e-5, "projection {kind} y");
                }
            }
        }
    }

    #[test]
    fn poly3_tca_uses_separate_channel_terms_including_the_radius_term() {
        let mut params = plan(5, 1);
        params.dimensions[3] = 2;
        params.tca = [0.5, 0.75, 0.125, 0.25];
        params.tca_tail = [0.0625, 0.03125, 0.0, 0.0];
        let source: Vec<f32> = (0..5).flat_map(|x| [x as f32; 3]).collect();
        let result = apply(&params, &source);
        for i in 0..5 {
            let x = i as f32;
            let red = (x * (0.0625 * x * x + 0.125 * x + 0.5)).min(4.0);
            let blue = (x * (0.03125 * x * x + 0.25 * x + 0.75)).min(4.0);
            assert_eq!(&result[i * 3..i * 3 + 3], &[red, x, blue]);
        }
    }
}
