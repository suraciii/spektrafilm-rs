//! Storage windows over physically paged full-source rasters.
use super::super::observation::ObservedCommand;
use super::super::paged::ImagePage;
use super::super::*;

pub(in crate::wgpu_backend) trait RasterStorage {
    fn visit(&self, visitor: &mut dyn FnMut(u64, &wgpu::Buffer));
    fn image_pages(&self) -> Vec<ImagePage>;
    fn samples(&self) -> u64 {
        let mut samples = 0;
        self.visit(&mut |first, buffer| samples = samples.max((first + buffer.size()) / 4));
        samples
    }
    fn pages(&self) -> Vec<ImagePage> {
        self.image_pages()
    }
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

/// Keep at most two submitted command batches alive while the next is encoded.
/// Submission alone only polls completed work: a fast producer can otherwise
/// retain every command buffer and its transient uniforms behind a slow GPU.
#[derive(Default)]
pub(in crate::wgpu_backend) struct SubmissionWindow {
    latest: Option<wgpu::SubmissionIndex>,
}

impl SubmissionWindow {
    pub fn submit(
        &mut self,
        backend: &WgpuBackend,
        command: ObservedCommand,
    ) -> Result<(), String> {
        backend.check_cancelled()?;
        let index = backend.queue.submit(Some(command));
        if let Some(previous) = self.latest.replace(index) {
            Self::retire(backend, previous)?;
        }
        backend.check_cancelled()
    }

    pub fn finish(&mut self, backend: &WgpuBackend) -> Result<(), String> {
        backend.check_cancelled()?;
        if let Some(index) = self.latest.take() {
            Self::retire(backend, index)?;
        }
        backend.check_cancelled()
    }

    fn retire(backend: &WgpuBackend, index: wgpu::SubmissionIndex) -> Result<(), String> {
        backend.check_cancelled()?;
        // This is resource retirement, not image-map readiness. The observed
        // blocking poll would incorrectly close the whole batch's timing here.
        // As with readback, an entered driver wait may finish after cancellation.
        backend
            .device
            .raw
            .poll(wgpu::Maintain::WaitForSubmissionIndex(index));
        if backend.device.context.enabled() {
            backend
                .device
                .context
                .record_work(&spektrafilm_backend::telemetry::WorkCounters {
                    host_wait_count: Some(1),
                    ..Default::default()
                });
        }
        backend.check_cancelled()
    }
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
        let mut window = SubmissionWindow::default();
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
            backend.check_cancelled()?;
            window.submit(backend, encoder.finish())?;
        }
        window.finish(backend)
    }
}

pub(super) fn transport(
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

pub(super) fn uniform<P: bytemuck::Pod>(backend: &WgpuBackend, params: &P) -> wgpu::Buffer {
    backend
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_window_params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_backend::telemetry::{
        AvailabilityReason, CollectionMode, Measurement, ObservationContext, ObservationKind,
        Purpose,
    };
    use std::sync::mpsc;

    #[test]
    fn submission_window_retires_work_without_claiming_image_readiness() {
        let gpu = crate::test_support::adapter();
        let mut backend = gpu.derived_explicit();
        let context = ObservationContext::new(CollectionMode::Summary);
        let stage = context.scope("grain_spektra_v1", ObservationKind::Stage, Purpose::Image);
        backend.context_mut(stage.context().clone());
        let (backend, batch_guard) = backend.observed_batch("grain_retirement");
        let expected = [17u32, 29, 43, 71];
        let source = backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("retirement_source"),
                contents: bytemuck::cast_slice(&expected),
                usage: wgpu::BufferUsages::COPY_SRC,
            });
        let intermediate = transport(&backend, source.size(), "retirement_intermediate").unwrap();
        let result = backend.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("retirement_readback"),
            size: source.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut window = SubmissionWindow::default();
        let mut first = backend.device.create_command_encoder(&Default::default());
        first.copy_buffer_to_buffer(&source, 0, &intermediate, 0, source.size());
        window.submit(&backend, first.finish()).unwrap();
        let (first_done, first_ready) = mpsc::channel();
        backend.queue.on_submitted_work_done(move || {
            let _ = first_done.send(());
        });
        let mut second = backend.device.create_command_encoder(&Default::default());
        second.copy_buffer_to_buffer(&intermediate, 0, &result, 0, source.size());
        window.submit(&backend, second.finish()).unwrap();
        assert_eq!(
            first_ready.try_recv(),
            Ok(()),
            "oldest batch must be retired"
        );

        let (mapped, map_ready) = mpsc::channel();
        result
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |status| {
                let _ = mapped.send(status);
            });
        let (finished, finish_ready) = mpsc::channel();
        backend.queue.on_submitted_work_done(move || {
            let _ = finished.send(());
        });
        window.finish(&backend).unwrap();
        // No test-side poll or submission may complete the callbacks for us.
        assert_eq!(
            finish_ready.try_recv(),
            Ok(()),
            "final batch must be retired"
        );
        map_ready
            .try_recv()
            .expect("final retirement must deliver the map callback")
            .expect("retired result must map successfully");
        let values = result.slice(..).get_mapped_range();
        assert_eq!(bytemuck::cast_slice::<u8, u32>(&values), &expected);
        drop(values);
        result.unmap();

        // Resource fences are not an image-map completion boundary, even when
        // a test readback happens to map while those fences are being retired.
        drop(batch_guard);
        drop(stage);
        let snapshot = context.snapshot();
        let measurements = snapshot
            .observations
            .iter()
            .find(|observation| observation.kind == ObservationKind::Batch)
            .unwrap()
            .batch
            .as_ref()
            .unwrap();
        let incomplete = Measurement::unavailable(AvailabilityReason::Incomplete);
        assert_eq!(measurements.submit_to_map_ready, incomplete);
        assert_eq!(measurements.host_materialize, incomplete);
    }
}
