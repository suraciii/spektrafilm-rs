use super::*;
use crate::telemetry::*;
use parking_lot::Mutex;
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
    time::Instant,
};
#[derive(Clone)]
pub(super) struct ObservedDevice {
    pub raw: wgpu::Device,
    pub context: ObservationContext,
    pub batch: Option<Arc<Mutex<BatchState>>>,
}
impl Deref for ObservedDevice {
    type Target = wgpu::Device;
    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}
#[derive(Clone)]
pub(super) struct ObservedQueue {
    pub raw: wgpu::Queue,
    pub context: ObservationContext,
    pub batch: Option<Arc<Mutex<BatchState>>>,
}
impl Deref for ObservedQueue {
    type Target = wgpu::Queue;
    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}
pub(super) struct BatchState {
    start: Instant,
    submitted: Option<Instant>,
    ready: Option<Instant>,
    queries: Option<QueryResources>,
    passes: Vec<PassObservation>,
    next_pass: u32,
    executed: u64,
    timed: u64,
    period: f64,
}
pub(super) struct PassObservation {
    pub index: u32,
    pub scope: Option<ObservationContext>,
    pub has_queries: bool,
    pub query_index: u32,
}
struct QueryResources {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    capacity: u32,
    receiver: Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    map_requested: bool,
}
pub(super) struct BatchGuard {
    scope: ObservationScope,
    state: Option<Arc<Mutex<BatchState>>>,
}
impl Drop for BatchGuard {
    fn drop(&mut self) {
        let Some(state) = &self.state else { return };
        let mut s = state.lock();
        let end = Instant::now();
        let prepare = s.submitted.map_or_else(
            || Measurement::unavailable(AvailabilityReason::Incomplete),
            |t| Measurement::available(t.duration_since(s.start).as_secs_f64()),
        );
        let wait = s.submitted.zip(s.ready).map_or_else(
            || Measurement::unavailable(AvailabilityReason::Incomplete),
            |(a, b)| Measurement::available(b.duration_since(a).as_secs_f64()),
        );
        let material = s.ready.map_or_else(
            || Measurement::unavailable(AvailabilityReason::Incomplete),
            |t| Measurement::available(end.duration_since(t).as_secs_f64()),
        );
        let mut sum = 0.0;
        let mut valid = 0u64;
        let mut any = false;
        let mut reason = self.scope.context().adapter_reason().unwrap_or_else(|| {
            if self.scope.context().mode() == CollectionMode::GpuTiming {
                AvailabilityReason::NotEnabledOnDevice
            } else {
                AvailabilityReason::NotCollected
            }
        });
        if let Some(q) = &s.queries {
            reason = AvailabilityReason::NotReady;
            if let Some(result) = q.receiver.as_ref().and_then(|r| r.try_recv().ok()) {
                if result.is_ok() {
                    let mapped = q.readback.slice(..).get_mapped_range();
                    let ticks: &[u64] = match bytemuck::try_cast_slice(&mapped) {
                        Ok(ticks)
                            if ticks.len()
                                >= s.passes
                                    .iter()
                                    .filter(|p| p.has_queries)
                                    .map(|p| p.query_index as usize + 2)
                                    .max()
                                    .unwrap_or(0) =>
                        {
                            ticks
                        }
                        _ => {
                            drop(mapped);
                            reason = AvailabilityReason::InvalidTimestamp;
                            self.scope.context().record_batch(BatchMeasurements {
                                host_prepare: prepare,
                                submit_to_map_ready: wait,
                                host_materialize: material,
                                completion_boundary: "image_map_ready".into(),
                                compute_pass_sum: Measurement::unavailable(reason),
                                partial_compute_pass_sum: Measurement::unavailable(reason),
                            });
                            if q.map_requested {
                                q.readback.unmap();
                            }
                            s.passes.clear();
                            return;
                        }
                    };
                    for p in s.passes.iter() {
                        let i = p.query_index as usize;
                        let d = if !p.has_queries {
                            Measurement::unavailable(AvailabilityReason::DetailLimit)
                        } else if ticks[i + 1] < ticks[i] {
                            Measurement::unavailable(AvailabilityReason::InvalidTimestamp)
                        } else {
                            let d = (ticks[i + 1] - ticks[i]) as f64 * s.period * 1e-9;
                            if d.is_finite() {
                                any = true;
                                sum += d;
                                valid += 1;
                                Measurement::available(d)
                            } else {
                                Measurement::unavailable(AvailabilityReason::InvalidTimestamp)
                            }
                        };
                        if let Some(context) = &p.scope {
                            context.record_pass(p.index as u64, d.clone());
                        }
                    }
                    drop(mapped);
                    if !any {
                        reason = AvailabilityReason::InvalidTimestamp
                    }
                } else {
                    reason = AvailabilityReason::QueryFailed;
                }
            }
        }
        if !any {
            for pass in &s.passes {
                if let Some(context) = &pass.scope {
                    context.record_pass(pass.index as u64, Measurement::unavailable(reason));
                }
            }
        }
        let full = valid == s.timed && s.timed == s.executed && s.submitted.is_some();
        let mut work = WorkCounters::default();
        work.valid_pass_count = Some(valid);
        self.scope.context().record_work(&work);
        let compute = if full {
            Measurement::available(sum)
        } else {
            Measurement::unavailable(if s.submitted.is_none() {
                AvailabilityReason::Incomplete
            } else if self.scope.context().mode() == CollectionMode::GpuTiming
                && s.queries.is_some()
            {
                if any {
                    AvailabilityReason::Incomplete
                } else {
                    reason
                }
            } else if self.scope.context().mode() == CollectionMode::GpuTiming {
                self.scope
                    .context()
                    .adapter_reason()
                    .unwrap_or(AvailabilityReason::NotEnabledOnDevice)
            } else {
                AvailabilityReason::NotCollected
            })
        };
        let partial = if any {
            Measurement::available(sum)
        } else {
            Measurement::unavailable(
                if self.scope.context().mode() == CollectionMode::GpuTiming {
                    reason
                } else {
                    AvailabilityReason::NotCollected
                },
            )
        };
        self.scope.context().record_batch(BatchMeasurements {
            host_prepare: prepare,
            submit_to_map_ready: wait,
            host_materialize: material,
            completion_boundary: "image_map_ready".into(),
            compute_pass_sum: compute,
            partial_compute_pass_sum: partial,
        });
        if let Some(q) = &s.queries {
            if q.map_requested {
                q.readback.unmap();
            }
        }
        s.passes.clear();
    }
}
impl WgpuBackend {
    #[cfg(test)]
    pub(crate) fn context_mut(&mut self, context: ObservationContext) {
        self.device.context = context.clone();
        self.queue.context = context;
    }
    pub(super) fn observed_batch(&self, name: &'static str) -> (Self, BatchGuard) {
        let scope =
            self.device
                .context
                .scope(name, ObservationKind::Batch, self.device.context.purpose());
        let context = scope.context().clone();
        let state = if context.enabled() {
            context.record_executor(Executor::Gpu, None);
            let capacity = (context.detail_remaining().min(1024) * 2).min(2048);
            let queries = if context.mode() == CollectionMode::GpuTiming
                && self
                    .device
                    .raw
                    .features()
                    .contains(wgpu::Features::TIMESTAMP_QUERY)
                && capacity > 0
            {
                let size = capacity as u64 * 8;
                let set = self.device.raw.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("telemetry_timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: capacity,
                });
                let resolve = self.device.raw.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("telemetry_resolve"),
                    size,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                let readback = self.device.raw.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("telemetry_readback"),
                    size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut w = WorkCounters::default();
                w.telemetry_buffer_create_count = Some(2);
                w.telemetry_buffer_create_bytes = Some(size * 2);
                context.record_work(&w);
                Some(QueryResources {
                    set,
                    resolve,
                    readback,
                    capacity,
                    receiver: None,
                    map_requested: false,
                })
            } else {
                None
            };
            Some(Arc::new(Mutex::new(BatchState {
                start: Instant::now(),
                submitted: None,
                ready: None,
                queries,
                passes: Vec::new(),
                next_pass: 0,
                executed: 0,
                timed: 0,
                period: self.queue.raw.get_timestamp_period() as f64,
            })))
        } else {
            None
        };
        let backend = Self {
            device: ObservedDevice {
                raw: self.device.raw.clone(),
                context: context.clone(),
                batch: state.clone(),
            },
            queue: ObservedQueue {
                raw: self.queue.raw.clone(),
                context,
                batch: state.clone(),
            },
            pipeline_cache: self.pipeline_cache.clone(),
            name: self.name.clone(),
            adapter: self.adapter.clone(),
        };
        (backend, BatchGuard { scope, state })
    }
}
fn upload_category(d: &wgpu::util::BufferInitDescriptor<'_>) -> UploadCategory {
    if d.usage.contains(wgpu::BufferUsages::UNIFORM) {
        UploadCategory::Uniform
    } else {
        match d.label.unwrap_or("") {
            "spectral_resource"
            | "film_log_exp"
            | "film_curves"
            | "film_spectral_cd"
            | "film_spectral_bd"
            | "print_spectral_cd"
            | "print_spectral_bd"
            | "viewing_illuminant"
            | "film_density_log_exp"
            | "film_density_curves"
            | "dir_density_log_exp"
            | "dir_density_curves_0"
            | "gamut_cmax"
            | "halation_lut"
            | "unsharp"
            | "scan_xyz_to_rgb"
            | "scan_normalization"
            | "hanatos_table_x"
            | "hanatos_table_y"
            | "hanatos_tc_lut" => UploadCategory::SpectralLut,
            "grain_encoded_input" | "image_input" => UploadCategory::Image,
            _ => UploadCategory::Scratch,
        }
    }
}
impl ObservedDevice {
    pub fn create_buffer(&self, d: &wgpu::BufferDescriptor<'_>) -> wgpu::Buffer {
        let b = self.raw.create_buffer(d);
        if self.context.enabled() {
            let mut w = WorkCounters::default();
            w.buffer_create_count = Some(1);
            w.buffer_create_bytes = Some(d.size);
            self.context.record_work(&w);
        }
        b
    }
    pub fn create_buffer_init(&self, d: &wgpu::util::BufferInitDescriptor<'_>) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        let b = self.raw.create_buffer_init(d);
        if self.context.enabled() {
            let mut w = WorkCounters::default();
            w.buffer_create_count = Some(1);
            w.buffer_create_bytes = Some(b.size());
            self.context.record_work(&w);
            self.upload(d.contents.len() as u64, upload_category(d));
        }
        b
    }
    pub fn upload(&self, bytes: u64, category: UploadCategory) {
        if !self.context.enabled() {
            return;
        }
        let mut w = WorkCounters::default();
        w.upload_count = Some(1);
        w.upload_bytes = Some(bytes);
        match category {
            UploadCategory::Image => {
                w.image_upload_count = Some(1);
                w.image_upload_bytes = Some(bytes)
            }
            UploadCategory::SpectralLut => {
                w.spectral_lut_upload_count = Some(1);
                w.spectral_lut_upload_bytes = Some(bytes)
            }
            UploadCategory::Uniform => {
                w.uniform_upload_count = Some(1);
                w.uniform_upload_bytes = Some(bytes)
            }
            UploadCategory::Scratch => {
                w.scratch_upload_count = Some(1);
                w.scratch_upload_bytes = Some(bytes)
            }
        }
        self.context.record_work(&w);
    }
    pub fn materialized(&self, bytes: u64) {
        if self.context.enabled() {
            let mut w = WorkCounters::default();
            w.readback_count = Some(1);
            w.readback_bytes = Some(bytes);
            self.context.record_work(&w);
        }
    }
    pub fn create_command_encoder(
        &self,
        d: &wgpu::CommandEncoderDescriptor<'_>,
    ) -> ObservedEncoder {
        ObservedEncoder {
            raw: self.raw.create_command_encoder(d),
            context: self.context.clone(),
            batch: self.batch.clone(),
            state: CommandState::default(),
        }
    }
    pub fn poll(&self, maintain: wgpu::Maintain) -> wgpu::MaintainResult {
        let blocking = matches!(
            maintain,
            wgpu::Maintain::Wait | wgpu::Maintain::WaitForSubmissionIndex(_)
        );
        let result = self.raw.poll(maintain);
        if blocking && self.context.enabled() {
            let first_wait = if let Some(batch) = &self.batch {
                let mut batch = batch.lock();
                if batch.ready.is_none() {
                    batch.ready = Some(Instant::now());
                    true
                } else {
                    false
                }
            } else {
                true
            };
            if first_wait {
                let mut work = WorkCounters::default();
                work.host_wait_count = Some(1);
                self.context.record_work(&work);
            }
        }
        result
    }
}
impl ObservedQueue {
    pub fn write_buffer(&self, b: &wgpu::Buffer, offset: u64, data: &[u8]) {
        self.raw.write_buffer(b, offset, data);
        if self.context.enabled() {
            let mut w = WorkCounters::default();
            w.upload_count = Some(1);
            w.upload_bytes = Some(data.len() as u64);
            w.image_upload_count = Some(1);
            w.image_upload_bytes = Some(data.len() as u64);
            self.context.record_work(&w);
        }
    }
    pub fn submit(
        &self,
        commands: impl IntoIterator<Item = ObservedCommand>,
    ) -> wgpu::SubmissionIndex {
        if !self.context.enabled() {
            return self
                .raw
                .submit(commands.into_iter().map(|command| command.raw));
        }
        let submitted = Instant::now();
        let mut count = 0;
        let mut passes = 0;
        let mut timed = 0;
        let mut dispatches = 0;
        let mut staging_count = 0;
        let mut staging_bytes = 0;
        let mut device_count = 0;
        let mut device_bytes = 0;
        let mut resolve_count = 0;
        let mut telemetry_bytes = 0;
        let result = self.raw.submit(commands.into_iter().map(|mut command| {
            count += 1;
            let state = &mut command.state;
            passes += state.pass_count;
            timed += state.timed_count;
            dispatches += state.dispatch_count;
            staging_count += state.stage_count;
            staging_bytes += state.stage_bytes;
            device_count += state.device_count;
            device_bytes += state.device_bytes;
            resolve_count += state.telemetry_resolve_count;
            telemetry_bytes += state.telemetry_copy_bytes;
            if let Some(batch) = &command.batch {
                let mut batch = batch.lock();
                batch.submitted.get_or_insert(submitted);
                batch.executed += state.pass_count;
                batch.timed += state.timed_count;
                batch.passes.append(&mut state.passes);
            }
            command.raw
        }));
        let mut work = WorkCounters::default();
        work.submit_count = Some(1);
        work.command_buffer_count = Some(count);
        work.dispatch_count = Some(dispatches);
        work.executed_pass_count = Some(passes);
        work.timed_pass_count = Some(timed);
        work.omitted_pass_count = Some(passes.saturating_sub(timed));
        work.staging_copy_count = Some(staging_count);
        work.staging_copy_bytes = Some(staging_bytes);
        work.device_copy_count = Some(device_count);
        work.device_copy_bytes = Some(device_bytes);
        work.telemetry_resolve_count = Some(resolve_count);
        work.telemetry_copy_count = Some(resolve_count);
        work.telemetry_copy_bytes = Some(telemetry_bytes);
        if let Some(batch) = &self.batch {
            let mut batch = batch.lock();
            if batch.timed > 0 {
                if let Some(query) = &mut batch.queries {
                    if !query.map_requested {
                        let (tx, rx) = std::sync::mpsc::channel();
                        query
                            .readback
                            .slice(..)
                            .map_async(wgpu::MapMode::Read, move |result| {
                                let _ = tx.send(result);
                            });
                        query.receiver = Some(rx);
                        query.map_requested = true;
                    }
                }
            }
        }
        self.context.record_work(&work);
        result
    }
}
#[derive(Default)]
struct CommandState {
    pass_count: u64,
    timed_count: u64,
    dispatch_count: u64,
    stage_count: u64,
    stage_bytes: u64,
    device_count: u64,
    device_bytes: u64,
    passes: Vec<PassObservation>,
    query_end: u32,
    telemetry_resolve_count: u64,
    telemetry_copy_bytes: u64,
}
pub(super) struct ObservedEncoder {
    raw: wgpu::CommandEncoder,
    context: ObservationContext,
    batch: Option<Arc<Mutex<BatchState>>>,
    state: CommandState,
}
pub(super) struct ObservedCommand {
    raw: wgpu::CommandBuffer,
    batch: Option<Arc<Mutex<BatchState>>>,
    state: CommandState,
}
pub(super) struct ObservedPass<'a> {
    raw: wgpu::ComputePass<'a>,
    state: &'a mut CommandState,
}
impl<'a> Deref for ObservedPass<'a> {
    type Target = wgpu::ComputePass<'a>;
    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}
