use crate::{
    params::{RuntimeParams, ScanOperatorParams},
    profile::Profile,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSpaceRole {
    Srgb,
    DisplayP3,
    Rec2020,
    AdobeRgb,
    ProphotoRgb,
    AcesCg,
    Aces2065,
    Xyz,
    Custom,
}

impl ColorSpaceRole {
    pub fn parse(value: &str) -> Self {
        let normalized: String = value
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        match normalized.as_str() {
            "srgb" => Self::Srgb,
            "displayp3" | "p3" => Self::DisplayP3,
            "rec2020" | "bt2020" | "iturbt2020" => Self::Rec2020,
            "adobergb" | "adobergb1998" => Self::AdobeRgb,
            "prophoto" | "prophotorgb" => Self::ProphotoRgb,
            "acescg" => Self::AcesCg,
            "aces2065" | "aces20651" => Self::Aces2065,
            "xyz" | "ciexyz" => Self::Xyz,
            _ => Self::Custom,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRoute {
    Input,
    FilmScan,
    FilmPrintScan,
    InputFilm,
    InputFilmPrint,
    StoredFilmScan,
    StoredFilmPrint,
    StoredFilmPrintScan,
    StoredPrintScan,
    ConvertFilmPrintScan,
    ConvertFilmScanMinusBase,
    ConvertFilmScan,
    Custom,
}

impl WorkflowRoute {
    pub fn parse(value: &str) -> Self {
        let mut nodes = value
            .split('>')
            .map(|node| node.trim_matches(|c: char| c.is_ascii_whitespace()));
        match (
            nodes.next(),
            nodes.next(),
            nodes.next(),
            nodes.next(),
            nodes.next(),
        ) {
            (Some("input"), None, None, None, None) => Self::Input,
            (Some("input"), Some("film"), Some("scan"), None, None) => Self::FilmScan,
            (Some("input"), Some("film"), Some("print"), Some("scan"), None) => Self::FilmPrintScan,
            (Some("input"), Some("film"), None, None, None) => Self::InputFilm,
            (Some("input"), Some("film"), Some("print"), None, None) => Self::InputFilmPrint,
            (Some("film"), Some("scan"), None, None, None) => Self::StoredFilmScan,
            (Some("film"), Some("print"), None, None, None) => Self::StoredFilmPrint,
            (Some("film"), Some("print"), Some("scan"), None, None) => Self::StoredFilmPrintScan,
            (Some("print"), Some("scan"), None, None, None) => Self::StoredPrintScan,
            (Some("input"), Some("convert-film"), Some("print"), Some("scan"), None) => {
                Self::ConvertFilmPrintScan
            }
            (Some("input"), Some("convert-film"), Some("scan-minus-base"), None, None) => {
                Self::ConvertFilmScanMinusBase
            }
            (Some("input"), Some("convert-film"), Some("scan"), None, None) => {
                Self::ConvertFilmScan
            }
            _ => Self::Custom,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::FilmScan => "input > film > scan",
            Self::FilmPrintScan => "input > film > print > scan",
            Self::InputFilm => "input > film",
            Self::InputFilmPrint => "input > film > print",
            Self::StoredFilmScan => "film > scan",
            Self::StoredFilmPrint => "film > print",
            Self::StoredFilmPrintScan => "film > print > scan",
            Self::StoredPrintScan => "print > scan",
            Self::ConvertFilmPrintScan => "input > convert-film > print > scan",
            Self::ConvertFilmScanMinusBase => "input > convert-film > scan-minus-base",
            Self::ConvertFilmScan => "input > convert-film > scan",
            Self::Custom => "",
        }
    }

    pub fn prints(&self) -> bool {
        matches!(
            self,
            Self::FilmPrintScan
                | Self::ConvertFilmPrintScan
                | Self::InputFilmPrint
                | Self::StoredFilmPrint
                | Self::StoredFilmPrintScan
        )
    }

    pub fn is_convert_film(&self) -> bool {
        matches!(
            self,
            Self::ConvertFilmPrintScan | Self::ConvertFilmScanMinusBase | Self::ConvertFilmScan
        )
    }

    pub fn scans_film(self) -> bool {
        matches!(self, Self::FilmScan | Self::StoredFilmScan)
    }

    pub fn exposes_film(self) -> bool {
        matches!(
            self,
            Self::FilmScan | Self::FilmPrintScan | Self::InputFilm | Self::InputFilmPrint
        )
    }

    // Non-full diagnostic endpoints run the ordinary tap topology, even on
    // input and convert routes. Material endpoints reject taps separately.
    fn diagnostic_span(self, taps: &crate::params::TapsParams) -> bool {
        self.artifact_input().is_none()
            && self.artifact_output().is_none()
            && (taps.inject.as_deref().is_some_and(|tap| tap != "rgb_in")
                || taps.collect.as_deref().is_some_and(|tap| tap != "rgb_out"))
    }

    /// Whether the selected span crosses a pipeline boundary. Artifact routes
    /// start and end at their material boundary rather than at RGB taps.
    fn crosses(self, taps: &crate::params::TapsParams, node: crate::params::Tap) -> bool {
        use crate::params::Tap;
        let start = taps
            .inject
            .as_deref()
            .and_then(|s| Tap::parse(s).ok())
            .unwrap_or(match self {
                Self::StoredFilmScan | Self::StoredFilmPrint | Self::StoredFilmPrintScan => {
                    Tap::CmyFilm
                }
                Self::StoredPrintScan => Tap::CmyPrint,
                _ => Tap::RgbIn,
            });
        let end = taps
            .collect
            .as_deref()
            .and_then(|s| Tap::parse(s).ok())
            .unwrap_or(match self {
                Self::Input if !self.diagnostic_span(taps) => Tap::RgbIn,
                Self::InputFilm => Tap::CmyFilm,
                Self::InputFilmPrint | Self::StoredFilmPrint => Tap::CmyPrint,
                _ => Tap::RgbOut,
            });
        start.position() <= node.position() && node.position() < end.position()
    }

    /// Metering consumes the source before RGB preprocessing. Convert-film
    /// bypasses that helper even when a diagnostic span reaches later taps.
    pub fn executes_input_metering(self, taps: &crate::params::TapsParams) -> bool {
        !self.is_convert_film() && self.crosses(taps, crate::params::Tap::RgbIn)
    }

    pub fn executes_film_exposure(self, taps: &crate::params::TapsParams) -> bool {
        (self.exposes_film() || self.diagnostic_span(taps))
            && self.crosses(taps, crate::params::Tap::RgbPre)
    }

    pub fn executes_film_development(self, taps: &crate::params::TapsParams) -> bool {
        (self.exposes_film() || self.diagnostic_span(taps))
            && self.crosses(taps, crate::params::Tap::LogEFilm)
    }

    pub fn executes_conversion(self, taps: &crate::params::TapsParams) -> bool {
        self.is_convert_film()
            && !self.diagnostic_span(taps)
            && self.crosses(taps, crate::params::Tap::RgbIn)
    }

    pub fn executes_print_exposure(self, taps: &crate::params::TapsParams) -> bool {
        (self.prints() || (self.diagnostic_span(taps) && !self.scans_film()))
            && self.crosses(taps, crate::params::Tap::CmyFilm)
    }

    pub fn executes_print_development(self, taps: &crate::params::TapsParams) -> bool {
        (self.prints() || (self.diagnostic_span(taps) && !self.scans_film()))
            && self.crosses(taps, crate::params::Tap::LogEPrint)
    }

    pub fn executes_scan(self, taps: &crate::params::TapsParams) -> bool {
        use crate::params::Tap;
        if self.diagnostic_span(taps) {
            return self.crosses(
                taps,
                if self.scans_film() {
                    Tap::CmyFilm
                } else {
                    Tap::CmyPrint
                },
            );
        }
        match self {
            Self::FilmScan
            | Self::StoredFilmScan
            | Self::ConvertFilmScan
            | Self::ConvertFilmScanMinusBase => self.crosses(taps, Tap::CmyFilm),
            Self::FilmPrintScan
            | Self::StoredFilmPrintScan
            | Self::StoredPrintScan
            | Self::ConvertFilmPrintScan => self.crosses(taps, Tap::CmyPrint),
            _ => false,
        }
    }

    /// Terminal RGB finishing runs on the input passthrough or after scan
    /// readout, but not on material outputs or earlier diagnostic taps.
    pub fn executes_terminal_rgb_finishing(self, taps: &crate::params::TapsParams) -> bool {
        (self == Self::Input && !self.diagnostic_span(taps)) || self.executes_scan(taps)
    }

    pub fn executes_print_readout(self, taps: &crate::params::TapsParams) -> bool {
        (self.diagnostic_span(taps) && !self.scans_film()
            || matches!(
                self,
                Self::FilmPrintScan
                    | Self::StoredFilmPrintScan
                    | Self::StoredPrintScan
                    | Self::ConvertFilmPrintScan
            ))
            && self.executes_scan(taps)
    }

    pub fn artifact_input(self) -> Option<crate::stage_artifact::MaterialKind> {
        use crate::stage_artifact::MaterialKind;
        match self {
            Self::StoredFilmScan | Self::StoredFilmPrint | Self::StoredFilmPrintScan => {
                Some(MaterialKind::Film)
            }
            Self::StoredPrintScan => Some(MaterialKind::Print),
            _ => None,
        }
    }

    pub fn artifact_output(self) -> Option<crate::stage_artifact::MaterialKind> {
        use crate::stage_artifact::MaterialKind;
        match self {
            Self::InputFilm => Some(MaterialKind::Film),
            Self::InputFilmPrint | Self::StoredFilmPrint => Some(MaterialKind::Print),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RgbToRawAlgorithm {
    Hanatos2025,
    Mallett2019,
    Arctic2026Alpha02,
    Arctic2026Beta04,
    GaussLasers,
    Jakob2019,
    Otsu2018,
    Custom,
}

impl RgbToRawAlgorithm {
    pub fn parse(value: &str) -> Self {
        use crate::params::validation::RgbToRawMethod;
        match RgbToRawMethod::parse(value) {
            Ok(RgbToRawMethod::Hanatos2025) => Self::Hanatos2025,
            Ok(RgbToRawMethod::Mallett2019) => Self::Mallett2019,
            Ok(RgbToRawMethod::Arctic2026Alpha02) => Self::Arctic2026Alpha02,
            Ok(RgbToRawMethod::Arctic2026Beta04) => Self::Arctic2026Beta04,
            Ok(RgbToRawMethod::GaussLasers) => Self::GaussLasers,
            Ok(RgbToRawMethod::Jakob2019) => Self::Jakob2019,
            Ok(RgbToRawMethod::Otsu2018) => Self::Otsu2018,
            Err(_) => Self::Custom,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamutAlgorithm {
    Off,
    AcesRgc,
    Oklch,
    Oklrab,
    Jzazbz,
    Cam16ucs,
    Custom,
}

impl GamutAlgorithm {
    pub fn parse(value: &str) -> Self {
        match value {
            "off" => Self::Off,
            "aces_rgc" => Self::AcesRgc,
            "oklch" => Self::Oklch,
            "oklrab" => Self::Oklrab,
            "jzazbz" => Self::Jzazbz,
            "cam16ucs" => Self::Cam16ucs,
            _ => Self::Custom,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrainEngine {
    Spektra,
    Procedural,
    Dye,
    Fotufilm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrainMode {
    Analogue,
    Noise,
    Spektra,
    /// The Dye conditional film-coordinate dye field.
    DyeField,
    /// The Fotufilm effective-appearance film population model.
    FotufilmFilm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordLaw {
    ScalarFixedContribution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordOpticalOrder {
    FormedDensityThenCellResponseThenFractionalIntegration,
}

/// Effective scalar-record controls resolved by the shared field owner.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RecordConfiguration {
    pub law: RecordLaw,
    pub event_contribution_d_um2: f64,
    pub support_um: f64,
    pub cell_um: f64,
    pub seed: u64,
    pub expectation: bool,
    pub optical_order: RecordOpticalOrder,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrainConfiguration {
    pub active: bool,
    pub engine: GrainEngine,
    pub mode: GrainMode,
    pub spektra_fast_statistics_sampler: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<RecordConfiguration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpatialConfiguration {
    pub lens_blur: bool,
    pub halation: bool,
    pub optical_diffusion: bool,
    pub dir: bool,
    pub glare: bool,
    pub unsharp: bool,
}

/// One bounded bundled stock identifier. Unknown user identifiers are `custom`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockId(String);

impl StockId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn bundled(profile: &Profile) -> Option<Self> {
        profile.info.stock.as_ref().map(|value| {
            if is_bundled_stock(value) {
                Self(bounded_text(value))
            } else {
                Self("custom".to_owned())
            }
        })
    }
}

pub(crate) fn is_bundled_stock(value: &str) -> bool {
    matches!(
        value,
        "fujifilm_c200"
            | "fujifilm_crystal_archive_typeii"
            | "fujifilm_pro_400h"
            | "fujifilm_provia_100f"
            | "fujifilm_velvia_100"
            | "fujifilm_xtra_400"
            | "kodak_2302"
            | "kodak_2383"
            | "kodak_2393"
            | "kodak_doublex"
            | "kodak_ektachrome_100"
            | "kodak_ektacolor_edge"
            | "kodak_ektar_100"
            | "kodak_endura_premier"
            | "kodak_gold_200"
            | "kodak_kodachrome_64"
            | "kodak_portra_160"
            | "kodak_portra_400"
            | "kodak_portra_800"
            | "kodak_portra_800_push1"
            | "kodak_portra_800_push2"
            | "kodak_portra_endura"
            | "kodak_supra_endura"
            | "kodak_trix"
            | "kodak_ultra_endura"
            | "kodak_ultramax_400"
            | "kodak_verita_200d"
            | "kodak_vision3_50d"
            | "kodak_vision3_200t"
            | "kodak_vision3_250d"
            | "kodak_vision3_500t"
    )
}

fn bounded_text(value: &str) -> String {
    let mut text = String::new();
    for (index, character) in value.chars().enumerate() {
        if index >= 256 {
            break;
        }
        text.push(character);
    }
    text
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanOutput {
    #[default]
    DirectScan,
    NegativeToPositive,
}

/// Effective scanner model of a render: the one built-in neutral scan model,
/// the readout illuminant selected by the material category, output white and
/// device matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanModelConfiguration {
    pub model: String,
    pub sensor: String,
    pub white_reference: String,
    pub output_white: String,
    pub matrix: String,
    /// Empty when reading a pre-cutover report that did not record this fact.
    #[serde(default)]
    pub readout_illuminant: String,
    #[serde(default)]
    pub readout_rule: String,
}

impl Default for ScanModelConfiguration {
    fn default() -> Self {
        Self {
            model: "lab_scan_v1".into(),
            sensor: "cie_1931_cmf".into(),
            white_reference: "clear_film_base".into(),
            output_white: "output_color_space_white".into(),
            matrix: "identity".into(),
            readout_illuminant: "D50".into(),
            readout_rule: "default".into(),
        }
    }
}

impl ScanModelConfiguration {
    fn from_runtime(params: &RuntimeParams, film: &Profile, print: &Profile) -> Self {
        let provenance = crate::ScannerProvenance::from_params(params);
        let scanned = if params.io.scan_film { film } else { print };
        Self {
            model: provenance.model.to_owned(),
            sensor: provenance.sensor.to_owned(),
            white_reference: provenance.white_reference.to_owned(),
            output_white: provenance.output_white.to_owned(),
            matrix: provenance.matrix.to_owned(),
            readout_illuminant: scanned.readout_illuminant().to_owned(),
            readout_rule: scanned.readout_rule().to_owned(),
        }
    }
}

/// Declared finishing configuration, not a timing or executor observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishingConfiguration {
    pub model: String,
    pub boundary: String,
    #[serde(deserialize_with = "crate::params::FinishingParams::deserialize_complete")]
    pub controls: crate::params::FinishingParams,
    pub applicable: bool,
}

impl FinishingConfiguration {
    pub(crate) fn from_params(params: &RuntimeParams, medium_is_positive: bool) -> Self {
        Self {
            model: crate::runtime::MODEL_VERSION.into(),
            boundary: "pre_output_gamut".into(),
            controls: params.finishing.clone(),
            applicable: crate::finishing::is_applicable(params, medium_is_positive),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.model != crate::runtime::MODEL_VERSION || self.boundary != "pre_output_gamut" {
            return Err("unsupported finishing definition or boundary".into());
        }
        self.controls
            .validate_detailed()
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderConfiguration {
    pub workflow_route: WorkflowRoute,
    pub film_stock: Option<StockId>,
    pub print_stock: Option<StockId>,
    pub rgb_to_raw_algorithm: RgbToRawAlgorithm,
    pub input_color_space: ColorSpaceRole,
    pub output_color_space: ColorSpaceRole,
    pub input_transfer_decoded: bool,
    pub output_transfer_encoded: bool,
    pub enlarger_lut_requested: bool,
    pub scanner_lut_requested: bool,
    #[serde(default)]
    pub scan_output: ScanOutput,
    #[serde(default = "default_conversion_style")]
    pub conversion_style: String,
    #[serde(default)]
    pub scan_operator: ScanOperatorParams,
    #[serde(default)]
    pub scan_model: ScanModelConfiguration,
    #[serde(default = "default_output_mix_amount")]
    pub output_mix_amount: f64,
    #[serde(default)]
    pub model_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finishing: Option<FinishingConfiguration>,
    pub gamut_algorithm: GamutAlgorithm,
    pub grain: GrainConfiguration,
    pub spatial: SpatialConfiguration,
}

fn default_output_mix_amount() -> f64 {
    1.0
}

fn default_conversion_style() -> String {
    "neutral".into()
}

impl RenderConfiguration {
    pub fn from_runtime(params: &RuntimeParams, film: &Profile, print: &Profile) -> Self {
        let grain = &params.film_render.grain;
        let grain_active = grain.active
            && !params.debug.deactivate_stochastic_effects
            && !(grain.engine == crate::params::GrainEngine::Fotufilm
                && grain.fotufilm.amount == 0.0);
        let spatial_active = !params.debug.deactivate_spatial_effects;
        Self {
            workflow_route: WorkflowRoute::parse(&params.workflow.route),
            film_stock: StockId::bundled(film),
            print_stock: StockId::bundled(print),
            rgb_to_raw_algorithm: RgbToRawAlgorithm::parse(&params.settings.rgb_to_raw_method),
            input_color_space: ColorSpaceRole::parse(&params.io.input_color_space),
            output_color_space: ColorSpaceRole::parse(&params.io.output_color_space),
            input_transfer_decoded: params.io.input_cctf_decoding,
            output_transfer_encoded: params.io.output_cctf_encoding,
            enlarger_lut_requested: params.settings.use_enlarger_lut,
            scanner_lut_requested: params.settings.use_scanner_lut,
            scan_output: if params.scanner.scan_output == "negative_to_positive" {
                ScanOutput::NegativeToPositive
            } else {
                ScanOutput::DirectScan
            },
            conversion_style: params.scanner.conversion_style_name().to_owned(),
            scan_operator: params.scanner.operator,
            scan_model: ScanModelConfiguration::from_runtime(params, film, print),
            output_mix_amount: params.output_mix.amount,
            model_version: crate::runtime::MODEL_VERSION.into(),
            finishing: Some(FinishingConfiguration::from_params(params, {
                crate::finishing::scanned_material(params).is_some_and(|material| {
                    let scanned = match material {
                        crate::stage_artifact::MaterialKind::Film => film,
                        crate::stage_artifact::MaterialKind::Print => print,
                    };
                    scanned.is_paper() || scanned.is_positive()
                })
            })),
            gamut_algorithm: GamutAlgorithm::parse(&params.io.output_gamut_compress.algorithm),
            grain: GrainConfiguration {
                active: grain_active,
                engine: match grain.engine {
                    crate::params::GrainEngine::Spektra => GrainEngine::Spektra,
                    crate::params::GrainEngine::Procedural => GrainEngine::Procedural,
                    crate::params::GrainEngine::Dye => GrainEngine::Dye,
                    crate::params::GrainEngine::Fotufilm => GrainEngine::Fotufilm,
                },
                mode: match grain.engine {
                    crate::params::GrainEngine::Spektra => GrainMode::Spektra,
                    crate::params::GrainEngine::Procedural => match grain.procedural_mode {
                        crate::params::grain::ProceduralGrainMode::Analogue => GrainMode::Analogue,
                        crate::params::grain::ProceduralGrainMode::Noise => GrainMode::Noise,
                    },
                    crate::params::GrainEngine::Dye => GrainMode::DyeField,
                    crate::params::GrainEngine::Fotufilm => GrainMode::FotufilmFilm,
                },
                spektra_fast_statistics_sampler: grain_active
                    && grain.engine == crate::params::GrainEngine::Spektra
                    && params.settings.use_fast_stats,
                record: None,
            },
            spatial: SpatialConfiguration {
                lens_blur: spatial_active
                    && (params.camera.lens_blur_um > 0.0
                        || params.enlarger.lens_blur > 0.0
                        || params.scanner.lens_blur > 0.0),
                halation: spatial_active && params.film_render.halation.active,
                optical_diffusion: spatial_active
                    && (params.camera.diffusion_filter.active
                        || params.enlarger.diffusion_filter.active),
                dir: spatial_active && params.film_render.dir_couplers.active,
                glare: spatial_active
                    && (params.film_render.glare.active || params.print_render.glare.active),
                unsharp: spatial_active
                    && params.scanner.unsharp_mask[0] > 0.0
                    && params.scanner.unsharp_mask[1] > 0.0,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Png,
    Jpeg,
    Tiff,
    Exr,
    Custom,
}

impl OutputFormat {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "png" => Self::Png,
            "jpg" | "jpeg" => Self::Jpeg,
            "tif" | "tiff" => Self::Tiff,
            "exr" => Self::Exr,
            _ => Self::Custom,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BitDepth {
    U8,
    U16,
    F16,
    F32,
    Other(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compression {
    None,
    Deflate,
    Lzw,
    Zip,
    Jpeg,
    Exr,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavingConfiguration {
    pub format: OutputFormat,
    pub depth: BitDepth,
    pub compression: Compression,
    pub color_space: ColorSpaceRole,
    pub transfer_encoded: bool,
}

impl SavingConfiguration {
    pub fn new(
        format: OutputFormat,
        depth: BitDepth,
        compression: Compression,
        color_space: ColorSpaceRole,
        transfer_encoded: bool,
    ) -> Self {
        Self {
            format,
            depth,
            compression,
            color_space,
            transfer_encoded,
        }
    }

    pub fn from_save_options(
        format: crate::image_io::ImageFormat,
        options: &crate::image_io::SaveOptions<'_>,
    ) -> Self {
        Self::new(
            match format {
                crate::image_io::ImageFormat::Jpeg => OutputFormat::Jpeg,
                crate::image_io::ImageFormat::Png => OutputFormat::Png,
                crate::image_io::ImageFormat::Tiff => OutputFormat::Tiff,
                crate::image_io::ImageFormat::Exr => OutputFormat::Exr,
            },
            match options.depth {
                crate::image_io::BitDepth::Eight => BitDepth::U8,
                crate::image_io::BitDepth::Sixteen => match format {
                    crate::image_io::ImageFormat::Exr => BitDepth::F16,
                    _ => BitDepth::U16,
                },
                crate::image_io::BitDepth::ThirtyTwo => BitDepth::F32,
            },
            match format {
                crate::image_io::ImageFormat::Jpeg => Compression::Jpeg,
                crate::image_io::ImageFormat::Png => Compression::Deflate,
                crate::image_io::ImageFormat::Tiff => match options.compression {
                    Some(crate::image_io::Compression::Zip) | None => Compression::Zip,
                    Some(crate::image_io::Compression::None) => Compression::None,
                },
                crate::image_io::ImageFormat::Exr => Compression::Exr,
            },
            ColorSpaceRole::parse(options.color_space),
            options.cctf_encoding,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fotufilm_engine_and_model_have_distinct_telemetry_identity() {
        assert_eq!(
            serde_json::to_value(GrainEngine::Fotufilm).unwrap(),
            "fotufilm"
        );
        assert_eq!(
            serde_json::to_value(GrainMode::FotufilmFilm).unwrap(),
            "fotufilm_film"
        );
        assert_ne!(GrainMode::FotufilmFilm, GrainMode::DyeField);
        assert_eq!(
            serde_json::to_value(GrainEngine::Spektra).unwrap(),
            "spektra"
        );
        assert_eq!(
            serde_json::to_value(GrainEngine::Procedural).unwrap(),
            "procedural"
        );
        assert_eq!(serde_json::to_value(GrainEngine::Dye).unwrap(), "dye");
    }
}
