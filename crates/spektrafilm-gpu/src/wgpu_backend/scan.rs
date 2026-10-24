//! Resident scanner acquisition and bounded inverse scan.
use super::WgpuBackend;
use super::chain_chunks::PointwisePass;
use super::grain::{
    RasterStorage,
    raster::RasterCommands,
    v1::blur::{GrainBlurState, build_grain_blur_paged},
};
use spektrafilm_backend::{ConvertSolverGpuParams, ScannerDensityGpuParams};

#[path = "scan_setup.rs"]
mod scan_setup;

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;

/// Immutable binary64 reconstruction. `exposure` pages contain 24 bytes per
/// RGB pixel (not the ordinary f32 RasterStorage transport).
pub(super) struct PreparedScan {
    pub(super) exposure: Vec<super::paged::ImagePage>,
    pub(super) setup: wgpu::Buffer,
    pub(super) status: wgpu::Buffer,
}

pub(super) struct ConvertInverseState {
    pass: PointwisePass,
}
impl ConvertInverseState {
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.pass.submit(backend)?;
        backend.check_cancelled()
    }
}
pub(super) struct ScanDensityState {
    pass: RasterCommands,
    reconstruct: Option<RasterCommands>,
    setup_pass: Option<scan_setup::FrameSetupState>,
    optics: Option<(PointwisePass, GrainBlurState, RasterCommands, PointwisePass)>,
    status: wgpu::Buffer,
    prepared: std::sync::Arc<PreparedScan>,
}
impl ScanDensityState {
    pub(super) fn has_setup(&self) -> bool {
        self.setup_pass.is_some()
    }
    pub(super) fn submit_optics(&self, backend: &WgpuBackend) -> Result<(), String> {
        if let Some((transmit, blur, restore, density)) = &self.optics {
            transmit.submit(backend)?;
            blur.submit(backend)?;
            restore.submit(backend)?;
            density.submit(backend)?;
        }
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn submit_setup(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.submit_setup_observed(backend, &backend.device.context)
    }
    pub(super) fn submit_setup_observed(
        &self,
        backend: &WgpuBackend,
        context: &spektrafilm_backend::telemetry::ObservationContext,
    ) -> Result<(), String> {
        let Some(setup) = &self.setup_pass else {
            return Ok(());
        };
        backend.check_cancelled()?;
        if !context.enabled() {
            return setup.submit(backend);
        }
        // Film-chain transport already has a batch. The caller supplies its
        // Phase/Stage ancestor so this Stage is a sibling, never a batch child.
        let mut scope = context.scope(
            "standard_setup",
            spektrafilm_backend::telemetry::ObservationKind::Stage,
            context.purpose(),
        );
        let mut observed = backend.derived_explicit();
        observed.explicit = backend.explicit;
        observed.context_mut(scope.context().clone());
        // A dedicated child batch preserves actual pass timing and attributes
        // submitted reduction dispatches to the real setup stage.
        let result = {
            let (batch, _guard) = observed.observed_batch("compute");
            setup.submit(&batch)
        };
        scope.set_complete(result.is_ok());
        result
    }
    pub(super) fn submit_inverse(&self, backend: &WgpuBackend) -> Result<(), String> {
        if let Some(reconstruct) = &self.reconstruct {
            reconstruct.submit(backend)?;
        }
        self.pass.submit(backend)
    }
    pub(super) fn check(
        &self,
        backend: &WgpuBackend,
        p: &ScannerDensityGpuParams,
        preparation_key: Option<&std::sync::Arc<dyn spektrafilm_backend::DeviceImagePayload>>,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        backend.check_cancelled()?;
        // Misses retain exactly this device status with the prepared exposure;
        // hits write a fresh render-only status so failures cannot poison it.
        let status = if self.reconstruct.is_some() {
            &self.prepared.status
        } else {
            &self.status
        };
        let code = backend.pipe.readback_buffer_as::<u32>(status, 4)[0];
        backend.check_cancelled()?;
        if code & 2 != 0 {
            return Err("standard setup requires finite film record densities".into());
        }
        if code & 4 != 0 {
            return Err("standard setup statistics exceed finite density range".into());
        }
        if code & 8 != 0 {
            return Err("standard setup correction exceeds finite exposure range".into());
        }
        if code & 16 != 0 {
            return Err(
                "scanner.operator or negative conversion combined tone coordinate overflow".into(),
            );
        }
        if code != 0 {
            return Err("density inverse exceeds finite relative exposure range or output is not representable in the selected precision".into());
        }
        if self.reconstruct.is_some()
            && let Some(key) = preparation_key
        {
            backend.publish_prepared_scan(key.clone(), p, width, height, self.prepared.clone())?;
        }
        Ok(())
    }
}
impl WgpuBackend {
    fn scan_pass(
        &self,
        shader: &'static str,
        types: &[wgpu::BufferBindingType],
        buffers: &[&dyn RasterStorage],
        images: &[(usize, bool)],
        count: u32,
    ) -> Result<PointwisePass, String> {
        let pipeline = self.cached_pipeline(shader, types);
        self.chain_pointwise(pipeline, buffers, images, count, false, "scan_device")
    }
    pub(super) fn build_scan_direct_state(
        &self,
        p: &spektrafilm_backend::FilmChainParams<'_>,
        source: &dyn RasterStorage,
        output: &dyn RasterStorage,
        width: u32,
        height: u32,
    ) -> Result<PointwisePass, String> {
        if !self.device.features().contains(wgpu::Features::SHADER_F64) {
            return Err("direct scanner acquisition requires native shader f64".into());
        }
        let (density, base) = if p.scan_film {
            (p.film_channel_density, p.film_base_density)
        } else {
            (p.print_channel_density, p.print_base_density)
        };
        let count = density
            .len()
            .min(p.scan_illuminant.len())
            .min(spektrafilm_math::spectral::N_WAVELENGTHS);
        let has_base = base.len() >= count && !base.is_empty();
        let active: Vec<_> = (0..count)
            .filter(|&i| {
                !density[i].iter().any(|v| v.is_nan())
                    && (!has_base || !base[i].is_nan())
                    && !p.scan_illuminant[i].is_nan()
            })
            .collect();
        if active.is_empty() {
            return Err("direct scanner acquisition has no contributing wavelengths".into());
        }
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct Params {
            header: [u32; 4],
            normalization: [f64; 2],
            cat: [[f64; 4]; 3],
            rgb: [[f64; 4]; 3],
        }
        let row = |r: [f64; 3]| [r[0], r[1], r[2], 0.0];
        let params = Params {
            header: [width * height, active.len() as u32, 0, 0],
            normalization: [p.scan_normalization, 0.0],
            cat: p.scan_cat.map(row),
            rgb: p.scan_base_xyz_to_rgb.map(row),
        };
        let table = |name, bytes: &[u8]| {
            self.pipe
                .static_buffer(name, bytes, wgpu::BufferUsages::STORAGE)
        };
        let dye: Vec<_> = active.iter().map(|&i| density[i]).collect();
        let base: Vec<_> = active
            .iter()
            .map(|&i| if has_base { base[i] } else { 0.0 })
            .collect();
        let lamp: Vec<_> = active.iter().map(|&i| p.scan_illuminant[i]).collect();
        let cmfs: Vec<_> = active
            .iter()
            .map(|&i| {
                [
                    spektrafilm_math::spectral::CMF_X_F64[i],
                    spektrafilm_math::spectral::CMF_Y_F64[i],
                    spektrafilm_math::spectral::CMF_Z_F64[i],
                ]
            })
            .collect();
        let uniform = self.pipe.static_buffer(
            "direct_scan_params",
            bytemuck::bytes_of(&params),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_SRC,
        );
        let dye = table("direct_scan_dyes", bytemuck::cast_slice(&dye));
        let base = table("direct_scan_base", bytemuck::cast_slice(&base));
        let lamp = table("direct_scan_lamp", bytemuck::cast_slice(&lamp));
        let cmfs = table("direct_scan_cmfs", bytemuck::cast_slice(&cmfs));
        let buffers: [&dyn RasterStorage; 7] =
            [&uniform, source, &dye, &base, &lamp, &cmfs, output];
        use wgpu::BufferBindingType::{Storage as S, Uniform as U};
        self.scan_pass(
            concat!(
                include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
                include_str!("../../../spektrafilm-shaders/wgsl/scan/direct_spectral.wgsl")
            ),
            &[
                U,
                S { read_only: true },
                S { read_only: true },
                S { read_only: true },
                S { read_only: true },
                S { read_only: true },
                S { read_only: false },
            ],
            &buffers,
            &[(1, false), (6, true)],
            width * height,
        )
    }
    pub(super) fn build_convert_inverse_state(
        &self,
        p: &ConvertSolverGpuParams<'_>,
        src: &(impl RasterStorage + ?Sized),
        dst: &(impl RasterStorage + ?Sized),
        n: u32,
    ) -> Result<ConvertInverseState, String> {
        if !self.device.features().contains(wgpu::Features::SHADER_F64) {
            return Err("convert-film inverse scan requires native shader f64".into());
        }
        let wl = p.n_wl as usize;
        if wl == 0
            || p.dye.len() < wl
            || p.illuminant.len() < wl
            || p.cmfs.len() < wl
            || !p.normalization.is_finite()
            || p.normalization == 0.0
        {
            return Err("convert-film inverse scan has invalid spectral coefficients".into());
        }
        // Missing spectral rows produce nonfinite light for every bounded CMY
        // and the CPU excludes them before integration. Compact those rows
        // once; uploading NaNs relies on driver fast-math NaN comparisons and
        // can contaminate the normal equations before their final clamp.
        let active: Vec<_> = (0..wl)
            .filter(|&i| {
                !p.base.get(i).copied().unwrap_or(0.0).is_nan()
                    && !p.dye[i].iter().any(|value| value.is_nan())
                    && p.illuminant[i].is_finite()
            })
            .collect();
        if active.is_empty() {
            return Err("convert-film inverse scan has no contributing spectral rows".into());
        }
        let dyes: Vec<_> = active.iter().map(|&i| p.dye[i]).collect();
        let bases: Vec<_> = active
            .iter()
            .map(|&i| p.base.get(i).copied().unwrap_or(0.0))
            .collect();
        let lamps: Vec<_> = active.iter().map(|&i| p.illuminant[i]).collect();
        let matching: Vec<_> = active.iter().map(|&i| p.cmfs[i]).collect();
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct Uniform {
            header: [u32; 4],
            gain: f64,
            normalization: f64,
            seed_log_rgb: [f64; 4],
            seed_inverse: [[f64; 4]; 3],
            xyz_to_rgb: [[f64; 4]; 3],
            calibration: [[f64; 4]; 3],
            cmy_max: [f64; 4],
        }
        let row = |v: [f64; 3]| [v[0], v[1], v[2], 0.0];
        let uniform = Uniform {
            header: [
                n,
                active.len() as u32,
                8,
                if p.decode_input { p.cctf_kind } else { 0 },
            ],
            gain: p.gain,
            normalization: p.normalization,
            seed_log_rgb: row(p.seed_log_rgb),
            seed_inverse: p.seed_inverse.map(row),
            xyz_to_rgb: p.xyz_to_rgb.map(row),
            calibration: p.calibration.map(row),
            cmy_max: row(p.cmy_max),
        };
        let table = |label, bytes: &[u8]| {
            self.pipe
                .static_buffer(label, bytes, wgpu::BufferUsages::STORAGE)
        };
        let uniform = self.pipe.static_buffer(
            "convert_params",
            bytemuck::bytes_of(&uniform),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_SRC,
        );
        let dye = table("convert_dye", bytemuck::cast_slice(&dyes));
        let base = table("convert_base", bytemuck::cast_slice(&bases));
        let lamp = table("convert_lamp", bytemuck::cast_slice(&lamps));
        let cmfs = table("convert_cmfs", bytemuck::cast_slice(&matching));
        let source = src.image_pages();
        let destination = dst.image_pages();
        let buffers: [&dyn RasterStorage; 7] =
            [&uniform, &source, &dye, &base, &lamp, &cmfs, &destination];
        use wgpu::BufferBindingType::{Storage as S, Uniform as U};
        Ok(ConvertInverseState {
            pass: self.scan_pass(
                concat!(
                    include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
                    include_str!("../../../spektrafilm-shaders/wgsl/cctf_lib.wgsl"),
                    include_str!(
                        "../../../spektrafilm-shaders/wgsl/scan/convert_inverse_solver.wgsl"
                    )
                ),
                &[
                    U,
                    S { read_only: true },
                    S { read_only: true },
                    S { read_only: true },
                    S { read_only: true },
                    S { read_only: true },
                    S { read_only: false },
                ],
                &buffers,
                &[(1, false), (6, true)],
                n,
            )?,
        })
    }
    pub(super) fn build_scan_density_state(
        &self,
        p: &ScannerDensityGpuParams,
        src: &(impl RasterStorage + ?Sized),
        dst: &(impl RasterStorage + ?Sized),
        width: u32,
        height: u32,
        preparation_key: Option<&std::sync::Arc<dyn spektrafilm_backend::DeviceImagePayload>>,
    ) -> Result<ScanDensityState, String> {
        self.check_cancelled()?;
        if !self
            .device
            .features()
            .contains(wgpu::Features::SHADER_F64 | wgpu::Features::SHADER_INT64)
        {
            return Err(
                "negative-to-positive acquisition requires native shader f64 and int64".into(),
            );
        }
        let pixels = width
            .checked_mul(height)
            .filter(|&n| n != 0)
            .ok_or("standard setup requires a nonempty RGB density frame")?;
        if p.counts
            .iter()
            .enumerate()
            .any(|(c, &n)| n < 2 || p.offsets[c] as usize + n as usize > p.density.len())
            || p.exposure.len() != p.density.len()
        {
            return Err("negative-to-positive acquisition has invalid calibrated axes".into());
        }
        if p.standard_setup
            && (!p.setup_aim_density.is_finite()
                || !p.setup_gamma.is_finite()
                || p.setup_gamma <= 0.0)
        {
            return Err(
                "standard setup requires a finite positive neutral-curve slope at 16%".into(),
            );
        }
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct Uniform {
            header: [u32; 4],
            offsets: [u32; 4],
            counts: [u32; 4],
            flags: [u32; 4],
            contrast: [f64; 4],
            dmax: [f64; 4],
            brightness: [f64; 4],
            denominator: [f64; 4],
            camera: [f64; 4],
            setup: [f64; 4],
            operator: [f64; 4],
            color: [f64; 4],
            destination: [[f64; 4]; 3],
        }
        let row = |v: [f64; 3]| [v[0], v[1], v[2], 0.0];
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let operator_active = p.operator_contrast_delta != 0.0
            || p.operator_exposure_ev != 0.0
            || p.operator_color.iter().any(|&v| v != 0.0);
        let mut uniform = Uniform {
            header: [pixels, 0, 0, pixels],
            offsets: [p.offsets[0], p.offsets[1], p.offsets[2], 0],
            counts: [p.counts[0], p.counts[1], p.counts[2], 0],
            flags: [
                u32::from(p.standard_setup),
                u32::from(operator_active),
                u32::from(p.tone_to_output != identity),
                0,
            ],
            contrast: row(p.contrast),
            dmax: row(p.dmax),
            brightness: row(p.brightness_ev),
            denominator: row(p.denominator),
            camera: [
                if p.standard_setup { 0.0 } else { p.camera_ev },
                0.0,
                0.0,
                0.0,
            ],
            setup: [p.setup_aim_density, p.setup_gamma, 0.0, 0.0],
            operator: [
                p.operator_pivot,
                p.operator_contrast_delta,
                p.operator_exposure_ev,
                0.0,
            ],
            color: row(p.operator_color),
            destination: p.tone_to_output.map(row),
        };
        let table = |label, bytes: &[u8]| {
            self.pipe
                .static_buffer(label, bytes, wgpu::BufferUsages::STORAGE)
        };
        use wgpu::BufferBindingType::{Storage as S, Uniform as U};
        let status = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("scan_density_status"),
                contents: bytemuck::bytes_of(&0u32),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });
        let density = table("density_axes", bytemuck::cast_slice(&p.density));
        let exposure = table("exposure_axes", bytemuck::cast_slice(&p.exposure));
        let limit = super::bounded::storage_binding_limit(self)?;
        if density.size() > limit || exposure.size() > limit {
            return Err(format!(
                "scanner calibrated coefficient binding exceeds {limit} bytes"
            ));
        }
        if limit < 112 {
            return Err(format!(
                "scanner setup working state needs 112 bytes per binding; limit is {limit}"
            ));
        }
        let capacity = (super::bounded::chunk_elements(limit, 24)? as u32).min(pixels);
        let cached = preparation_key
            .and_then(|key| self.cached_prepared_scan(key, p, width, height))
            .filter(|prepared| {
                prepared
                    .exposure
                    .iter()
                    .all(|page| page.buffer.size() <= limit)
            });
        let reuse = cached.is_some();
        let prepared = if let Some(prepared) = cached {
            prepared
        } else {
            let mut pages = Vec::new();
            let mut first = 0;
            while first < pixels {
                self.check_cancelled()?;
                let n = capacity.min(pixels - first);
                let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("scan_reconstructed_exposure"),
                    size: u64::from(n) * 24,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                pages.push(super::paged::ImagePage {
                    first_pixel: first,
                    n_pixels: n,
                    buffer,
                });
                first += n;
            }
            let setup = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("scan_fullframe_setup"),
                size: 112,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            std::sync::Arc::new(PreparedScan {
                exposure: pages,
                setup,
                status: status.clone(),
            })
        };
        let source = src.image_pages();
        let destination = dst.image_pages();
        let all_params = self.pipe.static_buffer(
            "density_params",
            bytemuck::bytes_of(&uniform),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_SRC,
        );
        let setup_pass = if p.standard_setup && !reuse {
            Some(scan_setup::build(
                self,
                &source,
                &all_params,
                &density,
                &exposure,
                &status,
                &prepared.setup,
                pixels,
            )?)
        } else {
            None
        };
        let optics = if p.lens_blur > 0.0 && !reuse {
            let make = |inverse: u32| {
                self.scan_pass(
                    concat!(
                        include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
                        include_str!("../../../spektrafilm-shaders/wgsl/scan/transmission.wgsl")
                    ),
                    &[U, S { read_only: false }],
                    &[
                        &self.pipe.static_buffer(
                            "transmission_params",
                            bytemuck::cast_slice(&[pixels, inverse, 0, 0]),
                            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_SRC,
                        ),
                        &source,
                    ],
                    &[(1, true)],
                    pixels,
                )
            };
            let blur = build_grain_blur_paged(
                self,
                width,
                height,
                &source,
                &destination,
                [f64::from(p.lens_blur); 3],
                0,
                0,
            )?;
            let mut restore = RasterCommands::default();
            restore.copy(&destination, 0, &source, 0, u64::from(pixels) * 12);
            Some((make(0)?, blur, restore, make(1)?))
        } else {
            None
        };
        let render_pipeline = self.cached_pipeline(
            concat!(
                include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
                include_str!("../../../spektrafilm-shaders/wgsl/scan/density_lib.wgsl"),
                include_str!("../../../spektrafilm-shaders/wgsl/scan/scanner_density.wgsl")
            ),
            &[
                U,
                S { read_only: true },
                S { read_only: true },
                S { read_only: true },
                S { read_only: false },
                S { read_only: false },
            ],
        );
        let reconstruct_pipeline = self.cached_pipeline(
            concat!(
                include_str!("../../../spektrafilm-shaders/wgsl/scan/density_lib.wgsl"),
                include_str!("../../../spektrafilm-shaders/wgsl/scan/density_reconstruct.wgsl")
            ),
            &[
                U,
                S { read_only: true },
                S { read_only: true },
                S { read_only: true },
                S { read_only: false },
                S { read_only: false },
                S { read_only: true },
            ],
        );
        let window = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scan_rgb_window"),
            size: u64::from(capacity) * 12,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut pass = RasterCommands::default();
        let mut reconstruct = RasterCommands::default();
        for page in &prepared.exposure {
            self.check_cancelled()?;
            uniform.header[0] = page.n_pixels;
            let params = self.pipe.static_buffer(
                "scan_page_params",
                bytemuck::bytes_of(&uniform),
                wgpu::BufferUsages::UNIFORM,
            );
            if !reuse {
                reconstruct.copy(
                    &source,
                    u64::from(page.first_pixel) * 12,
                    &window,
                    0,
                    u64::from(page.n_pixels) * 12,
                );
                reconstruct.dispatch(
                    &reconstruct_pipeline,
                    &[
                        &params,
                        &window,
                        &density,
                        &exposure,
                        &page.buffer,
                        &status,
                        &prepared.setup,
                    ],
                    page.n_pixels,
                    false,
                    self,
                );
            }
            pass.dispatch(
                &render_pipeline,
                &[&params, &page.buffer, &density, &exposure, &window, &status],
                page.n_pixels,
                false,
                self,
            );
            pass.copy(
                &window,
                0,
                &destination,
                u64::from(page.first_pixel) * 12,
                u64::from(page.n_pixels) * 12,
            );
        }
        Ok(ScanDensityState {
            pass,
            reconstruct: (!reuse).then_some(reconstruct),
            setup_pass,
            optics,
            status,
            prepared,
        })
    }
}
