/// Runtime parameters for the film simulation pipeline.
///
/// Mirrors Python `params_schema.py`. Every field has a sensible default
/// matching the Python implementation.
use serde::{Deserialize, Serialize};

pub mod couplers;
pub mod diffusion;
pub mod glare;
pub mod grain;
mod grain_compat;
pub use grain_compat::normalize_grain_control_names;
pub mod halation;
pub mod sources;

pub(crate) mod validation;
use couplers::DirCouplersParams;
pub use diffusion::DiffusionFilterParams;
use glare::GlareParams;
pub use grain::{FotufilmGrainParams, FotufilmMaterial};
pub use grain::{GrainEngine, GrainParams};
pub use halation::HalationParams;
pub use validation::{ValidationError, ValidationReason};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraParams {
    #[serde(default)]
    pub exposure_compensation_ev: f32,
    #[serde(default = "default_true")]
    pub auto_exposure: bool,
    #[serde(default = "default_center_weighted")]
    pub auto_exposure_method: String,
    #[serde(default)]
    pub lens_blur_um: f32,
    #[serde(default = "default_35")]
    pub film_format_mm: f32,
    #[serde(default = "default_filter_uv")]
    pub filter_uv: [f32; 3],
    #[serde(default = "default_filter_ir")]
    pub filter_ir: [f32; 3],
    /// Stable camera taking-filter identifier. `"none"` preserves the
    /// historical no-filter behavior; named values are loaded from the
    /// shipped measured transmission curves.
    #[serde(
        default = "default_color_filter",
        deserialize_with = "deserialize_color_filter"
    )]
    pub color_filter: String,
    #[serde(default)]
    pub diffusion_filter: DiffusionFilterParams,
    /// Scene illuminant the filming front pass projects through. `None` keeps
    /// the film profile's reference illuminant, preserving the historical
    /// behavior; an explicit named catalog source or `BB<kelvin>` blackbody
    /// overrides it. Distinct from the film reference (which stays the
    /// sensitivity calibration illuminant), the scan and enlarger illuminants
    /// and the output white point.
    #[serde(default)]
    pub capture_illuminant: Option<String>,
}

impl Default for CameraParams {
    fn default() -> Self {
        Self {
            exposure_compensation_ev: 0.0,
            auto_exposure: true,
            auto_exposure_method: "center_weighted".into(),
            lens_blur_um: 0.0,
            film_format_mm: 35.0,
            filter_uv: [0.0, 410.0, 8.0],
            filter_ir: [0.0, 675.0, 15.0],
            color_filter: "none".into(),
            diffusion_filter: DiffusionFilterParams::default(),
            capture_illuminant: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrintExposureMode {
    /// Apply `print_exposure` without midgray normalization.
    Manual,
    /// Normalize the print exposure so the developed midgray density prints
    /// to the paper midgray, and follow camera exposure compensation.
    #[default]
    Auto,
}
impl PrintExposureMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Auto => "auto",
        }
    }
}

/// Legacy carry-over of the two intermediate print-exposure behaviors that
/// the `manual | auto` pair cannot express. `follow_mode` tracks in `auto`
/// and does not track in `manual`; `never` disables tracking in `auto`;
/// `always` applies the relative compensation factor in `manual`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrintExposureEvTracking {
    #[default]
    FollowMode,
    Never,
    Always,
}
impl PrintExposureEvTracking {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FollowMode => "follow_mode",
            Self::Never => "never",
            Self::Always => "always",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnlargerParams {
    #[serde(default = "default_th_kg3")]
    pub illuminant: String,
    #[serde(default = "default_one")]
    pub print_exposure: f32,
    #[serde(default)]
    pub print_exposure_mode: PrintExposureMode,
    #[serde(default)]
    pub y_filter_shift: f32,
    #[serde(default)]
    pub m_filter_shift: f32,
    #[serde(default = "default_55")]
    pub y_filter_neutral: f32,
    #[serde(default = "default_65")]
    pub m_filter_neutral: f32,
    #[serde(default)]
    pub c_filter_neutral: f32,
    #[serde(default)]
    pub lens_blur: f32,
    #[serde(default)]
    pub diffusion_filter: DiffusionFilterParams,
    #[serde(default)]
    pub preflash_exposure: f32,
    #[serde(default)]
    pub preflash_y_filter_shift: f32,
    #[serde(default)]
    pub preflash_m_filter_shift: f32,
}

impl Default for EnlargerParams {
    fn default() -> Self {
        Self {
            illuminant: "TH-KG3".into(),
            print_exposure: 1.0,
            print_exposure_mode: PrintExposureMode::Auto,
            y_filter_shift: 0.0,
            m_filter_shift: 0.0,
            y_filter_neutral: 55.0,
            m_filter_neutral: 65.0,
            c_filter_neutral: 0.0,
            lens_blur: 0.0,
            diffusion_filter: DiffusionFilterParams::default(),
            preflash_exposure: 0.0,
            preflash_y_filter_shift: 0.0,
            preflash_m_filter_shift: 0.0,
        }
    }
}

/// Operator corrections after immutable standard frame setup and before tone.
/// Color axes are differential EV, not temperature or scanner button steps.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanOperatorParams {
    pub exposure_ev: f64,
    pub warmth_ev: f64,
    pub tint_ev: f64,
    pub contrast: f64,
}

impl Default for ScanOperatorParams {
    fn default() -> Self {
        Self {
            exposure_ev: 0.0,
            warmth_ev: 0.0,
            tint_ev: 0.0,
            contrast: 1.0,
        }
    }
}

impl ScanOperatorParams {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScannerParams {
    /// Scan output polarity: `direct_scan` preserves the scanner capture;
    /// `negative_to_positive` interprets each negative dye record's density as a
    /// positive image.
    #[serde(default = "default_direct_scan")]
    pub scan_output: String,
    /// Named positive-conversion style applied by the negative-to-positive
    /// interpretation: `standard` is the built-in frozen tone; any other
    /// name resolves to `data/scan-styles/<name>.json` (declared
    /// per-channel tone parameters, never fitted). Meaningful only with
    /// `scan_output = negative_to_positive`.
    #[serde(
        default = "default_conversion_style",
        deserialize_with = "deserialize_conversion_style",
        serialize_with = "serialize_conversion_style"
    )]
    pub conversion_style: String,
    /// Experimental full-frame setup for the negative-to-positive
    /// interpretation: it measures the developed frame for exposure, cast and
    /// contrast placement before the fixed tone. The algorithm is not a
    /// validated photographic model, so it is off unless a caller asks for it.
    /// It makes the scan frame-dependent and therefore unbakeable.
    #[serde(default)]
    pub experimental_setup: bool,
    /// Standard negative-to-positive corrections; inactive routes require defaults.
    #[serde(default)]
    pub operator: ScanOperatorParams,
    #[serde(default)]
    pub lens_blur: f32,
    #[serde(default = "default_unsharp")]
    pub unsharp_mask: [f64; 2],
}

