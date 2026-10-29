//! wgpu compute backend — dispatches WGSL shaders on GPU via Metal/Vulkan/DX12.

mod observation;
#[cfg(feature = "wgpu-backend")]
mod page_views;
mod profiling;
mod resources;
mod timestamps;
use observation::{ObservedDevice, ObservedEncoder, ObservedPass, ObservedQueue};
use spektrafilm_backend::telemetry::*;

#[cfg(feature = "wgpu-backend")]
use spektrafilm_math::image::ImageBuf;

use crate::gpu_helpers::{
    f32_to_scalars, is_uniform_grid_endpoint, nan_safe_rows_to_f32, nan_safe_slice_to_f32,
    sanitize_spectral_inputs, scalars_to_f32,
};
#[cfg(all(test, feature = "wgpu-backend"))]
use spektrafilm_backend::cpu_backend;
use spektrafilm_backend::{ComputeBackend, Lut3D};

#[cfg(feature = "wgpu-backend")]
mod cache;
#[cfg(feature = "wgpu-backend")]
mod film_chain;
#[cfg(feature = "wgpu-backend")]
mod paged;
#[cfg(feature = "wgpu-backend")]
use cache::{CachedPipelineRef, PipelineCache};
#[cfg(feature = "wgpu-backend")]
mod params;
#[cfg(feature = "wgpu-backend")]
mod pipe;
#[cfg(feature = "wgpu-backend")]
use params::{CctfParams, DensityParams, FrontParams, ScanParams};
#[cfg(feature = "wgpu-backend")]
use pipe::GpuPipe;

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

#[cfg(feature = "wgpu-backend")]
#[inline]
pub(crate) fn pack_matrix_columns(matrix: &[[f32; 3]; 3]) -> [[f32; 4]; 3] {
    [
        [matrix[0][0], matrix[1][0], matrix[2][0], 0.0],
        [matrix[0][1], matrix[1][1], matrix[2][1], 0.0],
        [matrix[0][2], matrix[1][2], matrix[2][2], 0.0],
    ]
}

#[inline]
fn pack_matrix_columns_f64(matrix: &[[f64; 3]; 3]) -> [[f32; 4]; 3] {
    [
        [
            matrix[0][0] as f32,
            matrix[1][0] as f32,
            matrix[2][0] as f32,
            0.0,
        ],
        [
            matrix[0][1] as f32,
            matrix[1][1] as f32,
            matrix[2][1] as f32,
            0.0,
        ],
        [
            matrix[0][2] as f32,
            matrix[1][2] as f32,
            matrix[2][2] as f32,
            0.0,
        ],
    ]
}

