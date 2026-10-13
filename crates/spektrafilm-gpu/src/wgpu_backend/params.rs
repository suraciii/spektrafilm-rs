//! Shared uniform ABI structs for GPU spectral shaders.
//!
//! These structs define the `#[repr(C)]` layout shared between the CPU
//! host code and the WGSL shaders. All fields are `f32`/`u32` — the GPU
//! boundary narrows any `f64` precision-preserved inputs.

/// Uniform parameters for density-curve interpolation shaders.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct DensityParams {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) k: u32,
    pub(super) uniform_grid: u32,
    pub(super) gamma_inv: [f32; 3],
    pub(super) _pad: f32,
}

/// Uniform parameters for scan-spectral shaders.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ScanParams {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) n_wavelengths: u32,
    pub(super) normalization: f32,
    pub(super) col0: [f32; 4],
    pub(super) col1: [f32; 4],
    pub(super) col2: [f32; 4],
    pub(super) bw: [f32; 4],
}

/// Uniform parameters for the front pass (hanatos LUT lookup or mallett matmul).
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct FrontParams {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) lut_size: u32,
    pub(super) _pad: u32,
    pub(super) col0: [f32; 4],
    pub(super) col1: [f32; 4],
    pub(super) col2: [f32; 4],
}
