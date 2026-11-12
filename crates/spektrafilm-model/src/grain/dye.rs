//! Film-coordinate Dye grain inputs.
//!
//! The delivered continuous law lives in [`texture`]; this module owns the
//! shared readout geometry and the resolved inputs it consumes. The law and
//! its acceptance contract are in `specs/film-grain/dye/design.md`.

pub mod texture;

mod diagnostic_env;
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
/// here: the field law only needs the record layout, the two look controls,
/// the seed, and the expectation diagnostic.
#[derive(Debug, Clone)]
pub struct DyeGrainParams {
    /// Profile-derived single-record layout, carried in three equal lanes.
    pub monochrome: bool,
    /// Relative delivered-law grain amplitude from the recipe. `1.0` keeps
    /// the delivered law; `0.0` realizes the target without random grain.
    pub dye_strength: f64,
    /// Relative delivered-law correlation-length scale from the recipe.
    pub dye_scale: f64,
    /// Full 64-bit seed; field and count addresses derive from it.
    pub seed: u64,
    /// Replace every draw with its conditional mean (diagnostic only).
    pub expectation: bool,
}
