//! GPU pipe execution core — the internal carrier defined by
//! `specs/gpu-execution/pipe-architecture.md` (slice S1).
//!
//! The pipe owns buffer reuse and the host transfer points: input upload,
//! fences, and the final readback. Per-stage trait methods dispatch through
//! `dispatch_host` with unchanged stage placement and telemetry semantics.
//! The assembled chain runs through `ChainStage` with `readback_buffer` and
//! `write_image_buffer` at each fence. The device-resident primitives
//! (`DeviceImage`, `dispatch_device`, `upload_image`, `fence`) are the
//! general execution core, exercised by this module's own tests.

use super::cache::PipelineCache;
use super::observation::{ObservedDevice, ObservedEncoder, ObservedQueue};
use super::{GpuBuffer, dispatch_linear};
use crate::telemetry::*;
use parking_lot::Mutex;
/// A device-resident RGB f32 image: storage buffer, pixel count, byte size.
#[allow(dead_code)]
pub(super) struct DeviceImage {
    pub(super) buf: wgpu::Buffer,
    pub(super) n_pixels: u32,
    pub(super) bytes: u64,
}

/// Pool of reusable MAP_READ readback buffers keyed by byte size. A buffer
/// returns to the pool only after `read_mapped_f32` unmapped it; that path
/// already polled the device, so a pooled buffer is immediately reusable.
#[derive(Default)]
struct ReadbackPool {
    buffers: Vec<(u64, wgpu::Buffer)>,
}

/// Cap pooled readback buffers so pathological image sizes cannot
/// accumulate device memory across frames.
const READBACK_POOL_MAX_BUFFERS: usize = 8;

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
            || entries
                .iter()
                .map(|entry| entry.contents.len() as u64)
                .sum::<u64>()
                > STATIC_CACHE_MAX_BYTES
        {
            entries.pop();
        }
        buffer
    }
}

pub(super) struct GpuPipe {
    device: ObservedDevice,
    queue: ObservedQueue,
    cache: std::sync::Arc<PipelineCache>,
    pool: std::sync::Arc<Mutex<ReadbackPool>>,
    static_cache: std::sync::Arc<StaticBufferCache>,
}

impl GpuPipe {
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
        }
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
    ) -> Vec<f32> {
        let binding_types: Vec<wgpu::BufferBindingType> =
            bindings.iter().map(|b| b.binding_type).collect();
        let (cached, hit) =
            self.cache
                .get_or_compile(&self.device.raw, shader_source, &binding_types);
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
        self.queue.submit(Some(encoder.finish()));

        let out = self.read_mapped_f32(&readback, output_size);
        self.return_readback(readback, output_size);
        out
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
    ) -> DeviceImage {
        let mut binding_types: Vec<wgpu::BufferBindingType> =
            resources.iter().map(|b| b.binding_type).collect();
        binding_types.push(wgpu::BufferBindingType::Storage { read_only: true });
        binding_types.push(wgpu::BufferBindingType::Storage { read_only: false });
        let (cached, _hit) =
            self.cache
                .get_or_compile(&self.device.raw, shader_source, &binding_types);

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
        self.queue.submit(Some(encoder.finish()));
        DeviceImage {
            buf: out,
            n_pixels,
            bytes: input.bytes,
        }
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

    /// Final readback of a device image. One of the three pipe transfer
    /// points.
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
    /// The one readback protocol: map `buffer`, wait for the GPU, record the
    /// materialized bytes, then copy the mapped range out as f32.
    pub(super) fn read_mapped_f32(&self, buffer: &wgpu::Buffer, bytes: u64) -> Vec<f32> {
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        self.device.materialized(bytes);
        let data = slice.get_mapped_range();
        let out: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
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
        if pool.buffers.len() < READBACK_POOL_MAX_BUFFERS {
            pool.buffers.push((size, buffer));
        }
    }
}

