//! Device-independent Dye parameter records.
//!
//! These records cross the crate boundary in the `ComputeBackend::dye_field`
//! signature and in `FilmChainParams::grain_dye`, so they live with the trait
//! rather than with the device passes that consume them.

/// Profile density curves consumed by the layered Dye field. `axis` is the
/// normalized composite curve and `curves[row][layer][channel]` is the raw
/// layer curve on the same exposure axis.
#[derive(Clone, Debug)]
pub struct DyeLayerTable {
    pub layer_count: u32,
    pub axis: Vec<[f64; 3]>,
    pub curves: Vec<[[f64; 3]; 3]>,
    pub sign: f64,
}

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
    /// Shared field fraction between the three color records. `1.0` is one
    /// shared field; `0.0` gives each record its own. The public color-grain
    /// default `0.75` maps to `0.25`; monochrome holds the previous `0.8`.
    pub rho: f64,
    /// Relative density amplitude at the taper peak (the mapped law value).
    pub white: f64,
    /// Event area of one developed cluster, in square micrometres (the
    /// mapped law value).
    pub event_area_um2: f64,
    /// Base (fog) density for this layer's record.
    pub fog: f64,
    /// Relative correlation length of the field (the mapped law value).
    pub length_scale: f64,
    /// Relative cell size of the positive microcontrast component.
    pub fine_scale: f64,
    /// Variance fraction of the positive microcontrast component.
    pub fine_weight: f64,
    /// Relative correlation scale per active layer. Negative profiles use the
    /// delivered ladder, where later entries resolve progressively finer
    /// density layers; positive profiles pass the base value for every layer.
    pub layer_scales: [f64; 3],
    /// Active profile layers. `1` means the total-density single-layer law.
    pub layer_count: u32,
    /// Profile layer curves when `layer_count > 1`.
    pub layer_table: Option<std::sync::Arc<DyeLayerTable>>,
}

/// Delivered Dye law constants.
///
/// The host model and the device passes read the same values from here, so a
/// delivered-look change cannot drift between executors. The value at `1.0`
/// look strength is the engine constant; the recipe strength is applied by the
/// caller.
pub mod law {
    /// Delivered base cell size of the value-noise field in micrometres.
    /// There is no octave stack; profile density layers carry the multi-layer
    /// structure, and positive profiles add one finer microcontrast component.
    pub const VALUE_BASE_UM: f64 = 14.0;
    /// Converts one smoothstep value-noise field to unit variance. Measured
    /// from the same process the CPU and device evaluate.
    pub const VALUE_FIELD_SCALE: f64 = 2.313;
    /// Relative correlation scale per profile density layer for negative
    /// profiles. The layers share one grain law but resolve as distinct
    /// emulsion depths; these engine constants are not Spektra's particle-scale
    /// controls. Positive profiles keep the base cell on every layer because
    /// their dominant layer would otherwise fall below one source pixel on a
    /// 35 mm 1:1 crop.
    pub const LAYER_SCALES: [f64; 3] = [1.0, 0.5, 0.3];
    /// Relative density amplitude at the taper peak. The delivered value keeps
    /// 100% midtone texture visible without restoring the old pixel-noise floor.
    pub const WHITE: f64 = 0.0308;
    /// Positive-profile delivered density-amplitude scale. Positive material
    /// spends much of its record range where a direct scan maps density
    /// modulation to a smaller encoded contrast than a print scan; the positive
    /// delivered law therefore carries twice the negative amplitude.
    pub const POSITIVE_AMPLITUDE_SCALE: f64 = 2.0;
    /// Relative cell of the positive microcontrast component.
    pub const POSITIVE_FINE_SCALE: f64 = 0.55;
    /// Variance fraction carried by the positive microcontrast component.
    pub const POSITIVE_FINE_WEIGHT: f64 = 0.35;
    /// Relative density amplitude per record.
    pub const STRENGTH: [f64; 3] = [1.0, 1.0, 1.18];
    /// Shared fraction retained by the monochrome law and the previous
    /// delivered color look. Color film maps the public value as
    /// `rho = 1 - dye_color_grain`; its default `0.75` maps to `0.25`.
    pub const RHO: f64 = 0.8;
    /// Event area of one developed cluster, in square micrometres.
    pub const EVENT_AREA_UM2: f64 = 0.00096;
    /// Base (fog) density that keeps the event rate alive in the deep shadows.
    pub const FOG: f64 = 0.15;
    /// Record density at which the density scale saturates.
    pub const DMAX: f64 = 1.2;
    /// Development-probability uniformity.
    pub const UNIFORMITY: f64 = 0.97;

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
            let cells_per_pixel = 1.0 / base;
            if (cells_per_pixel - cells_per_pixel.round()).abs() >= 2e-3 {
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
