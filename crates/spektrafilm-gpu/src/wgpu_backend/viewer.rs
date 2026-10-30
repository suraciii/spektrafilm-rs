//! Read-only retained source windows and the native viewer presentation boundary.
use super::paged::ImagePage;
use super::*;
use spektrafilm_backend::{
    DeviceImage, DisplayTransformGpuParams, SampleDomain, ViewerComposeGpuParams,
    ViewerFullSourceGpuParams, ViewerIccGpuParams,
};

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
        self.check_cancelled()?;
        self.buffer_payload(image)?;
        let count = width
            .checked_mul(height)
            .filter(|n| *n > 0)
            .ok_or("invalid viewer fit dimensions")?;
        let mut pages = Vec::new();
        let mut first = 0;
        while first < count {
            self.check_cancelled()?;
            let mut n =
                (count - first).min((self.viewer_limit() / 12).min(u64::from(u32::MAX)) as u32);
            let window = loop {
                let (x0, x1, y0, y1) = output_extent(first, n, width);
                let sample = |v: u32, src: u32, out: u32| {
                    ((v as f32 + 0.5) * src as f32 / out as f32 - 0.5).clamp(0.0, (src - 1) as f32)
                };
                let x =
                    (sample(x0, image.width(), width).floor().max(0.0) as u32).saturating_sub(1);
                let y =
                    (sample(y0, image.height(), height).floor().max(0.0) as u32).saturating_sub(1);
                let right =
                    (sample(x1, image.width(), width).floor() as u32 + 3).min(image.width());
                let bottom =
                    (sample(y1, image.height(), height).floor() as u32 + 3).min(image.height());
                let w = [x, y, right - x, bottom - y];
                if u64::from(w[2]) * u64::from(w[3]) * 12 <= self.viewer_limit() {
                    break w;
                }
                if n == 1 {
                    return Err("viewer bilinear footprint exceeds device limits".into());
                }
                n = n.div_ceil(2);
            };
            let source = self.viewer_window(image, window)?;
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
            let out = self.viewer_dispatch(
                include_str!("../../../spektrafilm-shaders/wgsl/viewer_resample_retained.wgsl"),
                &words,
                &[&source],
                n,
                "viewer_fit",
            )?;
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
        let mut pages = Vec::new();
        let mut first = 0;
        while first < count {
            self.check_cancelled()?;
            let mut n =
                (count - first).min((self.viewer_limit() / 12).min(u64::from(u32::MAX)) as u32);
            let window = loop {
                let w = source_window(
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
                );
                if u64::from(w[2]) * u64::from(w[3]) * 12 <= self.viewer_limit() {
                    break w;
                }
                if n == 1 {
                    return Err("viewer interpolation footprint exceeds device limits".into());
                }
                n = n.div_ceil(2);
            };
            let source = self.viewer_window(image, window)?;
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
            words.extend([window[0], window[1], window[2], first, n]);
            let out = self.viewer_dispatch(
                include_str!("../../../spektrafilm-shaders/wgsl/viewer_full_source_retained.wgsl"),
                &words,
                &[&source, &first_table, &second_table],
                n,
                "viewer_full_source",
            )?;
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
            Some((native, self.buffer_payload(native)?, thresholds))
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
            if let Some((native, payload, thresholds)) = clipping {
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
                        let Some([lo, hi]) = mapping.bounds(coordinate) else {
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
                        let batch_limit = (262_144 / n).min(32).max(1);
                        let mut block = 0;
                        while block < blocks {
                            self.check_cancelled()?;
                            let batch = (blocks - block).min(batch_limit);
                            let clip_words = [
                                native.width(),
                                n,
                                page.first_pixel,
                                page.n_pixels,
                                (thresholds[0] as f32).to_bits(),
                                (thresholds[1] as f32).to_bits(),
                                first,
                                p.width,
                                x0,
                                y0,
                                x_len,
                                block,
                                batch,
                                0,
                                0,
                                0,
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
                let clip_words = [n, mode, blue, red, first, p.width, x0, y0, x_len, 0, 0, 0];
                self.viewer_dispatch_into(
                    include_str!("../../../spektrafilm-shaders/wgsl/viewer_clipping_apply.wgsl"),
                    &clip_words,
                    &[&mask, &geometry],
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
