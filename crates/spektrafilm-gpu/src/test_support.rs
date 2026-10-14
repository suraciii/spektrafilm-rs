//! Adapter acquisition for GPU tests.
//!
//! Every target environment (CI and development machines) provides a Vulkan
//! adapter through llvmpipe, so a missing adapter is a hard error rather than
//! a silent skip. A test that skips reports `ok`, which is indistinguishable
//! from a pass that actually executed.

#[cfg(feature = "wgpu-backend")]
use crate::wgpu_backend::WgpuBackend;

/// Adapter for a test that runs on the automatic route.
#[cfg(feature = "wgpu-backend")]
pub fn adapter() -> WgpuBackend {
    WgpuBackend::new().expect("WGPU adapter required for GPU tests")
}

/// Adapter for a test that exercises an explicit GPU request.
#[cfg(feature = "wgpu-backend")]
pub fn explicit_adapter() -> WgpuBackend {
    WgpuBackend::new_explicit().expect("WGPU adapter required for GPU tests")
}
