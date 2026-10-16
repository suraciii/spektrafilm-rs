//! WGPU compute backend for the film simulation pipeline.
//!
//! The `ComputeBackend` trait, its default CPU implementations, the shared
//! observation vocabulary, and the device-independent parameter records live
//! in `spektrafilm-backend`. This crate owns the WGPU implementation, its
//! device resources, and the fused film chain pipe.

mod gpu_helpers;

use spektrafilm_backend::ComputeBackend;
use spektrafilm_backend::cpu_backend::CpuBackend;

/// GPU FIR kernels support a maximum 256-pixel half-width.
pub(crate) fn gpu_blur_supported(sigma: f32) -> bool {
    sigma.is_finite() && (3.0 * sigma.max(0.0)).ceil() <= 256.0
}
#[cfg(feature = "wgpu-backend")]
pub mod wgpu_backend;

#[cfg(feature = "wgpu-backend")]
pub mod test_support;

/// Explicit `SPEKTRAFILM_BACKEND=wgpu` requests select f32 preview arithmetic,
/// including in an f64 binary. Unavailable requests report a faithful CPU fallback.
pub fn select_backend() -> Box<dyn ComputeBackend> {
    let requested = std::env::var("SPEKTRAFILM_BACKEND")
        .ok()
        .map(|v| v.to_ascii_lowercase());
    #[cfg(feature = "precision-f64")]
    if requested.is_none() {
        tracing::info!(precision = "f64", "using CPU reference backend");
        return Box::new(CpuBackend);
    }

    if requested.as_deref() == Some("cpu") {
        tracing::info!(backend = CpuBackend.name(), "using CPU backend");
        return Box::new(CpuBackend);
    }

    #[cfg(not(feature = "wgpu-backend"))]
    if requested.as_deref() == Some("wgpu") {
        tracing::warn!("wgpu backend requested but spektrafilm-gpu was built without wgpu-backend");
    }
    #[cfg(feature = "wgpu-backend")]
    {
        if requested.as_deref().is_none() || requested.as_deref() == Some("wgpu") {
            let constructor = if requested.as_deref() == Some("wgpu") {
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
                return Box::new(gpu);
            }
            if requested.as_deref() == Some("wgpu") {
                tracing::warn!("wgpu backend requested but unavailable; falling back");
            }
        }
    }
    tracing::info!(backend = CpuBackend.name(), "using CPU backend");
    Box::new(CpuBackend)
}
