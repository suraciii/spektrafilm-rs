//! Compute backend abstraction and its default CPU implementations.
//!
//! This crate owns the `ComputeBackend` trait, the CPU reference
//! implementation, the shared observation vocabulary, and the
//! device-independent parameter records that cross the trait boundary. It
//! depends on no device toolkit, so domain code can reach the CPU reference
//! implementation without compiling device code.

pub mod dye_params;
use dye_params::DyeGpuParams;
pub mod raw_params;
pub use raw_params::{RawDemosaic, RawGpuParams, RawLensParams, RawSensor, RawSensorLayout};
pub mod telemetry;
pub use telemetry::{
    AdapterDescription, CpuReason, Executor, Observation, ObservationContext, ObservationKind,
};
pub mod device_image;
mod observed_backend;
pub use device_image::{
    DeviceImage, DeviceImagePayload, Precision, RetainedImageMeta, SampleDomain,
};
pub mod timing;
pub use observed_backend::{bind_backend, bind_cpu_fallback};
pub mod cpu_backend;
pub mod scalar_scan;
pub mod viewer_params;
pub use viewer_params::ViewerComposeGpuParams;
pub mod preparation;
pub use preparation::{
    GamutTableParams, InputGamutTableParams, PreparedTable, SpectralTcLutParams,
};

/// CPU stage closure type shared by all pipe fence fields.
pub type CpuStageFn<'a> = Box<dyn Fn(&ImageBuf) -> Result<ImageBuf, String> + 'a>;
use spektrafilm_math::image::ImageBuf;

/// The `cctf_kind` discriminant every device transfer shader takes
/// (`cctf_lib.wgsl`). It lives here, next to the parameter structs that carry
/// it, because it is part of the device ABI: the shaders and the host must
/// agree on one mapping, and more than one caller needs it.
pub fn cctf_kind(cctf: spektrafilm_math::colorspace::Cctf) -> u32 {
    use spektrafilm_math::colorspace::Cctf;
    match cctf {
        Cctf::Linear => 0,
        Cctf::Acescct => 1,
        Cctf::Srgb => 2,
        Cctf::ProPhoto => 3,
        Cctf::Rec2020 => 4,
        Cctf::AdobeRgb1998 => 5,
        Cctf::Gamma2_6 => 6,
    }
}

/// f32 boundary representation shared by the WGPU Grain Procedural shader and the
/// core parameter mapper. The field groups mirror the shader's 64-byte
/// uniform layout.
#[derive(Debug, Clone, Copy)]
pub struct GrainV2GpuParams {
    pub mode: u32,
    /// Host Film Resolution implementation: 0 = OpticalResolution, 1 = FastBlur.
    pub resolution_type: u32,
    pub amount: f32,
    pub shadows: f32,
    pub midtones: f32,
    pub highlights: f32,
    pub raw_scale: f32,
    pub cluster_size: f32,
    pub rotation: f32,
    pub color: f32,
    pub resolution_factor: f32,
    pub seed: u32,
    /// Explicit host timer; `None` derives the canonical phase from `seed`.
    pub timer: Option<f32>,
    pub colored: bool,
    pub clustered: bool,
}

pub struct Lut3D {
    pub size: u32,
    pub data: Vec<f32>,
}

