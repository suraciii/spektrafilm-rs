use super::default_true;
use serde::{Deserialize, Serialize};

/// Selects one of five independent grain routes. Spektra preserves the
/// upstream density model; Procedural is Dehancer-derived encoded RGB grain;
/// Dye is the self-developed dye field; Fotufilm is an appearance population model;
/// Exposure is a reconstructed post-scan encoded-RGB appearance model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum GrainEngine {
    Spektra,
    Procedural,
    #[default]
    Dye,
    Fotufilm,
    Exposure,
}

/// Stable serialized film format for the Exposure automatic-size model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExposureGrainFormat {
    #[default]
    #[serde(rename = "35mm")]
    ThirtyFive,
    #[serde(rename = "medium_format")]
    Medium,
    #[serde(rename = "large_format")]
    Large,
}

impl GrainEngine {
    /// Whether this engine is an encoded-RGB post-scan appearance stage.
    pub const fn is_post_scan(self) -> bool {
        matches!(self, Self::Procedural | Self::Exposure)
    }
}

impl ExposureGrainFormat {
    pub const fn short_edge_mm(self) -> f64 {
        match self {
            Self::ThirtyFive => 24.0,
            Self::Medium => 56.0,
            Self::Large => 101.6,
        }
    }
}

/// Procedural Grain Procedural generation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ProceduralGrainMode {
    #[default]
    Analogue,
    Noise,
}

