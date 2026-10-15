//! wgpu compute backend — dispatches WGSL shaders on GPU via Metal/Vulkan/DX12.

mod observation;
use crate::telemetry::*;
use observation::{ObservedDevice, ObservedEncoder, ObservedPass, ObservedQueue};

#[cfg(feature = "wgpu-backend")]
use spektrafilm_math::image::ImageBuf;

use crate::gpu_helpers::{
    f32_to_scalars, is_uniform_grid_endpoint, sanitize_spectral_inputs, scalars_to_f32,
};
#[cfg(feature = "wgpu-backend")]
use crate::{ComputeBackend, Lut3D, cpu_backend};

#[cfg(feature = "wgpu-backend")]
mod cache;
#[cfg(feature = "wgpu-backend")]
mod film_chain;
#[cfg(feature = "wgpu-backend")]
<<<<<<< HEAD
use cache::{CachedPipelineRef, CacheHit, PipelineCache};
=======
use cache::{CachedPipelineRef, PipelineCache};
#[cfg(feature = "wgpu-backend")]
mod pipe;
#[cfg(feature = "wgpu-backend")]
use pipe::GpuPipe;
#[cfg(feature = "wgpu-backend")]
mod params;
#[cfg(feature = "wgpu-backend")]
use params::{DensityParams, FrontParams, ScanParams};
>>>>>>> origin/main-0.3.4

/// FIR Gaussian-blur half-width `ceil(3σ)`, hard-capped so a pathological σ
/// can never build a multi-thousand-tap kernel that hangs the GPU (a
/// monster kernel froze the display once). 256 → at
/// most a 513-tap separable kernel; well above any legitimate σ here
/// (halation tops out at tens of pixels).
const MAX_BLUR_RADIUS: u32 = 256;

// Keep in sync with the linear WGSL kernels. 256 is supported by the
// default device limits; a two-dimensional grid preserves large-image support.
#[cfg(feature = "wgpu-backend")]
const LINEAR_WORKGROUP_SIZE: u32 = 256;

#[cfg(feature = "wgpu-backend")]
fn dispatch_grid(workgroups: u32) -> (u32, u32) {
    let x = workgroups.clamp(1, 65535);
    (x, workgroups.div_ceil(x))
}

#[cfg(feature = "wgpu-backend")]
fn dispatch_linear(pass: &mut ObservedPass<'_>, n_values: u32) {
    let (x, y) = dispatch_grid(n_values.div_ceil(LINEAR_WORKGROUP_SIZE));
    pass.dispatch_workgroups(x, y, 1);
}

#[inline]
fn fir_blur_radius(sigma: f32) -> u32 {
    ((3.0_f32 * sigma).ceil() as u32).min(MAX_BLUR_RADIUS)
}

#[cfg(feature = "wgpu-backend")]
pub struct WgpuBackend {
    device: ObservedDevice,
    queue: ObservedQueue,
    pipeline_cache: std::sync::Arc<PipelineCache>,
    pipe: GpuPipe,
    name: String,
    adapter: AdapterDescription,
    explicit: bool,
}

