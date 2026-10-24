use super::blur::DispatchJob;
use super::grain::raster::RasterStorage;
use super::paged::ImagePage;
use super::*;

/// A pointwise operation gathers only bounded windows from physically paged
/// full-source rasters. Every chunk retains its original global coordinates.
pub(super) struct PointwiseJob {
    chunks: Vec<(DispatchJob, u64, u32)>,
    bindings: Vec<PointwiseBinding>,
    bytes_per_item: u64,
    label: String,
}

struct PointwiseBinding {
    transport: Vec<ImagePage>,
    scratch: Option<wgpu::Buffer>,
    access: PointwiseAccess,
}

#[derive(Clone, Copy)]
pub(super) enum PointwiseAccess {
    Read,
    Write,
    ReadWrite,
    Fixed,
}

pub(super) fn image_transport_bytes(
    _backend: &WgpuBackend,
    width: u32,
    height: u32,
    label: &str,
) -> Result<u64, String> {
    let pixels = u64::from(width) * u64::from(height);
    let bytes = pixels.checked_mul(12).ok_or_else(|| format!("{label}: image byte count overflow"))?;
    if pixels == 0 || pixels > u64::from(u32::MAX) {
        return Err(format!("{label}: image needs {pixels} pixels, exceeding logical u32 pixel indexing"));
    }
    Ok(bytes)
}

/// `params` starts with the local item count. `global_offset_word`, when
/// present, identifies the u32 parameter holding the original item index.
#[allow(clippy::too_many_arguments)]
pub(super) fn pointwise_job<P: bytemuck::Pod>(
    backend: &WgpuBackend,
    n: u64,
    bytes_per_item: u64,
    shader: &'static str,
    params: &P,
    buffers: &[(&dyn RasterStorage, PointwiseAccess)],
    global_offset_word: Option<usize>,
    label: &str,
) -> Result<PointwiseJob, String> {
    let limit = super::bounded::storage_binding_limit(backend)?;
    let capacity = (limit / bytes_per_item).min(65_536) as u32;
    if capacity == 0 || n == 0 {
        return Err(format!("{label}: minimum pointwise binding needs {bytes_per_item} bytes; limit is {limit}"));
    }
    let bytes = n * bytes_per_item;
    let aliases = buffers.iter().enumerate().any(|(i, (a, access))| {
        !matches!(access, PointwiseAccess::Read | PointwiseAccess::Fixed)
            && buffers.iter().enumerate().any(|(j, (b, _))| {
                i != j && a.pages().iter().any(|ap| b.pages().iter().any(|bp| ap.buffer == bp.buffer))
            })
    });
    let split = n > u64::from(capacity) || aliases || buffers.iter().any(|(buffer, access)| {
        let pages = buffer.pages();
        !matches!(access, PointwiseAccess::Fixed)
            && (pages.len() != 1 || pages[0].buffer.size() > limit || !pages[0].buffer.usage().contains(wgpu::BufferUsages::STORAGE))
    });
    for (buffer, access) in buffers {
        let required = if matches!(access, PointwiseAccess::Fixed) { buffer.samples() * 4 } else { bytes };
        if required > buffer.samples() * 4 {
            return Err(format!("{label}: raster requires {required} bytes, available {}", buffer.samples() * 4));
        }
        if matches!(access, PointwiseAccess::Fixed) && required > limit {
            return Err(format!("{label}: fixed coefficient binding needs {required} bytes; limit is {limit}"));
        }
        let copy_usage = match access {
            PointwiseAccess::Read => wgpu::BufferUsages::COPY_SRC,
            PointwiseAccess::Write => wgpu::BufferUsages::COPY_DST,
            PointwiseAccess::ReadWrite => wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            PointwiseAccess::Fixed => wgpu::BufferUsages::STORAGE,
        };
        let mut missing_usage = false;
        buffer.visit(&mut |_, page| missing_usage |= !page.usage().contains(copy_usage));
        if (split || matches!(access, PointwiseAccess::Fixed)) && missing_usage {
            return Err(format!("{label}: bounded raster is missing required usage {copy_usage:?}"));
        }
    }
    let mut layout = vec![wgpu::BufferBindingType::Uniform];
    layout.extend(buffers.iter().map(|(_, access)| wgpu::BufferBindingType::Storage {
        read_only: matches!(access, PointwiseAccess::Read | PointwiseAccess::Fixed),
    }));
    let pipeline = backend.cached_pipeline(shader, &layout);
    let bindings: Vec<_> = buffers.iter().map(|(buffer, access)| PointwiseBinding {
        transport: buffer.pages(),
        scratch: (split && !matches!(access, PointwiseAccess::Fixed)).then(|| backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: u64::from(capacity).min(n) * bytes_per_item,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })),
        access: *access,
    }).collect();
    let mut chunks = Vec::new();
    let mut first = 0;
    while first < n {
        let count = (n - first).min(u64::from(capacity)) as u32;
        let mut local_params = bytemuck::bytes_of(params).to_vec();
        local_params[..4].copy_from_slice(&count.to_ne_bytes());
        if let Some(word) = global_offset_word {
            local_params[word * 4..word * 4 + 4].copy_from_slice(&(first as u32).to_ne_bytes());
        }
        let params_buf = backend.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label), contents: &local_params, usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: params_buf.as_entire_binding() }];
        entries.extend(bindings.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry {
            binding: i as u32 + 1,
            resource: b.scratch.as_ref().unwrap_or(&b.transport[0].buffer).as_entire_binding(),
        }));
        let bg = backend.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout: &pipeline.layout, entries: &entries });
        chunks.push((DispatchJob { _params_buf: params_buf, pipeline: pipeline.clone(), bg }, first, count));
        first += u64::from(count);
    }
    Ok(PointwiseJob { chunks, bindings, bytes_per_item, label: label.to_owned() })
}

