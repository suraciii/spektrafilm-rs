use serde::{Deserialize, Serialize};

/// Camera light leak: stray light that reaches the film record during
/// exposure. It is an additive term on record exposure, evaluated before
/// development, so its amount is stated in stops relative to the exposure that
/// produces mid density on the selected film.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightLeakParams {
    #[serde(default)]
    pub active: bool,
    /// Aperture exposure in stops over the film's mid-density exposure.
    /// Zero is an exact bypass; the default is visible when the switch turns on.
    #[serde(default = "default_exposure")]
    pub exposure_ev: f32,
    /// Aperture center around the frame perimeter, `[0, 1]`, clockwise from
    /// the top-left corner; 1 wraps to 0.
    #[serde(default)]
    pub position: f32,
    /// Aperture width along the perimeter, millimetres on the film.
    #[serde(default = "default_width")]
    pub width_mm: f32,
    /// Inward e-folding length, millimetres on the film.
    #[serde(default = "default_spread")]
    pub spread_mm: f32,
    /// Aperture tilt relative to the perimeter tangent, degrees.
    #[serde(default)]
    pub angle_deg: f32,
    /// Per-record added-exposure ratio in cyan, magenta, yellow order.
    #[serde(default = "default_spectrum")]
    pub leak_spectrum: [f32; 3],
    /// Strength of the seeded streak structure over the smooth falloff.
    #[serde(default = "default_texture_amount")]
    pub texture_amount: f32,
    /// Spatial scale of the streak structure, millimetres on the film.
    #[serde(default = "default_texture_scale")]
    pub texture_scale_mm: f32,
}

fn default_exposure() -> f32 {
    1.0
}
fn default_width() -> f32 {
    8.0
}
fn default_spread() -> f32 {
    4.0
}
fn default_texture_amount() -> f32 {
    0.25
}
fn default_texture_scale() -> f32 {
    6.0
}
fn default_spectrum() -> [f32; 3] {
    [1.0, 0.4, 0.15]
}

impl Default for LightLeakParams {
    fn default() -> Self {
        Self {
            active: false,
            exposure_ev: default_exposure(),
            position: 0.0,
            width_mm: default_width(),
            spread_mm: default_spread(),
            angle_deg: 0.0,
            leak_spectrum: default_spectrum(),
            texture_amount: default_texture_amount(),
            texture_scale_mm: default_texture_scale(),
        }
    }
}

impl LightLeakParams {
    /// The model representation. Conversion is total; every field is already a
    /// plain scalar in the model's own units.
    pub fn to_model(&self) -> spektrafilm_model::light_leak::LightLeakParams {
        spektrafilm_model::light_leak::LightLeakParams {
            exposure_ev: self.exposure_ev,
            position: self.position,
            width_mm: self.width_mm,
            spread_mm: self.spread_mm,
            angle_deg: self.angle_deg,
            leak_spectrum: self.leak_spectrum,
            texture_amount: self.texture_amount,
            texture_scale_mm: self.texture_scale_mm,
        }
    }
}