impl<'a> DerefMut for ObservedPass<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.raw
    }
}
impl ObservedPass<'_> {
    pub fn dispatch_workgroups(&mut self, x: u32, y: u32, z: u32) {
        self.raw.dispatch_workgroups(x, y, z);
        self.state.dispatch_count += 1;
    }
}
fn pass_name(label: Option<&str>) -> &'static str {
    match label.unwrap_or("") {
        "front_transform" => "front_transform",
        "log_exposure" => "log_exposure",
        "film_density" => "film_density",
        "print_density" => "print_density",
        "print_spectral" => "print_spectral",
        "scan_spectral" => "scan_spectral",
        "blur_h" | "simple_blur_h" => "blur_h",
        "blur_v" | "simple_blur_v" => "blur_v",
        "highlight_reduce" => "highlight_reduce",
        "highlight_boost" => "highlight_boost",
        "dir_linear" => "dir_linear",
        "dir_blur_h" => "dir_blur_h",
        "dir_blur_v" => "dir_blur_v",
        "glare_gen" => "glare_gen",
        "glare_apply" => "glare_apply",
        "glare_blur_h" => "glare_blur_h",
        "glare_blur_v" => "glare_blur_v",
        "halation_linear" => "halation_linear",
        "halation_blur_h" => "halation_blur_h",
        "halation_blur_v" => "halation_blur_v",
        "grain_pass" => "grain",
        "gamut_compress" => "gamut_compress",
        "unsharp_blur_h" => "unsharp_blur_h",
        "unsharp_blur_v" => "unsharp_blur_v",
        "unsharp_combine" => "unsharp_combine",
        x if x.starts_with("blur_multi_h") => "blur_h",
        x if x.starts_with("blur_multi_v") => "blur_v",
        _ => "compute",
    }
}
impl ObservedEncoder {
    pub fn begin_compute_pass(&mut self, d: &wgpu::ComputePassDescriptor<'_>) -> ObservedPass<'_> {
        let mut query = None;
        let mut query_index = 0;
        if let Some(batch) = &self.batch {
            let mut b = batch.lock();
            let occurrence = b.next_pass;
            b.next_pass = b.next_pass.saturating_add(1);
            self.state.pass_count += 1;
            let scope = self.context.scope(
                pass_name(d.label),
                ObservationKind::Pass,
                self.context.purpose(),
            );
            let retained = scope.context().retained();
            let slots = b.queries.as_ref().map_or(0, |q| q.capacity);
            query_index = occurrence.saturating_mul(2);
            let has_queries = retained && query_index.saturating_add(2) <= slots;
            if has_queries {
                query = b.queries.as_ref().map(|q| q.set.clone());
                self.state.timed_count += 1;
                self.state.query_end = query_index + 2;
            }
            if retained {
                self.state.passes.push(PassObservation {
                    index: occurrence,
                    scope: Some(scope.context().clone()),
                    has_queries,
                    query_index,
                });
            }
            drop(scope);
        }
        let writes = query.as_ref().map(|set| wgpu::ComputePassTimestampWrites {
            query_set: set,
            beginning_of_pass_write_index: Some(query_index),
            end_of_pass_write_index: Some(query_index + 1),
        });
        let raw = self.raw.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: d.label,
            timestamp_writes: writes,
        });
        ObservedPass {
            raw,
            state: &mut self.state,
        }
    }
    pub fn copy_buffer_to_buffer(
        &mut self,
        src: &wgpu::Buffer,
        a: u64,
        dst: &wgpu::Buffer,
        b: u64,
        size: u64,
    ) {
        self.raw.copy_buffer_to_buffer(src, a, dst, b, size);
        if dst.usage().contains(wgpu::BufferUsages::MAP_READ) {
            self.state.stage_count += 1;
            self.state.stage_bytes += size
        } else {
            self.state.device_count += 1;
            self.state.device_bytes += size
        }
    }
    pub fn finish(mut self) -> ObservedCommand {
        if let Some(batch) = &self.batch {
            let b = batch.lock();
            if let Some(q) = &b.queries {
                let count = self.state.query_end.min(q.capacity);
                if count > 0 {
                    self.raw.resolve_query_set(&q.set, 0..count, &q.resolve, 0);
                    self.raw
                        .copy_buffer_to_buffer(&q.resolve, 0, &q.readback, 0, count as u64 * 8);
                    self.state.telemetry_resolve_count = 1;
                    self.state.telemetry_copy_bytes = count as u64 * 8;
                }
            }
        }
        ObservedCommand {
            raw: self.raw.finish(),
            batch: self.batch,
            state: self.state,
        }
    }
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;