impl Default for ScannerParams {
    fn default() -> Self {
        Self {
            scan_output: default_direct_scan(),
            conversion_style: default_conversion_style(),
            experimental_setup: false,
            operator: ScanOperatorParams::default(),
            lens_blur: 0.0,
            unsharp_mask: default_unsharp(),
        }
    }
}

fn default_conversion_style() -> String {
    "standard".into()
}

impl ScannerParams {
    /// Experimental automatic setup must observe the complete working frame.
    pub fn requires_full_frame(&self) -> bool {
        self.scan_output == "negative_to_positive" && self.experimental_setup
    }

    /// Whether manual operator corrections reach the render: the built-in tone
    /// on the negative interpretation. Declared fixed styles are complete tone
    /// definitions and reject correction.
    pub fn operator_eligible(&self) -> bool {
        self.scan_output == "negative_to_positive" && self.uses_builtin_tone()
    }

    /// Whether the style resolves to the built-in frozen tone. `neutral` is an
    /// input-only migration alias for it.
    pub fn uses_builtin_tone(&self) -> bool {
        matches!(self.conversion_style.as_str(), "standard" | "neutral")
    }

    pub fn conversion_style_name(&self) -> &str {
        if self.uses_builtin_tone() {
            "standard"
        } else {
            &self.conversion_style
        }
    }
}

fn deserialize_conversion_style<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<String, D::Error> {
    let name = String::deserialize(d)?;
    Ok(if name == "neutral" {
        "standard".into()
    } else {
        name
    })
}

fn serialize_conversion_style<S: serde::Serializer>(name: &str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(if name == "neutral" { "standard" } else { name })
}

