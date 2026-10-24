//! Bounded pointwise passes over complete device transport rasters. Each
//! operation completes all its pages before the next photographic stage.
use super::grain::RasterStorage;
use super::*;

pub(super) struct PointwisePass {
    pipeline: CachedPipelineRef,
    chunks: Vec<PointwiseChunk>,
}
struct PointwiseChunk {
    group: wgpu::BindGroup,
    first: u64,
    count: u32,
    images: Vec<(Vec<super::paged::ImagePage>, wgpu::Buffer, bool)>,
}
impl PointwisePass {
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        for chunk in &self.chunks {
            backend.check_cancelled()?;
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            self.encode_chunk(chunk, &mut encoder);
            backend.queue.submit(Some(encoder.finish()));
        }
        Ok(())
    }
    fn encode_chunk(&self, chunk: &PointwiseChunk, encoder: &mut ObservedEncoder) {
        for (transport, window, _) in &chunk.images {
            transport.read(
                encoder,
                chunk.first * 3,
                u64::from(chunk.count) * 3,
                window,
                0,
            );
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("bounded_pointwise"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline.pipeline);
            pass.set_bind_group(0, &chunk.group, &[]);
            dispatch_linear(&mut pass, chunk.count);
        }
        for (transport, window, write) in &chunk.images {
            if *write {
                transport.write(
                    encoder,
                    window,
                    0,
                    chunk.first * 3,
                    u64::from(chunk.count) * 3,
                );
            }
        }
    }
}
impl WgpuBackend {
    /// Binding zero is a uniform whose first word is the pixel count (or
    /// width); `dimensions` additionally patches height to one. Other static
    /// bindings retain their original coefficients and precision.
    pub(super) fn chain_pointwise(
        &self,
        pipeline: CachedPipelineRef,
        buffers: &[&dyn RasterStorage],
        images: &[(usize, bool)],
        count: u32,
        dimensions: bool,
        label: &str,
    ) -> Result<PointwisePass, String> {
        let limit = super::bounded::storage_binding_limit(self)?;
        let capacity = super::bounded::chunk_elements(limit, 12)? as u32;
        let mut chunks = Vec::new();
        let scratch: Vec<_> = images
            .iter()
            .map(|(binding, write)| {
                let window = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: u64::from(capacity.min(count)) * 12,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (*binding, *write, window)
            })
            .collect();
        let mut first = 0u32;
        while first < count {
            let n = capacity.min(count - first);
            self.check_cancelled()?;
            let uniform_pages = buffers[0].pages();
            if uniform_pages.len() != 1 {
                return Err(format!("{label}: uniform must fit one physical buffer"));
            }
            let source_uniform = &uniform_pages[0].buffer;
            let params = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("chunk_params"),
                size: source_uniform.size(),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = self.device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(source_uniform, 0, &params, 0, source_uniform.size());
            self.queue.submit(Some(encoder.finish()));
            let geometry = [n, 1];
            let words = if dimensions {
                &geometry[..]
            } else {
                &geometry[..1]
            };
            self.queue
                .write_buffer(&params, 0, bytemuck::cast_slice(words));
            let mut owned = vec![params];
            let mut windows = Vec::new();
            for (index, buffer) in buffers.iter().enumerate().skip(1) {
                if let Some((_, write, window)) =
                    scratch.iter().find(|(binding, _, _)| *binding == index)
                {
                    windows.push((buffer.pages(), window.clone(), *write));
                    owned.push(window.clone());
                } else {
                    let pages = buffer.pages();
                    if pages.len() != 1 || pages[0].buffer.size() > limit {
                        return Err(format!(
                            "{label}: coefficient binding exceeds {limit} bytes"
                        ));
                    }
                    owned.push(pages[0].buffer.clone());
                }
            }
            let entries: Vec<_> = owned
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: b.as_entire_binding(),
                })
                .collect();
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &pipeline.layout,
                entries: &entries,
            });
            chunks.push(PointwiseChunk {
                group,
                first: u64::from(first),
                count: n,
                images: windows,
            });
            first += n;
        }
        Ok(PointwisePass { pipeline, chunks })
    }
}

impl WgpuBackend {
    pub(super) fn diffusion_transport(
        &self,
        width: u32,
        height: u32,
        source: &impl RasterStorage,
        output: &impl RasterStorage,
        params: &spektrafilm_backend::OpticalDiffusionGpuParams<'_>,
        print_exposure: Option<f64>,
    ) -> Result<(), String> {
        let payload = super::film_chain::device_image::BufferPayload::from_pages(
            self.backend_id(),
            source.pages(),
            width,
            height,
        );
        let pages = self.diffusion_pages(&payload, params, print_exposure)?;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for page in pages {
            output.write(
                &mut encoder,
                &page.buffer,
                0,
                u64::from(page.first_pixel) * 3,
                u64::from(page.n_pixels) * 3,
            );
        }
        self.check_cancelled()?;
        self.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    pub(super) fn print_lut_roundtrip(
        &self,
        image: &(impl RasterStorage + ?Sized),
        count: u32,
        scale: f64,
    ) -> Result<PointwisePass, String> {
        let words = [count, (scale as f32).to_bits(), 0, 0];
        let params = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("print_lut_roundtrip_params"),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_SRC,
            });
        let pipeline = self.cached_pipeline(
            include_str!("print_lut_exposure.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
        );
        let pages = image.pages();
        self.chain_pointwise(
            pipeline,
            &[&params, &pages],
            &[(1, true)],
            count,
            false,
            "print_lut_roundtrip",
        )
    }

    pub(super) fn spectral_lut_transport(
        &self,
        source: &(impl RasterStorage + ?Sized),
        output: &(impl RasterStorage + ?Sized),
        count: u32,
        execute: impl Fn(&wgpu::Buffer, &wgpu::Buffer, u32) -> Result<(), String>,
    ) -> Result<(), String> {
        let limit = super::bounded::storage_binding_limit(self)?;
        let capacity = super::bounded::chunk_elements(limit, 12)? as u32;
        let mut first = 0;
        while first < count {
            self.check_cancelled()?;
            let n = capacity.min(count - first);
            let make = |label| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: u64::from(n) * 12,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            let input = make("spectral_lut_window_input");
            let result = make("spectral_lut_window_output");
            let mut encoder = self.device.create_command_encoder(&Default::default());
            source.read(
                &mut encoder,
                u64::from(first) * 3,
                u64::from(n) * 3,
                &input,
                0,
            );
            self.queue.submit(Some(encoder.finish()));
            execute(&input, &result, n)?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            output.write(
                &mut encoder,
                &result,
                0,
                u64::from(first) * 3,
                u64::from(n) * 3,
            );
            self.queue.submit(Some(encoder.finish()));
            first += n;
        }
        Ok(())
    }
}
