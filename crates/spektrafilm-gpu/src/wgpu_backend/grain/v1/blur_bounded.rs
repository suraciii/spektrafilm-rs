//! Bounded reference Gaussian with raw-u32 GPU gather/scatter. Physical image
//! pages and layer offsets are byte-addressed on the host; kernels see only
//! aligned, capped views and packed target-channel samples. Both sample widths
//! retain f64 kernels and recursive state; f32 stores narrow at upstream's
//! original tap/pass boundaries rather than at device-page seams.
use super::*;
use crate::wgpu_backend::paged::ImagePage;

const MAX_PAGE_COORDINATES: u32 = 4096;
const TARGET_PAGE_COORDINATES: u32 = 64;
const MAX_PAGE_VALUES: u32 = 1 << 20;
const MAX_COMMANDS_PER_SUBMIT: u32 = 128;

/// Additional per-job buffers with caller-owned image intermediate/output.
/// The bound includes one encoded batch and two submitted batches of uniforms.
pub(in crate::wgpu_backend) fn workspace_bytes(
    width: u32,
    height: u32,
    limit: u64,
) -> Result<u64, String> {
    workspace_bytes_for_samples(width, height, limit, 4)
}

/// Additional binary64 filter workspace, excluding the HWC24 intermediate.
pub(in crate::wgpu_backend) fn workspace_bytes_f64(
    width: u32,
    height: u32,
    limit: u64,
) -> Result<u64, String> {
    workspace_bytes_for_samples(width, height, limit, 8)
}

fn workspace_bytes_for_samples(
    width: u32,
    height: u32,
    limit: u64,
    sample_bytes: u64,
) -> Result<u64, String> {
    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(3 * sample_bytes))
        .ok_or("Gaussian workspace geometry overflows")?;
    // The box halo can exceed the image itself, up to the binding limit.
    let page = limit.min(u64::from(MAX_PAGE_VALUES) * sample_bytes);
    // Input, output, adapter, line state, three kernels and retained uniforms.
    Ok(page * 4 + 3 * 152 + u64::from(MAX_COMMANDS_PER_SUBMIT) * 3 * 96)
}
#[derive(Clone)]
struct Store(Vec<BytePage>);

#[derive(Clone)]
struct BytePage {
    first: u64,
    bytes: u64,
    buffer: wgpu::Buffer,
}

impl Store {
    fn contiguous(buffer: &wgpu::Buffer) -> Self {
        Self(vec![BytePage {
            first: 0,
            bytes: buffer.size(),
            buffer: buffer.clone(),
        }])
    }

    fn image(pages: &[ImagePage]) -> Self {
        Self(
            pages
                .iter()
                .map(|p| BytePage {
                    first: u64::from(p.first_pixel) * 12,
                    bytes: u64::from(p.n_pixels) * 12,
                    buffer: p.buffer.clone(),
                })
                .collect(),
        )
    }

    fn raster(storage: &(impl super::super::super::raster::RasterStorage + ?Sized)) -> Self {
        let mut pages = Vec::new();
        storage.visit(&mut |first, buffer| {
            pages.push(BytePage {
                first,
                bytes: buffer.size(),
                buffer: buffer.clone(),
            })
        });
        Self(pages)
    }

    fn allocate(backend: &WgpuBackend, bytes: u64, limit: u64) -> Self {
        let mut pages = Vec::new();
        let mut first = 0;
        while first < bytes {
            let len = (limit / 4 * 4).min(bytes - first);
            pages.push(BytePage {
                first,
                bytes: len,
                buffer: transport(backend, "gaussian_image_page", len, true),
            });
            first += len;
        }
        Self(pages)
    }

    fn validate(
        &self,
        name: &str,
        offset: u64,
        bytes: u64,
        usage: wgpu::BufferUsages,
        max: u64,
    ) -> Result<(), String> {
        let mut end = 0u64;
        for page in &self.0 {
            if page.first != end
                || page.bytes == 0
                || page.bytes % 4 != 0
                || page.bytes > page.buffer.size()
                || page.buffer.size() > max
                || !page.buffer.usage().contains(usage)
            {
                return Err(format!(
                    "Gaussian bounded blur {name} requires contiguous copy pages with {usage:?} within max_buffer_size {max}"
                ));
            }
            end = end
                .checked_add(page.bytes)
                .ok_or("Gaussian copy page extent overflows")?;
        }
        if offset.checked_add(bytes).is_none_or(|value| value > end) {
            return Err(format!(
                "Gaussian bounded blur {name} needs {bytes} bytes at offset {offset}; page extent is {end}"
            ));
        }
        Ok(())
    }

    /// Find only the physical page and capped view containing this support
    /// sample. Never enumerate unrelated page pairs or intervening row gaps.
    fn view(&self, sample: u64, limit: u64, alignment: u64, adapter_bytes: u64) -> View<'_> {
        let byte = sample * 4;
        let page = &self.0[self.0.partition_point(|p| p.first + p.bytes <= byte)];
        let direct = page.buffer.usage().contains(wgpu::BufferUsages::STORAGE)
            && (page.bytes <= limit || limit >= alignment);
        let capacity = if direct && page.bytes > limit {
            limit / alignment * alignment
        } else if direct {
            limit / 4 * 4
        } else {
            adapter_bytes
        };
        let local = (byte - page.first) / capacity * capacity;
        View {
            page,
            local,
            first: (page.first + local) / 4,
            bytes: capacity.min(page.bytes - local),
            direct,
        }
    }

    #[cfg(all(test, not(feature = "precision-f64")))]
    fn copy(
        &self,
        encoder: &mut ObservedEncoder,
        mut offset: u64,
        contiguous: &wgpu::Buffer,
        mut other: u64,
        mut bytes: u64,
        scatter: bool,
    ) {
        let mut index = self
            .0
            .partition_point(|page| page.first + page.bytes <= offset);
        while bytes != 0 {
            let page = &self.0[index];
            let local = offset - page.first;
            let count = bytes.min(page.bytes - local);
            if scatter {
                encoder.copy_buffer_to_buffer(contiguous, other, &page.buffer, local, count);
            } else {
                encoder.copy_buffer_to_buffer(&page.buffer, local, contiguous, other, count);
            }
            offset += count;
            other += count;
            bytes -= count;
            index += 1;
        }
    }
}

struct View<'a> {
    page: &'a BytePage,
    local: u64,
    first: u64,
    bytes: u64,
    direct: bool,
}

