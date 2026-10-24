//! Device-only preparation and evaluation of the viewer's nine Polaroid layers.
use super::{ObservedEncoder, WgpuBackend};
use super::paged::ImagePage;
use spektrafilm_backend::{DeviceImage, SampleDomain};

impl WgpuBackend {
    /// Apply the pinned Polaroid layers using the caller's layer_coefficients(t).
    /// Pages share full-image divisor and density/chroma/neutral maxima. No
    /// samples or reduction values leave the device, and the source is read-only.
    pub(super) fn viewer_polaroid_gpu(
        &self,
        image: &DeviceImage,
        coefficients: [f32; 9],
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        let source = self.buffer_payload(image)?;
        let max_page = source.pages.iter().map(|page| page.n_pixels).max().unwrap_or(0);
        if max_page == 0 || max_page > u32::MAX / 3 {
            return Err("viewer Polaroid: page size exceeds the shader's RGB index range".into());
        }
        let groups = max_page.div_ceil(256);
        let statistics_bytes = u64::from(groups) * 32;
        let limits = self.device.limits();
        for (name, required) in [
            ("image page", u64::from(max_page) * 12),
            ("statistics", statistics_bytes),
        ] {
            if required > limits.max_buffer_size
                || required > u64::from(limits.max_storage_buffer_binding_size)
            {
                return Err(format!(
                    "viewer Polaroid: {name} requires {required} bytes, exceeding device buffer \
                     limits (allocation {}, storage binding {})",
                    limits.max_buffer_size, limits.max_storage_buffer_binding_size
                ));
            }
        }
        if limits.max_compute_invocations_per_workgroup < 256
            || limits.max_compute_workgroup_size_x < 256
            || limits.max_compute_workgroup_storage_size < 8192
            || limits.max_storage_buffers_per_shader_stage < 4
            || limits.max_bindings_per_bind_group < 5
            || limits.max_uniform_buffer_binding_size < 64
        {
            return Err("viewer Polaroid: device cannot support the 256-lane reduction resources".into());
        }
        let axis_limit = limits.max_compute_workgroups_per_dimension;
        if axis_limit == 0 || u64::from(groups) > u64::from(axis_limit) * u64::from(axis_limit) {
            return Err("viewer Polaroid: reduction dispatch exceeds device workgroup grid limits".into());
        }
        let make_storage = |label, size| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        // Mirror the input page boundaries, so no image-sized staging or page
        // repacking is needed. The reduction scratch is bounded by one page.
        let density: Vec<_> = source.pages.iter().map(|page| ImagePage {
            first_pixel: page.first_pixel,
            n_pixels: page.n_pixels,
            buffer: make_storage("viewer_polaroid_density", u64::from(page.n_pixels) * 12),
        }).collect();
        let output: Vec<_> = source.pages.iter().map(|page| ImagePage {
            first_pixel: page.first_pixel,
            n_pixels: page.n_pixels,
            buffer: make_storage("viewer_polaroid_output", u64::from(page.n_pixels) * 12),
        }).collect();
        let statistics_a = make_storage("viewer_polaroid_statistics_a", statistics_bytes);
        let statistics_b = make_storage("viewer_polaroid_statistics_b", statistics_bytes);
        let divisor = make_storage("viewer_polaroid_global_divisor", 32);
        let maxima = make_storage("viewer_polaroid_global_maxima", 32);
        use wgpu::BufferBindingType::{Storage, Uniform};
        let reduction = self.cached_pipeline(
            include_str!("../../../spektrafilm-shaders/wgsl/viewer_polaroid_reduce.wgsl"),
            &[
                Uniform,
                Storage { read_only: true },
                Storage { read_only: true },
                Storage { read_only: false },
                Storage { read_only: false },
            ],
        );
        let reduce = |encoder: &mut ObservedEncoder,
                      source_buffer: &wgpu::Buffer,
                      density_buffer: &wgpu::Buffer,
                      input: &wgpu::Buffer,
                      destination: &wgpu::Buffer,
                      count: u32,
                      mode: u32| {
            let params = self.pipe.static_buffer(
                "viewer_polaroid_reduction_params",
                bytemuck::cast_slice(&[count, mode, 0, 0]),
                wgpu::BufferUsages::UNIFORM,
            );
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("viewer_polaroid_reduction"),
                layout: &reduction.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: source_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: input.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: destination.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: density_buffer.as_entire_binding() },
                ],
            });
            let workgroups = count.div_ceil(256);
            let x = workgroups.min(axis_limit);
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("viewer_polaroid_reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&reduction.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(x, workgroups.div_ceil(x), 1);
        };
        // Sweep one complete reduction before the next: every density uses the
        // same global divisor, and every frame uses the same global maxima.
        for mode in [0, 1] {
            for (page, prepared) in source.pages.iter().zip(&density) {
                self.check_cancelled()?;
                let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("viewer_polaroid_page_reduction"),
                });
                let mut reduced = &statistics_a;
                let mut scratch = &statistics_b;
                reduce(&mut encoder, &page.buffer, &prepared.buffer, &divisor,
                    reduced, page.n_pixels, mode);
                let mut count = page.n_pixels.div_ceil(256);
                while count > 1 {
                    reduce(&mut encoder, &page.buffer, &prepared.buffer, reduced, scratch, count, 2);
                    std::mem::swap(&mut reduced, &mut scratch);
                    count = count.div_ceil(256);
                }
                let global = if mode == 0 { &divisor } else { &maxima };
                reduce(&mut encoder, &page.buffer, &prepared.buffer, reduced, global, 1, 3);
                self.check_cancelled()?;
                self.queue.submit(Some(encoder.finish()));
            }
        }
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct FrameParams {
            header: [u32; 4],
            coefficients: [[f32; 4]; 3],
        }
        let frame = self.cached_pipeline(
            include_str!("../../../spektrafilm-shaders/wgsl/viewer_polaroid_frame.wgsl"),
            &[
                Uniform,
                Storage { read_only: true },
                Storage { read_only: true },
                Storage { read_only: false },
            ],
        );
        for (prepared, destination) in density.iter().zip(&output) {
            self.check_cancelled()?;
            let params = FrameParams {
                header: [prepared.n_pixels, 0, 0, 0],
                coefficients: [
                    [coefficients[0], coefficients[1], coefficients[2], coefficients[3]],
                    [coefficients[4], coefficients[5], coefficients[6], coefficients[7]],
                    [coefficients[8], 0.0, 0.0, 0.0],
                ],
            };
            let uniform = self.pipe.static_buffer(
                "viewer_polaroid_frame_params",
                bytemuck::bytes_of(&params),
                wgpu::BufferUsages::UNIFORM,
            );
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("viewer_polaroid_frame"),
                layout: &frame.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: prepared.buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: maxima.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: destination.buffer.as_entire_binding() },
                ],
            });
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("viewer_polaroid_page_frame"),
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("viewer_polaroid_frame"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&frame.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                let count = prepared.n_pixels.div_ceil(256);
                let x = count.min(axis_limit);
                pass.dispatch_workgroups(x, count.div_ceil(x), 1);
            }
            self.check_cancelled()?;
            self.queue.submit(Some(encoder.finish()));
        }
        self.check_cancelled()?;
        Ok(self.retain_image_pages(
            output,
            image.width(),
            image.height(),
            SampleDomain::Encoded,
            spektrafilm_math::colorspace::resolve("sRGB")?,
            image.pixel_pitch_um(),
        ))
    }
}
