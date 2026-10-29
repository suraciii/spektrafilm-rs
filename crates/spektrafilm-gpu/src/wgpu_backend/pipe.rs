//! GPU pipe execution core — the internal carrier defined by
//! `specs/gpu-execution/design.md` (pipe assembly).
//!
//! The pipe owns buffer reuse and the host transfer points: input upload,
//! fences, and the final readback. Per-stage trait methods dispatch through
//! `dispatch_host` with unchanged stage placement and telemetry semantics.
//! The assembled chain runs through `ChainStage` with `readback_buffer` and
//! `write_image_buffer` at each fence.
//!
//! The device-resident primitives (`DeviceImage`, `dispatch_device`,
//! `upload_image`, `fence`, `readback_image`) are a complete zero-copy
//! execution path that the production chain does not take: production
//! materializes at every fence. They are kept because they pin the pipe's
//! transfer contract and are exercised by this module's own tests.

use super::cache::PipelineCache;
use super::observation::{ObservedDevice, ObservedQueue};
use super::{GpuBuffer, dispatch_linear};
use parking_lot::Mutex;
use spektrafilm_backend::telemetry::*;
/// A device-resident RGB f32 image: storage buffer, pixel count, byte size.
#[allow(dead_code)]
pub(super) struct DeviceImage {
    pub(super) buf: wgpu::Buffer,
    pub(super) n_pixels: u32,
    pub(super) bytes: u64,
}

/// Pool of reusable MAP_READ readback buffers keyed by byte size. A submitted
/// buffer returns only after mapping and unmapping finish; a cancelled copy
/// that was never submitted can return immediately.
#[derive(Default)]
struct ReadbackPool {
    buffers: Vec<(u64, wgpu::Buffer)>,
}

/// Cap pooled readback buffers so pathological image sizes cannot
/// accumulate device memory across frames.
const READBACK_POOL_MAX_BUFFERS: usize = 8;
const READBACK_POOL_MAX_BYTES: u64 = super::resources::WORKSPACE_BINDING_BYTES;

/// Cap the cross-frame static-resource cache so a parameter sweep (every frame
/// a new configuration) cannot grow device memory or the host-side content
/// copies without bound. Coldest entries are evicted first; eviction is safe
/// because wgpu keeps a buffer alive while a submitted command buffer still
/// uses it.
const STATIC_CACHE_MAX_ENTRIES: usize = 64;
const STATIC_CACHE_MAX_BYTES: u64 = 8 * 1024 * 1024;

struct StaticEntry {
    key: u64,
    usage: wgpu::BufferUsages,
    contents: Vec<u8>,
    buffer: wgpu::Buffer,
}

/// Cross-frame cache of device buffers addressed by their contents: spectral
/// lookups, density curves, uniform blocks, and blur kernels. Some cached
/// uniforms carry the image geometry, so identity is the buffer content, not a
/// configuration hash. The key hashes the usage flags and the contents, and a
/// hit additionally compares both, so a hash collision cannot serve a wrong
/// buffer. Owned by the pipe, so derived backends that share the pipe share
/// these buffers too (pipe architecture F3a).
#[derive(Default)]
pub(super) struct StaticBufferCache {
    entries: Mutex<Vec<StaticEntry>>,
}

impl StaticBufferCache {
    fn key(usage: wgpu::BufferUsages, contents: &[u8]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        usage.bits().hash(&mut hasher);
        contents.hash(&mut hasher);
        hasher.finish()
    }

    /// Return a device buffer holding `contents`. `label` applies to the miss
    /// that creates the buffer; a later hit keeps the first label.
    pub(super) fn get_or_create(
        &self,
        device: &ObservedDevice,
        label: &str,
        contents: &[u8],
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        let key = Self::key(usage, contents);
        let mut entries = self.entries.lock();
        if let Some(position) = entries.iter().position(|entry| {
            entry.key == key && entry.usage == usage && entry.contents == contents
        }) {
            let entry = entries.remove(position);
            let buffer = entry.buffer.clone();
            entries.insert(0, entry);
            return buffer;
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage,
        });
        entries.insert(
            0,
            StaticEntry {
                key,
                usage,
                contents: contents.to_vec(),
                buffer: buffer.clone(),
            },
        );
        while entries.len() > STATIC_CACHE_MAX_ENTRIES
            || entries.iter().map(|entry| entry.buffer.size()).sum::<u64>() > STATIC_CACHE_MAX_BYTES
        {
            entries.pop();
        }
        buffer
    }
}

