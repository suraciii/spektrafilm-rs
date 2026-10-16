use super::{default_one, default_true};
use serde::{Deserialize, Serialize};

/// Selects one of four independent grain routes. Spektra preserves the
/// upstream density model; Procedural is Dehancer-derived encoded RGB grain;
/// Dye is the self-developed dye field; Fotufilm is an appearance population model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum GrainEngine {
    #[default]
    Spektra,
    Procedural,
    Dye,
    Fotufilm,
}

/// Procedural Grain Procedural generation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum GrainV2Mode {
    #[default]
    Analogue,
    Noise,
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

/// Effective grain material. Auto follows the resolved film polarity and color
/// type; an explicit override supports chromogenic monochrome films.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FotufilmMaterial {
    #[default]
    Auto,
    Silver,
    DyeNegative,
    DyeReversal,
}

/// Authored effective-appearance controls, not measured stock calibration.
/// Population controls fit the model; amount, size, softness, color and layers
/// modify sampling only. Sublayer shares must sum to one, never auto-normalize.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FotufilmGrainParams {
    pub amount: f64,
    pub size: f64,
    pub softness: f64,
    pub color: f64,
    pub layers: [f64; 3],
    /// Effective RMS optical-density target, not measured stock granularity.
    pub rms_density: f64,
    /// Effective population clump radius in film micrometers.
    pub clump_radius_um: f64,
    pub layer_weights: [f64; 3],
    pub layer_size_ratio: [f64; 3],
    pub radius_span: f64,
    pub sublayer_shares: [f64; 4],
    pub coating_density_scale: f64,
    pub fog_density: f64,
    pub material: FotufilmMaterial,
}

