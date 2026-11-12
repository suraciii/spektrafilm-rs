//! Read-only retained source windows and the native viewer presentation boundary.
use super::paged::ImagePage;
use super::*;
use spektrafilm_backend::{
    DeviceImage, DisplayTransformGpuParams, SampleDomain, ViewerComposeGpuParams,
    ViewerFullSourceGpuParams, ViewerIccGpuParams,
};

#[cfg(feature = "precision-f64")]
struct ClippingF64Payload {
    pages: std::sync::Arc<super::film_chain::device_image::BufferPayload>,
}

#[cfg(feature = "precision-f64")]
impl spektrafilm_backend::DeviceImagePayload for ClippingF64Payload {
    fn backend_id(&self) -> u64 {
        spektrafilm_backend::DeviceImagePayload::backend_id(self.pages.as_ref())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl WgpuBackend {
    pub(super) fn retain_clipping_source_gpu(
        &self,
        image: &ImageBuf,
        meta: &spektrafilm_backend::RetainedImageMeta,
    ) -> Result<DeviceImage, String> {
        #[cfg(feature = "precision-f64")]
        {
            self.retain_clipping_f64_with_limit(
                image,
                meta,
                super::bounded::storage_binding_limit(self)?,
            )
        }
        #[cfg(not(feature = "precision-f64"))]
        {
            self.upload_retained(image, meta)
        }
    }

    #[cfg(feature = "precision-f64")]
    fn retain_clipping_f64_with_limit(
        &self,
        image: &ImageBuf,
        meta: &spektrafilm_backend::RetainedImageMeta,
        limit: u64,
    ) -> Result<DeviceImage, String> {
        use super::film_chain::device_image::BufferPayload;
        use spektrafilm_backend::Precision;
        self.check_cancelled()?;
        let count = u64::from(image.width) * u64::from(image.height);
        if meta.width != image.width
            || meta.height != image.height
            || meta.precision != Precision::F64
            || count > u64::from(u32::MAX)
            || count.checked_mul(3) != Some(image.data.len() as u64)
        {
            return Err("viewer clipping source metadata does not match samples".into());
        }
        let capacity = (limit.min(self.viewer_limit()) / 24).min(u64::from(u32::MAX)) as u32;
        if capacity == 0 && count != 0 {
            return Err("viewer clipping source exceeds device limits".into());
        }
        let mut pages = Vec::new();
        let mut first = 0u32;
        while first < count as u32 {
            self.check_cancelled()?;
            let n = capacity.min(count as u32 - first);
            let start = first as usize * 3;
            let end = start + n as usize * 3;
            let buffer = self.viewer_buffer(u64::from(n) * 24, "viewer_clipping_source");
            self.queue
                .write_buffer(&buffer, 0, bytemuck::cast_slice(&image.data[start..end]));
            pages.push(ImagePage {
                first_pixel: first,
                n_pixels: n,
                buffer,
            });
            first += n;
        }
        self.check_cancelled()?;
        let mut payload =
            BufferPayload::from_pages(self.backend_id(), pages, image.width, image.height);
        payload.bytes = count * 24;
        let payload = std::sync::Arc::new(payload);
        self.device.resources.register_image(&payload);
        Ok(DeviceImage::retained(
            *meta,
            std::sync::Arc::new(ClippingF64Payload { pages: payload }),
        ))
    }
}

impl WgpuBackend {
    pub(super) fn viewer_inspect_gpu(
        &self,
        image: &DeviceImage,
        x: u32,
        y: u32,
    ) -> Result<[f32; 3], String> {
        self.check_cancelled()?;
        if x >= image.width() || y >= image.height() {
            return Err("pixel inspection outside source".into());
        }
        let source = self.buffer_payload(image)?;
        let sample = self.viewer_buffer(12, "viewer_source_pixel");
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.copy_samples_to(
            &mut encoder,
            source,
            (u64::from(y) * u64::from(image.width()) + u64::from(x)) * 3,
            3,
            &sample,
            0,
        )?;
        self.check_cancelled()?;
        self.submit_commands(Some(encoder.finish()))?;
        let result: Vec<f32> = self.pipe.readback_buffer_cancellable(&sample, 12)?;
        Ok([result[0], result[1], result[2]])
    }

    fn viewer_limit(&self) -> u64 {
        let l = self.device.limits();
        l.max_buffer_size
            .min(u64::from(l.max_storage_buffer_binding_size))
    }
    fn viewer_buffer(&self, bytes: u64, label: &str) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.max(4),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }
    fn viewer_table(&self, bytes: &[u8]) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("viewer_profile_table"),
                contents: if bytes.is_empty() { &[0; 4] } else { bytes },
                usage: wgpu::BufferUsages::STORAGE,
            })
    }
    pub(super) fn viewer_dispatch(
        &self,
        shader: &'static str,
        words: &[u32],
        inputs: &[&wgpu::Buffer],
        count: u32,
        label: &'static str,
    ) -> Result<wgpu::Buffer, String> {
        self.check_cancelled()?;
        let bytes = u64::from(count) * 12;
        if count == 0 || bytes > self.viewer_limit() {
            return Err(format!(
                "{label}: minimum viewer working buffer exceeds device limits"
            ));
        }
        let out = self.viewer_buffer(bytes, label);
        self.viewer_dispatch_into(shader, words, inputs, count, &out, label)?;
        Ok(out)
    }

    fn viewer_dispatch_into(
        &self,
        shader: &'static str,
        words: &[u32],
        inputs: &[&wgpu::Buffer],
        count: u32,
        out: &wgpu::Buffer,
        label: &'static str,
    ) -> Result<(), String> {
        self.check_cancelled()?;
        #[cfg(test)]
        self.pipe.test_operation(label)?;
        let mut aligned = words.to_vec();
        aligned.resize(words.len().next_multiple_of(4), 0);
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&aligned),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let mut types = vec![wgpu::BufferBindingType::Uniform];
        types.extend(
            inputs
                .iter()
                .map(|_| wgpu::BufferBindingType::Storage { read_only: true }),
        );
        types.push(wgpu::BufferBindingType::Storage { read_only: false });
        let pipeline = self.cached_pipeline(shader, &types);
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: params.as_entire_binding(),
        }];
        entries.extend(
            inputs
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32 + 1,
                    resource: b.as_entire_binding(),
                }),
        );
        entries.push(wgpu::BindGroupEntry {
            binding: inputs.len() as u32 + 1,
            resource: out.as_entire_binding(),
        });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &pipeline.layout,
            entries: &entries,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &group, &[]);
            let groups = count.div_ceil(256);
            let max = self.device.limits().max_compute_workgroups_per_dimension;
            let x = groups.min(max);
            let y = groups.div_ceil(x);
            if y > max {
                return Err(format!("{label}: dispatch exceeds device limits"));
            }
            pass.dispatch_workgroups(x, y, 1);
        }
        self.check_cancelled()?;
        self.submit_commands(Some(encoder.finish()))?;
        self.check_cancelled()?;
        Ok(())
    }
    fn viewer_window(&self, image: &DeviceImage, window: [u32; 4]) -> Result<wgpu::Buffer, String> {
        self.check_cancelled()?;
        let bytes = u64::from(window[2]) * u64::from(window[3]) * 12;
        if bytes > self.viewer_limit() {
            return Err("viewer source window exceeds device limits".into());
        }
        let buffer = self.viewer_buffer(bytes, "viewer_source_window");
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.copy_window_to(
            &mut encoder,
            self.buffer_payload(image)?,
            window[0],
            window[1],
            window[2],
            window[3],
            &buffer,
        )?;
        self.check_cancelled()?;
        self.submit_commands(Some(encoder.finish()))?;
        self.check_cancelled()?;
        Ok(buffer)
    }
    // Partition logical taps, including repeated edge-clamped taps. Bound
    // each submission even when the source fits in a binding.
    #[allow(clippy::too_many_arguments)]
    fn viewer_area_tiled(
        &self,
        image: &DeviceImage,
        bounds: [i32; 4],
        words: &[u32],
        window_offset: usize,
        limit: u64,
        shader: &'static str,
        label: &'static str,
        extra_inputs: &[&wgpu::Buffer],
    ) -> Result<wgpu::Buffer, String> {
        let capacity = (limit / 12).min(262_144).min(i32::MAX as u64) as i32;
        if capacity == 0 {
            return Err(format!(
                "{label}: minimum source window exceeds device limits"
            ));
        }
        // The first three floats are the output sum; the next three preserve
        // compensation across submissions. Only the final tile publishes it.
        let out = self.viewer_buffer(24, label);
        let mut y = bounds[1];
        let mut accumulate = false;
        let width = i64::from(bounds[2]) - i64::from(bounds[0]);
        while y < bounds[3] {
            let rows = area_tile_rows(width, capacity, i64::from(bounds[3]) - i64::from(y));
            let mut x = bounds[0];
            while x < bounds[2] {
                self.check_cancelled()?;
                let end = x.saturating_add(capacity).min(bounds[2]);
                let window = bounds_window(
                    [x, y, end, y.saturating_add(rows)],
                    image.width(),
                    image.height(),
                );
                let source = self.viewer_window(image, window)?;
                let mut params = words.to_vec();
                params[window_offset..window_offset + 3].copy_from_slice(&window[..3]);
                let final_tile = end == bounds[2] && y.saturating_add(rows) >= bounds[3];
                let flags = u32::from(accumulate) | (u32::from(final_tile) << 1);
                params.extend([x as u32, end as u32, y as u32, rows as u32, flags]);
                let mut inputs = Vec::with_capacity(1 + extra_inputs.len());
                inputs.push(&source);
                inputs.extend_from_slice(extra_inputs);
                self.viewer_dispatch_into(shader, &params, &inputs, 1, &out, label)?;
                accumulate = true;
                x = end;
            }
            y += rows;
        }
        Ok(out)
    }

    fn viewer_finish(&self, pages: Vec<ImagePage>, width: u32, height: u32) -> DeviceImage {
        self.retain_image_pages(
            pages,
            width,
            height,
            SampleDomain::Encoded,
            spektrafilm_math::colorspace::resolve("sRGB").expect("registered sRGB"),
            None,
        )
    }

    pub(super) fn viewer_fit_gpu(
        &self,
        image: &DeviceImage,
        width: u32,
        height: u32,
        display: Option<&DisplayTransformGpuParams>,
        icc: Option<&ViewerIccGpuParams<'_>>,
    ) -> Result<DeviceImage, String> {
        self.viewer_fit_gpu_with_limit(image, width, height, display, icc, self.viewer_limit())
    }

    fn viewer_fit_gpu_with_limit(
        &self,
        image: &DeviceImage,
        width: u32,
        height: u32,
        display: Option<&DisplayTransformGpuParams>,
        icc: Option<&ViewerIccGpuParams<'_>>,
        limit: u64,
    ) -> Result<DeviceImage, String> {
        let limit = limit.min(self.viewer_limit());
        self.check_cancelled()?;
        self.buffer_payload(image)?;
        let count = width
            .checked_mul(height)
            .filter(|n| *n > 0)
            .ok_or("invalid viewer fit dimensions")?;
        let fx = (image.width() as f32 / width as f32).max(1.0);
        let fy = (image.height() as f32 / height as f32).max(1.0);
        let tile_work = area_exceeds_tile(fx, fy);
        let mut pages = Vec::new();
        let mut first = 0;
        while first < count {
            self.check_cancelled()?;
            let mut n = (count - first).min((limit / 12).min(u64::from(u32::MAX)) as u32);
            if n == 0 {
                return Err("viewer fit working buffer exceeds device limits".into());
            }
            let window = loop {
                let (x0, x1, y0, y1) = output_extent(first, n, width);
                let w = if image.width() == width && image.height() == height {
                    [x0, y0, x1 - x0 + 1, y1 - y0 + 1]
                } else {
                    let fx = (image.width() as f32 / width as f32).max(1.0);
                    let fy = (image.height() as f32 / height as f32).max(1.0);
                    let sample = |v: u32, src: u32, out: u32| {
                        (v as f32 + 0.5) * src as f32 / out as f32 - 0.5
                    };
                    let bounds = area_bounds(
                        sample(x0, image.width(), width),
                        sample(x1, image.width(), width),
                        sample(y0, image.height(), height),
                        sample(y1, image.height(), height),
                        fx,
                        fy,
                    );
                    bounds_window(bounds, image.width(), image.height())
                };
                if (u64::from(w[2]) * u64::from(w[3]) * 12 <= limit
                    && !tile_work
                    && u64::from(w[2]) * u64::from(w[3]) <= 262_144)
                    || n == 1
                {
                    break w;
                }
                n = n.div_ceil(2);
            };
            let words = [
                image.width(),
                image.height(),
                width,
                height,
                window[0],
                window[1],
                window[2],
                first,
                n,
            ];
            let out = if u64::from(window[2]) * u64::from(window[3]) * 12 > limit
                || tile_work
                || (n == 1 && u64::from(window[2]) * u64::from(window[3]) > 262_144)
            {
                let x = first % width;
                let y = first / width;
                let fx = (image.width() as f32 / width as f32).max(1.0);
                let fy = (image.height() as f32 / height as f32).max(1.0);
                let sx = (x as f32 + 0.5) * image.width() as f32 / width as f32 - 0.5;
                let sy = (y as f32 + 0.5) * image.height() as f32 / height as f32 - 0.5;
                self.viewer_area_tiled(
                    image,
                    area_bounds(sx, sx, sy, sy, fx, fy),
                    &words,
                    4,
                    limit,
                    include_str!("../../../spektrafilm-shaders/wgsl/viewer_resample_retained.wgsl"),
                    "viewer_fit",
                    &[],
                )?
            } else {
                let source = self.viewer_window(image, window)?;
                let mut words = words.to_vec();
                words.extend([0; 5]);
                self.viewer_dispatch(
                    include_str!("../../../spektrafilm-shaders/wgsl/viewer_resample_retained.wgsl"),
                    &words,
                    &[&source],
                    n,
                    "viewer_fit",
                )?
            };
            pages.push(ImagePage {
                first_pixel: first,
                n_pixels: n,
                buffer: out,
            });
            first += n;
        }
        let mut current = self.viewer_finish(pages, width, height);
        if let Some(d) = display {
            let mut pages = Vec::new();
            for page in &self.buffer_payload(&current)?.pages {
                self.check_cancelled()?;
                let mut words = vec![
                    page.n_pixels,
                    d.src_cctf,
                    u32::from(d.decode),
                    u32::from(d.clamp),
                ];
                words.extend(
                    pack_matrix_columns(&d.matrix)
                        .iter()
                        .flatten()
                        .map(|v| v.to_bits()),
                );
                let out = self.viewer_dispatch(
                    concat!(
                        include_str!("../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                        include_str!(
                            "../../../spektrafilm-shaders/wgsl/viewer_display_transform.wgsl"
                        )
                    ),
                    &words,
                    &[&page.buffer],
                    page.n_pixels,
                    "viewer_display",
                )?;
                pages.push(ImagePage {
                    first_pixel: page.first_pixel,
                    n_pixels: page.n_pixels,
                    buffer: out,
                });
            }
            current = self.viewer_finish(pages, width, height);
        }
        if let Some(c) = icc {
            current = self.viewer_icc_retained(&current, c)?;
        }
        self.check_cancelled()?;
        Ok(current)
    }

    pub(super) fn viewer_full_gpu(
        &self,
        image: &DeviceImage,
        p: &ViewerFullSourceGpuParams<'_>,
    ) -> Result<DeviceImage, String> {
        self.viewer_full_gpu_with_limit(image, p, self.viewer_limit())
    }

    fn viewer_full_gpu_with_limit(
        &self,
        image: &DeviceImage,
        p: &ViewerFullSourceGpuParams<'_>,
        limit: u64,
    ) -> Result<DeviceImage, String> {
        let limit = limit.min(self.viewer_limit());
        self.check_cancelled()?;
        self.buffer_payload(image)?;
        let count = p
            .tex_width
            .checked_mul(p.tex_height)
            .filter(|n| *n > 0)
            .ok_or("invalid viewer viewport dimensions")?;
        let first_table: Vec<i32> = p.icc_first.iter().flatten().copied().collect();
        let second_table: Vec<u32> = p.icc_second.iter().flatten().copied().collect();
        let first_table = self.viewer_table(bytemuck::cast_slice(&first_table));
        let second_table = self.viewer_table(bytemuck::cast_slice(&second_table));
        let [fx, fy] = p.source_footprint.map(|f| f.max(1.0));
        let area = p.interpolation != 0 && (fx > 1.0 || fy > 1.0);
        let tile_work = area && area_exceeds_tile(fx, fy);
        let visible = tile_work.then(|| full_visible_bounds(p));
        let mut pages = Vec::new();
        let mut first = 0;
        while first < count {
            self.check_cancelled()?;
            let mut n = (count - first).min((limit / 12).min(u64::from(u32::MAX)) as u32);
            if n == 0 {
                return Err("viewer viewport working buffer exceeds device limits".into());
            }
            let blank = if let Some(bounds) = visible {
                let (blank, run) = full_visibility_run(first, count, p.tex_width, bounds);
                n = n.min(run);
                blank
            } else {
                false
            };
            let window = if blank {
                [0, 0, 1, 1]
            } else {
                loop {
                    let w = if area {
                        let bounds =
                            full_area_bounds(first, n, image.width(), image.height(), p, fx, fy);
                        bounds_window(bounds, image.width(), image.height())
                    } else {
                        source_window(
                            first,
                            n,
                            p.tex_width,
                            p.tex_height,
                            [
                                p.rect_min[0],
                                p.rect_min[1],
                                p.rect_min[0] + p.rect_size[0],
                                p.rect_min[1] + p.rect_size[1],
                            ],
                            [
                                p.image_min[0],
                                p.image_min[1],
                                p.image_min[0] + p.image_size[0],
                                p.image_min[1] + p.image_size[1],
                            ],
                            image.width(),
                            image.height(),
                            p.interpolation,
                        )
                    };
                    if (u64::from(w[2]) * u64::from(w[3]) * 12 <= limit
                        && !tile_work
                        && (!area || u64::from(w[2]) * u64::from(w[3]) <= 262_144))
                        || (n == 1 && area)
                    {
                        break w;
                    }
                    if n == 1 {
                        return Err("viewer interpolation footprint exceeds device limits".into());
                    }
                    n = n.div_ceil(2);
                }
            };
            let oversized = !blank
                && area
                && (u64::from(window[2]) * u64::from(window[3]) * 12 > limit
                    || tile_work
                    || u64::from(window[2]) * u64::from(window[3]) > 262_144);
            let mut words = vec![p.tex_width, p.tex_height, image.width(), image.height()];
            words.extend(
                p.rect_min
                    .into_iter()
                    .chain(p.rect_size)
                    .chain(p.image_min)
                    .chain(p.image_size)
                    .map(f32::to_bits),
            );
            words.extend([
                p.interpolation,
                p.src_cctf,
                u32::from(p.decode),
                u32::from(p.transform),
            ]);
            words.extend(
                pack_matrix_columns(&p.matrix)
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits()),
            );
            words.extend([u32::from(p.icc), u32::from(p.icc_matrix)]);
            words.extend(p.icc_matrix_fixed14.iter().flatten().map(|v| *v as u32));
            words.extend(p.icc_offset_fixed14.map(|v| v as u32));
            words.extend(p.source_footprint.map(f32::to_bits));
            words.extend([window[0], window[1], window[2], first, n]);
            let out = if oversized {
                let bounds = full_area_bounds(first, 1, image.width(), image.height(), p, fx, fy);
                self.viewer_area_tiled(
                    image,
                    bounds,
                    &words,
                    words.len() - 5,
                    limit,
                    include_str!(
                        "../../../spektrafilm-shaders/wgsl/viewer_full_source_retained.wgsl"
                    ),
                    "viewer_full_source",
                    &[&first_table, &second_table],
                )?
            } else {
                let source = self.viewer_window(image, window)?;
                words.extend([0; 5]);
                self.viewer_dispatch(
                    include_str!(
                        "../../../spektrafilm-shaders/wgsl/viewer_full_source_retained.wgsl"
                    ),
                    &words,
                    &[&source, &first_table, &second_table],
                    n,
                    "viewer_full_source",
                )?
            };
            pages.push(ImagePage {
                first_pixel: first,
                n_pixels: n,
                buffer: out,
            });
            first += n;
        }
        self.check_cancelled()?;
        Ok(self.viewer_finish(pages, p.tex_width, p.tex_height))
    }

    pub(super) fn viewer_compose_gpu(
        &self,
        image: Option<&DeviceImage>,
        paper: Option<&DeviceImage>,
        previous: Option<&DeviceImage>,
        native: Option<&DeviceImage>,
        p: &ViewerComposeGpuParams,
    ) -> Result<Vec<[u8; 4]>, String> {
        self.viewer_compose_gpu_with_limit(image, paper, previous, native, p, self.viewer_limit())
    }

    fn viewer_compose_gpu_with_limit(
        &self,
        image: Option<&DeviceImage>,
        paper: Option<&DeviceImage>,
        previous: Option<&DeviceImage>,
        native: Option<&DeviceImage>,
        p: &ViewerComposeGpuParams,
        limit: u64,
    ) -> Result<Vec<[u8; 4]>, String> {
        self.check_cancelled()?;
        let limit = limit.min(self.viewer_limit());
        let previous = previous.filter(|_| image.is_some());
        let count = p
            .width
            .checked_mul(p.height)
            .filter(|n| *n > 0)
            .ok_or("invalid viewer presentation dimensions")?;
        if limit < 4 {
            return Err("viewer composition footprint exceeds device limits".into());
        }
        let clipping = if p.clipping.active() && image.is_some() {
            let native = native.ok_or("native image required for viewer clipping")?;
            if native.width() == 0
                || native.height() == 0
                || !p
                    .rect
                    .iter()
                    .chain(p.image_rect.iter())
                    .all(|v| v.is_finite())
                || p.rect[2] <= p.rect[0]
                || p.rect[3] <= p.rect[1]
                || p.image_rect[2] <= p.image_rect[0]
                || p.image_rect[3] <= p.image_rect[1]
            {
                return Err(
                    "viewer clipping requires nonempty finite rectangles and source".into(),
                );
            }
            let thresholds =
                spektrafilm_backend::viewer_clipping::endpoint_thresholds(native.meta())?;
            #[cfg(feature = "precision-f64")]
            let binary64 = native
                .payload_ref()?
                .as_any()
                .downcast_ref::<ClippingF64Payload>();
            #[cfg(feature = "precision-f64")]
            let (payload, is_f64) = if let Some(payload) = binary64 {
                if native.payload_ref()?.backend_id() != self.backend_id() {
                    return Err("viewer clipping source belongs to a different device".into());
                }
                (payload.pages.as_ref(), true)
            } else {
                (self.buffer_payload(native)?, false)
            };
            #[cfg(not(feature = "precision-f64"))]
            let (payload, is_f64) = (self.buffer_payload(native)?, false);
            Some((native, payload, thresholds, is_f64))
        } else {
            None
        };
        if clipping.is_some() && limit < 16 {
            return Err("viewer clipping footprint exceeds device limits".into());
        }
        let mut result = Vec::with_capacity(count as usize);
        let mut first = 0;
        while first < count {
            self.check_cancelled()?;
            let stride = if clipping.is_some() { 16 } else { 4 };
            let mut n = (count - first).min((limit / stride).min(u64::from(u32::MAX)) as u32);
            if clipping.is_some() {
                n = n.min(262_144);
            }
            let windows = loop {
                if clipping.is_some() && n < p.width {
                    n = n.min(p.width - first % p.width);
                }
                let mut windows = [[0, 0, 1, 1]; 3];
                for (i, source) in [image, paper, previous].into_iter().enumerate() {
                    if let Some(source) = source {
                        windows[i] = if i == 0 && p.full_source {
                            let (x0, x1, y0, y1) = output_extent(first, n, p.width);
                            [x0, y0, x1 - x0 + 1, y1 - y0 + 1]
                        } else {
                            source_window(
                                first,
                                n,
                                p.width,
                                p.height,
                                p.rect,
                                if i == 1 { p.paper_rect } else { p.image_rect },
                                source.width(),
                                source.height(),
                                if i == 1 { 4 } else { p.interpolation },
                            )
                        };
                    }
                }
                if windows
                    .iter()
                    .all(|w| u64::from(w[2]) * u64::from(w[3]) * 12 <= limit)
                {
                    break windows;
                }
                if n == 1 {
                    return Err("viewer composition footprint exceeds device limits".into());
                }
                n = n.div_ceil(2);
            };
            let mut buffers = Vec::new();
            for (i, source) in [image, paper, previous].into_iter().enumerate() {
                buffers.push(if let Some(source) = source {
                    self.viewer_window(source, windows[i])?
                } else {
                    self.viewer_buffer(12, "viewer_absent_layer")
                });
            }
            let mut words = vec![p.width, p.height];
            words.extend(image.map(|i| [i.width(), i.height()]).unwrap_or([0, 0]));
            for size in [paper, previous] {
                words.extend(size.map(|i| [i.width(), i.height()]).unwrap_or([0, 0]));
            }
            words.extend([
                p.interpolation,
                u32::from(p.full_source),
                u32::from(p.output_layer),
                0,
            ]);
            words.extend(
                p.rect
                    .into_iter()
                    .chain(p.border)
                    .chain(p.paper_rect)
                    .chain(p.image_rect)
                    .map(f32::to_bits),
            );
            words.extend(p.gray.map(f32::to_bits));
            words.push(p.fade.to_bits());
            words.extend(windows.into_iter().flatten());
            words.extend([first, n, 0, 0]);
            let references: Vec<_> = buffers.iter().collect();
            let bytes = u64::from(n) * 4;
            let out = self.viewer_buffer(bytes, "viewer_presentation");
            self.viewer_dispatch_into(
                include_str!("../../../spektrafilm-shaders/wgsl/viewer_compose_retained.wgsl"),
                &words,
                &references,
                n,
                &out,
                "viewer_presentation",
            )?;
            if let Some((native, payload, thresholds, is_f64)) = clipping {
                let mask = self.viewer_buffer(bytes, "viewer_clipping_mask");
                // Geometry is shared with the host mask; source samples stay on device.
                use spektrafilm_backend::viewer_clipping::ClippingAxis;
                let x_axis = ClippingAxis::new(
                    p.width,
                    [p.rect[0], p.rect[2]],
                    [p.image_rect[0], p.image_rect[2]],
                    native.width(),
                );
                let y_axis = ClippingAxis::new(
                    p.height,
                    [p.rect[1], p.rect[3]],
                    [p.image_rect[1], p.image_rect[3]],
                    native.height(),
                );
                let (x0, x1, y0, y1) = output_extent(first, n, p.width);
                let x_len = x1 - x0 + 1;
                let mut bounds =
                    Vec::<[u32; 2]>::with_capacity(x_len as usize + (y1 - y0 + 1) as usize);
                let mut limits = [[u32::MAX, 0]; 2];
                let mut max_span = [0u32; 2];
                for (axis, start, end, mapping) in [(0, x0, x1, &x_axis), (1, y0, y1, &y_axis)] {
                    for coordinate in start..=end {
                        if coordinate & 4095 == 0 {
                            self.check_cancelled()?;
                        }
                        let Some([lo, hi]) = mapping.bounds_with_compositor_roundoff(coordinate)
                        else {
                            bounds.push([u32::MAX, 0]);
                            continue;
                        };
                        bounds.push([lo, hi]);
                        if lo < hi {
                            limits[axis][0] = limits[axis][0].min(lo);
                            limits[axis][1] = limits[axis][1].max(hi);
                            max_span[axis] = max_span[axis].max(hi - lo);
                        }
                    }
                }
                let lower =
                    u64::from(limits[1][0]) * u64::from(native.width()) + u64::from(limits[0][0]);
                let upper = if limits[0][0] < limits[0][1] && limits[1][0] < limits[1][1] {
                    u64::from(limits[1][1] - 1) * u64::from(native.width())
                        + u64::from(limits[0][1])
                } else {
                    0
                };
                let geometry = self.viewer_table(bytemuck::cast_slice(&bounds));
                if lower < upper {
                    for page in &payload.pages {
                        self.check_cancelled()?;
                        if u64::from(page.first_pixel) >= upper
                            || u64::from(page.first_pixel) + u64::from(page.n_pixels) <= lower
                        {
                            continue;
                        }
                        // Each invocation scans at most 256 selected source pixels.
                        // Split large footprints into bounded batches so cancellation
                        // is observed between device dispatches.
                        let max_samples = (u64::from(max_span[0]) * u64::from(max_span[1]))
                            .min(u64::from(page.n_pixels));
                        let blocks = max_samples.div_ceil(256) as u32;
                        let batch_limit = (262_144 / n).clamp(1, 32);
                        let mut block = 0;
                        while block < blocks {
                            self.check_cancelled()?;
                            let batch = (blocks - block).min(batch_limit);
                            let shadow = if is_f64 {
                                thresholds[0].to_bits()
                            } else {
                                u64::from((thresholds[0] as f32).to_bits())
                            };
                            let highlight = if is_f64 {
                                thresholds[1].to_bits()
                            } else {
                                u64::from((thresholds[1] as f32).to_bits())
                            };
                            let clip_words = [
                                native.width(),
                                n,
                                page.first_pixel,
                                page.n_pixels,
                                shadow as u32,
                                highlight as u32,
                                first,
                                p.width,
                                x0,
                                y0,
                                x_len,
                                block,
                                batch,
                                u32::from(is_f64),
                                (shadow >> 32) as u32,
                                (highlight >> 32) as u32,
                            ];
                            self.viewer_dispatch_into(
                                include_str!(
                                    "../../../spektrafilm-shaders/wgsl/viewer_clipping_mask.wgsl"
                                ),
                                &clip_words,
                                &[&page.buffer, &geometry],
                                n * batch,
                                &mask,
                                "viewer_clipping_mask",
                            )?;
                            block += batch;
                        }
                    }
                }
                let mode = match p.clipping {
                    spektrafilm_backend::ViewerClipping::Off => 0,
                    spektrafilm_backend::ViewerClipping::Overlay {
                        shadows,
                        highlights,
                    } => u32::from(shadows) | (u32::from(highlights) << 1),
                    spektrafilm_backend::ViewerClipping::Shadows => 4,
                    spektrafilm_backend::ViewerClipping::Highlights => 5,
                };
                let blue = u32::from_le_bytes(spektrafilm_backend::viewer_clipping::SHADOW_RGBA);
                let red = u32::from_le_bytes(spektrafilm_backend::viewer_clipping::HIGHLIGHT_RGBA);
                let mut clip_words = vec![n, mode, blue, red, first, p.width, p.height, 0];
                clip_words.extend(p.rect.into_iter().chain(p.image_rect).map(f32::to_bits));
                self.viewer_dispatch_into(
                    include_str!("../../../spektrafilm-shaders/wgsl/viewer_clipping_apply.wgsl"),
                    &clip_words,
                    &[&mask],
                    n,
                    &out,
                    "viewer_clipping_apply",
                )?;
            }
            result.extend(
                self.pipe
                    .readback_buffer_cancellable::<u32>(&out, bytes)?
                    .into_iter()
                    .map(u32::to_le_bytes),
            );
            first += n;
        }
        self.check_cancelled()?;
        Ok(result)
    }
}

