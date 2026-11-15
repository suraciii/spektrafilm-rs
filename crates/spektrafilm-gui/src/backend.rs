//! Presentation backend selection for the eframe shell.
//!
//! The UI compositor backend (`gui_renderer`) and the compute backend
//! (`gui_backend`) are chosen independently: the compositor avoids
//! wgpu/D3D12 where it is not needed, while Preview and Scan prefer WGPU.
//!
//! The compute backend is a user setting (`rust.backend`, `rust.gpu_api` in
//! the GUI state) resolved through the one selection rule in
//! [`spektrafilm_gpu::select_backend_for`]. `SPEKTRAFILM_BACKEND` keeps its
//! precedence: when it is set, the GUI shows the request as environment-owned
//! and does not offer a different selection.

use std::sync::Arc;

use spektrafilm_backend::ComputeBackend;
use spektrafilm_core::telemetry::BackendRequested;
use spektrafilm_gpu::{BackendChoice, GpuApi, GpuSelection};

/// UI compositor backend. Metal-backed CAMetalLayer is required on macOS so
/// the window colorspace can be tagged sRGB; Windows and Linux keep the UI
/// compositor away from wgpu/D3D12 by default.
pub(super) fn gui_renderer() -> eframe::Renderer {
    match std::env::var("SPEKTRAFILM_GUI_RENDERER") {
        Ok(v) if v.eq_ignore_ascii_case("wgpu") => eframe::Renderer::Wgpu,
        Ok(v) if v.eq_ignore_ascii_case("glow") => eframe::Renderer::Glow,
        _ => {
            if cfg!(target_os = "macos") {
                eframe::Renderer::Wgpu
            } else {
                eframe::Renderer::Glow
            }
        }
    }
}

/// The raw `SPEKTRAFILM_BACKEND` value, when the environment owns selection.
/// An unrecognized value still owns it: the environment asked for automatic
/// selection explicitly, and the GUI must not silently override that.
pub(super) fn environment_value() -> Option<String> {
    std::env::var("SPEKTRAFILM_BACKEND").ok()
}

/// GUI Preview and Scan prefer WGPU even in a reference-precision build.
/// An explicit CPU override uses the native CPU backend, and an explicit
/// `SPEKTRAFILM_BACKEND=wgpu` request that cannot be served fails at launch
/// with [`spektrafilm_gpu::SelectionError`] instead of silently previewing on
/// CPU. Automatic selection keeps its faithful CPU fallback when no adapter
/// is available.
pub(super) fn gui_backend() -> Result<Selected, spektrafilm_gpu::SelectionError> {
    spektrafilm_gpu::select_backend_for(
        spektrafilm_gpu::environment_choice(),
        spektrafilm_gpu::GpuApi::Auto,
    )
    .map(|selected| (Arc::from(selected.backend), selected.gpu))
}

/// A built selection: the backend and the GPU device it claimed, if any.
pub(super) type Selected = (Arc<dyn ComputeBackend>, Option<GpuSelection>);

/// Build the backend one request names. The error is the selector's own
/// message, so a failed GUI choice reports the same reason the CLI would.
pub(super) fn build(choice: BackendChoice, api: GpuApi) -> Result<Selected, String> {
    let selected = spektrafilm_gpu::select_backend_for(choice, api).map_err(|error| {
        let mut message = error.to_string();
        if let Some(named) = api.adapter_api() {
            message.push_str(&format!(" (requested GPU API: {named})"));
        }
        message
    })?;
    Ok((Arc::from(selected.backend), selected.gpu))
}

/// The choice that actually applies: the environment request owns selection
/// when it is set, otherwise the persisted preference does.
pub(super) fn effective_choice(persisted: BackendChoice) -> BackendChoice {
    if environment_value().is_some() {
        spektrafilm_gpu::environment_choice()
    } else {
        persisted
    }
}