/// Compute backend abstraction. Each method corresponds to a GPU-friendly
/// operation in the film simulation pipeline.
///
/// Default implementations fall back to CPU. GPU backends override the
/// spectral methods for massive speedups.
pub trait ComputeBackend: Send + Sync {
    /// Generate and retain the spectral exposure table on the selected device.
    fn prepare_spectral_tc_lut(
        &self,
        _params: &SpectralTcLutParams<'_>,
    ) -> Result<Option<PreparedTable>, String> {
        Ok(None)
    }
    /// Upload a host-computed spectral exposure table for devices that cannot
    /// carry the reference contraction. Bounded, image-independent tables may
    /// be resolved on the host in reference precision.
    fn prepare_spectral_tc_lut_from_host(
        &self,
        _table: &[f64],
        _size: u32,
        _neutral: [f64; 3],
    ) -> Result<Option<PreparedTable>, String> {
        Ok(None)
    }
    /// Bisect and retain the destination-specific perceptual gamut boundary.
    fn prepare_gamut_table(
        &self,
        _params: &GamutTableParams,
    ) -> Result<Option<PreparedTable>, String> {
        Ok(None)
    }
    /// Upload a host-computed output gamut boundary for devices without
    /// native binary64. Bounded, image-independent tables may be resolved on
    /// the host in reference precision.
    fn prepare_gamut_table_from_host(
        &self,
        _table: &[f64],
        _n_l: u32,
        _n_h: u32,
    ) -> Result<Option<PreparedTable>, String> {
        Ok(None)
    }
    fn remap_input_gamut_table(
        &self,
        _source: &PreparedTable,
        _params: &InputGamutTableParams<'_>,
    ) -> Result<PreparedTable, String> {
        Err("selected backend cannot prepare input gamut remapping".to_owned())
    }
    fn observation_context(&self) -> Option<&telemetry::ObservationContext> {
        None
    }
    fn with_observation_context(
        &self,
        _context: telemetry::ObservationContext,
    ) -> Option<Box<dyn ComputeBackend + '_>> {
        None
    }
    /// Derive a backend sharing resources but observing one operation's signal.
    /// `None` clears a previous signal; unsupported backends return `None`.
    fn with_cancellation(
        &self,
        _cancellation: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Option<Box<dyn ComputeBackend + '_>> {
        None
    }
    fn adapter_description(&self) -> Option<telemetry::AdapterDescription> {
        None
    }
    /// Identity used to invalidate cached device-generated calibration resources.
    fn preparation_device_id(&self) -> Option<u64> {
        None
    }
    fn colorspace_convert(
        &self,
        img: &ImageBuf,
        matrix: &[[f32; 3]; 3],
    ) -> Result<ImageBuf, String>;
    fn cctf_encode_srgb(&self, img: &ImageBuf) -> Result<ImageBuf, String>;
    fn cctf_decode_srgb(&self, img: &ImageBuf) -> Result<ImageBuf, String>;
    fn gaussian_blur(&self, img: &ImageBuf, sigma: f32) -> Result<ImageBuf, String>;

    /// Blur `img` with each of `sigmas`, returning one image per sigma.
    /// GPU backends fuse the work into a single command buffer (one upload,
    /// one submit, one readback) — much cheaper than N independent
    /// `gaussian_blur` calls when N > 1. Default implementation calls the
    /// per-sigma kernel in a loop.
    fn gaussian_blur_multi(&self, img: &ImageBuf, sigmas: &[f32]) -> Result<Vec<ImageBuf>, String> {
        sigmas.iter().map(|&s| self.gaussian_blur(img, s)).collect()
    }
    fn table_lookup(
        &self,
        img: &ImageBuf,
        table_x: &[f32],
        table_y: &[[f32; 3]],
    ) -> Result<ImageBuf, String>;
    fn lut3d_interp(&self, img: &ImageBuf, lut: &Lut3D) -> Result<ImageBuf, String>;

    /// Spectral scanning: density CMY → RGB via spectral integration.
    /// GPU backends override this with a compute shader.
    ///
    /// The `cat` and `xyz_to_rgb` matrices are kept separate so the CPU
    /// path can apply them as two sequential matmuls (matching Python's
    /// `colour.XYZ_to_RGB` step-by-step). Pre-combining loses ~1 ULP per
    /// output channel which compounds to ~5e-6 of drift in the bare
    /// chain. GPU may collapse for performance.
    #[allow(clippy::too_many_arguments)]
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
        if self.is_gpu() {
            return Err("scan_spectral: selected GPU has no device implementation".into());
        }
        let _observation = self.observation_context().map(|context| {
            let scope = context.scope(
                "scan_spectral",
                telemetry::ObservationKind::Stage,
                context.purpose(),
            );
            scope.context().record_executor(
                telemetry::Executor::Cpu,
                Some(telemetry::CpuReason::BackendDefault),
            );
            scope
        });
        Ok(cpu_backend::scan_spectral_cpu(
            density_cmy,
            channel_density,
            base_density,
            illuminant,
            normalization,
            cat,
            xyz_to_rgb,
        ))
    }

    /// Runtime-grid scan. A selected GPU must implement the full grid;
    /// the CPU reference preserves all runtime CMF samples.
    #[allow(clippy::too_many_arguments)]
    fn scan_spectral_with_cmfs(
        &self,
        density_cmy: &ImageBuf,
        channel_density: &[[f64; 3]],
        base_density: &[f64],
        illuminant: &[f64],
        cmfs: &[[f64; 3]],
        normalization: f64,
        cat: &[[f64; 3]; 3],
        xyz_to_rgb: &[[f64; 3]; 3],
    ) -> Result<ImageBuf, String> {
        if self.is_gpu() {
            return Err(
                "scan_spectral_with_cmfs: selected GPU has no device implementation".into(),
            );
        }
        let _observation = self.observation_context().map(|context| {
            let scope = context.scope(
                "scan_spectral",
                telemetry::ObservationKind::Stage,
                context.purpose(),
            );
            scope.context().record_executor(
                telemetry::Executor::Cpu,
                Some(telemetry::CpuReason::BackendDefault),
            );
            scope
        });
        Ok(cpu_backend::scan_spectral_cpu_with_cmfs(
            density_cmy,
            channel_density,
            base_density,
            illuminant,
            cmfs,
            normalization,
            cat,
            xyz_to_rgb,
        ))
    }

    /// Spectral printing: film density CMY → print log-exposure via spectral integration.
    /// GPU backends override this with a compute shader.
    #[allow(clippy::too_many_arguments)]
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
        if self.is_gpu() {
            return Err("print_spectral: selected GPU has no device implementation".into());
        }
        let _observation = self.observation_context().map(|context| {
            let scope = context.scope(
                "print_spectral",
                telemetry::ObservationKind::Stage,
                context.purpose(),
            );
            scope.context().record_executor(
                telemetry::Executor::Cpu,
                Some(telemetry::CpuReason::BackendDefault),
            );
            scope
        });
        Ok(cpu_backend::print_spectral_cpu(
            density_cmy,
            channel_density,
            base_density,
            illuminant,
            sensitivity,
            normalization_factor,
            preflash,
        ))
    }

    /// Hanatos2025 RGB → film raw exposure (the per-pixel bicubic LUT lookup).
    ///
    /// CPU implementation does the two-step CAT02 adaptation (RGB → native
    /// XYZ, then CAT02 adapt) per pixel for bit-exact Python parity —
    /// fusing both matrices into a single matmul gives a different ULP than
    /// applying them sequentially. GPU backends override and may collapse
    /// the two into a single matrix per the precision budget for live preview.
    fn hanatos2025_rgb_to_raw(
        &self,
        image: &ImageBuf,
        tc_lut: &spektrafilm_math::spectral::TcLut,
        color_space: &str,
        ref_illuminant: &[f32],
        cat16: bool,
    ) -> Result<ImageBuf, String> {
        if self.is_gpu() {
            return Err("hanatos2025_rgb_to_raw: selected GPU has no device implementation".into());
        }
        let _observation = self.observation_context().map(|context| {
            let scope = context.scope(
                "rgb_to_raw",
                telemetry::ObservationKind::Stage,
                context.purpose(),
            );
            scope.context().record_executor(
                telemetry::Executor::Cpu,
                Some(telemetry::CpuReason::BackendDefault),
            );
            scope
        });
        Ok(spektrafilm_math::spectral::hanatos2025_rgb_to_raw(
            image,
            tc_lut,
            color_space,
            ref_illuminant,
            cat16,
        ))
    }

    /// log_raw → density_cmy via per-channel curve interpolation.
    ///
    /// Called twice per render (filming.develop + printing.develop). GPU
    /// backends override with `density_curve_interp.wgsl`; CPU falls back to
    /// the f64 curve interpolation in `spektrafilm-model::density_curves`.
    fn density_curve_interp(
        &self,
        log_raw: &ImageBuf,
        log_exposure: &[f64],
        density_curves: &[[f64; 3]],
        gamma_factor: f64,
    ) -> Result<ImageBuf, String> {
        if self.is_gpu() {
            return Err("density_curve_interp: selected GPU has no device implementation".into());
        }
        let _observation = self.observation_context().map(|context| {
            let scope = context.scope(
                "density_curve_interp",
                telemetry::ObservationKind::Stage,
                context.purpose(),
            );
            scope.context().record_executor(
                telemetry::Executor::Cpu,
                Some(telemetry::CpuReason::BackendDefault),
            );
            scope
        });
        // CPU fallback — uses the f64 reference (`fast_interp_image_f64`).
        // Scalar gamma is broadcast to all channels; we just stretch the x-axis once.
        let scaled: Vec<f64> = if (gamma_factor - 1.0).abs() < 1e-12 {
            log_exposure.to_vec()
        } else {
            log_exposure.iter().map(|&v| v / gamma_factor).collect()
        };
        Ok(spektrafilm_math::interp::fast_interp_image_f64(
            log_raw,
            &scaled,
            density_curves,
        ))
    }

    /// Return a rendered Grain Procedural image when this backend supports the shader.
    /// Input and output are native encoded RGB in the caller's color space;
    /// Grain performs no transfer-function or primaries conversion. Input preparation
    /// and final composition round to binary16 while sampling and noise use f32.
    /// Only a CPU-selected caller may use the independent reference on None.
    /// A selected device reports dispatch/resource failures through Err.
    fn grain_v2(
        &self,
        _image: &ImageBuf,
        _params: &GrainV2GpuParams,
    ) -> Result<Option<ImageBuf>, String> {
        Ok(None)
    }

    /// Film-grain Fotufilm residual readout on the device: form the pinned
    /// crystal bank, measure its pitch statistics, and sample the grain
    /// residual against the developed CMY density in `params.grid`.
    ///
    /// Every coefficient of the formation law is resolved on the host from the
    /// anchored population (`Population::device_records`) and uploaded as
    /// bounded records; the device draws the coating counts, crystal marks and
    /// depositions itself, so no image-dependent field is prepared on the CPU
    /// and no stochastic population data crosses the boundary. `None` marks a
    /// backend without a device Fotufilm path, so the caller keeps its host
    /// readout and records the fallback; `Some` marks an attempted device run
    /// whose error is a real failure to surface before publication, never a
    /// fallback trigger.
    fn fotufilm_grain(
        &self,
        _image: &ImageBuf,
        _params: &crate::FotufilmGpuParams,
    ) -> Option<Result<ImageBuf, String>> {
        None
    }

    /// Film-grain Dye field readout on the device: realize the conditional
    /// dye field from the resolved model tables in `params` and read it out
    /// to `params.grid`. `None` marks a backend without a device field path,
    /// so the caller keeps its host field and records the fallback; `Some`
    /// marks an attempted device run whose error is a real failure to
    /// surface before publication, never a fallback trigger.
    fn dye_field(
        &self,
        _image: &ImageBuf,
        _params: &crate::DyeGpuParams,
    ) -> Option<Result<ImageBuf, String>> {
        None
    }

    fn dye_field_with_response(
        &self,
        _image: &ImageBuf,
        _params: &DyeGpuParams,
        _response: &DyeResponseGpuParams<'_>,
    ) -> Option<Result<ImageBuf, String>> {
        None
    }
    fn dye_aperture_collapse(
        &self,
        _image: &ImageBuf,
        _factor: u32,
    ) -> Option<Result<ImageBuf, String>> {
        None
    }
    fn dye_log10(&self, _image: &ImageBuf) -> Option<Result<ImageBuf, String>> {
        None
    }
    fn dye_crop(&self, _image: &ImageBuf, _rect: [u32; 4]) -> Option<Result<ImageBuf, String>> {
        None
    }

    /// Upload host samples as one retained computation image.
    ///
    /// This is the single upload a host-decoded input makes: every later
    /// stage consumes the returned carrier, so no stage downloads and
    /// reuploads it to cross a runtime, stage, saving-conversion, or viewer
    /// boundary. `meta` describes the samples; the samples themselves keep
    /// their precision, which a device pass narrows to f32 at the boundary
    /// and reports through [`DeviceImage::precision`].
    fn retain_image(
        &self,
        _image: &ImageBuf,
        _meta: &RetainedImageMeta,
    ) -> Result<DeviceImage, String> {
        Err("backend does not retain device images".to_owned())
    }

    /// Borrow a retained carrier's backend payload for a device pass.
    ///
    /// `Err` reports a carrier this backend cannot consume: host samples, a
    /// foreign backend, or another device of the same backend. A stage that
    /// needs the samples must surface that error rather than fall back to
    /// host arithmetic (spec gpu-execution §Residency and resources).
    /// The payload type is `Any` and therefore `'static`; only this borrow
    /// is tied to the backend and image, not the backend receiver itself.
    fn image_payload<'a>(
        &'a self,
        _image: &'a DeviceImage,
    ) -> Result<&'a (dyn DeviceImagePayload + 'static), String> {
        Err("backend does not retain device images".to_owned())
    }

    /// Explicit host materialization of retained samples: the final handoff
    /// for a native writer, a diagnostic tap, or a CPU reference comparison.
    ///
    /// Called by [`DeviceImage::to_host`], never by a stage in the middle of
    /// a route.
    fn materialize_retained(&self, _image: &DeviceImage) -> Result<ImageBuf, String> {
        Err("backend cannot materialize retained samples".to_owned())
    }

    /// Working geometry on retained samples: rotation, exact pixel crop, and
    /// resize, in that order, on the compute device.
    ///
    /// The host resolves the destination geometry, the crop rectangle, and
    /// the working pixel pitch, so one authoritative rule decides the raster
    /// and a device pass cannot disagree with the runtime about it. The
    /// kernel reproduces the host reference exactly: `Spline3` matches
    /// `spektrafilm_math::resize::rescale_spline3` (cubic prefilter, Gaussian
    /// antialiasing, mirror extension, half-pixel centers, global range clip
    /// from a device-side min/max reduction) and `PreviewTriangle` matches
    /// the preview reference without clipping.
    ///
    /// `Ok(None)` reports that this backend has no device geometry, and the
    /// caller keeps the host reference; an explicit GPU request must fail
    /// instead of running geometry on the host. `Err` is a device failure to
    /// surface before output publication.
    fn input_geometry(
        &self,
        _image: &DeviceImage,
        _params: &InputGeometryGpuParams,
    ) -> Result<Option<DeviceImage>, String> {
        Ok(None)
    }

    /// Scale every retained sample by an image-independent exposure factor on
    /// its owning device. This is the device equivalent of the input exposure
    /// multiply performed by `apply_autoexposure`; it preserves the sample
    /// domain and metadata and never materializes the image.
    fn scale_retained(
        &self,
        _image: &DeviceImage,
        _scale: f32,
    ) -> Result<Option<DeviceImage>, String> {
        Ok(None)
    }

    /// Convert retained samples between RGB spaces and transfer states on
    /// their owning device. The returned carrier carries the destination
    /// primaries and whether its samples are encoded.
    fn convert_colorspace_retained(
        &self,
        _image: &DeviceImage,
        _params: &ConvertColorspaceGpuParams,
        _destination: &'static spektrafilm_math::colorspace::RgbColorSpace,
        _domain: SampleDomain,
    ) -> Result<Option<DeviceImage>, String> {
        Ok(None)
    }

    /// Apply the terminal CMYK ink style to a retained image on its owning
    /// device. The input-only route ends with this appearance.
    fn cmyk_retained(
        &self,
        _image: &DeviceImage,
        _params: &CmykGpuParams,
    ) -> Result<Option<DeviceImage>, String> {
        Ok(None)
    }

    /// Blend two retained images on their owning device. Both carriers must
    /// belong to the same backend/device and have identical geometry.
    fn mix_output_retained(
        &self,
        _result: &DeviceImage,
        _source: &DeviceImage,
        _params: &OutputMixGpuParams,
    ) -> Result<Option<DeviceImage>, String> {
        Ok(None)
    }

    /// The resident film chain, returned as host samples — the compatibility
    /// wrapper around [`Self::try_run_film_chain_retained`].
    ///
    /// It runs the identical stage sequence and materializes the result once,
    /// at this boundary. A production route calls the retained entry instead,
    /// so the samples stay on the device for the output mix, the saving
    /// conversion, and the viewer. `Ok(None)` selects the per-stage route;
    /// `Err` reports a fence that could not calibrate its output or a device
    /// failure.
    fn try_run_film_chain(
        &self,
        input: &DeviceImage,
        params: &FilmChainParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        match self.try_run_film_chain_retained(input, params)? {
            Some(retained) => Ok(Some(self.materialize_retained(&retained)?)),
            None => Ok(None),
        }
    }
    /// The resident film chain, returning its output as a retained carrier.
    ///
    /// This is the production entry point: the samples the chain produced
    /// stay on the device, so the route that called it can hand the same
    /// carrier to the output mix, the saving conversion, and the viewer
    /// without a host materialization or a reupload in between. The only
    /// difference from [`Self::try_run_film_chain`] is the returned
    /// ownership: the stage sequence, the parameters, the fences, and the
    /// failure policy are identical.
    ///
    /// `Ok(None)` selects the host-materializing route; `Err` reports a
    /// fence that could not calibrate its output or a device failure.
    fn try_run_film_chain_retained(
        &self,
        _input: &DeviceImage,
        _params: &FilmChainParams<'_>,
    ) -> Result<Option<DeviceImage>, String> {
        Ok(None)
    }

    /// Full-source auto-exposure reduction over an image already resident on
    /// this backend. The returned EV is a bounded control scalar; no input
    /// samples cross to the host.
    fn meter_autoexposure_retained(
        &self,
        _image: &DeviceImage,
        _params: &MeterAutoexposureParams,
    ) -> Result<Option<f64>, String> {
        Ok(None)
    }

    /// Measure the auto-exposure EV for `image` on the compute device.
    ///
    /// Full-source metering is a reduction, and a reduction is image
    /// arithmetic: a device route must not downscale, luminance-convert, and
    /// meter the input on the host. `Ok(None)` reports that this backend has
    /// no device meter, and the caller keeps the reference meter. `Ok(Some)`
    /// returns the EV the reference meter would return for the same request,
    /// computed on the device.
    fn meter_autoexposure(
        &self,
        _image: &ImageBuf,
        _params: &MeterAutoexposureParams,
    ) -> Result<Option<f64>, String> {
        Ok(None)
    }

    /// Blend the processed result with the prepared input source on the
    /// compute device, returning the mixed result.
    ///
    /// The final mix is image arithmetic, so a device route must not blend it
    /// on the host. Both operands are in the output primaries; `encoded`
    /// states whether they carry the output transfer already. `Ok(None)`
    /// reports that this backend has no device mixer, and the caller keeps the
    /// reference implementation.
    fn mix_output(
        &self,
        _result: &ImageBuf,
        _source: &ImageBuf,
        _params: &OutputMixGpuParams,
    ) -> Result<Option<ImageBuf>, String> {
        Ok(None)
    }

    /// Convert `image` from the source colour space to the destination colour
    /// space, with the requested transfer decoding and encoding, on the
    /// compute device.
    ///
    /// The input-only route runs no film stage, so this conversion is its
    /// entire numerical content. `Ok(None)` reports that this backend has no
    /// device conversion, and the caller keeps the reference implementation.
    fn convert_colorspace(
        &self,
        _image: &ImageBuf,
        _params: &ConvertColorspaceGpuParams,
    ) -> Result<Option<ImageBuf>, String> {
        Ok(None)
    }

    /// Convert retained final samples into the native writer's sample type.
    ///
    /// The source color space and linear/encoded domain are carried by the
    /// image; this request selects only the destination encoding and writer
    /// representation. Device conversion includes output clipping for integer
    /// formats and the requested integer quantization. Its returned typed
    /// samples are the final writer handoff and must not be converted again.
    fn convert_retained_samples(
        &self,
        _image: &DeviceImage,
        _params: &OutputSampleGpuParams,
    ) -> Result<Option<OutputSampleData>, String> {
        Ok(None)
    }

    /// Convert host samples into the native writer's sample type. This is the
    /// reference implementation for CPU execution and host-only backends.
    fn convert_image_samples(
        &self,
        _image: &ImageBuf,
        _params: &OutputSampleGpuParams,
    ) -> Result<Option<OutputSampleData>, String> {
        Ok(None)
    }

    /// Prepare disposable fitted viewing samples without materializing the source.
    /// `None` display means raw output-space viewing; ICC follows the display pass.
    fn viewer_fit_retained(
        &self,
        _image: &DeviceImage,
        _width: u32,
        _height: u32,
        _display: Option<&DisplayTransformGpuParams>,
        _icc: Option<&ViewerIccGpuParams<'_>>,
    ) -> Result<DeviceImage, String> {
        Err("backend has no retained viewer preparation".into())
    }
    /// Transform source taps before interpolation for a magnified view.
    fn viewer_full_source_retained(
        &self,
        _image: &DeviceImage,
        _params: &ViewerFullSourceGpuParams<'_>,
    ) -> Result<DeviceImage, String> {
        Err("backend has no retained full-source viewer".into())
    }
    /// Native presentation boundary: all sampling/blending runs on device;
    /// only packed RGBA bytes cross to the frontend. An absent image composes
    /// the empty canvas, without sampling a previous layer.
    fn viewer_compose_retained(
        &self,
        _image: Option<&DeviceImage>,
        _paper: Option<&DeviceImage>,
        _previous: Option<&DeviceImage>,
        _params: &ViewerComposeGpuParams,
    ) -> Result<Vec<[u8; 4]>, String> {
        Err("backend has no retained viewer composition".into())
    }
    /// Pinned virtual-paper synthesis from the decoded static watermark asset.
    fn viewer_paper_retained(
        &self,
        _width: u32,
        _height: u32,
        _logo: &[f32],
        _logo_width: u32,
        _logo_height: u32,
    ) -> Result<DeviceImage, String> {
        Err("backend has no device virtual-paper synthesis".into())
    }
    /// Nine-layer reveal, including full-image density maxima, on the device.
    fn viewer_polaroid_retained(
        &self,
        _image: &DeviceImage,
        _coefficients: [f32; 9],
    ) -> Result<DeviceImage, String> {
        Err("backend has no device viewer reveal".into())
    }

    /// Evaluate a parsed display-profile pipeline without host image samples.
    fn viewer_icc_retained(
        &self,
        _image: &DeviceImage,
        _params: &ViewerIccGpuParams<'_>,
    ) -> Result<DeviceImage, String> {
        Err("backend has no retained ICC evaluation".into())
    }
    /// Inspect one retained source pixel; a bounded readback, never a display raster.
    fn inspect_retained_pixel(
        &self,
        _image: &DeviceImage,
        _x: u32,
        _y: u32,
    ) -> Result<[f32; 3], String> {
        Err("backend has no retained pixel inspection".into())
    }
    /// Display-referred sRGB tonal bins, normalized together to a peak of one.
    /// Only these 192 reduced values cross the retained-image boundary.
    fn retained_histogram(&self, _image: &DeviceImage) -> Result<[[f32; 64]; 3], String> {
        Err("backend has no retained histogram reduction".into())
    }
    /// Resample `image` to the viewer's capped size on the compute device,
    /// returning the separable-bilinear display raster as flat `f32` RGB.
    ///
    /// The viewer's image-sized reduction is host arithmetic, so a device
    /// route must not reduce it on the host. The returned pixels are
    /// disposable viewer data: they are never a retained or saved sample, and
    /// the source image is never modified. `Ok(None)` reports that this
    /// backend has no device resampler, and the caller keeps the reference
    /// implementation.
    fn viewer_resample(
        &self,
        _image: &ImageBuf,
        _params: &ViewerResampleGpuParams,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        Ok(None)
    }

    /// Transform already-resampled native viewer samples into encoded sRGB
    /// viewing pixels on the compute device.
    ///
    /// This is the viewer's display transform, not a saving conversion: the
    /// result is disposable viewer data, never a retained or saved sample.
    /// `Ok(None)` reports that this backend has no device display transform,
    /// and the caller keeps the reference implementation.
    fn viewer_display_transform(
        &self,
        _rgb: &[[f32; 3]],
        _params: &DisplayTransformGpuParams,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        Ok(None)
    }

    /// Apply the selected display ICC profile to encoded sRGB viewing pixels
    /// on the compute device, returning the profile-converted viewing pixels.
    ///
    /// This is the viewer's ICC display conversion, not a saving conversion:
    /// the source is always encoded sRGB, the destination is the selected
    /// profile, and the result is disposable viewer data that is never a
    /// retained or saved sample. The request carries immutable, image-independent
    /// LittleCMS RGB8 Perceptual preparation. Fixed kernels apply identity,
    /// channel tables, the pinned Q14 matrix-shaper evaluator, or an exhaustive
    /// RGB8-domain map. The map contains every input code, not an interpolated
    /// approximation. Input clamps to 0..=1 and truncates to RGB8; NaN maps to
    /// zero. Output is the reference RGB8 result divided by 255.
    /// `Ok(None)` reports an unavailable operation. A selected GPU caller must
    /// fail instead of evaluating source-image ICC pixels on the host.
    fn viewer_icc_transform(
        &self,
        _rgb: &[[f32; 3]],
        _params: &ViewerIccGpuParams<'_>,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        Ok(None)
    }

    /// Compose the viewer's magnified full-source view on the compute device,
    /// returning the bounded viewport raster as flat `f32` RGB.
    ///
    /// The magnified view samples the full-resolution source at every visible
    /// viewport pixel, so reducing it on the host is image arithmetic on the
    /// host: a device route must not download the source to resample it. The
    /// request carries the viewport geometry, the interpolation mode, and the
    /// resolved display and ICC transforms, so the device evaluates the same
    /// per-pixel arithmetic the host composition does. The returned pixels are
    /// disposable viewer data and the source image is never modified.
    /// `Ok(None)` reports that this backend has no device composition, and
    /// the caller keeps the reference implementation.
    fn viewer_full_source(
        &self,
        _image: &ImageBuf,
        _params: &ViewerFullSourceGpuParams<'_>,
    ) -> Result<Option<Vec<[f32; 3]>>, String> {
        Ok(None)
    }

    /// Convolve `image` with a per-channel discrete PSF on the compute
    /// device, returning the blurred image.
    ///
    /// The optical diffusion filters (camera and enlarger) convolve a
    /// discrete, energy-normalised PSF sampled in image-plane pixels with
    /// reflect boundary extension. The kernel is a general 2-D tap
    /// convolution, so a device port must reproduce the exact tap weights
    /// and the exact reflect extension; it must not substitute a Gaussian
    /// approximation or truncate the kernel to a device radius cap.
    /// `Ok(None)` reports that this backend has no device convolution, and
    /// the caller keeps the reference implementation.
    fn optical_diffusion(
        &self,
        _image: &ImageBuf,
        _params: &OpticalDiffusionGpuParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        Ok(None)
    }

    /// Generate, prepare and evaluate the enlarger lattice without intermediate readback.
    fn enlarger_spectral_lut(
        &self,
        _image: &ImageBuf,
        _params: &EnlargerSpectralLutGpuParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        Ok(None)
    }

    /// Generate, prepare and evaluate log-XYZ, then apply the scanner colour transforms on device.
    fn scanner_spectral_lut(
        &self,
        _image: &ImageBuf,
        _params: &ScannerSpectralLutGpuParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        Ok(None)
    }

    /// Evaluate the prepared lattice using the reference Hermite walk.
    fn enlarger_lut_interp(
        &self,
        _image: &ImageBuf,
        _params: &EnlargerLutGpuParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
        Ok(None)
    }

    /// Run the RAW sensor pipeline on this backend: black subtraction, white
    /// balance scaling, demosaic, colour assignment and orientation, the
    /// stages LibRaw's `dcraw_process` performs after unpacking.
    ///
    /// The sensor record and the image-independent plan come from
    /// `spektrafilm-raw`; the measured sensor maximum is resolved here because
    /// it is image arithmetic. `Ok(None)` means this backend has no device RAW
    /// pipeline, and an explicit GPU request treats the missing route as an
    /// error rather than silently running the host reference.
    fn raw_process_sensor(
        &self,
        _sensor: &crate::RawSensor,
        _params: &crate::RawGpuParams,
    ) -> Result<Option<ImageBuf>, String> {
        Ok(None)
    }

    /// Run RAW numerical stages and keep the result on its producing device.
    fn raw_process_sensor_retained(
        &self,
        _sensor: &crate::RawSensor,
        _params: &crate::RawGpuParams,
    ) -> Result<Option<DeviceImage>, String> {
        Ok(None)
    }

    /// True for GPU compute backends.
    fn is_gpu(&self) -> bool {
        false
    }

    /// True when the user explicitly requested this GPU backend (the CLI
    /// `--backend gpu` flag, `SPEKTRAFILM_BACKEND=wgpu`, or the GUI export
    /// "GPU" selection). An explicit request must not run a
    /// capability-missing stage on the CPU silently: the operation fails
    /// at assembly instead of demoting the route.
    fn explicit_gpu_request(&self) -> bool {
        false
    }

    /// A reason why a single-image binding for `img` would exceed the
    /// device's buffer-binding limits, or `None` when the image fits. The
    /// auto route declines such a stage to its CPU implementation; an
    /// explicit GPU request fails the operation instead.
    fn image_binding_limit_reason(&self, _img: &ImageBuf) -> Option<String> {
        None
    }

    /// Derive this backend as an explicit GPU request, sharing the device,
    /// queue, pipeline cache, and resident pipe, or `None` when this backend
    /// cannot serve a GPU request. A frontend that maps an explicit user
    /// selection onto an already selected backend uses this instead of
    /// opening a second device.
    fn with_explicit_request(&self) -> Option<Box<dyn ComputeBackend>> {
        None
    }

    fn name(&self) -> &str;
}

