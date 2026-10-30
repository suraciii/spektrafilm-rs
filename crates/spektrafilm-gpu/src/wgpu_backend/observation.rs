use super::*;
use parking_lot::Mutex;
use spektrafilm_backend::telemetry::*;
use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};
#[derive(Clone)]
pub(super) struct ObservedDevice {
    pub raw: wgpu::Device,
    pub resources: Arc<resources::DeviceResources>,
    pub operation: Option<Arc<resources::OperationState>>,
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
    pub resources: Arc<resources::DeviceResources>,
    pub operation: Option<Arc<resources::OperationState>>,
    pub context: ObservationContext,
}
impl Deref for ObservedQueue {
    type Target = wgpu::Queue;
    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}
pub(super) struct BatchState {
    context: ObservationContext,
    start: Instant,
    submitted: Option<Instant>,
    ready: Option<Instant>,
    pending_details: u32,
    executed: u64,
    timed: u64,
    valid: u64,
    sum: f64,
    reason: AvailabilityReason,
    measurements: Option<BatchMeasurements>,
}
struct PassObservation {
    index: u64,
    scope: Option<ObservationContext>,
    query_index: Option<u32>,
    name: &'static str,
    stage: GpuStage,
    kind: GpuPassKind,
}
impl BatchState {
    fn update_timing(&mut self) {
        let Some(measurements) = &mut self.measurements else {
            return;
        };
        measurements.compute_pass_sum = if self.executed > 0
            && self.valid == self.timed
            && self.timed == self.executed
            && self.sum.is_finite()
        {
            Measurement::available(self.sum)
        } else {
            Measurement::unavailable(if self.valid > 0 {
                AvailabilityReason::Incomplete
            } else {
                self.reason
            })
        };
        measurements.partial_compute_pass_sum = if self.valid > 0 && self.sum.is_finite() {
            Measurement::available(self.sum)
        } else {
            Measurement::unavailable(self.reason)
        };
        self.context.record_batch(measurements.clone());
    }
}
pub(super) struct BatchGuard {
    _scope: ObservationScope,
    state: Option<Arc<Mutex<BatchState>>>,
}
impl Drop for BatchGuard {
    fn drop(&mut self) {
        let Some(state) = &self.state else { return };
        let mut s = state.lock();
        let prepare = s.submitted.map_or_else(
            || Measurement::unavailable(AvailabilityReason::NotReached),
            |t| Measurement::available(t.duration_since(s.start).as_secs_f64()),
        );
        let not_applicable = || Measurement {
            status: MeasurementStatus::NotApplicable,
            value: None,
            reason: Some(AvailabilityReason::NotApplicable),
        };
        s.measurements = Some(BatchMeasurements {
            host_prepare: prepare,
            submit_to_map_ready: not_applicable(),
            host_materialize: not_applicable(),
            completion_boundary: if s.ready.is_some() {
                "device_poll_complete"
            } else {
                "commands_submitted"
            }
            .into(),
            compute_pass_sum: Measurement::unavailable(s.reason),
            partial_compute_pass_sum: Measurement::unavailable(s.reason),
        });
        s.update_timing();
    }
}
impl WgpuBackend {
    pub(super) fn submit_commands(
        &self,
        commands: impl IntoIterator<Item = ObservedCommand>,
    ) -> Result<wgpu::SubmissionIndex, String> {
        self.queue
            .submit_cancellable(commands, self.cancellation.as_ref())
    }

    pub(super) fn context_mut(&mut self, context: ObservationContext) {
        self.device.context = context.clone();
        self.queue.context = context;
        self.pipe = self
            .pipe
            .with_observed(self.device.clone(), self.queue.clone());
    }
    pub(super) fn observed_stage(
        &self,
        name: &'static str,
        stage: GpuStage,
    ) -> (StageBackend<'_>, ObservationScope) {
        let scope = self.device.context.with_gpu_stage(stage).scope(
            name,
            ObservationKind::Stage,
            self.device.context.purpose(),
        );
        let observed = self.device.context.enabled().then(|| {
            let mut backend = self.derived_explicit();
            backend.explicit = self.explicit;
            backend.device.batch = self.device.batch.clone();
            backend.context_mut(scope.context().clone());
            backend
        });
        (
            StageBackend {
                original: self,
                observed,
            },
            scope,
        )
    }