const LAYOUT: [wgpu::BufferBindingType; 5] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
    wgpu::BufferBindingType::Storage { read_only: true },
    wgpu::BufferBindingType::Storage { read_only: false },
];
const COPY_LAYOUT: [wgpu::BufferBindingType; 3] = [
    wgpu::BufferBindingType::Uniform,
    wgpu::BufferBindingType::Storage { read_only: false },
    wgpu::BufferBindingType::Storage { read_only: false },
];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TransportParams {
    count: u32,
    columns: u32,
    image_start: u32,
    image_row_step: u32,
    scratch_start: i32,
    scratch_row_step: i32,
    scratch_col_step: i32,
    scatter: u32,
    sample_words: u32,
    _pad: [u32; 3],
}

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
    a: f64,
    b1: f64,
    b2: f64,
    b3: f64,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params32 {
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

impl From<Params> for Params32 {
    fn from(p: Params) -> Self {
        let b1 = p.b1 as f32;
        let b2 = p.b2 as f32;
        let b3 = p.b3 as f32;
        Self {
            count: p.count,
            lines: p.lines,
            stride: p.stride,
            channel: p.channel,
            radius: p.radius,
            mode: p.mode,
            initialize: p.initialize,
            axis_major: p.axis_major,
            a: 1.0 - ((b1 + b2) + b3),
            b1,
            b2,
            b3,
        }
    }
}

struct Page {
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
    box_filter: bool,
    kernel: wgpu::Buffer,
    pages: Vec<Page>,
}

pub(super) struct BoundedBlur {
    width: u32,
    height: u32,
    limit: u64,
    sample_bytes: u64,
    arithmetic_f64: bool,
    pipeline: CachedPipelineRef,
    box_pipeline: CachedPipelineRef,
    copy_pipeline: CachedPipelineRef,
    input: wgpu::Buffer,
    output: wgpu::Buffer,
    adapter: wgpu::Buffer,
    _state: wgpu::Buffer,
    _intermediate: Store,
    directions: Vec<Direction>,
    identities: Vec<(Store, Store, u64, u64, u32)>,
}

fn transport(backend: &WgpuBackend, name: &str, bytes: u64, storage: bool) -> wgpu::Buffer {
    backend.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(name),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST
            | if storage {
                wgpu::BufferUsages::STORAGE
            } else {
                wgpu::BufferUsages::empty()
            },
        mapped_at_creation: false,
    })
}

/// Bounds actual encoded commands and in-flight resources, rather than counting
/// pages whose physical intersections can vary greatly. Queue order preserves
/// recurrence and scratch dependencies across gather/scatter batch boundaries.
struct Submission<'a> {
    backend: &'a WgpuBackend,
    encoder: ObservedEncoder,
    pending: u32,
}

