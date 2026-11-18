//! Film-coordinate Dye grain inputs.
//!
//! The delivered continuous law lives in [`texture`]; this module owns the
//! shared readout geometry and the resolved inputs it consumes. The law and
//! its acceptance contract are in `specs/film-grain/dye/design.md`.

pub mod texture;

mod diagnostic_env;
mod value;
pub use diagnostic_env::diagnostic_env;

/// Rectangular readout footprints in global film coordinates. Cropping
/// changes the origin and dimensions, while retaining the full-frame pitch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReadoutGrid {
    pub origin_um: [f64; 2],
    pub pitch_um: [f64; 2],
    pub width: u32,
    pub height: u32,
}

/// Resolved Dye inputs for the delivered continuous law.
///
/// The profile, chemistry, density curves, and film geometry are consumed by
/// the upstream chain that forms the density target. They are not repeated
/// here: the field law only needs the record layout, the look controls,
/// the profile layer count, the seed, and the expectation diagnostic.
#[derive(Debug, Clone)]
pub struct DyeGrainParams {
    /// Profile-derived single-record layout, carried in three equal lanes.
    pub monochrome: bool,
    /// Profile polarity. Positive profiles use the finer delivered base cell.
    pub positive: bool,
    /// Relative delivered-law grain amplitude from the recipe. `1.0` keeps
    /// the delivered law; `0.0` realizes the target without random grain.
    pub dye_strength: f64,
    /// Relative correlation length. `1.0` keeps the polarity's delivered
    /// base cell; larger values scale the base, layer ladder, and fine cell.
    pub dye_size: f64,
    /// Relative separation of the three color records' grain fields.
    /// `0.0` shares the field; `1.0` gives each record its own. This maps to
    /// `rho = 1.0 - color_grain`; the default value is `0.75`.
    pub color_grain: f64,
    /// Active profile density layers. Each layer is an independent record of
    /// the same film; their density targets sum to the developed density.
    pub layer_count: u32,
    /// Per-layer, per-channel density peak from the profile's density curves.
    /// The layer taper reaches its asymptotic zero at
    /// `layer_dmax / uniformity`, above this sampled peak; unused entries
    /// repeat the first layer's value.
    pub layer_dmax: [[f64; 3]; 3],
    /// Full 64-bit seed; field and count addresses derive from it.
    pub seed: u64,
    /// Replace every draw with its conditional mean (diagnostic only).
    pub expectation: bool,
}
