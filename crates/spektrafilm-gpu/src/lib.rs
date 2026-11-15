//! WGPU compute backend for the film simulation pipeline.
//!
//! The `ComputeBackend` trait, its default CPU implementations, the shared
//! observation vocabulary, and the device-independent parameter records live
//! in `spektrafilm-backend`. This crate owns the WGPU implementation, its
//! device resources, and the fused film chain pipe.

mod gpu_helpers;

use parking_lot::Mutex;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_backend::cpu_backend::CpuBackend;
use spektrafilm_backend::telemetry::AdapterDescription;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "wgpu-backend")]
pub mod test_support;
#[cfg(feature = "wgpu-backend")]
pub mod wgpu_backend;

/// Failures wgpu reports for a device, attributed to the adapter that produced
/// them.
///
/// wgpu's default uncaptured-error handler panics on the first error, so a lost
/// device reaches a worker thread as a panic with no operation context
/// (`specs/compute-backend/spec.md`: a device failure MUST be a named operation
/// error, not a panic). This sink records failures, and every choke point that
/// crosses a device operation reports them as an ordinary `Err`.
///
/// Two kinds of failure are distinguished:
///
/// * A lost device is sticky. wgpu marks it invalid and every later call
///   fails; the first recorded cause is reported until the frontend replaces
///   the backend.
/// * An ordinary wgpu error (validation, out of memory) is reported once. It
///   fails the operation that hit it; the device itself stays usable, so the
///   next operation must not inherit the error.
///
/// The device-lost callback runs before the sink error for the same HAL
/// failure (`wgpu-core` loses the device inside `handle_hal_error`, then
/// reports the failed call), so the first record is the cause.
#[derive(Debug)]
pub struct DeviceFaults {
    identity: String,
    /// First failure ever seen, for the frontend's execution surface.
    first: Mutex<Option<String>>,
    /// Failure not yet returned to an operation.
    pending: Mutex<Option<String>>,
    lost: AtomicBool,
}

impl DeviceFaults {
    /// An empty sink attributed to `identity`. The device constructor installs
    /// it on the wgpu device; constructing one otherwise only serves
    /// `GpuSelection` records built by tests.
    pub fn new(identity: String) -> Self {
        Self {
            identity,
            first: Mutex::new(None),
            pending: Mutex::new(None),
            lost: AtomicBool::new(false),
        }
    }

    pub(crate) fn record(&self, message: String) {
        let mut first = self.first.lock();
        if first.is_none() {
            *first = Some(message.clone());
        }
        drop(first);
        let mut pending = self.pending.lock();
        if pending.is_none() {
            *pending = Some(message);
        }
    }

    pub(crate) fn record_lost(&self, reason: String, message: String) {
        self.lost.store(true, Ordering::Release);
        self.record(format!("device lost ({reason}): {message}"));
    }

    /// True once wgpu marked the device invalid. Only a lost device may skip a
    /// device wait: an ordinary error leaves the queue usable, so its
    /// submissions still have to retire.
    pub fn lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    /// The first attributed failure, once one has occurred. The device that
    /// produced it cannot be repaired in place.
    pub fn recorded(&self) -> Option<String> {
        self.first
            .lock()
            .as_ref()
            .map(|message| format!("{}: {message}", self.identity))
    }

    /// The attributed failure this operation must report, if any.
    pub fn check(&self) -> Result<(), String> {
        let message = if self.lost() {
            // Sticky: the device is invalid, so every later operation reports
            // the cause that lost it.
            self.recorded()
        } else {
            self.pending
                .lock()
                .take()
                .map(|message| format!("{}: {message}", self.identity))
        };
        match message {
            Some(message) => Err(format!("GPU device failure: {message}")),
            None => Ok(()),
        }
    }
}

/// What the selected device can execute, independent of its identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub shader_f64: bool,
    pub shader_int64: bool,
    pub max_buffer_size: u64,
    pub max_storage_buffer_binding_size: u32,
}

/// The claimed GPU device: its identity, what it can execute, and the failures
/// reported after selection.
#[derive(Clone, Debug)]
pub struct GpuSelection {
    pub adapter: AdapterDescription,
    pub capabilities: BackendCapabilities,
    /// Device failures reported after this selection. The frontend reads it to
    /// explain why a later operation failed; a failed device is replaced by a
    /// new selection, never repaired in place.
    pub faults: std::sync::Arc<DeviceFaults>,
}

impl GpuSelection {
    pub fn fault(&self) -> Option<String> {
        self.faults.recorded()
    }
}

/// The compute backend a caller requests explicitly.
///
/// This is the frontend spelling (`auto`, `gpu`, `cpu`) of the same policy the
/// historical `SPEKTRAFILM_BACKEND` variable expresses; the variable keeps its
/// own `wgpu` value (spec gpu-execution §backend selection: existing flags
/// retain their syntax).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackendChoice {
    #[default]
    Auto,
    Gpu,
    Cpu,
}

impl BackendChoice {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "gpu" | "wgpu" => Some(Self::Gpu),
            "cpu" => Some(Self::Cpu),
            _ => None,
        }
    }

    pub fn argument(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Gpu => "gpu",
            Self::Cpu => "cpu",
        }
    }
}

