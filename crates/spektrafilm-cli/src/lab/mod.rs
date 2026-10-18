//! Instrument command surface: the research commands exposed as
//! `spektrafilm-lab`.
//!
//! Instrument commands compute through `spektrafilm-core` and the
//! renderer-independent evaluation library, share the user binaries' command
//! plumbing and parameter surface, and are gated on the f64 reference
//! precision. They report values; they never judge them.

/// Scene-linear RGB space used when measuring spectral stimuli.
pub(super) const SCENE_RGB_SPACE: &str = "ACES2065-1";

pub mod arrays;
pub mod cli;
pub mod evaluate;
pub mod experiment;
pub mod measure;
pub mod metrics;
pub mod report;
pub mod spectra;
