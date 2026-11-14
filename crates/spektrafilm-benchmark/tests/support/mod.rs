//! Helpers shared by the test binaries of this package.
//!
//! Every integration test binary compiles this module on its own, so each
//! one uses a different subset of it.
#![allow(dead_code)]
pub mod fixtures;
pub mod progress;
