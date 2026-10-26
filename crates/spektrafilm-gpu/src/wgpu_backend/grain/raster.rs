//! Storage windows over physically paged full-source rasters.
use super::super::paged::ImagePage;
use super::super::*;

pub(in crate::wgpu_backend) trait RasterStorage {
    fn visit(&self, visitor: &mut dyn FnMut(u64, &wgpu::Buffer));
    fn image_pages(&self) -> Vec<ImagePage>;
    fn pages(&self) -> Vec<ImagePage> {
        self.image_pages()
    }
    #[cfg(test)]
    fn read(
        &self,
        encoder: &mut ObservedEncoder,
        first_sample: u64,
        count: u64,
        destination: &wgpu::Buffer,
        destination_sample: u64,
    ) {
        let begin = first_sample * 4;
        let end = begin + count * 4;
        self.visit(&mut |first, source| {
            let low = begin.max(first);
            let high = end.min(first + source.size());
            if low < high {
                encoder.copy_buffer_to_buffer(
                    source,
                    low - first,
                    destination,
                    destination_sample * 4 + low - begin,
                    high - low,
                );
            }
        });
    }
    fn write(
        &self,
        encoder: &mut ObservedEncoder,
        source: &wgpu::Buffer,
        source_sample: u64,
        destination_sample: u64,
        count: u64,
    ) {
        let begin = destination_sample * 4;
        let end = begin + count * 4;
        self.visit(&mut |first, destination| {
            let low = begin.max(first);
            let high = end.min(first + destination.size());
            if low < high {
                encoder.copy_buffer_to_buffer(
                    source,
                    source_sample * 4 + low - begin,
                    destination,
                    low - first,
                    high - low,
                );
            }
        });
    }
}

impl RasterStorage for wgpu::Buffer {
    fn visit(&self, visitor: &mut dyn FnMut(u64, &wgpu::Buffer)) {
        visitor(0, self);
    }
    fn image_pages(&self) -> Vec<ImagePage> {
        vec![ImagePage {
            first_pixel: 0,
            n_pixels: self.size().div_ceil(12) as u32,
            buffer: self.clone(),
        }]
    }
}

impl RasterStorage for [ImagePage] {
    fn visit(&self, visitor: &mut dyn FnMut(u64, &wgpu::Buffer)) {
        for page in self {
            visitor(u64::from(page.first_pixel) * 12, &page.buffer);
        }
    }
    fn image_pages(&self) -> Vec<ImagePage> {
        self.to_vec()
    }
}

impl RasterStorage for Vec<ImagePage> {
    fn visit(&self, visitor: &mut dyn FnMut(u64, &wgpu::Buffer)) {
        self.as_slice().visit(visitor);
    }
    fn image_pages(&self) -> Vec<ImagePage> {
        self.clone()
    }
}

/// Allocate whole-pixel STORAGE | COPY_SRC | COPY_DST pages. STORAGE permits
/// raw Gaussian transport kernels to bind capped aligned views directly;
/// RasterStorage still accepts legacy COPY-only pages through bulk adapters.
pub(in crate::wgpu_backend) fn paged_transport(
    backend: &WgpuBackend,
    bytes: u64,
    label: &str,
) -> Result<Vec<ImagePage>, String> {
    let pixels = bytes.div_ceil(12);
    let count =
        u32::try_from(pixels).map_err(|_| format!("{label} logical pixel indexing overflows"))?;
    backend.allocate_image_pages(count, 1, label)
}

pub(in crate::wgpu_backend) enum RasterCommand {
    Copy {
        source: wgpu::Buffer,
        source_offset: u64,
        destination: wgpu::Buffer,
        destination_offset: u64,
        bytes: u64,
    },
    Dispatch {
        pipeline: CachedPipelineRef,
        group: wgpu::BindGroup,
        count: u32,
        groups: bool,
    },
}

#[derive(Default)]
pub(in crate::wgpu_backend) struct RasterCommands(pub Vec<RasterCommand>);