/// The WGPU API a GPU selection is restricted to.
///
/// `Auto` claims the platform's complete primary adapter set, which is the
/// historical selection. A named API restricts the instance so a user can
/// avoid a driver whose device fails (`specs/gpu-execution`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuApi {
    #[default]
    Auto,
    Vulkan,
    Dx12,
    Metal,
}

impl GpuApi {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "vulkan" => Some(Self::Vulkan),
            "dx12" => Some(Self::Dx12),
            "metal" => Some(Self::Metal),
            _ => None,
        }
    }

    pub fn argument(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Vulkan => "vulkan",
            Self::Dx12 => "dx12",
            Self::Metal => "metal",
        }
    }

    /// APIs this platform can claim. `Auto` is always the first entry.
    pub fn available() -> &'static [Self] {
        #[cfg(target_os = "windows")]
        {
            &[Self::Auto, Self::Vulkan, Self::Dx12]
        }
        #[cfg(target_os = "macos")]
        {
            &[Self::Auto, Self::Metal]
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            &[Self::Auto, Self::Vulkan]
        }
    }

    /// The `AdapterDescription::api` string this choice claims, when the
    /// choice names one.
    pub fn adapter_api(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Vulkan => Some("Vulkan"),
            Self::Dx12 => Some("Dx12"),
            Self::Metal => Some("Metal"),
        }
    }

    #[cfg(feature = "wgpu-backend")]
    pub(crate) fn backends(self) -> wgpu::Backends {
        match self {
            Self::Auto => wgpu::Backends::PRIMARY,
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::Dx12 => wgpu::Backends::DX12,
            Self::Metal => wgpu::Backends::METAL,
        }
    }
}

/// The requested backend failed to initialize.
///
/// The selector returns `Err` only for an explicit request: a missing WGPU
/// implementation, an unavailable adapter, or a GPU API the caller restricted
/// to is reported instead of being demoted to CPU (spec gpu-execution
/// §Backend selection: a missing implementation is not hardware
/// unavailability, and an explicit GPU request must fail rather than silently
/// fall back). Automatic selection without a GPU request never returns `Err`:
/// without a usable adapter it selects the complete CPU path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    /// An explicit GPU request reached a `spektrafilm-gpu` built without the
    /// `wgpu-backend` feature.
    WgpuUncompiled,
    /// An explicit GPU request found no usable hardware adapter.
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

/// A selected backend plus the identity of the device it claimed.
pub struct SelectedBackend {
    pub backend: Box<dyn ComputeBackend>,
    /// The claimed GPU device, when the selection executed on one.
    pub gpu: Option<GpuSelection>,
}

impl SelectedBackend {
    fn cpu(backend: impl ComputeBackend + 'static) -> Self {
        Self {
            backend: Box::new(backend),
            gpu: None,
        }
    }
}

/// The choice `SPEKTRAFILM_BACKEND` requests.
///
/// Historical values keep their meaning: `wgpu` is an explicit GPU request,
/// `cpu` an explicit CPU request, and anything else — including an unset
/// variable — is the automatic request.
pub fn environment_choice() -> BackendChoice {
    match std::env::var("SPEKTRAFILM_BACKEND")
        .ok()
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("wgpu") => BackendChoice::Gpu,
        Some("cpu") => BackendChoice::Cpu,
        _ => BackendChoice::Auto,
    }
}

/// Select the compute backend from `SPEKTRAFILM_BACKEND`.
///
/// An explicit `SPEKTRAFILM_BACKEND=wgpu` request selects f32 preview
/// arithmetic, including in an f64 binary, and fails with
/// [`SelectionError`] when it cannot be served. Omitting the variable
/// keeps automatic selection: a usable hardware adapter, otherwise the
/// native CPU path (`CPU f64` under `precision-f64`).
pub fn select_backend() -> Result<Box<dyn ComputeBackend>, SelectionError> {
    select_backend_for(environment_choice(), default_gpu_api()).map(|selected| selected.backend)
}

