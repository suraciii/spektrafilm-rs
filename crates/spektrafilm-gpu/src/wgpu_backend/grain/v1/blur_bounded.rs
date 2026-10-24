//! Device-only line paging for the reference Gaussian. RGB intermediates are
//! physically paged copy transports, never whole-image storage bindings. FIR
//! pages include reflected support; IIR pages carry state across both sweeps.
// Per-binding minima: FIR 4*(2*radius+1) bytes (at most 76 for sigma<3),
// IIR 12 bytes (three carried f32 states), identity 4 bytes. No full line
// must fit. Live image storage is one paged 12*width*height intermediate,
// reusable across sequential jobs, plus two P-byte pages and at most
// min(P, 12*max(width,height)) state bytes, where P<=the effective limit.
// Kernel bindings are <=76 bytes; each encoded page owns a 48-byte uniform.
use super::*;
use crate::wgpu_backend::paged::ImagePage;

// Bound serial recurrence length and aggregate work independently of generous
// physical binding limits. Paging only transports samples and carried states.
const MAX_PAGE_COORDINATES: u32 = 4096;
const MAX_PAGE_VALUES: u32 = 65536;

#[derive(Clone)]
struct Store(Vec<BytePage>);

#[derive(Clone)]
struct BytePage { first: u64, bytes: u64, buffer: wgpu::Buffer }

impl Store {
    fn contiguous(buffer: &wgpu::Buffer) -> Self {
        Self(vec![BytePage { first: 0, bytes: buffer.size(), buffer: buffer.clone() }])
    }

    fn image(pages: &[ImagePage]) -> Self {
        Self(pages.iter().map(|p| BytePage { first: u64::from(p.first_pixel) * 12,
            bytes: u64::from(p.n_pixels) * 12, buffer: p.buffer.clone() }).collect())
    }

    fn allocate(backend: &WgpuBackend, bytes: u64, limit: u64) -> Self {
        let capacity = limit / 4 * 4;
        let mut first = 0;
        let mut pages = Vec::new();
        while first < bytes {
            let len = capacity.min(bytes - first);
            pages.push(BytePage { first, bytes: len,
                buffer: transport(backend, "gaussian_image_page", len, false) });
            first += len;
        }
        Self(pages)
    }

    fn validate(&self, name: &str, offset: u64, bytes: u64, usage: wgpu::BufferUsages,
        max: u64) -> Result<(), String> {
        let mut end = 0u64;
        for page in &self.0 {
            if page.first != end || page.bytes == 0 || page.bytes > page.buffer.size()
                || page.buffer.size() > max || !page.buffer.usage().contains(usage) {
                return Err(format!("Gaussian bounded blur {name} requires contiguous copy pages with {usage:?} within max_buffer_size {max}"));
            }
            end = end.checked_add(page.bytes).ok_or("Gaussian copy page extent overflows")?;
        }
        if offset.checked_add(bytes).is_none_or(|value| value > end) {
            return Err(format!("Gaussian bounded blur {name} needs {bytes} bytes at offset {offset}; page extent is {end}"));
        }
        Ok(())
    }

    fn copy(&self, encoder: &mut ObservedEncoder, mut offset: u64,
        contiguous: &wgpu::Buffer, mut other: u64, mut bytes: u64, scatter: bool) {
        let mut index = self.0.partition_point(|page| page.first + page.bytes <= offset);
        while bytes != 0 {
            let page = &self.0[index];
            let local = offset - page.first;
            let count = bytes.min(page.bytes - local);
            if scatter { encoder.copy_buffer_to_buffer(contiguous, other, &page.buffer, local, count); }
            else { encoder.copy_buffer_to_buffer(&page.buffer, local, contiguous, other, count); }
            offset += count;
            other += count;
            bytes -= count;
            index += 1;
        }
    }
}

const LAYOUT: [wgpu::BufferBindingType; 5] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    count: u32,
    lines: u32,
    stride: u32,
    channel: u32,
    radius: u32,
    mode: u32,
    initialize: u32,
    axis_major: u32,
    a: f32,
    b1: f32,
    b2: f32,
    b3: f32,
}

struct Page {
    _params: wgpu::Buffer,
    group: wgpu::BindGroup,
    params: Params,
    first: u32,
    line: u32,
}

struct Direction {
    source: Store,
    destination: Store,
    source_offset: u64,
    destination_offset: u64,
    channel: u32,
    vertical: bool,
    pages: Vec<Page>,
}

