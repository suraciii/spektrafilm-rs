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

const fn default_grain_v2_resolution_type() -> u32 {
    1
}

const fn default_grain_v2_timer() -> f32 {
    0.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrainParams {
    #[serde(default = "default_true")]
    pub active: bool,
    /// Selects the grain implementation; V1 preserves the historical default.
    #[serde(default)]
    pub engine: GrainEngine,
    /// Procedural Grain V2 profile id.
    #[serde(default = "default_grain_v2_profile")]
    pub v2_profile: String,
    #[serde(default)]
    pub v2_mode: GrainV2Mode,
    /// Optional controls inherit the selected profile when unset.
    /// Size uses the profile scale range 1..=48; tonal controls use 0..=1.
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
    /// 0 = Gaussian FIR, 1 = fractional box FIR approximating FastBlur.
    #[serde(default = "default_grain_v2_resolution_type")]
    pub v2_resolution_type: u32,
    /// Stable animation phase; photos should leave this at zero.
    #[serde(default = "default_grain_v2_timer")]
    pub v2_timer: f32,
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
            v2_size: None,
            v2_amount: None,
            v2_shadows: None,
            v2_midtones: None,
            v2_highlights: None,
            v2_chroma: None,
            v2_resolution_factor: None,
            v2_resolution_type: default_grain_v2_resolution_type(),
            v2_timer: default_grain_v2_timer(),
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
    /// Resolve profile defaults and explicit overrides after RuntimeParams::validate.
    /// The returned seed contains the timer phase; callers add the runtime seed.
    pub fn resolved_grain_v2(&self) -> spektrafilm_model::grain::v2::GrainV2Params {
        use spektrafilm_model::grain::v2::{self, GrainV2Params};
        let index = v2::profile_index(&self.v2_profile)
            .expect("Grain V2 profile must be validated before rendering");
        let mut params = GrainV2Params::for_profile(index);
        params.mode = match self.v2_mode {
            GrainV2Mode::Analogue => v2::GrainV2Mode::Analogue,
            GrainV2Mode::Noise => v2::GrainV2Mode::Noise,
        };
        params.size = self.v2_size.unwrap_or(params.size);
        params.amount = self.v2_amount.unwrap_or(params.amount);
        params.shadows = self.v2_shadows.unwrap_or(params.shadows);
        params.midtones = self.v2_midtones.unwrap_or(params.midtones);
        params.highlights = self.v2_highlights.unwrap_or(params.highlights);
        params.color = self.v2_chroma.unwrap_or(params.color);
        params.resolution_factor = self.v2_resolution_factor.unwrap_or(params.resolution_factor);
        params.resolution_type = self.v2_resolution_type;
        params.seed = self.v2_timer as u32;
        params
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