/// All inputs to the GPU-resident film chain. Bundled into a struct so the
/// trait method stays object-safe and adding more optional stages (halation,
/// DIR couplers, …) doesn't churn every callsite.
/// Optical response averaged over the realized Dye field cells.
pub enum DyeResponseGpuParams<'a> {
    Print {
        channel_density: &'a [[f64; 3]],
        base_density: &'a [f64],
        illuminant: &'a [f64],
        sensitivity: &'a [[f64; 3]],
        normalization_factor: f64,
        exposure_scale: f64,
        preflash: [f64; 3],
    },
    Scan {
        channel_density: &'a [[f64; 3]],
        base_density: &'a [f64],
        illuminant: &'a [f64],
        normalization: f64,
        xyz_to_rgb: &'a [[f64; 3]; 3],
    },
}

/// Named pipeline boundary — the Rust spelling of upstream `Tap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tap {
    RgbIn,
    RgbPre,
    LogEFilm,
    CmyFilm,
    LogEPrint,
    CmyPrint,
    RgbOut,
}

impl Tap {
    /// Parse a wire tap name. Errors mirror upstream's "unknown tap" and
    /// "no node path" failures: the message lists every valid name so a
    /// typo in a params file is immediately actionable.
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "rgb_in" => Ok(Self::RgbIn),
            "rgb_pre" => Ok(Self::RgbPre),
            "log_e_film" => Ok(Self::LogEFilm),
            "cmy_film" => Ok(Self::CmyFilm),
            "log_e_print" => Ok(Self::LogEPrint),
            "cmy_print" => Ok(Self::CmyPrint),
            "rgb_out" => Ok(Self::RgbOut),
            other => Err(format!(
                "unknown tap {other:?}: valid taps are rgb_in, rgb_pre, log_e_film, \
                 cmy_film, log_e_print, cmy_print, rgb_out"
            )),
        }
    }

    /// The boundary's position in the chain, counted from the input side. A
    /// route that skips a stage removes it instead of renumbering the rest, so
    /// positions stay comparable across topologies.
    pub const fn position(self) -> u8 {
        match self {
            Self::RgbIn => 0,
            Self::RgbPre => 1,
            Self::LogEFilm => 2,
            Self::CmyFilm => 3,
            Self::LogEPrint => 4,
            Self::CmyPrint => 5,
            Self::RgbOut => 6,
        }
    }

    /// The wire name (upstream `Tap` attribute values).
    pub fn name(self) -> &'static str {
        match self {
            Self::RgbIn => "rgb_in",
            Self::RgbPre => "rgb_pre",
            Self::LogEFilm => "log_e_film",
            Self::CmyFilm => "cmy_film",
            Self::LogEPrint => "log_e_print",
            Self::CmyPrint => "cmy_print",
            Self::RgbOut => "rgb_out",
        }
    }
}
pub struct FilmChainParams<'a> {
    /// Inclusive input and output boundaries, using the application's
    /// authoritative pipeline topology. No stage outside this range runs.
    pub inject: Tap,
    pub collect: Tap,
    /// Working raster geometry after the retained input-geometry stage.
    pub image_width: u32,
    pub image_height: u32,
    /// Working pixel pitch of that raster, in microns. Carried so the output
    /// carrier states the geometry every stage consumed, and no later stage
    /// re-derives it from dimensions.
    pub pixel_pitch_um: f64,
    /// RGB → film-raw front pass (the configured upsampler).
    pub front: FrontPass<'a>,
    pub film_log_exposure: &'a [f64],
    pub film_density_curves_normalized: &'a [[f64; 3]],
    pub film_gamma: f64,
    pub film_channel_density: &'a [[f64; 3]],
    pub film_base_density: &'a [f64],
    pub print_illuminant: &'a [f64],
    pub print_sensitivity: &'a [[f64; 3]],
    pub print_normalization_factor: f64,
    /// Linear print exposure gain after the inner floor and logarithm.
    pub print_exposure_scale: f64,
    pub print_log_exposure: &'a [f64],
    pub print_density_curves: &'a [[f64; 3]],
    pub print_gamma: f64,
    pub print_channel_density: &'a [[f64; 3]],
    pub print_base_density: &'a [f64],
    /// Constant enlarger preflash raw 3-vector, added to the print exposure
    /// before the inner log10 (Python `_compute_raw_preflash`). `[0; 3]` off.
    pub preflash: [f64; 3],
    /// Selected spectral acceleration: generate and prepare the enlarger lattice on device.
    pub enlarger_lut: Option<EnlargerSpectralLutGpuParams<'a>>,
    /// Selected scanner log-XYZ lattice, including its ordered output transforms.
    pub scanner_lut: Option<ScannerSpectralLutGpuParams<'a>>,
    pub scan_illuminant: &'a [f64],
    pub scan_normalization: f64,
    /// Precombined response used only by the exact-cell Dye scan readout.
    pub scan_xyz_to_rgb: &'a [[f64; 3]; 3],
    /// Ordered CPU scanner colour transforms (not precombined).
    pub scan_cat: &'a [[f64; 3]; 3],
    pub scan_base_xyz_to_rgb: &'a [[f64; 3]; 3],
    /// Scan the developed film directly instead of printing onto paper.
    /// When true the print_spectral + print density-curve passes are
    /// skipped and scan_spectral runs on the film density buffer using the
    /// film's `film_channel_density` / `film_base_density` (the caller sets
    /// `scan_illuminant`, `scan_normalization`, `scan_cat`, and
    /// `scan_base_xyz_to_rgb` from the neutral scan context).
    pub scan_film: bool,
    /// Optional halation pass — when `Some`, inserted on the raw film
    /// exposure buffer between hanatos2025 and log10.
    pub halation: Option<HalationGpuParams>,
    /// Optional DIR (development inhibitor release) couplers pass — when
    /// `Some`, inserted on the film density buffer between filming and
    /// printing. Re-interpolates density curves using `density_curves_0`.
    pub dir_couplers: Option<DirCouplersGpuParams<'a>>,
    /// Optional viewing glare pass — applied after scan spectral on the
    /// final RGB buffer. Lognormal-distributed per-pixel surface noise +
    /// blur + per-channel illuminant offset.
    pub glare: Option<GlareGpuParams>,
    /// Optional output gamut compression pass — applied after glare on
    /// the final linear RGB buffer, before unsharp (matching CPU scanning).
    pub gamut: Option<GamutGpuParams<'a>>,
    /// Optional unsharp mask pass — applied after glare (last step
    /// before readback). Blur σ in pixels + amount scalar; both come
    /// from `scanner.unsharp_mask`.
    pub unsharp: Option<UnsharpGpuParams>,
    /// Optional Grain Procedural pass — applied after scanner effects, the destination
    /// same-space matrix roundtrip, and transfer encoding. Returns encoded RGB.
    pub grain_v2: Option<GrainV2GpuParams>,
    /// Destination space used to encode the resident Grain Procedural input.
    pub scan_output_space: &'static spektrafilm_math::colorspace::RgbColorSpace,
    /// Optional camera lens Gaussian blur on the raw film exposure buffer,
    /// before halation.
    pub camera_lens_blur_px: Option<f32>,
    /// Reference sampled PSF after camera highlight boost, before lens blur.
    pub camera_diffusion: Option<OpticalDiffusionGpuParams<'a>>,
    /// Reference sampled PSF on recovered/scaled linear print irradiance.
    pub enlarger_diffusion: Option<OpticalDiffusionGpuParams<'a>>,
    /// Scanner blur on linear acquisition before glare and gamut correction.
    pub scanner_lens_blur_px: Option<f32>,
    /// Optional highlight reconstruction on the raw film exposure buffer,
    /// immediately after the front pass and before optical scatter.
    pub highlight_boost: Option<HighlightBoostGpuParams>,
    /// Optional film-grain Dye field pass — the device realization of the
    /// Dye field on the film density buffer, in the same position as
    /// `density_cpu_stage` (after the film density curve and DIR, before
    /// printing). The pass resolves the conditional field from the model
    /// tables and reads it out to `grid`; the chain preflights its resources
    /// before any allocation and checks its bounded status before the output
    /// is published. A Dye request the device cannot realize keeps the host
    /// fence in `density_cpu_stage` instead.
    pub grain_dye: Option<DyeGpuParams>,
    /// Publish only this rectangle of the field, when the readout was planned
    /// around an enclosing extent.
    pub grain_dye_crop: Option<[u32; 4]>,
    pub grain_fotufilm: Option<FotufilmGpuParams>,
    /// Optional CPU stage on the film density buffer after the film density
    /// curve and DIR, before printing — the assembled fence for the host
    /// grain engines (Spektra, and Dye whenever the field has no device
    /// path; pipe architecture, capability class). The closure receives the
    /// materialized density_cmy image and returns the grained image, which
    /// is written back to the same device buffer.
    pub density_cpu_stage: Option<CpuStageFn<'a>>,
    /// Device record-density acquisition, including transmission-domain optics.
    pub scan_readout: Option<&'a ScannerDensityGpuParams>,
    /// Immutable retained film-density owner for prepared scanner acquisition.
    /// Only a CmyFilm injection may provide this identity; ordinary chains
    /// leave it absent. Backends retain the owner rather than keying mutable
    /// scratch buffers or unowned addresses. Operator and tone changes may
    /// reuse acquisition only while this owner and acquisition controls match.
    pub scan_preparation_key: Option<&'a std::sync::Arc<dyn DeviceImagePayload>>,
    /// Optional explicit host acquisition fence. Selected scanner LUTs use
    /// `scanner_lut`, and negative-to-positive acquisition uses `scan_readout`.
    /// Receives density and returns captured RGB.
    pub scan_acquisition_cpu_stage: Option<CpuStageFn<'a>>,
    /// Optional output transfer encoding/decoding pass on the final buffer
    /// after every scanner effect (pipe architecture, S4). Encode applies
    /// the stored same-space roundtrip matrix and the space's encoding
    /// transfer; decode restores linear for a Procedural-grain output when
    /// the caller requested no encoding.
    pub output_transfer: Option<OutputTransferGpuParams>,
    /// Optional CMYK ink style pass on the final buffer after the output
    /// transfer (pipe architecture, S4). The residual host finishing moves
    /// onto the device; the input state is encoded exactly when the caller
    /// requested output encoding.
    pub cmyk: Option<CmykGpuParams>,
    /// Input transfer decoding (pipe architecture: input preparation).
    /// When `Some`, the chain decodes the uploaded samples with the input
    /// space's CCTF as its first device pass, before the front transform, so
    /// no host image arithmetic prepares the input. `cctf_kind` uses the
    /// shared discriminant (0 Linear, 1 ACEScct, 2 sRGB, 3 ProPhoto,
    /// 4 Rec2020, 5 Adobe RGB (1998), 6 Gamma 2.6).
    pub input_decode: Option<InputDecodeGpuParams>,
    /// Optional device Spektra v1 grain pass on the film density buffer, in
    /// the same position as `density_cpu_stage` (after the film density curve
    /// and DIR, before printing). `None` when the request samples the scipy
    /// MT19937 regime, whose single sequential generator over the whole array
    /// is not a per-pixel mapping — those requests keep the host fence, so the
    /// device never runs a different noise law.
    pub grain_v1: Option<GrainV1GpuParams>,
}