fn default_grain_procedural_profile() -> String {
    "35mm250".to_owned()
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProceduralGrainFilmType {
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

/// Authored post-scan Exposure Grain controls. The model is reconstructed from
/// the Exposure X7 luma path; these ranges are product bounds, not vendor
/// calibration or a pixel-parity claim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExposureGrainParams {
    pub strength: f64,
    pub size: f64,
    pub auto_size: bool,
    pub format: ExposureGrainFormat,
    pub absolute_size_px: f64,
    pub roughness: f64,
    pub shadows: f64,
    pub midtones: f64,
    pub highlights: f64,
    pub push: f64,
}

impl Default for ExposureGrainParams {
    fn default() -> Self {
        Self {
            strength: 100.0,
            size: 1.0,
            auto_size: true,
            format: ExposureGrainFormat::ThirtyFive,
            absolute_size_px: 1.5,
            roughness: 50.0,
            shadows: 50.0,
            midtones: 70.0,
            highlights: 30.0,
            push: 0.0,
        }
    }
}

impl ExposureGrainParams {
    pub fn validate(&self) -> Result<(), String> {
        self.validate_detailed().map_err(|error| error.to_string())
    }

    pub fn validate_detailed(&self) -> Result<(), super::validation::ValidationError> {
        for (name, value) in [
            ("strength", self.strength),
            ("size", self.size),
            ("absolute_size_px", self.absolute_size_px),
            ("roughness", self.roughness),
            ("shadows", self.shadows),
            ("midtones", self.midtones),
            ("highlights", self.highlights),
            ("push", self.push),
        ] {
            super::validation::validate_scalar_detailed(
                &format!("film_render.grain.exposure.{name}"),
                value,
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrainParams {
    #[serde(default = "default_true")]
    pub active: bool,
    /// Selects the grain route; new and sparse requests default to Dye.
    #[serde(default)]
    pub engine: GrainEngine,
    #[serde(default)]
    pub fotufilm: FotufilmGrainParams,
    #[serde(default)]
    pub exposure: ExposureGrainParams,
    /// Dehancer grain profile id, or "custom" for explicit controls.
    #[serde(default = "default_grain_procedural_profile")]
    #[serde(alias = "v2_profile")]
    pub procedural_profile: String,
    #[serde(default)]
    #[serde(alias = "v2_film_type")]
    pub procedural_film_type: ProceduralGrainFilmType,
    #[serde(default)]
    #[serde(alias = "v2_mode")]
    pub procedural_mode: ProceduralGrainMode,
    /// Custom controls; omitted values use the default 35mm250 profile.
    /// Size uses 1..=48; Amount, tonal controls and Chroma use 0..=100.
    #[serde(default)]
    #[serde(alias = "v2_size")]
    pub procedural_size: Option<f32>,
    #[serde(default)]
    #[serde(alias = "v2_amount")]
    pub procedural_amount: Option<f32>,
    #[serde(default)]
    #[serde(alias = "v2_shadows")]
    pub procedural_shadows: Option<f32>,
    #[serde(default)]
    #[serde(alias = "v2_midtones")]
    pub procedural_midtones: Option<f32>,
    #[serde(default)]
    #[serde(alias = "v2_highlights")]
    pub procedural_highlights: Option<f32>,
    #[serde(default)]
    #[serde(alias = "v2_chroma")]
    pub procedural_chroma: Option<f32>,
    #[serde(default = "default_grain_procedural_resolution_type")]
    #[serde(alias = "v2_resolution_type")]
    pub procedural_resolution_type: u32,
    /// Explicit host timer in the canonical phase domain. Legacy `0.0`
    /// remains the seed-derived phase sentinel.
    #[serde(default = "default_grain_procedural_timer")]
    #[serde(alias = "v2_timer")]
    pub procedural_timer: f32,
    /// Positive component scales for Spektra's relative particle scales per
    /// emulsion sublayer. Retained but not read by the delivered Dye law.
    #[serde(default = "default_particle_scale_sublayers_f64")]
    pub particle_scale_sublayers: [f64; 3],
    /// Retained legacy Dye support value, in micrometers. Loads and saves keep
    /// it unchanged; the delivered Dye law does not read it.
    #[serde(default = "default_dye_support_um")]
    #[serde(alias = "v3_dye_support_um")]
    pub dye_support_um: f32,
    /// Retained legacy Dye event area, in square film micrometers. Loads and
    /// saves keep it unchanged; the delivered Dye law does not read it.
    #[serde(default = "default_dye_event_area_um2")]
    #[serde(alias = "v3_event_area_um2")]
    pub dye_event_area_um2: f64,
    /// Relative Dye grain amplitude. `1.0` keeps the delivered law and `0.0`
    /// realizes the target density without random grain, including the
    /// residual count noise. Scales the correlated field and the event count
    /// together, so the density response and the per-record ratio are kept.
    /// Dye-only.
    #[serde(default = "default_dye_strength")]
    pub dye_strength: f64,
    /// Relative Dye grain scale. `1.0` keeps the delivered correlation
    /// length; the field's octaves and band limit scale together in film
    /// coordinates. It is not a crystal diameter, scanner blur, or output
    /// pixel size. Dye-only.
    #[serde(default = "default_dye_scale")]
    pub dye_scale: f64,
    /// Retained legacy Dye record-correlation value, in
    /// `[0, MEASURED_CHANNEL_CORRELATION_CEILING]`. Loads and saves keep it
    /// unchanged; the delivered Dye law does not read it.
    #[serde(default = "default_channel_correlation")]
    pub channel_correlation: f64,
    #[serde(default = "default_07")]
    pub mult_usm_sigma: f64,
    #[serde(default = "default_15")]
    pub mult_usm_amount: f64,
    #[serde(default = "default_micro_sublayers")]
    pub micro_sublayers: u32,
    /// Film Resolution override, 0..=100. Unset inherits the profile.
    #[serde(default)]
    #[serde(alias = "v2_resolution_factor")]
    pub procedural_resolution_factor: Option<f32>,
    /// Spektra's upstream RMS granularity control derives particle area from
    /// the film's density curves. It is not a certified optical RMS target.
    /// Retained for recipes that also select Dye, but the delivered Dye law
    /// does not read it.
    /// Typed defaults use zero. Source resolution supplies omitted Spektra
    /// targets `[6, 8, 10]` before stock defaults; direct Spektra API callers
    /// must supply positive targets themselves.
    #[serde(default)]
    pub rms_granularity: [f64; 3],
    #[serde(default = "default_density_min_f64")]
    pub density_min: [f64; 3],
    #[serde(default = "default_uniformity_f64")]
    pub uniformity: [f64; 3],
    #[serde(default = "default_grain_blur")]
    pub blur: f64,
    #[serde(default = "default_dye_cloud_blur")]
    pub blur_dye_clouds_um: f64,
    #[serde(default = "default_micro_structure")]
    pub micro_structure: [f64; 2],
    /// One shared noise field across all channels instead of independent
    /// per-channel RNG streams. Set by the pipeline for B&W films
    /// (upstream n_channels==1 has a single emulsion); not user-facing.
    #[serde(default)]
    pub monochrome: bool,
}

fn default_particle_scale_sublayers_f64() -> [f64; 3] {
    [1.0, 0.4, 0.25]
}
/// Default dye support selected with the default event area on the reference
/// condition (`specs/film-grain/dye/design.md` §Resolved parameter contract).
fn default_dye_support_um() -> f32 {
    3.5
}
/// Default statistical event area for the relative Dye reference scale
/// (`kodak_portra_800`), not a validated absolute material event area.
fn default_dye_event_area_um2() -> f64 {
    1.28
}
/// The delivered Dye amplitude: a recipe at `1.0` leaves the law unchanged.
fn default_dye_strength() -> f64 {
    1.0
}
/// The delivered Dye correlation length: a recipe at `1.0` leaves the law
/// unchanged.
fn default_dye_scale() -> f64 {
    1.0
}
/// Historical supported and acceptance ceiling for [`GrainParams::channel_correlation`].
///
/// The public name is retained for compatibility; it does not certify a
/// universal material limit or attribute higher correlation to scanner
/// processing. The Poisson split's structural domain is `[0, 1)`; the product
/// continues to accept only `[0, 0.45]`.
pub const MEASURED_CHANNEL_CORRELATION_CEILING: f64 = 0.45;

/// Off by default: the independent-field realization stays the shipped
/// behavior, and a recipe or preset that wants record correlation opts in
/// (`specs/film-grain/dye/design.md` §Channel coupling).
fn default_channel_correlation() -> f64 {
    0.0
}
fn default_grain_procedural_resolution_type() -> u32 {
    1
}
fn default_grain_procedural_timer() -> f32 {
    0.0
}
fn default_07() -> f64 {
    0.7
}
fn default_15() -> f64 {
    1.5
}
fn default_micro_sublayers() -> u32 {
    1
}
fn default_density_min_f64() -> [f64; 3] {
    [0.03, 0.03, 0.03]
}
fn default_uniformity_f64() -> [f64; 3] {
    [0.97, 0.97, 0.97]
}
fn default_grain_blur() -> f64 {
    0.89
}
fn default_dye_cloud_blur() -> f64 {
    2.0
}
impl Default for GrainParams {
    fn default() -> Self {
        Self {
            active: true,
            engine: GrainEngine::default(),
            fotufilm: FotufilmGrainParams::default(),
            exposure: ExposureGrainParams::default(),
            procedural_profile: default_grain_procedural_profile().to_owned(),
            procedural_mode: ProceduralGrainMode::Analogue,
            procedural_film_type: ProceduralGrainFilmType::Negative,
            procedural_size: None,
            procedural_amount: None,
            procedural_shadows: None,
            procedural_midtones: None,
            procedural_highlights: None,
            procedural_chroma: None,
            procedural_resolution_type: default_grain_procedural_resolution_type(),
            procedural_timer: default_grain_procedural_timer(),
            procedural_resolution_factor: None,
            rms_granularity: [0.0; 3],
            density_min: [0.03, 0.03, 0.03],
            uniformity: [0.97, 0.97, 0.97],
            particle_scale_sublayers: [1.0, 0.4, 0.25],
            dye_support_um: default_dye_support_um(),
            dye_event_area_um2: default_dye_event_area_um2(),
            dye_strength: default_dye_strength(),
            dye_scale: default_dye_scale(),
            channel_correlation: default_channel_correlation(),
            blur: 0.89,
            mult_usm_sigma: 0.7,
            mult_usm_amount: 1.5,
            blur_dye_clouds_um: 2.0,
            micro_sublayers: 1,
            micro_structure: [0.2, 30.0],
            monochrome: false,
        }
    }
}

impl GrainParams {
    /// Whether a post-scan appearance engine must encode the resident tail.
    /// `active = false` and Exposure `strength = 0` are exact bypasses.
    pub fn post_scan_active(&self) -> bool {
        self.active
            && match self.engine {
                GrainEngine::Procedural => true,
                GrainEngine::Exposure => self.exposure.strength > 0.0,
                _ => false,
            }
    }

    /// Whether the selected Exposure engine must execute. `active = false`
    /// and `strength = 0` are exact bypasses on both executors.
    pub fn exposure_active(&self) -> bool {
        self.engine == GrainEngine::Exposure && self.post_scan_active()
    }

    /// Resolve the selected Dehancer preset or custom controls.
    pub fn resolved_grain_procedural(
        &self,
    ) -> spektrafilm_model::grain::procedural::ProceduralGrainParams {
        use spektrafilm_model::grain::procedural::{self, ProceduralGrainParams};
        let custom = self.procedural_profile == "custom";
        let index = procedural::profile_index(if custom {
            "35mm250"
        } else {
            &self.procedural_profile
        })
        .expect("Grain Procedural profile must be validated before rendering");
        let mut params = ProceduralGrainParams::for_profile(index);
        params.resolution_type = self.procedural_resolution_type;
        params.timer = (self.procedural_timer != 0.0).then_some(self.procedural_timer);
        params.amount = self.procedural_amount.map_or(params.amount, |v| v / 100.0);
        if custom {
            params.mode = match self.procedural_mode {
                ProceduralGrainMode::Analogue => procedural::ProceduralGrainMode::Analogue,
                ProceduralGrainMode::Noise => procedural::ProceduralGrainMode::Noise,
            };
            params.film_type = match self.procedural_film_type {
                ProceduralGrainFilmType::Negative => 0,
                ProceduralGrainFilmType::Positive => 1,
            };
            params.size = self.procedural_size.unwrap_or(params.size);
            params.shadows = self
                .procedural_shadows
                .map_or(params.shadows, |v| v / 100.0);
            params.midtones = self
                .procedural_midtones
                .map_or(params.midtones, |v| v / 100.0);
            params.highlights = self
                .procedural_highlights
                .map_or(params.highlights, |v| v / 100.0);
            params.color = self.procedural_chroma.map_or(params.color, |v| v / 100.0);
            params.resolution_factor = self
                .procedural_resolution_factor
                .unwrap_or(params.resolution_factor);
        }
        params
    }
    /// Convert the selected Procedural profile to the shared f32 GPU boundary.
    pub fn gpu_params(
        &self,
        random_seed: u64,
        deactivate_spatial_effects: bool,
    ) -> spektrafilm_backend::ProceduralGrainGpuParams {
        let mut grain = self.resolved_grain_procedural();
        if deactivate_spatial_effects {
            grain.resolution_factor = 100.0;
        }
        grain.seed = random_seed as u32;
        spektrafilm_backend::ProceduralGrainGpuParams {
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

    /// Convert the selected Exposure controls to the shared f32 GPU boundary.
    pub fn gpu_params_exposure(
        &self,
        random_seed: u64,
    ) -> spektrafilm_backend::ExposureGrainGpuParams {
        spektrafilm_backend::ExposureGrainGpuParams {
            strength: self.exposure.strength as f32,
            size: self.exposure.size as f32,
            auto_size: self.exposure.auto_size,
            format_mm: self.exposure.format.short_edge_mm() as f32,
            absolute_size_px: self.exposure.absolute_size_px as f32,
            roughness: self.exposure.roughness as f32,
            shadows: self.exposure.shadows as f32,
            midtones: self.exposure.midtones as f32,
            highlights: self.exposure.highlights as f32,
            push: self.exposure.push as f32,
            seed: spektrafilm_model::grain::exposure::exposure_seed(random_seed),
            render_scale: 1.0,
        }
    }

    /// Resolve the selected Exposure engine controls for the host reference and
    /// GPU boundary. The shared seed is reduced to the frozen reference domain.
    pub fn resolved_grain_exposure(
        &self,
        random_seed: u64,
    ) -> spektrafilm_model::grain::exposure::ExposureGrainParams {
        use spektrafilm_model::grain::exposure::{ExposureFilmFormat, ExposureGrainParams};
        let format = match self.exposure.format {
            ExposureGrainFormat::ThirtyFive => ExposureFilmFormat::ThirtyFive,
            ExposureGrainFormat::Medium => ExposureFilmFormat::Medium,
            ExposureGrainFormat::Large => ExposureFilmFormat::Large,
        };
        ExposureGrainParams {
            strength: self.exposure.strength as f32,
            size: self.exposure.size as f32,
            auto_size: self.exposure.auto_size,
            format,
            absolute_size_px: self.exposure.absolute_size_px as f32,
            roughness: self.exposure.roughness as f32,
            shadows: self.exposure.shadows as f32,
            midtones: self.exposure.midtones as f32,
            highlights: self.exposure.highlights as f32,
            push: self.exposure.push as f32,
            seed: spektrafilm_model::grain::exposure::exposure_seed(random_seed),
            render_scale: 1.0,
        }
    }

    /// Custom starts with the values of the last selected preset.
    pub fn select_custom_grain_procedural(&mut self) {
        let params = self.resolved_grain_procedural();
        self.procedural_profile = "custom".into();
        self.procedural_mode = match params.mode {
            spektrafilm_model::grain::procedural::ProceduralGrainMode::Analogue => {
                ProceduralGrainMode::Analogue
            }
            spektrafilm_model::grain::procedural::ProceduralGrainMode::Noise => {
                ProceduralGrainMode::Noise
            }
        };
        self.procedural_film_type = if params.film_type == 1 {
            ProceduralGrainFilmType::Positive
        } else {
            ProceduralGrainFilmType::Negative
        };
        self.procedural_resolution_type = params.resolution_type;
        self.procedural_timer = params.timer.unwrap_or(0.0);
        self.procedural_size = Some(params.size);
        self.procedural_amount = Some(params.amount * 100.0);
        self.procedural_shadows = Some(params.shadows * 100.0);
        self.procedural_midtones = Some(params.midtones * 100.0);
        self.procedural_highlights = Some(params.highlights * 100.0);
        self.procedural_chroma = Some(params.color * 100.0);
        self.procedural_resolution_factor = Some(params.resolution_factor);
    }
}

fn default_micro_structure() -> [f64; 2] {
    [0.2, 30.0]
}

#[cfg(test)]
mod fotufilm_tests {
    use super::*;
    #[test]
    fn authored_and_sparse_grain_defaults_use_dye_without_rewriting_explicit_engines() {
        assert_eq!(GrainEngine::default(), GrainEngine::Dye);
        assert_eq!(GrainParams::default().engine, GrainEngine::Dye);
        assert_eq!(GrainParams::default().rms_granularity, [0.0; 3]);
        for source in [
            "{}",
            r#"{"film_render":{}}"#,
            r#"{"film_render":{"grain":{}}}"#,
        ] {
            let params: crate::params::RuntimeParams = serde_json::from_str(source).unwrap();
            assert_eq!(params.film_render.grain.engine, GrainEngine::Dye);
            assert_eq!(params.film_render.grain.rms_granularity, [0.0; 3]);
        }
        for source in ["", "[film_render]", "[film_render.grain]"] {
            let params: crate::params::RuntimeParams = toml::from_str(source).unwrap();
            assert_eq!(params.film_render.grain.engine, GrainEngine::Dye);
            assert_eq!(params.film_render.grain.rms_granularity, [0.0; 3]);
        }
        let edits = crate::params::sources::ParameterEdits::parse(
            "film_render.grain.dye_event_area_um2=2.25",
        )
        .unwrap();
        let mut params = crate::params::RuntimeParams::default();
        edits.apply(&mut params).unwrap();
        assert_eq!(params.film_render.grain.engine, GrainEngine::Dye);
        params.film_render.grain.engine = GrainEngine::Spektra;
        params.film_render.grain.rms_granularity = [6.0, 8.0, 10.0];
        edits.apply(&mut params).unwrap();
        assert_eq!(params.film_render.grain.engine, GrainEngine::Spektra);
        for (name, engine) in [
            ("spektra", GrainEngine::Spektra),
            ("procedural", GrainEngine::Procedural),
            ("dye", GrainEngine::Dye),
            ("fotufilm", GrainEngine::Fotufilm),
            ("exposure", GrainEngine::Exposure),
        ] {
            let source = serde_json::json!({"engine": name, "dye_event_area_um2": 2.25});
            let params: GrainParams = serde_json::from_value(source).unwrap();
            assert_eq!(params.engine, engine);
            let saved = serde_json::to_value(&params).unwrap();
            assert_eq!(
                serde_json::from_value::<GrainParams>(saved).unwrap(),
                params
            );
            let source = format!("engine = {name:?}\ndye_event_area_um2 = 2.25");
            assert_eq!(toml::from_str::<GrainParams>(&source).unwrap(), params);
        }
        for source in [
            r#"{"engine":null}"#,
            r#"{"engine":"unknown"}"#,
            r#"{"unknown":1}"#,
        ] {
            assert!(serde_json::from_str::<GrainParams>(source).is_err());
        }
    }

    /// A named control and the setter that writes it.
    type Setter = (&'static str, fn(&mut FotufilmGrainParams, f64));

    #[test]
    fn exposure_defaults_and_nested_roundtrip_keep_the_engine_independent() {
        let params = GrainParams::default();
        assert_eq!(params.engine, GrainEngine::Dye);
        assert_eq!(params.exposure, ExposureGrainParams::default());
        assert_eq!(params.exposure.format, ExposureGrainFormat::ThirtyFive);

        let source = serde_json::json!({
            "engine": "exposure",
            "exposure": {
                "strength": 73.0,
                "size": 2.5,
                "auto_size": false,
                "format": "large_format",
                "absolute_size_px": 3.0,
                "roughness": 61.0,
                "shadows": 12.0,
                "midtones": 80.0,
                "highlights": 44.0,
                "push": 2.0
            }
        });
        let parsed: GrainParams = serde_json::from_value(source.clone()).unwrap();
        assert_eq!(parsed.engine, GrainEngine::Exposure);
        assert_eq!(parsed.exposure.format, ExposureGrainFormat::Large);
        let saved = serde_json::to_value(&parsed).unwrap();
        assert_eq!(saved["exposure"], source["exposure"]);
        assert_eq!(
            serde_json::from_value::<GrainParams>(saved).unwrap(),
            parsed
        );
        let toml = toml::to_string(&parsed).unwrap();
        assert_eq!(toml::from_str::<GrainParams>(&toml).unwrap(), parsed);

        for source in [
            serde_json::json!({"engine": "exposure", "exposure": {"unknown": 1}}),
            serde_json::json!({"engine": "exposure", "exposure": {"format": "4x5"}}),
        ] {
            assert!(serde_json::from_value::<GrainParams>(source).is_err());
        }
    }

    #[test]
    fn fotufilm_defaults_partial_sections_and_material_roundtrip() {
        let defaults = FotufilmGrainParams::default();
        let sparse: GrainParams = serde_json::from_str("{}").unwrap();
        assert_eq!(sparse.engine, GrainEngine::Dye);
        assert_eq!(sparse.fotufilm, defaults);
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
