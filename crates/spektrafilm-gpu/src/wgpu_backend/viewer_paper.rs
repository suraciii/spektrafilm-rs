//! Pinned virtual-paper synthesis. Only the original decoded alpha and scalar
//! geometry cross into the device; the sole readback is four stamp-bound words.
use super::{ObservedEncoder, WgpuBackend, dispatch_linear};
use spektrafilm_backend::{DeviceImage, SampleDomain};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PrepareParams {
    source: [u32; 4],
    rotated: [u32; 4],
    geometry: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CenterParams {
    frame: [u32; 4],
    basis: [f32; 4],
    geometry: [f32; 4],
}

impl WgpuBackend {
    fn paper_buffer(&self, label: &str, bytes: u64) -> Result<wgpu::Buffer, String> {
        let limits = self.device.limits();
        if bytes == 0
            || bytes > limits.max_buffer_size
            || bytes > u64::from(limits.max_storage_buffer_binding_size)
        {
            return Err(format!(
                "virtual paper {label} requires an unsupported {bytes}-byte storage binding"
            ));
        }
        Ok(self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }))
    }

    // Low-level dispatch keeps shader entry, binding access and workgroup shape explicit.
    #[allow(clippy::too_many_arguments)]
    fn paper_pass(
        &self,
        encoder: &mut ObservedEncoder,
        shader: &'static str,
        entry: &'static str,
        buffers: &[&wgpu::Buffer],
        read_only: &[bool],
        count: u32,
        scalar: bool,
    ) {
        let types: Vec<_> = read_only
            .iter()
            .enumerate()
            .map(|(i, &read_only)| {
                if i == 0 {
                    wgpu::BufferBindingType::Uniform
                } else {
                    wgpu::BufferBindingType::Storage { read_only }
                }
            })
            .collect();
        let pipeline = self.cached_pipeline_at(shader, &types, entry);
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewer_paper"),
            layout: &pipeline.layout,
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(entry),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        if scalar {
            pass.dispatch_workgroups(1, 1, 1);
        } else {
            dispatch_linear(&mut pass, count);
        }
    }

    /// Reproduce the pinned CPU paper back in new retained encoded-sRGB pages.
    /// `logo` is the unmodified row-major alpha of the decoded watermark asset.
    /// No image samples or random arrays are computed or downloaded on the host.
    pub(super) fn viewer_paper_gpu(
        &self,
        width: u32,
        height: u32,
        logo: &[f32],
        logo_width: u32,
        logo_height: u32,
    ) -> Result<DeviceImage, String> {
        self.check_cancelled()?;
        let n = width
            .checked_mul(height)
            .filter(|&n| n > 0)
            .ok_or("virtual paper dimensions exceed the device index range")?;
        let logo_n = logo_width
            .checked_mul(logo_height)
            .filter(|&n| n > 0)
            .ok_or("virtual paper watermark dimensions exceed the device index range")?;
        if logo.len() != logo_n as usize
            || [width, height, logo_width, logo_height]
                .iter()
                .any(|&v| v > i32::MAX as u32)
        {
            return Err(
                "virtual paper requires valid dimensions and the original decoded watermark alpha"
                    .into(),
            );
        }
        let limits = self.device.limits();
        if limits.max_compute_workgroup_size_x < 256
            || limits.max_compute_invocations_per_workgroup < 256
            || limits.max_storage_buffers_per_shader_stage < 4
            || limits.max_uniform_buffers_per_shader_stage < 1
            || limits.max_bindings_per_bind_group < 5
            || limits.max_uniform_buffer_binding_size < 48
        {
            return Err("virtual paper requires 256-thread compute groups, four storage bindings and a 48-byte uniform".into());
        }
        let long = width.max(height) as f32;
        let scale = 0.22 * long / logo_width.max(logo_height) as f32;
        let sigma = (0.5 / scale - 0.5).max(0.0);
        let blur_radius = (4.0 * sigma + 0.5).floor() as u32;
        // The pinned kernel squares signed offsets before conversion to f32.
        if blur_radius > 46340 {
            return Err("virtual paper blur radius exceeds the pinned integer kernel range".into());
        }
        let tw = (logo_width as f32 * scale).round().max(1.0) as u32;
        let th = (logo_height as f32 * scale).round().max(1.0) as u32;
        let (sin, cos) = 16_f32.to_radians().sin_cos();
        let rw = (cos.abs() * tw as f32 + sin.abs() * th as f32 + 0.5).floor() as u32;
        let rh = (sin.abs() * tw as f32 + cos.abs() * th as f32 + 0.5).floor() as u32;
        let tile_n = tw
            .checked_mul(th)
            .filter(|&n| n > 0)
            .ok_or("virtual paper tile dimensions exceed the device index range")?;
        let rotated_n = rw
            .checked_mul(rh)
            .filter(|&n| n > 0)
            .ok_or("virtual paper rotated tile dimensions exceed the device index range")?;
        if [tw, th, rw, rh].iter().any(|&v| v > i32::MAX as u32) {
            return Err("virtual paper tile dimensions exceed signed device coordinates".into());
        }
        for count in [logo_n, tile_n, rotated_n] {
            let groups = count.div_ceil(256);
            let x = groups.clamp(1, 65535);
            let y = groups.div_ceil(x);
            if x > limits.max_compute_workgroups_per_dimension
                || y > limits.max_compute_workgroups_per_dimension
            {
                return Err("virtual paper dispatch exceeds the device workgroup grid".into());
            }
        }
        // Validate all large bindings before uploading the original alpha.
        for bytes in [
            u64::from(logo_n) * 4,
            u64::from(tile_n) * 4,
            u64::from(rotated_n) * 4,
            u64::from(2 * blur_radius + 1) * 4,
        ] {
            if bytes > limits.max_buffer_size
                || bytes > u64::from(limits.max_storage_buffer_binding_size)
            {
                return Err(format!(
                    "virtual paper requires an unsupported {bytes}-byte storage binding"
                ));
            }
        }
        let alpha = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("viewer_paper_original_alpha"),
                contents: bytemuck::cast_slice(logo),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let blur_buffers = if sigma > 0.0 {
            Some((
                self.paper_buffer("viewer_paper_blur_scratch", u64::from(logo_n) * 4)?,
                self.paper_buffer("viewer_paper_blurred_alpha", u64::from(logo_n) * 4)?,
            ))
        } else {
            None
        };
        let kernel =
            self.paper_buffer("viewer_paper_kernel", u64::from(2 * blur_radius + 1) * 4)?;
        let tile = self.paper_buffer("viewer_paper_tile", u64::from(tile_n) * 4)?;
        let rotated = self.paper_buffer("viewer_paper_rotated", u64::from(rotated_n) * 4)?;
        let bounds = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("viewer_paper_bounds"),
                contents: bytemuck::cast_slice(&[rw, 0, rh, 0]),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });
        let params = self.pipe.static_buffer(
            "viewer_paper_prepare_params",
            bytemuck::bytes_of(&PrepareParams {
                source: [logo_width, logo_height, tw, th],
                rotated: [rw, rh, blur_radius, 0],
                geometry: [sigma, sin, cos, 0.0],
            }),
            wgpu::BufferUsages::UNIFORM,
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        const PREPARE: &str = include_str!("viewer_paper_prepare.wgsl");
        let blur_source = if let Some((temp, blurred)) = &blur_buffers {
            self.paper_pass(
                &mut encoder,
                PREPARE,
                "make_kernel",
                &[&params, &alpha, temp, &kernel, &bounds],
                &[true, true, false, false, false],
                1,
                true,
            );
            self.paper_pass(
                &mut encoder,
                PREPARE,
                "blur_horizontal",
                &[&params, &alpha, temp, &kernel, &bounds],
                &[true, true, false, false, false],
                logo_n,
                false,
            );
            self.paper_pass(
                &mut encoder,
                PREPARE,
                "blur_vertical",
                &[&params, temp, blurred, &kernel, &bounds],
                &[true, true, false, false, false],
                logo_n,
                false,
            );
            blurred
        } else {
            &alpha
        };
        self.paper_pass(
            &mut encoder,
            PREPARE,
            "resample_tile",
            &[&params, blur_source, &tile, &kernel, &bounds],
            &[true, true, false, false, false],
            tile_n,
            false,
        );
        self.paper_pass(
            &mut encoder,
            PREPARE,
            "rotate_tile",
            &[&params, &tile, &rotated, &kernel, &bounds],
            &[true, true, false, false, false],
            rotated_n,
            false,
        );
        self.check_cancelled()?;
        self.queue.submit(Some(encoder.finish()));
        let bounds = self.pipe.readback_buffer_cancellable::<u32>(&bounds, 16)?;
        let [xmin, xmax, ymin, ymax] = bounds[..] else {
            return Err("virtual paper stamp-bound readback is incomplete".into());
        };
        if xmin > xmax || ymin > ymax || xmax >= rw || ymax >= rh {
            return Err(
                "virtual paper watermark has no support above the pinned alpha threshold".into(),
            );
        }
        let stamp_w = xmax - xmin + 1;
        let stamp_h = ymax - ymin + 1;
        let radius = 0.5 * ((u64::from(stamp_w).pow(2) + u64::from(stamp_h).pow(2)) as f32).sqrt();
        let half = 36_f32.to_radians();
        let distance = 0.25 * long;
        let basis = [
            distance * (half.cos() * cos + half.sin() * sin),
            distance * (-half.cos() * sin + half.sin() * cos),
            distance * (half.cos() * cos - half.sin() * sin),
            distance * (-half.cos() * sin - half.sin() * cos),
        ];
        let limit =
            (((width as f32).hypot(height as f32) + 2.0 * radius) / distance).ceil() as u32 + 2;
        let candidates = (2 * limit + 1)
            .checked_pow(2)
            .ok_or("virtual paper lattice exceeds the device index range")?;
        let centers = self.paper_buffer("viewer_paper_centers", 16 + u64::from(candidates) * 8)?;
        let center_params = self.pipe.static_buffer(
            "viewer_paper_center_params",
            bytemuck::bytes_of(&CenterParams {
                frame: [width, height, limit, candidates],
                basis,
                geometry: [radius, 0.0, 0.0, 0.0],
            }),
            wgpu::BufferUsages::UNIFORM,
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.paper_pass(
            &mut encoder,
            include_str!("viewer_paper_centers.wgsl"),
            "main",
            &[&center_params, &centers],
            &[true, false],
            1,
            true,
        );
        self.check_cancelled()?;
        self.queue.submit(Some(encoder.finish()));
        let pages = self.allocate_image_pages(width, height, "viewer_paper_rgb")?;
        let capacity = pages
            .iter()
            .map(|page| page.n_pixels)
            .max()
            .ok_or("virtual paper has no output pages")?;
        // Reuse one bounded scratch buffer. Queue submission order ensures each
        // page consumes both random slices before the next page overwrites them.
        let random = self.paper_buffer("viewer_paper_random_page", u64::from(capacity) * 8)?;
        for page in &pages {
            self.check_cancelled()?;
            let groups = page.n_pixels.div_ceil(256);
            let x = groups.clamp(1, 65535);
            let y = groups.div_ceil(x);
            if x > limits.max_compute_workgroups_per_dimension
                || y > limits.max_compute_workgroups_per_dimension
            {
                return Err("virtual paper page dispatch exceeds the device workgroup grid".into());
            }
            let noise_params = self.pipe.static_buffer(
                "viewer_paper_noise_params",
                bytemuck::cast_slice(&[page.n_pixels, page.first_pixel, n, 0]),
                wgpu::BufferUsages::UNIFORM,
            );
            let compose_params = self.pipe.static_buffer(
                "viewer_paper_compose_params",
                bytemuck::cast_slice(&[
                    width,
                    height,
                    rw,
                    page.n_pixels,
                    xmin,
                    ymin,
                    stamp_w,
                    stamp_h,
                    page.first_pixel,
                    0,
                    0,
                    0,
                ]),
                wgpu::BufferUsages::UNIFORM,
            );
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.paper_pass(
                &mut encoder,
                include_str!("viewer_paper_noise.wgsl"),
                "main",
                &[&noise_params, &random],
                &[true, false],
                page.n_pixels.div_ceil(256),
                false,
            );
            self.paper_pass(
                &mut encoder,
                include_str!("viewer_paper_compose.wgsl"),
                "main",
                &[&compose_params, &rotated, &centers, &random, &page.buffer],
                &[true, true, true, true, false],
                page.n_pixels,
                false,
            );
            self.check_cancelled()?;
            self.queue.submit(Some(encoder.finish()));
        }
        self.check_cancelled()?;
        Ok(self.retain_image_pages(
            pages,
            width,
            height,
            SampleDomain::Encoded,
            spektrafilm_math::colorspace::resolve("sRGB")?,
            None,
        ))
    }
}