/// Chain-head input transfer decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputDecodeGpuParams {
    /// Input colour space CCTF discriminant (see `input_decode`).
    pub cctf_kind: u32,
}

/// Device Spektra v1 grain on the film density CMY buffer, in the
/// density-fence position (after the film density curve and DIR, before
/// printing).
///
/// The model's `prepare_grain` resolves all density floors, adjusted maxima,
/// and particle counts from the profile curves and RMS controls. No host
/// image-dependent fields are generated. Three-slot coefficient rows retain
/// the profile carrier layout; only `layer_count` slots are active.
///
/// `curve_x` is the composite axis, negated for positive film, and
/// `curve_layers` is the matching fixed-three-slot layer tensor. Device
/// interpolation uses these f64 tables, then grain working arithmetic is f32.
/// Both tagged fast-statistics and sequential NumPy MT19937 sampling remain
/// resident, including continuation into NumPy microstructure normals.
#[derive(Debug, Clone, PartialEq)]
pub struct GrainV1GpuParams {
    /// Request seed shared by the plane MT19937 and tagged fast-statistics draws.
    pub base_seed: u64,
    /// Active layers in the fixed three-slot profile carrier (1..=3).
    pub layer_count: u32,
    /// Per-channel composite density floor.
    pub density_min: [f64; 3],
    /// Per-channel grain uniformity control.
    pub uniformity: [f64; 3],
    /// Per-sublayer density floor, row-major over sublayer.
    pub density_min_layers: [f64; 9],
    /// Per-sublayer density maximum (floor included), row-major over sublayer.
    pub density_max_adj: [f64; 9],
    /// Per-sublayer particle count per pixel, row-major over sublayer.
    pub n_particles: [f64; 9],
    /// Preserve the CPU's shared channel-zero stream for monochrome material.
    pub monochrome: bool,
    /// Sign of both the composite lookup axis and lookup query.
    pub lookup_sign: f32,
    /// Sublayer particle-field blur control, applied before summing layers.
    pub blur_dye_clouds: f64,
    /// Host input representation only; interpolation tables remain f64 and grain works in f32.
    pub reference_f64: bool,
    /// Micro-structure standard deviation in pixels (`micro_structure[1] ·
    /// 0.001 / pixel_size_um`); the clumping field is drawn only above 0.05.
    pub micro_sigma: f64,
    /// Micro-structure blur radius in pixels (`micro_structure[0] /
    /// pixel_size_um`).
    pub micro_blur_px: f64,
    /// Final grain blur σ in pixels.
    pub grain_blur: f64,
    /// Multiplicative unsharp mask σ in pixels.
    pub usm_sigma: f64,
    /// Multiplicative unsharp mask amount.
    pub usm_amount: f64,
    /// Composite curve x column, `knots` rows × 3 channels.
    pub curve_x: Vec<f64>,
    /// Layer curve tensor, `knots` rows × 3 sublayers × 3 channels.
    pub curve_layers: Vec<f64>,
    /// Preserves sequential NumPy draws instead of independent fast streams.
    pub mt19937_regime: bool,
}