impl<'a> Submission<'a> {
    fn new(backend: &'a WgpuBackend) -> Self {
        Self {
            backend,
            encoder: backend.device.create_command_encoder(&Default::default()),
            pending: 0,
        }
    }
    fn command(&mut self) -> Result<(), String> {
        self.pending += 1;
        self.backend.check_cancelled()?;
        if self.pending >= MAX_COMMANDS_PER_SUBMIT {
            self.flush()?;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), String> {
        self.backend.check_cancelled()?;
        if self.pending != 0 {
            let encoder = std::mem::replace(
                &mut self.encoder,
                self.backend
                    .device
                    .create_command_encoder(&Default::default()),
            );
            self.backend.submit_commands(Some(encoder.finish()))?;
            self.pending = 0;
        }
        Ok(())
    }
}

impl BoundedBlur {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build(
        backend: &WgpuBackend,
        width: u32,
        height: u32,
        source: &wgpu::Buffer,
        destination: &wgpu::Buffer,
        scratch: Option<&wgpu::Buffer>,
        sigma: [f64; 3],
        source_offset: u32,
        destination_offset: u32,
        limit: u64,
    ) -> Result<Self, String> {
        Self::build_stores(
            backend,
            width,
            height,
            Store::contiguous(source),
            Store::contiguous(destination),
            scratch.map(Store::contiguous),
            sigma,
            u64::from(source_offset),
            u64::from(destination_offset),
            limit,
            4,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_paged(
        backend: &WgpuBackend,
        width: u32,
        height: u32,
        source: &[ImagePage],
        destination: &[ImagePage],
        scratch: Option<&[ImagePage]>,
        sigma: [f64; 3],
        source_offset: u64,
        destination_offset: u64,
        limit: u64,
    ) -> Result<Self, String> {
        Self::build_stores(
            backend,
            width,
            height,
            Store::image(source),
            Store::image(destination),
            scratch.map(Store::image),
            sigma,
            source_offset,
            destination_offset,
            limit,
            4,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_f64_paged(
        backend: &WgpuBackend,
        width: u32,
        height: u32,
        source: &(impl super::super::super::raster::RasterStorage + ?Sized),
        destination: &(impl super::super::super::raster::RasterStorage + ?Sized),
        sigma: [f64; 3],
        source_offset: u64,
        destination_offset: u64,
        limit: u64,
    ) -> Result<Self, String> {
        Self::build_stores(
            backend,
            width,
            height,
            Store::raster(source),
            Store::raster(destination),
            None,
            sigma,
            source_offset,
            destination_offset,
            limit,
            8,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_stores(
        backend: &WgpuBackend,
        width: u32,
        height: u32,
        source: Store,
        destination: Store,
        scratch: Option<Store>,
        sigma: [f64; 3],
        source_offset: u64,
        destination_offset: u64,
        limit: u64,
        sample_bytes: u64,
    ) -> Result<Self, String> {
        let arithmetic_f64 = backend
            .device
            .features()
            .contains(wgpu::Features::SHADER_F64);
        if sample_bytes == 8 && !arithmetic_f64 {
            return Err("binary64 Gaussian requires native shader f64 arithmetic".into());
        }
        if width == 0 || height == 0 {
            return Err("Gaussian blur dimensions must be nonzero".into());
        }
        if width > i32::MAX as u32 / 2 || height > i32::MAX as u32 / 2 {
            return Err("Gaussian blur dimensions exceed the reflection coordinate range".into());
        }
        let bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|v| v.checked_mul(3 * sample_bytes))
            .ok_or("Gaussian paged blur sample size overflows")?;
        let max = backend.device.limits().max_buffer_size;
        let limit = limit.min(max).min(u64::from(
            backend.device.limits().max_storage_buffer_binding_size,
        ));
        if sigma.iter().any(|s| !s.is_finite()) {
            return Err("Gaussian blur sigma values must be finite".into());
        }
        let source_byte = source_offset
            .checked_mul(sample_bytes)
            .ok_or("Gaussian source sample offset overflows")?;
        let destination_byte = destination_offset
            .checked_mul(sample_bytes)
            .ok_or("Gaussian destination sample offset overflows")?;
        source.validate(
            "source",
            source_byte,
            bytes,
            wgpu::BufferUsages::COPY_SRC,
            max,
        )?;
        destination.validate(
            "destination",
            destination_byte,
            bytes,
            wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            max,
        )?;
        let minimum =
            sigma
                .iter()
                .try_fold(sample_bytes, |minimum, &s| -> Result<u64, String> {
                    if s <= 0.0 {
                        Ok(minimum)
                    } else if s < 3.0 {
                        Ok(minimum.max(
                            gaussian_kernel_f64(s)?.1.len() as u64
                                * if arithmetic_f64 { 8 } else { 4 },
                        ))
                    } else if s < 8.0 {
                        Ok(minimum.max(if arithmetic_f64 { 24 } else { 12 }))
                    } else {
                        let width = spektrafilm_math::gaussian::gaussian_box_width(s);
                        Ok(minimum.max((width as u64).saturating_mul(sample_bytes)))
                    }
                })?;
        if limit < minimum {
            return Err(format!(
                "Gaussian bounded blur minimum working set requires {minimum} bytes per storage binding; limit is {limit}"
            ));
        }
        let limit = limit / sample_bytes * sample_bytes;
        let page_bytes = limit
            .min(u64::from(MAX_PAGE_VALUES) * sample_bytes)
            .min(bytes.max(minimum));
        if page_bytes < minimum {
            return Err(format!(
                "Gaussian bounded blur minimum working set requires {minimum} bytes per storage binding; page capacity is {page_bytes}"
            ));
        }
        let input = transport(backend, "gaussian_page_input", page_bytes, true);
        let output = transport(backend, "gaussian_page_output", page_bytes, true);
        // COPY-only callers and sub-alignment caps use this one shared adapter,
        // not a retained allocation or bind group per physical image page.
        let adapter = transport(backend, "gaussian_transport_adapter", page_bytes, true);
        let state = transport(
            backend,
            "gaussian_line_state",
            (page_bytes / 4 * 4)
                .max(4)
                .min(u64::from(width.max(height)) * if arithmetic_f64 { 24 } else { 12 }),
            true,
        );
        let intermediate = if sigma.iter().all(|&s| s <= 0.0) {
            Store(Vec::new())
        } else {
            scratch
                .filter(|store| {
                    store
                        .validate(
                            "scratch",
                            0,
                            bytes,
                            wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                            max,
                        )
                        .is_ok()
                })
                .unwrap_or_else(|| Store::allocate(backend, bytes, limit))
        };
        let pipeline = backend.cached_pipeline(
            if sample_bytes == 8 {
                include_str!("blur_bounded_f64.wgsl")
            } else if arithmetic_f64 {
                include_str!("blur_bounded.wgsl")
            } else {
                include_str!("blur_bounded_f32.wgsl")
            },
            &LAYOUT,
        );
        let box_pipeline = backend.cached_pipeline(
            if sample_bytes == 8 {
                include_str!("blur_bounded_box_f64.wgsl")
            } else {
                include_str!("blur_bounded_box_f32.wgsl")
            },
            &LAYOUT,
        );
        let copy_pipeline =
            backend.cached_pipeline(include_str!("blur_bounded_copy.wgsl"), &COPY_LAYOUT);
        let mut directions = Vec::new();
        let mut identities = Vec::new();
        for channel in 0..3u32 {
            let s = sigma[channel as usize];
            if s <= 0.0 {
                identities.push((
                    source.clone(),
                    destination.clone(),
                    source_offset,
                    destination_offset,
                    channel,
                ));
                continue;
            }
            let box_filter = s >= 8.0;
            let fir = s < 3.0;
            let (radius, weights) = if box_filter {
                let width = spektrafilm_math::gaussian::gaussian_box_width(s);
                let radius = u32::try_from(width / 2)
                    .map_err(|_| "Gaussian box radius exceeds device coordinate range")?;
                (radius, vec![1.0 / width as f64])
            } else if fir {
                gaussian_kernel_f64(s)?
            } else {
                (0, vec![0.0])
            };
            let coefficients = if fir || box_filter {
                (0.0, 0.0, 0.0, 0.0)
            } else {
                iir_coefficients_f64(s)
            };
            if ![
                coefficients.0,
                coefficients.1,
                coefficients.2,
                coefficients.3,
            ]
            .iter()
            .all(|v| v.is_finite())
            {
                return Err("Gaussian IIR coefficients exceed the device scalar range".into());
            }
            let kernel_bytes: Vec<u8> = if arithmetic_f64 && !box_filter {
                bytemuck::cast_slice(&weights).to_vec()
            } else {
                let weights: Vec<f32> = weights.iter().map(|&v| v as f32).collect();
                bytemuck::cast_slice(&weights).to_vec()
            };
            let kernel = backend
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("gaussian_page_kernel"),
                    contents: &kernel_bytes,
                    usage: wgpu::BufferUsages::STORAGE,
                });
            for pass in 0..if box_filter { 6 } else { 2 } {
                let vertical = if box_filter {
                    pass >= 3
                } else {
                    (pass == 0) == fir
                };
                let (src, dst, src_offset, dst_offset) = if pass == 0 {
                    (&source, &intermediate, source_offset, 0)
                } else if pass % 2 == 1 {
                    (&intermediate, &destination, 0, destination_offset)
                } else {
                    (&destination, &intermediate, destination_offset, 0)
                };
                let length = if vertical { height } else { width };
                let line_count = if vertical { width } else { height };
                let halo = 2 * radius;
                let capacity = (page_bytes / sample_bytes) as u32;
                let max_lines = (page_bytes / 24) as u32;
                // Fill the page from the available independent lines before
                // splitting coordinates. This keeps full-fit rasters together
                // and avoids tiny serial chunks when a raster has few lines.
                let target_count =
                    TARGET_PAGE_COORDINATES.max((capacity / line_count).saturating_sub(halo));
                let count = length
                    .min(target_count)
                    .min(MAX_PAGE_COORDINATES)
                    .min(capacity.saturating_sub(halo).max(1));
                let lines = (capacity / (count + halo))
                    .min(max_lines)
                    .min(line_count)
                    .max(1);
                let mut pages = Vec::new();
                let mut line = 0;
                while line < line_count {
                    backend.check_cancelled()?;
                    let batch = lines.min(line_count - line);
                    for mode in if box_filter {
                        3..4
                    } else if fir {
                        0..1
                    } else {
                        1..3
                    } {
                        let page_count = length.div_ceil(count);
                        for ordinal in 0..page_count {
                            let first = if mode == 2 {
                                page_count - 1 - ordinal
                            } else {
                                ordinal
                            } * count;
                            pages.push(Page {
                                params: Params {
                                    count: count.min(length - first),
                                    lines: batch,
                                    stride: 1,
                                    channel: 0,
                                    radius,
                                    mode,
                                    initialize: u32::from(ordinal == 0),
                                    axis_major: u32::from(vertical),
                                    a: coefficients.0,
                                    b1: coefficients.1,
                                    b2: coefficients.2,
                                    b3: coefficients.3,
                                },
                                first,
                                line,
                            });
                        }
                    }
                    line += batch;
                }
                directions.push(Direction {
                    source: src.clone(),
                    destination: dst.clone(),
                    source_offset: src_offset,
                    destination_offset: dst_offset,
                    channel,
                    vertical,
                    box_filter,
                    kernel: kernel.clone(),
                    pages,
                });
            }
        }
        Ok(Self {
            width,
            height,
            limit,
            sample_bytes,
            arithmetic_f64,
            pipeline,
            box_pipeline,
            copy_pipeline,
            input,
            output,
            adapter,
            _state: state,
            _intermediate: intermediate,
            directions,
            identities,
        })
    }

    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        let mut submission = Submission::new(backend);
        for direction in &self.directions {
            for page in &direction.pages {
                backend.check_cancelled()?;
                self.encode_page(&mut submission, direction, page)?;
            }
        }
        // Identity uses bounded packed tiles too, with raw integer loads/stores.
        // It shares no filter arithmetic and preserves NaNs and signed zero.
        let capacity = (self.input.size() / self.sample_bytes) as u32;
        for (source, destination, source_offset, destination_offset, channel) in &self.identities {
            let columns = self.width.min(capacity);
            let rows = (capacity / columns).min(self.height).max(1);
            let mut y = 0;
            while y < self.height {
                let nrows = rows.min(self.height - y);
                let mut x = 0;
                while x < self.width {
                    let ncols = columns.min(self.width - x);
                    let rect = Rect {
                        x,
                        y,
                        columns: ncols,
                        rows: nrows,
                        scratch: 0,
                        row_step: ncols as i32,
                        col_step: 1,
                    };
                    self.transfer(
                        &mut submission,
                        source,
                        *source_offset,
                        *channel,
                        &self.input,
                        rect,
                        false,
                    )?;
                    self.transfer(
                        &mut submission,
                        destination,
                        *destination_offset,
                        *channel,
                        &self.input,
                        rect,
                        true,
                    )?;
                    x += ncols;
                }
                y += nrows;
            }
        }
        submission.flush()?;
        backend.check_cancelled()
    }

    fn encode_page(
        &self,
        submission: &mut Submission<'_>,
        direction: &Direction,
        page: &Page,
    ) -> Result<(), String> {
        let p = page.params;
        let (source, offset) = if p.mode == 2 {
            (&direction.destination, direction.destination_offset)
        } else {
            (&direction.source, direction.source_offset)
        };
        let length = if direction.vertical {
            self.height
        } else {
            self.width
        };
        let count = p.count + 2 * p.radius;
        // Split only at true reflection turns. A segment is an affine image
        // rectangle, including repeated halo support for one-pixel dimensions.
        let mut j = 0;
        while j < count {
            let coordinate = i64::from(page.first) - i64::from(p.radius) + i64::from(j);
            let start = reflect(coordinate, length);
            let folded = coordinate.rem_euclid(2 * i64::from(length)) as u32;
            let reversed = folded >= length;
            let run = (if reversed {
                2 * length - folded
            } else {
                length - folded
            })
            .min(count - j);
            let low = if reversed { start + 1 - run } else { start };
            let scratch = if reversed { j + run - 1 } else { j };
            let step = if reversed { -1 } else { 1 };
            let rect = if direction.vertical {
                Rect {
                    x: page.line,
                    y: low,
                    columns: p.lines,
                    rows: run,
                    scratch: (scratch * p.lines) as i32,
                    row_step: step * p.lines as i32,
                    col_step: 1,
                }
            } else {
                Rect {
                    x: low,
                    y: page.line,
                    columns: run,
                    rows: p.lines,
                    scratch: scratch as i32,
                    row_step: count as i32,
                    col_step: step,
                }
            };
            self.transfer(
                submission,
                source,
                offset,
                direction.channel,
                &self.input,
                rect,
                false,
            )?;
            j += run;
        }
        let backend = submission.backend;
        let p32 = Params32::from(p);
        let params = if self.arithmetic_f64 && !direction.box_filter {
            bytemuck::bytes_of(&p)
        } else {
            bytemuck::bytes_of(&p32)
        };
        let uniform = backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gaussian_page_params"),
                contents: params,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let buffers = [
            &uniform,
            &self.input,
            &self.output,
            &direction.kernel,
            &self._state,
        ];
        let entries = std::array::from_fn::<_, 5, _>(|binding| wgpu::BindGroupEntry {
            binding: binding as u32,
            resource: buffers[binding].as_entire_binding(),
        });
        let group = backend
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gaussian_page_group"),
                layout: &if direction.box_filter {
                    &self.box_pipeline
                } else {
                    &self.pipeline
                }
                .layout,
                entries: &entries,
            });
        {
            let mut pass = submission
                .encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("gaussian_bounded_line"),
                    timestamp_writes: None,
                });
            pass.set_pipeline(
                &if direction.box_filter {
                    &self.box_pipeline
                } else {
                    &self.pipeline
                }
                .pipeline,
            );
            pass.set_bind_group(0, &group, &[]);
            if direction.box_filter && self.sample_bytes == 4 {
                let groups = (p.count.div_ceil(4) * p.lines).div_ceil(64);
                let (x, y) = dispatch_grid(groups);
                pass.dispatch_workgroups(x, y, 1);
            } else {
                pass.dispatch_workgroups(p.lines.div_ceil(64), 1, 1);
            }
        }
        submission.command()?;
        let rect = if direction.vertical {
            Rect {
                x: page.line,
                y: page.first,
                columns: p.lines,
                rows: p.count,
                scratch: 0,
                row_step: p.lines as i32,
                col_step: 1,
            }
        } else {
            Rect {
                x: page.first,
                y: page.line,
                columns: p.count,
                rows: p.lines,
                scratch: 0,
                row_step: p.count as i32,
                col_step: 1,
            }
        };
        // Only the target channel is scattered: no destination-preserve gather.
        self.transfer(
            submission,
            &direction.destination,
            direction.destination_offset,
            direction.channel,
            &self.output,
            rect,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn transfer(
        &self,
        submission: &mut Submission<'_>,
        store: &Store,
        offset: u64,
        channel: u32,
        scratch: &wgpu::Buffer,
        rect: Rect,
        scatter: bool,
    ) -> Result<(), String> {
        let words = self.sample_bytes / 4;
        let row_stride = u64::from(self.width) * 3 * words;
        let alignment = u64::from(
            submission
                .backend
                .device
                .limits()
                .min_storage_buffer_offset_alignment,
        );
        for component in 0..words {
            let mut row = 0;
            let mut col = 0;
            while row < rect.rows {
                submission.backend.check_cancelled()?;
                let sample = (offset
                    + (u64::from(rect.y + row) * u64::from(self.width) + u64::from(rect.x + col))
                        * 3
                    + u64::from(channel))
                    * words
                    + component;
                let view = store.view(sample, self.limit, alignment, self.adapter.size());
                let end = view.first + view.bytes / 4;
                let columns = rect
                    .columns
                    .min(col + ((end - 1 - sample) / (3 * words) + 1) as u32)
                    - col;
                let rows = if col == 0 && columns == rect.columns {
                    (1 + (end - 1 - sample - u64::from(columns - 1) * 3 * words) / row_stride)
                        .min(u64::from(rect.rows - row)) as u32
                } else {
                    1
                };
                let params = TransportParams {
                    count: columns * rows,
                    columns,
                    image_start: (sample - view.first) as u32,
                    image_row_step: row_stride as u32,
                    scratch_start: (rect.scratch
                        + row as i32 * rect.row_step
                        + col as i32 * rect.col_step)
                        * words as i32
                        + component as i32,
                    scratch_row_step: rect.row_step * words as i32,
                    scratch_col_step: rect.col_step * words as i32,
                    scatter: u32::from(scatter),
                    sample_words: words as u32,
                    _pad: [0; 3],
                };
                self.encode_transfer(submission, &view, scratch, params)?;
                col += columns;
                if col == rect.columns {
                    col = 0;
                    row += rows;
                }
            }
        }
        Ok(())
    }

    fn encode_transfer(
        &self,
        submission: &mut Submission<'_>,
        view: &View<'_>,
        scratch: &wgpu::Buffer,
        params: TransportParams,
    ) -> Result<(), String> {
        let backend = submission.backend;
        let buffer = if view.direct {
            &view.page.buffer
        } else {
            &self.adapter
        };
        let binding_offset = if view.direct { view.local } else { 0 };
        if !view.direct {
            // Bulk copies adapt COPY-only or sub-alignment physical pages.
            // Scatter reads the same capped window first to preserve neighbours.
            submission.encoder.copy_buffer_to_buffer(
                &view.page.buffer,
                view.local,
                &self.adapter,
                0,
                view.bytes,
            );
            submission.command()?;
        }
        let uniform = backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("gaussian_transport_params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let group = backend
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gaussian_transport_group"),
                layout: &self.copy_pipeline.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer,
                            offset: binding_offset,
                            size: NonZeroU64::new(view.bytes),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: scratch.as_entire_binding(),
                    },
                ],
            });
        {
            let mut pass = submission
                .encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("gaussian_raw_transport"),
                    timestamp_writes: None,
                });
            pass.set_pipeline(&self.copy_pipeline.pipeline);
            pass.set_bind_group(0, &group, &[]);
            dispatch_linear(&mut pass, params.count);
        }
        submission.command()?;
        if !view.direct && params.scatter != 0 {
            submission.encoder.copy_buffer_to_buffer(
                &self.adapter,
                0,
                &view.page.buffer,
                view.local,
                view.bytes,
            );
            submission.command()?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x: u32,
    y: u32,
    columns: u32,
    rows: u32,
    scratch: i32,
    row_step: i32,
    col_step: i32,
}