type CpuFenceFn<'a> = Box<dyn FnOnce(&GpuPipe) -> Result<(), String> + 'a>;
/// One node of an assembled compute chain (pipe-architecture slices S2/S3).
/// A `Gpu` node encodes passes into the current command buffer without
/// host transfer. A `Cpu` node is an assembled fence: the executor submits
/// pending passes, the closure materializes a device image, runs the CPU
/// stage, and writes the result back to the device before the next nodes
/// encode against it.
pub(super) enum ChainStage<'a> {
    Gpu {
        /// Diagnostic stage identity for the assembled stage list. The
        /// chain reports one batch instead of per-stage host durations, so
        /// nothing reads the name yet (telemetry metrics: a fused chain
        /// reports its batch).
        #[allow(dead_code)]
        name: &'static str,
        encode: Box<dyn FnOnce(&mut ObservedEncoder) + 'a>,
    },
    Cpu {
        /// Diagnostic stage identity, unread for the same reason as the
        /// `Gpu` arm.
        #[allow(dead_code)]
        name: &'static str,
        fence: CpuFenceFn<'a>,
    },
}

impl GpuPipe {
    /// Execute an assembled stage list. Consecutive `Gpu` nodes share one
    /// command buffer and one submission, exactly like the former fused
    /// chain; a `Cpu` fence node flushes the submission, runs the closure,
    /// and continues with a fresh command buffer. A fence that cannot
    /// calibrate its output returns the error) before anything else is
    /// submitted or published.
    pub(super) fn run_chain(&self, stages: Vec<ChainStage<'_>>) -> Result<(), String> {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for stage in stages {
            match stage {
                ChainStage::Gpu { encode, .. } => encode(&mut encoder),
                ChainStage::Cpu { fence, .. } => {
                    self.queue.submit(Some(encoder.finish()));
                    fence(self)?;
                    encoder = self.device.create_command_encoder(&Default::default());
                }
            }
        }
        self.queue.submit(Some(encoder.finish()));
        Ok(())
    }

    /// Materialize a device image buffer through the pooled readback path.
    /// Records one readback of `bytes`.
    pub(super) fn readback_buffer(&self, buf: &wgpu::Buffer, bytes: u64) -> Vec<f32> {
        let readback = self.take_readback(bytes);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buf, 0, &readback, 0, bytes);
        self.queue.submit(Some(encoder.finish()));
        let out = self.read_mapped_f32(&readback, bytes);
        self.return_readback(readback, bytes);
        out
    }

    /// Overwrite a device image buffer with host samples. Records one
    /// image upload of the written bytes.
    pub(super) fn write_image_buffer(&self, buf: &wgpu::Buffer, data: &[f32]) {
        self.queue.write_buffer(buf, 0, bytemuck::cast_slice(data));
    }
}

#[cfg(all(test, feature = "wgpu-backend"))]
mod tests {
    use super::super::observation::TEST_DEVICE_LOCK as DEVICE_IN_USE;
    use super::*;

    // The software Vulkan driver (llvmpipe) is not safe under concurrent
    // device use in one process; every test holds the shared serial lock
    // for its whole body.
    fn backend() -> Option<(
        std::sync::MutexGuard<'static, ()>,
        crate::wgpu_backend::WgpuBackend,
    )> {
        let guard = DEVICE_IN_USE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Some((guard, crate::test_support::adapter()))
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
        let Some((_guard, b)) = backend() else {
            return;
        };
        let img = sample_image();
        let params = identity_params();
        let host = b.pipe.dispatch_host(
            COLORSPACE_SHADER,
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&img)),
                GpuBuffer::storage_rw(&vec![0u8; img.len() * 4]),
            ],
            16,
            2,
            "pipe_test_host",
        );
        let dev = b.pipe.upload_image(&img, 16);
        let out = b.pipe.dispatch_device(
            COLORSPACE_SHADER,
            &dev,
            &[GpuBuffer::uniform(bytemuck::bytes_of(&params))],
            16,
            "pipe_test_device",
        );
        assert_eq!(out.n_pixels, 16);
        assert_eq!(out.bytes, (img.len() * 4) as u64);
        let device_result = b.pipe.readback_image(&out);
        assert_eq!(host, device_result);
        // The identity matrix must be transparent.
        assert_eq!(device_result, img);
    }

    #[test]
    fn fence_roundtrip_preserves_samples() {
        let Some((_guard, b)) = backend() else {
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
        let Some((_guard, b)) = backend() else {
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
