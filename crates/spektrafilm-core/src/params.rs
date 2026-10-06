/// Runtime parameters for the film simulation pipeline.
///
/// Mirrors Python `params_schema.py`. Every field has a sensible default
/// matching the Python implementation.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffusionFilterParams {
    #[serde(default)]
    pub active: bool,
    #[serde(default = "default_bpm")]
    pub filter_family: String,
    #[serde(default = "default_half")]
    pub strength: f32,
    #[serde(default = "default_one")]
    pub spatial_scale: f32,
    #[serde(default)]
    pub halo_warmth: f32,
    #[serde(default = "default_one")]
    pub core_intensity: f32,
    #[serde(default = "default_one")]
    pub core_size: f32,
    #[serde(default = "default_one")]
    pub halo_intensity: f32,
    #[serde(default = "default_one")]
    pub halo_size: f32,
    #[serde(default = "default_one")]
    pub bloom_intensity: f32,
    #[serde(default = "default_one")]
    pub bloom_size: f32,
}

impl DiffusionFilterParams {
    /// Borrowed view as the model crate's `DiffusionFilter` (f64), for the
    /// CPU diffusion-filter apply. `family` borrows `self.filter_family`.
    pub fn to_model(&self) -> spektrafilm_model::diffusion::DiffusionFilter<'_> {
        spektrafilm_model::diffusion::DiffusionFilter {
            family: &self.filter_family,
            strength: self.strength as f64,
            spatial_scale: self.spatial_scale as f64,
            halo_warmth: self.halo_warmth as f64,
            core_intensity: self.core_intensity as f64,
            core_size: self.core_size as f64,
            halo_intensity: self.halo_intensity as f64,
            halo_size: self.halo_size as f64,
            bloom_intensity: self.bloom_intensity as f64,
            bloom_size: self.bloom_size as f64,
        }
    }
}