impl Default for FotufilmGrainParams {
    fn default() -> Self {
        Self {
            amount: 1.0,
            size: 1.0,
            softness: 1.0,
            color: 1.0,
            layers: [1.0; 3],
            rms_density: 0.009,
            clump_radius_um: 5.0,
            layer_weights: [0.7, 1.0, 1.35],
            layer_size_ratio: [1.0; 3],
            radius_span: 6.0,
            sublayer_shares: [0.25; 4],
            coating_density_scale: 1.0,
            fog_density: 0.03,
            material: FotufilmMaterial::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrainParams {
    #[serde(default = "default_true")]
    pub active: bool,
    /// Selects the grain route; Spektra preserves the upstream default.
    #[serde(default)]
    pub engine: GrainEngine,
    #[serde(default)]
    pub fotufilm: FotufilmGrainParams,
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
    #[serde(default = "default_grain_v2_resolution_type")]
    pub v2_resolution_type: u32,
    /// Explicit host timer in the canonical phase domain. Legacy `0.0`
    /// remains the seed-derived phase sentinel.
    #[serde(default = "default_grain_v2_timer")]
    pub v2_timer: f32,
    #[serde(default = "default_particle_scale_sublayers_f64")]
    pub particle_scale_sublayers: [f64; 3],
    /// Film-space Gaussian sigma of Dye's support, in micrometers. It
    /// conditions the field: the resolved field-cell pitch is `support / 2`.
    /// Dye-only; Spektra and Procedural never read it.
    #[serde(default = "default_v3_dye_support_um")]
    pub v3_dye_support_um: f32,
    /// Base statistical event area of Dye, in square film micrometers. The
    /// stock granularity preset scales it per channel; it never changes the
    /// dye support. Dye-only; Spektra reads `particle_area_um2` instead.
    #[serde(default = "default_v3_event_area_um2")]
    pub v3_event_area_um2: f64,
    #[serde(default = "default_07")]
    pub mult_usm_sigma: f32,
    #[serde(default = "default_15")]
    pub mult_usm_amount: f32,
    #[serde(default = "default_micro_sublayers")]
    pub micro_sublayers: u32,
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
    #[serde(default = "default_particle_scale_layers_f64", skip_serializing)]
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
fn default_particle_scale_sublayers_f64() -> [f64; 3] {
    [1.0, 0.4, 0.25]
}
/// Default dye support selected with the default event area on the reference
/// condition (`specs/film-grain/v3/design.md` §Parameters).
fn default_v3_dye_support_um() -> f32 {
    3.5
}
/// Validated event area of the reference stock `kodak_portra_800`
/// (`specs/film-grain/v3/design.md` §Parameters).
fn default_v3_event_area_um2() -> f64 {
    1.28
}
fn default_grain_v2_resolution_type() -> u32 {
    1
}
fn default_grain_v2_timer() -> f32 {
    0.0
}
fn default_07() -> f32 {
    0.7
}
fn default_15() -> f32 {
    1.5
}
fn default_micro_sublayers() -> u32 {
    1
}
fn default_density_min_f64() -> [f64; 3] {
    [0.03, 0.03, 0.03]
}
fn default_rms_granularity_f64() -> [f64; 3] {
    [0.0, 0.0, 0.0]
}
fn default_uniformity_f64() -> [f64; 3] {
    [0.97, 0.97, 0.97]
}
impl Default for GrainParams {
    fn default() -> Self {
        Self {
            active: true,
            engine: GrainEngine::Spektra,
            fotufilm: FotufilmGrainParams::default(),
            v2_profile: default_grain_v2_profile().to_owned(),
            v2_mode: GrainV2Mode::Analogue,
            v2_film_type: GrainV2FilmType::Negative,
            v2_size: None,
            v2_amount: None,
            v2_shadows: None,
            v2_midtones: None,
            v2_highlights: None,
            v2_chroma: None,
            v2_resolution_type: default_grain_v2_resolution_type(),
            v2_timer: default_grain_v2_timer(),
            v2_resolution_factor: None,
            sublayers_active: true,
            particle_area_um2: 0.2,
            particle_scale: [1.6, 1.6, 3.2],
            particle_scale_layers: [2.0, 1.0, 0.5],
            rms_granularity: [0.0, 0.0, 0.0],
            density_min: [0.03, 0.03, 0.03],
            uniformity: [0.97, 0.97, 0.97],
            particle_scale_sublayers: [1.0, 0.4, 0.25],
            v3_dye_support_um: default_v3_dye_support_um(),
            v3_event_area_um2: default_v3_event_area_um2(),
            blur: 0.89,
            mult_usm_sigma: 0.7,
            mult_usm_amount: 1.5,
            blur_dye_clouds_um: 2.0,
            micro_sublayers: 1,
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
            .expect("Grain Procedural profile must be validated before rendering");
        let mut params = GrainV2Params::for_profile(index);
        params.resolution_type = self.v2_resolution_type;
        params.timer = (self.v2_timer != 0.0).then_some(self.v2_timer);
        params.amount = self.v2_amount.map_or(params.amount, |v| v / 100.0);
        if custom {
            params.mode = match self.v2_mode {
                GrainV2Mode::Analogue => v2::GrainV2Mode::Analogue,
                GrainV2Mode::Noise => v2::GrainV2Mode::Noise,
            };
            params.film_type = match self.v2_film_type {
                GrainV2FilmType::Negative => 0,
                GrainV2FilmType::Positive => 1,
            };
            params.size = self.v2_size.unwrap_or(params.size);
            params.shadows = self.v2_shadows.map_or(params.shadows, |v| v / 100.0);
            params.midtones = self.v2_midtones.map_or(params.midtones, |v| v / 100.0);
            params.highlights = self.v2_highlights.map_or(params.highlights, |v| v / 100.0);
            params.color = self.v2_chroma.map_or(params.color, |v| v / 100.0);
            params.resolution_factor = self
                .v2_resolution_factor
                .unwrap_or(params.resolution_factor);
        }
        params
    }
    /// Convert the selected Procedural profile to the shared f32 GPU boundary.
    pub fn gpu_params(
        &self,
        random_seed: u64,
        deactivate_spatial_effects: bool,
    ) -> spektrafilm_backend::GrainV2GpuParams {
        let mut grain = self.resolved_grain_v2();
        if deactivate_spatial_effects {
            grain.resolution_factor = 100.0;
        }
        grain.seed = random_seed as u32;
        spektrafilm_backend::GrainV2GpuParams {
            mode: grain.mode as u32,
            resolution_type: grain.resolution_type,
            amount: grain.amount,
            shadows: grain.shadows,
            midtones: grain.midtones,
            highlights: grain.highlights,
            raw_scale: grain.size,
            cluster_size: grain.cluster_size,
            rotation: grain.rotation,
            color: grain.color,
            resolution_factor: grain.resolution_factor,
            seed: grain.seed,
            timer: grain.timer,
            colored: grain.colored,
            clustered: grain.clustered,
        }
    }

    /// Custom starts with the values of the last selected preset.
    pub fn select_custom_grain_v2(&mut self) {
        let params = self.resolved_grain_v2();
        self.v2_profile = "custom".into();
        self.v2_mode = match params.mode {
            spektrafilm_model::grain::v2::GrainV2Mode::Analogue => GrainV2Mode::Analogue,
            spektrafilm_model::grain::v2::GrainV2Mode::Noise => GrainV2Mode::Noise,
        };
        self.v2_film_type = if params.film_type == 1 {
            GrainV2FilmType::Positive
        } else {
            GrainV2FilmType::Negative
        };
        self.v2_resolution_type = params.resolution_type;
        self.v2_timer = params.timer.unwrap_or(0.0);
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

#[cfg(test)]
mod fotufilm_tests {
    use super::*;

    /// A named control and the setter that writes it.
    type Setter = (&'static str, fn(&mut FotufilmGrainParams, f64));

    #[test]
    fn fotufilm_defaults_partial_sections_and_material_roundtrip() {
        let defaults = FotufilmGrainParams::default();
        let legacy: GrainParams = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.engine, GrainEngine::Spektra);
        assert_eq!(legacy.fotufilm, defaults);
        let partial: GrainParams =
            serde_json::from_str(r#"{"engine":"fotufilm","fotufilm":{"amount":2.0}}"#).unwrap();
        assert_eq!(partial.fotufilm.amount, 2.0);
        assert_eq!(partial.fotufilm.sublayer_shares, [0.25; 4]);
        for material in [
            FotufilmMaterial::Auto,
            FotufilmMaterial::Silver,
            FotufilmMaterial::DyeNegative,
            FotufilmMaterial::DyeReversal,
        ] {
            let mut params = partial.clone();
            params.fotufilm.material = material;
            let json = serde_json::to_string(&params).unwrap();
            assert_eq!(serde_json::from_str::<GrainParams>(&json).unwrap(), params);
            let toml = toml::to_string(&params).unwrap();
            assert_eq!(toml::from_str::<GrainParams>(&toml).unwrap(), params);
        }
        assert!(serde_json::from_str::<GrainParams>(r#"{"fotufilm":{"unknown":1}}"#).is_err());
        assert!(serde_json::from_str::<GrainParams>(r#"{"engine":"v4"}"#).is_err());
    }

    #[test]
    fn fotufilm_typed_runtime_rejects_nonfinite_controls_even_when_inactive() {
        let setters: [Setter; 13] = [
            ("amount", |p, v| p.amount = v),
            ("size", |p, v| p.size = v),
            ("softness", |p, v| p.softness = v),
            ("color", |p, v| p.color = v),
            ("layers", |p, v| p.layers[1] = v),
            ("rms_density", |p, v| p.rms_density = v),
            ("clump_radius_um", |p, v| p.clump_radius_um = v),
            ("layer_weights", |p, v| p.layer_weights[1] = v),
            ("layer_size_ratio", |p, v| p.layer_size_ratio[1] = v),
            ("radius_span", |p, v| p.radius_span = v),
            ("sublayer_shares", |p, v| p.sublayer_shares[1] = v),
            ("coating_density_scale", |p, v| p.coating_density_scale = v),
            ("fog_density", |p, v| p.fog_density = v),
        ];
        for (name, set) in setters {
            for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                let mut params = crate::params::RuntimeParams::default();
                params.film_render.grain.active = false;
                set(&mut params.film_render.grain.fotufilm, value);
                let error = params.validate().unwrap_err();
                assert!(
                    error.contains(&format!("film_render.grain.fotufilm.{name}")),
                    "{error}"
                );
            }
        }
    }
}