#[cfg(test)]
#[derive(Default)]
struct TestFailure {
    operation: Option<&'static str>,
    attempts: usize,
}

#[cfg(test)]
type ReadbackHook = Box<dyn FnMut() + Send>;

pub(super) struct GpuPipe {
    device: ObservedDevice,
    queue: ObservedQueue,
    cache: std::sync::Arc<PipelineCache>,
    pool: std::sync::Arc<Mutex<ReadbackPool>>,
    static_cache: std::sync::Arc<StaticBufferCache>,
    pub(super) viewer_icc_cache: std::sync::Arc<Mutex<super::viewer_icc::IccDeviceCache>>,
    pub(super) dye_certification: std::sync::Arc<std::sync::OnceLock<Result<(), String>>>,
    pub(super) dye_scalar_certification: std::sync::Arc<std::sync::OnceLock<Result<(), String>>>,
    cancellation: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    #[cfg(test)]
    test_failure: std::sync::Arc<Mutex<TestFailure>>,
    #[cfg(test)]
    test_readback_hook: std::sync::Arc<Mutex<Option<ReadbackHook>>>,
}

impl GpuPipe {
    pub(super) fn retained_cache_bytes(&self) -> Result<u64, String> {
        let pool = self.pool.lock();
        let cache = self.static_cache.entries.lock();
        pool.buffers
            .iter()
            .map(|(_, buffer)| buffer.size())
            .chain(cache.iter().map(|entry| entry.buffer.size()))
            .try_fold(0u64, |bytes, size| {
                bytes
                    .checked_add(size)
                    .ok_or_else(|| "retained GPU pipe capacity overflows".to_owned())
            })
    }

    pub(super) fn new(
        device: ObservedDevice,
        queue: ObservedQueue,
        cache: std::sync::Arc<PipelineCache>,
    ) -> Self {
        Self {
            device,
            queue,
            cache,
            pool: std::sync::Arc::new(Mutex::new(ReadbackPool::default())),
            static_cache: std::sync::Arc::new(StaticBufferCache::default()),
            viewer_icc_cache: std::sync::Arc::new(Mutex::new(
                super::viewer_icc::IccDeviceCache::default(),
            )),
            dye_certification: std::sync::Arc::new(std::sync::OnceLock::new()),
            dye_scalar_certification: std::sync::Arc::new(std::sync::OnceLock::new()),
            cancellation: None,
            #[cfg(test)]
            test_failure: std::sync::Arc::new(Mutex::new(TestFailure::default())),
            #[cfg(test)]
            test_readback_hook: std::sync::Arc::new(Mutex::new(None)),
        }
    }

    /// Derive an observed pipe sharing this pipe's pipeline cache, buffer pool,
    /// and cross-frame static-resource cache. Observed stage instances rebind
    /// the backend per stage; sharing them keeps readback buffers reusable
    /// across those rebinds and across frames, and keeps configuration-invariant
    /// device buffers resident (pipe architecture S5/F3a).
    pub(super) fn with_observed(&self, device: ObservedDevice, queue: ObservedQueue) -> Self {
        Self {
            device,
            queue,
            cache: std::sync::Arc::clone(&self.cache),
            pool: std::sync::Arc::clone(&self.pool),
            static_cache: std::sync::Arc::clone(&self.static_cache),
            viewer_icc_cache: std::sync::Arc::clone(&self.viewer_icc_cache),
            dye_certification: std::sync::Arc::clone(&self.dye_certification),
            dye_scalar_certification: std::sync::Arc::clone(&self.dye_scalar_certification),
            cancellation: self.cancellation.clone(),
            #[cfg(test)]
            test_failure: std::sync::Arc::clone(&self.test_failure),
            #[cfg(test)]
            test_readback_hook: self.test_readback_hook.clone(),
        }
    }

