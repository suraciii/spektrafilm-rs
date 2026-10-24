//! Static GPU buffer and uniform setup for the resident film chain.
//!
//! Split out of `run_film_chain_inner`: pure code motion. Every binding the
//! former inline block read from the enclosing scope becomes an explicit
//! parameter; every buffer it produced is returned in [`ChainBuffers`].
use super::super::is_uniform_grid_endpoint;
use super::super::pack_matrix_columns_f64;
use super::super::{
    DensityParams, FrontParams, nan_safe_rows_to_f32, sanitize_spectral_inputs,
};

/// Every static buffer and uniform the chain uploads once per call.
pub(super) struct ChainBuffers {
    pub(super) film_log_exp_buf: wgpu::Buffer,
    pub(super) film_curves_buf: wgpu::Buffer,
    pub(super) film_cd_buf: wgpu::Buffer,
    pub(super) film_bd_buf: wgpu::Buffer,
    pub(super) print_illu_buf: wgpu::Buffer,
    pub(super) print_sens_buf: wgpu::Buffer,
    pub(super) print_log_exp_buf: wgpu::Buffer,
    pub(super) print_curves_buf: wgpu::Buffer,
    pub(super) front_params_buf: wgpu::Buffer,
    pub(super) film_density_params_buf: wgpu::Buffer,
    pub(super) print_density_params_buf: wgpu::Buffer,
}

/// Upload every static (per-call, non-ping-pong) buffer and uniform.
#[allow(clippy::too_many_arguments)]
pub(super) fn upload(
    image_width: u32,
    image_height: u32,
    film_log_exposure: &[f64],
    film_density_curves_normalized: &[[f64; 3]],
    film_gamma: f64,
    film_channel_density: &[[f64; 3]],
    film_base_density: &[f64],
    print_illuminant: &[f64],
    print_sensitivity: &[[f64; 3]],
    print_log_exposure: &[f64],
    print_density_curves: &[[f64; 3]],
    print_gamma: f64,
    params: &spektrafilm_backend::FilmChainParams<'_>,
    mk_storage: &dyn Fn(&str, &[u8]) -> wgpu::Buffer,
    mk_uniform: &dyn Fn(&str, &[u8]) -> wgpu::Buffer,
) -> ChainBuffers {
    // ── Pre-compute every static GPU-side buffer ─────────────────────
    // Filming density curves — already normalized by caller.
    let film_log_exp_f32: Vec<f32> = film_log_exposure.iter().map(|&v| v as f32).collect();
    let film_curves_f32: Vec<f32> = nan_safe_rows_to_f32(film_density_curves_normalized);
    let film_log_exp_buf = mk_storage("film_log_exp", bytemuck::cast_slice(&film_log_exp_f32));
    let film_curves_buf = mk_storage("film_curves", bytemuck::cast_slice(&film_curves_f32));

    // Film spectral data (for printing pass).
    let (film_cd_f32, mut film_bd_f32) = sanitize_spectral_inputs(
        film_channel_density,
        film_base_density,
        film_channel_density.len(),
    );
    film_bd_f32.resize(film_channel_density.len(), 0.0);
    let film_cd_buf = mk_storage("film_cd", bytemuck::cast_slice(&film_cd_f32));
    let film_bd_buf = mk_storage("film_bd", bytemuck::cast_slice(&film_bd_f32));

    let print_illu_f32: Vec<f32> = print_illuminant.iter().map(|&v| v as f32).collect();
    let print_sens_f32: Vec<f32> = nan_safe_rows_to_f32(print_sensitivity);
    let print_illu_buf = mk_storage("print_illu", bytemuck::cast_slice(&print_illu_f32));
    let print_sens_buf = mk_storage("print_sens", bytemuck::cast_slice(&print_sens_f32));

    // Print density curves (RAW, no normalization for print path).
    let print_log_exp_f32: Vec<f32> = print_log_exposure.iter().map(|&v| v as f32).collect();
    let print_curves_f32: Vec<f32> = nan_safe_rows_to_f32(print_density_curves);
    let print_log_exp_buf = mk_storage("print_log_exp", bytemuck::cast_slice(&print_log_exp_f32));
    let print_curves_buf = mk_storage("print_curves", bytemuck::cast_slice(&print_curves_f32));


    // ── Param structs ────────────────────────────────────────────────
    // Front pass (hanatos LUT lookup or mallett matmul). Both shaders
    // share this uniform layout; `lut_size` is hanatos-only (0 for
    // mallett, where the field is a pad).
    let (m, front_lut_size) = match &params.front {
        spektrafilm_backend::FrontPass::Hanatos2025 {
            tc_lut,
            rgb_to_adapted_xyz,
            ..
        } => (rgb_to_adapted_xyz, tc_lut.size as u32),
        spektrafilm_backend::FrontPass::Mallett2019 { matrix } => (matrix, 0u32),
        spektrafilm_backend::FrontPass::ScanInverse(_) => (&[[0.0; 3]; 3], 0u32),
    };
    let columns = pack_matrix_columns_f64(m);
    let front_params = FrontParams {
        width: params.image_width,
        height: params.image_height,
        lut_size: front_lut_size,
        _pad: 0,
        col0: columns[0],
        col1: columns[1],
        col2: columns[2],
    };
    let front_params_buf = mk_uniform("front_params", bytemuck::bytes_of(&front_params));

    let film_density_params = DensityParams {
        width: params.image_width,
        height: params.image_height,
        k: film_log_exposure.len() as u32,
        uniform_grid: if is_uniform_grid_endpoint(film_log_exposure) {
            1
        } else {
            0
        },
        gamma_inv: [(1.0 / film_gamma) as f32; 3],
        _pad: 0.0,
    };
    let film_density_params_buf =
        mk_uniform("film_dens_params", bytemuck::bytes_of(&film_density_params));


    let print_density_params = DensityParams {
        width: image_width,
        height: image_height,
        k: print_log_exposure.len() as u32,
        uniform_grid: if is_uniform_grid_endpoint(print_log_exposure) {
            1
        } else {
            0
        },
        gamma_inv: [(1.0 / print_gamma) as f32; 3],
        _pad: 0.0,
    };
    let print_density_params_buf = mk_uniform(
        "print_dens_params",
        bytemuck::bytes_of(&print_density_params),
    );

    ChainBuffers {
        film_log_exp_buf,
        film_curves_buf,
        film_cd_buf,
        film_bd_buf,
        print_illu_buf,
        print_sens_buf,
        print_log_exp_buf,
        print_curves_buf,
        front_params_buf,
        film_density_params_buf,
        print_density_params_buf,
    }
}