/// One auto-exposure metering request.
///
/// Mirrors `spektrafilm_core::stages::filming::measure_autoexposure_ev`: the
/// device meter must reproduce the reference preview rule (nearest resample
/// with per-axis antialiasing), the luminance row of the input space's
/// RGB→XYZ matrix, the optional CCTF decode applied to the sampled preview,
/// the named metering pattern, and the mid-grey target.
#[derive(Debug, Clone, PartialEq)]
pub struct MeterAutoexposureParams {
    /// Input width in pixels.
    pub width: u32,
    /// Input height in pixels.
    pub height: u32,
    /// Long-edge cap of the metering preview (`256` in the reference).
    pub max_size: u32,
    /// Luminance row of the input space's RGB→XYZ matrix.
    pub y_row: [f64; 3],
    /// Input CCTF discriminant for the decode applied to the sampled
    /// preview, or `None` when the request does not decode the input.
    pub cctf_kind: Option<u32>,
    /// Metering pattern: `average`, `median`, `center_weighted`, `partial`,
    /// `matrix`, `multi_zone`, `highlight_weighted`, or any other name,
    /// which meters the mid-grey target (0 EV) exactly like the reference
    /// does.
    pub method: String,
    /// Mean luminance the meter maps to mid-grey.
    pub mid_grey: f64,
}

