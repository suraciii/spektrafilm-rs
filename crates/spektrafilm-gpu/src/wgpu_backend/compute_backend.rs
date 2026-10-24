//! ComputeBackend trait implementation for WgpuBackend.
//! Extracted from mod.rs to separate the trait contract from dispatch infrastructure.

use super::film_chain::required_chain_buffers;
use super::pack_matrix_columns_f64;
use super::*;

impl ComputeBackend for WgpuBackend {
    fn preparation_device_id(&self) -> Option<u64> {
        Some(self.backend_id)
    }
    fn prepare_spectral_tc_lut(
        &self,
        p: &spektrafilm_backend::SpectralTcLutParams<'_>,
    ) -> Result<Option<spektrafilm_backend::PreparedTable>, String> {
        self.prepare_spectral_table_device(p).map(Some)
    }
    fn prepare_gamut_table(
        &self,
        p: &spektrafilm_backend::GamutTableParams,
    ) -> Result<Option<spektrafilm_backend::PreparedTable>, String> {
        self.prepare_gamut_table_device(p).map(Some)
    }
    fn remap_input_gamut_table(
        &self,
        source: &spektrafilm_backend::PreparedTable,
        p: &spektrafilm_backend::InputGamutTableParams<'_>,
    ) -> Result<spektrafilm_backend::PreparedTable, String> {
        self.remap_input_gamut_table_gpu(source, p)
    }
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
            scan_cache: self.scan_cache.clone(),
            name: self.name.clone(),
            adapter: self.adapter.clone(),
            explicit: self.explicit,
            backend_id: self.backend_id,
            cancellation: self.cancellation.clone(),
        }))
    }
    fn with_cancellation(
        &self,
        cancellation: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Option<Box<dyn ComputeBackend + '_>> {
        let mut backend = self.derived_explicit();
        backend.explicit = self.explicit;
        backend.pipe = self.pipe.with_cancellation(cancellation.clone());
        backend.cancellation = cancellation;
        Some(Box::new(backend))
    }
    fn colorspace_convert(
        &self,
        img: &ImageBuf,
        matrix: &[[f32; 3]; 3],
    ) -> Result<ImageBuf, String> {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "colorspace_convert",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
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
        let result = result.ok_or_else(|| {
            "colorspace_convert: device resource limits prevent dispatch".to_owned()
        })?;
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        Ok(ImageBuf::from_data(
            img.width,
            img.height,
            f32_to_scalars(result),
        ))
    }
    fn cctf_encode_srgb(&self, img: &ImageBuf) -> Result<ImageBuf, String> {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "output_transfer",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
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
        let result = result.ok_or_else(|| {
            "cctf_encode_srgb: device resource limits prevent dispatch".to_owned()
        })?;
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        Ok(ImageBuf::from_data(
            img.width,
            img.height,
            f32_to_scalars(result),
        ))
    }
    fn cctf_decode_srgb(&self, img: &ImageBuf) -> Result<ImageBuf, String> {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "input_transfer",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
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
        let result = result.ok_or_else(|| {
            "cctf_decode_srgb: device resource limits prevent dispatch".to_owned()
        })?;
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        Ok(ImageBuf::from_data(
            img.width,
            img.height,
            f32_to_scalars(result),
        ))
    }
    fn grain_v2(
        &self,
        img: &ImageBuf,
        params: &spektrafilm_backend::GrainV2GpuParams,
    ) -> Result<Option<ImageBuf>, String> {
        self.grain_v2_gpu(img, params).map(Some)
    }
    fn fotufilm_grain(
        &self,
        img: &ImageBuf,
        params: &spektrafilm_backend::FotufilmGpuParams,
    ) -> Option<Result<ImageBuf, String>> {
        WgpuBackend::fotufilm_grain_device(self, img, params)
    }
    fn dye_field(
        &self,
        img: &ImageBuf,
        params: &spektrafilm_backend::dye_params::DyeGpuParams,
    ) -> Option<Result<ImageBuf, String>> {
        // The device implementation pages both source and output; only its
        // bounded field footprint and coefficient limits can reject work.
        Some(WgpuBackend::dye_field(self, img, params))
    }
    fn dye_field_with_response(
        &self,
        image: &ImageBuf,
        params: &spektrafilm_backend::dye_params::DyeGpuParams,
        response: &spektrafilm_backend::DyeResponseGpuParams<'_>,
    ) -> Option<Result<ImageBuf, String>> {
        Some(WgpuBackend::dye_field_with_response(
            self, image, params, response,
        ))
    }
    fn dye_aperture_collapse(
        &self,
        image: &ImageBuf,
        factor: u32,
    ) -> Option<Result<ImageBuf, String>> {
        Some(WgpuBackend::dye_aperture_collapse(self, image, factor))
    }
    fn dye_log10(&self, image: &ImageBuf) -> Option<Result<ImageBuf, String>> {
        Some(WgpuBackend::dye_log10(self, image))
    }
    fn dye_crop(&self, image: &ImageBuf, rect: [u32; 4]) -> Option<Result<ImageBuf, String>> {
        Some(WgpuBackend::dye_crop(self, image, rect))
    }
    fn gaussian_blur(&self, img: &ImageBuf, sigma: f32) -> Result<ImageBuf, String> {
        self.gaussian_blur_gpu(img, sigma)
    }
    fn gaussian_blur_multi(&self, img: &ImageBuf, sigmas: &[f32]) -> Result<Vec<ImageBuf>, String> {
        self.gaussian_blur_multi_gpu(img, sigmas)
    }
    fn table_lookup(
        &self,
        img: &ImageBuf,
        table_x: &[f32],
        table_y: &[[f32; 3]],
    ) -> Result<ImageBuf, String> {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "table_lookup",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
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
        let result = result
            .ok_or_else(|| "table_lookup: device resource limits prevent dispatch".to_owned())?;
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        Ok(ImageBuf::from_data(
            img.width,
            img.height,
            f32_to_scalars(result),
        ))
    }
    fn lut3d_interp(&self, img: &ImageBuf, lut: &Lut3D) -> Result<ImageBuf, String> {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "spectral_lut",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        let result = self.lut3d_chunks(img, lut)?;
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        Ok(result)
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
    ) -> Result<ImageBuf, String> {
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
        // Combine the same factors at the GPU shader boundary.
        let combined_xyz_to_rgb = &combined;
        let n_wl = channel_density.len().min(illuminant.len());
        let n_pixels = density_cmy.pixel_count() as u32;

        // Pack uniform params — mat3x3 in WGSL std140 is 3 columns of vec4.
        // GPU shaders are f32-only: cast precision-preserved inputs at the shader boundary.

        let columns = pack_matrix_columns_f64(combined_xyz_to_rgb);
        let params = ScanParams {
            width: density_cmy.width,
            height: density_cmy.height,
            n_wavelengths: n_wl as u32,
            normalization: normalization as f32,
            col0: columns[0],
            col1: columns[1],
            col2: columns[2],
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
        let result = result
            .ok_or_else(|| "scan_spectral: device resource limits prevent dispatch".to_owned())?;

        Ok(ImageBuf::from_data(
            density_cmy.width,
            density_cmy.height,
            f32_to_scalars(result),
        ))
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
    ) -> Result<ImageBuf, String> {
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

        // GPU shaders are f32-only — narrow f64 inputs at the shader boundary.
        // See `sanitize_spectral_inputs` for the NaN-handling story (Metal fast-math).
        let (cd_flat, mut bd) = sanitize_spectral_inputs(channel_density, base_density, n_wl);
        let sens_flat: Vec<f32> = nan_safe_rows_to_f32(sensitivity);
        bd.resize(n_wl, 0.0);
        let illu_f32: Vec<f32> = nan_safe_slice_to_f32(illuminant);
        self.dispatch_rgb_chunks(
            density_cmy,
            include_str!("../../../spektrafilm-shaders/wgsl/spectral/print_spectral.wgsl"),
            &[&cd_flat, &bd, &illu_f32, &sens_flat],
            |n| Params {
                width: n,
                height: 1,
                n_wavelengths: n_wl as u32,
                normalization_factor: normalization_factor as f32,
                preflash: [preflash[0] as f32, preflash[1] as f32, preflash[2] as f32],
                _pad: 0.0,
            },
            "print_spectral",
        )
    }

    fn optical_diffusion(
        &self,
        img: &ImageBuf,
        params: &spektrafilm_backend::OpticalDiffusionGpuParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        let (backend, _batch) = self.observed_batch("optical_diffusion");
        backend.optical_diffusion_gpu(img, params).map(Some)
    }

    fn enlarger_spectral_lut(
        &self,
        image: &ImageBuf,
        params: &spektrafilm_backend::EnlargerSpectralLutGpuParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        self.enlarger_resident(image, params).map(Some)
    }

    fn scanner_spectral_lut(
        &self,
        image: &ImageBuf,
        params: &spektrafilm_backend::ScannerSpectralLutGpuParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        let (backend, _batch) = self.observed_batch("scanner_spectral_lut");
        backend.scanner_resident(image, params).map(Some)
    }

    fn enlarger_lut_interp(
        &self,
        img: &ImageBuf,
        params: &spektrafilm_backend::EnlargerLutGpuParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        self.pchip_chunks(img, params).map(Some)
    }

    fn hanatos2025_rgb_to_raw(
        &self,
        image: &ImageBuf,
        tc_lut: &spektrafilm_math::spectral::TcLut,
        color_space: &str,
        ref_illuminant: &[f32],
        cat16: bool,
    ) -> Result<ImageBuf, String> {
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
        let result = result.ok_or_else(|| {
            "hanatos2025_rgb_to_raw: device resource limits prevent dispatch".to_owned()
        })?;

        Ok(ImageBuf::from_data(
            image.width,
            image.height,
            f32_to_scalars(result),
        ))
    }

    fn density_curve_interp(
        &self,
        log_raw: &ImageBuf,
        log_exposure: &[f64],
        density_curves: &[[f64; 3]],
        gamma_factor: f64,
    ) -> Result<ImageBuf, String> {
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
        let result = result.ok_or_else(|| {
            "density_curve_interp: device resource limits prevent dispatch".to_owned()
        })?;

        Ok(ImageBuf::from_data(
            log_raw.width,
            log_raw.height,
            f32_to_scalars(result),
        ))
    }

    fn mix_output(
        &self,
        result: &ImageBuf,
        source: &ImageBuf,
        params: &spektrafilm_backend::OutputMixGpuParams,
    ) -> Result<Option<ImageBuf>, String> {
        if result.width != source.width || result.height != source.height {
            return Err("output_mix source and final output geometry differ".into());
        }
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "output_mix",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        let n_pixels = result.pixel_count() as u32;
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct MixParams {
            n_pixels: u32,
            encoded: u32,
            cctf_kind: u32,
            amount: f32,
        }
        let mix_params = MixParams {
            n_pixels,
            encoded: u32::from(params.encoded),
            cctf_kind: params.cctf_kind,
            amount: params.amount,
        };
        let result_f32 = scalars_to_f32(&result.data);
        let source_f32 = scalars_to_f32(&source.data);
        let mixed = self.dispatch_compute(
            concat!(
                include_str!("../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                include_str!("../../../spektrafilm-shaders/wgsl/output_mix.wgsl")
            ),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&mix_params)),
                GpuBuffer::storage_rw(bytemuck::cast_slice(&result_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&source_f32)),
            ],
            n_pixels,
            1,
            "output_mix",
        );
        let mixed = mixed
            .ok_or_else(|| "output_mix: device resource limits prevent dispatch".to_owned())?;
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        Ok(Some(ImageBuf::from_data(
            result.width,
            result.height,
            f32_to_scalars(mixed),
        )))
    }

    /// Full-source preview, luminance reduction, and EV interpretation run
    /// on the device. Only the bounded exposure control returns to the host.
    fn meter_autoexposure(
        &self,
        image: &ImageBuf,
        params: &spektrafilm_backend::MeterAutoexposureParams,
    ) -> Result<Option<f64>, String> {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "metering",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        let state = build_meter_state(&self.device, params, image, self)?;
        Ok(Some(state.meter(&self.pipe)?))
    }

    fn convert_colorspace(
        &self,
        img: &ImageBuf,
        params: &spektrafilm_backend::ConvertColorspaceGpuParams,
    ) -> Result<Option<ImageBuf>, String> {
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
        struct ConvertParams {
            n_pixels: u32,
            src_cctf: u32,
            decode: u32,
            dst_cctf: u32,
            encode: u32,
            _pad: [u32; 3],
            col0: [f32; 4],
            col1: [f32; 4],
            col2: [f32; 4],
        }
        let columns = pack_matrix_columns(&params.matrix);
        let convert_params = ConvertParams {
            n_pixels,
            src_cctf: params.src_cctf,
            decode: u32::from(params.decode),
            dst_cctf: params.dst_cctf,
            encode: u32::from(params.encode),
            _pad: [0; 3],
            col0: columns[0],
            col1: columns[1],
            col2: columns[2],
        };
        let input_f32 = scalars_to_f32(&img.data);
        let converted = self.dispatch_compute(
            concat!(
                include_str!("../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                include_str!("../../../spektrafilm-shaders/wgsl/convert_colorspace.wgsl")
            ),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&convert_params)),
                GpuBuffer::storage_rw(bytemuck::cast_slice(&input_f32)),
            ],
            n_pixels,
            1,
            "convert_colorspace",
        );
        let converted = converted.ok_or_else(|| {
            "convert_colorspace: device resource limits prevent dispatch".to_owned()
        })?;
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        Ok(Some(ImageBuf::from_data(
            img.width,
            img.height,
            f32_to_scalars(converted),
        )))
    }

    /// Saving sample conversion of retained output. The samples stay on the
    /// device until the writer's terminal readback; the returned payload is
    /// already clipped, transfer encoded, and quantized.
    fn convert_retained_samples(
        &self,
        image: &spektrafilm_backend::DeviceImage,
        params: &spektrafilm_backend::OutputSampleGpuParams,
    ) -> Result<Option<spektrafilm_backend::OutputSampleData>, String> {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "saving_convert",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        if let Some(scope) = &scope {
            scope.context().record_executor(Executor::Gpu, None);
        }
        if !image.is_retained() {
            return Err(
                "saving conversion: retained samples were requested for a host carrier".to_owned(),
            );
        }
        self.output_samples_from_retained(image, params).map(Some)
    }

    /// Saving sample conversion of host samples: one upload of the final
    /// image, the same device pass, one typed readback.
    fn convert_image_samples(
        &self,
        image: &ImageBuf,
        params: &spektrafilm_backend::OutputSampleGpuParams,
    ) -> Result<Option<spektrafilm_backend::OutputSampleData>, String> {
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "saving_convert",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        if let Some(scope) = &scope {
            scope.context().record_executor(Executor::Gpu, None);
        }
        self.output_samples_from_host(image, params).map(Some)
    }

    /// The RAW sensor pipeline on device: LibRaw's post-unpack stages, black
    /// subtraction through `convert_to_rgb`, in `dcraw_process`'s order. The
    /// sensor record and the plan come from `spektrafilm-raw`; the measured
    /// sensor maximum is resolved on device, because it is image arithmetic.
    fn raw_process_sensor(
        &self,
        sensor: &spektrafilm_backend::RawSensor,
        params: &spektrafilm_backend::RawGpuParams,
    ) -> Result<Option<ImageBuf>, String> {
        let scope = self.device.context.scope(
            "raw_sensor",
            ObservationKind::Stage,
            self.device.context.purpose(),
        );
        scope.context().record_executor(Executor::Gpu, None);
        Ok(Some(self.raw_process_sensor_gpu(sensor, params)?))
    }

    fn raw_process_sensor_retained(
        &self,
        sensor: &spektrafilm_backend::RawSensor,
        params: &spektrafilm_backend::RawGpuParams,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        let scope = self.device.context.scope(
            "raw_sensor",
            ObservationKind::Stage,
            self.device.context.purpose(),
        );
        scope.context().record_executor(Executor::Gpu, None);
        Ok(Some(self.raw_process_sensor_retained_gpu(sensor, params)?))
    }

    fn viewer_fit_retained(
        &self,
        i: &spektrafilm_backend::DeviceImage,
        w: u32,
        h: u32,
        d: Option<&spektrafilm_backend::DisplayTransformGpuParams>,
        c: Option<&spektrafilm_backend::ViewerIccGpuParams<'_>>,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.viewer_fit_gpu(i, w, h, d, c)
    }
    fn viewer_full_source_retained(
        &self,
        i: &spektrafilm_backend::DeviceImage,
        p: &spektrafilm_backend::ViewerFullSourceGpuParams<'_>,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.viewer_full_gpu(i, p)
    }
    fn viewer_compose_retained(
        &self,
        i: Option<&spektrafilm_backend::DeviceImage>,
        paper: Option<&spektrafilm_backend::DeviceImage>,
        previous: Option<&spektrafilm_backend::DeviceImage>,
        split: Option<&spektrafilm_backend::DeviceImage>,
        p: &spektrafilm_backend::ViewerComposeGpuParams,
    ) -> Result<Vec<[u8; 4]>, String> {
        self.viewer_compose_gpu(i, paper, previous, split, p)
    }
    fn viewer_paper_retained(
        &self,
        w: u32,
        h: u32,
        logo: &[f32],
        lw: u32,
        lh: u32,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.viewer_paper_gpu(w, h, logo, lw, lh)
    }
    fn inspect_retained_pixel(
        &self,
        i: &spektrafilm_backend::DeviceImage,
        x: u32,
        y: u32,
    ) -> Result<[f32; 3], String> {
        self.viewer_inspect_gpu(i, x, y)
    }
    fn retained_histogram(
        &self,
        image: &spektrafilm_backend::DeviceImage,
    ) -> Result<[[f32; 64]; 3], String> {
        self.retained_histogram_gpu(image)
    }
    fn viewer_icc_retained(
        &self,
        i: &spektrafilm_backend::DeviceImage,
        p: &spektrafilm_backend::ViewerIccGpuParams<'_>,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.viewer_icc_gpu(i, p)
    }
    fn viewer_polaroid_retained(
        &self,
        i: &spektrafilm_backend::DeviceImage,
        c: [f32; 9],
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.viewer_polaroid_gpu(i, c)
    }

    /// The viewer's capped raster on the device: one bilinear tap per output
    /// pixel over the exact source rectangle the host maps it to, reading the
    /// source image read-only. The host still owns the destination size, so a
    /// backend cannot disagree with the viewer about how many pixels it keeps.
    fn viewer_resample(
        &self,
        image: &ImageBuf,
        params: &spektrafilm_backend::ViewerResampleGpuParams,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        if image.width == 0 || image.height == 0 {
            return Ok(None);
        }
        // The host supplies the destination size it already computed in f64;
        // re-deriving it here would let the device disagree with the viewer
        // about the raster geometry.
        if params.width != image.width || params.height != image.height {
            return Err("viewer resample request geometry differs from the source image".into());
        }
        let out_width = params.out_width;
        let out_height = params.out_height;
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "viewer_resample",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct ResampleParams {
            width: u32,
            height: u32,
            out_width: u32,
            out_height: u32,
        }
        let shader_params = ResampleParams {
            width: image.width,
            height: image.height,
            out_width,
            out_height,
        };
        let input_f32 = scalars_to_f32(&image.data);
        let n_out = out_width * out_height;
        let output_bytes = vec![0u8; n_out as usize * 3 * 4];
        let resampled = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/viewer_resample.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&shader_params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input_f32)),
                GpuBuffer::storage_rw(&output_bytes),
            ],
            n_out,
            2,
            "viewer_resample",
        );
        let Some(resampled) = resampled else {
            return Err("viewer_resample exceeds device resource limits".into());
        };
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        Ok(Some(resampled.as_chunks::<3>().0.to_vec()))
    }

    /// The viewer's display transform on the device: decode the source
    /// transfer, apply the source space's display matrix, encode sRGB, and
    /// clamp to 0..=1. The host supplies the display matrix it resolved in
    /// reference precision, so the device arithmetic uses the same matrix the
    /// viewer would.
    fn viewer_display_transform(
        &self,
        rgb: &[[f32; 3]],
        params: &spektrafilm_backend::DisplayTransformGpuParams,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        if rgb.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "viewer_display_transform",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        let n_pixels = rgb.len() as u32;
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct DisplayParams {
            n_pixels: u32,
            src_cctf: u32,
            decode: u32,
            clamp: u32,
            col0: [f32; 4],
            col1: [f32; 4],
            col2: [f32; 4],
        }
        let columns = pack_matrix_columns(&params.matrix);
        let display_params = DisplayParams {
            n_pixels,
            src_cctf: params.src_cctf,
            decode: u32::from(params.decode),
            clamp: u32::from(params.clamp),
            col0: columns[0],
            col1: columns[1],
            col2: columns[2],
        };
        let input: Vec<f32> = rgb.iter().flat_map(|p| *p).collect();
        let transformed = self.dispatch_compute(
            concat!(
                include_str!("../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                include_str!("../../../spektrafilm-shaders/wgsl/viewer_display_transform.wgsl")
            ),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&display_params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&input)),
                GpuBuffer::storage_rw(&vec![0u8; rgb.len() * 3 * 4]),
            ],
            n_pixels,
            2,
            "viewer_display_transform",
        );
        let Some(transformed) = transformed else {
            return Err("viewer_display_transform exceeds device resource limits".into());
        };
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        Ok(Some(transformed.as_chunks::<3>().0.to_vec()))
    }

    /// Upload disposable viewer samples, prepare the parsed ICC pipeline on
    /// device, and materialize only the final viewing raster.
    fn viewer_icc_transform(
        &self,
        rgb: &[[f32; 3]],
        params: &spektrafilm_backend::ViewerIccGpuParams<'_>,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        self.viewer_icc_samples_gpu(rgb, params).map(Some)
    }

    /// The viewer's magnified full-source composition on the device: one
    /// separable interpolation tap set per visible viewport pixel over the
    /// full-resolution source, then the display transform and the ICC
    /// pipeline when the request carries them. The host supplies the geometry
    /// and the resolved transforms, so the device evaluates the same
    /// per-pixel arithmetic the host composition does.
    fn viewer_full_source(
        &self,
        image: &ImageBuf,
        params: &spektrafilm_backend::ViewerFullSourceGpuParams<'_>,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        if image.width == 0 || image.height == 0 {
            return Ok(None);
        }
        let n_pixels = params.tex_width * params.tex_height;
        if n_pixels == 0 {
            return Ok(Some(Vec::new()));
        }
        let scope = if self.device.context.enabled() {
            Some(self.device.context.scope(
                "viewer_full_source",
                ObservationKind::Stage,
                self.device.context.purpose(),
            ))
        } else {
            None
        };
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct FullSourceParams {
            tex_width: u32,
            tex_height: u32,
            src_width: u32,
            src_height: u32,
            rect_min_x: f32,
            rect_min_y: f32,
            rect_width: f32,
            rect_height: f32,
            image_min_x: f32,
            image_min_y: f32,
            image_width: f32,
            image_height: f32,
            interpolation: u32,
            src_cctf: u32,
            decode: u32,
            transform: u32,
            col0: [f32; 4],
            col1: [f32; 4],
            col2: [f32; 4],
            icc: u32,
            icc_matrix: u32,
            icc_mat00: i32,
            icc_mat01: i32,
            icc_mat02: i32,
            icc_mat10: i32,
            icc_mat11: i32,
            icc_mat12: i32,
            icc_mat20: i32,
            icc_mat21: i32,
            icc_mat22: i32,
            icc_off0: i32,
            icc_off1: i32,
            icc_off2: i32,
        }
        let columns = pack_matrix_columns(&params.matrix);
        let icc_matrix = params.icc_matrix_fixed14;
        let icc_offset = params.icc_offset_fixed14;
        let shader_params = FullSourceParams {
            tex_width: params.tex_width,
            tex_height: params.tex_height,
            src_width: image.width,
            src_height: image.height,
            rect_min_x: params.rect_min[0],
            rect_min_y: params.rect_min[1],
            rect_width: params.rect_size[0],
            rect_height: params.rect_size[1],
            image_min_x: params.image_min[0],
            image_min_y: params.image_min[1],
            image_width: params.image_size[0],
            image_height: params.image_size[1],
            interpolation: params.interpolation,
            src_cctf: params.src_cctf,
            decode: u32::from(params.decode),
            transform: u32::from(params.transform),
            col0: columns[0],
            col1: columns[1],
            col2: columns[2],
            icc: u32::from(params.icc),
            icc_matrix: u32::from(params.icc_matrix),
            icc_mat00: icc_matrix[0][0],
            icc_mat01: icc_matrix[0][1],
            icc_mat02: icc_matrix[0][2],
            icc_mat10: icc_matrix[1][0],
            icc_mat11: icc_matrix[1][1],
            icc_mat12: icc_matrix[1][2],
            icc_mat20: icc_matrix[2][0],
            icc_mat21: icc_matrix[2][1],
            icc_mat22: icc_matrix[2][2],
            icc_off0: icc_offset[0],
            icc_off1: icc_offset[1],
            icc_off2: icc_offset[2],
        };
        let source_f32 = scalars_to_f32(&image.data);
        let icc_first: Vec<i32> = params
            .icc_first
            .iter()
            .flat_map(|table| table.iter().copied())
            .collect();
        let icc_second: Vec<u32> = params
            .icc_second
            .iter()
            .flat_map(|table| table.iter().copied())
            .collect();
        let composed = self.dispatch_compute(
            include_str!("../../../spektrafilm-shaders/wgsl/viewer_full_source.wgsl"),
            &[
                GpuBuffer::uniform(bytemuck::bytes_of(&shader_params)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&source_f32)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&icc_first)),
                GpuBuffer::storage_ro(bytemuck::cast_slice(&icc_second)),
                GpuBuffer::storage_rw(&vec![0u8; n_pixels as usize * 3 * 4]),
            ],
            n_pixels,
            4,
            "viewer_full_source",
        );
        let Some(composed) = composed else {
            return Err("viewer_full_source exceeds device resource limits".into());
        };
        if let Some(s) = &scope {
            s.context().record_executor(Executor::Gpu, None);
        }
        Ok(Some(composed.as_chunks::<3>().0.to_vec()))
    }
    fn try_run_film_chain_retained(
        &self,
        input: &spektrafilm_backend::DeviceImage,
        params: &spektrafilm_backend::FilmChainParams<'_>,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        let required = required_chain_buffers(params);
        let binding_limit = super::bounded::storage_binding_limit(self)?;
        if required.static_bytes > binding_limit {
            return Err(format!(
                "chain minimum coefficient working set requires {} bytes per binding; limit is {binding_limit}",
                required.static_bytes
            ));
        }
        if !params.gpu_blurs_supported() {
            return Err(
                "selected GPU chain cannot represent the requested blur working set".to_owned(),
            );
        }
        Ok(Some(self.run_film_chain_retained(input, params)?))
    }

    fn retain_image(
        &self,
        image: &ImageBuf,
        meta: &spektrafilm_backend::RetainedImageMeta,
    ) -> Result<spektrafilm_backend::DeviceImage, String> {
        self.upload_retained(image, meta)
    }

    fn image_payload<'a>(
        &'a self,
        image: &'a spektrafilm_backend::DeviceImage,
    ) -> Result<&'a (dyn spektrafilm_backend::DeviceImagePayload + 'static), String> {
        Ok(self.buffer_payload(image)?)
    }

    fn materialize_retained(
        &self,
        image: &spektrafilm_backend::DeviceImage,
    ) -> Result<ImageBuf, String> {
        self.materialize_buffer(image)
    }

    fn input_geometry(
        &self,
        image: &spektrafilm_backend::DeviceImage,
        params: &spektrafilm_backend::InputGeometryGpuParams,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        self.encode_input_geometry(image, params)
    }

    fn scale_retained(
        &self,
        image: &spektrafilm_backend::DeviceImage,
        scale: f32,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        Ok(Some(self.scale_retained_image(image, scale)?))
    }

    fn convert_colorspace_retained(
        &self,
        image: &spektrafilm_backend::DeviceImage,
        params: &spektrafilm_backend::ConvertColorspaceGpuParams,
        destination: &'static spektrafilm_math::colorspace::RgbColorSpace,
        domain: spektrafilm_backend::SampleDomain,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        Ok(Some(self.convert_colorspace_image(
            image,
            params,
            destination,
            domain,
        )?))
    }

    fn cmyk_retained(
        &self,
        image: &spektrafilm_backend::DeviceImage,
        params: &spektrafilm_backend::CmykGpuParams,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        Ok(Some(self.cmyk_retained(image, params)?))
    }

    fn mix_output_retained(
        &self,
        result: &spektrafilm_backend::DeviceImage,
        source: &spektrafilm_backend::DeviceImage,
        params: &spektrafilm_backend::OutputMixGpuParams,
    ) -> Result<Option<spektrafilm_backend::DeviceImage>, String> {
        Ok(Some(self.mix_output_images(result, source, params)?))
    }

    fn meter_autoexposure_retained(
        &self,
        image: &spektrafilm_backend::DeviceImage,
        params: &spektrafilm_backend::MeterAutoexposureParams,
    ) -> Result<Option<f64>, String> {
        if !self.device.context.enabled() {
            // No batch scope: the meter reads its bounded scalar back here.
        }
        let payload = self.buffer_payload(image)?;
        if image.width() != params.width || image.height() != params.height {
            return Err(format!(
                "metering source carrier is {}x{} but the resolved request is {}x{}",
                image.width(),
                image.height(),
                params.width,
                params.height
            ));
        }
        let state = super::metering::build_meter_state_from_payload(self, params, payload)?;
        Ok(Some(state.meter(&self.pipe)?))
    }

    fn is_gpu(&self) -> bool {
        true
    }

    fn explicit_gpu_request(&self) -> bool {
        self.explicit
    }

    fn image_binding_limit_reason(&self, img: &ImageBuf) -> Option<String> {
        // Every image binding is the f32 RGB buffer of the image.
        let bytes = img.pixel_count() as u64 * 3 * 4;
        super::film_chain::required_buffer_exceeds_limits(bytes, &self.device.limits())
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
        observed
            .colorspace_convert(&image, &matrix)
            .expect("base device colorspace conversion");

        let derived_context = ObservationContext::new(CollectionMode::Summary);
        let observed = derived
            .with_observation_context(derived_context.clone())
            .unwrap();
        observed
            .colorspace_convert(&image, &matrix)
            .expect("derived device colorspace conversion");
        assert_eq!(
            derived_context.snapshot().totals.pipeline_cache_hit_count,
            Some(1),
            "the derived backend did not reuse the base pipeline cache"
        );
    }

    /// The device mixer must reproduce the reference blend, which decodes only
    /// the processed operand: the prepared source is linear output primaries.
    /// A shader that decodes both operands produces a visibly different image,
    /// so this catches that class of mistake rather than a rounding wobble.
    #[test]
    fn device_output_mix_matches_the_reference_blend() {
        use spektrafilm_math::colorspace::{Cctf, cctf_decode, cctf_encode};
        use spektrafilm_math::precision::{from_f64, to_f64};
        let backend = crate::test_support::adapter();
        // A chromatic wedge: the result in the encoded output domain, the
        // source linear — the exact operand domains the pipeline supplies.
        let n = 24u32;
        let (result, source): (Vec<_>, Vec<_>) = (0..n * 3)
            .map(|i| {
                let x = (i / 3) as f64 / (n - 1) as f64;
                let linear = 0.02 + 0.9 * x;
                (from_f64(cctf_encode(linear, Cctf::Srgb)), from_f64(linear))
            })
            .unzip();
        let result = ImageBuf::from_data(n, 1, result);
        let source = ImageBuf::from_data(n, 1, source);
        for amount in [0.0f32, 0.25, 0.5, 0.75] {
            let mixed = backend
                .mix_output(
                    &result,
                    &source,
                    &spektrafilm_backend::OutputMixGpuParams {
                        amount,
                        encoded: true,
                        cctf_kind: 2,
                    },
                )
                .expect("device mixer")
                .expect("WGPU provides the device mixer");
            let mut worst = 0.0f64;
            for (i, (r, s)) in result.data.iter().zip(source.data.iter()).enumerate() {
                let rl = cctf_decode(to_f64(*r), Cctf::Srgb);
                let reference = cctf_encode(
                    f64::from(1.0 - amount) * to_f64(*s) + f64::from(amount) * rl,
                    Cctf::Srgb,
                );
                worst = worst.max((reference - to_f64(mixed.data[i])).abs());
            }
            assert!(
                worst < 1e-4,
                "amount {amount}: the device mix differs from the reference blend by {worst:.3e}"
            );
        }
    }
}
