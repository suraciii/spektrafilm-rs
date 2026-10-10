use crate::{params::RuntimeParams, profile::Profile};
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
    ConvertFilmPrintScan,
    ConvertFilmScanMinusBase,
    ConvertFilmScan,
    Custom,
}

impl WorkflowRoute {
    pub fn parse(value: &str) -> Self {
        match value {
            "input" => Self::Input,
            "input > film > scan" => Self::FilmScan,
            "input > film > print > scan" => Self::FilmPrintScan,
            "input > convert-film > print > scan" => Self::ConvertFilmPrintScan,
            "input > convert-film > scan-minus-base" => Self::ConvertFilmScanMinusBase,
            "input > convert-film > scan" => Self::ConvertFilmScan,
            _ => Self::Custom,
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
    V1,
    V2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrainMode {
    Analogue,
    Noise,
    V1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrainConfiguration {
    pub active: bool,
    pub engine: GrainEngine,
    pub mode: GrainMode,
    pub v1_fast_statistics_sampler: bool,
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
    PositiveScan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub gamut_algorithm: GamutAlgorithm,
    pub grain: GrainConfiguration,
    pub spatial: SpatialConfiguration,
}

impl RenderConfiguration {
    pub fn from_runtime(params: &RuntimeParams, film: &Profile, print: &Profile) -> Self {
        let grain = &params.film_render.grain;
        let grain_active = grain.active
            && !params.debug.deactivate_stochastic_effects
            && !(grain.engine == crate::params::GrainEngine::V2
                && params.settings.rgb_to_raw_method == "mallett2019");
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
            scan_output: if params.scanner.scan_output == "positive_scan" {
                ScanOutput::PositiveScan
            } else {
                ScanOutput::DirectScan
            },
            gamut_algorithm: GamutAlgorithm::parse(&params.io.output_gamut_compress.algorithm),
            grain: GrainConfiguration {
                active: grain_active,
                engine: match grain.engine {
                    crate::params::GrainEngine::V1 => GrainEngine::V1,
                    crate::params::GrainEngine::V2 => GrainEngine::V2,
                },
                mode: match grain.engine {
                    crate::params::GrainEngine::V1 => GrainMode::V1,
                    crate::params::GrainEngine::V2 => match grain.v2_mode {
                        crate::params::grain::GrainV2Mode::Analogue => GrainMode::Analogue,
                        crate::params::grain::GrainV2Mode::Noise => GrainMode::Noise,
                    },
                },
                v1_fast_statistics_sampler: grain_active
                    && grain.engine == crate::params::GrainEngine::V1
                    && grain.sublayers_active
                    && params.settings.use_fast_stats,
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
