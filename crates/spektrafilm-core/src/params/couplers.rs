use serde::{Deserialize, Serialize};
use super::{default_true, default_one_f64};
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
    #[serde(default = "default_one_rgb_f64")]
    pub langmuir_donor_k_rgb: [f64; 3],
    #[serde(default = "default_one_rgb_f64")]
    pub langmuir_receiver_k_rgb: [f64; 3],
    #[serde(default = "default_20_f64")]
    pub diffusion_size_um: f64,
    #[serde(default = "default_200_f64")]
    pub diffusion_tail_um: f64,
    #[serde(default = "default_006_f64")]
    pub diffusion_tail_weight: f64,
}
fn default_one_rgb_f64() -> [f64; 3] {
    [1.0; 3]
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
            langmuir_donor_k_rgb: [1.0; 3],
            langmuir_receiver_k_rgb: [1.0; 3],
            diffusion_size_um: 20.0,
            diffusion_tail_um: 200.0,
            diffusion_tail_weight: 0.06,
        }
    }
}
