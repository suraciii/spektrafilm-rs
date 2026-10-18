//! Compute backend abstraction and its default CPU implementations.
//!
//! This crate owns the `ComputeBackend` trait, the CPU reference
//! implementation, the shared observation vocabulary, and the
//! device-independent parameter records that cross the trait boundary. It
//! depends on no device toolkit, so domain code can reach the CPU reference
//! implementation without compiling device code.

pub mod dye_params;
use dye_params::DyeGpuParams;
pub mod telemetry;
pub use telemetry::{
    AdapterDescription, CpuReason, Executor, Observation, ObservationContext, ObservationKind,
};
mod observed_backend;
pub mod timing;
pub use observed_backend::{bind_backend, bind_cpu_fallback};
pub mod cpu_backend;

/// CPU stage closure type shared by all pipe fence fields.
pub type CpuStageFn<'a> = Box<dyn Fn(&ImageBuf) -> Result<ImageBuf, String> + 'a>;
use spektrafilm_math::image::ImageBuf;

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
    fn observation_context(&self) -> Option<&telemetry::ObservationContext> {
        None
    }
    fn with_observation_context(
        &self,
        _context: telemetry::ObservationContext,
    ) -> Option<Box<dyn ComputeBackend + '_>> {
        None
    }
    fn adapter_description(&self) -> Option<telemetry::AdapterDescription> {
        None
    }
    fn colorspace_convert(&self, img: &ImageBuf, matrix: &[[f32; 3]; 3]) -> ImageBuf;
    fn cctf_encode_srgb(&self, img: &ImageBuf) -> ImageBuf;
    fn cctf_decode_srgb(&self, img: &ImageBuf) -> ImageBuf;
    fn gaussian_blur(&self, img: &ImageBuf, sigma: f32) -> ImageBuf;

    /// Blur `img` with each of `sigmas`, returning one image per sigma.
    /// GPU backends fuse the work into a single command buffer (one upload,
    /// one submit, one readback) — much cheaper than N independent
    /// `gaussian_blur` calls when N > 1. Default implementation calls the
    /// per-sigma kernel in a loop.
    fn gaussian_blur_multi(&self, img: &ImageBuf, sigmas: &[f32]) -> Vec<ImageBuf> {
        sigmas.iter().map(|&s| self.gaussian_blur(img, s)).collect()
    }
    fn table_lookup(&self, img: &ImageBuf, table_x: &[f32], table_y: &[[f32; 3]]) -> ImageBuf;
    fn lut3d_interp(&self, img: &ImageBuf, lut: &Lut3D) -> ImageBuf;

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
    ) -> ImageBuf {
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
        cpu_backend::scan_spectral_cpu(
            density_cmy,
            channel_density,
            base_density,
            illuminant,
            normalization,
            cat,
            xyz_to_rgb,
        )
    }

    /// Runtime-grid scan. GPU implementations deliberately route this
    /// uncommon path through the faithful CPU integrator rather than silently
    /// truncating CMFs to the historical 81-sample grid.
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
    ) -> ImageBuf {
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
        cpu_backend::scan_spectral_cpu_with_cmfs(
            density_cmy,
            channel_density,
            base_density,
            illuminant,
            cmfs,
            normalization,
            cat,
            xyz_to_rgb,
        )
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
    ) -> ImageBuf {
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
        cpu_backend::print_spectral_cpu(
            density_cmy,
            channel_density,
            base_density,
            illuminant,
            sensitivity,
            normalization_factor,
            preflash,
        )
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
    ) -> ImageBuf {
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
        spektrafilm_math::spectral::hanatos2025_rgb_to_raw(
            image,
            tc_lut,
            color_space,
            ref_illuminant,
            cat16,
        )
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
    ) -> ImageBuf {
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
        spektrafilm_math::interp::fast_interp_image_f64(log_raw, &scaled, density_curves)
    }

    /// Return a rendered Grain Procedural image when this backend supports the shader.
    /// Input and output are native encoded RGB in the caller's color space;
    /// Grain performs no transfer-function or primaries conversion. Input preparation
    /// and final composition round to binary16 while sampling and noise use f32.
    /// The caller uses the independent CPU reference when this returns None.
    fn grain_v2(&self, _image: &ImageBuf, _params: &GrainV2GpuParams) -> Option<ImageBuf> {
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

    /// Optional fused fast-path: runs filming + printing + scanning as a single
    /// GPU-resident command buffer (one upload at start, one readback at end).
    /// Returns native encoded destination RGB when Grain Procedural is active, and
    /// linear destination RGB otherwise, or `None` for the per-stage path.
    /// The caller decodes only the transfer when linear grain output is requested.
    /// `Ok(None)` selects the per-stage path; `Err` reports a CPU fence that
    /// could not calibrate its output (the per-stage route returns the same
    /// error, so the caller surfaces it instead of falling back).
    fn try_run_film_chain(
        &self,
        _params: &FilmChainParams<'_>,
    ) -> Result<Option<ImageBuf>, String> {
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
pub struct FilmChainParams<'a> {
    pub image: &'a ImageBuf,
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
    pub viewing_illuminant: &'a [f64],
    pub scan_normalization: f64,
    pub scan_xyz_to_rgb: &'a [[f64; 3]; 3],
    /// Scanner white and black correction luminance remap `(m, q)` applied
    /// per-pixel on the scan XYZ (`clip(m·Y+q, 0, 1)/(Y+1e-10)`). `None` is
    /// identity. The filming/printing exposure halves of the correction are
    /// folded by the caller into `rgb_to_adapted_xyz` /
    /// `print_normalization_factor`.
    pub white_black_xyz_remap: Option<(f64, f64)>,
    /// Scan the developed film directly instead of printing onto paper.
    /// When true the print_spectral + print density-curve passes are
    /// skipped and scan_spectral runs on the film density buffer using the
    /// film's `film_channel_density` / `film_base_density` (the caller sets
    /// `viewing_illuminant` / `scan_normalization` / `scan_xyz_to_rgb` from
    /// the film's viewing illuminant). Mirrors the CPU `scan_film` path.
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
    pub scan_output_space: &'a spektrafilm_math::colorspace::RgbColorSpace,
    /// Optional camera lens Gaussian blur on the raw film exposure buffer,
    /// before halation.
    pub camera_lens_blur_px: Option<f32>,
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
    /// Optional CPU stage on the film density buffer after the film density
    /// curve and DIR, before printing — the assembled fence for the host
    /// grain engines (Spektra, and Dye whenever the field has no device
    /// path; pipe architecture, capability class). The closure receives the
    /// materialized density_cmy image and returns the grained image, which
    /// is written back to the same device buffer.
    pub density_cpu_stage: Option<CpuStageFn<'a>>,
    /// Optional CPU stage on the scanner readout buffer between glare and
    /// output gamut compression — the assembled fence for the scanner
    /// readout interpretation (slide points, the faithful luminance remap,
    /// negative-to-positive conversion, and the LabScan finish; pipe
    /// architecture, capability class). Receives the acquisition RGB and
    /// returns the interpreted image, which is written back in place. When
    /// present, `white_black_xyz_remap` must be `None`: the fence owns the
    /// remap in the faithful order, after optical blur and glare.
    pub scan_tail_cpu_stage: Option<CpuStageFn<'a>>,
    /// Optional CPU stage replacing the GPU scan spectral pass whenever the
    /// CPU route acquires differently: the PCHIP scanner LUT (the slide
    /// luminance remap or `use_scanner_lut`), the modeled receiver capture,
    /// and the negative-to-positive density reading and Digital Reference
    /// (pipe architecture, capability class). Receives the film/print
    /// density and returns the captured RGB, which is written to the
    /// acquisition buffer.
    pub scan_acquisition_cpu_stage: Option<CpuStageFn<'a>>,
    /// Optional CPU stage replacing the fused GPU print spectral pass when
    /// the CPU route would take the PCHIP enlarger LUT or the enlarger
    /// diffusion filter (pipe architecture, capability class). Receives the
    /// film density and returns the print log-raw after the outer log —
    /// the same domain the fused shader emits — so the GPU print density
    /// curve pass continues unchanged.
    pub print_acquisition_cpu_stage: Option<CpuStageFn<'a>>,
    /// Optional CPU stage on the raw film exposure buffer between
    /// highlight reconstruction and the camera lens blur — the assembled
    /// fence for the camera optical diffusion filter's discrete PSF
    /// convolution (pipe architecture, capability class). Receives the
    /// front-pass raw and returns the diffused raw, written back in place.
    pub camera_diffusion_cpu_stage: Option<CpuStageFn<'a>>,
    /// Optional output transfer encoding/decoding pass on the final buffer
    /// after every scanner effect (pipe architecture, S4). Encode applies
    /// the stored same-space roundtrip matrix and the space's encoding
    /// transfer; decode restores linear for a Procedural-grain output when
    /// the caller requested no encoding.
    pub output_transfer: Option<OutputTransferGpuParams>,
    /// Optional magazine print appearance pass on the final buffer after
    /// the output transfer (pipe architecture, S4). The residual host
    /// finishing moves onto the device; the input state is encoded exactly
    /// when the caller requested output encoding.
    pub magazine: Option<MagazineGpuParams>,
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

/// One pipe-tail magazine appearance pass (pipe architecture, S4).
pub struct MagazineGpuParams {
    /// Mix strength, clamped to 0..=1 by the caller.
    pub strength: f32,
    /// Input/output state: true when the buffer carries encoded samples.
    pub encoded: bool,
    /// `Cctf` discriminant: 0 Linear, 1 ACEScct, 2 sRGB, 3 ProPhoto,
    /// 4 Rec2020, 5 Adobe RGB (1998), 6 Gamma 2.6.
    pub cctf_kind: u32,
    /// Destination RGB → CIELAB D50 (Bradford-adapted).
    pub to_lab: [[f32; 3]; 3],
    /// CIELAB D50 → destination RGB.
    pub from_lab: [[f32; 3]; 3],
    /// Tone-curve input knots in CIELAB L*.
    pub tone_in: [f32; 10],
    /// Tone-curve output knots in CIELAB L*.
    pub tone_out: [f32; 10],
    /// Tone-curve Hermite slopes.
    pub tone_slopes: [f32; 10],
}

impl FilmChainParams<'_> {
    pub fn gpu_blurs_supported(&self) -> bool {
        /// GPU FIR kernels support a maximum 256-pixel half-width.
        fn supported(sigma: f32) -> bool {
            sigma.is_finite() && (3.0 * sigma.max(0.0)).ceil() <= 256.0
        }
        let supports = supported;
        self.camera_lens_blur_px.is_none_or(supports)
            && self.scanner_lens_blur_px.is_none_or(supports)
            && self.unsharp.is_none_or(|p| supports(p.sigma_px))
            && self.glare.is_none_or(|p| supports(p.blur_px))
            && self.dir_couplers.is_none_or(|p| {
                let max_tail_ratio = spektrafilm_math::gaussian::EXPONENTIAL_GAUSSIAN_FITS
                    .iter()
                    .map(|(_, ratio)| *ratio as f32)
                    .fold(0.0_f32, f32::max);
                supports(p.diffusion_size_px) && supports(p.diffusion_tail_px * max_tail_ratio)
            })
            && self.halation.is_none_or(|p| {
                supports(p.scatter_core_px)
                    && supports(p.scatter_tail_px)
                    && supports(p.halation_first_sigma_px * (p.halation_n_bounces as f32).sqrt())
            })
    }
}

/// RGB → film-raw front pass of the GPU-resident chain. Both variants are
/// homogeneous in the input RGB, so the caller folds the exposure scale
/// (auto-exposure × EV compensation × B&W filming correction) into the
/// matrix — no separate scale pass.
pub enum FrontPass<'a> {
    /// Hanatos2025 spectral upsampling: per-pixel chromaticity LUT lookup.
    Hanatos2025 {
        tc_lut: &'a spektrafilm_math::spectral::TcLut,
        /// Combined input-RGB → CAT-adapted-XYZ matrix (row-major).
        rgb_to_adapted_xyz: [[f64; 3]; 3],
    },
    /// Mallett2019 reflectance-basis upsampling: one rgb → raw 3×3 matmul
    /// (`core · M_cs`, see `spektrafilm-core/src/mallett.rs`).
    Mallett2019 { matrix: [[f64; 3]; 3] },
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
#[derive(Debug, Clone, Copy)]
pub struct GamutGpuParams<'a> {
    /// 0 = aces_rgc, 1 = oklch, 2 = oklrab, 3 = cam16ucs — must match the
    /// mode dispatch in `gamut_compress.wgsl`.
    pub mode: u32,
    /// Reinhard knee on normalized chroma: (threshold, limit, power).
    pub knee: [f32; 3],
    /// Lightness-axis bounds of the `C_max` table grid.
    pub l_min: f32,
    pub l_max: f32,
    /// `C_max(L, h)` table, row-major `[n_l][n_h]`. Empty for aces_rgc.
    pub cmax: &'a [f64],
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
    /// LogNormal μ and σ derived on the CPU from `percent` + `roughness`.
    pub mu: f32,
    pub sigma: f32,
    pub blur_px: f32,
    pub base_seed: u32,
    /// `(XYZ→RGB) * illuminant_xyz`, pre-divided by 100. The shader
    /// just multiplies the lognormal scalar by this per-channel offset.
    pub rgb_offset: [f32; 3],
}