fn reflect(index: i64, length: u32) -> u32 {
    let period = 2 * i64::from(length);
    let folded = index.rem_euclid(period);
    if folded < i64::from(length) {
        folded as u32
    } else {
        (period - 1 - folded) as u32
    }
}

#[cfg(test)]
mod logic_tests {
    use super::*;

    #[test]
    fn workspace_caps_pages_by_sample_width_and_rejects_geometry_overflow() {
        let f32_cap = u64::from(MAX_PAGE_VALUES) * 4;
        let f64_cap = u64::from(MAX_PAGE_VALUES) * 8;
        let small = workspace_bytes(3, 5, 1024).unwrap();
        assert_eq!(small, workspace_bytes_f64(3, 5, 1024).unwrap());
        assert_eq!(workspace_bytes(3, 5, 1025).unwrap() - small, 4);
        assert_eq!(
            workspace_bytes(3, 5, f32_cap).unwrap(),
            workspace_bytes(3, 5, u64::MAX).unwrap()
        );
        assert_eq!(
            workspace_bytes_f64(3, 5, f64_cap).unwrap(),
            workspace_bytes_f64(3, 5, u64::MAX).unwrap()
        );
        assert_eq!(
            workspace_bytes_f64(3, 5, u64::MAX).unwrap() - workspace_bytes(3, 5, u64::MAX).unwrap(),
            4 * (f64_cap - f32_cap)
        );
        assert!(workspace_bytes(u32::MAX, u32::MAX, 1024).is_err());
        assert!(workspace_bytes_f64(u32::MAX, u32::MAX, 1024).is_err());
    }

