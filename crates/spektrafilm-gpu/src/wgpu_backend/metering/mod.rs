//! Full-source preview, luminance measurement, and EV interpretation on device.
//! Only one preview row's vertical samples are live, split into bounded blocks.
use super::bounded::{chunk_elements, storage_binding_limit};
use super::film_chain::device_image::BufferPayload;
use super::*;
use spektrafilm_backend::MeterAutoexposureParams;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MeterParams {
    src_width: u32,
    src_height: u32,
    cols: u32,
    rows: u32,
    radius_y: u32,
    radius_x: u32,
    cctf_kind: u32,
    pattern: u32,
    sigma_y: f64,
    sigma_x: f64,
    factor_y: f64,
    factor_x: f64,
    y_row: [f64; 4],
    mid_grey: f64,
    decode: u32,
    pad: u32,
    tail: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MeterChunk {
    row: u32,
    first: u32,
    count: u32,
    tap_first: u32,
    tap_count: u32,
    page_first: u32,
    page_len: u32,
    axis: u32,
}

pub(super) struct MeterState {
    pipeline: CachedPipelineRef,
    group: wgpu::BindGroup,
    result: wgpu::Buffer,
    cancellation: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

pub(super) fn preview_dimensions(width: u32, height: u32) -> (u32, u32, f64, f64) {
    let scale = (256.0 / f64::from(width.max(height))).min(1.0);
    let cols = (f64::from(width) * scale).round_ties_even().max(1.0) as u32;
    let rows = (f64::from(height) * scale).round_ties_even().max(1.0) as u32;
    (
        cols,
        rows,
        f64::from(width) / f64::from(cols),
        f64::from(height) / f64::from(rows),
    )
}

pub(super) fn build_meter_state(
    _device: &ObservedDevice,
    params: &MeterAutoexposureParams,
    image: &ImageBuf,
    backend: &WgpuBackend,
) -> Result<MeterState, String> {
    if image.width != params.width || image.height != params.height {
        return Err("metering source dimensions do not match the resolved request".into());
    }
    let pages = backend.allocate_image_pages(image.width, image.height, "meter_source")?;
    for page in &pages {
        backend.check_cancelled()?;
        let first = page.first_pixel as usize * 3;
        let samples = scalars_to_f32(&image.data[first..first + page.n_pixels as usize * 3]);
        backend
            .queue
            .write_buffer(&page.buffer, 0, bytemuck::cast_slice(&samples));
    }
    let source = BufferPayload::from_pages(backend.backend_id(), pages, image.width, image.height);
    build_meter_state_from_payload(backend, params, &source)
}

pub(super) fn build_meter_state_from_payload(
    backend: &WgpuBackend,
    params: &MeterAutoexposureParams,
    source: &BufferPayload,
) -> Result<MeterState, String> {
    build_meter_state_with_limit(backend, params, source, storage_binding_limit(backend)?)
}

fn build_meter_state_with_limit(
    backend: &WgpuBackend,
    params: &MeterAutoexposureParams,
    source: &BufferPayload,
    limit: u64,
) -> Result<MeterState, String> {
    backend.check_cancelled()?;
    let device = &backend.device;
    if !device
        .features()
        .contains(wgpu::Features::SHADER_F64 | wgpu::Features::SHADER_INT64)
    {
        return Err(
            "auto-exposure metering requires device binary64 arithmetic and 64-bit integers".into(),
        );
    }
    if params.width == 0 || params.height == 0 || params.max_size != 256 {
        return Err("auto-exposure metering requires nonempty input and the reference 256-pixel preview cap".into());
    }
    if source.width != params.width || source.height != params.height {
        return Err("metering source dimensions do not match the resolved request".into());
    }
    let (cols, rows, factor_x, factor_y) = preview_dimensions(params.width, params.height);
    let luminance_bytes = u64::from(rows) * u64::from(cols) * 8;
    // The reference's bounded preview and fixed reduction coefficients remain
    // the minimum working set. Source size and rows*width are no longer limits.
    if luminance_bytes.max(200) > limit {
        return Err(format!(
            "metering minimum preview/reduction binding needs {} bytes; limit is {limit}",
            luminance_bytes.max(200)
        ));
    }
    // Contiguous buffers produced by spatial stages are valid carriers too.
    // Split their device view before binding; no source samples return to host.
    let normalized;
    let source = if source.pages.iter().any(|page| page.buffer.size() > limit) {
        let pixels = chunk_elements(limit, 12)? as u32;
        let mut pages = Vec::new();
        let mut encoder = device.create_command_encoder(&Default::default());
        for first_pixel in (0..source.n_pixels).step_by(pixels as usize) {
            backend.check_cancelled()?;
            let n_pixels = pixels.min(source.n_pixels - first_pixel);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("meter_source_page"),
                size: u64::from(n_pixels) * 12,
                mapped_at_creation: false,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            });
            backend.copy_samples_to(
                &mut encoder,
                source,
                u64::from(first_pixel) * 3,
                u64::from(n_pixels) * 3,
                &buffer,
                0,
            )?;
            pages.push(super::paged::ImagePage {
                first_pixel,
                n_pixels,
                buffer,
            });
        }
        backend.check_cancelled()?;
        backend.queue.submit(Some(encoder.finish()));
        normalized =
            BufferPayload::from_pages(backend.backend_id(), pages, source.width, source.height);
        &normalized
    } else {
        source
    };
    let capacity = chunk_elements(limit, 24)? as u32;
    let sigma_x = ((factor_x - 1.0) / 2.0).max(0.0);
    let sigma_y = ((factor_y - 1.0) / 2.0).max(0.0);
    let p = MeterParams {
        src_width: params.width,
        src_height: params.height,
        cols,
        rows,
        radius_y: (4.0 * sigma_y + 0.5).floor() as u32,
        radius_x: (4.0 * sigma_x + 0.5).floor() as u32,
        cctf_kind: params.cctf_kind.unwrap_or(0),
        pattern: match params.method.as_str() {
            "average" => 0,
            "median" => 1,
            "center_weighted" => 2,
            "partial" => 3,
            "matrix" => 4,
            "multi_zone" => 5,
            "highlight_weighted" => 6,
            _ => 7,
        },
        sigma_y,
        sigma_x,
        factor_y,
        factor_x,
        y_row: [params.y_row[0], params.y_row[1], params.y_row[2], 0.0],
        mid_grey: params.mid_grey,
        decode: params.cctf_kind.is_some() as u32,
        pad: 0,
        tail: [0; 4],
    };
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("meter_params"),
        contents: bytemuck::bytes_of(&p),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let buffer = |label, size| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        })
    };
    let vertical_taps = p.radius_y * 2 + 1;
    let horizontal_taps = p.radius_x * 2 + 1;
    let capacity = capacity.min(
        (u64::from(params.width) * u64::from(vertical_taps))
            .max(u64::from(cols) * u64::from(horizontal_taps))
            .min(u64::from(u32::MAX)) as u32,
    );
    let block = (capacity / vertical_taps).max(1).min(params.width);
    let horizontal_block = (capacity / horizontal_taps).max(1).min(cols);
    let mut vertical = Vec::new();
    for first in (0..params.width).step_by(block as usize) {
        let count = block.min(params.width - first);
        vertical.push((
            first,
            count,
            buffer("meter_vertical_page", u64::from(count) * 24),
        ));
    }
    let gathered = buffer("meter_tap_gather", u64::from(capacity) * 24);
    let accumulated = buffer("meter_horizontal_sum", u64::from(horizontal_block) * 24);
    let luminance = buffer("meter_luminance", luminance_bytes);
    let result = buffer("meter_ev", 8);
    // Gather only copies source values on device. Accumulation is a separate
    // ordered loop, so page order cannot change floating-point summation.
    for row in 0..rows {
        backend.check_cancelled()?;
        let mut encoder = device.create_command_encoder(&Default::default());
        for (first, count, output) in &vertical {
            let tap_capacity = capacity / count;
            for tap_first in (0..vertical_taps).step_by(tap_capacity as usize) {
                backend.check_cancelled()?;
                let tap_count = tap_capacity.min(vertical_taps - tap_first);
                let chunk = MeterChunk {
                    row,
                    first: *first,
                    count: *count,
                    tap_first,
                    tap_count,
                    page_first: 0,
                    page_len: 0,
                    axis: 0,
                };
                for page in &source.pages {
                    backend.check_cancelled()?;
                    encode_chunk(
                        backend,
                        &mut encoder,
                        &uniform,
                        concat!(
                            include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
                            include_str!("../../../../spektrafilm-shaders/wgsl/meter_lib.wgsl"),
                            include_str!("gather_vertical.wgsl")
                        ),
                        MeterChunk {
                            page_first: page.first_pixel,
                            page_len: page.n_pixels,
                            ..chunk
                        },
                        &page.buffer,
                        &gathered,
                        count * tap_count,
                        "meter_vertical_gather",
                    );
                }
                encode_chunk(
                    backend,
                    &mut encoder,
                    &uniform,
                    concat!(
                        include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
                        include_str!("../../../../spektrafilm-shaders/wgsl/meter_lib.wgsl"),
                        include_str!("accumulate.wgsl")
                    ),
                    chunk,
                    &gathered,
                    output,
                    *count,
                    "meter_vertical_accumulate",
                );
            }
        }
        for first in (0..cols).step_by(horizontal_block as usize) {
            backend.check_cancelled()?;
            let count = horizontal_block.min(cols - first);
            let tap_capacity = capacity / count;
            for tap_first in (0..horizontal_taps).step_by(tap_capacity as usize) {
                let tap_count = tap_capacity.min(horizontal_taps - tap_first);
                let chunk = MeterChunk {
                    row,
                    first,
                    count,
                    tap_first,
                    tap_count,
                    page_first: 0,
                    page_len: 0,
                    axis: 1,
                };
                for (page_first, page_len, page) in &vertical {
                    encode_chunk(
                        backend,
                        &mut encoder,
                        &uniform,
                        concat!(
                            include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
                            include_str!("../../../../spektrafilm-shaders/wgsl/meter_lib.wgsl"),
                            include_str!("gather_horizontal.wgsl")
                        ),
                        MeterChunk {
                            page_first: *page_first,
                            page_len: *page_len,
                            ..chunk
                        },
                        page,
                        &gathered,
                        count * tap_count,
                        "meter_horizontal_gather",
                    );
                }
                encode_chunk(
                    backend,
                    &mut encoder,
                    &uniform,
                    concat!(
                        include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
                        include_str!("../../../../spektrafilm-shaders/wgsl/meter_lib.wgsl"),
                        include_str!("accumulate.wgsl")
                    ),
                    chunk,
                    &gathered,
                    &accumulated,
                    count,
                    "meter_horizontal_accumulate",
                );
            }
            encode_chunk(
                backend,
                &mut encoder,
                &uniform,
                concat!(
                    include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
                    include_str!("../../../../spektrafilm-shaders/wgsl/meter_lib.wgsl"),
                    include_str!("luminance.wgsl")
                ),
                MeterChunk {
                    row,
                    first,
                    count,
                    tap_first: 0,
                    tap_count: 0,
                    page_first: 0,
                    page_len: 0,
                    axis: 0,
                },
                &accumulated,
                &luminance,
                count,
                "meter_luminance",
            );
        }
        backend.check_cancelled()?;
        backend.queue.submit(Some(encoder.finish()));
        backend.check_cancelled()?;
    }
    let weights: [f64; 25] = std::array::from_fn(|i| {
        let y = (i / 5) as f64 / 2.0 - 1.0;
        let x = (i % 5) as f64 / 2.0 - 1.0;
        let distance = (x * x + y * y).sqrt() / 2.0f64.sqrt();
        0.5 * (1.0 + (std::f64::consts::PI * distance).cos())
    });
    let weights = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("meter_matrix_weights"),
        contents: bytemuck::cast_slice(&weights),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let pipeline = backend.cached_pipeline(
        concat!(
            include_str!("../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
            include_str!("../../../../spektrafilm-shaders/wgsl/meter_lib.wgsl"),
            include_str!("../../../../spektrafilm-shaders/wgsl/meter_reduce.wgsl")
        ),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: true },
        ],
    );
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("meter_reduce"),
        layout: &pipeline.layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: luminance.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: result.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: weights.as_entire_binding(),
            },
        ],
    });
    Ok(MeterState {
        pipeline,
        group,
        result,
        cancellation: backend.cancellation.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn encode_chunk(
    backend: &WgpuBackend,
    encoder: &mut ObservedEncoder,
    uniform: &wgpu::Buffer,
    shader: &'static str,
    chunk: MeterChunk,
    source: &wgpu::Buffer,
    output: &wgpu::Buffer,
    count: u32,
    label: &str,
) {
    let pipeline = backend.cached_pipeline(
        shader,
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let chunk = backend
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meter_chunk"),
            contents: bytemuck::bytes_of(&chunk),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let group = backend
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: chunk.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: source.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: output.as_entire_binding(),
                },
            ],
        });
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some(label),
        timestamp_writes: None,
    });
    pass.set_pipeline(&pipeline.pipeline);
    pass.set_bind_group(0, &group, &[]);
    dispatch_linear(&mut pass, count);
}

impl MeterState {
    pub(super) fn meter(&self, pipe: &GpuPipe) -> Result<f64, String> {
        let check_cancelled = || {
            if self
                .cancellation
                .as_ref()
                .is_some_and(|token| token.load(std::sync::atomic::Ordering::Relaxed))
            {
                Err("operation cancelled".to_owned())
            } else {
                Ok(())
            }
        };
        check_cancelled()?;
        let staging = pipe.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("meter_ev_readback"),
            size: 8,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        });
        let mut encoder = pipe.device().create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meter_reduce"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline.pipeline);
            pass.set_bind_group(0, &self.group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&self.result, 0, &staging, 0, 8);
        check_cancelled()?;
        pipe.queue().submit(Some(encoder.finish()));
        let ev = pipe.read_mapped::<f64>(&staging, 8)[0];
        check_cancelled()?;
        Ok(ev)
    }
}

#[cfg(test)]
mod tests;
