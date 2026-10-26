//! Pointwise execution over checked common resident page intersections.
use super::grain::RasterStorage;
use super::page_views::{Access, StorageView, ViewPlan};
use super::*;

pub(super) struct PointwisePass {
    pipeline: CachedPipelineRef,
    plan: ViewPlan,
    uniform: wgpu::Buffer,
    bindings: Vec<usize>,
    dimensions: bool,
    label: String,
}
impl PointwisePass {
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.plan.for_each(backend, |encoder, chunk| {
            let params = backend.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&self.label),
                size: self.uniform.size(),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(&self.uniform, 0, &params, 0, self.uniform.size());
            let geometry = [chunk.count, 1];
            let patch = backend
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("pointwise_geometry"),
                    contents: bytemuck::cast_slice(if self.dimensions {
                        &geometry[..]
                    } else {
                        &geometry[..1]
                    }),
                    usage: wgpu::BufferUsages::COPY_SRC,
                });
            encoder.copy_buffer_to_buffer(
                &patch,
                0,
                &params,
                0,
                if self.dimensions { 8 } else { 4 },
            );
            let mut bases = [0u32; 16];
            for (index, view) in chunk.views.iter().enumerate() {
                bases[index + 1] = view.base;
            }
            let base_buffer =
                backend
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("pointwise_bases"),
                        contents: bytemuck::cast_slice(&bases),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            }];
            entries.extend(chunk.views.iter().enumerate().map(|(index, view)| {
                wgpu::BindGroupEntry {
                    binding: self.bindings[index] as u32,
                    resource: view.binding(),
                }
            }));
            entries.push(wgpu::BindGroupEntry {
                binding: self.bindings.len() as u32 + 1,
                resource: base_buffer.as_entire_binding(),
            });
            let group = backend
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&self.label),
                    layout: &self.pipeline.layout,
                    entries: &entries,
                });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(&self.label),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline.pipeline);
            pass.set_bind_group(0, &group, &[]);
            dispatch_linear(&mut pass, chunk.count);
            Ok(())
        })
    }
}
impl WgpuBackend {
    /// Binding zero is the original uniform. Images use independent scalar
    /// residuals; fixed coefficients retain their original indexing.
    pub(super) fn chain_pointwise(
        &self,
        pipeline: CachedPipelineRef,
        buffers: &[&dyn RasterStorage],
        images: &[(usize, Access)],
        count: u32,
        dimensions: bool,
        label: &str,
    ) -> Result<PointwisePass, String> {
        if buffers.len() < 2 || buffers.len() > 16 {
            return Err(format!("{label}: unsupported pointwise binding count"));
        }
        let uniform = buffers[0].pages();
        if uniform.len() != 1
            || !uniform[0]
                .buffer
                .usage()
                .contains(wgpu::BufferUsages::COPY_SRC)
            || uniform[0].buffer.size() < if dimensions { 8 } else { 4 }
        {
            return Err(format!("{label}: uniform requires one COPY_SRC buffer"));
        }
        if images
            .iter()
            .any(|(index, _)| *index == 0 || *index >= buffers.len())
        {
            return Err(format!("{label}: invalid image binding"));
        }
        let raster_bindings: Vec<_> = buffers
            .iter()
            .enumerate()
            .skip(1)
            .map(|(index, buffer)| {
                (
                    *buffer,
                    images
                        .iter()
                        .find(|(binding, _)| *binding == index)
                        .map_or(Access::Fixed, |(_, access)| *access),
                )
            })
            .collect();
        let plan = ViewPlan::new(self, u64::from(count), 12, &raster_bindings, label)?;
        Ok(PointwisePass {
            pipeline,
            plan,
            uniform: uniform[0].buffer.clone(),
            bindings: (1..buffers.len()).collect(),
            dimensions,
            label: label.into(),
        })
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
        let pipeline = self.cached_pointwise_pipeline(
            include_str!("print_lut_exposure.wgsl"),
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            &[(1, "image")],
        );
        let pages = image.pages();
        self.chain_pointwise(
            pipeline,
            &[&params, &pages],
            &[(1, Access::ReadWrite)],
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
        execute: impl Fn(&StorageView, &StorageView, u32) -> Result<(), String>,
    ) -> Result<(), String> {
        let source = source.pages();
        let output = output.pages();
        let plan = ViewPlan::new(
            self,
            u64::from(count),
            12,
            &[(&source, Access::Read), (&output, Access::Write)],
            "spectral_lut",
        )?;
        plan.for_each_submitted(self, |chunk| {
            execute(&chunk.views[0], &chunk.views[1], chunk.count)
        })
    }
}