#[cfg(feature = "wgpu-backend")]
pub struct WgpuBackend {
    device: ObservedDevice,
    queue: ObservedQueue,
    pipeline_cache: std::sync::Arc<PipelineCache>,
    /// One immutable acquisition shared by observation/cancellation derivatives.
    scan_cache: std::sync::Arc<scan_cache::ScanCache>,
    pipe: GpuPipe,
    name: String,
    adapter: AdapterDescription,
    explicit: bool,
    /// Identity of the device this backend owns, shared with every backend
    /// derived from it. Retained carriers record it so a consumer refuses a
    /// carrier from a different device instead of reading foreign memory.
    backend_id: u64,
    cancellation: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

#[cfg(feature = "wgpu-backend")]
impl WgpuBackend {
    pub fn new() -> Option<Self> {
        #[cfg(test)]
        {
            crate::test_support::assert_device_allowed();
        }
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
            // Spektra grain conditions its Poisson/binomial decisions on
            // f64 values the reference reads from JSON at full precision
            // (`specs/film-grain/spektra/design.md`: f32 conversion before those
            // decisions is not allowed), and its per-element `fast_stats`
            // streams are splitmix64. The device therefore asks for native
            // f64 and 64-bit integers; an adapter without them simply does
            // not report the features, and the Spektra pass declines to the
            // host fence rather than approximating (see `grain::spektra`).
            features |=
                adapter.features() & (wgpu::Features::SHADER_F64 | wgpu::Features::SHADER_INT64);
            // Acceptance hook: simulate an adapter without 64-bit shader
            // arithmetic. The default flow must run on such hardware, so this
            // strips both features and lets the real CLI/GUI expose every
            // remaining device-precision dependency.
            if std::env::var_os("SPEKTRAFILM_GPU_DISABLE_F64").is_some() {
                features &= !(wgpu::Features::SHADER_F64 | wgpu::Features::SHADER_INT64);
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

        let resources = std::sync::Arc::new(resources::DeviceResources::new(
            device.clone(),
            adapter_info.backend == wgpu::Backend::Vulkan,
        ));
        let device = ObservedDevice {
            raw: device,
            resources: resources.clone(),
            operation: None,
            context: ObservationContext::default(),
            batch: None,
        };
        let queue = ObservedQueue {
            raw: queue,
            resources,
            operation: None,
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
            scan_cache: std::sync::Arc::new(scan_cache::ScanCache::default()),
            adapter: description,
            explicit: false,
            backend_id: film_chain::device_image::next_backend_id(),
            cancellation: None,
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
            resources: self.device.resources.clone(),
            operation: self.device.operation.clone(),
            context: self.device.context.clone(),
            batch: None,
        };
        let queue = ObservedQueue {
            raw: self.queue.raw.clone(),
            resources: self.queue.resources.clone(),
            operation: self.queue.operation.clone(),
            context: self.queue.context.clone(),
        };
        Self {
            pipe: self.pipe.with_observed(device.clone(), queue.clone()),
            device,
            queue,
            pipeline_cache: std::sync::Arc::clone(&self.pipeline_cache),
            scan_cache: std::sync::Arc::clone(&self.scan_cache),
            name: self.name.clone(),
            adapter: self.adapter.clone(),
            explicit: true,
            backend_id: self.backend_id,
            cancellation: self.cancellation.clone(),
        }
    }

    pub(in crate::wgpu_backend) fn check_cancelled(&self) -> Result<(), String> {
        self.pipe.check_cancelled()
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
    ) -> Option<Vec<f32>> {
        self.pipe
            .dispatch_host(shader_source, bindings, n_pixels, output_idx, name)
    }

    /// The one readback protocol: map `buffer`, wait for the GPU, record the
    /// materialized bytes, then copy the mapped range out as f32.
    fn read_mapped_f32(&self, buffer: &wgpu::Buffer, bytes: u64) -> Vec<f32> {
        self.pipe.read_mapped(buffer, bytes)
    }

    fn read_mapped_u32(&self, buffer: &wgpu::Buffer, bytes: u64) -> Vec<u32> {
        self.pipe.read_mapped(buffer, bytes)
    }

    /// Get or compile a pipeline using the shared shader-and-layout cache.
    fn cached_pipeline(
        &self,
        shader_source: &'static str,
        binding_types: &[wgpu::BufferBindingType],
    ) -> CachedPipelineRef {
        self.cached_pipeline_at(shader_source, binding_types, "main")
    }

    /// Get or compile the pipeline of one named entry point. A module may
    /// declare several entry points — the Fotufilm modules do — and each is
    /// its own pipeline over its own binding layout.
    fn cached_pipeline_at(
        &self,
        shader_source: &'static str,
        binding_types: &[wgpu::BufferBindingType],
        entry_point: &'static str,
    ) -> CachedPipelineRef {
        let (cached, hit) = self.pipeline_cache.get_or_compile_at(
            &self.device,
            shader_source,
            binding_types,
            entry_point,
        );
        self.record_pipeline_cache_lookup(hit);
        cached
    }

    fn record_pipeline_cache_lookup(&self, hit: bool) {
        if self.device.context.enabled() {
            let mut w = WorkCounters::default();
            if hit {
                w.pipeline_cache_hit_count = Some(1)
            } else {
                w.pipeline_cache_miss_count = Some(1)
            }
            self.device.context.record_work(&w);
        }
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
mod analysis;
#[cfg(feature = "wgpu-backend")]
mod compute_backend;
#[cfg(test)]
mod failure_tests;
#[cfg(feature = "wgpu-backend")]
mod viewer;
#[cfg(feature = "wgpu-backend")]
mod viewer_icc;
#[cfg(feature = "wgpu-backend")]
mod viewer_paper;
#[cfg(feature = "wgpu-backend")]
mod viewer_polaroid;

#[cfg(feature = "wgpu-backend")]
mod bounded;
#[cfg(feature = "wgpu-backend")]
mod chain_chunks;
#[cfg(feature = "wgpu-backend")]
mod chain_finish;
#[cfg(feature = "wgpu-backend")]
mod enlarger_resident;
#[cfg(feature = "wgpu-backend")]
mod input_gamut_prepare;
#[cfg(feature = "wgpu-backend")]
mod lut_chunks;
#[cfg(feature = "wgpu-backend")]
mod pchip_chunks;
#[cfg(feature = "wgpu-backend")]
mod pointwise_pages;
#[cfg(feature = "wgpu-backend")]
mod preparation;

#[cfg(feature = "wgpu-backend")]
mod blur;
#[cfg(feature = "wgpu-backend")]
mod couplers;
#[cfg(feature = "wgpu-backend")]
mod gamut;
#[cfg(feature = "wgpu-backend")]
mod glare;
#[cfg(feature = "wgpu-backend")]
pub mod grain;

pub use spektrafilm_backend::dye_params::{DyeGpuParams, DyeReadoutGrid};
#[cfg(feature = "wgpu-backend")]
mod halation;
#[cfg(feature = "wgpu-backend")]
mod highlight;
#[cfg(feature = "wgpu-backend")]
mod input;
#[cfg(feature = "wgpu-backend")]
mod metering;
#[cfg(feature = "wgpu-backend")]
mod optics;
#[cfg(feature = "wgpu-backend")]
mod raw;
#[cfg(feature = "wgpu-backend")]
mod scan;
#[cfg(feature = "wgpu-backend")]
mod scan_cache;
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
use input::*;
#[cfg(feature = "wgpu-backend")]
use metering::*;
#[cfg(feature = "wgpu-backend")]
use unsharp::*;