impl PointwiseJob {
    fn encode_chunk(&self, encoder: &mut ObservedEncoder, job: &DispatchJob, first: u64, count: u32) {
            let offset = first * self.bytes_per_item;
            let bytes = u64::from(count) * self.bytes_per_item;
            for binding in &self.bindings {
                if let Some(scratch) = &binding.scratch
                    && matches!(binding.access, PointwiseAccess::Read | PointwiseAccess::ReadWrite) {
                    binding.transport.read(encoder, offset / 4, bytes / 4, scratch, 0);
                }
            }
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(&self.label), timestamp_writes: None });
                pass.set_pipeline(&job.pipeline.pipeline);
                pass.set_bind_group(0, &job.bg, &[]);
                dispatch_linear(&mut pass, count);
            }
            for binding in &self.bindings {
                if let Some(scratch) = &binding.scratch
                    && matches!(binding.access, PointwiseAccess::Write | PointwiseAccess::ReadWrite) {
                    binding.transport.write(encoder, scratch, 0, offset / 4, bytes / 4);
                }
            }
    }

    pub(super) fn execute(&self, backend: &WgpuBackend) -> Result<(), String> {
        for (job, first, count) in &self.chunks {
            backend.check_cancelled()?;
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            self.encode_chunk(&mut encoder, job, *first, *count);
            backend.check_cancelled()?;
            backend.queue.submit(Some(encoder.finish()));
        }
        backend.check_cancelled()
    }
}

struct ReductionChunk {
    first: u64,
    count: u32,
    jobs: Vec<(DispatchJob, u32)>,
    merge: DispatchJob,
}

pub(super) struct HighlightBoostState {
    image: Vec<ImagePage>,
    input: wgpu::Buffer,
    _buffers: Vec<wgpu::Buffer>,
    _maximum: wgpu::Buffer,
    reductions: Vec<ReductionChunk>,
    boost: PointwiseJob,
}

