//! Device-resident numerical preparation shared by calibration and its consumers.
use crate::DeviceImagePayload;
use std::sync::Arc;

#[derive(Clone)]
pub struct PreparedTable {
    pub values: Arc<dyn DeviceImagePayload>,
    /// The bounded neutral response needed by scalar exposure calibration.
    pub neutral: [f64; 3],
}
impl std::fmt::Debug for PreparedTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedTable").field("backend_id", &self.values.backend_id())
            .field("neutral", &self.neutral).finish()
    }
}
pub struct SpectralTcLutParams<'a> {
    pub spectra: &'a [f32],
    pub size: u32,
    pub n_wavelengths: u32,
    pub sensitivity: &'a [[f64; 3]],
    pub illuminant: &'a [f64],
    pub gaussian_sigma: f64,
    pub window: &'a [[f64; 3]],
    pub surface: &'a [Vec<f64>],
    pub reference_tc: [f64; 2],
    pub reflectance: bool,
    pub neutral_tc: [f64; 2],
}
pub struct GamutTableParams {
    pub mode: u32,
    pub n_l: u32,
    pub n_h: u32,
    pub n_bisect: u32,
    pub l_min: f64,
    pub l_max: f64,
    pub chroma_upper: f64,
    pub xyz_to_rgb: [[f64; 3]; 3],
    pub cam16: [f64; 4],
}

pub struct InputGamutTableParams<'a> {
    pub size: u32,
    pub algorithm: u32,
    pub boundary: u32,
    pub hull_detail: f64,
    pub knee: [f64; 3],
    pub white_xy: [f64; 2],
    pub neutral_tc: [f64; 2],
    pub locus: &'a [[f64; 2]],
}
