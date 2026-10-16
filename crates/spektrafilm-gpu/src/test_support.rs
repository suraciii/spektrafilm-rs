//! Adapter acquisition for GPU tests.
//!
//! Every target environment (CI and development machines) provides a Vulkan
//! adapter through llvmpipe, so a missing adapter is a hard error rather than
//! a silent skip. A test that skips reports `ok`, which is indistinguishable
//! from a pass that actually executed.
//!
//! The software Vulkan driver (llvmpipe) is not safe under concurrent device
//! use in one process: parallel test threads crash with SIGSEGV (observed in
//! ~4 of 10 runs at four threads, 0 of 12 serial). Every device-creating test
//! therefore serializes on the shared lock below for its whole body — this
//! crate's own modules and every downstream crate that builds a backend.
//! The lock is not `#[cfg(test)]`: a downstream test binary links this crate
//! as a plain library, so a test-only item would be invisible to it.
//!
//! [`adapter`] and [`explicit_adapter`] acquire the lock themselves and return
//! a [`DeviceBackend`] that holds the guard for its whole lifetime, so a test
//! cannot hold a device without serializing on it. [`device_lock`] stays
//! available for the few tests that build a device some other way.

#[cfg(feature = "wgpu-backend")]
use crate::wgpu_backend::WgpuBackend;
#[cfg(feature = "wgpu-backend")]
use std::ops::Deref;

/// Serial lock shared by all wgpu-backend test modules: the software Vulkan
/// driver (llvmpipe) is not safe under concurrent device use in one process.
/// Held for the whole test so the device outlives the guard.
#[cfg(feature = "wgpu-backend")]
pub static DEVICE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Guard serializing device use for the whole test. Declare the returned guard
/// *before* the backend so reverse-order drop destroys the backend while the
/// lock is still held.
#[cfg(feature = "wgpu-backend")]
pub fn device_lock() -> std::sync::MutexGuard<'static, ()> {
    DEVICE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A backend that owns the device-serialization guard for its whole lifetime,
/// so the device can never outlive the lock that makes using it safe. Deref
/// exposes the backend, so call sites pass `&gpu` where `&dyn ComputeBackend`
/// is expected unchanged.
///
/// Field order matters in principle: Rust drops struct fields in *declaration*
/// order (unlike local variables, which drop in reverse), so declaring the
/// guard first would release the lock before the device is torn down. The
/// order below is the correct one and costs nothing to keep. Note that this
/// particular ordering was never observed to crash on llvmpipe — swapping it
/// ran 30/30 clean — so it is defensive, not a reproduced cause.
#[cfg(feature = "wgpu-backend")]
pub struct DeviceBackend {
    backend: WgpuBackend,
    _guard: std::sync::MutexGuard<'static, ()>,
}

#[cfg(feature = "wgpu-backend")]
impl Deref for DeviceBackend {
    type Target = WgpuBackend;
    fn deref(&self) -> &Self::Target {
        &self.backend
    }
}

/// The backend as a trait object. Rust does not coerce `&DeviceBackend` through
/// `Deref` into `&dyn ComputeBackend`, so a call site that passes the backend
/// to a `&dyn ComputeBackend` parameter writes `gpu.backend()`.
#[cfg(feature = "wgpu-backend")]
impl DeviceBackend {
    pub fn backend(&self) -> &dyn crate::ComputeBackend {
        &self.backend
    }
}

/// Wrap an already-built backend in the device lock. The lock is acquired by
/// [`adapter`]/[`explicit_adapter`] *before* the device is created, so device
/// creation itself is serialized too.
#[cfg(feature = "wgpu-backend")]
fn locked(build: impl FnOnce() -> WgpuBackend) -> DeviceBackend {
    // Acquire the lock first: `WgpuBackend::new` drives llvmpipe during
    // enumeration and device creation, which is exactly the operation that
    // must not overlap another thread's device work.
    let guard = device_lock();
    DeviceBackend {
        backend: build(),
        _guard: guard,
    }
}

/// Adapter for a test that runs on the automatic route. Holds the device lock
/// for the returned value's whole lifetime, including device creation.
#[cfg(feature = "wgpu-backend")]
pub fn adapter() -> DeviceBackend {
    locked(|| WgpuBackend::new().expect("WGPU adapter required for GPU tests"))
}

/// Adapter for a test that exercises an explicit GPU request. Holds the device
/// lock for the returned value's whole lifetime, including device creation.
#[cfg(feature = "wgpu-backend")]
pub fn explicit_adapter() -> DeviceBackend {
    locked(|| WgpuBackend::new_explicit().expect("WGPU adapter required for GPU tests"))
}
