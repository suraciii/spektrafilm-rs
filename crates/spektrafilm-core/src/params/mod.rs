/// Runtime parameters for the film simulation pipeline.
///
/// Mirrors Python `params_schema.py`. Every field has a sensible default
/// matching the Python implementation.
use serde::{Deserialize, Serialize};

pub mod couplers;
pub mod diffusion;
pub mod glare;
pub mod grain;
pub mod halation;
pub mod sources;

pub(crate) mod validation;
use couplers::DirCouplersParams;
pub use diffusion::DiffusionFilterParams;
use glare::GlareParams;
pub use grain::{FotufilmGrainParams, FotufilmMaterial};
pub use grain::{GrainEngine, GrainParams};
pub use halation::HalationParams;

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
    #[serde(default = "default_true")]
    pub print_exposure_compensation: bool,
    #[serde(default = "default_true")]
    pub normalize_print_exposure: bool,
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
            print_exposure_compensation: true,
            normalize_print_exposure: true,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiverProfile {
    #[default]
    LegacyColorimetric,
    LabScanApprox,
}
impl ReceiverProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyColorimetric => "legacy_colorimetric",
            Self::LabScanApprox => "lab_scan_approx",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositiveInterpretation {
    /// Read record densities directly: the film's own monotone characteristic
    /// curves, with no synthetic capture in the loop.
    #[default]
    Density,
    /// Interpret a receiver capture through a per-channel monotone inverse.
    Separable,
    /// Interpret a receiver capture through the three-channel coupled inverse.
    Coupled,
}
impl PositiveInterpretation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Density => "density",
            Self::Separable => "separable",
            Self::Coupled => "coupled",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScannerAutoSetup {
    #[default]
    Off,
    PerFrame,
}
impl ScannerAutoSetup {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::PerFrame => "per_frame",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScannerFinish {
    #[default]
    Neutral,
    LabScan,
}
impl ScannerFinish {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::LabScan => "lab_scan",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScannerParams {
    #[serde(default)]
    pub receiver_profile: ReceiverProfile,
    #[serde(default)]
    pub positive_interpretation: PositiveInterpretation,
    #[serde(default)]
    pub auto_setup: ScannerAutoSetup,
    #[serde(default)]
    pub finish: ScannerFinish,
    #[serde(default = "default_finish_strength")]
    pub finish_strength: f64,
    #[serde(default)]
    pub lens_blur: f32,
    /// Scan output polarity: `direct_scan` preserves the scanner capture;
    /// `negative_to_positive` interprets each negative dye record's density as a
    /// positive image.
    #[serde(default = "default_direct_scan")]
    pub scan_output: String,
    #[serde(default)]
    pub white_correction: bool,
    #[serde(default)]
    pub black_correction: bool,
    #[serde(default = "default_098")]
    pub white_level: f32,
    #[serde(default = "default_001")]
    pub black_level: f32,
    #[serde(default = "default_unsharp")]
    pub unsharp_mask: [f64; 2],
    #[serde(default)]
    pub black_point: [f64; 3],
    #[serde(default = "default_scanner_white_point")]
    pub white_point: [f64; 3],
    #[serde(default = "default_positive_density_ev")]
    pub positive_density_ev: f64,
    #[serde(default)]
    pub positive_balance_ev: [f64; 3],
    #[serde(default = "default_positive_contrast")]
    pub positive_contrast: f64,
    #[serde(default = "default_positive_dmax")]
    pub positive_dmax: f64,
}

impl Default for ScannerParams {
    fn default() -> Self {
        Self {
            receiver_profile: ReceiverProfile::default(),
            positive_interpretation: PositiveInterpretation::default(),
            auto_setup: ScannerAutoSetup::default(),
            finish: ScannerFinish::default(),
            finish_strength: default_finish_strength(),
            lens_blur: 0.0,
            scan_output: default_direct_scan(),
            white_correction: false,
            black_correction: false,
            white_level: 0.98,
            black_level: 0.01,
            unsharp_mask: [0.7, 0.7],
            black_point: [0.0; 3],
            white_point: default_scanner_white_point(),
            positive_density_ev: default_positive_density_ev(),
            positive_balance_ev: [0.0; 3],
            positive_contrast: default_positive_contrast(),
            positive_dmax: default_positive_dmax(),
        }
    }
}

fn default_finish_strength() -> f64 {
    1.0
}

fn default_scanner_white_point() -> [f64; 3] {
    [1.0; 3]
}

fn default_positive_density_ev() -> f64 {
    0.8781766618
}
fn default_positive_contrast() -> f64 {
    0.73551400058216643
}
fn default_positive_dmax() -> f64 {
    1.5625841465572565
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
    #[serde(default = "default_d55")]
    pub scan_illuminant: String,
    #[serde(default)]
    pub exposure_compensation_ev: f64,
    #[serde(default = "default_99")]
    pub base_percentile: f64,
    #[serde(default = "default_calibration")]
    pub calibration: String,
}
impl Default for ConvertFilmParams {
    fn default() -> Self {
        Self {
            scan_illuminant: "D55".into(),
            exposure_compensation_ev: 0.0,
            base_percentile: 99.0,
            calibration: default_calibration(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MagazinePrintColorParams {
    #[serde(default)]
    pub active: bool,
    #[serde(default = "default_one_f64")]
    pub strength: f64,
}
impl Default for MagazinePrintColorParams {
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

/// Output gamut compression config — mirrors upstream `OutputGamutCompressSpec`.
/// `"oklch"` (the upstream default) applies perceptual OkLab chroma reduction
/// against the destination RGB cube plus one-sided lightness roll-off;
/// `"off"` passes RGB through unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputGamutCompressParams {
    #[serde(default = "default_output_gamut_algorithm")]
    pub algorithm: String,
    #[serde(default = "default_gamut_knee")]
    pub knee: [f32; 3],
    #[serde(default = "default_gamut_lightness")]
    pub lightness_compression: Option<[f32; 3]>,
}

impl Default for OutputGamutCompressParams {
    fn default() -> Self {
        Self {
            algorithm: "oklch".into(),
            knee: [0.95, 1.0, 1.6],
            lightness_compression: Some([0.95, 1.0, 1.6]),
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
fn default_gamut_lightness() -> Option<[f32; 3]> {
    Some([0.95, 1.0, 1.6])
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

impl RuntimeParams {
    /// Validate enum-like string fields against the values the engine
    /// actually implements. The validation rules live in `validation` so
    /// this module remains the schema/default aggregation point.
    pub fn validate(&self) -> Result<(), String> {
        validation::validate(self)
    }

    /// Snapshot transient preparation ownership separately from the runtime schema.
    pub fn render_preparation_state(&self) -> RenderPreparationState {
        RenderPreparationState {
            explicit_section_paths: self.explicit_section_paths.iter().cloned().collect(),
            neutral_print_filters_protected: self.neutral_print_filters_protected,
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
                .filter(|path| sources::describe_field(path).is_ok())
                .cloned(),
        );
        self.neutral_print_filters_protected = state.neutral_print_filters_protected;
    }
}
/// Serde-visible GUI state carrier for runtime preparation ownership.
/// Kept separate so runtime parameters, recipes and digests retain their shape.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderPreparationState {
    pub explicit_section_paths: Vec<String>,
    pub neutral_print_filters_protected: [bool; 3],
}

/// Select the complete simulation graph. Historical documents default to SpektraFilm.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationEngine {
    #[default]
    Spektrafilm,
    Fotufilm,
}

/// Top-level runtime parameters. Combines all sub-parameter groups.
///
/// Mirrors upstream 0.3.4 `RuntimePhotoParams` minus the `film` / `print`
/// profile objects (Rust passes `Profile` values alongside the params).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeParams {
    #[serde(default)]
    pub engine: SimulationEngine,
    #[serde(default)]
    pub fotufilm: spektrafilm_fotufilm::Request,
    /// Per-render stochastic seed. The recipe runner sets this out-of-band so
    /// it does not become part of the user parameter schema or digest.
    #[serde(skip)]
    pub random_seed: u64,
    /// CMY axes owned by a resolved look or explicit parameter edits.
    /// Calibration uses database precision only for unprotected axes.
    #[serde(skip)]
    pub(crate) neutral_print_filters_protected: [bool; 3],
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
    pub magazine_print_color: MagazinePrintColorParams,
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
        spektrafilm_math::colorspace::resolve(&self.io.input_color_space)
            .map_err(|e| format!("io.input_color_space: {e}"))?;
        spektrafilm_math::colorspace::resolve(&self.io.output_color_space)
            .map_err(|e| format!("io.output_color_space: {e}"))?;
        if self.engine == SimulationEngine::Fotufilm {
            return Ok(());
        }
        crate::input_gamut::InputGamutCompress::build(&self.io.input_gamut_compress)?;
        crate::gamut_compression::OutputGamutCompress::build(
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

fn default_098() -> f32 {
    0.98
}
fn default_001() -> f32 {
    0.01
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
fn default_d55() -> String {
    "D55".into()
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
    fn scanner_selector_defaults_and_sparse_edits_are_compatible() {
        let legacy: super::ScannerParams = serde_json::from_str("{}").unwrap();
        assert_eq!(
            legacy.receiver_profile,
            super::ReceiverProfile::LegacyColorimetric
        );
        assert_eq!(
            legacy.positive_interpretation,
            super::PositiveInterpretation::Density
        );
        assert_eq!(legacy.auto_setup, super::ScannerAutoSetup::Off);
        assert_eq!(legacy.finish, super::ScannerFinish::Neutral);
        assert_eq!(legacy.finish_strength, 1.0);
        for (path, allowed) in [
            (
                "scanner.receiver_profile",
                &["legacy_colorimetric", "lab_scan_approx"][..],
            ),
            (
                "scanner.positive_interpretation",
                &["density", "separable", "coupled"][..],
            ),
            ("scanner.auto_setup", &["off", "per_frame"][..]),
            ("scanner.finish", &["neutral", "lab_scan"][..]),
        ] {
            for value in allowed {
                assert!(super::sources::ParameterEdits::parse(&format!("{path}={value}")).is_ok());
            }
            assert!(super::sources::ParameterEdits::parse(&format!("{path}=unknown")).is_err());
        }
        assert!(super::sources::ParameterEdits::parse("scanner.finish_strength=1.1").is_err());
        assert!(super::sources::ParameterEdits::parse("scanner.finish_strength=-0.1").is_err());
        let encoded = serde_json::to_value(&legacy).unwrap();
        assert_eq!(encoded["receiver_profile"], "legacy_colorimetric");
        assert_eq!(encoded["positive_interpretation"], "density");
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
    fn scanner_points_validate_each_channel_without_partial_pairs() {
        let mut params = RuntimeParams::default();
        params.scanner.black_point = [0.0, 0.2, 0.4];
        params.scanner.white_point = [0.5, 0.7, 1.0];
        params.validate().unwrap();
        for channel in 0..3 {
            let valid = params.scanner.clone();
            for value in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
                params.scanner.black_point[channel] = value;
                assert!(params.validate().is_err());
                params.scanner = valid.clone();
                params.scanner.white_point[channel] = value;
                assert!(params.validate().is_err());
                params.scanner = valid.clone();
            }
            params.scanner.black_point[channel] = params.scanner.white_point[channel];
            assert!(params.validate().is_err());
            params.scanner.black_point[channel] += 0.1;
            assert!(params.validate().is_err());
            params.scanner = valid;
        }
    }
    #[test]
    fn grain_v2_profile_inheritance_and_overrides() {
        let mut params = super::RuntimeParams::default();
        let grain = &mut params.film_render.grain;
        grain.v2_profile = "8mm500".into();
        grain.v2_amount = Some(25.0);
        grain.v2_mode = super::grain::GrainV2Mode::Noise;
        grain.v2_film_type = super::grain::GrainV2FilmType::Negative;
        grain.v2_resolution_type = 0;
        grain.v2_timer = 0.25;
        let preset = grain.resolved_grain_v2();
        assert_eq!(preset.amount, 0.25);
        assert_eq!(
            preset.mode,
            spektrafilm_model::grain::v2::GrainV2Mode::Analogue
        );
        assert_eq!(preset.resolution_type, 0);
        assert_eq!(preset.timer, Some(0.25));
        assert_eq!(preset.film_type, 0);
        grain.v2_film_type = super::grain::GrainV2FilmType::Positive;
        assert_eq!(grain.resolved_grain_v2().film_type, 0);
        assert_eq!(grain.resolved_grain_v2().resolution_type, 0);
        grain.select_custom_grain_v2();
        grain.v2_film_type = super::grain::GrainV2FilmType::Positive;
        assert_eq!(grain.v2_resolution_type, 0);
        assert_eq!(grain.v2_timer, 0.25);
        assert_eq!(grain.v2_amount, Some(25.0));
        assert_eq!(grain.v2_size, Some(48.0));
        assert_eq!(grain.v2_resolution_factor, Some(75.0));
        grain.v2_amount = Some(25.0);
        grain.v2_mode = super::grain::GrainV2Mode::Noise;
        grain.v2_film_type = super::grain::GrainV2FilmType::Negative;
        let roundtrip: super::RuntimeParams =
            serde_json::from_value(serde_json::to_value(&params).unwrap()).unwrap();
        let custom = roundtrip.film_render.grain.resolved_grain_v2();
        assert_eq!(custom.amount, 0.25);
        assert_eq!(
            custom.mode,
            spektrafilm_model::grain::v2::GrainV2Mode::Noise
        );
        assert_eq!(custom.resolution_type, 0);
        assert_eq!(custom.timer, Some(0.25));
        params.film_render.grain.v2_profile = "35mm250".into();
        params.film_render.grain.v2_amount = None;
        params.film_render.grain.select_custom_grain_v2();
        assert_eq!(params.film_render.grain.v2_amount, Some(35.0));
        assert_eq!(
            params.film_render.grain.v2_mode,
            super::grain::GrainV2Mode::Analogue
        );
        for value in [0.0, 100.0] {
            params.film_render.grain.v2_amount = Some(value);
            params.validate().unwrap();
        }
        for value in [f32::NAN, f32::INFINITY, -0.1, 100.1] {
            params.film_render.grain.v2_amount = Some(value);
            assert!(params.validate().is_err());
        }
        params.film_render.grain.v2_resolution_type = 2;
        assert!(params.validate().is_err());
        params.film_render.grain.v2_resolution_type = 0;
        params.film_render.grain.v2_timer = 1.0;
        assert!(params.validate().is_err());
        params.film_render.grain.v2_timer = f32::NAN;
        assert!(params.validate().is_err());
        params.film_render.grain.v2_timer = 0.0;
        params.film_render.grain.v2_amount = None;
        params.film_render.grain.v2_profile = "unknown".into();
        assert!(params.validate().is_err());
    }
    #[test]
    fn legacy_zero_timer_keeps_seed_derived_phase() {
        let mut value = serde_json::to_value(super::RuntimeParams::default()).unwrap();
        value["film_render"]["grain"]["v2_timer"] = serde_json::json!(0.0);
        let params: super::RuntimeParams = serde_json::from_value(value).unwrap();
        assert_eq!(params.film_render.grain.v2_timer, 0.0);
        assert_eq!(params.film_render.grain.resolved_grain_v2().timer, None);
    }
    #[test]
    fn v3_engine_and_controls_round_trip() {
        let mut value = serde_json::to_value(super::RuntimeParams::default()).unwrap();
        // Spektra stays the default for existing recipes.
        let default: super::RuntimeParams = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            default.film_render.grain.engine,
            super::grain::GrainEngine::Spektra
        );
        assert_eq!(default.film_render.grain.v3_dye_support_um, 3.5);
        assert_eq!(default.film_render.grain.v3_event_area_um2, 1.28);
        // A recipe saved before the Dye event area existed gets its default.
        let mut legacy = value.clone();
        legacy["film_render"]["grain"]
            .as_object_mut()
            .unwrap()
            .remove("v3_event_area_um2");
        let legacy: super::RuntimeParams = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.film_render.grain.v3_event_area_um2, 1.28);
        value["film_render"]["grain"]["engine"] = serde_json::json!("dye");
        value["film_render"]["grain"]["v3_dye_support_um"] = serde_json::json!(4.5);
        value["film_render"]["grain"]["v3_event_area_um2"] = serde_json::json!(0.6);
        let params: super::RuntimeParams = serde_json::from_value(value).unwrap();
        let reparsed: super::RuntimeParams =
            serde_json::from_value(serde_json::to_value(&params).unwrap()).unwrap();
        for grain in [&params.film_render.grain, &reparsed.film_render.grain] {
            assert_eq!(grain.engine, super::grain::GrainEngine::Dye);
            assert_eq!(grain.v3_dye_support_um, 4.5);
            assert_eq!(grain.v3_event_area_um2, 0.6);
        }
        assert!(serde_json::from_str::<super::GrainParams>(r#"{"engine":"unknown"}"#).is_err());
    }
    #[test]
    fn v3_controls_must_be_finite_and_positive() {
        let mut params = super::RuntimeParams::default();
        params.film_render.grain.engine = super::grain::GrainEngine::Dye;
        params.validate().unwrap();
        for value in [0.0, -0.5, f32::NAN, f32::INFINITY] {
            let mut bad = params.clone();
            bad.film_render.grain.v3_dye_support_um = value;
            let error = bad.validate().unwrap_err();
            assert!(
                error.contains("film_render.grain.v3_dye_support_um"),
                "unexpected error for {value}: {error}"
            );
            let mut bad = params.clone();
            bad.film_render.grain.v3_event_area_um2 = f64::from(value);
            let error = bad.validate().unwrap_err();
            assert!(
                error.contains("film_render.grain.v3_event_area_um2"),
                "unexpected error for {value}: {error}"
            );
        }
        // Spektra ignores the controls, but stored values stay validated so a
        // later switch to Dye cannot render from an invalid recipe.
        params.film_render.grain.engine = super::grain::GrainEngine::Spektra;
        params.film_render.grain.v3_event_area_um2 = -1.0;
        assert!(params.validate().is_err());
    }

    #[test]
    fn film_format_validation_and_persistence() {
        let mut params = super::RuntimeParams::default();
        assert_eq!(params.camera.film_format_mm, 35.0);
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
}