    /// Rebind one operation's signal without changing shared device resources.
    pub(super) fn with_cancellation(
        &self,
        cancellation: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Self {
        let mut pipe = self.with_observed(self.device.clone(), self.queue.clone());
        pipe.cancellation = cancellation;
        pipe
    }

    pub(super) fn check_cancelled(&self) -> Result<(), String> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(|token| token.load(std::sync::atomic::Ordering::Relaxed))
        {
            Err("operation cancelled".to_owned())
        } else {
            Ok(())
        }
    }

    /// Inject a resource failure only in this backend and its observed derivatives.
    #[cfg(test)]
    pub(super) fn test_fail_operation(&self, operation: &'static str) {
        *self.test_failure.lock() = TestFailure {
            operation: Some(operation),
            attempts: 0,
        };
    }

    /// Count attempts independently of telemetry, including CollectionMode::Off.
    #[cfg(test)]
    pub(super) fn test_failure_attempts(&self) -> usize {
        self.test_failure.lock().attempts
    }

    #[cfg(test)]
    pub(super) fn test_operation(&self, operation: &'static str) -> Result<(), String> {
        let mut failure = self.test_failure.lock();
        if failure.operation == Some(operation) {
            failure.attempts += 1;
            return Err(format!("{operation}: injected device resource failure"));
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn test_on_readback(&self, hook: impl FnMut() + Send + 'static) {
        *self.test_readback_hook.lock() = Some(Box::new(hook));
    }

    /// The observed device, for a stage that encodes its own command buffer.
    pub(super) fn device(&self) -> &ObservedDevice {
        &self.device
    }

    /// The observed queue, for a stage that submits its own command buffer.
    pub(super) fn queue(&self) -> &ObservedQueue {
        &self.queue
    }

    /// A device buffer holding configuration-invariant contents, reused across
    /// frames (and across every backend that shares this pipe) when the same
    /// bytes were uploaded before.
    pub(super) fn static_buffer(
        &self,
        label: &str,
        contents: &[u8],
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        self.static_cache
            .get_or_create(&self.device, label, contents, usage)
    }

    /// Host-image dispatch for the per-stage route. Semantics, including
    /// telemetry, match the former `WgpuBackend::dispatch_compute`: every
    /// binding is uploaded, the kernel runs once, and the output binding is
    /// materialized on the host.
    pub(super) fn dispatch_host(
        &self,
        shader_source: &'static str,
        bindings: &[GpuBuffer],
        n_pixels: u32,
        output_idx: usize,
        name: &'static str,
    ) -> Option<Vec<f32>> {
        #[cfg(test)]
        if self.test_operation(name).is_err() {
            return None;
        }
        // Reject unrepresentable bindings before wgpu's fatal validation.
        // The selected-device caller converts this decline to an operation error.
        if let Some(reason) = bindings_exceed_device_limits(bindings, &self.device.limits()) {
            tracing::info!(
                execution = "gpu_rejected",
                %reason,
                "device dispatch rejected because a buffer exceeds resource limits"
            );
            return None;
        }
        let additional = bindings.iter().try_fold(
            bindings[output_idx].data.len() as u64,
            |bytes, binding| {
                let allocation = (binding.data.len() as u64).max(4).checked_add(3)? / 4 * 4;
                bytes.checked_add(allocation)
            },
        )?;
        // A chain already owns its complete checked working set. Standalone
        // host dispatches still share admission with render/export derivatives.
        let operation = if self.queue.operation.is_none() {
            let cached = if self.device.resources.native_buffer_bytes().is_none() {
                self.retained_cache_bytes().ok()?
            } else {
                0
            };
            Some(
                self.device
                    .resources
                    .admit(
                        additional,
                        cached,
                        self.cancellation.as_ref(),
                        &self.device.context,
                    )
                    .ok()?,
            )
        } else {
            None
        };
        let queue = operation.as_ref().map_or_else(
            || self.queue.clone(),
            |operation| operation.queue(&self.queue),
        );
        let binding_types: Vec<wgpu::BufferBindingType> =
            bindings.iter().map(|b| b.binding_type).collect();
        let (cached, hit) = self
            .cache
            .get_or_compile(&self.device, shader_source, &binding_types);
        if self.device.context.enabled() {
            let mut work = WorkCounters::default();
            if hit {
                work.pipeline_cache_hit_count = Some(1);
            } else {
                work.pipeline_cache_miss_count = Some(1);
            }
            self.device.context.record_work(&work);
        }
        let pipeline = &cached.pipeline;
        let bind_group_layout = &cached.layout;

        let gpu_buffers: Vec<wgpu::Buffer> = bindings
            .iter()
            .enumerate()
            .map(|(index, b)| {
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(if index == 1 {
                            "image_input"
                        } else if index == output_idx {
                            "scratch_init"
                        } else {
                            "spectral_resource"
                        }),
                        contents: &b.data,
                        usage: b.usage,
                    })
            })
            .collect();

        let bind_entries: Vec<wgpu::BindGroupEntry> = gpu_buffers
            .iter()
            .enumerate()
            .map(|(i, buf)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: buf.as_entire_binding(),
            })
            .collect();
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compute_bind_group"),
            layout: bind_group_layout,
            entries: &bind_entries,
        });

        let output_size = bindings[output_idx].data.len() as u64;
        let readback = self.take_readback(output_size);

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(name),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            dispatch_linear(&mut pass, n_pixels);
        }
        encoder.copy_buffer_to_buffer(&gpu_buffers[output_idx], 0, &readback, 0, output_size);
        if queue
            .submit_cancellable(Some(encoder.finish()), self.cancellation.as_ref())
            .is_err()
        {
            self.return_readback(readback, output_size);
            return None;
        }

        let out = self.read_mapped_f32(&readback, output_size);
        self.return_readback(readback, output_size);
        Some(out)
    }

    /// Upload host RGB f32 samples as a device image. One of the three pipe
    /// transfer points.
    #[allow(dead_code)]
    pub(super) fn upload_image(&self, data: &[f32], n_pixels: u32) -> DeviceImage {
        let bytes = std::mem::size_of_val(data) as u64;
        let buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("pipe_upload"),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });
        DeviceImage {
            buf,
            n_pixels,
            bytes,
        }
    }

    /// Device-resident dispatch: `resources` (uniforms, lookup tables) are
    /// uploaded, the input image stays on the device, and the output is a
    /// fresh device image of the same size. No host materialization.
    /// Binding order is `resources..., input, output`.
    #[allow(dead_code)]
    pub(super) fn dispatch_device(
        &self,
        shader_source: &'static str,
        input: &DeviceImage,
        resources: &[GpuBuffer],
        n_pixels: u32,
        name: &'static str,
    ) -> Option<DeviceImage> {
        if let Some(reason) = bindings_exceed_device_limits(resources, &self.device.limits()) {
            tracing::info!(
                execution = "cpu",
                %reason,
                "using the CPU stage because a buffer exceeds the device resource limits"
            );
            return None;
        }
        for (label, bytes) in [("pipe_input", input.bytes), ("pipe_stage_out", input.bytes)] {
            if let Some(reason) =
                super::film_chain::required_buffer_exceeds_limits(bytes, &self.device.limits())
            {
                tracing::info!(
                    execution = "cpu",
                    binding = label,
                    %reason,
                    "using the CPU stage because a buffer exceeds the device resource limits"
                );
                return None;
            }
        }
        let mut binding_types: Vec<wgpu::BufferBindingType> =
            resources.iter().map(|b| b.binding_type).collect();
        binding_types.push(wgpu::BufferBindingType::Storage { read_only: true });
        binding_types.push(wgpu::BufferBindingType::Storage { read_only: false });
        let (cached, _hit) = self
            .cache
            .get_or_compile(&self.device, shader_source, &binding_types);

        let resource_buffers: Vec<wgpu::Buffer> = resources
            .iter()
            .map(|b| {
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("pipe_resource"),
                        contents: &b.data,
                        usage: b.usage,
                    })
            })
            .collect();
        let out = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pipe_stage_out"),
            size: input.bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let mut entries: Vec<wgpu::BindGroupEntry> = resource_buffers
            .iter()
            .enumerate()
            .map(|(index, buf)| wgpu::BindGroupEntry {
                binding: index as u32,
                resource: buf.as_entire_binding(),
            })
            .collect();
        let n_res = resource_buffers.len() as u32;
        entries.push(wgpu::BindGroupEntry {
            binding: n_res,
            resource: input.buf.as_entire_binding(),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: n_res + 1,
            resource: out.as_entire_binding(),
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pipe_bind_group"),
            layout: &cached.layout,
            entries: &entries,
        });

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(name),
                timestamp_writes: None,
            });
            pass.set_pipeline(&cached.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            dispatch_linear(&mut pass, n_pixels);
        }
        self.queue
            .submit_cancellable(Some(encoder.finish()), self.cancellation.as_ref())
            .ok()?;
        Some(DeviceImage {
            buf: out,
            n_pixels,
            bytes: input.bytes,
        })
    }

    /// Fence: materialize the device image, run the CPU stage on host
    /// samples, and re-upload its output. The two pipe transfer points an
    /// assembled `Cpu` node contributes.
    #[allow(dead_code)]
    pub(super) fn fence(
        &self,
        input: &DeviceImage,
        cpu: impl FnOnce(Vec<f32>) -> Vec<f32>,
    ) -> DeviceImage {
        let host = self.readback_image(input);
        let out = cpu(host);
        self.upload_image(&out, input.n_pixels)
    }

    /// Infallible final readback for the legacy device-image/fence helpers.
    /// These helpers are not part of the cancellable production chain.
    #[allow(dead_code)]
    pub(super) fn readback_image(&self, image: &DeviceImage) -> Vec<f32> {
        let readback = self.take_readback(image.bytes);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&image.buf, 0, &readback, 0, image.bytes);
        self.queue.submit(Some(encoder.finish()));
        let out = self.read_mapped_f32(&readback, image.bytes);
        self.return_readback(readback, image.bytes);
        out
    }
    /// Readback of f32 stages, the crate's dominant element type.
    pub(super) fn read_mapped_f32(&self, buffer: &wgpu::Buffer, bytes: u64) -> Vec<f32> {
        self.read_mapped(buffer, bytes)
    }

    /// The one readback protocol: map `buffer`, wait for the GPU, record the
    /// materialized bytes, then copy the mapped range out.
    pub(super) fn read_mapped<T: bytemuck::Pod>(
        &self,
        buffer: &wgpu::Buffer,
        bytes: u64,
    ) -> Vec<T> {
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        self.device.materialized(bytes);
        let data = slice.get_mapped_range();
        let out: Vec<T> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        buffer.unmap();
        out
    }

    fn take_readback(&self, size: u64) -> wgpu::Buffer {
        let mut pool = self.pool.lock();
        if let Some(pos) = pool.buffers.iter().position(|(sz, _)| *sz == size) {
            let (_, buf) = pool.buffers.swap_remove(pos);
            buf
        } else {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pipe_readback"),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        }
    }

    fn return_readback(&self, buffer: wgpu::Buffer, size: u64) {
        let mut pool = self.pool.lock();
        if pool.buffers.len() < READBACK_POOL_MAX_BUFFERS
            && size
                <= READBACK_POOL_MAX_BYTES
                    .saturating_sub(pool.buffers.iter().map(|(bytes, _)| *bytes).sum())
        {
            pool.buffers.push((size, buffer));
        }
    }
}