pub(super) struct BoundedBlur {
    width: u32,
    height: u32,
    pipeline: CachedPipelineRef,
    input: wgpu::Buffer,
    output: wgpu::Buffer,
    _state: wgpu::Buffer,
    _intermediate: Store,
    directions: Vec<Direction>,
    copies: Vec<(Store, Store, u64, u64, u32)>,
}

fn transport(backend: &WgpuBackend, name: &str, bytes: u64, storage: bool) -> wgpu::Buffer {
    backend.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(name),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST
            | if storage { wgpu::BufferUsages::STORAGE } else { wgpu::BufferUsages::empty() },
        mapped_at_creation: false,
    })
}


impl BoundedBlur {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build(backend: &WgpuBackend, width: u32, height: u32,
        source: &wgpu::Buffer, destination: &wgpu::Buffer, scratch: Option<&wgpu::Buffer>, sigma: [f64; 3],
        source_offset: u32, destination_offset: u32, limit: u64) -> Result<Self, String> {
        Self::build_stores(backend, width, height, Store::contiguous(source),
            Store::contiguous(destination), scratch.map(Store::contiguous), sigma,
            u64::from(source_offset), u64::from(destination_offset), limit)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_paged(backend: &WgpuBackend, width: u32, height: u32,
        source: &[ImagePage], destination: &[ImagePage], scratch: Option<&[ImagePage]>, sigma: [f64; 3],
        source_offset: u64, destination_offset: u64, limit: u64) -> Result<Self, String> {
        Self::build_stores(backend, width, height, Store::image(source), Store::image(destination),
            scratch.map(Store::image), sigma, source_offset, destination_offset, limit)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_stores(backend: &WgpuBackend, width: u32, height: u32,
        source: Store, destination: Store, scratch: Option<Store>, sigma: [f64; 3],
        source_offset: u64, destination_offset: u64, limit: u64) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("Gaussian blur dimensions must be nonzero".into());
        }
        if width > i32::MAX as u32 / 2 || height > i32::MAX as u32 / 2 {
            return Err("Gaussian blur dimensions exceed the reflection coordinate range".into());
        }
        let bytes = u64::from(width).checked_mul(u64::from(height)).and_then(|v| v.checked_mul(12))
            .ok_or("Gaussian paged blur sample size overflows")?;
        let max = backend.device.limits().max_buffer_size;
        let limit = limit.min(max).min(u64::from(backend.device.limits().max_storage_buffer_binding_size));
        if sigma.iter().any(|s| !s.is_finite()) {
            return Err("Gaussian blur sigma values must be finite".into());
        }
        let source_byte = source_offset.checked_mul(4).ok_or("Gaussian source sample offset overflows")?;
        let destination_byte = destination_offset.checked_mul(4).ok_or("Gaussian destination sample offset overflows")?;
        source.validate("source", source_byte, bytes, wgpu::BufferUsages::COPY_SRC, max)?;
        destination.validate("destination", destination_byte, bytes,
            wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, max)?;
        let minimum = sigma.iter().try_fold(4u64, |minimum, &s| -> Result<u64, String> {
            if s <= 0.0 { return Ok(minimum); }
            if from_f64(s) < from_f64(3.0) {
                let (_, weights) = gaussian_kernel(s)?;
                Ok(minimum.max(weights.len() as u64 * 4))
            } else { Ok(minimum.max(12)) }
        })?;
        if limit < minimum {
            return Err(format!("Gaussian bounded blur minimum working set requires {minimum} bytes per storage binding; limit is {limit}"));
        }
        let page_bytes = (limit / 4 * 4).min(bytes.saturating_add(
            minimum.saturating_sub(4) * u64::from(width.max(height)) * 3,
        )).min(u64::from(MAX_PAGE_VALUES) * 12).max(minimum);
        let input = transport(backend, "gaussian_page_input", page_bytes, true);
        let output = transport(backend, "gaussian_page_output", page_bytes, true);
        let state = transport(backend, "gaussian_line_state",
            page_bytes.min(u64::from(width.max(height)) * 12), true);
        let intermediate = match scratch.filter(|store| store.validate("scratch", 0, bytes,
            wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, max).is_ok()) {
            Some(store) => store,
            None => Store::allocate(backend, bytes, limit),
        };
        let pipeline = backend.cached_pipeline(include_str!("blur_bounded.wgsl"), &LAYOUT);
        let mut directions = Vec::new();
        let mut copies = Vec::new();
        for channel in 0..3u32 {
            let s = sigma[channel as usize];
            if s <= 0.0 {
                copies.push((source.clone(), destination.clone(), source_offset * 4,
                    destination_offset * 4, channel));
                continue;
            }
            let fir = from_f64(s) < from_f64(3.0);
            let (radius, weights) = if fir { gaussian_kernel(s)? } else { (0, vec![0.0]) };
            let coefficients = if fir { (0.0, 0.0, 0.0, 0.0) } else { iir_coefficients(s) };
            if ![coefficients.0, coefficients.1, coefficients.2, coefficients.3].iter().all(|v| v.is_finite()) {
                return Err("Gaussian IIR coefficients exceed the f32 device range".into());
            }
            let kernel = backend.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gaussian_page_kernel"), contents: bytemuck::cast_slice(&weights),
                usage: wgpu::BufferUsages::STORAGE,
            });
            for first_axis in [true, false] {
                let vertical = first_axis == fir;
                let (src, dst, src_offset, dst_offset) = if first_axis {
                    (&source, &intermediate, source_offset, 0)
                } else { (&intermediate, &destination, 0, destination_offset) };
                let length = if vertical { height } else { width };
                let line_count = if vertical { width } else { height };
                let halo = 2 * radius;
                // RGB transport coalesces entire horizontal spans and batches
                // adjacent columns. Tiny limits use packed single-channel pages.
                let stride = if page_bytes / 12 > u64::from(halo) { 3 } else { 1 };
                let capacity = (page_bytes / (u64::from(stride) * 4))
                    .min(u64::from(MAX_PAGE_VALUES)) as u32;
                let count = length.min(capacity - halo).min(MAX_PAGE_COORDINATES);
                let lines = (capacity / (count + halo)).min((page_bytes / 12) as u32)
                    .min(backend.device.limits().max_compute_workgroups_per_dimension.saturating_mul(64))
                    .min(line_count).max(1);
                let mut pages = Vec::new();
                let mut line = 0;
                while line < line_count {
                    let batch = lines.min(line_count - line);
                    for mode in if fir { 0..1 } else { 1..3 } {
                        let page_count = length.div_ceil(count);
                        for ordinal in 0..page_count {
                            let page = if mode == 2 { page_count - 1 - ordinal } else { ordinal };
                            let first = page * count;
                            let params = Params {
                                count: count.min(length - first), lines: batch, stride,
                                channel: if stride == 3 { channel } else { 0 }, radius,
                                mode, initialize: u32::from(ordinal == 0), axis_major: u32::from(vertical),
                                a: coefficients.0, b1: coefficients.1, b2: coefficients.2, b3: coefficients.3,
                            };
                            let uniform = backend.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("gaussian_page_params"), contents: bytemuck::bytes_of(&params),
                                usage: wgpu::BufferUsages::UNIFORM,
                            });
                            let group = backend.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("gaussian_page_group"), layout: &pipeline.layout,
                                entries: &[
                                    wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                                    wgpu::BindGroupEntry { binding: 1, resource: input.as_entire_binding() },
                                    wgpu::BindGroupEntry { binding: 2, resource: output.as_entire_binding() },
                                    wgpu::BindGroupEntry { binding: 3, resource: kernel.as_entire_binding() },
                                    wgpu::BindGroupEntry { binding: 4, resource: state.as_entire_binding() },
                                ],
                            });
                            pages.push(Page { _params: uniform, group, params, first, line });
                        }
                    }
                    line += batch;
                }
                directions.push(Direction { source: src.clone(), destination: dst.clone(),
                    source_offset: src_offset, destination_offset: dst_offset, channel, vertical, pages });
            }
        }
        Ok(Self { width, height, pipeline, input, output, _state: state,
            _intermediate: intermediate, directions, copies })
    }

    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        for direction in &self.directions {
            for page in &direction.pages {
                backend.check_cancelled()?;
                let mut encoder = backend.device.create_command_encoder(&Default::default());
                self.encode_page(&mut encoder, direction, page);
                backend.check_cancelled()?;
                backend.queue.submit(Some(encoder.finish()));
            }
        }
        let pixels = u64::from(self.width) * u64::from(self.height);
        let capacity = (self.input.size() / 12).clamp(1, 4096);
        for copy in &self.copies {
            let mut first = 0;
            while first < pixels {
                backend.check_cancelled()?;
                let mut encoder = backend.device.create_command_encoder(&Default::default());
                let count = capacity.min(pixels - first);
                self.encode_identity(&mut encoder, copy, first, count);
                backend.check_cancelled()?;
                backend.queue.submit(Some(encoder.finish()));
                first += count;
            }
        }
        backend.check_cancelled()
    }

    fn encode_page(&self, encoder: &mut ObservedEncoder, direction: &Direction, page: &Page) {
        let length = if direction.vertical { self.height } else { self.width };
        let p = page.params;
        let backward = p.mode == 2;
        let source = if backward { &direction.destination } else { &direction.source };
        let source_offset = if backward { direction.destination_offset } else { direction.source_offset };
        self.gather(encoder, source, source_offset, &self.input, direction, page,
            p.count + 2 * p.radius, i64::from(page.first) - i64::from(p.radius), length);
        // Preserve other RGB channels when transporting a packed span.
        if p.stride == 3 {
            self.gather(encoder, &direction.destination, direction.destination_offset,
                &self.output, direction, page, p.count, i64::from(page.first), length);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gaussian_bounded_line"), timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline.pipeline);
            pass.set_bind_group(0, &page.group, &[]);
            pass.dispatch_workgroups(p.lines.div_ceil(64), 1, 1);
        }
        self.scatter(encoder, direction, page);
    }

    fn encode_identity(&self, encoder: &mut ObservedEncoder,
        copy: &(Store, Store, u64, u64, u32), first: u64, count: u64) {
        let (source, destination, src, dst, channel) = copy;
        for pixel in first..first + count {
            source.copy(encoder, src + pixel * 12 + u64::from(*channel) * 4,
                &self.input, 0, 4, false);
            destination.copy(encoder, dst + pixel * 12 + u64::from(*channel) * 4,
                &self.input, 0, 4, true);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn gather(&self, encoder: &mut ObservedEncoder, source: &Store, source_offset: u64,
        scratch: &wgpu::Buffer, direction: &Direction, page: &Page, count: u32, first: i64, length: u32) {
        let p = page.params;
        let mut run: Option<(u64, u64, u64)> = None;
        for outer in 0..if direction.vertical { count } else { p.lines } {
            for inner in 0..if direction.vertical { p.lines } else { count } {
                let (coordinate, line) = if direction.vertical { (outer, inner) } else { (inner, outer) };
                let coordinate = reflect(first + i64::from(coordinate), length);
                let line = page.line + line;
                let (x, y) = if direction.vertical { (line, coordinate) } else { (coordinate, line) };
                let source_byte = (source_offset + (u64::from(y) * u64::from(self.width) + u64::from(x)) * 3
                    + if p.stride == 1 { u64::from(direction.channel) } else { 0 }) * 4;
                let scratch_byte = (u64::from(outer) * u64::from(if direction.vertical { p.lines } else { count })
                    + u64::from(inner)) * u64::from(p.stride) * 4;
                let bytes = u64::from(p.stride) * 4;
                if let Some((src, dst, size)) = &mut run
                    && *src + *size == source_byte && *dst + *size == scratch_byte {
                    *size += bytes;
                    continue;
                }
                if let Some((src, dst, size)) = run.take() {
                    source.copy(encoder, src, scratch, dst, size, false);
                }
                run = Some((source_byte, scratch_byte, bytes));
            }
        }
        if let Some((src, dst, size)) = run {
            source.copy(encoder, src, scratch, dst, size, false);
        }
    }

    fn scatter(&self, encoder: &mut ObservedEncoder, direction: &Direction, page: &Page) {
        let p = page.params;
        let mut run: Option<(u64, u64, u64)> = None;
        for outer in 0..if direction.vertical { p.count } else { p.lines } {
            for inner in 0..if direction.vertical { p.lines } else { p.count } {
                let (coordinate, line) = if direction.vertical { (outer, inner) } else { (inner, outer) };
                let (x, y) = if direction.vertical { (page.line + line, page.first + coordinate) }
                    else { (page.first + coordinate, page.line + line) };
                let destination_byte = (direction.destination_offset
                    + (u64::from(y) * u64::from(self.width) + u64::from(x)) * 3
                    + if p.stride == 1 { u64::from(direction.channel) } else { 0 }) * 4;
                let scratch_byte = (u64::from(outer) * u64::from(if direction.vertical { p.lines } else { p.count })
                    + u64::from(inner)) * u64::from(p.stride) * 4;
                let bytes = u64::from(p.stride) * 4;
                if let Some((src, dst, size)) = &mut run
                    && *src + *size == scratch_byte && *dst + *size == destination_byte {
                    *size += bytes;
                    continue;
                }
                if let Some((src, dst, size)) = run.take() {
                    direction.destination.copy(encoder, dst, &self.output, src, size, true);
                }
                run = Some((scratch_byte, destination_byte, bytes));
            }
        }
        if let Some((src, dst, size)) = run {
            direction.destination.copy(encoder, dst, &self.output, src, size, true);
        }
    }
}

fn reflect(index: i64, length: u32) -> u32 {
    let period = 2 * i64::from(length);
    let folded = index.rem_euclid(period);
    if folded < i64::from(length) { folded as u32 } else { (period - 1 - folded) as u32 }
}

#[cfg(all(test, not(feature = "precision-f64")))]
mod tests {
    use super::*;
    use spektrafilm_math::gaussian::gaussian_blur_channel;

    fn compare(width: u32, height: u32, sigma: [f64; 3], limit: u64) {
        let gpu = crate::test_support::adapter();
        let samples = width as usize * height as usize * 3;
        let source_offset = 5;
        let destination_offset = 7;
        // Strong true-endpoint impulses plus a nonperiodic interior distinguish
        // recurrence restarts, missing FIR halo and flipped pass order.
        let mut source = vec![0.0f32; samples + source_offset as usize + 4];
        for (i, value) in source[source_offset as usize..source_offset as usize + samples].iter_mut().enumerate() {
            *value = ((i * 37 + i * i * 13) % 103) as f32 / 103.0;
        }
        source[source_offset as usize] = 7.0;
        source[source_offset as usize + samples - 1] = -3.0;
        let input = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bounded_gaussian_test_input"), contents: bytemuck::cast_slice(&source),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let sentinel = vec![123.0f32; samples + destination_offset as usize + 4];
        let make_output = || gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bounded_gaussian_test_output"), contents: bytemuck::cast_slice(&sentinel),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        });
        let output = make_output();
        let untiled = make_output();
        let state = BoundedBlur::build(&gpu, width, height, &input, &output, None, sigma,
            source_offset, destination_offset, limit).unwrap();
        assert!(state.input.size() <= limit);
        assert!(state.output.size() <= limit);
        assert!(state._state.size() <= limit);
        assert!(state._intermediate.0.iter().all(|p| p.bytes <= limit
            && !p.buffer.usage().contains(wgpu::BufferUsages::STORAGE)));
        let direct = build_grain_blur(&gpu, width, height, &input, &untiled, sigma,
            source_offset, destination_offset).unwrap();
        let bytes = sentinel.len() as u64 * 4;
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded_gaussian_test_readback"), size: bytes * 2,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        state.submit(&gpu).unwrap();
        direct.submit(&gpu).unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        encoder.copy_buffer_to_buffer(&untiled, 0, &readback, bytes, bytes);
        gpu.queue.submit(Some(encoder.finish()));
        let actual = gpu.read_mapped_f32(&readback, bytes * 2);
        let (tiled, whole) = actual.split_at(sentinel.len());
        for channel in 0..3 {
            let input_channel: Vec<Scalar> = (0..samples / 3)
                .map(|pixel| source[source_offset as usize + pixel * 3 + channel]).collect();
            let expected = gaussian_blur_channel(&input_channel, width, height, sigma[channel]);
            for (pixel, expected) in expected.into_iter().enumerate() {
                let index = destination_offset as usize + pixel * 3 + channel;
                assert!((tiled[index] - expected).abs() < 2e-5,
                    "limit {limit} {width}x{height} channel {channel} pixel {pixel}: {} != {expected}", tiled[index]);
                assert!((tiled[index] - whole[index]).abs() < 2e-5,
                    "paged/direct mismatch channel {channel} pixel {pixel}");
            }
        }
        assert!(tiled[..destination_offset as usize].iter().all(|&x| x == 123.0));
        assert!(tiled[destination_offset as usize + samples..].iter().all(|&x| x == 123.0));
    }

    #[test]
    fn paged_iir_preserves_both_true_endpoints_and_recursion_seams() {
        compare(71, 67, [3.0, 8.0, 20.0], 192);
        compare(1, 39, [3.0, 8.0, 20.0], 12);
        compare(39, 1, [3.0, 8.0, 20.0], 12);
    }

    #[test]
    fn paged_fir_reflects_true_support_and_keeps_mixed_channel_order() {
        compare(29, 23, [2.99, 3.5, 0.0], 76);
        compare(7, 3, [2.99, 0.4, 0.1667], 76);
        compare(1, 1, [2.99, 3.0, 0.0], 76);
    }

    #[test]
    fn column_batches_match_reference_without_per_pixel_channel_transport() {
        compare(37, 5, [0.4, 3.0, 0.0], 192);
    }
}

#[cfg(test)]
mod working_set_tests {
    use super::*;

    #[test]
    fn impossible_minimum_reports_required_binding_bytes() {
        let gpu = crate::test_support::adapter();
        let source = transport(&gpu, "gaussian_minimum_source", 12, false);
        let destination = transport(&gpu, "gaussian_minimum_destination", 12, false);
        for (sigma, limit, minimum) in [([2.99, 0.0, 0.0], 75, 76), ([3.0, 0.0, 0.0], 11, 12)] {
            let error = match BoundedBlur::build(&gpu, 1, 1, &source, &destination, None,
                sigma, 0, 0, limit) {
                Ok(_) => panic!("binding limit {limit} accepted below minimum {minimum}"),
                Err(error) => error,
            };
            assert!(error.contains(&format!("requires {minimum} bytes")), "{error}");
            assert!(error.contains(&format!("limit is {limit}")), "{error}");
        }
    }
}

#[cfg(all(test, not(feature = "precision-f64")))]
mod physical_page_tests {
    use super::*;
    use spektrafilm_math::gaussian::gaussian_blur_channel;

    #[test]
    fn physical_pages_preserve_layer_offsets_axis_seams_and_neighbours() {
        let gpu = crate::test_support::adapter();
        let (width, height) = (29, 23);
        let pixels = (width * height) as usize;
        let source_sample = 9u64;
        let destination_sample = 6u64;
        let mut values: Vec<f32> = (0..pixels * 3 + 15)
            .map(|i| ((i * 37 + i * i * 13) % 103) as f32 / 103.0).collect();
        values[source_sample as usize] = 7.0;
        values[source_sample as usize + pixels * 3 - 1] = -3.0;
        let split = |samples: &[f32], page_pixels: usize| -> Vec<ImagePage> {
            samples.chunks(page_pixels * 3).enumerate().map(|(index, samples)| ImagePage {
                first_pixel: (index * page_pixels) as u32,
                n_pixels: (samples.len() / 3) as u32,
                buffer: gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("gaussian_physical_test_page"), contents: bytemuck::cast_slice(samples),
                    usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                }),
            }).collect()
        };
        let source = split(&values, 7);
        let sentinel = vec![123.0f32; pixels * 3 + 15];
        let destination = split(&sentinel, 11);
        let sigma = [2.99, 8.0, 0.0];
        let state = BoundedBlur::build_paged(&gpu, width, height, &source, &destination,
            None, sigma, source_sample, destination_sample, 76).unwrap();
        assert!(state._intermediate.0.len() > 1);
        assert!(state._intermediate.0.iter().all(|p| p.bytes <= 76));
        let bytes = sentinel.len() as u64 * 4;
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gaussian_physical_test_readback"), size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        state.submit(&gpu).unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        Store::image(&destination).copy(&mut encoder, 0, &readback, 0, bytes, false);
        gpu.queue.submit(Some(encoder.finish()));
        let actual = gpu.read_mapped_f32(&readback, bytes);
        for channel in 0..3 {
            let input: Vec<Scalar> = (0..pixels)
                .map(|pixel| values[source_sample as usize + pixel * 3 + channel]).collect();
            let expected = gaussian_blur_channel(&input, width, height, sigma[channel]);
            for (pixel, expected) in expected.into_iter().enumerate() {
                let got = actual[destination_sample as usize + pixel * 3 + channel];
                assert!((got - expected).abs() < 2e-5,
                    "physical page seam channel {channel} pixel {pixel}: {got} != {expected}");
            }
        }
        assert!(actual[..destination_sample as usize].iter().all(|&x| x == 123.0));
        assert!(actual[destination_sample as usize + pixels * 3..].iter().all(|&x| x == 123.0));
    }
}