    pub(super) fn observed_batch(&self, name: &'static str) -> (Self, BatchGuard) {
        let scope =
            self.device
                .context
                .scope(name, ObservationKind::Batch, self.device.context.purpose());
        let context = scope.context().clone();
        let state = if context.enabled() {
            context.record_executor(Executor::Gpu, None);
            let reason = if context.mode() != CollectionMode::GpuTiming {
                AvailabilityReason::NotCollected
            } else {
                context
                    .adapter_reason()
                    .unwrap_or(AvailabilityReason::NotReady)
            };
            Some(Arc::new(Mutex::new(BatchState {
                context: context.clone(),
                start: Instant::now(),
                submitted: None,
                ready: None,
                pending_details: 0,
                executed: 0,
                timed: 0,
                valid: 0,
                sum: 0.0,
                reason,
                measurements: None,
            })))
        } else {
            None
        };
        let backend = Self {
            pipe: self.pipe.with_observed(
                ObservedDevice {
                    raw: self.device.raw.clone(),
                    resources: self.device.resources.clone(),
                    operation: self.device.operation.clone(),
                    context: context.clone(),
                    batch: state.clone(),
                },
                ObservedQueue {
                    raw: self.queue.raw.clone(),
                    resources: self.queue.resources.clone(),
                    operation: self.queue.operation.clone(),
                    context: context.clone(),
                },
            ),
            device: ObservedDevice {
                raw: self.device.raw.clone(),
                resources: self.device.resources.clone(),
                operation: self.device.operation.clone(),
                context: context.clone(),
                batch: state.clone(),
            },
            queue: ObservedQueue {
                raw: self.queue.raw.clone(),
                resources: self.queue.resources.clone(),
                operation: self.queue.operation.clone(),
                context,
            },
            pipeline_cache: self.pipeline_cache.clone(),
            scan_cache: self.scan_cache.clone(),
            name: self.name.clone(),
            adapter: self.adapter.clone(),
            explicit: self.explicit,
            backend_id: self.backend_id,
            cancellation: self.cancellation.clone(),
        };
        (
            backend,
            BatchGuard {
                _scope: scope,
                state,
            },
        )
    }
}
/// Off mode borrows the backend: semantic scopes allocate no derivative resources.
pub(super) struct StageBackend<'a> {
    original: &'a WgpuBackend,
    observed: Option<WgpuBackend>,
}
impl Deref for StageBackend<'_> {
    type Target = WgpuBackend;
    fn deref(&self) -> &Self::Target {
        self.observed.as_ref().unwrap_or(self.original)
    }
}

