//! Presentation backend selection for the eframe shell.
//!
//! The UI compositor backend (`gui_renderer`) and the compute backend
//! (`gui_backend`) are chosen independently: the compositor avoids
//! wgpu/D3D12 where it is not needed, while Preview and Scan prefer WGPU.

use std::sync::Arc;

use spektrafilm_backend::ComputeBackend;

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

/// GUI Preview and Scan prefer WGPU even in a reference-precision build.
/// An explicit CPU override uses the native CPU backend, and an explicit
/// `SPEKTRAFILM_BACKEND=wgpu` request that cannot be served fails at launch
/// with [`spektrafilm_gpu::SelectionError`] instead of silently previewing on
/// CPU. Automatic selection keeps its faithful CPU fallback when no adapter
/// is available.
pub(super) fn gui_backend() -> Result<Arc<dyn ComputeBackend>, spektrafilm_gpu::SelectionError> {
    if std::env::var("SPEKTRAFILM_BACKEND").is_ok() {
        return Ok(Arc::from(spektrafilm_gpu::select_backend()?));
    }
    Ok(spektrafilm_gpu::wgpu_backend::WgpuBackend::new()
        .map(|backend| Arc::new(backend) as Arc<dyn ComputeBackend>)
        .unwrap_or_else(|| Arc::new(spektrafilm_backend::cpu_backend::CpuBackend)))
}
