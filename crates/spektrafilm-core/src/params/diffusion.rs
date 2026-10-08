use serde::{Deserialize, Serialize};
use super::{default_half, default_one};

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
fn default_bpm() -> String {
    "black_pro_mist".into()
}
