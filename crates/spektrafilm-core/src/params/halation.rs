use super::{default_one_f64, default_true};
use serde::{Deserialize, Serialize};

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
fn default_03() -> f32 {
    0.3
}
fn default_4() -> f32 {
    4.0
}
fn default_3i() -> u32 {
    3
}