/// Halation parameters for the GPU-resident scatter + multi-bounce pass.
/// Mirrors the field layout of the CPU `apply_halation_um`, but pre-converts
/// the per-channel µm sigmas to a single pixel-space sigma (the GPU path
/// uses the average across channels, matching the existing CPU impl).
#[derive(Debug, Clone, Copy)]
pub struct HalationGpuParams {
    pub scatter_amount: f32,
    pub scatter_core_px: f32,
    pub scatter_tail_px: f32,
    /// Per-channel tail weight. Python applies these per channel
    /// (Portra 0.78 / 0.65 / 0.67); averaging is a noticeable
    /// chromatic drift.
    pub scatter_tail_weight: [f32; 3],
    /// Experimental local source-response compression applied only to the
    /// halation blur input. Zero selects the existing linear source path.
    pub source_compression: f32,
    pub halation_amount: f32,
    pub halation_strength_avg: f32,
    /// Per-channel `halation_strength[c] * halation_amount`. Used only
    /// for the renormalize pass (which is per-channel even though the
    /// blur/add use the averaged scalar). When `halation_renormalize`
    /// is false this is ignored.
    pub halation_a_tot: [f32; 3],
    pub halation_first_sigma_px: f32,
    pub halation_n_bounces: u32,
    pub halation_bounce_decay: f32,
    pub halation_renormalize: bool,
}