/// One ordered device-only chain node. Device operations own bounded
/// submissions and can stop between chunks without publishing partial output.
pub(super) enum ChainStage<'a> {
    /// A device operation that owns bounded submissions and can fail before
    /// publication. It never materializes an image or runs host numerics.
    Device {
        name: &'static str,
        execute: Box<dyn FnOnce() -> Result<(), String> + 'a>,
    },
}

impl GpuPipe {
    /// Execute ordered device submissions. Buffers stay resident;
    /// queue ordering provides dependencies without host image readbacks.
    /// Banded workloads expose each band as a separate node.
    pub(super) fn run_chain(&self, stages: Vec<ChainStage<'_>>) -> Result<(), String> {
        for stage in stages {
            self.check_cancelled()?;
            match stage {
                ChainStage::Device { name, execute } => {
                    self.device.context.chain_progress(name, true);
                    let result = execute();
                    self.device.context.chain_progress(name, false);
                    result?;
                }
            }
        }
        self.check_cancelled()?;
        Ok(())
    }

    /// Infallible diagnostic materialization through the pooled readback path.
    /// Records one readback of `bytes`; request work instead uses
    /// `readback_buffer_cancellable` so cancellation can prevent its COPY.
    /// The source stays a plain storage buffer; only the pooled copy target
    /// carries `MAP_READ | COPY_DST`.
    pub(super) fn readback_buffer_as<T: bytemuck::Pod>(
        &self,
        buf: &wgpu::Buffer,
        bytes: u64,
    ) -> Vec<T> {
        let readback = self.take_readback(bytes);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buf, 0, &readback, 0, bytes);
        self.queue.submit(Some(encoder.finish()));
        let out = self.read_mapped::<T>(&readback, bytes);
        self.return_readback(readback, bytes);
        out
    }

    /// Cancellation can prevent the copy submission, but cannot interrupt a
    /// driver wait already entered. Always finish mapping and return the pooled
    /// buffer before discarding a cancelled readback.
    pub(super) fn readback_buffer_cancellable<T: bytemuck::Pod>(
        &self,
        buffer: &wgpu::Buffer,
        bytes: u64,
    ) -> Result<Vec<T>, String> {
        self.check_cancelled()?;
        let readback = self.take_readback(bytes);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &readback, 0, bytes);
        if let Err(error) = self
            .queue
            .submit_cancellable(Some(encoder.finish()), self.cancellation.as_ref())
        {
            self.return_readback(readback, bytes);
            return Err(error);
        }
        let result = self.read_mapped::<T>(&readback, bytes);
        self.return_readback(readback, bytes);
        #[cfg(test)]
        if let Some(hook) = self.test_readback_hook.lock().as_mut() {
            hook();
        }
        self.check_cancelled()?;
        Ok(result)
    }

    /// The f32 specialization for noncancellable diagnostic materialization.
    pub(super) fn readback_buffer(&self, buf: &wgpu::Buffer, bytes: u64) -> Vec<f32> {
        self.readback_buffer_as(buf, bytes)
    }
}