/// One final output-mix request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputMixGpuParams {
    /// Mix strength: 0 keeps the prepared source, 1 keeps the result.
    pub amount: f32,
    /// Whether the operands carry the output transfer's encoded samples.
    pub encoded: bool,
    /// Output colour space CCTF discriminant.
    pub cctf_kind: u32,
}

/// One source→destination colour-space conversion request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConvertColorspaceGpuParams {
    /// Source CCTF discriminant, used only when `decode` is set.
    pub src_cctf: u32,
    /// Destination CCTF discriminant, used only when `encode` is set.
    pub dst_cctf: u32,
    /// Whether to decode the source transfer before the matrix.
    pub decode: bool,
    /// Whether to encode the destination transfer after the matrix.
    pub encode: bool,
    /// Source→destination matrix.
    pub matrix: [[f32; 3]; 3],
}

/// One GUI viewer resample request: the long-edge cap the viewer applies to a
/// full-resolution image before it composes viewport pixels.
///
/// The resample is the viewer's only image-sized reduction, so a device route
/// must not compute it on the host. The kernel is separable bilinear over the
/// exact source rectangle each output pixel covers, matching the host
/// `capped_raster`; the returned pixels are disposable viewer data, never a
/// retained or saved sample.
///
/// The host owns the destination size: it derives the long-edge factor and the
/// rounded destination dimensions in f64, so one authoritative rule decides
/// how many pixels the viewer keeps, and a device backend cannot disagree
/// with the viewer about the raster geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewerResampleGpuParams {
    /// Source width in pixels.
    pub width: u32,
    /// Source height in pixels.
    pub height: u32,
    /// Destination width in pixels, computed host-side.
    pub out_width: u32,
    /// Destination height in pixels, computed host-side.
    pub out_height: u32,
}

/// One GUI viewer display-colour transform request: native image samples to
/// encoded sRGB viewing pixels.
///
/// This is the viewer's display transform (`display_rgb`), not the saving
/// conversion: the matrix is the source space's display matrix and the
/// destination transfer is always sRGB. The result is clamped to 0..=1 for the
/// bounded display surface, exactly as the host viewer does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayTransformGpuParams {
    /// Source CCTF discriminant, used only when `decode` is set.
    pub src_cctf: u32,
    /// Whether to decode the source transfer before the matrix.
    pub decode: bool,
    /// Source → encoded sRGB matrix.
    pub matrix: [[f32; 3]; 3],
    /// Whether to clamp the encoded result to 0..=1.
    pub clamp: bool,
}

/// Fixed RGB8 display-transform representations prepared by the host CMM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ViewerIccKind {
    Identity = 0,
    /// Three packed 256-byte channel tables.
    Curves = 1,
    /// Twelve Q14 matrix/offset words, 768 Q14 input-shaper words, then
    /// three packed 16385-byte output-shaper tables.
    MatrixShaper = 2,
    /// Exact RGB8-domain map, lexicographic RGB order, packed RGB bytes.
    /// Unlike a sampled 3D LUT, this representation has no interpolation error.
    ExactRgb8 = 3,
}

impl ViewerIccKind {
    pub fn word_count(self) -> usize {
        match self {
            Self::Identity => 1,
            Self::Curves => 3 * 256 / 4,
            Self::MatrixShaper => 12 + 3 * 256 + (3 * 16385usize).div_ceil(4),
            Self::ExactRgb8 => 3 * 256 * 256 * 256 / 4,
        }
    }

    pub fn from_id(id: u32) -> Option<Self> {
        match id {
            0 => Some(Self::Identity),
            1 => Some(Self::Curves),
            2 => Some(Self::MatrixShaper),
            3 => Some(Self::ExactRgb8),
            _ => None,
        }
    }
}

/// Immutable, image-independent LittleCMS preparation for fixed GPU application.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewerIccGpuParams<'a> {
    /// Content identity of the profiles, conversion semantics, CMM and format.
    pub identity: [u8; 32],
    pub kind: ViewerIccKind,
    pub words: &'a [u32],
}

/// One GUI viewer magnified full-source composition request: the bounded
/// viewport raster the magnified view produces from the full-resolution
/// source.
///
/// The magnified view samples the source at every visible viewport pixel, so
/// this is the viewer's per-pixel composition rather than a reduction. The
/// request carries the viewport geometry the host already resolved, the
/// interpolation mode, and the resolved display and ICC transforms, so the
/// device evaluates the same per-pixel arithmetic the host composition does.
/// A viewport pixel outside the image rectangle stays transparent, exactly as
/// the host leaves it.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewerFullSourceGpuParams<'a> {
    /// Viewport texture size in pixels.
    pub tex_width: u32,
    /// Viewport texture size in pixels.
    pub tex_height: u32,
    /// Viewport rectangle minimum, in points.
    pub rect_min: [f32; 2],
    /// Viewport rectangle size, in points.
    pub rect_size: [f32; 2],
    /// Image rectangle minimum, in points.
    pub image_min: [f32; 2],
    /// Image rectangle size, in points.
    pub image_size: [f32; 2],
    /// Interpolation mode discriminant, matching the viewer's `Interpolation`
    /// order: nearest, linear, cubic, spline16, spline36, lanczos, blackman.
    pub interpolation: u32,
    /// Whether the display colour transform applies at all. When it does not,
    /// the host views raw output-space values.
    pub transform: bool,
    /// Source CCTF discriminant, used only when `transform` is set.
    pub src_cctf: u32,
    /// Whether to decode the source transfer before the display matrix.
    pub decode: bool,
    /// Source → encoded sRGB display matrix.
    pub matrix: [[f32; 3]; 3],
    /// Whether the request carries the resolved ICC pipeline.
    pub icc: bool,
    /// Whether the ICC request carries a matrix stage.
    pub icc_matrix: bool,
    /// ICC matrix in the reference's 1.14 fixed point, used only when
    /// `icc_matrix` is set.
    pub icc_matrix_fixed14: [[i32; 3]; 3],
    /// ICC matrix offset in 1.14 fixed point, used only when `icc_matrix` is
    /// set.
    pub icc_offset_fixed14: [i32; 3],
    /// ICC first-shaper tables, one per channel, used only when `icc` is set.
    pub icc_first: &'a [Vec<i32>],
    /// ICC second-shaper tables, one per channel, used only when `icc` is set.
    pub icc_second: &'a [Vec<u32>],
}

/// One pipe-tail transfer pass (pipe architecture, S4).
pub struct OutputTransferGpuParams {
    /// false = encode (matrix + cctf), true = decode (cctf only).
    pub decode: bool,
    /// `Cctf` discriminant: 0 Linear, 1 ACEScct, 2 sRGB, 3 ProPhoto,
    /// 4 Rec2020, 5 Adobe RGB (1998), 6 Gamma 2.6.
    pub cctf_kind: u32,
    /// Same-space RGB roundtrip matrix (`rgb_to_rgb_identity`).
    pub matrix: [[f32; 3]; 3],
}

/// One pipe-tail CMYK ink style pass (pipe architecture, S4).
pub struct CmykGpuParams {
    /// Mix strength, clamped to 0..=1 by the caller.
    pub strength: f32,
    /// Input/output state: true when the buffer carries encoded samples.
    pub encoded: bool,
    /// `Cctf` discriminant: 0 Linear, 1 ACEScct, 2 sRGB, 3 ProPhoto,
    pub cctf_kind: u32,
    /// Destination RGB → CIELAB D50 matrix, before the Lab appearance grade.
    pub to_lab: [[f32; 3]; 3],
    /// CIELAB D50 → destination RGB matrix, after the Lab appearance grade.
    pub from_lab: [[f32; 3]; 3],
}

impl FilmChainParams<'_> {
    /// Every resident optical Gaussian now uses the reference FIR/IIR pair, so
    /// no radius cap applies; only a non-finite width has no device meaning.
    pub fn gpu_blurs_supported(&self) -> bool {
        fn supported(sigma: f32) -> bool {
            sigma.is_finite()
        }
        let supports = supported;
        self.camera_lens_blur_px.is_none_or(supports)
            && self.scanner_lens_blur_px.is_none_or(supports)
            && self.unsharp.is_none_or(|p| supports(p.sigma_px))
            && self.glare.is_none_or(|p| supports(p.blur_px))
            && self
                .dir_couplers
                .is_none_or(|p| supports(p.diffusion_size_px) && supports(p.diffusion_tail_px))
            && self.halation.is_none_or(|p| {
                (0..3).all(|c| {
                    supports(p.scatter_core_px[c])
                        && supports(p.scatter_tail_px[c])
                        && supports(p.halation_first_sigma_px[c])
                })
            })
            && self
                .grain_v1
                .as_ref()
                .is_none_or(|p| p.grain_blur <= 0.0 || p.grain_blur.is_finite())
    }
}