    #[test]
    fn blur_reflection_repeats_endpoints_across_multiple_periods() {
        for index in -12..=12 {
            assert_eq!(reflect(index, 1), 0);
            assert_eq!(reflect(index, 3), reflect(index + 6, 3));
        }
        assert_eq!(
            (-1..=5).map(|i| reflect(i, 3)).collect::<Vec<_>>(),
            [0, 0, 1, 2, 2, 1, 0]
        );
    }
}

#[cfg(all(test, not(feature = "precision-f64")))]
mod tests {
    use super::super::reference::blur_channel;
    use super::*;
    use spektrafilm_backend::telemetry::{CollectionMode, ObservationContext};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    fn compare(width: u32, height: u32, sigma: [f64; 3], limit: u64) {
        compare_with_reuse(width, height, sigma, limit, false);
    }

    fn compare_with_reuse(
        width: u32,
        height: u32,
        sigma: [f64; 3],
        limit: u64,
        cancelled_reuse: bool,
    ) -> Vec<f32> {
        let gpu = crate::test_support::adapter();
        let mut gpu = gpu.derived_explicit();
        let cancellation = Arc::new(AtomicBool::new(false));
        gpu.pipe = gpu.pipe.with_cancellation(Some(cancellation.clone()));
        gpu.cancellation = Some(cancellation.clone());
        let samples = width as usize * height as usize * 3;
        let source_offset = 5;
        let destination_offset = 7;
        // Strong true-endpoint impulses plus a nonperiodic interior distinguish
        // recurrence restarts, missing FIR halo and flipped pass order.
        let mut source = vec![0.0f32; samples + source_offset as usize + 4];
        for (i, value) in source[source_offset as usize..source_offset as usize + samples]
            .iter_mut()
            .enumerate()
        {
            *value = ((i * 37 + i * i * 13) % 103) as f32 / 103.0;
        }
        source[source_offset as usize] = 7.0;
        source[source_offset as usize + samples - 1] = -3.0;
        let input = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bounded_gaussian_test_input"),
                contents: bytemuck::cast_slice(&source),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });
        let sentinel = vec![123.0f32; samples + destination_offset as usize + 4];
        let make_output = || {
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("bounded_gaussian_test_output"),
                    contents: bytemuck::cast_slice(&sentinel),
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                })
        };
        let output = make_output();
        let untiled = make_output();
        let state = BoundedBlur::build(
            &gpu,
            width,
            height,
            &input,
            &output,
            None,
            sigma,
            source_offset,
            destination_offset,
            limit,
        )
        .unwrap();
        assert!(state.input.size() <= limit);
        assert!(state.output.size() <= limit);
        assert!(state._state.size() <= limit);
        assert!(
            state._intermediate.0.iter().all(
                |p| p.bytes <= limit && p.buffer.usage().contains(wgpu::BufferUsages::STORAGE)
            )
        );
        if cancelled_reuse {
            // Leave genuine partial recurrence/scratch work on the queue, then
            // cancel and rerun the same state. The first true endpoint must
            // initialize fresh state rather than consuming the abandoned sweep.
            let mut partial = Submission::new(&gpu);
            let direction = &state.directions[0];
            state
                .encode_page(&mut partial, direction, &direction.pages[0])
                .unwrap();
            partial.flush().unwrap();
            cancellation.store(true, Ordering::Relaxed);
            assert_eq!(state.submit(&gpu).unwrap_err(), "operation cancelled");
            cancellation.store(false, Ordering::Relaxed);
        }
        let direct = build_grain_blur(
            &gpu,
            width,
            height,
            &input,
            &untiled,
            sigma,
            source_offset,
            destination_offset,
        )
        .unwrap();
        let bytes = sentinel.len() as u64 * 4;
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded_gaussian_test_readback"),
            size: bytes * 2,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let context = ObservationContext::new(CollectionMode::Summary);
        gpu.context_mut(context.clone());
        state.submit(&gpu).unwrap();
        if cancelled_reuse {
            let work = context.snapshot().totals;
            // Real encoded work, not source inspection: ordinary STORAGE
            // images need no device copies at all for transport. Commands scale
            // with intersecting bounded rectangles, never with image samples.
            assert_eq!(work.device_copy_count, Some(0));
            assert!(work.dispatch_count.unwrap() < u64::from(width) * u64::from(height) / 100);
            assert!(work.submit_count.unwrap() <= 2);
        }
        direct.submit(&gpu).unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        encoder.copy_buffer_to_buffer(&untiled, 0, &readback, bytes, bytes);
        gpu.queue.submit(Some(encoder.finish()));
        let actual = gpu.read_mapped_f32(&readback, bytes * 2);
        let (tiled, whole) = actual.split_at(sentinel.len());
        for channel in 0..3 {
            let input_channel: Vec<f32> = (0..samples / 3)
                .map(|pixel| source[source_offset as usize + pixel * 3 + channel])
                .collect();
            let expected = blur_channel(&input_channel, width, height, sigma[channel]);
            for (pixel, expected) in expected.into_iter().enumerate() {
                let index = destination_offset as usize + pixel * 3 + channel;
                assert!(
                    (tiled[index] - expected).abs() < 2e-5,
                    "limit {limit} {width}x{height} channel {channel} pixel {pixel}: {} != {expected}",
                    tiled[index]
                );
                assert!(
                    (tiled[index] - whole[index]).abs() < 2e-5,
                    "paged/direct mismatch channel {channel} pixel {pixel}"
                );
            }
        }
        assert!(
            tiled[..destination_offset as usize]
                .iter()
                .all(|&x| x == 123.0)
        );
        assert!(
            tiled[destination_offset as usize + samples..]
                .iter()
                .all(|&x| x == 123.0)
        );
        tiled.to_vec()
    }

    #[test]
    fn paged_iir_preserves_both_true_endpoints_and_recursion_seams() {
        compare(71, 67, [3.0, 4.0, 7.9], 192);
        compare(1, 39, [3.0, 4.0, 7.9], 24);
        compare(39, 1, [3.0, 4.0, 7.9], 24);
    }

    /// Compare the optimized box kernel with the original ordered-window
    /// kernel on identical packed inputs, not with a shared CPU approximation.
    #[test]
    fn vector_boxes_preserve_ordered_window_bits_and_support() {
        let gpu = crate::test_support::adapter();
        let original = gpu.cached_pipeline(include_str!("blur_bounded_f32.wgsl"), &LAYOUT);
        let vector = gpu.cached_pipeline(include_str!("blur_bounded_box_f32.wgsl"), &LAYOUT);
        let lines = 6usize;
        for count in [1usize, 3, 7, 31, 33, 129] {
            for radius in [0usize, 9, 47, 141] {
                for axis_major in [0u32, 1] {
                    let span = count + 2 * radius;
                    let mut values = vec![0.0f32; span * lines];
                    for j in 0..span {
                        for line in 0..lines {
                            let value = match line {
                                0 => ((j * 37 + 11) % 103) as f32 / 51.0 - 1.0,
                                1 if j == radius => f32::NAN,
                                2 if j == radius => f32::INFINITY,
                                2 if j == radius + 7 => f32::NEG_INFINITY,
                                3 => {
                                    if j % 4 < 2 {
                                        f32::MAX * 0.75
                                    } else {
                                        -f32::MAX * 0.75
                                    }
                                }
                                4 if j == 0 => 16777216.0,
                                5 => {
                                    if j % 2 == 0 {
                                        -0.0
                                    } else {
                                        0.0
                                    }
                                }
                                _ => 1.0,
                            };
                            let index = if axis_major == 0 {
                                line * span + j
                            } else {
                                j * lines + line
                            };
                            values[index] = value;
                        }
                    }
                    let source = gpu
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("box_order_input"),
                            contents: bytemuck::cast_slice(&values),
                            usage: wgpu::BufferUsages::STORAGE,
                        });
                    let params = Params32 {
                        count: count as u32,
                        lines: lines as u32,
                        stride: 1,
                        channel: 0,
                        radius: radius as u32,
                        mode: 3,
                        initialize: 1,
                        axis_major,
                        a: 0.0,
                        b1: 0.0,
                        b2: 0.0,
                        b3: 0.0,
                    };
                    let uniform = super::super::super::raster::uniform(&gpu, &params);
                    let kernel = gpu
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("box_order_weight"),
                            contents: bytemuck::cast_slice(&[1.0 / (2 * radius + 1) as f32]),
                            usage: wgpu::BufferUsages::STORAGE,
                        });
                    let state = transport(&gpu, "box_order_state", lines as u64 * 12, true);
                    let bytes = (count * lines * 4) as u64;
                    let a = transport(&gpu, "box_order_original", bytes, true);
                    let b = transport(&gpu, "box_order_vector", bytes, true);
                    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("box_order_readback"),
                        size: bytes * 2,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    let mut commands = super::super::super::raster::RasterCommands::default();
                    // Both kernels use 64-thread workgroups, not the generic
                    // RasterCommands 256-thread linear dispatch convention.
                    commands.dispatch(
                        &original,
                        &[&uniform, &source, &a, &kernel, &state],
                        (lines as u32).div_ceil(64),
                        true,
                        &gpu,
                    );
                    let threads = count.div_ceil(4) * lines;
                    commands.dispatch(
                        &vector,
                        &[&uniform, &source, &b, &kernel, &state],
                        (threads as u32).div_ceil(64),
                        true,
                        &gpu,
                    );
                    commands.copy(&a, 0, &readback, 0, bytes);
                    commands.copy(&b, 0, &readback, bytes, bytes);
                    commands.submit(&gpu).unwrap();
                    let result = gpu.read_mapped_f32(&readback, bytes * 2);
                    let (expected, actual) = result.split_at(count * lines);
                    for (i, (&want, &got)) in expected.iter().zip(actual).enumerate() {
                        if want.is_nan() {
                            assert!(got.is_nan());
                        } else {
                            assert_eq!(
                                got.to_bits(),
                                want.to_bits(),
                                "count={count} radius={radius} axis={axis_major} sample={i}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn vector_boxes_preserve_bits_across_page_layouts() {
        let mut baseline: Option<Vec<f32>> = None;
        for limit in [512, 1024, 2048] {
            let actual = compare_with_reuse(71, 67, [8.0, 20.0, 32.0], limit, false);
            if let Some(expected) = &baseline {
                assert_eq!(
                    actual.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                    expected.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                    "transport layout changed bits at limit={limit}"
                );
            } else {
                baseline = Some(actual);
            }
        }
    }

    #[test]
    fn paged_boxes_reflect_endpoints_and_page_seams() {
        compare(71, 67, [8.0, 20.0, 32.0], 512);
        compare(1, 39, [8.0, 20.0, 32.0], 512);
        compare(39, 1, [8.0, 20.0, 32.0], 512);
    }

    #[test]
    fn paged_fir_reflects_true_support_and_keeps_mixed_channel_order() {
        compare(29, 23, [2.99, 3.5, 0.0], 152);
        compare(7, 3, [2.99, 0.4, 0.1667], 152);
        compare(1, 1, [2.99, 3.0, 0.0], 152);
    }

    #[test]
    fn column_batches_match_reference_without_per_pixel_channel_transport() {
        compare(37, 5, [0.4, 3.0, 0.0], 192);
    }

    #[test]
    fn paged_fir_and_iir_match_reference_across_both_axis_batch_tails() {
        compare(137, 131, [2.99, 3.0, 0.0], 4096);
    }

    #[test]
    fn gpu_transport_command_growth_and_cancelled_partial_state_reuse() {
        compare_with_reuse(129, 97, [3.0, 0.4, 0.0], 65536, true);
    }

    #[test]
    fn narrow_rasters_bound_transport_and_preserve_cancelled_recurrence() {
        for (width, height) in [(1, 65537), (65537, 1)] {
            compare_with_reuse(width, height, [3.0, 0.0, 0.0], 262144, true);
        }
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
        for (sigma, limit, minimum) in [([2.99, 0.0, 0.0], 151, 152), ([3.0, 0.0, 0.0], 23, 24)] {
            let error = match BoundedBlur::build(
                &gpu,
                1,
                1,
                &source,
                &destination,
                None,
                sigma,
                0,
                0,
                limit,
            ) {
                Ok(_) => panic!("binding limit {limit} accepted below minimum {minimum}"),
                Err(error) => error,
            };
            assert!(
                error.contains(&format!("requires {minimum} bytes")),
                "{error}"
            );
            assert!(error.contains(&format!("limit is {limit}")), "{error}");
        }
    }
}

#[cfg(all(test, not(feature = "precision-f64")))]
mod physical_page_tests {
    use super::super::reference::blur_channel;
    use super::*;

    #[test]
    fn physical_pages_preserve_layer_offsets_axis_seams_and_neighbours() {
        let gpu = crate::test_support::adapter();
        let (width, height) = (29, 23);
        let pixels = (width * height) as usize;
        let source_sample = 5u64;
        let destination_sample = 7u64;
        let mut values: Vec<f32> = (0..pixels * 3 + 15)
            .map(|i| ((i * 37 + i * i * 13) % 103) as f32 / 103.0)
            .collect();
        values[source_sample as usize] = 7.0;
        values[source_sample as usize + pixels * 3 - 1] = -3.0;
        let split = |samples: &[f32], page_pixels: usize| -> Vec<ImagePage> {
            samples
                .chunks(page_pixels * 3)
                .enumerate()
                .map(|(index, samples)| ImagePage {
                    first_pixel: (index * page_pixels) as u32,
                    n_pixels: (samples.len() / 3) as u32,
                    buffer: gpu
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("gaussian_physical_test_page"),
                            contents: bytemuck::cast_slice(samples),
                            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                        }),
                })
                .collect()
        };
        let source = split(&values, 7);
        let sentinel = vec![123.0f32; pixels * 3 + 15];
        let destination = split(&sentinel, 11);
        let sigma = [2.99, 8.0, 0.0];
        let state = BoundedBlur::build_paged(
            &gpu,
            width,
            height,
            &source,
            &destination,
            None,
            sigma,
            source_sample,
            destination_sample,
            152,
        )
        .unwrap();
        assert!(state._intermediate.0.len() > 1);
        assert!(state._intermediate.0.iter().all(|p| p.bytes <= 152));
        let bytes = sentinel.len() as u64 * 4;
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gaussian_physical_test_readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        state.submit(&gpu).unwrap();
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        Store::image(&destination).copy(&mut encoder, 0, &readback, 0, bytes, false);
        gpu.queue.submit(Some(encoder.finish()));
        let actual = gpu.read_mapped_f32(&readback, bytes);
        for channel in 0..3 {
            let input: Vec<f32> = (0..pixels)
                .map(|pixel| values[source_sample as usize + pixel * 3 + channel])
                .collect();
            let expected = blur_channel(&input, width, height, sigma[channel]);
            for (pixel, expected) in expected.into_iter().enumerate() {
                let got = actual[destination_sample as usize + pixel * 3 + channel];
                assert!(
                    (got - expected).abs() < 2e-5,
                    "physical page seam channel {channel} pixel {pixel}: {got} != {expected}"
                );
            }
        }
        assert!(
            actual[..destination_sample as usize]
                .iter()
                .all(|&x| x == 123.0)
        );
        assert!(
            actual[destination_sample as usize + pixels * 3..]
                .iter()
                .all(|&x| x == 123.0)
        );
    }

    #[test]
    fn identity_preserves_uncertain_bits_offsets_and_neighbours_on_copy_only_pages() {
        let gpu = crate::test_support::adapter();
        let (width, height) = (13, 5);
        let samples = (width * height * 3) as usize;
        let source_offset = 5u64;
        let destination_offset = 7u64;
        let patterns = [
            0u32,
            0x8000_0000,
            0x7f80_0000,
            0xff80_0000,
            0x7fc1_2345,
            0x7f81_2345,
            1,
            0x807f_ffff,
            0x3f80_0001,
        ];
        let values: Vec<f32> = (0..samples + 15)
            .map(|i| f32::from_bits(patterns[i % patterns.len()]))
            .collect();
        let sentinel = vec![f32::from_bits(0xdead_beef); values.len()];
        for (copy_only, limit) in [
            (true, 12),
            (true, 76),
            (true, 192),
            (false, 192),
            (false, 4096),
        ] {
            let split = |values: &[f32], page_pixels: usize| -> Vec<ImagePage> {
                values
                    .chunks(page_pixels * 3)
                    .enumerate()
                    .map(|(i, chunk)| ImagePage {
                        first_pixel: (i * page_pixels) as u32,
                        n_pixels: (chunk.len() / 3) as u32,
                        buffer: gpu
                            .device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("gaussian_identity_bits_page"),
                                contents: bytemuck::cast_slice(chunk),
                                usage: wgpu::BufferUsages::COPY_SRC
                                    | wgpu::BufferUsages::COPY_DST
                                    | if copy_only {
                                        wgpu::BufferUsages::empty()
                                    } else {
                                        wgpu::BufferUsages::STORAGE
                                    },
                            }),
                    })
                    .collect()
            };
            let source = split(&values, if copy_only { 7 } else { values.len() / 3 });
            let destination = split(&sentinel, if copy_only { 11 } else { values.len() / 3 });
            let state = BoundedBlur::build_paged(
                &gpu,
                width,
                height,
                &source,
                &destination,
                None,
                [0.0, -1.0, -0.0],
                source_offset,
                destination_offset,
                limit,
            )
            .unwrap();
            let bytes = values.len() as u64 * 4;
            let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gaussian_identity_bits_readback"),
                size: bytes * 2,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            state.submit(&gpu).unwrap();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            Store::image(&destination).copy(&mut encoder, 0, &readback, 0, bytes, false);
            Store::image(&source).copy(&mut encoder, 0, &readback, bytes, bytes, false);
            gpu.queue.submit(Some(encoder.finish()));
            let actual = gpu.read_mapped_f32(&readback, bytes * 2);
            let (output, unchanged_source) = actual.split_at(values.len());
            for (i, value) in output.iter().enumerate() {
                let expected = if (destination_offset as usize
                    ..destination_offset as usize + samples)
                    .contains(&i)
                {
                    values[source_offset as usize + i - destination_offset as usize]
                } else {
                    sentinel[i]
                };
                assert_eq!(
                    value.to_bits(),
                    expected.to_bits(),
                    "COPY-only {copy_only}, cap {limit}, sample {i}"
                );
            }
            assert!(
                unchanged_source
                    .iter()
                    .zip(&values)
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            );
        }
    }
}