/// Select the backend a frontend request and GPU API restriction name.
///
/// This is the one selection rule: the environment path and every frontend
/// route call it, so a GUI choice cannot drift from `SPEKTRAFILM_BACKEND`.
pub fn select_backend_for(
    choice: BackendChoice,
    api: GpuApi,
) -> Result<SelectedBackend, SelectionError> {
    #[cfg(feature = "precision-f64")]
    if choice == BackendChoice::Auto {
        tracing::info!(precision = "f64", "using CPU reference backend");
        return Ok(SelectedBackend::cpu(CpuBackend));
    }

    if choice == BackendChoice::Cpu {
        tracing::info!(backend = CpuBackend.name(), "using CPU backend");
        return Ok(SelectedBackend::cpu(CpuBackend));
    }

    #[cfg(not(feature = "wgpu-backend"))]
    {
        let _ = api;
        return Err(SelectionError::WgpuUncompiled);
    }

    #[cfg(feature = "wgpu-backend")]
    {
        let explicit = choice == BackendChoice::Gpu;
        // A named API is a demand, not a preference: falling back to another
        // driver would hide the failure the user is investigating.
        let api_restricted = api != GpuApi::Auto;
        let construct = if explicit {
            wgpu_backend::WgpuBackend::new_explicit_with
        } else {
            wgpu_backend::WgpuBackend::new_with
        };
        if let Some(gpu) = construct(api.backends()) {
            tracing::info!(
                precision = "f32",
                reference = false,
                api = api.argument(),
                "using wgpu preview backend"
            );
            let selection = GpuSelection {
                adapter: gpu
                    .adapter_description()
                    .expect("a WGPU backend reports its adapter"),
                capabilities: gpu.capabilities().clone(),
                faults: gpu.faults(),
            };
            return Ok(SelectedBackend {
                backend: Box::new(gpu),
                gpu: Some(selection),
            });
        }
        // An unrestricted automatic request keeps its CPU fallback. A request
        // that named a GPU or an API asked for a device and must fail instead.
        if explicit || api_restricted {
            return Err(SelectionError::WgpuUnavailable);
        }
    }
    tracing::info!(backend = CpuBackend.name(), "using CPU backend");
    Ok(SelectedBackend::cpu(CpuBackend))
}

/// The selection an environment request makes: no API restriction.
fn default_gpu_api() -> GpuApi {
    GpuApi::Auto
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_failure_keeps_its_cause_and_adapter_identity() {
        let faults = DeviceFaults::new("Vulkan · Test Adapter (discrete)".to_owned());
        assert_eq!(faults.recorded(), None);
        assert_eq!(faults.check(), Ok(()));
        assert!(!faults.lost());
        faults.record_lost("Unknown".to_owned(), "HAL lost".to_owned());
        assert!(faults.lost());
        // The invalid-device validation error that follows the loss must not
        // replace the cause.
        faults.record("Validation Error: Device is invalid".to_owned());
        assert_eq!(
            faults.recorded().as_deref(),
            Some("Vulkan · Test Adapter (discrete): device lost (Unknown): HAL lost")
        );
        // A lost device stays failed for every later operation.
        for _ in 0..2 {
            assert_eq!(
                faults.check(),
                Err("GPU device failure: Vulkan · Test Adapter (discrete): \
                     device lost (Unknown): HAL lost"
                    .to_owned())
            );
        }
    }

    #[test]
    fn an_ordinary_error_fails_one_operation_and_leaves_the_device_usable() {
        let faults = DeviceFaults::new("Vulkan · Test Adapter".to_owned());
        faults.record("Validation Error: usage mismatch".to_owned());
        assert_eq!(
            faults.check(),
            Err(
                "GPU device failure: Vulkan · Test Adapter: Validation Error: usage mismatch"
                    .to_owned()
            )
        );
        assert_eq!(faults.check(), Ok(()));
        // The device is still valid, so its submissions must still retire; only
        // the execution surface remembers what happened.
        assert!(!faults.lost());
        assert!(faults.recorded().is_some());
    }

    #[test]
    fn frontend_spelling_keeps_the_historical_environment_values() {
        assert_eq!(BackendChoice::parse("auto"), Some(BackendChoice::Auto));
        assert_eq!(BackendChoice::parse("GPU"), Some(BackendChoice::Gpu));
        assert_eq!(BackendChoice::parse("wgpu"), Some(BackendChoice::Gpu));
        assert_eq!(BackendChoice::parse(" cpu "), Some(BackendChoice::Cpu));
        assert_eq!(BackendChoice::parse("metal"), None);
        for choice in [BackendChoice::Auto, BackendChoice::Gpu, BackendChoice::Cpu] {
            assert_eq!(BackendChoice::parse(choice.argument()), Some(choice));
        }
    }

    #[test]
    fn gpu_api_spelling_round_trips_and_auto_leads_every_platform() {
        for api in [GpuApi::Auto, GpuApi::Vulkan, GpuApi::Dx12, GpuApi::Metal] {
            assert_eq!(GpuApi::parse(api.argument()), Some(api));
        }
        assert_eq!(GpuApi::parse("vulkan "), Some(GpuApi::Vulkan));
        assert_eq!(GpuApi::parse("gl"), None);
        assert_eq!(GpuApi::available().first(), Some(&GpuApi::Auto));
        assert_eq!(GpuApi::Dx12.adapter_api(), Some("Dx12"));
        assert_eq!(GpuApi::Auto.adapter_api(), None);
    }

    #[cfg(feature = "wgpu-backend")]
    #[test]
    fn gpu_api_maps_onto_the_wgpu_backend_flags() {
        assert_eq!(GpuApi::Auto.backends(), wgpu::Backends::PRIMARY);
        assert_eq!(GpuApi::Vulkan.backends(), wgpu::Backends::VULKAN);
        assert_eq!(GpuApi::Dx12.backends(), wgpu::Backends::DX12);
    }

    #[test]
    fn an_explicit_cpu_request_never_claims_a_device() {
        let selected =
            select_backend_for(BackendChoice::Cpu, GpuApi::Auto).expect("CPU is always available");
        assert!(!selected.backend.is_gpu());
        assert!(selected.gpu.is_none());
    }
}