#[cfg(feature = "wgpu-backend")]
impl WgpuBackend {
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        };
        let mut adapter = pollster::block_on(instance.request_adapter(&options))?;
        let policy = |adapter: &wgpu::Adapter, timestamp: bool| {
            let info = adapter.get_info();
            let hardware = adapter.limits();
            let mut limits = wgpu::Limits::default();
            limits.max_storage_buffer_binding_size = hardware.max_storage_buffer_binding_size;
            limits.max_buffer_size = hardware.max_buffer_size;
            limits.max_compute_workgroups_per_dimension =
                hardware.max_compute_workgroups_per_dimension;
            limits.max_bind_groups = hardware.max_bind_groups.max(limits.max_bind_groups);
            let mut features = if info.device_type == wgpu::DeviceType::IntegratedGpu {
                wgpu::Features::MAPPABLE_PRIMARY_BUFFERS & adapter.features()
            } else {
                wgpu::Features::empty()
            };
            if timestamp {
                features |= adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
            }
            wgpu::DeviceDescriptor {
                label: Some("spektrafilm"),
                required_features: features,
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
            }
        };
        let timestamp_requested = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let first = pollster::block_on(adapter.request_device(&policy(&adapter, true), None));
        let mut feature_request_failed = false;
        let (device, queue) = match first {
            Ok(value) => value,
            Err(_) if timestamp_requested => {
                feature_request_failed = true;
                adapter = pollster::block_on(instance.request_adapter(&options))?;
                match pollster::block_on(adapter.request_device(&policy(&adapter, false), None)) {
                    Ok(v) => v,
                    Err(_) => return None,
                }
            }
            Err(_) => return None,
        };
        let adapter_info = adapter.get_info();
        let timestamp_supported = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let timestamp_enabled = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let description = AdapterDescription {
            api: format!("{:?}", adapter_info.backend),
            device_type: match adapter_info.device_type {
                wgpu::DeviceType::Cpu => AdapterDeviceType::Cpu,
                wgpu::DeviceType::IntegratedGpu => AdapterDeviceType::Integrated,
                wgpu::DeviceType::DiscreteGpu => AdapterDeviceType::Discrete,
                wgpu::DeviceType::VirtualGpu => AdapterDeviceType::Virtual,
                _ => AdapterDeviceType::Other,
            },
            name: adapter_info.name.chars().take(256).collect(),
            description_truncated: adapter_info.name.chars().count() > 256,
            shared_device: true,
            timestamp_supported,
            timestamp_enabled,
            timestamp_period_ns: if timestamp_enabled {
                Measurement::available(queue.get_timestamp_period() as f64)
            } else {
                Measurement::unavailable(if feature_request_failed {
                    AvailabilityReason::FeatureRequestFailed
                } else {
                    AvailabilityReason::Unsupported
                })
            },
            timestamp_reason: if timestamp_enabled {
                None
            } else {
                Some(if feature_request_failed {
                    AvailabilityReason::FeatureRequestFailed
                } else {
                    AvailabilityReason::Unsupported
                })
            },
        };

        tracing::info!(
            adapter = adapter_info.name,
            backend = ?adapter_info.backend,
            "wgpu backend initialized"
        );

        let device = ObservedDevice {
            raw: device,
            context: ObservationContext::default(),
            batch: None,
        };
        let queue = ObservedQueue {
            raw: queue,
            context: ObservationContext::default(),
        };
        let pipeline_cache = std::sync::Arc::new(PipelineCache::default());
        Some(Self {
            name: format!(
                "WGPU f32 · {} ({:?}, {:?})",
                adapter_info.name, adapter_info.device_type, adapter_info.backend
            ),
            pipe: GpuPipe::new(
                device.clone(),
                queue.clone(),
                std::sync::Arc::clone(&pipeline_cache),
            ),
            device,
            queue,
            pipeline_cache,
            adapter: description,
            explicit: false,
        })
    }

    /// Construct an explicitly requested backend
    /// (`SPEKTRAFILM_BACKEND=wgpu`, the CLI `--backend gpu` flag, or the GUI
    /// export "GPU" selection): capability-missing stages must fail at
    /// assembly instead of silently demoting the route to CPU.
    pub fn new_explicit() -> Option<Self> {
        Self::new().map(|mut b| {
            b.explicit = true;
            b
        })
    }

    /// Derive an explicitly requested backend sharing this backend's device,
    /// queue, pipeline cache, and readback pool. A frontend that already
    /// holds a selected GPU backend (the GUI preview backend) uses this for
    /// an explicit export request instead of opening a second device and
    /// recompiling every pipeline.
    pub fn derived_explicit(&self) -> Self {
        let device = ObservedDevice {
            raw: self.device.raw.clone(),
            context: self.device.context.clone(),
            batch: None,
        };
        let queue = ObservedQueue {
            raw: self.queue.raw.clone(),
            context: self.queue.context.clone(),
        };
        Self {
            pipe: self.pipe.with_observed(device.clone(), queue.clone()),
            device,
            queue,
            pipeline_cache: std::sync::Arc::clone(&self.pipeline_cache),
            name: self.name.clone(),
            adapter: self.adapter.clone(),
            explicit: true,
        }
    }

    /// Generic GPU compute dispatch helper.
    ///
    /// The cache key includes both shader identity and binding layout so a
    /// shader reused by two passes cannot receive the wrong bind-group
    /// layout. Execution, buffer reuse, and the readback live on the
    /// [pipe](pipe) core.
    fn dispatch_compute(
        &self,
        shader_source: &'static str,
        bindings: &[GpuBuffer],
        n_pixels: u32,
        output_idx: usize,
        name: &'static str,
    ) -> Vec<f32> {
<<<<<<< HEAD
        let binding_types: Vec<wgpu::BufferBindingType> =
            bindings.iter().map(|b| b.binding_type).collect();
        let (cached, hit) =
            self.pipeline_cache
                .pipeline_for(&self.device, shader_source, &binding_types);
        if self.device.context.enabled() {
            let mut work = WorkCounters::default();
            match hit {
                CacheHit::Hit => work.pipeline_cache_hit_count = Some(1),
                CacheHit::Miss => work.pipeline_cache_miss_count = Some(1),
            }
            self.device.context.record_work(&work);
        }
        let pipeline = &cached.pipeline;
        let bind_group_layout = &cached.layout;

        // Create GPU buffers
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

        // Readback buffer
        let output_size = bindings[output_idx].data.len() as u64;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: output_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Dispatch
        // Linear kernels flatten a two-dimensional workgroup grid in WGSL.

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

        // Read back
        self.read_mapped_f32(&readback, output_size)
=======
        self.pipe
            .dispatch_host(shader_source, bindings, n_pixels, output_idx, name)
>>>>>>> origin/main-0.3.4
    }

    /// The one readback protocol: map `buffer`, wait for the GPU, record the
    /// materialized bytes, then copy the mapped range out as f32.
    fn read_mapped_f32(&self, buffer: &wgpu::Buffer, bytes: u64) -> Vec<f32> {
        self.pipe.read_mapped_f32(buffer, bytes)
    }

    /// Get or compile a pipeline using the shared shader-and-layout cache.
    fn cached_pipeline(
        &self,
        shader_source: &'static str,
        binding_types: &[wgpu::BufferBindingType],
    ) -> CachedPipelineRef {
        let (cached, hit) =
            self.pipeline_cache
                .pipeline_for(&self.device.raw, shader_source, binding_types);
        if self.device.context.enabled() {
            let mut w = WorkCounters::default();
            match hit {
                CacheHit::Hit => w.pipeline_cache_hit_count = Some(1),
                CacheHit::Miss => w.pipeline_cache_miss_count = Some(1),
            }
            self.device.context.record_work(&w);
        }
        cached
    }
}

