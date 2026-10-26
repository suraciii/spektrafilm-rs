//! Immutable coefficients uploaded only when their chain stage is reached.
use super::super::{
    DensityParams, FrontParams, WgpuBackend, is_uniform_grid_endpoint, nan_safe_rows_to_f32,
    pack_matrix_columns_f64, sanitize_spectral_inputs,
};

pub(super) fn storage(backend: &WgpuBackend, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    backend
        .pipe
        .static_buffer(label, bytes, wgpu::BufferUsages::STORAGE)
}

pub(super) fn uniform(backend: &WgpuBackend, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    backend.pipe.static_buffer(
        label,
        bytes,
        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_SRC,
    )
}

pub(super) fn front(
    backend: &WgpuBackend,
    p: &spektrafilm_backend::FilmChainParams<'_>,
) -> wgpu::Buffer {
    let (matrix, lut_size) = match &p.front {
        spektrafilm_backend::FrontPass::Hanatos2025 {
            tc_lut,
            rgb_to_adapted_xyz,
            ..
        } => (rgb_to_adapted_xyz, tc_lut.size as u32),
        spektrafilm_backend::FrontPass::Mallett2019 { matrix } => (matrix, 0),
        spektrafilm_backend::FrontPass::ScanInverse(_) => {
            unreachable!("inverse has no front transform")
        }
    };
    let columns = pack_matrix_columns_f64(matrix);
    uniform(
        backend,
        "front_params",
        bytemuck::bytes_of(&FrontParams {
            width: p.image_width,
            height: p.image_height,
            lut_size,
            _pad: 0,
            col0: columns[0],
            col1: columns[1],
            col2: columns[2],
        }),
    )
}

pub(super) fn density(
    backend: &WgpuBackend,
    width: u32,
    height: u32,
    axis: &[f64],
    curves: &[[f64; 3]],
    gamma: f64,
) -> (wgpu::Buffer, wgpu::Buffer, wgpu::Buffer) {
    let axis_f32: Vec<f32> = axis.iter().map(|&v| v as f32).collect();
    let curves_f32 = nan_safe_rows_to_f32(curves);
    let params = DensityParams {
        width,
        height,
        k: axis.len() as u32,
        uniform_grid: u32::from(is_uniform_grid_endpoint(axis)),
        gamma_inv: [(1.0 / gamma) as f32; 3],
        _pad: 0.0,
    };
    (
        uniform(backend, "density_params", bytemuck::bytes_of(&params)),
        storage(backend, "density_axis", bytemuck::cast_slice(&axis_f32)),
        storage(backend, "density_curves", bytemuck::cast_slice(&curves_f32)),
    )
}

pub(super) fn print(
    backend: &WgpuBackend,
    p: &spektrafilm_backend::FilmChainParams<'_>,
    width: u32,
    height: u32,
) -> [wgpu::Buffer; 5] {
    let (cd, mut bd) = sanitize_spectral_inputs(
        p.film_channel_density,
        p.film_base_density,
        p.film_channel_density.len(),
    );
    bd.resize(p.film_channel_density.len(), 0.0);
    let illuminant: Vec<f32> = p.print_illuminant.iter().map(|&v| v as f32).collect();
    let sensitivity = nan_safe_rows_to_f32(p.print_sensitivity);
    let params = [
        width,
        height,
        p.film_channel_density.len() as u32,
        (p.print_normalization_factor as f32).to_bits(),
        (p.print_exposure_scale as f32).to_bits(),
        u32::from(p.enlarger_diffusion.is_some()),
        0,
        0,
        (p.preflash[0] as f32).to_bits(),
        (p.preflash[1] as f32).to_bits(),
        (p.preflash[2] as f32).to_bits(),
        0,
    ];
    [
        uniform(backend, "dye_print_params", bytemuck::cast_slice(&params)),
        storage(backend, "film_cd", bytemuck::cast_slice(&cd)),
        storage(backend, "film_bd", bytemuck::cast_slice(&bd)),
        storage(backend, "print_illu", bytemuck::cast_slice(&illuminant)),
        storage(backend, "print_sens", bytemuck::cast_slice(&sensitivity)),
    ]
}