fn upload_category(d: &wgpu::util::BufferInitDescriptor<'_>) -> UploadCategory {
    if d.usage.contains(wgpu::BufferUsages::UNIFORM) {
        UploadCategory::Uniform
    } else {
        match d.label.unwrap_or("") {
            "spectral_resource"
            | "film_cd"
            | "film_bd"
            | "print_illu"
            | "print_sens"
            | "print_log_exp"
            | "print_curves"
            | "scan_cd"
            | "scan_bd"
            | "cmf_x"
            | "cmf_y"
            | "cmf_z"
            | "tc_lut"
            | "film_log_exp"
            | "film_curves"
            | "film_spectral_cd"
            | "film_spectral_bd"
            | "print_spectral_cd"
            | "print_spectral_bd"
            | "scan_illuminant"
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
    fn resource<T>(&self, create: impl FnOnce() -> T) -> T {
        let start = self.context.enabled().then(Instant::now);
        let result = create();
        if let Some(start) = start {
            self.context.record_host_timing(
                HostTimingKind::ResourceCreate,
                start.elapsed().as_secs_f64(),
            );
        }
        result
    }
    pub fn create_shader_module(
        &self,
        descriptor: wgpu::ShaderModuleDescriptor<'_>,
    ) -> wgpu::ShaderModule {
        self.resource(|| self.raw.create_shader_module(descriptor))
    }
    pub fn create_bind_group_layout(
        &self,
        descriptor: &wgpu::BindGroupLayoutDescriptor<'_>,
    ) -> wgpu::BindGroupLayout {
        self.resource(|| self.raw.create_bind_group_layout(descriptor))
    }
    pub fn create_pipeline_layout(
        &self,
        descriptor: &wgpu::PipelineLayoutDescriptor<'_>,
    ) -> wgpu::PipelineLayout {
        self.resource(|| self.raw.create_pipeline_layout(descriptor))
    }
    pub fn create_compute_pipeline(
        &self,
        descriptor: &wgpu::ComputePipelineDescriptor<'_>,
    ) -> wgpu::ComputePipeline {
        self.resource(|| self.raw.create_compute_pipeline(descriptor))
    }
    pub fn create_bind_group(&self, descriptor: &wgpu::BindGroupDescriptor<'_>) -> wgpu::BindGroup {
        self.resource(|| self.raw.create_bind_group(descriptor))
    }
    pub fn create_buffer(&self, d: &wgpu::BufferDescriptor<'_>) -> wgpu::Buffer {
        let b = self.resource(|| self.raw.create_buffer(d));
        if self.context.enabled() {
            let w = WorkCounters {
                buffer_create_count: Some(1),
                buffer_create_bytes: Some(d.size),
                ..Default::default()
            };
            self.context.record_work(&w);
        }
        b
    }
    pub fn create_buffer_init(&self, d: &wgpu::util::BufferInitDescriptor<'_>) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        let start = self.context.enabled().then(Instant::now);
        let b = self.raw.create_buffer_init(d);
        if let Some(start) = start {
            let seconds = start.elapsed().as_secs_f64();
            // This API creates and initializes in one call; these intervals overlap.
            self.context
                .record_host_timing(HostTimingKind::ResourceCreate, seconds);
            self.context
                .record_host_timing(HostTimingKind::Upload, seconds);
            let w = WorkCounters {
                buffer_create_count: Some(1),
                buffer_create_bytes: Some(b.size()),
                ..Default::default()
            };
            self.context.record_work(&w);
            self.upload(d.contents.len() as u64, upload_category(d));
        }
        b
    }
    pub fn upload(&self, bytes: u64, category: UploadCategory) {
        if !self.context.enabled() {
            return;
        }
        let mut w = WorkCounters {
            upload_count: Some(1),
            upload_bytes: Some(bytes),
            ..Default::default()
        };
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
            let w = WorkCounters {
                readback_count: Some(1),
                readback_bytes: Some(bytes),
                ..Default::default()
            };
            self.context.record_work(&w);
        }
    }
    pub fn create_command_encoder(
        &self,
        d: &wgpu::CommandEncoderDescriptor<'_>,
    ) -> ObservedEncoder {
        let timing = self.context.enabled().then(|| EncodingTiming {
            context: self.context.clone(),
            start: Instant::now(),
        });
        let queries = if self.context.mode() == CollectionMode::GpuTiming
            && self
                .raw
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY)
        {
            self.resources.timestamps.acquire(self, &self.context)
        } else {
            None
        };
        let reason = if self.context.mode() != CollectionMode::GpuTiming {
            AvailabilityReason::NotCollected
        } else {
            self.context
                .adapter_reason()
                .unwrap_or(if queries.is_some() {
                    AvailabilityReason::NotReady
                } else {
                    AvailabilityReason::QueryCapacity
                })
        };
        let mut state = CommandState::default();
        state.batch = self.batch.clone();
        state.operation = self.operation.clone();
        state.reason = Some(reason);
        ObservedEncoder {
            raw: self.raw.create_command_encoder(d),
            context: self.context.clone(),
            batch: self.batch.clone(),
            timing,
            queries,
            state,
        }
    }
    pub fn poll(&self, maintain: wgpu::Maintain) -> wgpu::MaintainResult {
        let blocking = matches!(
            maintain,
            wgpu::Maintain::Wait | wgpu::Maintain::WaitForSubmissionIndex(_)
        );
        let start = (blocking && self.context.enabled()).then(Instant::now);
        let result = self.raw.poll(maintain);
        if let Some(start) = start {
            self.context
                .record_host_timing(HostTimingKind::ReadbackWait, start.elapsed().as_secs_f64());
            self.context.record_work(&WorkCounters {
                host_wait_count: Some(1),
                ..Default::default()
            });
            if let Some(batch) = &self.batch {
                batch.lock().ready = Some(Instant::now());
            }
        }
        result
    }
}
impl ObservedQueue {
    pub fn write_buffer(&self, b: &wgpu::Buffer, offset: u64, data: &[u8]) {
        let start = self.context.enabled().then(Instant::now);
        self.raw.write_buffer(b, offset, data);
        if let Some(start) = start {
            self.context
                .record_host_timing(HostTimingKind::Upload, start.elapsed().as_secs_f64());
            let mut w = WorkCounters {
                upload_count: Some(1),
                upload_bytes: Some(data.len() as u64),
                ..Default::default()
            };
            if b.usage().contains(wgpu::BufferUsages::UNIFORM) {
                w.uniform_upload_count = Some(1);
                w.uniform_upload_bytes = Some(data.len() as u64);
            } else {
                w.image_upload_count = Some(1);
                w.image_upload_bytes = Some(data.len() as u64);
            }
            self.context.record_work(&w);
        }
    }
    pub fn submit(
        &self,
        commands: impl IntoIterator<Item = ObservedCommand>,
    ) -> wgpu::SubmissionIndex {
        self.submit_cancellable(commands, None)
            .expect("noncancellable submission cannot be cancelled")
    }

