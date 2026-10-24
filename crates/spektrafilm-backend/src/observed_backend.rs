use crate::telemetry::*;
use crate::*;
/// Bind explicit attribution without replacing backend behavior or resources.
pub fn bind_backend<'a>(
    backend: &'a dyn ComputeBackend,
    context: ObservationContext,
) -> Box<dyn ComputeBackend + 'a> {
    if let Some(bound) = backend.with_observation_context(context.clone()) {
        return bound;
    }
    if backend.is_gpu() {
        context.set_backend_selected(BackendSelected::Wgpu);
        if let Some(a) = backend.adapter_description() {
            context.set_adapter(a);
        }
    } else {
        context.set_backend_selected(BackendSelected::Cpu);
    }
    Box::new(ObservedBackend {
        backend,
        context,
        cpu_reason: None,
    })
}

pub fn bind_cpu_fallback(
    context: ObservationContext,
    reason: CpuReason,
) -> Box<dyn ComputeBackend> {
    Box::new(ObservedBackend {
        backend: &cpu_backend::CpuBackend,
        context,
        cpu_reason: Some(reason),
    })
}
struct ObservedBackend<'a> {
    backend: &'a dyn ComputeBackend,
    context: ObservationContext,
    cpu_reason: Option<CpuReason>,
}
impl ObservedBackend<'_> {
    fn cpu(&self) {
        if !self.backend.is_gpu() {
            self.context.record_executor(
                Executor::Cpu,
                Some(self.cpu_reason.unwrap_or(CpuReason::CpuSelected)),
            );
        }
    }
}
impl ComputeBackend for ObservedBackend<'_> {
    fn preparation_device_id(&self) -> Option<u64> {
        self.backend.preparation_device_id()
    }
    fn prepare_spectral_tc_lut(
        &self,
        p: &SpectralTcLutParams<'_>,
    ) -> Result<Option<PreparedTable>, String> {
        self.backend.prepare_spectral_tc_lut(p)
    }
    fn prepare_gamut_table(&self, p: &GamutTableParams) -> Result<Option<PreparedTable>, String> {
        self.backend.prepare_gamut_table(p)
    }
    fn remap_input_gamut_table(
        &self,
        source: &PreparedTable,
        p: &InputGamutTableParams<'_>,
    ) -> Result<PreparedTable, String> {
        self.backend.remap_input_gamut_table(source, p)
    }
    fn observation_context(&self) -> Option<&ObservationContext> {
        Some(&self.context)
    }
    fn with_observation_context(
        &self,
        c: ObservationContext,
    ) -> Option<Box<dyn ComputeBackend + '_>> {
        Some(Box::new(ObservedBackend {
            backend: self.backend,
            context: c,
            cpu_reason: self.cpu_reason,
        }))
    }
    fn with_cancellation(
        &self,
        cancellation: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Option<Box<dyn ComputeBackend + '_>> {
        let backend = self.backend.with_cancellation(cancellation)?;
        // Device backends preserve their observation context when deriving.
        Some(backend)
    }
    fn name(&self) -> &str {
        self.backend.name()
    }
    fn is_gpu(&self) -> bool {
        self.backend.is_gpu()
    }
    /// An explicit request must keep its meaning through this wrapper: the
    /// assembly guards read this flag, so a wrapped explicit backend must not
    /// look automatic.
    fn explicit_gpu_request(&self) -> bool {
        self.backend.explicit_gpu_request()
    }
    /// The route preflight reads the binding limits through this wrapper, so
    /// a wrapped backend must report the same limits as the backend it
    /// observes.
    fn image_binding_limit_reason(&self, img: &ImageBuf) -> Option<String> {
        self.backend.image_binding_limit_reason(img)
    }
    fn raw_process_sensor(
        &self,
        sensor: &RawSensor,
        params: &RawGpuParams,
    ) -> Result<Option<ImageBuf>, String> {
        self.backend.raw_process_sensor(sensor, params)
    }
    fn raw_process_sensor_retained(
        &self,
        sensor: &RawSensor,
        params: &RawGpuParams,
    ) -> Result<Option<DeviceImage>, String> {
        self.backend.raw_process_sensor_retained(sensor, params)
    }
    fn colorspace_convert(&self, i: &ImageBuf, m: &[[f32; 3]; 3]) -> Result<ImageBuf, String> {
        self.cpu();
        self.backend.colorspace_convert(i, m)
    }
    fn cctf_encode_srgb(&self, i: &ImageBuf) -> Result<ImageBuf, String> {
        self.cpu();
        self.backend.cctf_encode_srgb(i)
    }
    fn cctf_decode_srgb(&self, i: &ImageBuf) -> Result<ImageBuf, String> {
        self.cpu();
        self.backend.cctf_decode_srgb(i)
    }
    fn gaussian_blur(&self, i: &ImageBuf, s: f32) -> Result<ImageBuf, String> {
        self.cpu();
        self.backend.gaussian_blur(i, s)
    }
    fn gaussian_blur_multi(&self, i: &ImageBuf, s: &[f32]) -> Result<Vec<ImageBuf>, String> {
        self.cpu();
        self.backend.gaussian_blur_multi(i, s)
    }
    fn table_lookup(&self, i: &ImageBuf, x: &[f32], y: &[[f32; 3]]) -> Result<ImageBuf, String> {
        self.cpu();
        self.backend.table_lookup(i, x, y)
    }
    fn lut3d_interp(&self, i: &ImageBuf, l: &Lut3D) -> Result<ImageBuf, String> {
        self.cpu();
        self.backend.lut3d_interp(i, l)
    }
    fn enlarger_spectral_lut(
        &self,
        image: &ImageBuf,
        params: &crate::EnlargerSpectralLutGpuParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        self.cpu();
        self.backend.enlarger_spectral_lut(image, params)
    }
    fn scanner_spectral_lut(
        &self,
        image: &ImageBuf,
        params: &crate::ScannerSpectralLutGpuParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        self.cpu();
        self.backend.scanner_spectral_lut(image, params)
    }
    fn scan_spectral(
        &self,
        i: &ImageBuf,
        c: &[[f64; 3]],
        b: &[f64],
        l: &[f64],
        n: f64,
        a: &[[f64; 3]; 3],
        m: &[[f64; 3]; 3],
    ) -> Result<ImageBuf, String> {
        self.cpu();
        self.backend.scan_spectral(i, c, b, l, n, a, m)
    }
    fn scan_spectral_with_cmfs(
        &self,
        i: &ImageBuf,
        c: &[[f64; 3]],
        b: &[f64],
        l: &[f64],
        cm: &[[f64; 3]],
        n: f64,
        a: &[[f64; 3]; 3],
        m: &[[f64; 3]; 3],
    ) -> Result<ImageBuf, String> {
        self.cpu();
        self.backend
            .scan_spectral_with_cmfs(i, c, b, l, cm, n, a, m)
    }
    fn print_spectral(
        &self,
        i: &ImageBuf,
        c: &[[f64; 3]],
        b: &[f64],
        l: &[f64],
        s: &[[f64; 3]],
        n: f64,
        p: [f64; 3],
    ) -> Result<ImageBuf, String> {
        self.cpu();
        self.backend.print_spectral(i, c, b, l, s, n, p)
    }
    fn hanatos2025_rgb_to_raw(
        &self,
        i: &ImageBuf,
        l: &spektrafilm_math::spectral::TcLut,
        c: &str,
        r: &[f32],
        a: bool,
    ) -> Result<ImageBuf, String> {
        self.cpu();
        self.backend.hanatos2025_rgb_to_raw(i, l, c, r, a)
    }
    fn density_curve_interp(
        &self,
        i: &ImageBuf,
        x: &[f64],
        y: &[[f64; 3]],
        g: f64,
    ) -> Result<ImageBuf, String> {
        self.cpu();
        self.backend.density_curve_interp(i, x, y, g)
    }
    fn grain_v2(&self, i: &ImageBuf, p: &GrainV2GpuParams) -> Result<Option<ImageBuf>, String> {
        self.cpu();
        self.backend.grain_v2(i, p)
    }
    fn dye_field(
        &self,
        i: &ImageBuf,
        p: &crate::dye_params::DyeGpuParams,
    ) -> Option<Result<ImageBuf, String>> {
        self.backend.dye_field(i, p)
    }
    fn dye_field_with_response(
        &self,
        image: &ImageBuf,
        params: &crate::DyeGpuParams,
        response: &crate::DyeResponseGpuParams<'_>,
    ) -> Option<Result<ImageBuf, String>> {
        self.backend
            .dye_field_with_response(image, params, response)
    }
    fn dye_aperture_collapse(
        &self,
        image: &ImageBuf,
        factor: u32,
    ) -> Option<Result<ImageBuf, String>> {
        self.backend.dye_aperture_collapse(image, factor)
    }
    fn dye_log10(&self, image: &ImageBuf) -> Option<Result<ImageBuf, String>> {
        self.backend.dye_log10(image)
    }
    fn dye_crop(&self, image: &ImageBuf, rect: [u32; 4]) -> Option<Result<ImageBuf, String>> {
        self.backend.dye_crop(image, rect)
    }
    fn fotufilm_grain(
        &self,
        i: &ImageBuf,
        p: &crate::FotufilmGpuParams,
    ) -> Option<Result<ImageBuf, String>> {
        self.backend.fotufilm_grain(i, p)
    }
    fn try_run_film_chain(
        &self,
        input: &DeviceImage,
        p: &FilmChainParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        self.backend.try_run_film_chain(input, p)
    }
    fn retain_image(&self, i: &ImageBuf, m: &RetainedImageMeta) -> Result<DeviceImage, String> {
        self.backend.retain_image(i, m)
    }
    fn image_payload<'a>(
        &'a self,
        i: &'a DeviceImage,
    ) -> Result<&'a (dyn DeviceImagePayload + 'static), String> {
        self.backend.image_payload(i)
    }
    fn materialize_retained(&self, i: &DeviceImage) -> Result<ImageBuf, String> {
        self.backend.materialize_retained(i)
    }
    fn input_geometry(
        &self,
        i: &DeviceImage,
        p: &InputGeometryGpuParams,
    ) -> Result<Option<DeviceImage>, String> {
        self.backend.input_geometry(i, p)
    }
    fn try_run_film_chain_retained(
        &self,
        i: &DeviceImage,
        p: &FilmChainParams<'_>,
    ) -> Result<Option<DeviceImage>, String> {
        self.backend.try_run_film_chain_retained(i, p)
    }
    fn meter_autoexposure_retained(
        &self,
        i: &DeviceImage,
        p: &MeterAutoexposureParams,
    ) -> Result<Option<f64>, String> {
        self.backend.meter_autoexposure_retained(i, p)
    }
    fn scale_retained(&self, i: &DeviceImage, scale: f32) -> Result<Option<DeviceImage>, String> {
        self.backend.scale_retained(i, scale)
    }
    fn convert_colorspace_retained(
        &self,
        i: &DeviceImage,
        p: &ConvertColorspaceGpuParams,
        dst: &'static spektrafilm_math::colorspace::RgbColorSpace,
        domain: SampleDomain,
    ) -> Result<Option<DeviceImage>, String> {
        self.backend.convert_colorspace_retained(i, p, dst, domain)
    }
    fn cmyk_retained(
        &self,
        i: &DeviceImage,
        p: &CmykGpuParams,
    ) -> Result<Option<DeviceImage>, String> {
        self.backend.cmyk_retained(i, p)
    }
    fn mix_output_retained(
        &self,
        r: &DeviceImage,
        s: &DeviceImage,
        p: &OutputMixGpuParams,
    ) -> Result<Option<DeviceImage>, String> {
        self.backend.mix_output_retained(r, s, p)
    }
    fn viewer_fit_retained(
        &self,
        i: &DeviceImage,
        w: u32,
        h: u32,
        d: Option<&DisplayTransformGpuParams>,
        c: Option<&ViewerIccGpuParams<'_>>,
    ) -> Result<DeviceImage, String> {
        self.backend.viewer_fit_retained(i, w, h, d, c)
    }
    fn viewer_full_source_retained(
        &self,
        i: &DeviceImage,
        p: &ViewerFullSourceGpuParams<'_>,
    ) -> Result<DeviceImage, String> {
        self.backend.viewer_full_source_retained(i, p)
    }
    fn viewer_compose_retained(
        &self,
        i: Option<&DeviceImage>,
        paper: Option<&DeviceImage>,
        previous: Option<&DeviceImage>,
        split: Option<&DeviceImage>,
        p: &ViewerComposeGpuParams,
    ) -> Result<Vec<[u8; 4]>, String> {
        self.backend
            .viewer_compose_retained(i, paper, previous, split, p)
    }
    fn viewer_paper_retained(
        &self,
        w: u32,
        h: u32,
        logo: &[f32],
        lw: u32,
        lh: u32,
    ) -> Result<DeviceImage, String> {
        self.backend.viewer_paper_retained(w, h, logo, lw, lh)
    }
    fn viewer_polaroid_retained(
        &self,
        i: &DeviceImage,
        c: [f32; 9],
    ) -> Result<DeviceImage, String> {
        self.backend.viewer_polaroid_retained(i, c)
    }
    fn inspect_retained_pixel(&self, i: &DeviceImage, x: u32, y: u32) -> Result<[f32; 3], String> {
        self.backend.inspect_retained_pixel(i, x, y)
    }
    fn retained_histogram(&self, image: &DeviceImage) -> Result<[[f32; 64]; 3], String> {
        self.backend.retained_histogram(image)
    }
    fn viewer_icc_retained(
        &self,
        i: &DeviceImage,
        p: &ViewerIccGpuParams<'_>,
    ) -> Result<DeviceImage, String> {
        self.backend.viewer_icc_retained(i, p)
    }
    /// The viewer's disposable raster work runs on the device when the bound
    /// backend has a device path; the wrapper only forwards the request, so a
    /// wrapped device backend does not silently lose its viewer kernels.
    fn viewer_resample(
        &self,
        i: &ImageBuf,
        p: &ViewerResampleGpuParams,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        self.backend.viewer_resample(i, p)
    }
    fn viewer_display_transform(
        &self,
        rgb: &[[f32; 3]],
        p: &DisplayTransformGpuParams,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        self.backend.viewer_display_transform(rgb, p)
    }
    /// Saving conversion forwards unchanged, so a wrapped device backend
    /// keeps its sample kernel instead of silently returning the trait
    /// default through `bind_backend`.
    fn convert_retained_samples(
        &self,
        i: &DeviceImage,
        p: &OutputSampleGpuParams,
    ) -> Result<Option<OutputSampleData>, String> {
        self.backend.convert_retained_samples(i, p)
    }
    fn convert_image_samples(
        &self,
        i: &ImageBuf,
        p: &OutputSampleGpuParams,
    ) -> Result<Option<OutputSampleData>, String> {
        self.backend.convert_image_samples(i, p)
    }
    fn viewer_icc_transform(
        &self,
        rgb: &[[f32; 3]],
        p: &ViewerIccGpuParams<'_>,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        self.backend.viewer_icc_transform(rgb, p)
    }
    fn viewer_full_source(
        &self,
        i: &ImageBuf,
        p: &ViewerFullSourceGpuParams<'_>,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        self.backend.viewer_full_source(i, p)
    }
}