impl RasterCommands {
    pub fn copy(
        &mut self,
        source: &(impl RasterStorage + ?Sized),
        source_offset: u64,
        destination: &(impl RasterStorage + ?Sized),
        destination_offset: u64,
        bytes: u64,
    ) {
        source.visit(&mut |source_first, source_buffer| {
            let begin = source_offset.max(source_first);
            let end = (source_offset + bytes).min(source_first + source_buffer.size());
            if begin >= end {
                return;
            }
            let target_begin = destination_offset + begin - source_offset;
            let target_end = target_begin + end - begin;
            destination.visit(&mut |destination_first, destination_buffer| {
                let low = target_begin.max(destination_first);
                let high = target_end.min(destination_first + destination_buffer.size());
                if low < high {
                    self.0.push(RasterCommand::Copy {
                        source: source_buffer.clone(),
                        source_offset: begin - source_first + low - target_begin,
                        destination: destination_buffer.clone(),
                        destination_offset: low - destination_first,
                        bytes: high - low,
                    });
                }
            });
        });
    }
    pub fn dispatch(
        &mut self,
        pipeline: &CachedPipelineRef,
        buffers: &[&wgpu::Buffer],
        count: u32,
        groups: bool,
        backend: &WgpuBackend,
    ) {
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let group = backend
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("grain_storage_window"),
                layout: &pipeline.layout,
                entries: &entries,
            });
        self.0.push(RasterCommand::Dispatch {
            pipeline: pipeline.clone(),
            group,
            count,
            groups,
        });
    }
    pub fn encode(&self, encoder: &mut ObservedEncoder) {
        for command in &self.0 {
            match command {
                RasterCommand::Copy {
                    source,
                    source_offset,
                    destination,
                    destination_offset,
                    bytes,
                } => encoder.copy_buffer_to_buffer(
                    source,
                    *source_offset,
                    destination,
                    *destination_offset,
                    *bytes,
                ),
                RasterCommand::Dispatch {
                    pipeline,
                    group,
                    count,
                    groups,
                } => {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("grain_storage_window"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&pipeline.pipeline);
                    pass.set_bind_group(0, group, &[]);
                    if *groups {
                        let (x, y) = dispatch_grid(*count);
                        pass.dispatch_workgroups(x, y, 1);
                    } else {
                        dispatch_linear(&mut pass, *count);
                    }
                }
            }
        }
    }
    /// Bound both commands per submission and submitted resource lifetimes.
    /// The queue preserves scratch reuse and stochastic state ordering.
    pub fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.submit_named(backend, "grain_storage_window")
    }

    /// Keep the owning pass observable when non-grain stages reuse transport.
    pub fn submit_named(&self, backend: &WgpuBackend, label: &str) -> Result<(), String> {
        for commands in self.0.chunks(64) {
            backend.check_cancelled()?;
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            for command in commands {
                match command {
                    RasterCommand::Copy {
                        source,
                        source_offset,
                        destination,
                        destination_offset,
                        bytes,
                    } => encoder.copy_buffer_to_buffer(
                        source,
                        *source_offset,
                        destination,
                        *destination_offset,
                        *bytes,
                    ),
                    RasterCommand::Dispatch {
                        pipeline,
                        group,
                        count,
                        groups,
                    } => {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some(label),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(&pipeline.pipeline);
                        pass.set_bind_group(0, group, &[]);
                        if *groups {
                            let (x, y) = dispatch_grid(*count);
                            pass.dispatch_workgroups(x, y, 1);
                        } else {
                            dispatch_linear(&mut pass, *count);
                        }
                    }
                }
            }
            backend.check_cancelled()?;
            backend.queue.submit(Some(encoder.finish()));
        }
        backend.check_cancelled()
    }
}

pub(in crate::wgpu_backend) fn transport(
    backend: &WgpuBackend,
    bytes: u64,
    label: &str,
) -> Result<wgpu::Buffer, String> {
    if bytes == 0 || bytes > backend.device.limits().max_buffer_size {
        return Err(format!(
            "{label} physical transport needs {bytes} bytes, exceeding max_buffer_size"
        ));
    }
    Ok(backend.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    }))
}

pub(in crate::wgpu_backend) fn uniform<P: bytemuck::Pod>(
    backend: &WgpuBackend,
    params: &P,
) -> wgpu::Buffer {
    backend
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_window_params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        })
}
