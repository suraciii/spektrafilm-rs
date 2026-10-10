use crate::telemetry::*;
use crate::*;
/// Bind explicit attribution without replacing backend behavior or resources.
pub fn bind_backend<'a>(
    backend: &'a dyn ComputeBackend,
    context: ObservationContext,
) -> Box<dyn ComputeBackend + 'a> {
    if backend.is_gpu() {
        context.set_backend_selected(BackendSelected::Wgpu);
        if let Some(a) = backend.adapter_description() {
            context.set_adapter(a);
        }
    } else {
        context.set_backend_selected(BackendSelected::Cpu);
    }
    backend
        .with_observation_context(context.clone())
        .unwrap_or_else(|| Box::new(ObservedBackend { backend, context }))
}
struct ObservedBackend<'a> {
    backend: &'a dyn ComputeBackend,
    context: ObservationContext,
}
impl ObservedBackend<'_> {
    fn cpu(&self) {
        if !self.backend.is_gpu() {
            self.context.set_backend_selected(BackendSelected::Cpu);
            self.context
                .record_executor(Executor::Cpu, Some(CpuReason::CpuSelected));
        }
    }
}
impl ComputeBackend for ObservedBackend<'_> {
    fn observation_context(&self) -> Option<&ObservationContext> {
        Some(&self.context)
    }
    fn with_observation_context(
        &self,
        c: ObservationContext,
    ) -> Option<Box<dyn ComputeBackend + '_>> {
        Some(bind_backend(self.backend, c))
    }
    fn name(&self) -> &str {
        self.backend.name()
    }
    fn is_gpu(&self) -> bool {
        self.backend.is_gpu()
    }
    fn colorspace_convert(&self, i: &ImageBuf, m: &[[f32; 3]; 3]) -> ImageBuf {
        self.cpu();
        self.backend.colorspace_convert(i, m)
    }
    fn cctf_encode_srgb(&self, i: &ImageBuf) -> ImageBuf {
        self.cpu();
        self.backend.cctf_encode_srgb(i)
    }
    fn cctf_decode_srgb(&self, i: &ImageBuf) -> ImageBuf {
        self.cpu();
        self.backend.cctf_decode_srgb(i)
    }
    fn gaussian_blur(&self, i: &ImageBuf, s: f32) -> ImageBuf {
        self.cpu();
        self.backend.gaussian_blur(i, s)
    }
    fn gaussian_blur_multi(&self, i: &ImageBuf, s: &[f32]) -> Vec<ImageBuf> {
        self.cpu();
        self.backend.gaussian_blur_multi(i, s)
    }
    fn table_lookup(&self, i: &ImageBuf, x: &[f32], y: &[[f32; 3]]) -> ImageBuf {
        self.cpu();
        self.backend.table_lookup(i, x, y)
    }
    fn lut3d_interp(&self, i: &ImageBuf, l: &Lut3D) -> ImageBuf {
        self.cpu();
        self.backend.lut3d_interp(i, l)
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
    ) -> ImageBuf {
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
    ) -> ImageBuf {
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
    ) -> ImageBuf {
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
    ) -> ImageBuf {
        self.cpu();
        self.backend.hanatos2025_rgb_to_raw(i, l, c, r, a)
    }
    fn density_curve_interp(&self, i: &ImageBuf, x: &[f64], y: &[[f64; 3]], g: f64) -> ImageBuf {
        self.cpu();
        self.backend.density_curve_interp(i, x, y, g)
    }
    fn grain_v2(&self, i: &ImageBuf, p: &GrainV2GpuParams) -> Option<ImageBuf> {
        self.cpu();
        self.backend.grain_v2(i, p)
    }
    fn try_run_film_chain(&self, p: &FilmChainParams<'_>) -> Option<ImageBuf> {
        self.backend.try_run_film_chain(p)
    }
}