#[cfg(test)]
mod binary64_tests {
    use super::*;
    use crate::wgpu_backend::grain::raster::RasterStorage;

    fn reference_axis(
        input: &[f64],
        width: usize,
        height: usize,
        sigma: f64,
        vertical: bool,
        f32_working: bool,
    ) -> Vec<f64> {
        let mut output = vec![0.0; input.len()];
        let length = if vertical { height } else { width };
        let lines = if vertical { width } else { height };
        let at = |line: usize, coordinate: usize| {
            if vertical {
                coordinate * width + line
            } else {
                line * width + coordinate
            }
        };
        for line in 0..lines {
            if sigma < 3.0 {
                let (radius, kernel) = gaussian_kernel_f64(sigma).unwrap();
                for coordinate in 0..length {
                    for (tap, &weight) in kernel.iter().enumerate() {
                        let source = reflect(
                            coordinate as i64 + tap as i64 - i64::from(radius),
                            length as u32,
                        );
                        output[at(line, coordinate)] += input[at(line, source as usize)] * weight;
                        if f32_working && vertical {
                            output[at(line, coordinate)] =
                                f64::from(output[at(line, coordinate)] as f32);
                        }
                    }
                    if f32_working {
                        output[at(line, coordinate)] =
                            f64::from(output[at(line, coordinate)] as f32);
                    }
                }
            } else {
                let (a, b1, b2, b3) = iir_coefficients_f64(sigma);
                let mut state = [input[at(line, 0)]; 3];
                for coordinate in 0..length {
                    let value = a * input[at(line, coordinate)]
                        + b1 * state[0]
                        + b2 * state[1]
                        + b3 * state[2];
                    output[at(line, coordinate)] = if f32_working {
                        f64::from(value as f32)
                    } else {
                        value
                    };
                    state = [value, state[0], state[1]];
                }
                state = [output[at(line, length - 1)]; 3];
                for coordinate in (0..length).rev() {
                    let value = a * output[at(line, coordinate)]
                        + b1 * state[0]
                        + b2 * state[1]
                        + b3 * state[2];
                    output[at(line, coordinate)] = if f32_working {
                        f64::from(value as f32)
                    } else {
                        value
                    };
                    state = [value, state[0], state[1]];
                }
            }
        }
        output
    }

