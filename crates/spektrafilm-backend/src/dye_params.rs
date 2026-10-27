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
}