/// The device-limit preflight for one dispatch's bindings. Returns the reason
/// of the first binding the device cannot represent, so a dispatch declines
/// to its caller's CPU implementation instead of reaching wgpu's fatal
/// validation error. The buffer-size limits follow the resident chain's
/// [`super::film_chain::required_buffer_exceeds_limits`]; uniform bindings
/// additionally observe the smaller uniform binding limit.
fn bindings_exceed_device_limits(bindings: &[GpuBuffer], limits: &wgpu::Limits) -> Option<String> {
    bindings.iter().enumerate().find_map(|(index, binding)| {
        let bytes = binding.data.len() as u64;
        let binding_limit = match binding.binding_type {
            wgpu::BufferBindingType::Uniform => u64::from(limits.max_uniform_buffer_binding_size),
            wgpu::BufferBindingType::Storage { .. } => {
                u64::from(limits.max_storage_buffer_binding_size)
            }
        };
        if bytes > binding_limit {
            return Some(format!(
                "binding {index} of {bytes} bytes exceeds the buffer binding size limit \
                 ({binding_limit} bytes)"
            ));
        }
        super::film_chain::required_buffer_exceeds_limits(bytes, limits)
    })
}

#[cfg(all(test, feature = "wgpu-backend"))]
mod logic_tests {
    use super::*;