#[cfg(feature = "wgpu-backend")]
struct GpuBuffer {
    data: Vec<u8>,
    binding_type: wgpu::BufferBindingType,
    usage: wgpu::BufferUsages,
}

#[cfg(feature = "wgpu-backend")]
impl GpuBuffer {
    fn uniform(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            binding_type: wgpu::BufferBindingType::Uniform,
            usage: wgpu::BufferUsages::UNIFORM,
        }
    }
    fn storage_ro(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            binding_type: wgpu::BufferBindingType::Storage { read_only: true },
            usage: wgpu::BufferUsages::STORAGE,
        }
    }
    fn storage_rw(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            binding_type: wgpu::BufferBindingType::Storage { read_only: false },
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        }
    }
}

#[cfg(feature = "wgpu-backend")]
#[cfg(feature = "wgpu-backend")]
mod compute_backend;

#[cfg(feature = "wgpu-backend")]
mod blur;
#[cfg(feature = "wgpu-backend")]
mod couplers;
#[cfg(feature = "wgpu-backend")]
mod gamut;
#[cfg(feature = "wgpu-backend")]
mod glare;
#[cfg(feature = "wgpu-backend")]
mod grain;
#[cfg(feature = "wgpu-backend")]
mod halation;
#[cfg(feature = "wgpu-backend")]
mod highlight;
#[cfg(feature = "wgpu-backend")]
mod unsharp;
#[cfg(feature = "wgpu-backend")]
use blur::*;
#[cfg(feature = "wgpu-backend")]
use couplers::*;
#[cfg(feature = "wgpu-backend")]
use gamut::*;
#[cfg(feature = "wgpu-backend")]
use glare::*;
#[cfg(feature = "wgpu-backend")]
use grain::*;
#[cfg(feature = "wgpu-backend")]
use halation::*;
#[cfg(feature = "wgpu-backend")]
use highlight::*;
#[cfg(feature = "wgpu-backend")]
use unsharp::*;
