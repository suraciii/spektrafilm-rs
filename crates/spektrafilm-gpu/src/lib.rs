//! WGPU compute backend for the film simulation pipeline.
//!
//! The `ComputeBackend` trait, its default CPU implementations, the shared
//! observation vocabulary, and the device-independent parameter records live
//! in `spektrafilm-backend`. This crate owns the WGPU implementation, its
//! device resources, and the fused film chain pipe.

mod gpu_helpers;

use spektrafilm_backend::ComputeBackend;
use spektrafilm_backend::cpu_backend::CpuBackend;

#[cfg(feature = "wgpu-backend")]
pub mod wgpu_backend;

#[cfg(feature = "wgpu-backend")]
pub mod test_support;

/// The requested backend failed to initialize.
///
/// The selector returns `Err` only for an explicit request: a missing
/// WGPU implementation or an unavailable adapter is reported to the caller
/// instead of being demoted to CPU (spec gpu-execution §Backend selection:
/// a missing implementation is not hardware unavailability, and an explicit
/// GPU request must fail rather than silently fall back). Automatic
/// selection never returns `Err`: without a usable adapter it selects the
/// complete CPU path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    /// An explicit `SPEKTRAFILM_BACKEND=wgpu` request reached a
    /// `spektrafilm-gpu` built without the `wgpu-backend` feature.
    WgpuUncompiled,
    /// An explicit `SPEKTRAFILM_BACKEND=wgpu` request found no usable
    /// hardware adapter.
    WgpuUnavailable,
}

impl std::fmt::Display for SelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WgpuUncompiled => write!(
                f,
                "explicit GPU request cannot be served: this spektrafilm build was compiled \
                 without the wgpu-backend feature; rebuild with --features wgpu-backend or \
                 retry with an explicit CPU request"
            ),
            Self::WgpuUnavailable => write!(
                f,
                "explicit GPU request cannot be served: no usable WGPU adapter is available \
                 on this system; retry with an explicit CPU request"
            ),
        }
    }
}

impl std::error::Error for SelectionError {}

/// Select the compute backend from `SPEKTRAFILM_BACKEND`.
///
/// An explicit `SPEKTRAFILM_BACKEND=wgpu` request selects f32 preview
/// arithmetic, including in an f64 binary, and fails with
/// [`SelectionError`] when it cannot be served. Omitting the variable
/// keeps automatic selection: a usable hardware adapter, otherwise the
/// native CPU path (`CPU f64` under `precision-f64`).
pub fn select_backend() -> Result<Box<dyn ComputeBackend>, SelectionError> {
    let requested = std::env::var("SPEKTRAFILM_BACKEND")
        .ok()
        .map(|v| v.to_ascii_lowercase());
    let explicit_wgpu = requested.as_deref() == Some("wgpu");
    #[cfg(feature = "precision-f64")]
    if requested.is_none() {
        tracing::info!(precision = "f64", "using CPU reference backend");
        return Ok(Box::new(CpuBackend));
    }

    if requested.as_deref() == Some("cpu") {
        tracing::info!(backend = CpuBackend.name(), "using CPU backend");
        return Ok(Box::new(CpuBackend));
    }

    #[cfg(not(feature = "wgpu-backend"))]
    if explicit_wgpu {
        return Err(SelectionError::WgpuUncompiled);
    }
    #[cfg(feature = "wgpu-backend")]
    {
        if requested.is_none() || explicit_wgpu {
            let constructor = if explicit_wgpu {
                wgpu_backend::WgpuBackend::new_explicit
            } else {
                wgpu_backend::WgpuBackend::new
            };
            if let Some(gpu) = constructor() {
                tracing::info!(
                    precision = "f32",
                    reference = false,
                    "using wgpu preview backend"
                );
                return Ok(Box::new(gpu));
            }
            if explicit_wgpu {
                return Err(SelectionError::WgpuUnavailable);
            }
        }
    }
    tracing::info!(backend = CpuBackend.name(), "using CPU backend");
    Ok(Box::new(CpuBackend))
}
