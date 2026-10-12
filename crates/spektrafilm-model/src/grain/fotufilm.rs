//! Fotufilm Film: a fitted crystal population and periodic light-integrating grain bank.
//! Port of Fotufilm revision 59d43471 (Apache-2.0). Stock data is supplied by callers;
//! this module does not borrow upstream stock identities or modify the developed tone.
mod population;
mod tiles;
pub use population::{Bin, RecordDiagnostics, Sublayer};
use spektrafilm_math::image::ImageBuf;
use std::sync::Arc;
pub use tiles::{Bank, PitchTables, TileLevel};

pub const PROVENANCE: &str = "Fotufilm 59d43471dbef45c85ae442e507a291fdebfb2ec5 CPU port; endpoint grain lookup; complete periodic rectangular aperture extension above 96um";
pub type Cancellation<'a> = &'a dyn Fn() -> Result<(), String>;
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub log_exposure: Vec<f64>,
    pub density: Vec<f64>,
    pub d_min: f64,
    pub d_max: f64,
}
impl Curve {
    pub fn validate(&self) -> Result<(), String> {
        if self.log_exposure.len() < 2
            || self.log_exposure.len() != self.density.len()
            || !self.d_min.is_finite()
            || !self.d_max.is_finite()
            || self.d_max <= self.d_min
            || self
                .log_exposure
                .iter()
                .chain(&self.density)
                .any(|v| !v.is_finite())
            || self.log_exposure.windows(2).any(|v| v[1] <= v[0])
            || self.density.windows(2).any(|v| v[1] < v[0])
        {
            return Err("Fotufilm requires finite ascending exposure samples and monotone developed density curves with positive range".into());
        }
        Ok(())
    }
    pub fn density_at(&self, x: f64) -> f64 {
        interpolate(&self.log_exposure, &self.density, x)
    }
    pub fn exposure_at(&self, d: f64) -> f64 {
        interpolate(&self.density, &self.log_exposure, d)
    }
}
fn interpolate(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    if x <= xs[0] {
        return ys[0];
    }
    if x >= xs[xs.len() - 1] {
        return ys[ys.len() - 1];
    }
    let i = xs.partition_point(|v| *v <= x).min(xs.len() - 1);
    let span = xs[i] - xs[i - 1];
    if span <= 0.0 {
        ys[i]
    } else {
        ys[i - 1] + (ys[i] - ys[i - 1]) * (x - xs[i - 1]) / span
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Metadata {
    pub sigma48: [f64; 3],
    pub anchor_net_density: [f64; 3],
    pub cloud_radius_mm: f64,
    pub layer_size_ratio: [f64; 3],
    pub radius_span: f64,
    pub sublayer_shares: [f64; 4],
    pub coating_density_scale: f64,
    pub fog_density: f64,
    pub reversal: bool,
    pub silver: bool,
    pub monochrome: bool,
}
impl Default for Metadata {
    fn default() -> Self {
        Self {
            sigma48: [0.009 * 0.7, 0.009, 0.009 * 1.35],
            anchor_net_density: [1.0; 3],
            cloud_radius_mm: 0.005,
            layer_size_ratio: [1.0; 3],
            radius_span: 6.0,
            sublayer_shares: [0.25; 4],
            coating_density_scale: 1.0,
            fog_density: 0.03,
            reversal: false,
            silver: false,
            monochrome: false,
        }
    }
}
impl Metadata {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .sigma48
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0 || *v > 0.2)
            || self
                .anchor_net_density
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0)
            || !self.cloud_radius_mm.is_finite()
            || self.cloud_radius_mm <= 0.0
            || self.cloud_radius_mm > 0.1
            || self
                .layer_size_ratio
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v > 20.0)
            || !self.radius_span.is_finite()
            || !(1.0..=20.0).contains(&self.radius_span)
            || self
                .sublayer_shares
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v >= 1.0)
            || (self.sublayer_shares.iter().sum::<f64>() - 1.0).abs() > 1e-6
            || !self.coating_density_scale.is_finite()
            || !(0.25..=4.0).contains(&self.coating_density_scale)
            || !self.fog_density.is_finite()
            || self.fog_density < 0.0
            || self.fog_density > 1.0
        {
            return Err("invalid Fotufilm effective population metadata".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct PopulationInput {
    pub curves: [Curve; 3],
    pub reference_curves: Option<[Curve; 3]>,
    pub metadata: Metadata,
    pub reference_metadata: Option<Metadata>,
}
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub source_pitch_mm: [f64; 2],
    /// Full-frame top-left boundary of the first output pixel, in film millimetres.
    pub origin_mm: [f64; 2],
    pub output_pitch_mm: [f64; 2],
    pub footprint_mm: [f64; 2],
}
#[derive(Debug, Clone, Copy)]
pub struct Look {
    pub amount: f64,
    pub size: f64,
    pub color: f64,
    pub softness: f64,
    pub layers: [f64; 3],
}
impl Default for Look {
    fn default() -> Self {
        Self {
            amount: 1.0,
            size: 1.0,
            color: 1.0,
            softness: 1.0,
            layers: [1.0; 3],
        }
    }
}
#[derive(Debug, Clone)]
pub struct Diagnostics {
    pub provenance: &'static str,
    pub records: [RecordDiagnostics; 3],
    pub anchor_target: [f64; 3],
    pub anchor_measured: [f64; 3],
    pub anchor_log_error: [f64; 3],
    pub anchor_passes: [usize; 3],
}
#[derive(Debug, Clone)]
pub struct Population {
    pub records: [population::Record; 3],
    pub monochrome: bool,
    diagnostics: Diagnostics,
}
impl Population {
    pub fn prepare(input: PopulationInput, cancel: Cancellation<'_>) -> Result<Self, String> {
        population::prepare(input, cancel)
    }
    pub fn diagnostics(&self) -> Diagnostics {
        self.diagnostics.clone()
    }
    pub fn build_bank(&self, cancel: Cancellation<'_>) -> Result<Arc<Bank>, String> {
        tiles::build_bank(self, cancel).map(Arc::new)
    }
}
impl Bank {
    pub fn apply(
        &self,
        input: &ImageBuf,
        geometry: Geometry,
        look: Look,
        seed: u64,
        cancel: Cancellation<'_>,
    ) -> Result<ImageBuf, String> {
        self.apply_image(input, geometry, look, seed, cancel)
    }
}
