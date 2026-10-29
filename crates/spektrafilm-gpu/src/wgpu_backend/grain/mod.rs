use super::*;

pub mod dye;
mod dye_law;
mod dye_law_paged;
pub(in crate::wgpu_backend) mod fotufilm;
pub(in crate::wgpu_backend) mod mt19937;
pub(in crate::wgpu_backend) mod raster;
pub mod v1;
pub(in crate::wgpu_backend) use raster::RasterStorage;
pub(in crate::wgpu_backend) use v1::{GRAIN_V1_STAGE_NAME, build_spektra_grain_state};

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ShaderParams {
    dimensions: [u32; 4],
    controls: [f32; 4],
    geometry: [f32; 4],
    flags: [f32; 4],
    window: [u32; 4],
    canvas: [u32; 4],
    generator_geometry: [f32; 4],
}

fn shader_params(
    width: u32,
    height: u32,
    params: &spektrafilm_backend::GrainV2GpuParams,
) -> ShaderParams {
    let timer = params
        .timer
        .unwrap_or_else(|| spektrafilm_math::grain::seeded_phase(params.seed));
    let gsf = (5200.0f32 / width.max(1) as f32).max(3100.0f32 / height.max(1) as f32);
    let size = [(width as f32 * gsf).floor(), (height as f32 * gsf).floor()];
    let scale = params.raw_scale.clamp(0.5, 1.4);
    let scaled = size.map(|v| v * scale);
    // Image-independent generator coefficients retain CPU stepwise f32
    // division. Native Vulkan division can change the rnm hash lattice.
    let generator_geometry = [
        scaled[0] / params.cluster_size / scale,
        scaled[1] / params.cluster_size / scale,
        scaled[0] / scaled[1],
        1.0f32 / 256.0 / params.cluster_size,
    ];
    ShaderParams {
        dimensions: [width, height, timer.to_bits(), params.mode],
        controls: [
            params.amount,
            params.shadows,
            params.midtones,
            params.highlights,
        ],
        geometry: [
            params.raw_scale,
            params.cluster_size,
            params.rotation,
            params.color,
        ],
        flags: [
            params.resolution_factor,
            params.resolution_type as f32,
            params.colored as u32 as f32,
            params.clustered as u32 as f32,
        ],
        window: [0, width * height, 0, 0],
        canvas: [0; 4],
        generator_geometry,
    }
}

impl WgpuBackend {
    pub fn grain_v2_gpu(
        &self,
        img: &ImageBuf,
        params: &spektrafilm_backend::GrainV2GpuParams,
    ) -> Result<ImageBuf, String> {
        if !self.device.context.enabled() || img.width == 0 || img.height == 0 {
            return self.grain_v2_gpu_inner(img, params);
        }
        let (backend, _batch) = self.observed_batch("grain_v2");
        backend.grain_v2_gpu_inner(img, params)
    }
    /// Execute Grain Procedural on native encoded RGB with no transfer or primaries conversion.
    fn grain_v2_gpu_inner(
        &self,
        img: &ImageBuf,
        params: &spektrafilm_backend::GrainV2GpuParams,
    ) -> Result<ImageBuf, String> {
        if img.width == 0 || img.height == 0 {
            return Ok(img.clone());
        }
        procedural::validate_coordinate_scale(params)?;
        let input = scalars_to_f32(&img.data);
        let original = self.allocate_image_pages(img.width, img.height, "grain_encoded_input")?;
        for page in &original {
            let first = page.first_pixel as usize * 3;
            let end = first + page.n_pixels as usize * 3;
            self.queue
                .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&input[first..end]));
        }
        let state = build_grain_v2_state(
            &self.device,
            params,
            img.width,
            img.height,
            &original,
            None,
            self,
        )?;
        let output_pages = self.allocate_image_pages(img.width, img.height, "grain_output")?;
        state.submit(self, &output_pages)?;
        let mut output = Vec::with_capacity(input.len());
        for page in &output_pages {
            output.extend(
                self.pipe.readback_buffer_cancellable::<f32>(
                    &page.buffer,
                    u64::from(page.n_pixels) * 12,
                )?,
            );
        }
        Ok(ImageBuf::from_data(
            img.width,
            img.height,
            f32_to_scalars(output),
        ))
    }

    /// Run only the destination transfer encode pass over `image`.
    ///
    /// The grain path encodes the native destination before composing, so the
    /// transfer is the one place a signed-headroom mistake poisons the whole
    /// composition. Exposing the single pass lets a test compare it against
    /// the reference transfer directly instead of inferring it from a
    /// stochastic grain result.
    #[cfg(test)]
    pub(crate) fn encode_in_space(
        &self,
        image: &ImageBuf,
        cctf: spektrafilm_math::colorspace::Cctf,
    ) -> ImageBuf {
        let n_bytes = image.pixel_count() as u64 * 3 * 4;
        let input = scalars_to_f32(&image.data);
        let src = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("encode_space_src"),
                contents: bytemuck::cast_slice(input.as_ref()),
                usage: wgpu::BufferUsages::STORAGE,
            });
        // The pass writes storage; a separate mappable buffer receives the
        // copy so the readback uses the shared protocol.
        let dst = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("encode_space_dst"),
            size: n_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("encode_space_readback"),
            size: n_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct EncodeParams {
            dimensions: [u32; 4],
            taps: [u32; 4],
            matrix: [[f32; 4]; 3],
            window: [u32; 4],
        }
        let params = EncodeParams {
            dimensions: [image.width, image.height, 2, 0],
            taps: [0, cctf.code(), 0, 0],
            matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            window: [0, image.width * image.height, 0, 0],
        };
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("encode_space_params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let weights = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("encode_space_weights"),
                contents: bytemuck::cast_slice(&[0.0f32, 0.0f32]),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let pipe = self.cached_pipeline(
            include_str!("film_resolution.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bg_encode_space"),
            layout: &pipe.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: src.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: weights.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: dst.as_entire_binding(),
                },
            ],
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("encode_space"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipe.pipeline);
            pass.set_bind_group(0, &bg, &[]);
            dispatch_linear(&mut pass, image.pixel_count() as u32);
        }
        encoder.copy_buffer_to_buffer(&dst, 0, &readback, 0, n_bytes);
        self.queue.submit(Some(encoder.finish()));
        let output = self.read_mapped_f32(&readback, n_bytes);
        ImageBuf::from_data(image.width, image.height, f32_to_scalars(output))
    }
}