/// True when the running backend already satisfies the request. Rebuilding an
/// identical selection would compile every pipeline again for nothing.
///
/// The API restriction is judged on a claimed device. A CPU request never needs
/// one, and automatic selection may already have concluded CPU for this build
/// or machine: demanding a device there would report a failure the request did
/// not cause.
pub(super) fn satisfied(
    running_gpu: bool,
    gpu: Option<&GpuSelection>,
    choice: BackendChoice,
    api: GpuApi,
) -> bool {
    let claimed_api = || match api.adapter_api() {
        None => true,
        Some(named) => gpu.is_some_and(|gpu| gpu.adapter.api.eq_ignore_ascii_case(named)),
    };
    match choice {
        BackendChoice::Cpu => !running_gpu,
        BackendChoice::Gpu => running_gpu && claimed_api(),
        BackendChoice::Auto => !running_gpu || claimed_api(),
    }
}

/// The request a recorded operation names, for telemetry.
pub(super) fn requested(choice: BackendChoice) -> BackendRequested {
    match choice {
        BackendChoice::Auto => BackendRequested::Auto,
        BackendChoice::Gpu => BackendRequested::Wgpu,
        BackendChoice::Cpu => BackendRequested::Cpu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_requests_accept_any_running_backend() {
        assert!(satisfied(false, None, BackendChoice::Auto, GpuApi::Auto));
        assert!(satisfied(false, None, BackendChoice::Cpu, GpuApi::Auto));
        assert!(!satisfied(false, None, BackendChoice::Gpu, GpuApi::Auto));
    }

    /// A claimed device selection with the API and no real device behind it.
    fn gpu_selection(api: &str) -> GpuSelection {
        use spektrafilm_backend::telemetry::{
            AdapterDescription, AdapterDeviceType, AvailabilityReason, Measurement,
        };
        GpuSelection {
            adapter: AdapterDescription {
                api: api.to_owned(),
                device_type: AdapterDeviceType::Discrete,
                name: "Test Adapter".to_owned(),
                description_truncated: false,
                shared_device: true,
                timestamp_supported: false,
                timestamp_enabled: false,
                timestamp_period_ns: Measurement::unavailable(AvailabilityReason::Unsupported),
                timestamp_reason: Some(AvailabilityReason::Unsupported),
            },
            capabilities: spektrafilm_gpu::BackendCapabilities {
                shader_f64: true,
                shader_int64: true,
                max_buffer_size: 1 << 30,
                max_storage_buffer_binding_size: 1 << 27,
            },
            faults: std::sync::Arc::new(spektrafilm_gpu::DeviceFaults::new("test".to_owned())),
        }
    }

    #[test]
    fn a_named_gpu_api_is_only_satisfied_by_that_api() {
        // Automatic selection already concluded CPU: the API is not a demand.
        assert!(satisfied(false, None, BackendChoice::Auto, GpuApi::Vulkan));
        // A CPU request never needs a device, whatever API is recorded.
        assert!(satisfied(false, None, BackendChoice::Cpu, GpuApi::Vulkan));
        assert!(!satisfied(false, None, BackendChoice::Gpu, GpuApi::Auto));
        let vulkan = gpu_selection("Vulkan");
        assert!(satisfied(
            true,
            Some(&vulkan),
            BackendChoice::Gpu,
            GpuApi::Vulkan
        ));
        assert!(satisfied(
            true,
            Some(&vulkan),
            BackendChoice::Auto,
            GpuApi::Vulkan
        ));
        assert!(!satisfied(
            true,
            Some(&vulkan),
            BackendChoice::Gpu,
            GpuApi::Dx12
        ));
        assert!(!satisfied(
            true,
            Some(&vulkan),
            BackendChoice::Cpu,
            GpuApi::Auto
        ));
        assert_eq!(GpuApi::Dx12.adapter_api(), Some("Dx12"));
    }

    #[test]
    fn telemetry_keeps_the_historical_request_vocabulary() {
        assert_eq!(requested(BackendChoice::Auto), BackendRequested::Auto);
        assert_eq!(requested(BackendChoice::Gpu), BackendRequested::Wgpu);
        assert_eq!(requested(BackendChoice::Cpu), BackendRequested::Cpu);
    }
}