    pub fn submit_cancellable(
        &self,
        commands: impl IntoIterator<Item = ObservedCommand>,
        cancellation: Option<&Arc<AtomicBool>>,
    ) -> Result<wgpu::SubmissionIndex, String> {
        // Commands carry their originating context and owner across backend derivatives.
        // Only a successful submission registers executed work or starts a map.
        let mut owners: Vec<Arc<resources::OperationState>> = Vec::new();
        let mut pending = Vec::new();
        let submitted = Instant::now();
        let result = self.resources.submit(
            &self.raw,
            commands.into_iter().map(|mut command| {
                if let Some(owner) = command.state.operation.take()
                    && self
                        .operation
                        .as_ref()
                        .is_none_or(|current| !Arc::ptr_eq(current, &owner))
                    && !owners.iter().any(|current| Arc::ptr_eq(current, &owner))
                {
                    owners.push(owner);
                }
                if command.context.enabled() {
                    pending.push((command.context, command.state));
                }
                command.raw
            }),
            self.operation.as_ref(),
            cancellation,
            &self.context,
        )?;
        for owner in &owners {
            owner.retain_submission(result.clone());
        }
        let submission_context = if self.context.enabled() {
            Some(&self.context)
        } else {
            pending.first().map(|(context, _)| context)
        };
        if let Some(context) = submission_context {
            context.record_work(&WorkCounters {
                submit_count: Some(1),
                ..Default::default()
            });
        }
        for (context, mut state) in pending {
            let reason = state.reason.unwrap_or(AvailabilityReason::NotCollected);
            let mut detail_attempts = 0;
            if let Some(batch) = &state.batch {
                let mut batch = batch.lock();
                batch.submitted.get_or_insert(submitted);
                let offset = batch.executed;
                batch.executed += state.pass_count;
                batch.timed += state.timed_count;
                if state.timed_count < state.pass_count {
                    batch.reason = reason;
                }
                batch.pending_details -= state.reservation_count;
                state.reservation_count = 0;
                let closed = batch.measurements.is_some();
                for pass in &mut state.passes {
                    pass.index += offset;
                    if let Some(parent) = pass.scope.take()
                        && !closed
                    {
                        detail_attempts += 1;
                        let scope =
                            parent.scope(pass.name, ObservationKind::Pass, parent.purpose());
                        pass.scope = scope.context().retained().then(|| scope.context().clone());
                    }
                }
            }
            context.record_omitted_details(state.pass_count.saturating_sub(detail_attempts));
            for group in &state.groups {
                context.record_gpu_pass_submission(
                    group.stage,
                    group.kind,
                    group.executed,
                    group.timed,
                );
                if group.timed < group.executed {
                    context.record_gpu_pass_unavailable(group.stage, group.kind, reason);
                }
            }
            context.record_work(&WorkCounters {
                command_buffer_count: Some(1),
                dispatch_count: Some(state.dispatch_count),
                executed_pass_count: Some(state.pass_count),
                timed_pass_count: Some(state.timed_count),
                omitted_pass_count: Some(state.pass_count - state.timed_count),
                staging_copy_count: Some(state.stage_count),
                staging_copy_bytes: Some(state.stage_bytes),
                device_copy_count: Some(state.device_count),
                device_copy_bytes: Some(state.device_bytes),
                telemetry_resolve_count: Some(state.telemetry_resolve_count),
                telemetry_copy_count: Some(state.telemetry_resolve_count),
                telemetry_copy_bytes: Some(state.telemetry_copy_bytes),
                ..Default::default()
            });
            if state.dispatch_count > 0 {
                context.record_executor(Executor::Gpu, None);
            }
            for pass in &state.passes {
                if let Some(scope) = &pass.scope {
                    let reason = if pass.query_index.is_some() {
                        AvailabilityReason::NotReady
                    } else {
                        reason
                    };
                    scope.record_pass(pass.index, Measurement::unavailable(reason));
                }
            }
            if let Some(queries) = state.queries.take() {
                let period = f64::from(self.raw.get_timestamp_period());
                let passes = std::mem::take(&mut state.passes);
                let batch = state.batch.clone();
                queries.map(state.query_end, move |result| {
                    let mut valid = 0;
                    let mut sum = 0.0;
                    let mut failure = None;
                    for pass in passes {
                        let Some(index) = pass.query_index else {
                            continue;
                        };
                        let duration = match result {
                            Ok(ticks) => {
                                let i = index as usize;
                                if i + 1 < ticks.len()
                                    && ticks[i + 1] >= ticks[i]
                                    && period.is_finite()
                                    && period > 0.0
                                {
                                    let seconds = (ticks[i + 1] - ticks[i]) as f64 * period * 1e-9;
                                    if seconds.is_finite() {
                                        Measurement::available(seconds)
                                    } else {
                                        Measurement::unavailable(
                                            AvailabilityReason::InvalidTimestamp,
                                        )
                                    }
                                } else {
                                    Measurement::unavailable(AvailabilityReason::InvalidTimestamp)
                                }
                            }
                            Err(reason) => Measurement::unavailable(reason),
                        };
                        if let Some(value) = duration.value {
                            valid += 1;
                            sum += value;
                        } else {
                            failure = duration.reason;
                        }
                        context.record_gpu_pass_duration(pass.stage, pass.kind, duration.clone());
                        if let Some(scope) = pass.scope {
                            scope.record_pass(pass.index, duration);
                        }
                    }
                    if let Some(batch) = batch {
                        let mut batch = batch.lock();
                        batch.valid += valid;
                        batch.sum += sum;
                        if let Some(reason) = failure {
                            batch.reason = reason;
                        }
                        if !batch.sum.is_finite() {
                            batch.reason = AvailabilityReason::CounterOverflow;
                        }
                        batch.update_timing();
                    }
                });
            }
        }
        // A foreign owner's final drop can perform the retirement poll. Keep
        // every owner alive until all maps from this submission are registered.
        drop(owners);
        Ok(result.index)
    }
}
#[derive(Default)]
struct CommandState {
    operation: Option<Arc<resources::OperationState>>,
    pass_count: u64,
    timed_count: u64,
    dispatch_count: u64,
    stage_count: u64,
    stage_bytes: u64,
    device_count: u64,
    device_bytes: u64,
    groups: Vec<PassCounts>,
    queries: Option<timestamps::QueryLease>,
    reason: Option<AvailabilityReason>,
    passes: Vec<PassObservation>,
    query_end: u32,
    telemetry_resolve_count: u64,
    telemetry_copy_bytes: u64,
    batch: Option<Arc<Mutex<BatchState>>>,
    reservation_count: u32,
}
impl Drop for CommandState {
    fn drop(&mut self) {
        if let Some(batch) = &self.batch {
            let mut batch = batch.lock();
            batch.pending_details -= self.reservation_count;
            // An unsubmitted encoder releases its detail reservations and query lease.
        }
    }
}
struct PassCounts {
    stage: GpuStage,
    kind: GpuPassKind,
    executed: u64,
    timed: u64,
}
struct EncodingTiming {
    context: ObservationContext,
    start: Instant,
}
impl Drop for EncodingTiming {
    fn drop(&mut self) {
        self.context.record_host_timing(
            HostTimingKind::CommandEncoding,
            self.start.elapsed().as_secs_f64(),
        );
    }
}
pub(super) struct ObservedEncoder {
    timing: Option<EncodingTiming>,
    raw: wgpu::CommandEncoder,
    context: ObservationContext,
    batch: Option<Arc<Mutex<BatchState>>>,
    state: CommandState,
    queries: Option<timestamps::QueryLease>,
}
pub(super) struct ObservedCommand {
    raw: wgpu::CommandBuffer,
    context: ObservationContext,
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
        // Pipe tail (S4): output transfer encoding and cmyk appearance
        // must identify their executed work rather than the generic name
        // (telemetry metrics: pass names identify the executed work).
        "cctf_transfer_params" => "cctf_transfer_params",
        "output_transfer" => "output_transfer",
        "finishing" => "finishing",
        "cmyk_params" => "cmyk_params",
        "cmyk" => "cmyk",
        x if x.starts_with("blur_multi_h") => "blur_h",
        x if x.starts_with("blur_multi_v") => "blur_v",
        _ => "compute",
    }
}
impl ObservedEncoder {
    pub fn begin_compute_pass(&mut self, d: &wgpu::ComputePassDescriptor<'_>) -> ObservedPass<'_> {
        let mut query_index = None;
        if self.context.enabled() {
            let occurrence = self.state.pass_count;
            self.state.pass_count += 1;
            let (stage, kind) = profiling::classify_pass(self.context.gpu_stage(), d.label);
            if self.queries.is_some() && self.state.query_end + 2 <= timestamps::QUERY_CAPACITY {
                query_index = Some(self.state.query_end);
                self.state.query_end += 2;
                self.state.timed_count += 1;
            } else if self.queries.is_some() {
                self.state.reason = Some(AvailabilityReason::QueryCapacity);
            }
            if let Some(group) = self
                .state
                .groups
                .iter_mut()
                .find(|group| group.stage == stage && group.kind == kind)
            {
                group.executed += 1;
                group.timed += u64::from(query_index.is_some());
            } else {
                self.state.groups.push(PassCounts {
                    stage,
                    kind,
                    executed: 1,
                    timed: u64::from(query_index.is_some()),
                });
            }
            let parent = self.batch.as_ref().and_then(|batch| {
                let mut b = batch.lock();
                if b.pending_details < self.context.detail_remaining() {
                    b.pending_details += 1;
                    self.state.reservation_count += 1;
                    Some(b.context.clone())
                } else {
                    None
                }
            });
            if query_index.is_some() || parent.is_some() {
                self.state.passes.push(PassObservation {
                    index: occurrence,
                    scope: parent,
                    query_index,
                    name: pass_name(d.label),
                    stage,
                    kind,
                });
            }
        }
        let writes = query_index.map(|index| wgpu::ComputePassTimestampWrites {
            query_set: self
                .queries
                .as_ref()
                .expect("reserved timestamp lease")
                .set(),
            beginning_of_pass_write_index: Some(index),
            end_of_pass_write_index: Some(index + 1),
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
    /// Device-side initialization is not a transfer: there is no source
    /// payload, staging copy, upload, or materialization to count.
    pub fn clear_buffer(&mut self, buffer: &wgpu::Buffer, offset: u64, size: Option<u64>) {
        self.raw.clear_buffer(buffer, offset, size);
    }
    pub fn finish(mut self) -> ObservedCommand {
        if self.state.query_end > 0
            && let Some(queries) = self.queries.take()
        {
            queries.resolve(&mut self.raw, self.state.query_end);
            self.state.telemetry_resolve_count = 1;
            self.state.telemetry_copy_bytes = u64::from(self.state.query_end) * 8;
            self.state.queries = Some(queries);
        }
        let raw = self.raw.finish();
        drop(self.timing.take());
        ObservedCommand {
            raw,
            context: self.context,
            state: self.state,
        }
    }
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;