/// Image-independent coefficients for the bounded inverse scan solver.
#[derive(Clone, Copy)]
pub struct ConvertSolverGpuParams<'a> {
    pub dye: &'a [[f64; 3]],
    pub base: &'a [f64],
    pub illuminant: &'a [f64],
    pub cmfs: &'a [[f64; 3]],
    pub normalization: f64,
    pub xyz_to_rgb: [[f64; 3]; 3],
    pub cmy_max: [f64; 3],
    pub calibration: [[f64; 3]; 3],
    pub gain: f64,
    pub seed_log_rgb: [f64; 3],
    pub seed_inverse: [[f64; 3]; 3],
    pub cctf_kind: u32,
    pub decode_input: bool,
    pub n_wl: u32,
}

/// Prepared monotone record axes and resolved per-channel positive tones.
pub struct ScannerDensityGpuParams {
    pub density: Vec<f64>,
    pub exposure: Vec<f64>,
    pub offsets: [u32; 3],
    pub counts: [u32; 3],
    pub contrast: [f64; 3],
    pub dmax: [f64; 3],
    pub brightness_ev: [f64; 3],
    pub denominator: [f64; 3],
    pub camera_ev: f64,
    pub lens_blur: f32,
    /// Measure the complete post-optics density frame on the device.
    pub standard_setup: bool,
    pub setup_aim_density: f64,
    pub setup_gamma: f64,
    pub operator_pivot: f64,
    pub operator_contrast_delta: f64,
    pub operator_exposure_ev: f64,
    pub operator_color: [f64; 3],
    /// The fixed positive tone is linear sRGB, independently of destination.
    pub tone_to_output: [[f64; 3]; 3],
}

/// Front pass of the GPU-resident chain: RGB → film raw, or inverse scanning
/// to developed film density. The RGB-to-raw variants are homogeneous, so
/// the caller folds exposure (auto-exposure × EV compensation × B&W filming
/// correction) into their matrix instead of dispatching a separate scale pass.
pub enum FrontPass<'a> {
    /// Hanatos2025 spectral upsampling: per-pixel chromaticity LUT lookup.
    Hanatos2025 {
        tc_lut: &'a spektrafilm_math::spectral::TcLut,
        /// Generated device table, absent only for the CPU reference table.
        generated: Option<&'a PreparedTable>,
        /// Combined input-RGB → CAT-adapted-XYZ matrix (row-major).
        rgb_to_adapted_xyz: [[f64; 3]; 3],
    },
    /// Mallett2019 reflectance-basis upsampling: one rgb → raw 3×3 matmul
    /// (`core · M_cs`, see `spektrafilm-core/src/mallett.rs`).
    Mallett2019 { matrix: [[f64; 3]; 3] },
    /// Scanned RGB → developed film density, without exposure/development.
    /// Borrows the caller's image-independent solver parameter record.
    ScanInverse(&'a ConvertSolverGpuParams<'a>),
}

#[derive(Debug, Clone, Copy)]
pub struct HighlightBoostGpuParams {
    pub boost_ev: f32,
    pub boost_range: f32,
    pub protect_ev: f32,
}

/// DIR couplers parameters for the GPU-resident matmul + diffusion + final
/// density-curve re-interpolation pass. Mirrors the CPU
/// `apply_density_correction`.
#[derive(Debug, Clone, Copy)]
pub struct DirCouplersGpuParams<'a> {
    /// Already-scaled `couplers_matrix * amount`, row-major.
    pub couplers_matrix_scaled: [[f32; 3]; 3],
    /// Per-channel max density from the normalized curves (used only when
    /// `is_positive` is true; ignored otherwise).
    pub density_max: [f32; 3],
    pub is_positive: bool,
    pub diffusion_size_px: f32,
    pub diffusion_tail_px: f32,
    pub diffusion_tail_weight: f32,
    /// Donor Langmuir knee per channel (`langmuir_donor_k * d_max`) —
    /// `Some` for negative films. `None` leaves the developed silver
    /// linear.
    pub donor_k: Option<[f32; 3]>,
    /// Donor amplitude-match point per channel (`d_max / 2`), meaningful
    /// only when `donor_k` is `Some`.
    pub donor_ref: [f32; 3],
    /// Receiver Langmuir knee per channel — `Some` for positive films;
    /// applied to the arrived inhibitor after spatial diffusion.
    pub receiver_knee: Option<[f32; 3]>,
    /// Receiver operating-point inhibitor per channel, meaningful only
    /// when `receiver_knee` is `Some`.
    pub receiver_ref: [f32; 3],
    /// "Density curves before DIR" — re-interpolated by the final shader
    /// pass with the corrected log-exposure.
    pub density_curves_0: &'a [[f64; 3]],
    pub log_exposure: &'a [f64],
    pub gamma_factor: f64,
}

/// Output gamut compression parameters for the GPU-resident per-pixel pass.
/// CPU equivalent: `OutputGamutCompress::compress`. The `C_max(L, h)` table
/// is baked once on the CPU (bisection against the output cube) and uploaded
/// as a storage buffer; the shader only does the per-pixel forward/inverse
/// perceptual transform + Reinhard knee.
///
/// Every destination-specific quantity the CPU compressor reads from its
/// colour space arrives here: the RGB↔XYZ matrices and the CIECAM16 viewing
/// conditions. The shader holds no space constants, so one kernel serves
/// every registered destination instead of sRGB alone.
#[derive(Debug, Clone, Copy)]
pub struct GamutGpuParams<'a> {
    /// 0 = aces_rgc, 1 = oklch, 2 = oklrab, 3 = cam16ucs, 4 = jzazbz — must
    /// match the mode dispatch in `gamut_compress.wgsl`. `jzazbz` runs its
    /// perceptual transform in binary64 (its `b` row cancels ~105x, so the
    /// C_max-indexing hue is not f32-representable), so the pass needs native
    /// shader binary64 and fails by name on a device without it.
    pub mode: u32,
    /// Reinhard knee on normalized chroma: (threshold, limit, power).
    pub knee: [f32; 3],
    /// Lightness-axis bounds of the `C_max` table grid.
    pub l_min: f32,
    pub l_max: f32,
    /// Destination linear RGB → XYZ at the destination whitepoint, row-major.
    pub rgb_to_xyz: [[f32; 3]; 3],
    /// Destination XYZ → linear RGB, row-major (the space's stored inverse).
    pub xyz_to_rgb: [[f32; 3]; 3],
    /// CIECAM16 viewing conditions for the destination white: `[d_r, d_g,
    /// d_b, a_w]`. See `Cam16Viewing::new(destination.whitepoint_xyz())`.
    pub cam16: [f32; 4],
    /// `C_max(L, h)` table, row-major `[n_l][n_h]`. Empty for aces_rgc.
    pub cmax: &'a [f64],
    /// Device-generated table, consumed without a host table upload.
    pub prepared: Option<&'a PreparedTable>,
    pub n_l: u32,
    pub n_h: u32,
}

/// Unsharp mask parameters: blur sigma in pixels and amount.
#[derive(Debug, Clone, Copy)]
pub struct UnsharpGpuParams {
    pub sigma_px: f32,
    pub amount: f32,
}

/// Viewing glare parameters for the GPU-resident lognormal + blur + apply
/// pass. CPU equivalent: `compute_random_glare_amount` followed by
/// `add_glare_with_amount`.
#[derive(Debug, Clone, Copy)]
pub struct GlareGpuParams {
    /// LogNormal conditioning derived from `percent` + `roughness`.
    /// The device normal transform uses f32 arithmetic.
    pub mu: f32,
    pub sigma: f32,
    pub blur_px: f32,
    pub base_seed: u32,
    /// `(XYZ→RGB) * illuminant_xyz`, pre-divided by 100. The shader
    /// just multiplies the lognormal scalar by this per-channel offset.
    pub rgb_offset: [f32; 3],
}

/// Halation parameters for the GPU-resident scatter + multi-bounce pass.
/// Pixel-space widths and amplitudes preserve their independent RGB controls.
#[derive(Debug, Clone, Copy)]
pub struct HalationGpuParams {
    pub scatter_amount: f32,
    pub scatter_core_px: [f32; 3],
    pub scatter_tail_px: [f32; 3],
    pub scatter_tail_weight: [f32; 3],
    /// Experimental local source-response compression applied only to the
    /// halation blur input. Zero selects the existing linear source path.
    pub source_compression: f32,
    /// Per-channel `halation_strength[c] * halation_amount`.
    pub halation_a_tot: [f32; 3],
    pub halation_first_sigma_px: [f32; 3],
    pub halation_n_bounces: u32,
    pub halation_bounce_decay: f32,
    pub halation_renormalize: bool,
}

/// One optical diffusion-filter request: a per-channel discrete PSF
/// convolution with reflect boundary extension.
///
/// CPU equivalent: `spektrafilm_model::diffusion::apply_diffusion_filter_um`.
/// The per-channel `k×k` PSF is resolved on the host from the filter family
/// and strength (the same reference coefficients, sampled in image-plane
/// pixels) and uploaded as a storage buffer; the shader only evaluates the
/// exact discrete convolution the reference performs. `radius` is the
/// half-width, so the tap count is `2*radius + 1` per axis.
#[derive(Debug, Clone, Copy)]
pub struct OpticalDiffusionGpuParams<'a> {
    /// Per-channel PSF taps, one row-major `k*k` kernel per channel, each
    /// already sum-normalised by the reference. Channel-specific optical
    /// parameters are never averaged across channels, so each channel
    /// carries its own taps.
    pub psf: &'a [Vec<f64>; 3],
    /// Kernel half-width in pixels.
    pub radius: u32,
    /// Deflected-photon fraction blended with the original exposure.
    pub scatter_fraction: f64,
}

/// Image-independent spectral coefficients and domain of an enlarger lattice.
#[derive(Debug, Clone, Copy)]
pub struct EnlargerSpectralLutGpuParams<'a> {
    pub channel_density: &'a [[f64; 3]],
    pub base_density: &'a [f64],
    pub illuminant: &'a [f64],
    pub sensitivity: &'a [[f64; 3]],
    pub normalization: f64,
    pub preflash: [f64; 3],
    pub minimum: [f64; 3],
    pub maximum: [f64; 3],
    pub size: u32,
}