fn default_one_f64() -> f64 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilmBaseParams {
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default = "default_one_f64")]
    pub scale: f64,
    #[serde(default)]
    pub tilt: f64,
    #[serde(default = "default_one_f64")]
    pub cyan: f64,
    #[serde(default = "default_one_f64")]
    pub magenta: f64,
    #[serde(default = "default_one_f64")]
    pub yellow: f64,
}
impl Default for FilmBaseParams {
    fn default() -> Self {
        Self {
            active: true,
            scale: 1.0,
            tilt: 0.0,
            cyan: 1.0,
            magenta: 1.0,
            yellow: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintBaseParams {
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default = "default_one_f64")]
    pub scale: f64,
    #[serde(default = "default_one_f64")]
    pub cyan: f64,
    #[serde(default = "default_one_f64")]
    pub magenta: f64,
    #[serde(default = "default_one_f64")]
    pub yellow: f64,
}
impl Default for PrintBaseParams {
    fn default() -> Self {
        Self {
            active: true,
            scale: 1.0,
            cyan: 1.0,
            magenta: 1.0,
            yellow: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConvertFilmParams {
    /// Illumination used to acquire the imported RGB capture. This belongs
    /// to capture inversion, not the product readout illuminant.
    #[serde(default = "default_convert_illuminant")]
    pub scan_illuminant: String,
    #[serde(default)]
    pub exposure_compensation_ev: f64,
    #[serde(default = "default_99")]
    pub base_percentile: f64,
    #[serde(default = "default_calibration")]
    pub calibration: String,
}
fn default_convert_illuminant() -> String {
    "D55".into()
}
impl Default for ConvertFilmParams {
    fn default() -> Self {
        Self {
            scan_illuminant: default_convert_illuminant(),
            exposure_compensation_ev: 0.0,
            base_percentile: 99.0,
            calibration: default_calibration(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CmykParams {
    #[serde(default)]
    pub active: bool,
    #[serde(default = "default_one_f64")]
    pub strength: f64,
}
impl Default for CmykParams {
    fn default() -> Self {
        Self {
            active: false,
            strength: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowParams {
    #[serde(default = "default_route")]
    pub route: String,
}
impl Default for WorkflowParams {
    fn default() -> Self {
        Self {
            route: default_route(),
        }
    }
}
impl WorkflowParams {
    pub fn route_enum(&self) -> crate::WorkflowRoute {
        crate::WorkflowRoute::parse(&self.route)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilmRenderingParams {
    #[serde(default = "default_one")]
    pub density_curve_gamma: f32,
    #[serde(default)]
    pub development_time: Option<f64>,
    #[serde(default)]
    pub chemistry: PrintCurvesMorphParams,
    #[serde(default)]
    pub grain: GrainParams,
    #[serde(default)]
    pub halation: HalationParams,
    #[serde(default)]
    pub dir_couplers: DirCouplersParams,
    #[serde(default)]
    pub glare: GlareParams,
    #[serde(default)]
    pub base: FilmBaseParams,
    #[serde(default)]
    pub convert: ConvertFilmParams,
}
impl Default for FilmRenderingParams {
    fn default() -> Self {
        Self {
            density_curve_gamma: 1.0,
            development_time: None,
            chemistry: PrintCurvesMorphParams::default(),
            grain: GrainParams::default(),
            halation: HalationParams::default(),
            dir_couplers: DirCouplersParams::default(),
            glare: GlareParams::default(),
            base: FilmBaseParams::default(),
            convert: ConvertFilmParams::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintRenderingParams {
    #[serde(default = "default_one")]
    pub density_curve_gamma: f32,
    /// Development time (minutes) for B&W print stocks (e.g. kodak_2302) —
    /// same semantics as `FilmRenderingParams::development_time`.
    #[serde(default)]
    pub development_time: Option<f64>,
    #[serde(default)]
    pub glare: GlareParams,
    #[serde(
        default = "default_print_density_curves_morph",
        deserialize_with = "deserialize_print_density_curves_morph"
    )]
    pub density_curves_morph: PrintCurvesMorphParams,
    #[serde(default)]
    pub base: PrintBaseParams,
}

impl Default for PrintRenderingParams {
    fn default() -> Self {
        Self {
            density_curve_gamma: 1.0,
            development_time: None,
            glare: GlareParams::default(),
            density_curves_morph: default_print_density_curves_morph(),
            base: PrintBaseParams::default(),
        }
    }
}

fn default_color_filter() -> String {
    "none".into()
}

fn deserialize_color_filter<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
        .map(|value| value.unwrap_or_else(default_color_filter))
}

fn default_print_density_curves_morph() -> PrintCurvesMorphParams {
    PrintCurvesMorphParams {
        active: false,
        ..PrintCurvesMorphParams::default()
    }
}

fn deserialize_print_density_curves_morph<'de, D>(
    deserializer: D,
) -> Result<PrintCurvesMorphParams, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut value = serde_json::Value::deserialize(deserializer)?;
    if let Some(object) = value.as_object_mut() {
        object
            .entry("active")
            .or_insert(serde_json::Value::Bool(false));
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

/// Film and print chemistry controls. Kept in f64 for parity-sensitive curve
/// evaluation; both stages default to active in the experimental contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrintCurvesMorphParams {
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default = "default_one_f64")]
    pub gamma_factor: f64,
    #[serde(default = "default_one_f64")]
    pub gamma_factor_fast: f64,
    #[serde(default = "default_one_f64")]
    pub gamma_factor_slow: f64,
    #[serde(default = "default_one_f64")]
    pub gamma_factor_red: f64,
    #[serde(default = "default_one_f64")]
    pub gamma_factor_green: f64,
    #[serde(default = "default_one_f64")]
    pub gamma_factor_blue: f64,
    #[serde(default)]
    pub developer_exhaustion: f64,
}

impl Default for PrintCurvesMorphParams {
    fn default() -> Self {
        Self {
            active: true,
            gamma_factor: 1.0,
            gamma_factor_fast: 1.0,
            gamma_factor_slow: 1.0,
            gamma_factor_red: 1.0,
            gamma_factor_green: 1.0,
            gamma_factor_blue: 1.0,
            developer_exhaustion: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IoParams {
    #[serde(default = "default_prophoto")]
    pub input_color_space: String,
    #[serde(default)]
    pub input_cctf_decoding: bool,
    #[serde(default = "default_srgb")]
    pub output_color_space: String,
    #[serde(default = "default_true")]
    pub output_cctf_encoding: bool,
    #[serde(default)]
    pub crop: bool,
    #[serde(default = "default_crop_center")]
    pub crop_center: [f64; 2],
    #[serde(default = "default_crop_size")]
    pub crop_size: [f64; 2],
    #[serde(default = "default_one_f64")]
    pub upscale_factor: f64,
    #[serde(default)]
    pub scan_film: bool,
    #[serde(default)]
    pub output_gamut_compress: OutputGamutCompressParams,
    #[serde(default)]
    pub input_gamut_compress: InputGamutCompressParams,
}

impl Default for IoParams {
    fn default() -> Self {
        Self {
            input_color_space: "ProPhoto RGB".into(),
            input_cctf_decoding: false,
            output_color_space: "sRGB".into(),
            output_cctf_encoding: true,
            crop: false,
            crop_center: [0.5, 0.5],
            crop_size: [0.1, 0.1],
            upscale_factor: 1.0,
            scan_film: false,
            output_gamut_compress: OutputGamutCompressParams::default(),
            input_gamut_compress: InputGamutCompressParams::default(),
        }
    }
}

/// Input gamut compression config — mirrors upstream `InputGamutCompressSpec`.
/// Baked into the tc_lut at build time, so the per-pixel path stays
/// compression-agnostic. `active` (default true) is the upstream on/off flag;
/// `algorithm` selects `"xy"` (the production default — ACES-RGC-style radial
/// compression toward the spectral locus) or `"oklch"` (CSS-Color-4-style
/// chroma reduction around the film reference illuminant, kept for
/// inspection / per-bundle override).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputGamutCompressParams {
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default = "default_xy")]
    pub algorithm: String,
    #[serde(default = "default_gamut_boundary")]
    pub boundary: String,
    #[serde(default = "default_hull_detail")]
    pub hull_detail: f64,
    #[serde(default = "default_input_gamut_knee")]
    pub knee: [f32; 3],
}

impl Default for InputGamutCompressParams {
    fn default() -> Self {
        Self {
            active: true,
            algorithm: "xy".into(),
            boundary: "inscribed_hull".into(),
            hull_detail: 5.0,
            knee: [0.815, 1.0, 1.2],
        }
    }
}

/// Output gamut compression applies chroma reduction against the destination RGB cube.
/// `"off"` passes RGB through unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputGamutCompressParams {
    #[serde(default = "default_output_gamut_algorithm")]
    pub algorithm: String,
    #[serde(default = "default_gamut_knee")]
    pub knee: [f32; 3],
}

impl Default for OutputGamutCompressParams {
    fn default() -> Self {
        Self {
            algorithm: "oklch".into(),
            knee: [0.95, 1.0, 1.6],
        }
    }
}

fn default_gamut_boundary() -> String {
    "inscribed_hull".into()
}
fn default_hull_detail() -> f64 {
    5.0
}
fn default_xy() -> String {
    "xy".into()
}
fn default_output_gamut_algorithm() -> String {
    "oklch".into()
}
fn default_input_gamut_knee() -> [f32; 3] {
    [0.815, 1.0, 1.2]
}
fn default_gamut_knee() -> [f32; 3] {
    [0.95, 1.0, 1.6]
}

/// Overall effect amount in the common linear output color space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputMixParams {
    pub amount: f64,
}

impl Default for OutputMixParams {
    fn default() -> Self {
        Self { amount: 1.0 }
    }
}

impl OutputMixParams {
    pub fn validate(&self) -> Result<(), String> {
        self.validate_detailed().map_err(|error| error.to_string())
    }

    pub fn validate_detailed(&self) -> Result<(), ValidationError> {
        validation::validate_scalar_detailed("output_mix.amount", self.amount)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsParams {
    /// Working wavelength grid: [start_nm, end_nm, step_nm].
    #[serde(default = "crate::spectral_service::default_spectral_shape")]
    pub spectral_shape: [f64; 3],
    #[serde(default = "default_hanatos")]
    pub rgb_to_raw_method: String,
    #[serde(default = "default_true")]
    pub apply_hanatos2025_adaptation_window: bool,
    #[serde(default)]
    pub apply_hanatos2025_adaptation_surface: bool,
    #[serde(default)]
    pub spectral_gaussian_blur: f32,
    #[serde(default)]
    pub use_enlarger_lut: bool,
    #[serde(default)]
    pub use_scanner_lut: bool,
    #[serde(default = "default_17")]
    pub lut_resolution: u32,
    #[serde(default)]
    pub use_fast_stats: bool,
    #[serde(default = "default_640")]
    pub preview_max_size: u32,
    #[serde(default)]
    pub preview_mode: bool,
    #[serde(default = "default_true")]
    pub neutral_print_filters_from_database: bool,
    /// Use CAT16 (instead of CAT02) for the input RGB→tc chromatic adaptation
    /// that feeds the Hanatos spectral upsampling. Default true matches
    /// upstream 0.3.4 (`_rgb_to_tc_b` hard-codes CAT16); false restores the
    /// CAT02 0.3.2 behavior.
    #[serde(default = "default_true")]
    pub use_cat16: bool,
    /// Legacy carry-over of the two intermediate print-exposure behaviors
    /// that `enlarger.print_exposure_mode` cannot express. See
    /// `PrintExposureEvTracking`.
    #[serde(default)]
    pub print_exposure_ev_tracking: PrintExposureEvTracking,
}

impl Default for SettingsParams {
    fn default() -> Self {
        Self {
            spectral_shape: crate::spectral_service::default_spectral_shape(),
            rgb_to_raw_method: "hanatos2025".into(),
            apply_hanatos2025_adaptation_window: true,
            apply_hanatos2025_adaptation_surface: false,
            spectral_gaussian_blur: 0.0,
            use_enlarger_lut: false,
            use_scanner_lut: false,
            lut_resolution: 17,
            use_fast_stats: false,
            preview_max_size: 640,
            preview_mode: false,
            neutral_print_filters_from_database: true,
            use_cat16: true,
            print_exposure_ev_tracking: PrintExposureEvTracking::default(),
        }
    }
}

/// Debug switches — mirrors upstream 0.3.4 `DebugParams`.
///
/// `lut_mode` promotes the spatial/stochastic deactivation and disables the
/// image-aware adjustments (auto-exposure, print-exposure compensation,
/// highlight boost, scanner corrections) at digest time; the promoted flags
/// are visible on the digested params (see `params_builder::digest_params`).
///
/// `print_timings` is accepted, serialized and honored as a declared field
/// but is a no-op at digest time — upstream 0.3.4 declares it without ever
/// reading it (timing output goes through the `Simulator.print_timings()`
/// call-site flag instead), so it is preserved as an upstream no-op.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DebugParams {
    #[serde(default)]
    pub deactivate_spatial_effects: bool,
    #[serde(default)]
    pub deactivate_stochastic_effects: bool,
    #[serde(default)]
    pub print_timings: bool,
    #[serde(default)]
    pub lut_mode: bool,
}

/// Pipeline tap configuration — mirrors upstream 0.3.4 `TapsParams`.
///
/// `inject` and `collect` name the entry and exit points in the pipeline
/// topology (`rgb_in`, `rgb_pre`, `log_e_film`, `cmy_film`, `log_e_print`,
/// `cmy_print`, `rgb_out`). Defaults of `None` mean "normal end-to-end run"
/// (inject at `rgb_in`, collect at `rgb_out`); call-site overrides passed to
/// `Pipeline::process_with_taps` win over these persistent values.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TapsParams {
    #[serde(default)]
    pub inject: Option<String>,
    #[serde(default)]
    pub collect: Option<String>,
}

pub use spektrafilm_backend::Tap;

impl RuntimeParams {
    /// Validate enum-like string fields against the values the engine
    /// actually implements. The validation rules live in `validation` so
    /// this module remains the schema/default aggregation point.
    pub fn validate(&self) -> Result<(), String> {
        validation::validate(self)
    }

    /// Validate using the same rules while retaining semantic failures for adapters.
    pub fn validate_detailed(&self) -> Result<(), ValidationError> {
        validation::validate_detailed(self)
    }

    /// Validate canonical value domains without route or resource checks.
    pub fn validate_values(&self) -> Result<(), String> {
        self.validate_values_detailed()
            .map_err(|error| error.to_string())
    }

    /// Validate canonical value domains while retaining semantic failure details.
    pub fn validate_values_detailed(&self) -> Result<(), ValidationError> {
        validation::validate_values_detailed(self)
    }

    /// Record already-validated canonical leaf paths without changing filter protection.
    pub fn mark_explicit_paths<'a>(
        &mut self,
        paths: impl IntoIterator<Item = impl Into<std::borrow::Cow<'a, str>>>,
    ) {
        for path in paths {
            let path = path.into();
            if !self.explicit_section_paths.contains(path.as_ref()) {
                self.explicit_section_paths.insert(path.into_owned());
            }
        }
    }

    /// Release complete paths and their dot-delimited descendants.
    pub fn clear_explicit_paths(&mut self, scopes: &[&str]) -> bool {
        let before = self.explicit_section_paths.len();
        self.explicit_section_paths.retain(|path| {
            !scopes.iter().any(|scope| {
                path.as_str() == *scope
                    || path
                        .strip_prefix(*scope)
                        .is_some_and(|suffix| suffix.starts_with('.'))
            })
        });
        before != self.explicit_section_paths.len()
    }

    /// Compare leaf ownership without copying or normalizing filter flags.
    pub fn explicit_paths_match(&self, other: &Self) -> bool {
        self.explicit_section_paths == other.explicit_section_paths
    }

    /// Protect neutral filter axes explicitly supplied by a source.
    pub fn protect_neutral_filter_axes<'a>(&mut self, paths: impl IntoIterator<Item = &'a str>) {
        for path in paths {
            let axis = match path {
                "enlarger.c_filter_neutral" => 0,
                "enlarger.m_filter_neutral" => 1,
                "enlarger.y_filter_neutral" => 2,
                _ => continue,
            };
            self.neutral_print_filters_protected
                .get_or_insert([false; 3])[axis] = true;
        }
    }

    /// Snapshot preparation ownership for state carriers that keep it beside
    /// the parameters. The filter flags also persist in the parameter
    /// document itself so replays reproduce their render.
    pub fn render_preparation_state(&self) -> RenderPreparationState {
        RenderPreparationState {
            explicit_section_paths: self.explicit_section_paths.iter().cloned().collect(),
            neutral_print_filters_protected: self
                .neutral_print_filters_protected
                .unwrap_or([false; 3]),
        }
    }

    /// Restore ownership, ignoring leaves removed from the editable schema.
    /// Values and paths need not belong to any currently known preset section.
    pub fn restore_render_preparation_state(&mut self, state: &RenderPreparationState) {
        self.explicit_section_paths.clear();
        self.explicit_section_paths.extend(
            state
                .explicit_section_paths
                .iter()
                .map(|path| grain_compat::canonical_grain_path(path).into_owned())
                .filter(|path| sources::describe_field(path).is_ok()),
        );
        self.neutral_print_filters_protected = Some(state.neutral_print_filters_protected);
    }
}
/// Serde-visible GUI state carrier for runtime preparation ownership.
/// Kept beside the parameter document so state restoration can override the
/// persisted flags explicitly.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderPreparationState {
    #[serde(deserialize_with = "grain_compat::deserialize_preparation_paths")]
    pub explicit_section_paths: Vec<String>,
    pub neutral_print_filters_protected: [bool; 3],
}

/// Top-level runtime parameters. Combines all sub-parameter groups.
///
/// Mirrors upstream 0.3.4 `RuntimePhotoParams` minus the `film` / `print`
/// profile objects (Rust passes `Profile` values alongside the params).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeParams {
    /// Per-render stochastic seed. The recipe runner sets this out-of-band so
    /// it does not become part of the user parameter schema or digest.
    #[serde(skip)]
    pub random_seed: u64,
    /// CMY axes owned by a resolved look, explicit parameter edits, or
    /// restored state. Calibration uses database precision only for
    /// baseline (unprotected) axes. `None` marks documents without the
    /// flags; resolution then treats every axis as baseline unless a look,
    /// explicit edit, or restored state marked it authored.
    #[serde(default)]
    pub neutral_print_filters_protected: Option<[bool; 3]>,
    /// Editable leaves explicitly supplied by section documents. Stock initialization
    /// retains these values; legacy parameter decoding does not populate this set.
    #[serde(skip)]
    pub(crate) explicit_section_paths: std::collections::BTreeSet<String>,
    #[serde(default)]
    pub camera: CameraParams,
    #[serde(default)]
    pub enlarger: EnlargerParams,
    #[serde(default)]
    pub scanner: ScannerParams,
    #[serde(default)]
    pub film_render: FilmRenderingParams,
    #[serde(default)]
    pub print_render: PrintRenderingParams,
    #[serde(default)]
    pub io: IoParams,
    #[serde(default)]
    pub workflow: WorkflowParams,
    #[serde(default)]
    pub cmyk: CmykParams,
    #[serde(default)]
    pub output_mix: OutputMixParams,
    #[serde(default)]
    pub settings: SettingsParams,
    #[serde(default)]
    pub debug: DebugParams,
    #[serde(default)]
    pub taps: TapsParams,
}

impl RuntimeParams {
    /// Resolve colour transforms and reject unsupported gamut configurations
    /// before the pipeline can produce an image or LUT.
    pub fn validate_color(&self) -> Result<(), String> {
        self.validate_color_detailed()
            .map_err(|error| error.to_string())
    }

    fn validate_color_detailed(&self) -> Result<(), ValidationError> {
        spektrafilm_math::colorspace::resolve(&self.io.input_color_space).map_err(|error| {
            ValidationError::new(
                ValidationReason::ColorSpace,
                "io.input_color_space",
                format!("io.input_color_space: {error}"),
            )
            .arg("value", format!("{:?}", self.io.input_color_space))
        })?;
        spektrafilm_math::colorspace::resolve(&self.io.output_color_space).map_err(|error| {
            ValidationError::new(
                ValidationReason::ColorSpace,
                "io.output_color_space",
                format!("io.output_color_space: {error}"),
            )
            .arg("value", format!("{:?}", self.io.output_color_space))
        })?;
        crate::input_gamut::InputGamutCompress::validate_detailed(&self.io.input_gamut_compress)?;
        crate::gamut_compression::OutputGamutCompress::validate_detailed(
            &self.io.output_gamut_compress,
            &self.io.output_color_space,
        )?;
        Ok(())
    }
}

// Default value helpers
fn default_half() -> f32 {
    0.5
}
fn default_one() -> f32 {
    1.0
}
fn default_true() -> bool {
    true
}
fn default_center_weighted() -> String {
    "center_weighted".into()
}
fn default_35() -> f32 {
    35.0
}
fn default_filter_uv() -> [f32; 3] {
    [0.0, 410.0, 8.0]
}
fn default_filter_ir() -> [f32; 3] {
    [0.0, 675.0, 15.0]
}
fn default_th_kg3() -> String {
    "TH-KG3".into()
}
fn default_55() -> f32 {
    55.0
}
fn default_65() -> f32 {
    65.0
}
fn default_direct_scan() -> String {
    "direct_scan".into()
}

fn default_unsharp() -> [f64; 2] {
    [0.7, 0.7]
}
fn default_prophoto() -> String {
    "ProPhoto RGB".into()
}
fn default_srgb() -> String {
    "sRGB".into()
}
fn default_crop_center() -> [f64; 2] {
    [0.5, 0.5]
}
fn default_crop_size() -> [f64; 2] {
    [0.1, 0.1]
}
fn default_hanatos() -> String {
    "hanatos2025".into()
}
fn default_17() -> u32 {
    17
}
fn default_640() -> u32 {
    640
}
fn default_99() -> f64 {
    99.0
}
fn default_calibration() -> String {
    "1 0 0  0 1 0  0 0 1".into()
}
fn default_route() -> String {
    "input > film > print > scan".into()
}

#[cfg(test)]
mod tests {
    use super::{CameraParams, RuntimeParams};

    #[test]
    fn explicit_path_scopes_preserve_unrelated_paths_and_optional_filter_flags() {
        let mut params = RuntimeParams::default();
        let borrowed = "camera.auto_exposure".to_owned();
        params.mark_explicit_paths([borrowed.as_str(), "camera.auto_exposure_method"]);
        params.mark_explicit_paths([
            borrowed,
            "film_render.grain.uniformity".to_owned(),
            "film_render.halation.halation_strength".to_owned(),
        ]);
        let mut expected = RuntimeParams::default();
        expected.mark_explicit_paths([
            "camera.auto_exposure_method",
            "film_render.halation.halation_strength",
        ]);
        expected.neutral_print_filters_protected = Some([false; 3]);
        expected.mark_explicit_paths(["camera.auto_exposure"]);
        assert!(expected.clear_explicit_paths(&["camera.auto_exposure"]));

        assert!(params.clear_explicit_paths(&["camera.auto_exposure", "film_render.grain"]));
        assert!(!params.clear_explicit_paths(&["film_render.grain", "camera.auto"]));
        assert!(params.explicit_paths_match(&expected));
        assert_eq!(params.neutral_print_filters_protected, None);
        assert_eq!(expected.neutral_print_filters_protected, Some([false; 3]));

        params.protect_neutral_filter_axes(["camera.auto_exposure_method"]);
        assert_eq!(params.neutral_print_filters_protected, None);
        params.protect_neutral_filter_axes([
            "enlarger.m_filter_neutral",
            "enlarger.c_filter_neutral",
            "enlarger.m_filter_neutral",
        ]);
        assert_eq!(
            params.neutral_print_filters_protected,
            Some([true, true, false])
        );
        expected.protect_neutral_filter_axes(["enlarger.y_filter_neutral"]);
        assert_eq!(
            expected.neutral_print_filters_protected,
            Some([false, false, true])
        );
        assert!(params.explicit_paths_match(&expected));
    }

    #[test]
    fn operator_requires_finite_values_positive_contrast_and_standard_negative_route() {
        use super::ScanOperatorParams;
        let mut params = RuntimeParams::default();
        assert!(params.scanner.operator.is_default());
        assert!(!params.scanner.requires_full_frame());
        params.scanner.operator.exposure_ev = 0.5;
        assert!(params.validate().unwrap_err().contains("scanner.operator"));
        params.scanner.scan_output = "negative_to_positive".into();
        assert!(!params.scanner.requires_full_frame());
        assert!(
            params
                .validate()
                .unwrap_err()
                .contains("direct film scan route")
        );
        params.workflow.route = "input > film > scan".into();
        params.validate().unwrap();
        params.workflow.route = "input > convert-film > scan".into();
        params.validate().unwrap();
        params.scanner.conversion_style = "fixed-probe".into();
        assert!(!params.scanner.requires_full_frame());
        assert!(params.validate().unwrap_err().contains("scanner.operator"));
        params.scanner.operator = ScanOperatorParams::default();
        params.validate().unwrap();
        for field in 0..4 {
            for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                let mut operator = ScanOperatorParams::default();
                match field {
                    0 => operator.exposure_ev = invalid,
                    1 => operator.warmth_ev = invalid,
                    2 => operator.tint_ev = invalid,
                    _ => operator.contrast = invalid,
                }
                assert!(operator.validate().is_err());
            }
        }
        for contrast in [0.0, -1.0] {
            assert!(
                ScanOperatorParams {
                    contrast,
                    ..ScanOperatorParams::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            ScanOperatorParams {
                exposure_ev: 4.0,
                warmth_ev: -2.0,
                tint_ev: 2.0,
                contrast: 3.0
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn standard_is_canonical_for_defaults_legacy_sources_and_snapshots() {
        assert_eq!(super::ScannerParams::default().conversion_style, "standard");
        let mut params: RuntimeParams = serde_json::from_value(serde_json::json!({
            "scanner": {"conversion_style": "neutral"}
        }))
        .unwrap();
        assert_eq!(params.scanner.conversion_style, "standard");
        super::sources::ParameterEdits::parse("scanner.conversion_style=neutral")
            .unwrap()
            .apply(&mut params)
            .unwrap();
        assert_eq!(params.scanner.conversion_style, "standard");
        params.scanner.conversion_style = "neutral".into();
        assert_eq!(
            serde_json::to_value(&params).unwrap()["scanner"]["conversion_style"],
            "standard"
        );
        assert!(!params.scanner.experimental_setup);
        params.debug.lut_mode = true;
        params.scanner.scan_output = "negative_to_positive".into();
        params.validate().unwrap();
        params.scanner.experimental_setup = true;
        assert!(params.validate().unwrap_err().contains("point-wise LUT"));
        params.scanner.experimental_setup = false;
        params.scanner.conversion_style = "fixed-probe".into();
        params.validate().unwrap();
        assert!(
            serde_json::from_value::<super::ScannerParams>(
                serde_json::json!({"conversion_style": null})
            )
            .is_err()
        );
    }

    #[test]
    fn scanner_defaults_and_sparse_edits_are_compatible() {
        let legacy: super::ScannerParams = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.scan_output, "direct_scan");
        assert_eq!(legacy.lens_blur, 0.0);
        assert_eq!(legacy.unsharp_mask, [0.7, 0.7]);
        for (path, allowed) in [
            (
                "scanner.scan_output",
                &["direct_scan", "negative_to_positive"][..],
            ),
            ("enlarger.print_exposure_mode", &["manual", "auto"][..]),
            (
                "settings.print_exposure_ev_tracking",
                &["follow_mode", "never", "always"][..],
            ),
        ] {
            for value in allowed {
                assert!(super::sources::ParameterEdits::parse(&format!("{path}={value}")).is_ok());
            }
            assert!(super::sources::ParameterEdits::parse(&format!("{path}=unknown")).is_err());
        }
        let encoded = serde_json::to_value(&legacy).unwrap();
        assert_eq!(encoded["scan_output"], "direct_scan");
        for removed in [
            "receiver_profile",
            "negative_conversion_method",
            "auto_setup",
            "finish",
            "white_correction",
            "black_point",
        ] {
            assert!(encoded.get(removed).is_none());
        }
        let enlarger = serde_json::to_value(super::EnlargerParams::default()).unwrap();
        assert_eq!(enlarger["print_exposure_mode"], "auto");
        assert!(
            enlarger.get("normalize_print_exposure").is_none()
                && enlarger.get("print_exposure_compensation").is_none()
        );
        let settings = serde_json::to_value(super::SettingsParams::default()).unwrap();
        assert_eq!(settings["print_exposure_ev_tracking"], "follow_mode");
    }
    #[test]
    fn conversion_style_requires_negative_to_positive() {
        let mut params = RuntimeParams::default();
        params.scanner.conversion_style = "warm-probe".into();
        params.scanner.scan_output = "negative_to_positive".into();
        params.validate().unwrap();

        params.scanner.scan_output = "direct_scan".into();
        let error = params.validate().unwrap_err();
        assert!(
            error.contains("scanner.conversion_style") && error.contains("negative_to_positive"),
            "{error}"
        );

        params.scanner.conversion_style = "../etc/passwd".into();
        let error = params.validate().unwrap_err();
        assert!(error.contains("scanner.conversion_style"), "{error}");
    }
    #[test]
    fn null_or_missing_camera_color_filter_uses_no_filter() {
        let null: CameraParams = serde_json::from_str(r#"{"color_filter":null}"#).unwrap();
        assert_eq!(null.color_filter, "none");

        let missing: CameraParams = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.color_filter, "none");

        let named: CameraParams = serde_json::from_str(r#"{"color_filter":"hoya_r1"}"#).unwrap();
        assert_eq!(named.color_filter, "hoya_r1");
    }

    #[test]
    fn scanner_output_defaults_to_direct_and_rejects_removed_or_unknown_modes() {
        let legacy: super::ScannerParams = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.scan_output, "direct_scan");

        let mut params = RuntimeParams::default();
        params.scanner.scan_output = "negative_to_positive".into();
        params.validate().unwrap();
        let encoded = serde_json::to_string(&params).unwrap();
        let decoded: RuntimeParams = serde_json::from_str(&encoded).unwrap();
        decoded.validate().unwrap();
        assert_eq!(decoded.scanner.scan_output, "negative_to_positive");
        assert!(
            super::sources::ParameterEdits::parse("scanner.scan_output=negative_to_positive")
                .is_ok()
        );
        for mode in ["positive_scan", "unknown"] {
            params.scanner.scan_output = mode.into();
            let error = params.validate().unwrap_err();
            assert!(error.contains("scanner.scan_output"));
            assert!(
                super::sources::ParameterEdits::parse(&format!("scanner.scan_output={mode}"))
                    .is_err()
            );
        }
    }

    #[test]
    fn grain_procedural_profile_inheritance_and_overrides() {
        let mut params = super::RuntimeParams::default();
        let grain = &mut params.film_render.grain;
        grain.procedural_profile = "8mm500".into();
        grain.procedural_amount = Some(25.0);
        grain.procedural_mode = super::grain::ProceduralGrainMode::Noise;
        grain.procedural_film_type = super::grain::ProceduralGrainFilmType::Negative;
        grain.procedural_resolution_type = 0;
        grain.procedural_timer = 0.25;
        let preset = grain.resolved_grain_procedural();
        assert_eq!(preset.amount, 0.25);
        assert_eq!(
            preset.mode,
            spektrafilm_model::grain::procedural::ProceduralGrainMode::Analogue
        );
        assert_eq!(preset.resolution_type, 0);
        assert_eq!(preset.timer, Some(0.25));
        assert_eq!(preset.film_type, 0);
        grain.procedural_film_type = super::grain::ProceduralGrainFilmType::Positive;
        assert_eq!(grain.resolved_grain_procedural().film_type, 0);
        assert_eq!(grain.resolved_grain_procedural().resolution_type, 0);
        grain.select_custom_grain_procedural();
        grain.procedural_film_type = super::grain::ProceduralGrainFilmType::Positive;
        assert_eq!(grain.procedural_resolution_type, 0);
        assert_eq!(grain.procedural_timer, 0.25);
        assert_eq!(grain.procedural_amount, Some(25.0));
        assert_eq!(grain.procedural_size, Some(48.0));
        assert_eq!(grain.procedural_resolution_factor, Some(75.0));
        grain.procedural_amount = Some(25.0);
        grain.procedural_mode = super::grain::ProceduralGrainMode::Noise;
        grain.procedural_film_type = super::grain::ProceduralGrainFilmType::Negative;
        let roundtrip: super::RuntimeParams =
            serde_json::from_value(serde_json::to_value(&params).unwrap()).unwrap();
        let custom = roundtrip.film_render.grain.resolved_grain_procedural();
        assert_eq!(custom.amount, 0.25);
        assert_eq!(
            custom.mode,
            spektrafilm_model::grain::procedural::ProceduralGrainMode::Noise
        );
        assert_eq!(custom.resolution_type, 0);
        assert_eq!(custom.timer, Some(0.25));
        params.film_render.grain.procedural_profile = "35mm250".into();
        params.film_render.grain.procedural_amount = None;
        params.film_render.grain.select_custom_grain_procedural();
        assert_eq!(params.film_render.grain.procedural_amount, Some(35.0));
        assert_eq!(
            params.film_render.grain.procedural_mode,
            super::grain::ProceduralGrainMode::Analogue
        );
        for value in [0.0, 100.0] {
            params.film_render.grain.procedural_amount = Some(value);
            params.validate().unwrap();
        }
        for value in [f32::NAN, f32::INFINITY, -0.1, 100.1] {
            params.film_render.grain.procedural_amount = Some(value);
            assert!(params.validate().is_err());
        }
        params.film_render.grain.procedural_resolution_type = 2;
        assert!(params.validate().is_err());
        params.film_render.grain.procedural_resolution_type = 0;
        params.film_render.grain.procedural_timer = 1.0;
        assert!(params.validate().is_err());
        params.film_render.grain.procedural_timer = f32::NAN;
        assert!(params.validate().is_err());
        params.film_render.grain.procedural_timer = 0.0;
        params.film_render.grain.procedural_amount = None;
        params.film_render.grain.procedural_profile = "unknown".into();
        assert!(params.validate().is_err());
    }
    #[test]
    fn legacy_zero_timer_keeps_seed_derived_phase() {
        let mut value = serde_json::to_value(super::RuntimeParams::default()).unwrap();
        value["film_render"]["grain"]["procedural_timer"] = serde_json::json!(0.0);
        let params: super::RuntimeParams = serde_json::from_value(value).unwrap();
        assert_eq!(params.film_render.grain.procedural_timer, 0.0);
        assert_eq!(
            params.film_render.grain.resolved_grain_procedural().timer,
            None
        );
    }
    #[test]
    fn dye_engine_and_controls_round_trip() {
        let mut value = serde_json::to_value(super::RuntimeParams::default()).unwrap();
        // New runtime recipes author Dye explicitly.
        let default: super::RuntimeParams = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            default.film_render.grain.engine,
            super::grain::GrainEngine::Dye
        );
        assert_eq!(default.film_render.grain.dye_support_um, 3.5);
        assert_eq!(default.film_render.grain.dye_event_area_um2, 1.28);
        assert_eq!(default.film_render.grain.dye_strength, 1.0);
        assert_eq!(default.film_render.grain.dye_scale, 1.0);
        assert_eq!(default.film_render.grain.channel_correlation, 0.0);
        // Recipes saved before the Dye event area and the channel
        // correlation existed get their defaults.
        let mut legacy = value.clone();
        legacy["film_render"]["grain"]
            .as_object_mut()
            .unwrap()
            .remove("dye_event_area_um2");
        legacy["film_render"]["grain"]
            .as_object_mut()
            .unwrap()
            .remove("channel_correlation");
        legacy["film_render"]["grain"]
            .as_object_mut()
            .unwrap()
            .remove("dye_strength");
        legacy["film_render"]["grain"]
            .as_object_mut()
            .unwrap()
            .remove("dye_scale");
        let legacy: super::RuntimeParams = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.film_render.grain.dye_event_area_um2, 1.28);
        assert_eq!(legacy.film_render.grain.dye_strength, 1.0);
        assert_eq!(legacy.film_render.grain.dye_scale, 1.0);
        assert_eq!(legacy.film_render.grain.channel_correlation, 0.0);
        value["film_render"]["grain"]["engine"] = serde_json::json!("dye");
        value["film_render"]["grain"]["dye_support_um"] = serde_json::json!(4.5);
        value["film_render"]["grain"]["dye_event_area_um2"] = serde_json::json!(0.6);
        value["film_render"]["grain"]["channel_correlation"] = serde_json::json!(0.35);
        let params: super::RuntimeParams = serde_json::from_value(value).unwrap();
        let reparsed: super::RuntimeParams =
            serde_json::from_value(serde_json::to_value(&params).unwrap()).unwrap();
        for grain in [&params.film_render.grain, &reparsed.film_render.grain] {
            assert_eq!(grain.engine, super::grain::GrainEngine::Dye);
            assert_eq!(grain.dye_support_um, 4.5);
            assert_eq!(grain.dye_event_area_um2, 0.6);
            assert_eq!(grain.channel_correlation, 0.35);
        }
        assert!(serde_json::from_str::<super::GrainParams>(r#"{"engine":"unknown"}"#).is_err());
    }
    #[test]
    fn dye_controls_must_be_finite_and_positive() {
        let mut params = super::RuntimeParams::default();
        params.film_render.grain.engine = super::grain::GrainEngine::Dye;
        params.validate().unwrap();
        for value in [0.0, -0.5, f32::NAN, f32::INFINITY] {
            let mut bad = params.clone();
            bad.film_render.grain.dye_support_um = value;
            let error = bad.validate().unwrap_err();
            assert!(
                error.contains("film_render.grain.dye_support_um"),
                "unexpected error for {value}: {error}"
            );
            let mut bad = params.clone();
            bad.film_render.grain.dye_event_area_um2 = f64::from(value);
            let error = bad.validate().unwrap_err();
            assert!(
                error.contains("film_render.grain.dye_event_area_um2"),
                "unexpected error for {value}: {error}"
            );
        }
        // Preserve the historical supported and acceptance range, not an
        // identified material limit or scanner-processing attribution.
        for value in [-0.5, 0.4500001, 1.0, f64::NAN, f64::INFINITY] {
            let mut bad = params.clone();
            bad.film_render.grain.channel_correlation = value;
            let error = bad.validate().unwrap_err();
            assert!(
                error.contains("film_render.grain.channel_correlation"),
                "unexpected error for {value}: {error}"
            );
        }
        for value in [0.0, 0.342, 0.45] {
            let mut ok = params.clone();
            ok.film_render.grain.channel_correlation = value;
            ok.validate().unwrap();
        }
        // The Dye look controls keep an explicit zero strength and a
        // positive scale; values outside the declared domain fail.
        for value in [0.0, 0.5, 1.0, 4.0] {
            let mut ok = params.clone();
            ok.film_render.grain.dye_strength = value;
            ok.validate().unwrap();
        }
        for value in [-0.1, 4.1, f64::NAN, f64::INFINITY] {
            let mut bad = params.clone();
            bad.film_render.grain.dye_strength = value;
            let error = bad.validate().unwrap_err();
            assert!(
                error.contains("film_render.grain.dye_strength"),
                "unexpected error for {value}: {error}"
            );
        }
        for value in [0.5, 1.0, 4.0] {
            let mut ok = params.clone();
            ok.film_render.grain.dye_scale = value;
            ok.validate().unwrap();
        }
        for value in [0.49, 4.01, f64::NAN, f64::INFINITY] {
            let mut bad = params.clone();
            bad.film_render.grain.dye_scale = value;
            let error = bad.validate().unwrap_err();
            assert!(
                error.contains("film_render.grain.dye_scale"),
                "unexpected error for {value}: {error}"
            );
        }
        // Spektra ignores the controls, but stored values stay validated so a
        // later switch to Dye cannot render from an invalid recipe.
        params.film_render.grain.engine = super::grain::GrainEngine::Spektra;
        params.film_render.grain.rms_granularity = [6.0, 8.0, 10.0];
        params.film_render.grain.dye_event_area_um2 = -1.0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn capture_illuminant_defaults_to_none_and_roundtrips_missing_and_null() {
        let mut params = super::RuntimeParams::default();
        assert_eq!(params.camera.capture_illuminant, None);

        // A document without the leaf deserializes to None.
        let restored: super::RuntimeParams =
            serde_json::from_str(r#"{"camera":{"film_format_mm":35.0}}"#).unwrap();
        assert_eq!(restored.camera.capture_illuminant, None);
        // An explicit null deserializes to None as well.
        let restored: super::RuntimeParams =
            serde_json::from_str(r#"{"camera":{"capture_illuminant":null}}"#).unwrap();
        assert_eq!(restored.camera.capture_illuminant, None);

        params.camera.capture_illuminant = Some("BB4000".into());
        params.validate().unwrap();
        let saved = serde_json::to_vec(&params).unwrap();
        let restored: super::RuntimeParams = serde_json::from_slice(&saved).unwrap();
        assert_eq!(
            restored.camera.capture_illuminant.as_deref(),
            Some("BB4000")
        );
        restored.validate().unwrap();

        // None stays valid and preserves the historical behavior.
        params.camera.capture_illuminant = None;
        params.validate().unwrap();
    }

    #[test]
    fn capture_illuminant_rejects_unsupported_explicit_values() {
        let mut params = super::RuntimeParams::default();
        for invalid in ["", "not-an-illuminant", "BB1000", "BB25001", "bb4000"] {
            params.camera.capture_illuminant = Some(invalid.into());
            let error = params.validate().unwrap_err();
            assert!(
                error.starts_with("camera.capture_illuminant:"),
                "unexpected error for {invalid:?}: {error}"
            );
        }
        // None and every supported spelling validate.
        params.camera.capture_illuminant = None;
        params.validate().unwrap();
        for valid in [
            "D65", "T", "TH-KG3", "TH-KG3-L", "K75P", "BB4000", "BB25000",
        ] {
            params.camera.capture_illuminant = Some(valid.into());
            params.validate().unwrap();
        }
    }

    #[test]
    fn film_format_validation_and_persistence() {
        let mut params = super::RuntimeParams::default();
        assert_eq!(params.camera.film_format_mm, 35.0);
        params.film_render.grain.rms_granularity = [6.0, 8.0, 10.0];
        for engine in [
            super::GrainEngine::Spektra,
            super::GrainEngine::Procedural,
            super::GrainEngine::Dye,
        ] {
            params.film_render.grain.engine = engine;
            for mm in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                params.camera.film_format_mm = mm;
                let error = params.validate().unwrap_err();
                assert!(error.contains("camera.film_format_mm"), "{error}");
            }
            for mm in [8.0, 35.0, 35.5, 120.0, 150.25] {
                params.camera.film_format_mm = mm;
                params.validate().unwrap();
                let saved = serde_json::to_vec(&params).unwrap();
                let restored: super::RuntimeParams = serde_json::from_slice(&saved).unwrap();
                restored.validate().unwrap();
                assert_eq!(restored.camera.film_format_mm, mm);
                assert_eq!(restored.film_render.grain.engine, engine);
            }
        }
    }

    #[test]
    fn cmyk_params_default_and_roundtrip() {
        let params = super::RuntimeParams::default();
        assert!(!params.cmyk.active);
        assert_eq!(params.cmyk.strength, 1.0);
        // Serialization uses the canonical `cmyk` group name.
        let value = serde_json::to_value(&params).unwrap();
        assert_eq!(value["cmyk"]["active"], false);
        assert_eq!(value["cmyk"]["strength"], 1.0);
        assert!(value.get("magazine_print_color").is_none());
        // Round-trip through JSON preserves the canonical fields.
        let restored: super::RuntimeParams = serde_json::from_value(value).unwrap();
        assert_eq!(restored.cmyk, params.cmyk);
        // Sparse edits address the canonical paths.
        let mut edited = super::RuntimeParams::default();
        super::sources::ParameterEdits::parse("cmyk.active=true,cmyk.strength=0.5")
            .unwrap()
            .apply(&mut edited)
            .unwrap();
        assert!(edited.cmyk.active);
        assert_eq!(edited.cmyk.strength, 0.5);
        edited.validate().unwrap();
    }
}
