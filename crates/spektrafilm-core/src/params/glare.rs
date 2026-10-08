use serde::{Deserialize, Serialize};
use super::{default_true, default_half};

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
fn default_003() -> f32 {
    0.03
}
fn default_07() -> f32 {
    0.7
}