/// Image-independent scanner log-XYZ lattice and post-interpolation colour transforms.
#[derive(Debug, Clone, Copy)]
pub struct ScannerSpectralLutGpuParams<'a> {
    pub channel_density: &'a [[f64; 3]],
    pub base_density: &'a [f64],
    pub illuminant: &'a [f64],
    /// Clear-film Y reading; spectral XYZ is divided by this value.
    pub normalization: f64,
    pub cat: &'a [[f64; 3]; 3],
    pub xyz_to_rgb: &'a [[f64; 3]; 3],
    pub minimum: [f64; 3],
    pub maximum: [f64; 3],
    pub size: u32,
}

/// One enlarger PCHIP LUT request: a prepared 3-D PCHIP lattice evaluated
/// against the normalized input domain.
///
/// CPU equivalent: `apply_pchip_lut` in `stages::printing`. The prepared
/// slopes and per-cell bounds are resolved on the host by the same
/// reference preparation and uploaded as storage buffers; the shader only
/// performs the same per-pixel Hermite walk, including the soft
/// monotonicity clamp to each cell's corner hull.
#[derive(Debug, Clone, Copy)]
pub struct EnlargerLutGpuParams<'a> {
    /// LUT lattice, row-major `[size³][3]`.
    pub lut: &'a [f64],
    /// Per-axis monotone slopes, row-major `[size³][3]`.
    pub slope_x: &'a [f64],
    pub slope_y: &'a [f64],
    pub slope_z: &'a [f64],
    /// Per-cell corner hull bounds, row-major `[(size-1)³][3]`.
    pub cell_min: &'a [f64],
    pub cell_max: &'a [f64],
    /// Lattice edge length per axis.
    pub size: u32,
    /// Domain minimum per channel, subtracted before normalization.
    pub data_min: [f64; 3],
    /// Normalization scale per channel, mapping the domain onto
    /// `[0, size-1]`.
    pub data_scale: [f64; 3],
}

/// One working-geometry request: rotation, exact pixel crop, and resize,
/// applied in that order to retained samples on the compute device.
///
/// The host resolves every quantity here, exactly as
/// `crate::resizing::crop_and_rescale` does for the reference route, so one
/// authoritative rule decides the raster geometry and a device pass cannot
/// disagree with the runtime about it. The device pass reproduces the
/// reference kernels; it does not re-derive the geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputGeometryGpuParams {
    /// Quarter turns applied first, counter-clockwise, matching
    /// `ImageBuf::rotated_quarter_turns`. Zero is a no-op.
    pub rotation_quarter_turns: u32,
    /// Exact pixel crop `[x, y, width, height]` resolved against the
    /// post-rotation dimensions with `resizing::requested_crop_rect`
    /// semantics. `None` when no crop is requested.
    pub crop: Option<[u32; 4]>,
    /// Resize applied last. `None` when the crop already reached the working
    /// size.
    pub resize: Option<InputResizeGpuParams>,
    /// Working pixel pitch in microns the route derived for the resulting
    /// geometry. The runtime computes it; the carrier carries it.
    pub pixel_pitch_um: f64,
}

/// Destination geometry of the resize step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputResizeGpuParams {
    pub width: u32,
    pub height: u32,
    pub method: InputResizeMethod,
}

/// Which host reference kernel the device resize must reproduce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputResizeMethod {
    /// `spektrafilm_math::resize::rescale_spline3`: cubic prefilter,
    /// Gaussian antialiasing, mirror extension, half-pixel centers, and the
    /// global range clip measured from the source on the device.
    Spline3,
    /// The preview resampler's triangle kernel. No range clip.
    PreviewTriangle,
}

/// Sample representation the native writer receives after GPU-side saving
/// conversion. Half values are the exact IEEE-754 binary16 bit patterns in
/// `u16` storage; F32 values are native binary32 samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputSampleKind {
    U8,
    U16,
    Half,
    F32,
}

/// Typed output sample bytes. The variant is authoritative for the sample
/// representation; writer code must not clip, encode, or quantize again.
#[derive(Debug, Clone, PartialEq)]
pub enum OutputSampleValues {
    U8(Vec<u8>),
    U16(Vec<u16>),
    Half(Vec<u16>),
    F32(Vec<f32>),
}

/// Final converted samples handed to the native file writer.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputSampleData {
    pub width: u32,
    pub height: u32,
    pub kind: OutputSampleKind,
    pub values: OutputSampleValues,
}

impl OutputSampleData {
    pub fn sample_count(&self) -> usize {
        self.width as usize * self.height as usize * 3
    }
}

/// Parameters for independent saving color and sample conversion.
///
/// One request carries the complete resolved transfer state, so the host and
/// retained device routes cannot disagree about whether the samples need
/// decoding. `src_cctf`/`decode` are derived from the source's declared
/// transfer state together with the pinned saving no-op (`same`), never from
/// the carrier alone: an encoded source that is already the destination
/// transfer state must not be decoded and left linear.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputSampleGpuParams {
    /// Source transfer function, used only when `decode` is true.
    pub src_cctf: u32,
    /// Whether to decode the source transfer before the matrix.
    pub decode: bool,
    /// Destination transfer function, used only when `encode` is true.
    pub dst_cctf: u32,
    /// Whether to encode the destination transfer after the matrix.
    pub encode: bool,
    /// Source-to-destination RGB primaries and white adaptation matrix.
    pub matrix: [[f32; 3]; 3],
    /// Requested representation at the native writer boundary.
    pub kind: OutputSampleKind,
}

/// One Fotufilm Film grain residual request.
///
/// CPU equivalent: `Bank::apply_image` in `spektrafilm_model::grain::fotufilm`.
/// The host resolves every image-independent quantity from the anchored
/// population and the request, and the device performs every image-dependent
/// one: the stochastic coating formation, the pitch statistics, and the
/// residual readout. No host-prepared grain field, population sample, or
/// per-pixel result crosses the boundary, and the host never evaluates the
/// developed density image.
///
/// The formation records come from `Population::device_records`, so the device
/// sees the same fitted sublayer coefficients, count thresholds and developing
/// fractions the reference forms its bank from. The device conservatively
/// resamples the full developed source onto the readout lattice before adding
/// grain. Source pitch is physical microns; the size-compensated bank coordinates
/// below are converted back to physical coordinates for that resample only.
#[derive(Debug, Clone)]
pub struct FotufilmGpuParams {
    /// Formation seed, the source's pinned `SEED` derived exactly as
    /// `light` derives it.
    pub formation_seed: u32,
    /// Per-record formation coefficients, in record order. A record with no
    /// sublayers forms no bank levels, exactly as `build_bank` skips it.
    pub records: Vec<FotufilmRecordGpuParams>,
    /// Whether the population is monochrome: one record's realization is
    /// shared across output channels.
    pub monochrome: bool,
    /// Source pitch in microns of the density image the device reads.
    pub source_pitch_um: f64,
    /// Full-frame output origin and pitch in bank texels, already divided by
    /// clamped `size`. Despite the historical suffix, these are not unscaled
    /// physical microns; one bank texel represents one micron before scaling.
    pub origin_um: [f64; 2],
    pub pitch_um: [f64; 2],
    /// Readout lattice dimensions: the developed source is resampled onto
    /// this lattice before the grain is read out of it.
    pub width: u32,
    pub height: u32,
    /// Pixel dimensions of the developed source the device reads, which the
    /// conservative resample needs as the source's extent. They are the
    /// image's own dimensions and are independent of the readout lattice.
    pub source_width: u32,
    pub source_height: u32,
    /// Readout footprint in bank texels, already divided by clamped `size` and
    /// multiplied by clamped `softness`; used directly by pitch and residual.
    pub footprint_um: [f64; 2],
    /// Residual sampling controls.
    pub seed: u64,
    pub amount: f64,
    pub size: f64,
    pub softness: f64,
    pub color: f64,
    pub layers: [f64; 3],
}

/// One record's device formation coefficients.
///
/// The coating is a list of sublayers, exactly as the reference's
/// `Population::device_records` reports them: `light` walks every sublayer of
/// a record in order and accumulates its own deposits into one lattice, so
/// the device forms each sublayer with its own cells, mark spread, capacity,
/// thresholds and developing fractions.
#[derive(Debug, Clone)]
pub struct FotufilmRecordGpuParams {
    /// Per-sublayer formation coefficients, in sublayer order. A record with
    /// no sublayers forms no bank levels, exactly as `build_bank` skips it.
    pub sublayers: Vec<FotufilmSublayerGpuParams>,
    /// Bank density floor and ceiling of this record.
    pub d_min: f64,
    pub d_max: f64,
}

/// One sublayer's device formation coefficients.
#[derive(Debug, Clone)]
pub struct FotufilmSublayerGpuParams {
    /// Coating grid divisions per axis of one bank formation pass.
    pub cells: u32,
    /// Whether the sublayer develops silver (uncovered-grain) or dye clouds.
    pub silver: bool,
    /// Mark spread in millimetres, before the source's texel scaling.
    pub sigma_mm: f64,
    /// Local dye capacity in density units.
    pub capacity: f64,
    /// Unit-crystal mark scale.
    pub edge: f64,
    /// Developing fraction against normalized density, 129 taps.
    pub forming: Vec<f64>,
    /// Cumulative Poisson count thresholds, 48 values.
    pub thresholds: Vec<f32>,
    /// Largest texel distance from a crystal centre to any texel it can
    /// deposit into, as an upper bound of the law's support.
    pub reach_texels: f64,
    /// The largest crystal occupied area this sublayer can draw, as the
    /// reference's `occupied_area` resolves it. The device uses it to bound
    /// the uncovered square's side.
    pub occupied_area: f64,
}
