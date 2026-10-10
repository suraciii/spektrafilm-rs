mod observation;
use crate::telemetry::*;
use observation::{ObservedDevice, ObservedEncoder, ObservedPass, ObservedQueue};
/// wgpu compute backend — dispatches WGSL shaders on GPU via Metal/Vulkan/DX12.

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
use cache::{CachedPipelineRef, PipelineCache};

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
    name: String,
    adapter: AdapterDescription,
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

        Some(Self {
            name: format!(
                "WGPU f32 · {} ({:?}, {:?})",
                adapter_info.name, adapter_info.device_type, adapter_info.backend
            ),
            device: ObservedDevice {
                raw: device,
                context: ObservationContext::default(),
                batch: None,
            },
            queue: ObservedQueue {
                raw: queue,
                context: ObservationContext::default(),
                batch: None,
            },
            pipeline_cache: std::sync::Arc::new(PipelineCache::default()),
            adapter: description,
        })
    }

    /// Generic GPU compute dispatch helper.
    ///
    /// The cache key includes both shader identity and binding layout so a
    /// shader reused by two passes cannot receive the wrong bind-group layout.
    fn dispatch_compute(
        &self,
        shader_source: &'static str,
        bindings: &[GpuBuffer],
        n_pixels: u32,
        output_idx: usize,
        name: &'static str,
    ) -> Vec<f32> {
        if !self.device.context.enabled() {
            return self.dispatch_compute_inner(
                shader_source,
                bindings,
                n_pixels,
                output_idx,
                name,
            );
        }
        let (backend, _batch) = self.observed_batch("compute");
        backend.dispatch_compute_inner(shader_source, bindings, n_pixels, output_idx, name)
    }
    fn dispatch_compute_inner(
        &self,
        shader_source: &'static str,
        bindings: &[GpuBuffer],
        n_pixels: u32,
        output_idx: usize,
        name: &'static str,
    ) -> Vec<f32> {
        let t_start = std::time::Instant::now();
        let binding_types: Vec<wgpu::BufferBindingType> =
            bindings.iter().map(|b| b.binding_type).collect();
        let (cached, hit) =
            self.pipeline_cache
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
        let t_compile = t_start.elapsed();

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
            layout: &bind_group_layout,
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
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            dispatch_linear(&mut pass, n_pixels);
        }
        encoder.copy_buffer_to_buffer(&gpu_buffers[output_idx], 0, &readback, 0, output_size);
        self.queue.submit(Some(encoder.finish()));

        // Read back
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();
        self.device.materialized(output_size);
        let data = slice.get_mapped_range();
        let result: Vec<f32> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        readback.unmap();
        let _t_total = t_start.elapsed();
        let _ = t_compile;
        result
    }

    /// Get or compile a pipeline using the shared shader-and-layout cache.
    fn cached_pipeline(
        &self,
        shader_source: &'static str,
        binding_types: &[wgpu::BufferBindingType],
    ) -> CachedPipelineRef {
        let (cached, hit) =
            self.pipeline_cache
                .get_or_compile(&self.device.raw, shader_source, binding_types);
        if self.device.context.enabled() {
            let mut w = WorkCounters::default();
            if hit {
                w.pipeline_cache_hit_count = Some(1)
            } else {
                w.pipeline_cache_miss_count = Some(1)
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
impl ComputeBackend for WgpuBackend {
    fn observation_context(&self) -> Option<&ObservationContext> {
        Some(&self.device.context)
    }
    fn adapter_description(&self) -> Option<AdapterDescription> {
        Some(self.adapter.clone())
    }
    fn with_observation_context(
        &self,
        context: ObservationContext,
    ) -> Option<Box<dyn ComputeBackend + '_>> {
        context.set_backend_selected(BackendSelected::Wgpu);
        context.set_adapter(self.adapter.clone());
        if context.mode() == CollectionMode::GpuTiming && !self.adapter.timestamp_enabled {
            context.set_effective_mode(CollectionMode::Summary);
        }
        Some(Box::new(Self {
            device: ObservedDevice {
                raw: self.device.raw.clone(),
                context: context.clone(),
                batch: None,
            },
            queue: ObservedQueue {
                raw: self.queue.raw.clone(),
                context,
                batch: None,
            },
            pipeline_cache: self.pipeline_cache.clone(),
            name: self.name.clone(),
            adapter: self.adapter.clone(),
        }))
    }
    fn colorspace_convert(&self, img: &ImageBuf, matrix: &[[f32; 3]; 3]) -> ImageBuf {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "colorspace_convert",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        if let Some(s) = &scope {
            s.context()
                .record_executor(Executor::Cpu, Some(CpuReason::BackendDefault));
        }
        cpu_backend::CpuBackend.colorspace_convert(img, matrix)
    }
    fn cctf_encode_srgb(&self, img: &ImageBuf) -> ImageBuf {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "output_transfer",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        if let Some(s) = &scope {
            s.context()
                .record_executor(Executor::Cpu, Some(CpuReason::BackendDefault));
        }
        cpu_backend::CpuBackend.cctf_encode_srgb(img)
    }
    fn cctf_decode_srgb(&self, img: &ImageBuf) -> ImageBuf {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "input_transfer",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        if let Some(s) = &scope {
            s.context()
                .record_executor(Executor::Cpu, Some(CpuReason::BackendDefault));
        }
        cpu_backend::CpuBackend.cctf_decode_srgb(img)
    }
    fn grain_v2(&self, img: &ImageBuf, params: &crate::GrainV2GpuParams) -> Option<ImageBuf> {
        Some(self.grain_v2_gpu(img, params))
    }
    fn gaussian_blur(&self, img: &ImageBuf, sigma: f32) -> ImageBuf {
        if sigma <= 0.0 {
            return img.clone();
        }
        if !crate::gpu_blur_supported(sigma) {
            tracing::info!(
                sigma,
                execution = "cpu",
                "Gaussian blur exceeds GPU FIR support"
            );
            self.device.context.record_executor(
                Executor::Cpu,
                Some(CpuReason::BlurRadiusExceedsBackendSupport),
            );
            return cpu_backend::CpuBackend.gaussian_blur(img, sigma);
        }
        // For very small sigmas the FIR overhead dominates; CPU path is fine.
        // For practical halation/glare sigmas (1-40 pixels) the GPU is much faster.
        self.gaussian_blur_gpu(img, sigma)
    }
    fn gaussian_blur_multi(&self, img: &ImageBuf, sigmas: &[f32]) -> Vec<ImageBuf> {
        if sigmas.is_empty() {
            return Vec::new();
        }
        if sigmas
            .iter()
            .any(|&sigma| !crate::gpu_blur_supported(sigma))
        {
            tracing::info!(
                execution = "cpu",
                "Gaussian blur batch exceeds GPU FIR support"
            );
            self.device.context.record_executor(
                Executor::Cpu,
                Some(CpuReason::BlurRadiusExceedsBackendSupport),
            );
            return cpu_backend::CpuBackend.gaussian_blur_multi(img, sigmas);
        }
        self.gaussian_blur_multi_gpu(img, sigmas)
    }
    fn table_lookup(&self, img: &ImageBuf, table_x: &[f32], table_y: &[[f32; 3]]) -> ImageBuf {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "table_lookup",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        if let Some(s) = &scope {
            s.context()
                .record_executor(Executor::Cpu, Some(CpuReason::BackendDefault));
        }
        cpu_backend::CpuBackend.table_lookup(img, table_x, table_y)
    }
    fn lut3d_interp(&self, img: &ImageBuf, lut: &Lut3D) -> ImageBuf {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "spectral_lut",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        if let Some(s) = &scope {
            s.context()
                .record_executor(Executor::Cpu, Some(CpuReason::SpectralLut));
        }
        cpu_backend::CpuBackend.lut3d_interp(img, lut)
    }

    fn scan_spectral(
        &self,
        density_cmy: &ImageBuf,
        channel_density: &[[f64; 3]],
        base_density: &[f64],
        illuminant: &[f64],
        normalization: f64,
        cat: &[[f64; 3]; 3],
        xyz_to_rgb: &[[f64; 3]; 3],
    ) -> ImageBuf {
        // GPU live-preview path collapses CAT and XYZ→RGB into a single
        // matrix — small precision drop acceptable for preview, matches
        // the same trade-off as `hanatos2025_rgb_to_raw`.
        let combined: [[f64; 3]; 3] = {
            let mut out = [[0.0f64; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    out[i][j] = xyz_to_rgb[i][0] * cat[0][j]
                        + xyz_to_rgb[i][1] * cat[1][j]
                        + xyz_to_rgb[i][2] * cat[2][j];
                }
            }
            out
        };
        let xyz_to_rgb = &combined;
        let n_wl = channel_density.len();
        let n_pixels = density_cmy.pixel_count() as u32;

        // Pack uniform params — mat3x3 in WGSL std140 is 3 columns of vec4.
        // GPU shaders are f32-only: cast precision-preserved inputs at the shader boundary.
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            width: u32,
            height: u32,
            n_wavelengths: u32,
            normalization: f32,
            col0: [f32; 4],
            col1: [f32; 4],
            col2: [f32; 4],
            bw: [f32; 4],
        }

        let params = Params {
            width: density_cmy.width,
            height: density_cmy.height,
            n_wavelengths: n_wl as u32,
            normalization: normalization as f32,
            col0: [
                xyz_to_rgb[0][0] as f32,
                xyz_to_rgb[1][0] as f32,
                xyz_to_rgb[2][0] as f32,
                0.0,
            ],
            col1: [
                xyz_to_rgb[0][1] as f32,
                xyz_to_rgb[1][1] as f32,
                xyz_to_rgb[2][1] as f32,
                0.0,
            ],
            col2: [
                xyz_to_rgb[0][2] as f32,
                xyz_to_rgb[1][2] as f32,
                xyz_to_rgb[2][2] as f32,
                0.0,
            ],
            // Standalone scan path carries no B&W remap (the resident chain
            // is the only caller that sets it). Identity / disabled.
            // Per-stage CPU callers consume linear scan RGB; final encoding
            // and clipping happen at the end of the scanning stage.
            bw: [1.0, 0.0, 0.0, 1.0],
        };

        // GPU shaders are f32-only — narrow f64 inputs at the shader boundary.
        // Metal's compiler runs fast-math (no-NaN assumption), so we sanitize CPU-side:
        // any wavelength with a NaN input has its `base_density` bumped to +1000 so
        // `pow(10, -d) ≈ 0`, zeroing that wavelength's contribution exactly the same
        // way Python's `density_to_light` zeros NaN-bearing wavelengths.
        let (cd_flat, mut bd) = sanitize_spectral_inputs(channel_density, base_density, n_wl);
        let illu_f32: Vec<f32> = illuminant.iter().map(|&v| v as f32).collect();
        bd.resize(n_wl, 0.0);
        let input_f32 = scalars_to_f32(&density_cmy.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];

        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/spectral/scan_spectral.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&cd_flat)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&bd)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&illu_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&spektrafilm_math::spectral::CMF_X)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&spektrafilm_math::spectral::CMF_Y)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&spektrafilm_math::spectral::CMF_Z)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            8, // output buffer index
            "scan_spectral",
        );

        ImageBuf::from_data(
            density_cmy.width,
            density_cmy.height,
            f32_to_scalars(result),
        )
    }

    fn print_spectral(
        &self,
        density_cmy: &ImageBuf,
        channel_density: &[[f64; 3]],
        base_density: &[f64],
        illuminant: &[f64],
        sensitivity: &[[f64; 3]],
        normalization_factor: f64,
        preflash: [f64; 3],
    ) -> ImageBuf {
        let n_wl = channel_density.len();
        let n_pixels = density_cmy.pixel_count() as u32;

        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            width: u32,
            height: u32,
            n_wavelengths: u32,
            normalization_factor: f32,
            preflash: [f32; 3],
            _pad: f32,
        }

        let params = Params {
            width: density_cmy.width,
            height: density_cmy.height,
            n_wavelengths: n_wl as u32,
            normalization_factor: normalization_factor as f32,
            preflash: [preflash[0] as f32, preflash[1] as f32, preflash[2] as f32],
            _pad: 0.0,
        };

        // GPU shaders are f32-only — narrow f64 inputs at the shader boundary.
        // See `sanitize_spectral_inputs` for the NaN-handling story (Metal fast-math).
        let (cd_flat, mut bd) = sanitize_spectral_inputs(channel_density, base_density, n_wl);
        let sens_flat: Vec<f32> = sensitivity
            .iter()
            .flat_map(|r| r.iter().map(|&v| if v.is_nan() { 0.0 } else { v as f32 }))
            .collect();
        bd.resize(n_wl, 0.0);
        let illu_f32: Vec<f32> = illuminant.iter().map(|&v| v as f32).collect();
        let input_f32 = scalars_to_f32(&density_cmy.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];

        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/spectral/print_spectral.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&cd_flat)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&bd)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&illu_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&sens_flat)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            6, // output buffer index
            "print_spectral",
        );

        ImageBuf::from_data(
            density_cmy.width,
            density_cmy.height,
            f32_to_scalars(result),
        )
    }

    fn hanatos2025_rgb_to_raw(
        &self,
        image: &ImageBuf,
        tc_lut: &spektrafilm_math::spectral::TcLut,
        color_space: &str,
        ref_illuminant: &[f32],
        cat16: bool,
    ) -> ImageBuf {
        // GPU live-preview path collapses the two-step CAT02 adaptation
        // into a single matmul — small visible-spectrum precision drop
        // that's acceptable for preview. The CPU path keeps the two-step
        // for export bit-parity with Python.
        let rgb_to_adapted_xyz = spektrafilm_math::spectral::build_rgb_to_adapted_xyz(
            color_space,
            ref_illuminant,
            cat16,
        );
        let rgb_to_adapted_xyz = &rgb_to_adapted_xyz;
        let n_pixels = image.pixel_count() as u32;
        let lut_size = tc_lut.size as u32;

        // Pack uniform: mat3x3 in std140 = 3 vec4 columns. f32 only on GPU.
        // WGSL columns are read as columns of the matrix; storing row-major as
        // [col[0], col[1], col[2]] where col[j][i] = mat[i][j].
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            width: u32,
            height: u32,
            lut_size: u32,
            _pad: u32,
            col0: [f32; 4],
            col1: [f32; 4],
            col2: [f32; 4],
        }

        let m = rgb_to_adapted_xyz;
        let params = Params {
            width: image.width,
            height: image.height,
            lut_size,
            _pad: 0,
            col0: [m[0][0] as f32, m[1][0] as f32, m[2][0] as f32, 0.0],
            col1: [m[0][1] as f32, m[1][1] as f32, m[2][1] as f32, 0.0],
            col2: [m[0][2] as f32, m[1][2] as f32, m[2][2] as f32, 0.0],
        };

        // TC LUT data is f64 on disk; narrow to f32 at the GPU boundary.
        let tc_lut_f32: Vec<f32> = tc_lut.data.iter().map(|&v| v as f32).collect();
        let input_f32 = scalars_to_f32(&image.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];

        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/spectral/hanatos2025_rgb_to_raw.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&tc_lut_f32)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            3, // output buffer index
            "front_transform",
        );

        ImageBuf::from_data(image.width, image.height, f32_to_scalars(result))
    }

    fn density_curve_interp(
        &self,
        log_raw: &ImageBuf,
        log_exposure: &[f64],
        density_curves: &[[f64; 3]],
        gamma_factor: f64,
    ) -> ImageBuf {
        let n_pixels = log_raw.pixel_count() as u32;
        let k = log_exposure.len() as u32;

        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            width: u32,
            height: u32,
            k: u32,
            uniform_grid: u32,
            gamma_inv: [f32; 3],
            _pad: f32,
        }
        // Detect uniformly-spaced log_exposure (typical case) to use the fast path.
        let uniform = is_uniform_grid_endpoint(log_exposure);
        let gamma_inv = if gamma_factor.abs() > 1e-12 {
            (1.0 / gamma_factor) as f32
        } else {
            1.0
        };
        let params = Params {
            width: log_raw.width,
            height: log_raw.height,
            k,
            uniform_grid: if uniform { 1 } else { 0 },
            gamma_inv: [gamma_inv; 3],
            _pad: 0.0,
        };

        let log_exp_f32: Vec<f32> = log_exposure.iter().map(|&v| v as f32).collect();
        let curves_f32: Vec<f32> = density_curves
            .iter()
            .flat_map(|r| r.iter().map(|&v| if v.is_nan() { 0.0 } else { v as f32 }))
            .collect();
        let input_f32 = scalars_to_f32(&log_raw.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];

        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/spectral/density_curve_interp.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&log_exp_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&curves_f32)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            4, // output buffer index
            "film_density",
        );

        ImageBuf::from_data(log_raw.width, log_raw.height, f32_to_scalars(result))
    }

    fn try_run_film_chain(&self, params: &crate::FilmChainParams<'_>) -> Option<ImageBuf> {
        if !params.gpu_blurs_supported() {
            tracing::info!(
                execution = "per_stage_cpu_blur",
                "resident blur exceeds GPU FIR support"
            );
            self.device
                .context
                .decline_resident(ResidentDeclineReason::BlurRadiusExceedsBackendSupport);
            return None;
        }
        Some(self.run_film_chain(params))
    }

    fn is_gpu(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        &self.name
    }
}

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