mod procedural;
pub(super) use procedural::build_grain_v2_state;

#[cfg(test)]
mod tests {
    #[test]
    fn destination_transfers_match_cpu_across_toes_and_headroom() {
        use spektrafilm_math::colorspace::{Cctf, cctf_encode};
        use spektrafilm_math::precision::{from_f64, to_f64};

        let backend = crate::test_support::adapter();
        let mut samples = vec![-1.5, -0.25, -1e-6, 0.0, 1e-6, 0.18, 0.5, 1.0, 1.5, 4.0];
        for threshold in [1.0 / 512.0, 0.0031308, 0.0078125, 0.018] {
            samples.extend([threshold - 1e-6, threshold, threshold + 1e-6]);
        }
        let image = spektrafilm_math::image::ImageBuf::from_data(
            1,
            samples.len() as u32,
            samples
                .iter()
                .flat_map(|&v| [from_f64(v), from_f64(v), from_f64(v)])
                .collect(),
        );
        for cctf in [
            Cctf::Linear,
            Cctf::Acescct,
            Cctf::Srgb,
            Cctf::ProPhoto,
            Cctf::Rec2020,
        ] {
            let encoded = backend.encode_in_space(&image, cctf);
            for (i, &v) in samples.iter().enumerate() {
                let reference = cctf_encode(v, cctf);
                // The shader rounds to binary16; allow its relative rounding
                // error plus a small absolute margin for subnormals and f32 math.
                let tolerance = reference.abs() * 0.0006 + 1e-6;
                for channel in 0..3 {
                    let got = to_f64(encoded.data[i * 3 + channel]);
                    assert!(
                        got.is_finite() && (got - reference).abs() <= tolerance,
                        "{cctf:?} at {v}, channel {channel}: device {got} != reference {reference}"
                    );
                }
            }
        }
    }

    /// The pure-power destination transfers must preserve signed headroom,
    /// exactly like `spektrafilm_math::colorspace::cctf_encode` (which uses
    /// `spow`). Returning NaN for a negative sample poisons the whole grain
    /// composition for an Adobe RGB or DCI-P3 output, so this is pinned by
    /// running the real kernel over signed samples rather than by matching
    /// shader text.
    #[test]
    fn pure_power_destination_transfers_preserve_signed_headroom() {
        use spektrafilm_math::colorspace::{Cctf, cctf_encode};
        use spektrafilm_math::precision::{from_f64, to_f64};
        let backend = crate::test_support::adapter();
        let samples = [-1.5f64, -0.25, -1e-6, 0.0, 1e-6, 0.25, 0.5, 1.5];
        for cctf in [Cctf::AdobeRgb1998, Cctf::Gamma2_6] {
            let image = spektrafilm_math::image::ImageBuf::from_data(
                1,
                samples.len() as u32,
                samples
                    .iter()
                    .flat_map(|&v| [from_f64(v), from_f64(v), from_f64(v)])
                    .collect(),
            );
            let encoded = backend.encode_in_space(&image, cctf);
            for (i, &v) in samples.iter().enumerate() {
                let got = to_f64(encoded.data[i * 3]);
                assert!(
                    got.is_finite(),
                    "{cctf:?} at {v}: the device transfer returned {got}"
                );
                // The pass rounds its encoded result to half precision before
                // composing (`half_round`), exactly like the reference working
                // domain, so the budget is that rounding and not f32.
                let reference = cctf_encode(v, cctf);
                let tolerance = reference.abs().max(1.0) * 1e-3;
                assert!(
                    (got - reference).abs() <= tolerance,
                    "{cctf:?} at {v}: device {got} != reference {reference}"
                );
            }
        }
    }
}