    #[test]
    fn bindings_over_the_device_limits_decline_instead_of_binding() {
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: 1024,
            max_uniform_buffer_binding_size: 256,
            max_buffer_size: 2048,
            ..wgpu::Limits::default()
        };
        let fitting = [
            GpuBuffer::uniform(&[0; 128]),
            GpuBuffer::storage_ro(&[0; 512]),
        ];
        assert!(bindings_exceed_device_limits(&fitting, &limits).is_none());
        let uniform_over = [GpuBuffer::uniform(&[0; 512])];
        assert!(bindings_exceed_device_limits(&uniform_over, &limits).is_some());
        let storage_over = [GpuBuffer::storage_ro(&[0; 2048])];
        assert!(bindings_exceed_device_limits(&storage_over, &limits).is_some());
    }
}

#[cfg(all(test, feature = "wgpu-backend"))]
mod tests {
    use super::*;

    #[test]
    fn chain_cancellation_stops_the_next_stage_without_poisoning_shared_pipe() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let backend = crate::test_support::adapter();
        let token = Arc::new(AtomicBool::new(false));
        let pipe = backend.pipe.with_cancellation(Some(token.clone()));
        let observed = pipe.with_observed(pipe.device.clone(), pipe.queue.clone());
        let second_executed = AtomicBool::new(false);
        let result = observed.run_chain(vec![
            ChainStage::Device {
                name: "cancel_operation",
                execute: Box::new(|| {
                    token.store(true, Ordering::Relaxed);
                    Ok(())
                }),
            },
            ChainStage::Device {
                name: "must_not_execute",
                execute: Box::new(|| {
                    second_executed.store(true, Ordering::Relaxed);
                    Ok(())
                }),
            },
        ]);
        assert_eq!(result.unwrap_err(), "operation cancelled");
        assert!(!second_executed.load(Ordering::Relaxed));
        assert!(backend.pipe.run_chain(Vec::new()).is_ok());
        assert!(
            observed
                .with_cancellation(None)
                .run_chain(Vec::new())
                .is_ok()
        );
    }

    // The adapter owns the device-serialization guard for its whole lifetime:
    // the software Vulkan driver (llvmpipe) is not safe under concurrent device
    // use in one process.
    fn backend() -> Option<crate::test_support::DeviceBackend> {
        Some(crate::test_support::adapter())
    }

    const COLORSPACE_SHADER: &str =
        include_str!("../../../spektrafilm-shaders/wgsl/colorspace_convert.wgsl");

    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct ColorspaceParams {
        width: u32,
        height: u32,
        m00: f32,
        m01: f32,
        m02: f32,
        m10: f32,
        m11: f32,
        m12: f32,
        m20: f32,
        m21: f32,
        m22: f32,
    }

    fn identity_params() -> ColorspaceParams {
        ColorspaceParams {
            width: 4,
            height: 4,
            m00: 1.0,
            m01: 0.0,
            m02: 0.0,
            m10: 0.0,
            m11: 1.0,
            m12: 0.0,
            m20: 0.0,
            m21: 0.0,
            m22: 1.0,
        }
    }

    fn sample_image() -> Vec<f32> {
        (0..48).map(|i| i as f32 * 0.03 - 0.5).collect()
    }

    #[test]
    fn dispatch_device_matches_dispatch_host() {
        let Some(b) = backend() else {
            return;
        };
        let img = sample_image();
        let params = identity_params();
        let host = b
            .pipe
            .dispatch_host(
                COLORSPACE_SHADER,
                &[
                    GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                    GpuBuffer::storage_ro(bytemuck::cast_slice(&img)),
                    GpuBuffer::storage_rw(&vec![0u8; img.len() * 4]),
                ],
                16,
                2,
                "pipe_test_host",
            )
            .expect("test bindings fit the device");
        let dev = b.pipe.upload_image(&img, 16);
        let out = b
            .pipe
            .dispatch_device(
                COLORSPACE_SHADER,
                &dev,
                &[GpuBuffer::uniform(bytemuck::bytes_of(&params))],
                16,
                "pipe_test_device",
            )
            .expect("test bindings fit the device");
        assert_eq!(out.n_pixels, 16);
        assert_eq!(out.bytes, (img.len() * 4) as u64);
        let device_result = b.pipe.readback_image(&out);
        assert_eq!(host, device_result);
        // The identity matrix must be transparent.
        assert_eq!(device_result, img);
    }

    #[test]
    fn fence_roundtrip_preserves_samples() {
        let Some(b) = backend() else {
            return;
        };
        let img = sample_image();
        let dev = b.pipe.upload_image(&img, 16);
        let fenced = b
            .pipe
            .fence(&dev, |host| host.iter().map(|v| v + 1.0).collect());
        let out = b.pipe.readback_image(&fenced);
        let expected: Vec<f32> = img.iter().map(|v| v + 1.0).collect();
        assert_eq!(out, expected);
        assert_eq!(fenced.n_pixels, 16);
    }

    #[test]
    fn readback_pool_reuses_buffers() {
        let Some(b) = backend() else {
            return;
        };
        let img = sample_image();
        let dev = b.pipe.upload_image(&img, 16);
        let first = b.pipe.readback_image(&dev);
        // Same-size readback again: the pooled buffer must be reusable and
        // still correct.
        let second = b.pipe.readback_image(&dev);
        assert_eq!(first, img);
        assert_eq!(second, img);
        assert_eq!(b.pipe.pool.lock().buffers.len(), 1);
    }
}
