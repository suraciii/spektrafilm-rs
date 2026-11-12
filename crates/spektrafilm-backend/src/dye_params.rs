//! Device-independent Dye parameter records.
//!
//! These records cross the crate boundary in the `ComputeBackend::dye_field`
//! signature and in `FilmChainParams::grain_dye`, so they live with the trait
//! rather than with the device passes that consume them.

#[derive(Clone, Debug)]
pub struct DyeReadoutGrid {
    pub origin_um: [f64; 2],
    pub pitch_um: [f64; 2],
    pub width: u32,
    pub height: u32,
}

/// Packed representation of one device field request: where to sample the film
/// lattice, with which pitch, and with which draw.
#[derive(Clone, Debug)]
pub struct DyeGpuParams {
    pub source_pitch_um: f64,
    pub grid: DyeReadoutGrid,
    pub seed: u64,
    pub expectation: bool,
    /// One effective record carried in three equal lanes: every record reads the
    /// same field, the same strength and the same count draw.
    pub monochrome: bool,
    /// Relative density amplitude at the taper peak (the mapped law value).
    pub white: f64,
    /// Event area of one developed cluster, in square micrometres (the
    /// mapped law value).
    pub event_area_um2: f64,
    /// Relative correlation length of the field (the mapped law value).
    pub length_scale: f64,
}

/// Delivered Dye law constants.
///
/// The host model and the device passes read the same values from here, so a
/// delivered-look change cannot drift between executors. The value at `1.0`
/// look strength is the engine constant; recipe strength and scale are applied
/// by the caller.
pub mod law {
    /// Nominal base cell size of the value-noise field in micrometres, before
    /// `dye_scale`. Smaller octaves halve this size; there is no high-pass.
    pub const VALUE_BASE_UM: f64 = 7.3;
    /// Relative weights of the three value-noise octaves.
    pub const VALUE_WEIGHTS: [f64; 3] = [1.0, 0.5, 0.25];
    /// Converts the weighted value-noise sum to unit variance. Measured from
    /// the same smoothstep value-noise process the CPU and device evaluate.
    pub const VALUE_FIELD_SCALE: f64 = 2.3325;
    /// Relative density amplitude at the taper peak.
    pub const WHITE: f64 = 0.0385;
    /// Relative density amplitude per record.
    pub const STRENGTH: [f64; 3] = [1.0, 1.0, 1.18];
    /// Shared fraction of the field between records.
    pub const RHO: f64 = 0.8;
    /// Event area of one developed cluster, in square micrometres.
    pub const EVENT_AREA_UM2: f64 = 0.0015;
    /// Base (fog) density that keeps the event rate alive in the deep shadows.
    pub const FOG: f64 = 0.15;
    /// Record density at which the density scale saturates.
    pub const DMAX: f64 = 1.2;
    /// Development-probability uniformity.
    pub const UNIFORMITY: f64 = 0.97;
    /// Residual granularity fraction at full development.
    pub const TAPER_FLOOR: f64 = 0.18;

    /// Source-lattice cell size for `base_cell_px`.
    ///
    /// A cell size whose reciprocal is an integer puts every source pixel on
    /// the same value-noise lattice phase, which inflates or deflates the
    /// realized amplitude. Nudge only those commensurate sizes by 1.37%; the
    /// field is a stochastic texture, so this invisible change is preferable
    /// to a visible amplitude step. CPU and device both call this function.
    pub fn base_cell_px(scale: f64, pitch_um: f64) -> f64 {
        let mut base = VALUE_BASE_UM * scale / pitch_um;
        if !(base.is_finite() && base > 0.0) {
            return base;
        }
        for _ in 0..8 {
            let commensurate = (0..VALUE_WEIGHTS.len()).any(|octave| {
                let cells_per_pixel = (1u32 << octave) as f64 / base;
                (cells_per_pixel - cells_per_pixel.round()).abs() < 2e-3
            });
            if !commensurate {
                break;
            }
            base *= 1.0137;
        }
        base
    }

    /// Peak of the derived granularity shape over the record density.
    pub fn taper_peak() -> f64 {
        (1..=512)
            .map(|i| {
                let p = f64::from(i) / 512.0;
                let sat = 1.0 - p * UNIFORMITY;
                (sat * p * (1.0 - p)).max(0.0).sqrt()
            })
            .fold(0.0, f64::max)
            .max(1e-6)
    }
}
