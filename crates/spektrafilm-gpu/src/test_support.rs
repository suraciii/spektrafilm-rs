//! Adapter acquisition for GPU tests that must distinguish "verified" from
//! "not run".
//!
//! A GPU test that returns early when no adapter is present reports `ok`,
//! which is indistinguishable from a pass that actually executed. Setting
//! `SPEKTRAFILM_REQUIRE_GPU=1` (CI does) turns a missing adapter into a
//! failure, so a green suite means the device path ran.

#[cfg(feature = "wgpu-backend")]
use crate::wgpu_backend::WgpuBackend;

/// True when the environment demands a real adapter instead of a skip.
pub fn require_gpu() -> bool {
    std::env::var("SPEKTRAFILM_REQUIRE_GPU").is_ok_and(|value| {
        let value = value.trim();
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    })
}

/// Panic instead of skipping when [`require_gpu`] is set.
fn missing_adapter(context: &str) -> ! {
    panic!(
        "SPEKTRAFILM_REQUIRE_GPU is set but no WGPU adapter is available ({context}); \
         install a Vulkan driver (mesa-vulkan-drivers) or unset the variable to skip"
    )
}

/// Adapter for a test that runs on the automatic route.
///
/// Returns `None` when no adapter is available and skipping is allowed;
/// panics when `SPEKTRAFILM_REQUIRE_GPU=1` is set.
#[cfg(feature = "wgpu-backend")]
pub fn adapter_or_skip() -> Option<WgpuBackend> {
    match WgpuBackend::new() {
        Some(backend) => Some(backend),
        None if require_gpu() => missing_adapter("WgpuBackend::new"),
        None => {
            eprintln!("no GPU adapter available; skipping");
            None
        }
    }
}

/// Adapter for a test that exercises an explicit GPU request.
#[cfg(feature = "wgpu-backend")]
pub fn explicit_adapter_or_skip() -> Option<WgpuBackend> {
    match WgpuBackend::new_explicit() {
        Some(backend) => Some(backend),
        None if require_gpu() => missing_adapter("WgpuBackend::new_explicit"),
        None => {
            eprintln!("no GPU adapter available; skipping");
            None
        }
    }
}
