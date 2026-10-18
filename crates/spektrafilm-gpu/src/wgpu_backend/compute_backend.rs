//! ComputeBackend trait implementation for WgpuBackend.
//! Extracted from mod.rs to separate the trait contract from dispatch infrastructure.

use super::film_chain::{
    explicit_resource_error, required_buffer_exceeds_limits, required_chain_buffers,
};
use super::pack_matrix_columns_f64;
use super::*;

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
        let device = ObservedDevice {
            raw: self.device.raw.clone(),
            context: context.clone(),
            batch: None,
        };
        let queue = ObservedQueue {
            raw: self.queue.raw.clone(),
            context,
        };
        Some(Box::new(Self {
            pipe: self.pipe.with_observed(device.clone(), queue.clone()),
            device,
            queue,
            pipeline_cache: self.pipeline_cache.clone(),
            name: self.name.clone(),
            adapter: self.adapter.clone(),
            explicit: self.explicit,
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
            s.context().record_executor(Executor::Gpu, None);
        }
        let n_pixels = img.pixel_count() as u32;
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
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
        let params = Params {
            width: img.width,
            height: img.height,
            m00: matrix[0][0],
            m01: matrix[0][1],
            m02: matrix[0][2],
            m10: matrix[1][0],
            m11: matrix[1][1],
            m12: matrix[1][2],
            m20: matrix[2][0],
            m21: matrix[2][1],
            m22: matrix[2][2],
        };
        let input_f32 = scalars_to_f32(&img.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];
        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/colorspace_convert.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            2,
            "colorspace_convert",
        );
        ImageBuf::from_data(img.width, img.height, f32_to_scalars(result))
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
            s.context().record_executor(Executor::Gpu, None);
        }
        let n_pixels = img.pixel_count() as u32;
        let params = CctfParams {
            width: img.width,
            height: img.height,
            mode: 0,
            _pad: 0,
        };
        let input_f32 = scalars_to_f32(&img.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];
        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/cctf_srgb.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            2,
            "cctf_encode_srgb",
        );
        ImageBuf::from_data(img.width, img.height, f32_to_scalars(result))
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
            s.context().record_executor(Executor::Gpu, None);
        }
        let n_pixels = img.pixel_count() as u32;
        let params = CctfParams {
            width: img.width,
            height: img.height,
            mode: 1,
            _pad: 0,
        };
        let input_f32 = scalars_to_f32(&img.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];
        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/cctf_srgb.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            2,
            "cctf_decode_srgb",
        );
        ImageBuf::from_data(img.width, img.height, f32_to_scalars(result))
    }
    fn grain_v2(
        &self,
        img: &ImageBuf,
        params: &spektrafilm_backend::GrainV2GpuParams,
    ) -> Option<ImageBuf> {
        Some(self.grain_v2_gpu(img, params))
    }
    fn dye_field(
        &self,
        img: &ImageBuf,
        params: &spektrafilm_backend::dye_params::DyeGpuParams,
    ) -> Option<Result<ImageBuf, String>> {
        // The device entry point is the inherent `WgpuBackend::dye_field`;
        // this hook only marks the attempt so a backend without a device
        // field path keeps the caller's host fallback.
        Some(WgpuBackend::dye_field(self, img, params))
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
            s.context().record_executor(Executor::Gpu, None);
        }
        let n_pixels = img.pixel_count() as u32;
        let k = table_x.len() as u32;
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            width: u32,
            height: u32,
            table_len: u32,
            #[allow(dead_code)]
            pad: u32,
        }
        let params = Params {
            width: img.width,
            height: img.height,
            table_len: k,
            pad: 0,
        };
        let table_y_f32: Vec<f32> = table_y.iter().flat_map(|r| r.iter().copied()).collect();
        let input_f32 = scalars_to_f32(&img.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];
        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/table_lookup.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(table_x)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&table_y_f32)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            4,
            "table_lookup",
        );
        ImageBuf::from_data(img.width, img.height, f32_to_scalars(result))
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
            s.context().record_executor(Executor::Gpu, None);
        }
        let n_pixels = img.pixel_count() as u32;
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            width: u32,
            height: u32,
            size: u32,
            #[allow(dead_code)]
            pad: u32,
        }
        let params = Params {
            width: img.width,
            height: img.height,
            size: lut.size,
            pad: 0,
        };
        let input_f32 = scalars_to_f32(&img.data);
        let output_bytes = vec![0u8; n_pixels as usize * 3 * 4];
        let result = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/lut3d_interp.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&lut.data)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_pixels,
            3,
            "lut3d_interp",
        );
        ImageBuf::from_data(img.width, img.height, f32_to_scalars(result))
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
        if self.device.context.enabled() && self.device.batch.is_none() {
            let (backend, _batch) = self.observed_batch("compute");
            return backend.scan_spectral(
                density_cmy,
                channel_density,
                base_density,
                illuminant,
                normalization,
                cat,
                xyz_to_rgb,
            );
        }
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
        let n_wl = channel_density.len().min(illuminant.len());
        let n_pixels = density_cmy.pixel_count() as u32;

        // Pack uniform params — mat3x3 in WGSL std140 is 3 columns of vec4.
        // GPU shaders are f32-only: cast precision-preserved inputs at the shader boundary.

        let columns = pack_matrix_columns_f64(xyz_to_rgb);
        let params = ScanParams {
            width: density_cmy.width,
            height: density_cmy.height,
            n_wavelengths: n_wl as u32,
            normalization: normalization as f32,
            col0: columns[0],
            col1: columns[1],
            col2: columns[2],
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
        let illu_f32: Vec<f32> = nan_safe_slice_to_f32(illuminant);
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
        if self.device.context.enabled() && self.device.batch.is_none() {
            let (backend, _batch) = self.observed_batch("compute");
            return backend.print_spectral(
                density_cmy,
                channel_density,
                base_density,
                illuminant,
                sensitivity,
                normalization_factor,
                preflash,
            );
        }
        let n_wl = channel_density
            .len()
            .min(illuminant.len())
            .min(sensitivity.len());
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
        let sens_flat: Vec<f32> = nan_safe_rows_to_f32(sensitivity);
        bd.resize(n_wl, 0.0);
        let illu_f32: Vec<f32> = nan_safe_slice_to_f32(illuminant);
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
        if self.device.context.enabled() && self.device.batch.is_none() {
            let (backend, _batch) = self.observed_batch("compute");
            return backend.hanatos2025_rgb_to_raw(
                image,
                tc_lut,
                color_space,
                ref_illuminant,
                cat16,
            );
        }
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

        let m = rgb_to_adapted_xyz;
        let columns = pack_matrix_columns_f64(m);
        let params = FrontParams {
            width: image.width,
            height: image.height,
            lut_size,
            _pad: 0,
            col0: columns[0],
            col1: columns[1],
            col2: columns[2],
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
        if self.device.context.enabled() && self.device.batch.is_none() {
            let (backend, _batch) = self.observed_batch("compute");
            return backend.density_curve_interp(
                log_raw,
                log_exposure,
                density_curves,
                gamma_factor,
            );
        }
        let n_pixels = log_raw.pixel_count() as u32;
        let k = log_exposure.len() as u32;

        // Detect uniformly-spaced log_exposure (typical case) to use the fast path.
        let uniform = is_uniform_grid_endpoint(log_exposure);
        let gamma_inv = if gamma_factor.abs() > 1e-12 {
            (1.0 / gamma_factor) as f32
        } else {
            1.0
        };
        let params = DensityParams {
            width: log_raw.width,
            height: log_raw.height,
            k,
            uniform_grid: if uniform { 1 } else { 0 },
            gamma_inv: [gamma_inv; 3],
            _pad: 0.0,
        };

        let log_exp_f32: Vec<f32> = log_exposure.iter().map(|&v| v as f32).collect();
        let curves_f32: Vec<f32> = nan_safe_rows_to_f32(density_curves);
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

    fn try_run_film_chain(
        &self,
        params: &spektrafilm_backend::FilmChainParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        // Bounded-execution preflight: compare the arithmetically required
        // buffer sizes with the device limits before any allocation, so an
        // oversized request is an identified failure instead of an
        // uncaptured wgpu-core validation error.
        let required = required_chain_buffers(params);
        if let Some(reason) =
            required_buffer_exceeds_limits(required.largest(), &self.device.limits())
        {
            if self.explicit_gpu_request() {
                return Err(explicit_resource_error(&reason));
            }
            tracing::info!(
                execution = "per_stage_cpu",
                reason = %reason,
                "using per-stage path because a resident buffer exceeds the device resource limits"
            );
            return Ok(None);
        }
        if !params.gpu_blurs_supported() {
            tracing::info!(
                execution = "per_stage_cpu_blur",
                "resident blur exceeds GPU FIR support"
            );
            self.device
                .context
                .decline_resident(ResidentDeclineReason::BlurRadiusExceedsBackendSupport);
            return Ok(None);
        }
        Ok(Some(self.run_film_chain(params)?))
    }

    fn is_gpu(&self) -> bool {
        true
    }

    fn explicit_gpu_request(&self) -> bool {
        self.explicit
    }

    fn with_explicit_request(&self) -> Option<Box<dyn ComputeBackend>> {
        Some(Box::new(self.derived_explicit()))
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// The derivation a frontend uses to turn an already selected GPU backend into
/// an explicit request: same device, same pipeline cache, `explicit` flipped so
/// the assembly-time guards apply.
#[cfg(test)]
mod tests {
    use super::*;
    use spektrafilm_math::image::ImageBuf;

    #[test]
    fn derived_explicit_request_reuses_the_device_and_pipeline_cache() {
        // The software Vulkan driver is not safe under concurrent device use
        // in one process; hold the shared lock for the whole test.
        let base = crate::test_support::adapter();
        assert!(!base.explicit_gpu_request());
        let derived = base
            .with_explicit_request()
            .expect("a WGPU backend derives an explicit request");
        assert!(derived.explicit_gpu_request());
        assert!(derived.is_gpu());
        assert_eq!(derived.name(), base.name());
        assert_eq!(
            derived.adapter_description().map(|adapter| adapter.name),
            base.adapter_description().map(|adapter| adapter.name)
        );

        // A kernel compiled through the base backend must be a pipeline-cache
        // hit for the derived backend; otherwise the frontend opened a second
        // device and recompiled everything.
        let matrix = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let image = ImageBuf::new(4, 4);
        let base_context = ObservationContext::new(CollectionMode::Summary);
        let observed = base.with_observation_context(base_context).unwrap();
        let _ = observed.colorspace_convert(&image, &matrix);

        let derived_context = ObservationContext::new(CollectionMode::Summary);
        let observed = derived
            .with_observation_context(derived_context.clone())
            .unwrap();
        let _ = observed.colorspace_convert(&image, &matrix);
        assert_eq!(
            derived_context.snapshot().totals.pipeline_cache_hit_count,
            Some(1),
            "the derived backend did not reuse the base pipeline cache"
        );
    }
}
