//! Compute backend abstraction and its default CPU implementations.
//!
//! This crate owns the `ComputeBackend` trait, the CPU reference
//! implementation, the shared observation vocabulary, and the
//! device-independent parameter records that cross the trait boundary. It
//! depends on no device toolkit, so domain code can reach the CPU reference
//! implementation without compiling device code.

pub mod analysis;
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
mod chain_params;
pub mod cpu_backend;
mod grain_params;
pub mod scalar_scan;
pub mod viewer_params;
pub use chain_params::*;
pub use grain_params::*;
pub use viewer_params::{
    DisplayTransformGpuParams, ViewerClipping, ViewerComposeGpuParams, ViewerFullSourceGpuParams,
    ViewerIccGpuParams, ViewerIccKind, ViewerResampleGpuParams,
};
pub mod preparation;
pub mod viewer_clipping;
pub use preparation::{
    GamutTableParams, InputGamutTableParams, PreparedTable, SpectralTcLutParams,
};

use spektrafilm_math::image::ImageBuf;

/// The `cctf_kind` discriminant every device transfer shader takes
/// (`cctf_lib.wgsl`). It lives here, next to the trait and its crate-root
/// re-exports, because it is part of the device ABI: the shaders and the host
/// must agree on one mapping, and more than one caller needs it.
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

    /// Apply prepared common finishing to linear RGB on the selected device.
    /// `None` means this backend does not implement the device stage.
    fn finishing(
        &self,
        _img: &ImageBuf,
        _params: &FinishingGpuParams,
    ) -> Result<Option<ImageBuf>, String> {
        Ok(None)
    }

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
    fn grain_procedural(
        &self,
        _image: &ImageBuf,
        _params: &ProceduralGrainGpuParams,
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

    /// Upload immutable host samples for native viewer warnings. The returned
    /// carrier is only valid as the native argument to viewer composition; it
    /// must not be used for rendering, inspection, or output. Backends may
    /// preserve host precision separately from their normal computation carrier.
    fn retain_viewer_clipping_source(
        &self,
        image: &ImageBuf,
        meta: &RetainedImageMeta,
    ) -> Result<DeviceImage, String> {
        self.retain_image(image, meta)
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
    /// Reduction uses area averaging before the display and ICC passes; an
    /// unchanged size preserves the source samples. `None` display means raw
    /// output-space viewing.
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
    /// Prepare a full-source viewport, transforming source taps before sampling.
    /// Non-nearest minification uses area averaging; other sampling keeps the
    /// requested interpolation mode.
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
        _native: Option<&DeviceImage>,
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
    /// Reduce native samples in the selected analysis region.
    fn analyze_retained(
        &self,
        image: &DeviceImage,
        request: &analysis::AnalysisRequest,
    ) -> Result<analysis::AnalysisData, String> {
        let host = image
            .host_image()
            .ok_or_else(|| "backend has no retained analysis reduction".to_owned())?;
        analysis::analyze_host(host, image.meta(), request, None)
    }
    /// Inspect one native pixel in the analysis color interpretation.
    fn inspect_analysis_pixel(
        &self,
        image: &DeviceImage,
        x: u32,
        y: u32,
    ) -> Result<analysis::AnalysisPixel, String> {
        let host = image
            .host_image()
            .ok_or_else(|| "backend has no retained analysis inspection".to_owned())?;
        analysis::inspect_host(host, image.meta(), x, y)
    }
    /// Resample `image` to the viewer's capped size on the compute device,
    /// returning the area-averaged display raster as flat `f32` RGB. A
    /// same-size request preserves the source samples.
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

    /// Compose the viewer's full-source view on the compute device,
    /// returning the bounded viewport raster as flat `f32` RGB.
    ///
    /// The view samples the full-resolution source at every visible viewport
    /// pixel, so a device route must not download it to resample on the host.
    /// Non-nearest minification area-averages transformed source taps; otherwise
    /// it uses the requested interpolation mode. The request carries viewport
    /// geometry and resolved display and ICC transforms, so the device evaluates
    /// the same per-pixel arithmetic as the host composition. The returned pixels are
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