    #[test]
    fn binary64_pages_preserve_dispatch_true_endpoints_and_split_sample_bits() {
        let gpu = crate::test_support::adapter();
        let below_three = f64::from_bits(3.0f64.to_bits() - 1);
        assert_eq!(below_three as f32, 3.0);
        for (width, height) in [(137u32, 3u32), (1, 17), (17, 1)] {
            let samples = (width * height * 3) as usize;
            let values: Vec<f64> = (0..samples + 15)
                .map(|index| {
                    if index == 5 {
                        7.0
                    } else if index == samples + 4 {
                        -3.0
                    } else {
                        ((index * 37 + index * index * 13) % 103) as f64 / 103.0 + 1e-12
                    }
                })
                .collect();
            let sentinel = vec![123.0f64; values.len()];
            // Seven raw RGB12 units per source page split f64 samples. Different
            // destination boundaries catch dropped high words and neighbour writes.
            let split = |data: &[f64], page_bytes: usize| -> Vec<ImagePage> {
                bytemuck::cast_slice::<f64, u8>(data)
                    .chunks(page_bytes)
                    .enumerate()
                    .map(|(index, bytes)| ImagePage {
                        first_pixel: (index * page_bytes / 12) as u32,
                        n_pixels: (bytes.len() / 12) as u32,
                        buffer: gpu
                            .device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("binary64_gaussian_raw_page"),
                                contents: bytes,
                                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                            }),
                    })
                    .collect()
            };
            let source = split(&values, 84);
            let destination = split(&sentinel, 132);
            let sigma = [below_three, 3.0, 0.0];
            let state = BoundedBlur::build_f64_paged(
                &gpu,
                width,
                height,
                &source,
                &destination,
                sigma,
                5,
                7,
                192,
            )
            .unwrap();
            assert!(
                state.directions[0].vertical,
                "original f64 sigma below 3 must use vertical-first FIR"
            );
            assert!(
                !state.directions[2].vertical,
                "sigma exactly 3 must use horizontal-first IIR"
            );
            assert!(state.input.size() <= 192 && state._state.size() <= 192);
            state.submit(&gpu).unwrap();
            let bytes = values.len() as u64 * 8;
            let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("binary64_gaussian_readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            destination.visit(&mut |first, buffer| {
                encoder.copy_buffer_to_buffer(buffer, 0, &readback, first, buffer.size())
            });
            gpu.queue.submit(Some(encoder.finish()));
            let actual: Vec<f64> = gpu.pipe.read_mapped(&readback, bytes);
            assert_eq!(&actual[..7], &sentinel[..7]);
            assert_eq!(&actual[7 + samples..], &sentinel[7 + samples..]);
            for channel in 0..3 {
                let input: Vec<f64> = (0..samples / 3)
                    .map(|pixel| values[5 + pixel * 3 + channel])
                    .collect();
                let expected = if sigma[channel] <= 0.0 {
                    input
                } else {
                    let vertical_first = sigma[channel] < 3.0;
                    let tmp = reference_axis(
                        &input,
                        width as usize,
                        height as usize,
                        sigma[channel],
                        vertical_first,
                        false,
                    );
                    reference_axis(
                        &tmp,
                        width as usize,
                        height as usize,
                        sigma[channel],
                        !vertical_first,
                        false,
                    )
                };
                for (pixel, want) in expected.into_iter().enumerate() {
                    let got = actual[7 + pixel * 3 + channel];
                    assert!(
                        (got - want).abs() <= 2e-12 * want.abs().max(1.0),
                        "{width}x{height}, channel {channel}, pixel {pixel}: {got} != {want}"
                    );
                }
            }
        }
    }

    #[test]
    fn f32_storage_retains_f64_taps_accumulation_and_recursive_state_across_pages() {
        let gpu = crate::test_support::adapter();
        let (width, height) = (71u32, 9u32);
        let samples = (width * height * 3) as usize;
        let values: Vec<f32> = (0..samples)
            .map(|index| {
                if index < 3 {
                    7.0
                } else {
                    ((index * 37 + index * index * 13) % 103) as f32 / 103.0
                }
            })
            .collect();
        let source = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mixed_precision_gaussian_source"),
                contents: bytemuck::cast_slice(&values),
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::STORAGE,
            });
        let bytes = samples as u64 * 4;
        let output = transport(&gpu, "mixed_precision_gaussian_output", bytes, true);
        let sigma = [0.5 - 1e-10, f64::from_bits(3.0f64.to_bits() - 1), 7.0];
        let bounded = BoundedBlur::build(
            &gpu, width, height, &source, &output, None, sigma, 0, 0, 192,
        )
        .unwrap();
        let direct_output = transport(&gpu, "mixed_precision_gaussian_direct", bytes, true);
        let direct =
            build_grain_blur(&gpu, width, height, &source, &direct_output, sigma, 0, 0).unwrap();
        bounded.submit(&gpu).unwrap();
        direct.submit(&gpu).unwrap();
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mixed_precision_gaussian_readback"),
            size: bytes * 2,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        encoder.copy_buffer_to_buffer(&direct_output, 0, &readback, bytes, bytes);
        gpu.queue.submit(Some(encoder.finish()));
        let actual: Vec<f32> = gpu.pipe.read_mapped(&readback, bytes * 2);
        for channel in 0..3 {
            let input: Vec<f64> = (0..samples / 3)
                .map(|pixel| f64::from(values[pixel * 3 + channel]))
                .collect();
            let vertical_first = sigma[channel] < 3.0;
            let tmp = reference_axis(
                &input,
                width as usize,
                height as usize,
                sigma[channel],
                vertical_first,
                true,
            );
            let expected = reference_axis(
                &tmp,
                width as usize,
                height as usize,
                sigma[channel],
                !vertical_first,
                true,
            );
            for (pixel, want) in expected.into_iter().enumerate() {
                for base in [0, samples] {
                    let got = f64::from(actual[base + pixel * 3 + channel]);
                    assert!(
                        (got - want).abs() <= 2e-7 * want.abs().max(1.0),
                        "sample storage f32, base {base}, channel {channel}, pixel {pixel}: {got} != {want}"
                    );
                }
            }
        }
    }
}
