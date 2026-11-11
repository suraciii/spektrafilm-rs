//! Device-independent film-chain and stage parameter records.
//!
//! These records cross the [`ComputeBackend`](crate::ComputeBackend) trait boundary and
//! describe the GPU-resident chain. The trait and its crate-root re-exports stay in
//! `lib.rs`.
use crate::device_image::DeviceImagePayload;
use crate::dye_params::DyeGpuParams;
use crate::grain_params::{FotufilmGpuParams, ProceduralGrainGpuParams, SpektraGrainGpuParams};
use crate::preparation::PreparedTable;
use spektrafilm_math::image::ImageBuf;

/// CPU stage closure type shared by all pipe fence fields.
pub type CpuStageFn<'a> = Box<dyn Fn(&ImageBuf) -> Result<ImageBuf, String> + 'a>;

pub struct Lut3D {
    pub size: u32,
    pub data: Vec<f32>,
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

/// All inputs to the GPU-resident film chain. Bundled into a struct so the
/// trait method stays object-safe and adding more optional stages (halation,
/// DIR couplers, …) doesn't churn every callsite.
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
    /// Optional common finishing on completed positive linear RGB, before gamut.
    pub finishing: Option<FinishingGpuParams>,
    /// Optional output gamut compression pass — applied after glare on
    /// the final linear RGB buffer, before unsharp (matching CPU scanning).
    pub gamut: Option<GamutGpuParams<'a>>,
    /// Optional unsharp mask pass — applied after glare (last step
    /// before readback). Blur σ in pixels + amount scalar; both come
    /// from `scanner.unsharp_mask`.
    pub unsharp: Option<UnsharpGpuParams>,
    /// Optional Grain Procedural pass — applied after scanner effects, the destination
    /// same-space matrix roundtrip, and transfer encoding. Returns encoded RGB.
    pub grain_procedural: Option<ProceduralGrainGpuParams>,
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
    /// Optional device Spektra grain pass on the film density buffer, in
    /// the same position as `density_cpu_stage` (after the film density curve
    /// and DIR, before printing). `None` when the request samples the scipy
    /// MT19937 regime, whose single sequential generator over the whole array
    /// is not a per-pixel mapping — those requests keep the host fence, so the
    /// device never runs a different noise law.
    pub grain_spektra: Option<SpektraGrainGpuParams>,
}

/// Chain-head input transfer decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputDecodeGpuParams {
    /// Input colour space CCTF discriminant (see `input_decode`).
    pub cctf_kind: u32,
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
                .grain_spektra
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

/// Prepared common-finishing response and row-major color transforms.
/// The core owns the response coefficients and all six matrix definitions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FinishingGpuParams {
    /// Brightness, contrast, warmth and saturation response coefficients.
    pub tone: [f32; 4],
    /// Chroma reference and normalized warmth direction, with one unused lane.
    pub color: [f32; 4],
    pub expand_enabled: bool,
    pub black: f32,
    pub inverse_span: f32,
    pub to_srgb: [[f32; 3]; 3],
    pub from_srgb: [[f32; 3]; 3],
    pub rgb_to_lms: [[f32; 3]; 3],
    pub lms_to_lab: [[f32; 3]; 3],
    pub lab_to_lms: [[f32; 3]; 3],
    pub lms_to_rgb: [[f32; 3]; 3],
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

/// One LUT-look table evaluation: shaper, domain policy, tetrahedral 3D
/// interpolation and LUT strength. The image is already in the LUT input
/// space; `original` is the same input transformed to the LUT output space,
/// so the shader can apply strength without another device pass.
#[derive(Debug, Clone, Copy)]
pub struct Lut3dLookGpuParams<'a> {
    pub size: u32,
    /// Internal blue-fast RGB lattice, `(r * size + g) * size + b`.
    pub table: &'a [f32],
    pub shaper_size: u32,
    /// Shaper entries in file order; empty when no shaper is declared.
    pub shaper: &'a [f32],
    pub domain_min: [f32; 3],
    pub domain_span: [f32; 3],
    pub clamp_domain: bool,
    pub strength: f32,
}

/// LUT halation appearance: a linear-light highlight mask followed by the
/// declared multi-bounce response. `halation` carries the spatial response;
/// scatter is disabled by the caller because LUT halation is an additive
/// highlight layer, not a rewrite of the base image.
#[derive(Debug, Clone, Copy)]
pub struct LutHalationGpuParams {
    pub mask_low: f32,
    pub mask_high: f32,
    pub mask_transition: f32,
    pub halation: HalationGpuParams,
}

/// Output-referred appearance grain for a LUT look.
#[derive(Debug, Clone, Copy)]
pub struct LutGrainGpuParams {
    pub amount: f32,
    pub size: f32,
    pub shadows: f32,
    pub midtones: f32,
    pub highlights: f32,
    pub chroma: f32,
    pub seed: u32,
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