impl Default for DiffusionFilterParams {
    fn default() -> Self {
        Self {
            active: false,
            filter_family: "black_pro_mist".into(),
            strength: 0.5,
            spatial_scale: 1.0,
            halo_warmth: 0.0,
            core_intensity: 1.0,
            core_size: 1.0,
            halo_intensity: 1.0,
            halo_size: 1.0,
            bloom_intensity: 1.0,
            bloom_size: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default = "default_color_filter", deserialize_with = "deserialize_color_filter")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScannerParams {
    #[serde(default)]
    pub lens_blur: f32,
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
}

impl Default for ScannerParams {
    fn default() -> Self {
        Self {
            lens_blur: 0.0,
            white_correction: false,
            black_correction: false,
            white_level: 0.98,
            black_level: 0.01,
            unsharp_mask: [0.7, 0.7],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrainParams {
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default = "default_true")]
    pub sublayers_active: bool,
    // f64 to preserve Python JSON precision through the Poisson/Binomial
    // RNG pipeline — the f32 truncation of these values shifts the
    // Poisson lambda by ~5e-8 and produces a different RNG stream.
    // Field names mirror upstream 0.3.4 `GrainParams` exactly
    // (`particle_area_um2` / `particle_scale` / `particle_scale_layers`).
    #[serde(default = "default_02_f64")]
    pub particle_area_um2: f64,
    #[serde(default = "default_particle_scale_f64")]
    pub particle_scale: [f64; 3],
    #[serde(default = "default_particle_scale_layers_f64")]
    pub particle_scale_layers: [f64; 3],
    #[serde(default = "default_rms_granularity_f64")]
    pub rms_granularity: [f64; 3],
    #[serde(default = "default_density_min_f64")]
    pub density_min: [f64; 3],
    #[serde(default = "default_uniformity_f64")]
    pub uniformity: [f64; 3],
    #[serde(default = "default_065")]
    pub blur: f32,
    #[serde(default = "default_one")]
    pub blur_dye_clouds_um: f32,
    #[serde(default = "default_micro_structure")]
    pub micro_structure: [f32; 2],
    #[serde(default = "default_1i")]
    pub n_sub_layers: u32,
    /// One shared noise field across all channels instead of independent
    /// per-channel RNG streams. Set by the pipeline for B&W films
    /// (upstream n_channels==1 has a single emulsion); not user-facing.
    #[serde(default)]
    pub monochrome: bool,
}

fn default_02_f64() -> f64 {
    0.2
}
fn default_particle_scale_f64() -> [f64; 3] {
    [1.6, 1.6, 3.2]
}
fn default_particle_scale_layers_f64() -> [f64; 3] {
    [2.0, 1.0, 0.5]
}
fn default_density_min_f64() -> [f64; 3] {
    [0.03, 0.03, 0.03]
}
fn default_rms_granularity_f64() -> [f64; 3] {
    [0.0, 0.0, 0.0]
}
fn default_uniformity_f64() -> [f64; 3] {
    [0.97, 0.99, 0.97]
}
impl Default for GrainParams {
    fn default() -> Self {
        Self {
            active: true,
            sublayers_active: true,
            particle_area_um2: 0.2,
            particle_scale: [1.6, 1.6, 3.2],
            particle_scale_layers: [2.0, 1.0, 0.5],
            rms_granularity: [0.0, 0.0, 0.0],
            density_min: [0.03, 0.03, 0.03],
            uniformity: [0.97, 0.99, 0.97],
            blur: 0.65,
            blur_dye_clouds_um: 1.0,
            micro_structure: [0.2, 30.0],
            n_sub_layers: 1,
            monochrome: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HalationParams {
    #[serde(default = "default_true")]
    pub active: bool,
    // f64 to match Python's `np.asarray(..., dtype=np.float64)` —
    // f32 storage truncates ~7 decimals which shifts every sigma/lambda
    // by ~3e-8, accumulating through Gaussian/exponential kernels.
    #[serde(default = "default_one_f64")]
    pub scatter_amount: f64,
    #[serde(default = "default_one_f64")]
    pub scatter_spatial_scale: f64,
    #[serde(default = "default_one_f64")]
    pub halation_amount: f64,
    #[serde(default = "default_one_f64")]
    pub halation_spatial_scale: f64,
    #[serde(default = "default_scatter_core_f64")]
    pub scatter_core_um: [f64; 3],
    #[serde(default = "default_scatter_tail_f64")]
    pub scatter_tail_um: [f64; 3],
    #[serde(default = "default_scatter_tail_weight_f64")]
    pub scatter_tail_weight: [f64; 3],
    #[serde(default)]
    pub boost_ev: f32,
    #[serde(default = "default_03")]
    pub boost_range: f32,
    #[serde(default = "default_4")]
    pub protect_ev: f32,
    #[serde(default = "default_halation_strength_f64")]
    pub halation_strength: [f64; 3],
    #[serde(default = "default_halation_sigma_f64")]
    pub halation_first_sigma_um: [f64; 3],
    #[serde(default = "default_3i")]
    pub halation_n_bounces: u32,
    #[serde(default = "default_half_f64")]
    pub halation_bounce_decay: f64,
    #[serde(default = "default_true")]
    pub halation_renormalize: bool,
}

fn default_one_f64() -> f64 {
    1.0
}
fn default_half_f64() -> f64 {
    0.5
}
fn default_scatter_core_f64() -> [f64; 3] {
    [2.2, 2.0, 1.6]
}
fn default_scatter_tail_f64() -> [f64; 3] {
    [9.3, 9.7, 9.1]
}
fn default_scatter_tail_weight_f64() -> [f64; 3] {
    [0.78, 0.65, 0.67]
}
fn default_halation_strength_f64() -> [f64; 3] {
    [0.05, 0.015, 0.0]
}
fn default_halation_sigma_f64() -> [f64; 3] {
    [65.0, 65.0, 65.0]
}

impl Default for HalationParams {
    fn default() -> Self {
        Self {
            active: true,
            scatter_amount: 1.0,
            scatter_spatial_scale: 1.0,
            halation_amount: 1.0,
            halation_spatial_scale: 1.0,
            scatter_core_um: [2.2, 2.0, 1.6],
            scatter_tail_um: [9.3, 9.7, 9.1],
            scatter_tail_weight: [0.78, 0.65, 0.67],
            boost_ev: 0.0,
            boost_range: 0.3,
            protect_ev: 4.0,
            halation_strength: [0.05, 0.015, 0.0],
            halation_first_sigma_um: [65.0, 65.0, 65.0],
            halation_n_bounces: 3,
            halation_bounce_decay: 0.5,
            halation_renormalize: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirCouplersParams {
    #[serde(default = "default_true")]
    pub active: bool,
    // f64 throughout — Python reads these as JSON floats (f64). The
    // f32 truncation of values like 0.341 (→ 0.3409999907... in f32 vs
    // 0.341 = 0.34100000000000003 in f64) shifts every coupler weight
    // and diffusion sigma by ~3e-8 and amplifies through the per-channel
    // density correction.
    #[serde(default = "default_one_f64")]
    pub amount: f64,
    #[serde(default = "default_one_f64")]
    pub inhibition_samelayer: f64,
    #[serde(default = "default_one_f64")]
    pub inhibition_interlayer: f64,
    #[serde(default = "default_gamma_same_f64")]
    pub gamma_samelayer_rgb: [f64; 3],
    #[serde(default = "default_gamma_r_gb_f64")]
    pub gamma_interlayer_r_to_gb: [f64; 2],
    #[serde(default = "default_gamma_g_rb_f64")]
    pub gamma_interlayer_g_to_rb: [f64; 2],
    #[serde(default = "default_gamma_b_rg_f64")]
    pub gamma_interlayer_b_to_rg: [f64; 2],
    #[serde(default = "default_20_f64")]
    pub diffusion_size_um: f64,
    #[serde(default = "default_200_f64")]
    pub diffusion_tail_um: f64,
    #[serde(default = "default_006_f64")]
    pub diffusion_tail_weight: f64,
}

fn default_gamma_same_f64() -> [f64; 3] {
    [0.341, 0.324, 0.273]
}
fn default_gamma_r_gb_f64() -> [f64; 2] {
    [0.355, 0.305]
}
fn default_gamma_g_rb_f64() -> [f64; 2] {
    [0.154, 0.358]
}
fn default_gamma_b_rg_f64() -> [f64; 2] {
    [0.171, 0.225]
}
fn default_20_f64() -> f64 {
    20.0
}
fn default_200_f64() -> f64 {
    200.0
}
fn default_006_f64() -> f64 {
    0.06
}

impl Default for DirCouplersParams {
    fn default() -> Self {
        Self {
            active: true,
            amount: 1.0,
            inhibition_samelayer: 1.0,
            inhibition_interlayer: 1.0,
            gamma_samelayer_rgb: [0.341, 0.324, 0.273],
            gamma_interlayer_r_to_gb: [0.355, 0.305],
            gamma_interlayer_g_to_rb: [0.154, 0.358],
            gamma_interlayer_b_to_rg: [0.171, 0.225],
            diffusion_size_um: 20.0,
            diffusion_tail_um: 200.0,
            diffusion_tail_weight: 0.06,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlareParams {
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default = "default_003")]
    pub percent: f32,
    #[serde(default = "default_07")]
    pub roughness: f32,
    #[serde(default = "default_half")]
    pub blur: f32,
}

impl Default for GlareParams {
    fn default() -> Self {
        Self {
            active: true,
            percent: 0.03,
            roughness: 0.7,
            blur: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    fn default() -> Self { Self { active: true, scale: 1.0, tilt: 0.0, cyan: 1.0, magenta: 1.0, yellow: 1.0 } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    fn default() -> Self { Self { active: true, scale: 1.0, cyan: 1.0, magenta: 1.0, yellow: 1.0 } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    fn default() -> Self { Self { scan_illuminant: "D55".into(), exposure_compensation_ev: 0.0, base_percentile: 99.0, calibration: default_calibration() } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowParams {
    #[serde(default = "default_route")]
    pub route: String,
}
impl Default for WorkflowParams { fn default() -> Self { Self { route: default_route() } } }

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default = "default_print_density_curves_morph", deserialize_with = "deserialize_print_density_curves_morph")]
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
    Option::<String>::deserialize(deserializer).map(|value| value.unwrap_or_else(default_color_filter))
}

fn default_print_density_curves_morph() -> PrintCurvesMorphParams {
    PrintCurvesMorphParams { active: false, ..PrintCurvesMorphParams::default() }
}

fn deserialize_print_density_curves_morph<'de, D>(deserializer: D) -> Result<PrintCurvesMorphParams, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut value = serde_json::Value::deserialize(deserializer)?;
    if let Some(object) = value.as_object_mut() {
        object.entry("active").or_insert(serde_json::Value::Bool(false));
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

/// User-facing controls for the s023 print density-curve morph (see
/// `crate::print_morph`). Kept in f64 — the morph is a parity-sensitive f64
/// computation. The standalone helper defaults `active` to `true`, matching
/// upstream `PrintCurvesMorphParams`; nested print-render controls default
/// it to `false`, matching `PrintRenderingParams`. Development still evaluates
/// a profile's fitted model when present; `active` controls coupled-gamma morphing.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default = "default_gamut_knee")]
    pub knee: [f32; 3],
}

impl Default for InputGamutCompressParams {
    fn default() -> Self {
        Self {
            active: true,
            algorithm: "xy".into(),
            knee: [0.0, 1.0, 6.0],
        }
    }
}

/// Output gamut compression config — mirrors upstream `OutputGamutCompressSpec`.
/// `"cam16ucs"` (the default, matching upstream 0.3.4) applies the CAM16-UCS
/// chroma knee + one-sided lightness roll-off; `"off"` passes RGB through
/// unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputGamutCompressParams {
    #[serde(default = "default_cam16ucs")]
    pub algorithm: String,
    #[serde(default = "default_gamut_knee")]
    pub knee: [f32; 3],
    #[serde(default = "default_gamut_lightness")]
    pub lightness_compression: Option<[f32; 3]>,
}

impl Default for OutputGamutCompressParams {
    fn default() -> Self {
        Self {
            algorithm: "cam16ucs".into(),
            knee: [0.0, 1.0, 6.0],
            lightness_compression: Some([0.7, 1.0, 2.2]),
        }
    }
}

fn default_xy() -> String {
    "xy".into()
}
fn default_cam16ucs() -> String {
    "cam16ucs".into()
}
fn default_gamut_knee() -> [f32; 3] {
    [0.0, 1.0, 6.0]
}
fn default_gamut_lightness() -> Option<[f32; 3]> {
    Some([0.7, 1.0, 2.2])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl Default for DebugParams {
    fn default() -> Self {
        Self {
            deactivate_spatial_effects: false,
            deactivate_stochastic_effects: false,
            print_timings: false,
            lut_mode: false,
        }
    }
}

/// Pipeline tap configuration — mirrors upstream 0.3.4 `TapsParams`.
///
/// `inject` and `collect` name the entry and exit points in the pipeline
/// topology (`rgb_in`, `rgb_pre`, `log_e_film`, `cmy_film`, `log_e_print`,
/// `cmy_print`, `rgb_out`). Defaults of `None` mean "normal end-to-end run"
/// (inject at `rgb_in`, collect at `rgb_out`); call-site overrides passed to
/// `Pipeline::process_with_taps` win over these persistent values.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// actually implements. Mirrors the upstream failures that Python
    /// raises for unknown color spaces (`colour` `KeyError`), unknown
    /// `rgb_to_raw_method` (`ValueError` in `FilmingStage`), unknown
    /// gamut algorithms (`ValueError` in the specs' `__post_init__`),
    /// unknown diffusion filter families (`ValueError` in
    /// `apply_diffusion_filter_um`) and unknown tap names — all surfaced
    /// *before* any artifact is produced. Returns the first failure.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_color()?;
        let spectral_shape = crate::spectral_service::SpectralShape::new(self.settings.spectral_shape)
            .map_err(|e| format!("settings.spectral_shape: {e}"))?;
        if spectral_shape.bounds != crate::spectral_service::default_spectral_shape() {
            return Err(format!(
                "settings.spectral_shape: only the bundled {:?} grid is supported by the current profile/CMF/LUT contract, got {:?}",
                crate::spectral_service::default_spectral_shape(),
                spectral_shape.bounds
            ));
        }
        let calibration_values = self
            .film_render
            .convert
            .calibration
            .split(|c: char| c.is_ascii_whitespace() || matches!(c, ',' | ';' | '[' | ']' | '(' | ')'))
            .filter(|part| !part.is_empty())
            .map(|part| {
                part.parse::<f64>()
                    .map_err(|_| format!("film_render.convert.calibration contains non-numeric token {part:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if calibration_values.len() != 9 || calibration_values.iter().any(|value| !value.is_finite()) {
            return Err(format!(
                "film_render.convert.calibration must contain exactly 9 finite numbers, got {}",
                calibration_values.len()
            ));
        }
        if !crate::spectral_service::is_supported_color_filter(&self.camera.color_filter) {
            let supported = crate::spectral_service::available_color_filters().join(", ");
            return Err(format!(
                "camera.color_filter: unsupported filter {:?}; supported: {supported}",
                self.camera.color_filter
            ));
        }
        if !matches!(
            self.settings.rgb_to_raw_method.as_str(),
            "hanatos2025" | "mallett2019" | "arctic2026alpha02" | "arctic2026beta04"
                | "gauss-lasers" | "jakob2019" | "otsu2018"
        ) {
            return Err(format!(
                "settings.rgb_to_raw_method: unsupported method {:?}; supported: \
                 hanatos2025, mallett2019, arctic2026alpha02, arctic2026beta04, \
                 gauss-lasers, jakob2019, otsu2018",
                self.settings.rgb_to_raw_method
            ));
        }
        if !crate::spectral_service::is_supported_illuminant(&self.enlarger.illuminant) {
            let supported = crate::spectral_service::available_illuminants().join(", ");
            return Err(format!(
                "enlarger.illuminant: unsupported illuminant {:?}; supported: {supported}",
                self.enlarger.illuminant
            ));
        }
        if !crate::spectral_service::is_supported_illuminant(&self.film_render.convert.scan_illuminant) {
            let supported = crate::spectral_service::available_illuminants().join(", ");
            return Err(format!(
                "film_render.convert.scan_illuminant: unsupported illuminant {:?}; supported: {supported}",
                self.film_render.convert.scan_illuminant
            ));
        }
        const FILTER_FAMILIES: &str =
            "glimmerglass, black_pro_mist, pro_mist, cinebloom";
        for (label, df) in [
            (
                "camera.diffusion_filter.filter_family",
                &self.camera.diffusion_filter,
            ),
            (
                "enlarger.diffusion_filter.filter_family",
                &self.enlarger.diffusion_filter,
            ),
        ] {
            if df.active && df.strength > 0.0 && df.spatial_scale > 0.0
                && !matches!(
                    df.filter_family.as_str(),
                    "glimmerglass" | "black_pro_mist" | "pro_mist" | "cinebloom"
                ) {
                return Err(format!(
                    "{label}: unknown diffusion filter family {:?}; available: \
                     {FILTER_FAMILIES}",
                    df.filter_family
                ));
            }
        }
        if let Some(t) = self.taps.inject.as_deref() {
            Tap::parse(t).map_err(|e| format!("taps.inject: {e}"))?;
        }
        if !matches!(self.workflow.route.as_str(),
            "input" | "input > film > scan" | "input > film > print > scan" |
            "input > convert-film > print > scan" | "input > convert-film > scan-minus-base" |
            "input > convert-film > scan") {
            return Err(format!("workflow.route: unsupported route {:?}", self.workflow.route));
        }
        if let Some(t) = self.taps.collect.as_deref() {
            Tap::parse(t).map_err(|e| format!("taps.collect: {e}"))?;
        }
        Ok(())
    }
}
/// Top-level runtime parameters. Combines all sub-parameter groups.
///
/// Mirrors upstream 0.3.4 `RuntimePhotoParams` minus the `film` / `print`
/// profile objects (Rust passes `Profile` values alongside the params).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeParams {
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
    pub settings: SettingsParams,
    #[serde(default)]
    pub debug: DebugParams,
    #[serde(default)]
    pub taps: TapsParams,
}

impl Default for RuntimeParams {
    fn default() -> Self {
        Self {
            camera: CameraParams::default(),
            enlarger: EnlargerParams::default(),
            scanner: ScannerParams::default(),
            film_render: FilmRenderingParams::default(),
            print_render: PrintRenderingParams::default(),
            io: IoParams::default(),
            workflow: WorkflowParams::default(),
            settings: SettingsParams::default(),
            debug: DebugParams::default(),
            taps: TapsParams::default(),
        }
    }
}

impl RuntimeParams {
    /// Resolve colour transforms and reject unsupported gamut configurations
    /// before the pipeline can produce an image or LUT.
    pub fn validate_color(&self) -> Result<(), String> {
        spektrafilm_math::colorspace::resolve(&self.io.input_color_space)
            .map_err(|e| format!("io.input_color_space: {e}"))?;
        spektrafilm_math::colorspace::resolve(&self.io.output_color_space)
            .map_err(|e| format!("io.output_color_space: {e}"))?;
        crate::input_gamut::InputGamutCompress::build(&self.io.input_gamut_compress)?;
        crate::gamut_compression::OutputGamutCompress::build(
            &self.io.output_gamut_compress,
            &self.io.output_color_space,
        )?;
        Ok(())
    }
}

// Default value helpers
fn default_bpm() -> String {
    "black_pro_mist".into()
}
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
fn default_098() -> f32 {
    0.98
}
fn default_001() -> f32 {
    0.01
}
fn default_unsharp() -> [f64; 2] {
    [0.7, 0.7]
}
fn default_065() -> f32 {
    0.65
}
fn default_micro_structure() -> [f32; 2] {
    [0.2, 30.0]
}
fn default_1i() -> u32 {
    1
}
fn default_03() -> f32 {
    0.3
}
fn default_4() -> f32 {
    4.0
}
fn default_3i() -> u32 {
    3
}
fn default_003() -> f32 {
    0.03
}
fn default_07() -> f32 {
    0.7
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
fn default_d55() -> String { "D55".into() }
fn default_99() -> f64 { 99.0 }
fn default_calibration() -> String { "1 0 0  0 1 0  0 0 1".into() }
fn default_route() -> String { "input > film > print > scan".into() }

#[cfg(test)]
mod tests {
    use super::CameraParams;

    #[test]
    fn null_or_missing_camera_color_filter_uses_no_filter() {
        let null: CameraParams = serde_json::from_str(r#"{"color_filter":null}"#).unwrap();
        assert_eq!(null.color_filter, "none");

        let missing: CameraParams = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.color_filter, "none");

        let named: CameraParams =
            serde_json::from_str(r#"{"color_filter":"hoya_r1"}"#).unwrap();
        assert_eq!(named.color_filter, "hoya_r1");
    }
}
