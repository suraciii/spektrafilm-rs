//! Development-only spectral diagnostics exposed as `spektrafilm-lab`.
//!
//! Lab commands compute through `spektrafilm-core`, share the user binaries'
//! command plumbing and parameter surface, and are gated on the f64
//! reference precision. They report values; they never judge them.

pub mod arrays;
pub mod cli;
pub mod evaluate;
pub mod experiment;
pub mod measure;
pub mod metrics;
pub mod report;
pub mod spectra;
