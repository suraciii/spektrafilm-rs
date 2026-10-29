//! Full-support PSF convolution over bounded global-coordinate windows.
use super::*;
use crate::wgpu_backend::film_chain::device_image::BufferPayload;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    header: [u32; 4],
    controls: [f32; 4],
}

fn reflect(index: i64, extent: u32) -> u32 {
    if extent <= 1 {
        return 0;
    }
    let period = 2 * i64::from(extent - 1);
    let p = index.rem_euclid(period);
    if p >= i64::from(extent) {
        (period - p) as u32
    } else {
        p as u32
    }
}

fn window_geometry(width: u32, height: u32, k: u32, limit: u64) -> (u32, u32) {
    let halo_pixels = limit / 12;
    let tile_width = width
        .min(
            (halo_pixels / u64::from(k))
                .saturating_sub(u64::from(k - 1))
                .max(1) as u32,
        )
        .min(256);
    let halo_width = tile_width + k - 1;
    let tile_height = height
        .min(
            (halo_pixels / u64::from(halo_width))
                .saturating_sub(u64::from(k - 1))
                .max(1) as u32,
        )
        .min(64);
    (tile_width, tile_height)
}

pub(super) fn workspace_plan(
    width: u32,
    height: u32,
    k: u32,
    limit: u64,
    print: bool,
) -> Result<u64, String> {
    if width == 0 || height == 0 || k == 0 || limit < 12 {
        return Err("optical diffusion cannot hold its minimum working set".into());
    }
    let image = u128::from(width) * u128::from(height) * 12;
    let taps = u128::from(k) * u128::from(k) * 12;
    let uniforms = std::mem::size_of::<Params>() as u128;
    let temporary = if taps <= u128::from(limit) {
        let (tw, th) = window_geometry(width, height, k, limit);
        let halo = (u128::from(tw) + u128::from(k - 1)) * (u128::from(th) + u128::from(k - 1)) * 12;
        taps + 3 * (halo + u128::from(tw) * u128::from(th) * 12 + uniforms)
    } else {
        let capacity = limit / 12;
        let count = capacity.min(u64::from(width));
        let n = capacity.min(u64::from(k));
        u128::from(count) * 36
            + 3 * (u128::from(count + n - 1) * 12 + u128::from(n) * 12 + uniforms)
    };
    // Two queued batches and one encoded batch can own tile/domain adapters.
    let domain = if print {
        3 * image.min(u128::from(limit))
    } else {
        0
    };
    u64::try_from(image * if print { 3 } else { 1 } + temporary + domain)
        .map_err(|_| "optical diffusion resource plan overflows".into())
}

impl WgpuBackend {
    pub(in crate::wgpu_backend) fn diffusion_workspace_bytes(
        &self,
        width: u32,
        height: u32,
        params: &spektrafilm_backend::OpticalDiffusionGpuParams<'_>,
        print: bool,
    ) -> Result<u64, String> {
        let k = params
            .radius
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .ok_or("optical diffusion support overflows")?;
        workspace_plan(
            width,
            height,
            k,
            crate::wgpu_backend::bounded::storage_binding_limit(self)?,
            print,
        )
    }