fn area_tile_rows(width: i64, capacity: i32, remaining: i64) -> i32 {
    if width <= i64::from(capacity) {
        (i64::from(capacity) / width).min(remaining) as i32
    } else {
        1
    }
}

// Include guard cells when deciding whether a logical footprint needs tiles.
fn area_exceeds_tile(fx: f32, fy: f32) -> bool {
    (f64::from(fx).ceil() + 4.0) * (f64::from(fy).ceil() + 4.0) > 262_144.0
}

// Binary-search the same viewport pixel centres and half-open image rectangle
// used by the shader. The extra pixel on each side protects against rounding
// differences between host and device calculations.
fn full_visible_bounds(p: &ViewerFullSourceGpuParams<'_>) -> [u32; 4] {
    let axis = |size: u32, rect_min: f32, rect_size: f32, image_min: f32, image_size: f32| {
        if !rect_min.is_finite()
            || !rect_size.is_finite()
            || rect_size < 0.0
            || !image_min.is_finite()
            || !image_size.is_finite()
            || image_size < 0.0
        {
            return (0, size);
        }
        let find = |edge: f32| {
            let mut lo = 0u32;
            let mut hi = size;
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                let point = rect_min + (mid as f32 + 0.5) * rect_size / size as f32;
                if point < edge {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        (
            find(image_min).saturating_sub(1),
            find(image_min + image_size).saturating_add(1).min(size),
        )
    };
    let (x0, x1) = axis(
        p.tex_width,
        p.rect_min[0],
        p.rect_size[0],
        p.image_min[0],
        p.image_size[0],
    );
    let (y0, y1) = axis(
        p.tex_height,
        p.rect_min[1],
        p.rect_size[1],
        p.image_min[1],
        p.image_size[1],
    );
    [x0, y0, x1, y1]
}

// Return the next row-major run known blank or potentially visible. Blank
// runs can span rows; visible runs stop at the end of the visible row span.
fn full_visibility_run(first: u32, count: u32, width: u32, bounds: [u32; 4]) -> (bool, u32) {
    let [left, top, right, bottom] = bounds;
    let x = first % width;
    let y = first / width;
    let index = |row: u32, col: u32| u64::from(row) * u64::from(width) + u64::from(col);
    let end = if left >= right || top >= bottom || y >= bottom {
        u64::from(count)
    } else if y < top {
        index(top, left)
    } else if x < left {
        index(y, left)
    } else if x >= right {
        if y + 1 < bottom {
            index(y + 1, left)
        } else {
            u64::from(count)
        }
    } else {
        return (false, right - x);
    };
    (true, (end.min(u64::from(count)) - u64::from(first)) as u32)
}

fn output_extent(first: u32, count: u32, width: u32) -> (u32, u32, u32, u32) {
    let last = first + count - 1;
    let y0 = first / width;
    let y1 = last / width;
    if y0 == y1 {
        (first % width, last % width, y0, y1)
    } else {
        (0, width - 1, y0, y1)
    }
}

// Bounds are logical source-cell indices, with a guard cell for host/device
// floating point rounding differences. The shader clips to the exact footprint.
fn area_bounds(x0: f32, x1: f32, y0: f32, y1: f32, fx: f32, fy: f32) -> [i32; 4] {
    let axis = |a: f32, b: f32, f: f32| {
        let lo = (a.min(b) + 0.5) - f / 2.0;
        let hi = (a.max(b) + 0.5) - f / 2.0 + f;
        (
            (lo.floor() as i32).saturating_sub(1),
            (hi.ceil() as i32).saturating_add(1),
        )
    };
    let (left, right) = axis(x0, x1, fx);
    let (top, bottom) = axis(y0, y1, fy);
    [left, top, right, bottom]
}

fn bounds_window(bounds: [i32; 4], width: u32, height: u32) -> [u32; 4] {
    let left = bounds[0].clamp(0, width as i32 - 1) as u32;
    let top = bounds[1].clamp(0, height as i32 - 1) as u32;
    let right = bounds[2].saturating_sub(1).clamp(0, width as i32 - 1) as u32 + 1;
    let bottom = bounds[3].saturating_sub(1).clamp(0, height as i32 - 1) as u32 + 1;
    [left, top, right - left, bottom - top]
}

#[allow(clippy::too_many_arguments)]
fn full_area_bounds(
    first: u32,
    count: u32,
    sw: u32,
    sh: u32,
    p: &ViewerFullSourceGpuParams<'_>,
    fx: f32,
    fy: f32,
) -> [i32; 4] {
    let (x0, x1, y0, y1) = output_extent(first, count, p.tex_width);
    let coord = |v: u32, axis: usize, tex: u32, src: u32| {
        let point = p.rect_min[axis] + (v as f32 + 0.5) * p.rect_size[axis] / tex as f32;
        (point - p.image_min[axis]) / p.image_size[axis] * src as f32 - 0.5
    };
    area_bounds(
        coord(x0, 0, p.tex_width, sw),
        coord(x1, 0, p.tex_width, sw),
        coord(y0, 1, p.tex_height, sh),
        coord(y1, 1, p.tex_height, sh),
        fx,
        fy,
    )
}

// Both callers provide independent output/source geometry and interpolation bounds.
#[allow(clippy::too_many_arguments)]
fn source_window(
    first: u32,
    count: u32,
    width: u32,
    height: u32,
    rect: [f32; 4],
    image: [f32; 4],
    sw: u32,
    sh: u32,
    mode: u32,
) -> [u32; 4] {
    let (x0, x1, y0, y1) = output_extent(first, count, width);
    let radius = match mode {
        0 | 1 => 1,
        2 | 3 => 2,
        4 => 3,
        _ => 4,
    };
    let coord = |v: u32, extent: u32, lo: f32, hi: f32, ilo: f32, ihi: f32, size: u32| {
        let p = lo + (v as f32 + 0.5) * (hi - lo) / extent as f32;
        ((p - ilo) / (ihi - ilo) * size as f32 - 0.5).floor() as i32
    };
    let x0 = coord(x0, width, rect[0], rect[2], image[0], image[2], sw);
    let x1 = coord(x1, width, rect[0], rect[2], image[0], image[2], sw);
    let y0 = coord(y0, height, rect[1], rect[3], image[1], image[3], sh);
    let y1 = coord(y1, height, rect[1], rect[3], image[1], image[3], sh);
    let left = x0
        .min(x1)
        .saturating_sub(radius + 1)
        .clamp(0, sw as i32 - 1) as u32;
    let right = x0.max(x1).saturating_add(radius + 2).clamp(1, sw as i32) as u32;
    let top = y0
        .min(y1)
        .saturating_sub(radius + 1)
        .clamp(0, sh as i32 - 1) as u32;
    let bottom = y0.max(y1).saturating_add(radius + 2).clamp(1, sh as i32) as u32;
    [left, top, (right - left).max(1), (bottom - top).max(1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::{ComputeBackend, Precision, RetainedImageMeta};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    fn area_test_source(gpu: &WgpuBackend, width: u32, height: u32) -> (ImageBuf, DeviceImage) {
        let values: Vec<f32> = (0..height)
            .flat_map(|y| (0..width).flat_map(move |x| [(x % 2) as f32, (y % 2) as f32, 0.25]))
            .collect();
        let host = ImageBuf::from_data(width, height, f32_to_scalars(values.clone()));
        // Page boundaries deliberately split rows and area footprints.
        let pages = values
            .chunks(9)
            .enumerate()
            .map(|(i, samples)| ImagePage {
                first_pixel: i as u32 * 3,
                n_pixels: samples.len() as u32 / 3,
                buffer: gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("viewer_area_test_source_page"),
                        contents: bytemuck::cast_slice(samples),
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    }),
            })
            .collect();
        let source = gpu.retain_image_pages(
            pages,
            width,
            height,
            SampleDomain::Encoded,
            spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
            None,
        );
        (host, source)
    }

    fn assert_area_pixels(actual: &[[f32; 3]], expected: &[[f32; 3]]) {
        assert_eq!(actual.len(), expected.len());
        for (i, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            for c in 0..3 {
                assert!(
                    (actual[c] - expected[c]).abs() < 0.00002,
                    "pixel {i} channel {c}: expected {}, got {}",
                    expected[c],
                    actual[c]
                );
            }
        }
    }

    fn retained_area_pixels(gpu: &WgpuBackend, image: &DeviceImage) -> Vec<[f32; 3]> {
        let host = image.to_host(gpu).unwrap();
        host.data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
            .collect()
    }

    #[test]
    fn area_fit_averages_stripes_across_pages_and_preserves_source() {
        use spektrafilm_backend::ViewerResampleGpuParams;
        let gpu = crate::test_support::explicit_adapter();
        let (host, source) = area_test_source(&gpu, 8, 4);
        let original = host.data.clone();
        let expected = vec![[0.5, 0.5, 0.25]; 4];
        let params = ViewerResampleGpuParams {
            width: 8,
            height: 4,
            out_width: 2,
            out_height: 2,
        };
        assert_area_pixels(
            &gpu.viewer_resample(&host, &params).unwrap().unwrap(),
            &expected,
        );
        for limit in [24, gpu.viewer_limit()] {
            let result = gpu
                .viewer_fit_gpu_with_limit(&source, 2, 2, None, None, limit)
                .unwrap();
            assert_area_pixels(&retained_area_pixels(&gpu, &result), &expected);
        }
        let display = DisplayTransformGpuParams {
            src_cctf: 0,
            decode: false,
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            clamp: true,
        };
        let transformed = gpu
            .viewer_fit_gpu_with_limit(&source, 2, 2, Some(&display), None, 24)
            .unwrap();
        assert_area_pixels(
            &retained_area_pixels(&gpu, &transformed),
            &[[0.7353569, 0.7353569, 0.5370987]; 4],
        );
        let identity = ViewerResampleGpuParams {
            out_width: 8,
            out_height: 4,
            ..params
        };
        let original_pixels: Vec<_> = original
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
            .collect();
        assert_area_pixels(
            &gpu.viewer_resample(&host, &identity).unwrap().unwrap(),
            &original_pixels,
        );
        let same = gpu
            .viewer_fit_gpu_with_limit(&source, 8, 4, None, None, 24)
            .unwrap();
        assert_area_pixels(&retained_area_pixels(&gpu, &same), &original_pixels);
        assert_eq!(source.to_host(gpu.backend()).unwrap().data, original);
        assert_eq!(host.data, original);
    }

    #[test]
    fn area_full_source_fractional_edges_and_nearest_across_pages() {
        let gpu = crate::test_support::explicit_adapter();
        let (host, source) = area_test_source(&gpu, 4, 2);
        let original = host.data.clone();
        let mut p = ViewerFullSourceGpuParams {
            tex_width: 2,
            tex_height: 1,
            rect_min: [0.0, 0.0],
            rect_size: [4.0, 2.0],
            image_min: [0.25, 0.25],
            image_size: [3.0, 1.5],
            source_footprint: [8.0 / 3.0, 8.0 / 3.0],
            interpolation: 1,
            transform: false,
            src_cctf: 0,
            decode: false,
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            icc: false,
            icc_matrix: false,
            icc_matrix_fixed14: [[0; 3]; 3],
            icc_offset_fixed14: [0; 3],
            icc_first: &[vec![0]],
            icc_second: &[vec![0]],
        };
        // Each axis covers 8/3 source pixels, including clamped edge cells.
        let expected = [[0.375, 0.5, 0.25], [0.75, 0.5, 0.25]];
        for mode in 1..=6 {
            p.interpolation = mode;
            assert_area_pixels(
                &gpu.viewer_full_source(&host, &p).unwrap().unwrap(),
                &expected,
            );
            for limit in [24, gpu.viewer_limit()] {
                let result = gpu.viewer_full_gpu_with_limit(&source, &p, limit).unwrap();
                assert_area_pixels(&retained_area_pixels(&gpu, &result), &expected);
            }
        }
        p.interpolation = 0;
        let nearest = [[1.0, 1.0, 0.25]; 2];
        assert_area_pixels(
            &gpu.viewer_full_source(&host, &p).unwrap().unwrap(),
            &nearest,
        );
        let result = gpu.viewer_full_gpu_with_limit(&source, &p, 96).unwrap();
        assert_area_pixels(&retained_area_pixels(&gpu, &result), &nearest);

        p.interpolation = 1;
        p.transform = true;
        let transformed = [[0.375, 0.5, 0.5370987], [0.75, 0.5, 0.5370987]];
        assert_area_pixels(
            &gpu.viewer_full_source(&host, &p).unwrap().unwrap(),
            &transformed,
        );
        let result = gpu.viewer_full_gpu_with_limit(&source, &p, 24).unwrap();
        assert_area_pixels(&retained_area_pixels(&gpu, &result), &transformed);

        p.transform = false;
        p.tex_width = 3;
        p.tex_height = 2;
        p.rect_min = [-1.0, -0.5];
        p.rect_size = [6.0, 3.0];
        p.image_min = [0.0, 0.0];
        p.image_size = [4.0, 2.0];
        p.source_footprint = [2.0, 1.5];
        let boundary = [
            [0.0, 0.0, 0.25],
            [0.5, 0.0, 0.25],
            [0.0; 3],
            [0.0, 1.0, 0.25],
            [0.5, 1.0, 0.25],
            [0.0; 3],
        ];
        assert_area_pixels(
            &gpu.viewer_full_source(&host, &p).unwrap().unwrap(),
            &boundary,
        );
        let result = gpu.viewer_full_gpu_with_limit(&source, &p, 24).unwrap();
        assert_area_pixels(&retained_area_pixels(&gpu, &result), &boundary);

        // Magnification keeps the requested interpolation rather than using area.
        p.tex_width = 8;
        p.tex_height = 4;
        p.rect_min = [0.0, 0.0];
        p.rect_size = [4.0, 2.0];
        p.source_footprint = [0.5, 0.5];
        for (mode, pixel) in [(0, [0.0, 0.0, 0.25]), (1, [0.25, 0.25, 0.25])] {
            p.interpolation = mode;
            let legacy = gpu.viewer_full_source(&host, &p).unwrap().unwrap();
            let retained = gpu.viewer_full_gpu_with_limit(&source, &p, 96).unwrap();
            assert_area_pixels(&legacy[9..10], &[pixel]);
            assert_area_pixels(&retained_area_pixels(&gpu, &retained), &legacy);
        }
        assert_eq!(source.to_host(gpu.backend()).unwrap().data, original);
        assert_eq!(host.data, original);
    }

    #[test]
    fn area_tiles_batch_rows_and_segment_wide_footprints() {
        let rows = area_tile_rows(6002, 262_144, 6002);
        assert_eq!(rows, 43);
        assert_eq!((6002 + rows - 1) / rows, 140);
        assert_eq!(area_tile_rows(300_000, 262_144, 10), 1);
    }

    #[test]
    fn oversized_footprint_batches_blank_viewport_pixels() {
        let gpu = crate::test_support::explicit_adapter();
        let host = ImageBuf::from_data(1000, 1000, f32_to_scalars(vec![0.25; 3_000_000]));
        let meta = RetainedImageMeta {
            width: 1000,
            height: 1000,
            domain: SampleDomain::Encoded,
            precision: Precision::host(),
            space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
            pixel_pitch_um: None,
        };
        let source = gpu.retain_image(&host, &meta).unwrap();
        let p = ViewerFullSourceGpuParams {
            tex_width: 64,
            tex_height: 64,
            rect_min: [0.0, 0.0],
            rect_size: [64.0, 64.0],
            image_min: [31.0, 31.0],
            image_size: [1.0, 1.0],
            source_footprint: [1000.0, 1000.0],
            interpolation: 1,
            transform: false,
            src_cctf: 0,
            decode: false,
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            icc: false,
            icc_matrix: false,
            icc_matrix_fixed14: [[0; 3]; 3],
            icc_offset_fixed14: [0; 3],
            icc_first: &[vec![0]],
            icc_second: &[vec![0]],
        };
        let result = gpu
            .viewer_full_gpu_with_limit(&source, &p, gpu.viewer_limit())
            .unwrap();
        assert!(gpu.buffer_payload(&result).unwrap().pages.len() <= 16);
        let pixels = retained_area_pixels(&gpu, &result);
        assert_eq!(pixels.len(), 64 * 64);
        for (i, pixel) in pixels.iter().enumerate() {
            let expected = if i == 31 * 64 + 31 { 0.25 } else { 0.0 };
            for channel in pixel {
                assert!((*channel - expected).abs() <= 1e-5, "pixel {i}: {pixel:?}");
            }
        }
    }

    #[test]
    fn million_tap_area_means_preserve_brightness_and_source() {
        use spektrafilm_backend::ViewerResampleGpuParams;
        let gpu = crate::test_support::explicit_adapter();
        let meta = RetainedImageMeta {
            width: 1000,
            height: 1000,
            domain: SampleDomain::Encoded,
            precision: Precision::host(),
            space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
            pixel_pitch_um: None,
        };
        let fit = ViewerResampleGpuParams {
            width: 1000,
            height: 1000,
            out_width: 1,
            out_height: 1,
        };
        let full = ViewerFullSourceGpuParams {
            tex_width: 1,
            tex_height: 1,
            rect_min: [0.0, 0.0],
            rect_size: [1000.0, 1000.0],
            image_min: [0.0, 0.0],
            image_size: [1000.0, 1000.0],
            source_footprint: [1000.0, 1000.0],
            interpolation: 1,
            transform: false,
            src_cctf: 0,
            decode: false,
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            icc: false,
            icc_matrix: false,
            icc_matrix_fixed14: [[0; 3]; 3],
            icc_offset_fixed14: [0; 3],
            icc_first: &[vec![0]],
            icc_second: &[vec![0]],
        };
        for chromatic in [false, true] {
            let values: Vec<f32> = (0..1_000_000)
                .flat_map(|i| {
                    if !chromatic {
                        [0.17, 0.53, 0.91]
                    } else if i % 2 == 0 {
                        [0.2, 0.4, 0.9]
                    } else {
                        [0.8, 0.6, 0.1]
                    }
                })
                .collect();
            let expected = if chromatic {
                [0.5, 0.5, 0.5]
            } else {
                [0.17, 0.53, 0.91]
            };
            let host = ImageBuf::from_data(1000, 1000, f32_to_scalars(values));
            let original = host.data.clone();
            let source = gpu.retain_image(&host, &meta).unwrap();
            let assert_mean = |pixels: &[[f32; 3]]| {
                assert_eq!(pixels.len(), 1);
                for c in 0..3 {
                    assert!(
                        (pixels[0][c] - expected[c]).abs() <= 1e-5,
                        "channel {c}: expected {}, got {}",
                        expected[c],
                        pixels[0][c]
                    );
                }
            };
            // Host-upload shaders exercise the untiled million-tap loop.
            assert_mean(&gpu.viewer_resample(&host, &fit).unwrap().unwrap());
            assert_mean(&gpu.viewer_full_source(&host, &full).unwrap().unwrap());
            // These limits exercise both work-budget and binding-budget tiling.
            for limit in [512 * 1024, gpu.viewer_limit()] {
                let fitted = gpu
                    .viewer_fit_gpu_with_limit(&source, 1, 1, None, None, limit)
                    .unwrap();
                assert_mean(&retained_area_pixels(&gpu, &fitted));
                let viewed = gpu
                    .viewer_full_gpu_with_limit(&source, &full, limit)
                    .unwrap();
                assert_mean(&retained_area_pixels(&gpu, &viewed));
            }
            assert_eq!(source.to_host(gpu.backend()).unwrap().data, original);
            assert_eq!(host.data, original);
        }
    }

    #[test]
    fn fractional_native_hidpi_uses_selected_interpolation_kernel() {
        let gpu = crate::test_support::explicit_adapter();
        let (host, source) = area_test_source(&gpu, 4, 2);
        let mut p = ViewerFullSourceGpuParams {
            tex_width: 2,
            tex_height: 2,
            rect_min: [0.105, 0.2],
            rect_size: [0.020000003, 0.02],
            image_min: [0.1, 0.2],
            image_size: [0.04, 0.02],
            source_footprint: [1.0, 1.0],
            interpolation: 1,
            transform: false,
            src_cctf: 0,
            decode: false,
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            icc: false,
            icc_matrix: false,
            icc_matrix_fixed14: [[0; 3]; 3],
            icc_offset_fixed14: [0; 3],
            icc_first: &[vec![0]],
            icc_second: &[vec![0]],
        };
        assert!(4.0 * p.rect_size[0] / (p.image_size[0] * 2.0) > 1.0);
        let linear = gpu.viewer_full_source(&host, &p).unwrap().unwrap();
        let retained = gpu.viewer_full_gpu_with_limit(&source, &p, 96).unwrap();
        assert_area_pixels(&retained_area_pixels(&gpu, &retained), &linear);
        p.interpolation = 2;
        let cubic = gpu.viewer_full_source(&host, &p).unwrap().unwrap();
        let retained = gpu.viewer_full_gpu_with_limit(&source, &p, 96).unwrap();
        assert_area_pixels(&retained_area_pixels(&gpu, &retained), &cubic);
        assert!((linear[0][0] - cubic[0][0]).abs() > 0.01);
        // Large point origins must not turn true slight reductions into native
        // sampling, including the texture-cap 4097/4096 case.
        p.rect_min = [2000.5, 0.2];
        p.rect_size = [2.0, 0.02];
        p.image_min = [2000.0, 0.2];
        for footprint in [100.0 / 99.9, 4097.0 / 4096.0] {
            p.image_size = [4.0 / footprint, 0.02];
            p.source_footprint = [footprint, 1.0];
            p.interpolation = 1;
            let reduced = gpu.viewer_full_source(&host, &p).unwrap().unwrap();
            assert!((reduced[0][0] - 0.5).abs() < 0.002);
            let retained = gpu.viewer_full_gpu_with_limit(&source, &p, 96).unwrap();
            assert_area_pixels(&retained_area_pixels(&gpu, &retained), &reduced);
            p.interpolation = 2;
            let cubic = gpu.viewer_full_source(&host, &p).unwrap().unwrap();
            assert_area_pixels(&cubic, &reduced);
            let retained = gpu.viewer_full_gpu_with_limit(&source, &p, 96).unwrap();
            assert_area_pixels(&retained_area_pixels(&gpu, &retained), &reduced);
            p.interpolation = 0;
            let nearest = gpu.viewer_full_source(&host, &p).unwrap().unwrap();
            assert!((nearest[0][0] - 1.0).abs() < 1e-6);
            let retained = gpu.viewer_full_gpu_with_limit(&source, &p, 96).unwrap();
            assert_area_pixels(&retained_area_pixels(&gpu, &retained), &nearest);
        }
    }

    #[test]
    fn area_accumulation_preserves_positive_infinity() {
        use spektrafilm_backend::ViewerResampleGpuParams;
        let gpu = crate::test_support::explicit_adapter();
        let mut values = vec![0.5_f32; 4 * 2 * 3];
        values[0] = f32::INFINITY;
        let host = ImageBuf::from_data(4, 2, f32_to_scalars(values));
        let meta = RetainedImageMeta {
            width: 4,
            height: 2,
            domain: SampleDomain::Encoded,
            precision: Precision::host(),
            space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
            pixel_pitch_um: None,
        };
        let source = gpu.retain_image(&host, &meta).unwrap();
        let fit = ViewerResampleGpuParams {
            width: 4,
            height: 2,
            out_width: 1,
            out_height: 1,
        };
        let full = ViewerFullSourceGpuParams {
            tex_width: 1,
            tex_height: 1,
            rect_min: [0.0, 0.0],
            rect_size: [4.0, 2.0],
            image_min: [0.0, 0.0],
            image_size: [4.0, 2.0],
            source_footprint: [4.0, 2.0],
            interpolation: 1,
            transform: false,
            src_cctf: 0,
            decode: false,
            matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            icc: false,
            icc_matrix: false,
            icc_matrix_fixed14: [[0; 3]; 3],
            icc_offset_fixed14: [0; 3],
            icc_first: &[vec![0]],
            icc_second: &[vec![0]],
        };
        let check = |pixels: &[[f32; 3]]| {
            assert_eq!(pixels.len(), 1);
            assert_eq!(pixels[0][0], f32::INFINITY);
            assert!((pixels[0][1] - 0.5).abs() < 1e-6);
            assert!((pixels[0][2] - 0.5).abs() < 1e-6);
        };
        check(&gpu.viewer_resample(&host, &fit).unwrap().unwrap());
        check(&gpu.viewer_full_source(&host, &full).unwrap().unwrap());
        for limit in [24, gpu.viewer_limit()] {
            let fitted = gpu
                .viewer_fit_gpu_with_limit(&source, 1, 1, None, None, limit)
                .unwrap();
            check(&retained_area_pixels(&gpu, &fitted));
            let viewed = gpu
                .viewer_full_gpu_with_limit(&source, &full, limit)
                .unwrap();
            check(&retained_area_pixels(&gpu, &viewed));
        }
    }

    #[test]
    fn clipping_never_paints_fractional_photo_padding() {
        use spektrafilm_backend::ViewerClipping;
        let gpu = crate::test_support::explicit_adapter();
        let meta = RetainedImageMeta {
            width: 1,
            height: 1,
            domain: SampleDomain::Linear,
            precision: Precision::host(),
            space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
            pixel_pitch_um: None,
        };
        let source = ImageBuf::from_data(1, 1, vec![0.0 as _; 3]);
        let display = gpu.retain_image(&source, &meta).unwrap();
        let native = gpu.retain_viewer_clipping_source(&source, &meta).unwrap();
        let mut p = ViewerComposeGpuParams {
            width: 150,
            height: 1,
            rect: [0.0, 0.0, 100.0, 1.0],
            border: [0.0; 4],
            paper_rect: [0.0; 4],
            image_rect: [0.0, 0.0, 33.66666793823242_f32, 1.0],
            gray: [0.2, 0.4, 0.6],
            interpolation: 0,
            full_source: false,
            output_layer: false,
            fade: 1.0,
            clipping: ViewerClipping::Off,
        };
        for image_rect in [
            [0.0, 0.0, 33.66666793823242_f32, 1.0],
            [17.166667, 0.0, 83.5, 1.0],
            [33.66666793823242_f32, 0.0, 80.333336, 1.0],
        ] {
            p.image_rect = image_rect;
            p.clipping = ViewerClipping::Off;
            let baseline = gpu
                .viewer_compose_retained(Some(&display), None, None, None, &p)
                .unwrap();
            if image_rect[0] == 0.0 {
                assert_eq!(baseline[50], [51, 102, 153, 255]);
            }
            for mode in [
                ViewerClipping::Overlay {
                    shadows: true,
                    highlights: false,
                },
                ViewerClipping::Overlay {
                    shadows: false,
                    highlights: true,
                },
                ViewerClipping::Shadows,
                ViewerClipping::Highlights,
            ] {
                p.clipping = mode;
                for limit in [48, gpu.viewer_limit()] {
                    let pixels = gpu
                        .viewer_compose_gpu_with_limit(
                            Some(&display),
                            None,
                            None,
                            Some(&native),
                            &p,
                            limit,
                        )
                        .unwrap();
                    for (i, pixel) in baseline.iter().enumerate() {
                        if *pixel == [51, 102, 153, 255] {
                            assert_eq!(pixels[i], *pixel, "{mode:?} {image_rect:?} pixel {i}");
                        }
                    }
                }
            }
        }
    }

    #[cfg(feature = "precision-f64")]
    #[test]
    fn f64_warning_source_keeps_endpoints_invalids_and_pages_exact() {
        use spektrafilm_backend::{ViewerClipping, viewer_clipping};
        let gpu = crate::test_support::explicit_adapter();
        for (space, domain) in [
            ("sRGB", SampleDomain::Linear),
            ("sRGB", SampleDomain::Encoded),
            ("ACEScct", SampleDomain::Encoded),
        ] {
            let meta = RetainedImageMeta {
                width: 15,
                height: 1,
                domain,
                precision: Precision::F64,
                space: spektrafilm_math::colorspace::resolve(space).unwrap(),
                pixel_pitch_um: None,
            };
            let [black, white] = viewer_clipping::endpoint_thresholds(meta).unwrap();
            let mid = (black + white) / 2.0;
            let values = [
                [black; 3],
                [black.next_up(); 3],
                [black.next_down(); 3],
                [white.next_down(), mid, mid],
                [white, mid, mid],
                [white.next_up(), mid, mid],
                [1e-46; 3],
                [1e300, mid, mid],
                [-1e300; 3],
                [f64::NAN, black, black],
                [f64::INFINITY, mid, mid],
                [f64::NEG_INFINITY, black, black],
                [0.999999999, mid, mid],
                [-0.0; 3],
                [0.0, -0.0, 0.0],
            ];
            let host = ImageBuf::from_data(15, 1, values.into_iter().flatten().collect());
            let display = gpu
                .retain_image(
                    &ImageBuf::from_data(1, 1, vec![0.4; 3]),
                    &RetainedImageMeta { width: 1, ..meta },
                )
                .unwrap();
            let normal = gpu.retain_viewer_clipping_source(&host, &meta).unwrap();
            let paged = gpu
                .retain_clipping_f64_with_limit(&host, &meta, 72)
                .unwrap();
            assert!(gpu.viewer_fit_retained(&normal, 1, 1, None, None).is_err());
            let rect = [0.0, 0.0, 15.0, 1.0];
            let mut p = ViewerComposeGpuParams {
                width: 15,
                height: 1,
                rect,
                border: rect,
                paper_rect: rect,
                image_rect: rect,
                gray: [0.2; 3],
                interpolation: 0,
                full_source: false,
                output_layer: false,
                fade: 1.0,
                clipping: ViewerClipping::Off,
            };
            let baseline = gpu
                .viewer_compose_retained(Some(&display), None, None, None, &p)
                .unwrap();
            p.clipping = ViewerClipping::Overlay {
                shadows: true,
                highlights: true,
            };
            let mask = viewer_clipping::mask_host(&host, meta, &p, None).unwrap();
            assert_eq!(&mask[..6], &[1, 0, 1, 0, 2, 2]);
            assert_eq!(&mask[7..12], &[2, 1, 0, 0, 0]);
            if domain == SampleDomain::Linear {
                assert_eq!(mask[6], 0);
                assert_eq!(&mask[12..], &[0, 1, 1]);
            }
            let expected: Vec<_> = baseline
                .iter()
                .zip(mask)
                .map(|(&pixel, flags)| viewer_clipping::apply_rgba(pixel, flags, p.clipping))
                .collect();
            for source in [&normal, &paged] {
                for limit in [48, gpu.viewer_limit()] {
                    assert_eq!(
                        gpu.viewer_compose_gpu_with_limit(
                            Some(&display),
                            None,
                            None,
                            Some(source),
                            &p,
                            limit
                        )
                        .unwrap(),
                        expected,
                        "{space}"
                    );
                }
            }
            if domain == SampleDomain::Linear {
                let device_source = gpu.retain_image(&host, &meta).unwrap();
                assert_eq!(device_source.precision(), Precision::F32);
                let pixels = gpu
                    .viewer_compose_retained(Some(&display), None, None, Some(&device_source), &p)
                    .unwrap();
                assert_eq!(pixels[6], viewer_clipping::SHADOW_RGBA);
                assert_eq!(pixels[7], baseline[7]);
                assert_eq!(pixels[12], viewer_clipping::HIGHLIGHT_RGBA);
            }
        }
    }

    #[test]
    fn packed_viewer_chunks_preserve_rgba_and_four_byte_resource_bound() {
        let gpu = crate::test_support::explicit_adapter();
        let mut observed = gpu.derived_explicit();
        let context = ObservationContext::new(CollectionMode::Summary);
        observed.context_mut(context.clone());
        let rect = [0.0, 0.0, 37.0, 1.0];
        let params = ViewerComposeGpuParams {
            width: 37,
            height: 1,
            rect,
            border: [8.0, 0.0, 21.0, 1.0],
            paper_rect: rect,
            image_rect: rect,
            gray: [0.2, 0.4, 0.6],
            interpolation: 0,
            full_source: false,
            output_layer: true,
            fade: 1.0,
            clipping: spektrafilm_backend::ViewerClipping::Off,
        };
        let pixels = observed
            .viewer_compose_gpu_with_limit(None, None, None, None, &params, 28)
            .unwrap();
        let expected: Vec<_> = (0..37)
            .map(|x| {
                if (8..21).contains(&x) {
                    [255; 4]
                } else {
                    [51, 102, 153, 255]
                }
            })
            .collect();
        assert_eq!(pixels, expected);
        let work = context.snapshot().totals;
        // Five seven-pixel chunks and a two-pixel tail. RGB source windows
        // remain 12 bytes/pixel; only presentation and readback are packed.
        assert_eq!(work.readback_count, Some(6));
        assert_eq!(work.readback_bytes, Some(37 * 4));
        // Per chunk: three absent RGB pixels and the 192-byte uniform.
        // Output plus readback must fit two copies of the packed RGBA raster.
        assert!(work.buffer_create_bytes.unwrap() <= 6 * (36 + 192) + 2 * 37 * 4);
        assert!(
            observed
                .viewer_compose_gpu_with_limit(None, None, None, None, &params, 3)
                .unwrap_err()
                .contains("exceeds device limits")
        );
    }

    #[test]
    fn packed_viewer_source_chunks_preserve_channel_order_and_clamping() {
        let gpu = crate::test_support::explicit_adapter();
        let expected: Vec<_> = (0..15u8)
            .map(|i| [i * 17, 255 - i * 13, i * 7, 255])
            .collect();
        let mut values: Vec<f32> = expected
            .iter()
            .flat_map(|v| v[..3].iter().map(|&v| f32::from(v) / 255.0))
            .collect();
        values[0] = -1.0;
        values[1] = 2.0;
        let host = ImageBuf::from_data(5, 3, f32_to_scalars(values));
        let image = gpu
            .retain_image(
                &host,
                &RetainedImageMeta {
                    width: 5,
                    height: 3,
                    domain: SampleDomain::Encoded,
                    precision: Precision::host(),
                    space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
                    pixel_pitch_um: None,
                },
            )
            .unwrap();
        let rect = [0.0, 0.0, 5.0, 3.0];
        let params = ViewerComposeGpuParams {
            width: 5,
            height: 3,
            rect,
            border: rect,
            paper_rect: rect,
            image_rect: rect,
            gray: [0.0; 3],
            interpolation: 0,
            full_source: true,
            output_layer: true,
            fade: 1.0,
            clipping: spektrafilm_backend::ViewerClipping::Off,
        };
        assert_eq!(
            gpu.viewer_compose_gpu_with_limit(Some(&image), None, None, None, &params, 72)
                .unwrap(),
            expected
        );
        assert_eq!(
            gpu.viewer_compose_retained(Some(&image), None, None, None, &params)
                .unwrap(),
            expected
        );
    }

    #[test]
    fn viewer_page_cancellation_stops_readback_and_preserves_healthy_reuse() {
        let gpu = crate::test_support::explicit_adapter();
        let host = ImageBuf::from_data(1, 1, vec![0.5 as _; 3]);
        let image = gpu
            .retain_image(
                &host,
                &RetainedImageMeta {
                    width: 1,
                    height: 1,
                    domain: SampleDomain::Encoded,
                    precision: Precision::host(),
                    space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
                    pixel_pitch_um: None,
                },
            )
            .unwrap();
        let rect = [0.0, 0.0, 7.0, 1.0];
        let params = ViewerComposeGpuParams {
            width: 7,
            height: 1,
            rect,
            border: rect,
            paper_rect: rect,
            image_rect: rect,
            gray: [0.0; 3],
            interpolation: 0,
            full_source: false,
            output_layer: true,
            fade: 1.0,
            clipping: spektrafilm_backend::ViewerClipping::Overlay {
                shadows: true,
                highlights: true,
            },
        };
        let cancellation = Arc::new(AtomicBool::new(false));
        let readbacks = Arc::new(AtomicUsize::new(0));
        gpu.pipe.test_on_readback({
            let cancellation = cancellation.clone();
            let readbacks = readbacks.clone();
            move || {
                readbacks.fetch_add(1, Ordering::Relaxed);
                cancellation.store(true, Ordering::Relaxed);
            }
        });
        let mut operation = gpu.derived_explicit();
        operation.pipe = operation.pipe.with_cancellation(Some(cancellation.clone()));
        operation.cancellation = Some(cancellation.clone());
        assert_eq!(
            operation
                .viewer_compose_gpu_with_limit(Some(&image), None, None, Some(&image), &params, 24)
                .unwrap_err(),
            "operation cancelled"
        );
        assert_eq!(
            readbacks.load(Ordering::Relaxed),
            1,
            "cancelled viewer must not read the next presentation page"
        );
        gpu.pipe.test_on_readback(|| {});
        let healthy = operation.with_cancellation(None).unwrap();
        let pixels = healthy
            .viewer_compose_retained(Some(&image), None, None, Some(&image), &params)
            .unwrap();
        assert_eq!(pixels, vec![[128, 128, 128, 255]; 7]);
        assert_eq!(
            gpu.viewer_compose_gpu_with_limit(Some(&image), None, None, Some(&image), &params, 24)
                .unwrap(),
            pixels
        );
        assert_eq!(image.to_host(gpu.backend()).unwrap().data, host.data);
        gpu.pipe.test_fail_operation("viewer_presentation");
        assert!(
            healthy
                .viewer_compose_retained(Some(&image), None, None, Some(&image), &params)
                .unwrap_err()
                .contains("injected device resource failure")
        );
        assert_eq!(gpu.pipe.test_failure_attempts(), 1);
    }

    #[test]
    fn native_clipping_matches_host_across_pages_and_viewport_geometry() {
        use spektrafilm_backend::{ViewerClipping, viewer_clipping};
        let gpu = crate::test_support::explicit_adapter();
        let display_host = ImageBuf::from_data(1, 1, vec![0.4 as _; 3]);
        let display_meta = RetainedImageMeta {
            width: 1,
            height: 1,
            domain: SampleDomain::Encoded,
            precision: Precision::host(),
            space: spektrafilm_math::colorspace::resolve("sRGB").unwrap(),
            pixel_pitch_um: None,
        };
        let display = gpu.retain_image(&display_host, &display_meta).unwrap();
        for (space, domain) in [
            ("ACEScct", SampleDomain::Encoded),
            ("sRGB", SampleDomain::Linear),
        ] {
            let meta = RetainedImageMeta {
                width: 8,
                height: 2,
                domain,
                precision: Precision::F32,
                space: spektrafilm_math::colorspace::resolve(space).unwrap(),
                pixel_pitch_um: None,
            };
            let [black, white] = viewer_clipping::endpoint_thresholds(meta)
                .unwrap()
                .map(|x| x as f32);
            let mid = (black + white) / 2.0;
            let mut values = vec![mid; 8 * 2 * 3];
            values[0..3].copy_from_slice(&[black; 3]);
            values[3..6].copy_from_slice(&[black - 0.01, mid, mid]);
            values[6..9].copy_from_slice(&[mid, white, mid]);
            values[9..12].copy_from_slice(&[f32::NAN, black, black]);
            values[12..15].copy_from_slice(&[f32::INFINITY, mid, mid]);
            values[30..33].copy_from_slice(&[black; 3]);
            values[42..45].copy_from_slice(&[white, mid, mid]);
            let host = ImageBuf::from_data(8, 2, f32_to_scalars(values.clone()));
            let pages = values
                .chunks(9)
                .enumerate()
                .map(|(i, samples)| ImagePage {
                    first_pixel: i as u32 * 3,
                    n_pixels: samples.len() as u32 / 3,
                    buffer: gpu
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("clipping_test_native_page"),
                            contents: bytemuck::cast_slice(samples),
                            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                        }),
                })
                .collect();
            let native = gpu.retain_image_pages(pages, 8, 2, domain, meta.space, None);
            let geometries = [
                (8, 2, [0.0, 0.0, 8.0, 2.0], [0.0, 0.0, 8.0, 2.0]),
                (4, 1, [0.0, 0.0, 8.0, 2.0], [0.0, 0.0, 8.0, 2.0]),
                (4, 2, [2.0, 0.0, 6.0, 2.0], [0.0, 0.0, 8.0, 2.0]),
                (6, 2, [-2.0, 0.0, 10.0, 2.0], [0.0, 0.0, 8.0, 2.0]),
                (7, 3, [-0.7, 0.13, 7.4, 2.17], [-0.15, -0.2, 7.83, 2.12]),
            ];
            for (width, height, rect, image_rect) in geometries {
                let mut p = ViewerComposeGpuParams {
                    width,
                    height,
                    rect,
                    border: rect,
                    paper_rect: rect,
                    image_rect,
                    gray: [0.2; 3],
                    interpolation: 0,
                    full_source: false,
                    output_layer: false,
                    fade: 1.0,
                    clipping: ViewerClipping::Off,
                };
                let baseline = gpu
                    .viewer_compose_retained(Some(&display), None, None, None, &p)
                    .unwrap();
                let mut inactive = p;
                inactive.clipping = ViewerClipping::Overlay {
                    shadows: false,
                    highlights: false,
                };
                assert_eq!(
                    gpu.viewer_compose_retained(Some(&display), None, None, None, &inactive)
                        .unwrap(),
                    baseline
                );
                for mode in [
                    ViewerClipping::Overlay {
                        shadows: true,
                        highlights: true,
                    },
                    ViewerClipping::Overlay {
                        shadows: true,
                        highlights: false,
                    },
                    ViewerClipping::Overlay {
                        shadows: false,
                        highlights: true,
                    },
                    ViewerClipping::Shadows,
                    ViewerClipping::Highlights,
                ] {
                    p.clipping = mode;
                    let mask = viewer_clipping::mask_host(&host, meta, &p, None).unwrap();
                    if width == 8 {
                        assert_eq!(&mask[0..5], &[1, 0, 2, 0, 0]);
                    }
                    if width == 4 && height == 1 {
                        assert_eq!(mask, [1, 3, 0, 2]);
                    }
                    let expected: Vec<_> = baseline
                        .iter()
                        .enumerate()
                        .map(|(i, &pixel)| {
                            let x = i as u32 % width;
                            let y = i as u32 / width;
                            let r = rect.map(f64::from);
                            let photo = image_rect.map(f64::from);
                            let cx = r[0]
                                + f64::from(x) * ((r[2] - r[0]) / f64::from(width))
                                + 0.5 * ((r[2] - r[0]) / f64::from(width));
                            let cy = r[1]
                                + f64::from(y) * ((r[3] - r[1]) / f64::from(height))
                                + 0.5 * ((r[3] - r[1]) / f64::from(height));
                            if cx >= photo[0] && cx < photo[2] && cy >= photo[1] && cy < photo[3] {
                                viewer_clipping::apply_rgba(pixel, mask[i], mode)
                            } else {
                                pixel
                            }
                        })
                        .collect();
                    assert_eq!(
                        gpu.viewer_compose_retained(Some(&display), None, None, Some(&native), &p)
                            .unwrap(),
                        expected,
                        "{space} {mode:?} {rect:?} {image_rect:?}"
                    );
                    assert_eq!(
                        gpu.viewer_compose_gpu_with_limit(
                            Some(&display),
                            None,
                            None,
                            Some(&native),
                            &p,
                            24
                        )
                        .unwrap(),
                        expected
                    );
                    for limit in [48, 96] {
                        assert_eq!(
                            gpu.viewer_compose_gpu_with_limit(
                                Some(&display),
                                None,
                                None,
                                Some(&native),
                                &p,
                                limit
                            )
                            .unwrap(),
                            expected
                        );
                    }
                }
            }
            let retained = native.to_host(gpu.backend()).unwrap();
            for (a, b) in retained.data.iter().zip(host.data.iter()) {
                assert!(a == b || (a.is_nan() && b.is_nan()));
            }
        }
    }

    #[test]
    fn clipping_reduces_large_footprint_across_blocks_and_pages() {
        use spektrafilm_backend::{ViewerClipping, viewer_clipping};
        let gpu = crate::test_support::explicit_adapter();
        let space = spektrafilm_math::colorspace::resolve("sRGB").unwrap();
        let meta = RetainedImageMeta {
            width: 9131,
            height: 1,
            domain: SampleDomain::Linear,
            precision: Precision::F32,
            space,
            pixel_pitch_um: None,
        };
        let mut values = vec![0.5f32; meta.width as usize * 3];
        values[0..3].copy_from_slice(&[f32::NAN, 0.0, 0.0]);
        values[9000 * 3] = 1.0;
        values[9130 * 3..9131 * 3].copy_from_slice(&[0.0; 3]);
        let host = ImageBuf::from_data(meta.width, 1, f32_to_scalars(values.clone()));
        let pages = values
            .chunks(9017 * 3)
            .enumerate()
            .map(|(i, samples)| ImagePage {
                first_pixel: i as u32 * 9017,
                n_pixels: samples.len() as u32 / 3,
                buffer: gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("clipping_block_test_page"),
                        contents: bytemuck::cast_slice(samples),
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    }),
            })
            .collect();
        let native = gpu.retain_image_pages(pages, meta.width, 1, meta.domain, space, None);
        let display = gpu
            .retain_image(
                &ImageBuf::from_data(1, 1, vec![0.5 as _; 3]),
                &RetainedImageMeta {
                    width: 1,
                    height: 1,
                    ..meta
                },
            )
            .unwrap();
        let rect = [0.0, 0.0, 1.0, 1.0];
        let mut p = ViewerComposeGpuParams {
            width: 1,
            height: 1,
            rect,
            border: rect,
            paper_rect: rect,
            image_rect: rect,
            gray: [0.0; 3],
            interpolation: 0,
            full_source: false,
            output_layer: false,
            fade: 1.0,
            clipping: ViewerClipping::Off,
        };
        let baseline = gpu
            .viewer_compose_retained(Some(&display), None, None, None, &p)
            .unwrap();
        for mode in [
            ViewerClipping::Overlay {
                shadows: true,
                highlights: false,
            },
            ViewerClipping::Overlay {
                shadows: false,
                highlights: true,
            },
            ViewerClipping::Shadows,
        ] {
            p.clipping = mode;
            let flags = viewer_clipping::mask_host(&host, meta, &p, None).unwrap();
            assert_eq!(flags, [3]);
            let expected = viewer_clipping::apply_rgba(baseline[0], flags[0], mode);
            assert_eq!(
                gpu.viewer_compose_retained(Some(&display), None, None, Some(&native), &p)
                    .unwrap(),
                [expected]
            );
        }
    }
}
