//! Device-side description of a prepared Fotufilm population.
//!
//! The population solver (`population`) and the bank builder (`tiles`) own the
//! reference arithmetic. This module translates an anchored [`Population`] into
//! the plain records a device pass needs, so the GPU crate never depends on the
//! model and never re-derives a constant: every value below is either copied
//! from the fitted population or is one of the source's own fixed tables.
//!
//! Nothing here evaluates an image. The records are the resolved, bounded
//! coefficients of the formation law; the device evaluates that law over the
//! stochastic coating grid and reads the result out.

use super::*;
use super::{population, tiles};

/// Formation taps of one sublayer's developing-fraction table.
pub const FORMING_TAPS: usize = 129;
/// Poisson threshold count of one sublayer's coating count law.
pub const THRESHOLD_COUNT: usize = 48;
/// Covariance-window lag count of the 48-micrometer sigma48 measurement.
pub const SHARING_LAGS: usize = 17;
/// Canonical bank density levels.
pub const BANK_LEVELS: usize = 17;
/// Formation sample edge, in texels (the source's `SAMPLE`).
pub const SAMPLE_EDGE: usize = 768;
/// Canonical tile edge, in texels (the source's `N`).
pub const TILE_EDGE: usize = 256;
/// Periodic bank block edge, in texels.
pub const BLOCK: f64 = 128.0;
/// The four dye cloud terms: `(sigma scale, weight)`.
pub const CLOUD_TERMS: [(f32, f32); 4] = [
    (0.1162, 0.1096),
    (0.3089, 0.2167),
    (0.7469, 0.4030),
    (1.732, 0.2510),
];

/// One sublayer's formation coefficients.
#[derive(Clone, Debug)]
pub struct DeviceSublayer {
    /// Coating grid divisions along one axis (`round(0.256 / cell_mm)`).
    pub cells: u32,
    /// Silver develops uncovered grains; dye develops a dye cloud.
    pub silver: bool,
    /// Mark spread in millimetres, before the source's texel scaling.
    pub sigma_mm: f64,
    /// Local dye capacity in density units.
    pub capacity: f64,
    /// Unit-crystal mark scale.
    pub edge: f64,
    /// Developing fraction against normalized density, `FORMING_TAPS` values.
    pub forming: Vec<f64>,
    /// Cumulative Poisson count thresholds, `THRESHOLD_COUNT` values.
    pub thresholds: Vec<f32>,
    /// Largest texel distance from a crystal centre to any texel it can
    /// deposit into. The device uses it to bound the coating cells a tile
    /// visits; it is an upper bound, never a truncation of the law.
    pub reach_texels: f64,
    /// The largest crystal occupied area this sublayer can draw, as the
    /// reference's `occupied_area` resolves it. The device uses it to bound
    /// the uncovered square's side.
    pub occupied_area: f64,
}

/// One record's formation coefficients.
#[derive(Clone, Debug)]
pub struct DeviceRecord {
    pub d_min: f64,
    pub d_max: f64,
    pub sublayers: Vec<DeviceSublayer>,
}

/// The population's formation seed: the source's pinned `SEED`.
pub fn formation_seed() -> u32 {
    tiles::formation_seed()
}

impl Population {
    /// The device description of every record, in record order.
    ///
    /// A record with no sublayers stays empty: the device forms no bank level
    /// for it, exactly as `build_bank` skips it.
    pub fn device_records(&self) -> Vec<DeviceRecord> {
        self.records
            .iter()
            .map(|record| DeviceRecord {
                d_min: record.d_min,
                d_max: record.d_max,
                sublayers: record
                    .sublayers
                    .iter()
                    .map(|layer| {
                        // `light` derives the coating grid from the cell size
                        // and then the cell size back from the grid, so the
                        // device receives the grid and re-derives both.
                        let cells = (0.256 / layer.cell_mm).round().max(1.0) as u32;
                        let cell_mm = 0.256 / cells as f64;
                        // The count law's mean is formed in f32 exactly as the
                        // source forms it, then widened for the threshold sum.
                        let mean = (layer.crystals_per_mm2 as f32 * cell_mm as f32 * cell_mm as f32)
                            as f64;
                        let sigma = (layer.sigma_mm as f32) / (0.001f32 / 3.0_f32);
                        // A crystal deposits into every texel within
                        // `4.2 * sigma` of its centre, and its uncovered square
                        // spans `sigma * sqrt(occupied_area(peak))`; both are
                        // bounded by the largest mark the sublayer can draw.
                        let reach = 4.2 * sigma as f64
                            + sigma as f64
                                * population::occupied_area(layer.edge * 17.0, layer.silver).sqrt()
                            + 2.0;
                        DeviceSublayer {
                            cells,
                            silver: layer.silver,
                            sigma_mm: layer.sigma_mm,
                            capacity: layer.capacity,
                            edge: layer.edge,
                            forming: layer.forming.clone(),
                            thresholds: tiles::thresholds(mean).to_vec(),
                            reach_texels: reach,
                            // `light` sizes a silver crystal's uncovered
                            // square by `sigma * sqrt(occupied_area(peak))`,
                            // and `occupied_area` is monotone in the mark, so
                            // the sublayer's largest mark bounds it. The
                            // device receives that resolved bound rather than
                            // re-deriving the integral.
                            occupied_area: population::occupied_area(
                                layer.edge * 17.0,
                                layer.silver,
                            ),
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    /// Whether the device must form one bank level set or three.
    pub fn device_monochrome(&self) -> bool {
        self.monochrome
    }
}
