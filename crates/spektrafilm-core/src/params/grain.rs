use serde::{Deserialize, Serialize};
use super::{default_true, default_one};

/// Selects the film-grain implementation. V1 remains the default for
/// backwards-compatible recipes; V2 is procedural grain in linear scanner RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrainEngine {
    V1,
    V2,
}

impl Default for GrainEngine {
    fn default() -> Self {
        Self::V1
    }
}

/// Procedural Grain V2 generation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrainV2Mode {
    Analogue,
    Noise,
}

impl Default for GrainV2Mode {
    fn default() -> Self {
        Self::Analogue
    }
}

fn default_grain_v2_profile() -> String {
    "35mm250".to_owned()
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrainV2FilmType {
    #[default]
    Negative,
    Positive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrainParams {
    #[serde(default = "default_true")]
    pub active: bool,
    /// Selects the grain implementation; V1 preserves the historical default.
    #[serde(default)]
    pub engine: GrainEngine,
    /// Dehancer grain profile id, or "custom" for explicit controls.
    #[serde(default = "default_grain_v2_profile")]
    pub v2_profile: String,
    #[serde(default)]
    pub v2_film_type: GrainV2FilmType,
    #[serde(default)]
    pub v2_mode: GrainV2Mode,
    /// Custom controls; omitted values use the default 35mm250 profile.
    /// Size uses 1..=48; Amount, tonal controls and Chroma use 0..=100.
    #[serde(default)]
    pub v2_size: Option<f32>,
    #[serde(default)]
    pub v2_amount: Option<f32>,
    #[serde(default)]
    pub v2_shadows: Option<f32>,
    #[serde(default)]
    pub v2_midtones: Option<f32>,
    #[serde(default)]
    pub v2_highlights: Option<f32>,
    #[serde(default)]
    pub v2_chroma: Option<f32>,
    /// Film Resolution override, 0..=100. Unset inherits the profile.
    #[serde(default)]
    pub v2_resolution_factor: Option<f32>,
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
            engine: GrainEngine::V1,
            v2_profile: default_grain_v2_profile().to_owned(),
            v2_mode: GrainV2Mode::Analogue,
            v2_film_type: GrainV2FilmType::Negative,
            v2_size: None,
            v2_amount: None,
            v2_shadows: None,
            v2_midtones: None,
            v2_highlights: None,
            v2_chroma: None,
            v2_resolution_factor: None,
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

impl GrainParams {
    /// Resolve the selected Dehancer preset or custom controls.
    pub fn resolved_grain_v2(&self) -> spektrafilm_model::grain::v2::GrainV2Params {
        use spektrafilm_model::grain::v2::{self, GrainV2Params};
        let custom = self.v2_profile == "custom";
        let index = v2::profile_index(if custom { "35mm250" } else { &self.v2_profile })
            .expect("Grain V2 profile must be validated before rendering");
        let mut params = GrainV2Params::for_profile(index);
        params.amount = self.v2_amount.map_or(params.amount, |v| v / 100.0);
        if custom {
            params.mode = match self.v2_mode {
                GrainV2Mode::Analogue => v2::GrainV2Mode::Analogue,
                GrainV2Mode::Noise => v2::GrainV2Mode::Noise,
            };
            params.size = self.v2_size.unwrap_or(params.size);
            params.shadows = self.v2_shadows.map_or(params.shadows, |v| v / 100.0);
            params.midtones = self.v2_midtones.map_or(params.midtones, |v| v / 100.0);
            params.highlights = self.v2_highlights.map_or(params.highlights, |v| v / 100.0);
            params.color = self.v2_chroma.map_or(params.color, |v| v / 100.0);
            params.resolution_factor = self.v2_resolution_factor.unwrap_or(params.resolution_factor);
        }
        params
    }

    /// Custom starts with the values of the last selected preset.
    pub fn select_custom_grain_v2(&mut self) {
        let params = self.resolved_grain_v2();
        self.v2_profile = "custom".into();
        self.v2_mode = match params.mode {
            spektrafilm_model::grain::v2::GrainV2Mode::Analogue => GrainV2Mode::Analogue,
            spektrafilm_model::grain::v2::GrainV2Mode::Noise => GrainV2Mode::Noise,
        };
        self.v2_size = Some(params.size);
        self.v2_amount = Some(params.amount * 100.0);
        self.v2_shadows = Some(params.shadows * 100.0);
        self.v2_midtones = Some(params.midtones * 100.0);
        self.v2_highlights = Some(params.highlights * 100.0);
        self.v2_chroma = Some(params.color * 100.0);
        self.v2_resolution_factor = Some(params.resolution_factor);
    }
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