    pub(in crate::wgpu_backend) fn diffusion_pages(
        &self,
        source: &BufferPayload,
        params: &spektrafilm_backend::OpticalDiffusionGpuParams<'_>,
        print_exposure: Option<f64>,
    ) -> Result<Vec<crate::wgpu_backend::paged::ImagePage>, String> {
        if let Some(scale) = print_exposure {
            if !scale.is_finite() {
                return Err("optical diffusion print exposure is not finite".into());
            }
            let linear = self.diffusion_domain_pages(source, scale, false)?;
            let linear =
                BufferPayload::from_pages(self.backend_id(), linear, source.width, source.height);
            let filtered = self.diffusion_pages(&linear, params, None)?;
            let filtered =
                BufferPayload::from_pages(self.backend_id(), filtered, source.width, source.height);
            return self.diffusion_domain_pages(&filtered, 1.0, true);
        }
        let k = params
            .radius
            .checked_mul(2)
            .and_then(|v| v.checked_add(1))
            .ok_or("optical diffusion PSF support exceeds device indexing")?;
        let area = (k as usize)
            .checked_mul(k as usize)
            .ok_or("optical diffusion PSF size overflow")?;
        if params
            .psf
            .iter()
            .any(|p| p.len() != area || p.iter().any(|tap| !tap.is_finite()))
            || !params.scatter_fraction.is_finite()
        {
            return Err("optical diffusion has invalid resolved coefficients".into());
        }
        let limit = crate::wgpu_backend::bounded::storage_binding_limit(self)?;
        // Three f32 lanes per tap and per output accumulator. No full-image
        // or full-kernel binding is needed, even when the radius exceeds a page.
        let capacity = (limit / 12).min(u64::from(u32::MAX)) as u32;
        if capacity == 0 {
            return Err(format!(
                "optical diffusion minimum working set needs 12 bytes per binding; limit is {limit}"
            ));
        }
        #[cfg(test)]
        self.pipe.test_operation("optical_diffusion")?;
        let minimum_halo = u64::from(k) * u64::from(k) * 12;
        if minimum_halo <= limit && (area as u64) * 12 <= limit {
            return self.diffusion_windows(source, params, limit, k);
        }
        let pipeline = self.cached_pipeline(
            include_str!("../../../../spektrafilm-shaders/wgsl/optical_diffusion_bounded.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let output = self.allocate_image_pages(source.width, source.height, "diffusion_output")?;
        let buffer = |label: &str, bytes: u64| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        for y in 0..source.height {
            let mut x = 0;
            while x < source.width {
                let count = capacity.min(source.width - x);
                let centers = buffer("diffusion_centers", u64::from(count) * 12);
                let accumulation = buffer("diffusion_accumulation", u64::from(count) * 12);
                let result = buffer("diffusion_result", u64::from(count) * 12);
                let mut encoder = self.device.create_command_encoder(&Default::default());
                self.copy_samples_to(
                    &mut encoder,
                    source,
                    (u64::from(y) * u64::from(source.width) + u64::from(x)) * 3,
                    u64::from(count) * 3,
                    &centers,
                    0,
                )?;
                self.submit_commands(Some(encoder.finish()))?;
                for ky in 0..k {
                    let sy = reflect(
                        i64::from(y) + i64::from(ky) - i64::from(params.radius),
                        source.height,
                    );
                    let mut first = 0;
                    while first < k {
                        let n = capacity.min(k - first);
                        let halo_count = count + n - 1;
                        let halo = buffer("diffusion_halo", u64::from(halo_count) * 12);
                        let mut encoder = self.device.create_command_encoder(&Default::default());
                        let mut offset = 0;
                        while offset < halo_count {
                            let sx = reflect(
                                i64::from(x) + i64::from(first) + i64::from(offset)
                                    - i64::from(params.radius),
                                source.width,
                            );
                            let mut run = 1;
                            while offset + run < halo_count
                                && reflect(
                                    i64::from(x) + i64::from(first) + i64::from(offset + run)
                                        - i64::from(params.radius),
                                    source.width,
                                ) == sx + run
                            {
                                run += 1;
                            }
                            self.copy_samples_to(
                                &mut encoder,
                                source,
                                (u64::from(sy) * u64::from(source.width) + u64::from(sx)) * 3,
                                u64::from(run) * 3,
                                &halo,
                                u64::from(offset) * 3,
                            )?;
                            offset += run;
                        }
                        let mut taps = Vec::with_capacity(n as usize * 3);
                        for channel in params.psf {
                            let start = ky as usize * k as usize + first as usize;
                            taps.extend(
                                channel[start..start + n as usize]
                                    .iter()
                                    .map(|&tap| tap as f32),
                            );
                        }
                        let taps = self.pipe.static_buffer(
                            "diffusion_taps",
                            bytemuck::cast_slice(&taps),
                            wgpu::BufferUsages::STORAGE,
                        );
                        let uniform = Params {
                            header: [
                                count,
                                n,
                                u32::from(ky == 0 && first == 0),
                                u32::from(ky == k - 1 && first + n == k),
                            ],
                            controls: [params.scatter_fraction as f32, 0.0, 0.0, 0.0],
                        };
                        let uniform = self.pipe.static_buffer(
                            "diffusion_params",
                            bytemuck::bytes_of(&uniform),
                            wgpu::BufferUsages::UNIFORM,
                        );
                        let buffers = [&uniform, &halo, &taps, &accumulation, &centers, &result];
                        let entries: Vec<_> = buffers
                            .iter()
                            .enumerate()
                            .map(|(i, b)| wgpu::BindGroupEntry {
                                binding: i as u32,
                                resource: b.as_entire_binding(),
                            })
                            .collect();
                        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("diffusion_bounded"),
                            layout: &pipeline.layout,
                            entries: &entries,
                        });
                        {
                            let mut pass =
                                encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                                    label: Some("optical_diffusion"),
                                    timestamp_writes: None,
                                });
                            pass.set_pipeline(&pipeline.pipeline);
                            pass.set_bind_group(0, &group, &[]);
                            dispatch_linear(&mut pass, count);
                        }
                        self.submit_commands(Some(encoder.finish()))?;
                        first += n;
                    }
                }
                let mut encoder = self.device.create_command_encoder(&Default::default());
                self.copy_samples_from(
                    &mut encoder,
                    &result,
                    0,
                    &output,
                    (u64::from(y) * u64::from(source.width) + u64::from(x)) * 3,
                    u64::from(count) * 3,
                )?;
                self.submit_commands(Some(encoder.finish()))?;
                x += count;
            }
        }
        Ok(output)
    }

    fn diffusion_domain_pages(
        &self,
        source: &BufferPayload,
        scale: f64,
        log: bool,
    ) -> Result<Vec<crate::wgpu_backend::paged::ImagePage>, String> {
        let pipeline = self.cached_pipeline(
            include_str!("../../../../spektrafilm-shaders/wgsl/optical_diffusion_domain.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let output =
            self.allocate_image_pages(source.width, source.height, "diffusion_domain_output")?;
        for page in &output {
            let input = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("diffusion_domain_input"),
                size: u64::from(page.n_pixels) * 12,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let uniform = Params {
                header: [page.n_pixels, u32::from(log), 0, 0],
                controls: [
                    scale.abs().log2() as f32,
                    if scale == 0.0 {
                        0.0
                    } else {
                        scale.signum() as f32
                    },
                    0.0,
                    0.0,
                ],
            };
            let uniform = self.pipe.static_buffer(
                "diffusion_domain_params",
                bytemuck::bytes_of(&uniform),
                wgpu::BufferUsages::UNIFORM,
            );
            let buffers = [&uniform, &input, &page.buffer];
            let entries: Vec<_> = buffers
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: b.as_entire_binding(),
                })
                .collect();
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("diffusion_domain"),
                layout: &pipeline.layout,
                entries: &entries,
            });
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.copy_samples_to(
                &mut encoder,
                source,
                u64::from(page.first_pixel) * 3,
                u64::from(page.n_pixels) * 3,
                &input,
                0,
            )?;
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("diffusion_print_domain"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline.pipeline);
                pass.set_bind_group(0, &group, &[]);
                dispatch_linear(&mut pass, page.n_pixels);
            }
            self.submit_commands(Some(encoder.finish()))?;
        }
        Ok(output)
    }

    fn diffusion_windows(
        &self,
        source: &BufferPayload,
        params: &spektrafilm_backend::OpticalDiffusionGpuParams<'_>,
        limit: u64,
        k: u32,
    ) -> Result<Vec<crate::wgpu_backend::paged::ImagePage>, String> {
        let pipeline = self.cached_pipeline(
            include_str!("../../../../spektrafilm-shaders/wgsl/optical_diffusion_window.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let taps: Vec<f32> = params
            .psf
            .iter()
            .flat_map(|channel| channel.iter().map(|&tap| tap as f32))
            .collect();
        let taps = self.pipe.static_buffer(
            "diffusion_full_psf",
            bytemuck::cast_slice(&taps),
            wgpu::BufferUsages::STORAGE,
        );
        let output =
            self.allocate_image_pages(source.width, source.height, "diffusion_window_output")?;
        let (tile_width, tile_height) = window_geometry(source.width, source.height, k, limit);
        let buffer = |label: &str, bytes: u64| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        for y in (0..source.height).step_by(tile_height as usize) {
            let height = tile_height.min(source.height - y);
            for x in (0..source.width).step_by(tile_width as usize) {
                let width = tile_width.min(source.width - x);
                let halo_width = width + k - 1;
                let halo_height = height + k - 1;
                let halo = buffer(
                    "diffusion_window_halo",
                    u64::from(halo_width) * u64::from(halo_height) * 12,
                );
                let result = buffer(
                    "diffusion_window_result",
                    u64::from(width) * u64::from(height) * 12,
                );
                let mut encoder = self.device.create_command_encoder(&Default::default());
                for row in 0..halo_height {
                    let sy = reflect(
                        i64::from(y) + i64::from(row) - i64::from(params.radius),
                        source.height,
                    );
                    let mut offset = 0;
                    while offset < halo_width {
                        let sx = reflect(
                            i64::from(x) + i64::from(offset) - i64::from(params.radius),
                            source.width,
                        );
                        let mut run = 1;
                        while offset + run < halo_width
                            && reflect(
                                i64::from(x) + i64::from(offset + run) - i64::from(params.radius),
                                source.width,
                            ) == sx + run
                        {
                            run += 1;
                        }
                        self.copy_samples_to(
                            &mut encoder,
                            source,
                            (u64::from(sy) * u64::from(source.width) + u64::from(sx)) * 3,
                            u64::from(run) * 3,
                            &halo,
                            (u64::from(row) * u64::from(halo_width) + u64::from(offset)) * 3,
                        )?;
                        offset += run;
                    }
                }
                let uniform = Params {
                    header: [width, height, halo_width, k],
                    controls: [params.scatter_fraction as f32, 0.0, 0.0, 0.0],
                };
                let uniform = self.pipe.static_buffer(
                    "diffusion_window_params",
                    bytemuck::bytes_of(&uniform),
                    wgpu::BufferUsages::UNIFORM,
                );
                let buffers = [&uniform, &halo, &taps, &result];
                let entries: Vec<_> = buffers
                    .iter()
                    .enumerate()
                    .map(|(i, b)| wgpu::BindGroupEntry {
                        binding: i as u32,
                        resource: b.as_entire_binding(),
                    })
                    .collect();
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("diffusion_window"),
                    layout: &pipeline.layout,
                    entries: &entries,
                });
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("optical_diffusion"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&pipeline.pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    dispatch_linear(&mut pass, width * height);
                }
                for row in 0..height {
                    self.copy_samples_from(
                        &mut encoder,
                        &result,
                        u64::from(row) * u64::from(width) * 3,
                        &output,
                        (u64::from(y + row) * u64::from(source.width) + u64::from(x)) * 3,
                        u64::from(width) * 3,
                    )?;
                }
                self.submit_commands(Some(encoder.finish()))?;
            }
        }
        Ok(output)
    }
}

#[cfg(test)]
mod logic_tests {
    use super::*;

    #[test]
    fn diffusion_window_holds_the_full_halo_and_obeys_tile_caps() {
        assert_eq!(window_geometry(10, 10, 3, 108), (1, 1));
        assert_eq!(window_geometry(10, 10, 3, 180), (3, 1));
        assert_eq!(window_geometry(500, 500, 1, 1_000_000), (256, 64));
        for (width, height, k, limit) in [(17, 13, 3, 180), (4, 8, 5, 600), (1, 1, 9, 972)] {
            let (tw, th) = window_geometry(width, height, k, limit);
            assert!(tw >= 1 && tw <= width && th >= 1 && th <= height);
            assert!(u64::from(tw + k - 1) * u64::from(th + k - 1) * 12 <= limit);
        }
    }

    #[test]
    fn diffusion_reflection_excludes_endpoints_and_repeats_periodically() {
        for index in -12..=12 {
            assert_eq!(reflect(index, 1), 0);
            assert_eq!(reflect(index, 3), reflect(index + 4, 3));
        }
        assert_eq!(
            (-1..=5).map(|i| reflect(i, 3)).collect::<Vec<_>>(),
            [1, 0, 1, 2, 1, 0, 1]
        );
    }
}