pub(super) fn build_highlight_boost_state(
    device: &ObservedDevice,
    hp: &spektrafilm_backend::HighlightBoostGpuParams,
    n_pixels: u32,
    img: &(impl RasterStorage + ?Sized),
    backend: &WgpuBackend,
) -> Result<HighlightBoostState, String> {
    let n_values = u64::from(n_pixels) * 3;
    let limit = super::bounded::storage_binding_limit(backend)?;
    let capacity = (limit / 4).min(65_536) as u32;
    if capacity == 0 || n_values == 0 {
        return Err(format!("highlight: minimum maximum-reduction binding needs 4 bytes; limit is {limit}"));
    }
    if img.samples() < n_values {
        return Err("highlight: full-source raster is shorter than its logical image".into());
    }
    let mut missing_copy = false;
    img.visit(&mut |_, page| missing_copy |= !page.usage().contains(wgpu::BufferUsages::COPY_SRC));
    if missing_copy { return Err("highlight: maximum reduction requires COPY_SRC pages".into()); }
    let mk_buffer = |size, label| device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label), size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
    });
    let input = mk_buffer(u64::from(capacity).min(n_values) * 4, "highlight_chunk");
    let maximum = mk_buffer(4, "highlight_global_maximum");
    let mut buffers = Vec::new();
    let mut values = u64::from(capacity).min(n_values) as u32;
    loop {
        values = values.div_ceil(2048).max(1);
        buffers.push(mk_buffer(u64::from(values) * 4, "highlight_reduce"));
        if values == 1 { break; }
    }
    let reduce_pipe = backend.cached_pipeline(include_str!("max_reduce.wgsl"), &[
        wgpu::BufferBindingType::Uniform,
        wgpu::BufferBindingType::Storage { read_only: true },
        wgpu::BufferBindingType::Storage { read_only: false },
    ]);
    let merge_pipe = backend.cached_pipeline(include_str!("max_merge.wgsl"), &[
        wgpu::BufferBindingType::Uniform,
        wgpu::BufferBindingType::Storage { read_only: true },
        wgpu::BufferBindingType::Storage { read_only: false },
    ]);
    let mut reductions = Vec::new();
    let mut first = 0;
    while first < n_values {
        let count = (n_values - first).min(u64::from(capacity)) as u32;
        let mut values = count;
        let mut jobs = Vec::new();
        for (i, output) in buffers.iter().enumerate() {
            let blocks = values.div_ceil(2048).max(1);
            let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("highlight_reduce_params"), contents: bytemuck::cast_slice(&[values, 0, 0, 0, 0, 0, 0, 0]), usage: wgpu::BufferUsages::UNIFORM,
            });
            let src = if i == 0 { &input } else { &buffers[i - 1] };
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("highlight_reduce"), layout: &reduce_pipe.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: src.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: output.as_entire_binding() },
                ],
            });
            jobs.push((DispatchJob { _params_buf: params, pipeline: reduce_pipe.clone(), bg }, blocks));
            values = blocks;
        }
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("highlight_merge_params"), contents: bytemuck::cast_slice(&[u32::from(first == 0), 0, 0, 0]), usage: wgpu::BufferUsages::UNIFORM,
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("highlight_merge"), layout: &merge_pipe.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: buffers.last().unwrap().as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: maximum.as_entire_binding() },
            ],
        });
        reductions.push(ReductionChunk {
            first, count, jobs,
            merge: DispatchJob { _params_buf: params, pipeline: merge_pipe.clone(), bg },
        });
        first += u64::from(count);
    }
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct BoostParams { n_values: u32, boost_ev: f32, boost_range: f32, protect_ev: f32 }
    let boost = pointwise_job(backend, n_values, 4, include_str!("highlight_boost.wgsl"),
        &BoostParams { n_values: 0, boost_ev: hp.boost_ev, boost_range: hp.boost_range, protect_ev: hp.protect_ev },
        &[(&maximum, PointwiseAccess::Fixed), (&img.pages(), PointwiseAccess::ReadWrite)], None, "highlight_boost")?;
    Ok(HighlightBoostState { image: img.pages(), input, _buffers: buffers, _maximum: maximum, reductions, boost })
}

impl HighlightBoostState {
    fn encode_reduction(
        &self, encoder: &mut ObservedEncoder, first: u64, count: u32,
        jobs: &[(DispatchJob, u32)], merge: &DispatchJob,
    ) {
            self.image.read(encoder, first, u64::from(count), &self.input, 0);
            for (job, blocks) in jobs {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("highlight_reduce"), timestamp_writes: None });
                pass.set_pipeline(&job.pipeline.pipeline);
                pass.set_bind_group(0, &job.bg, &[]);
                let (x, y) = dispatch_grid(*blocks);
                pass.dispatch_workgroups(x, y, 1);
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("highlight_merge"), timestamp_writes: None });
            pass.set_pipeline(&merge.pipeline.pipeline);
            pass.set_bind_group(0, &merge.bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
    }

    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        for chunk in &self.reductions {
            backend.check_cancelled()?;
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            self.encode_reduction(&mut encoder, chunk.first, chunk.count, &chunk.jobs, &chunk.merge);
            backend.check_cancelled()?;
            backend.queue.submit(Some(encoder.finish()));
        }
        self.boost.execute(backend)
    }
}

#[cfg(test)]
mod tests;
