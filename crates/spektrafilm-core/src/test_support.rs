//! Helpers shared by this crate's unit tests.

use std::path::{Path, PathBuf};

/// Workspace `data/` directory, resolved from this crate's manifest directory.
///
/// Every in-crate test that loads profiles, presets or LUT fixtures uses this
/// instead of re-deriving the path from `CARGO_MANIFEST_DIR`.
pub(crate) fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}
