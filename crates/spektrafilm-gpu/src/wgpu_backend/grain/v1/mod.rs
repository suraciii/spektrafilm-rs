//! Resident Spektra grain formation and density finishing.
//!
//! Profile coefficients and curve tables are resolved without image sampling.
//! Device stages interpolate layers, sample counts, filter particle and clump
//! fields, compose density, and apply final blur and mass-conserving USM.
//! Count decisions require binary64 conditioning and 64-bit random words.
//! Unsupported arithmetic and unrepresentable working sets return errors.

pub(in crate::wgpu_backend) mod blur;
mod post;

use super::super::*;
use super::mt19937::{SpektraMt19937State, build_spektra_mt19937_state};
use super::raster::{RasterCommands, RasterStorage, paged_transport, transport};
use blur::{GrainBlurState, build_grain_blur_paged};
use post::{GrainPostState, build_grain_post};
use spektrafilm_backend::GrainV1GpuParams;

/// Pass name in the resident chain's stage list (the density-fence slot).
pub(in crate::wgpu_backend) const GRAIN_V1_STAGE_NAME: &str = "grain_spektra_v1";

/// Resident grain formation, layer optics, clumping, and density finishing.
pub(in crate::wgpu_backend) struct SpektraGrainV1State {
    dispatch: RasterCommands,
    layer_blurs: Vec<GrainBlurState>,
    clump_blur: Option<GrainBlurState>,
    compose: RasterCommands,
    post: GrainPostState,
    mt19937: Option<SpektraMt19937State>,
}

impl SpektraGrainV1State {
    pub(in crate::wgpu_backend) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        self.dispatch.submit(backend)?;
        if let Some(mt) = &self.mt19937 {
            mt.submit(backend)?;
        }
        for blur in &self.layer_blurs {
            blur.submit(backend)?;
        }
        if let Some(blur) = &self.clump_blur {
            blur.submit(backend)?;
        }
        self.compose.submit(backend)?;
        self.post.submit(backend)
    }
}

/// Build the complete resident grain stage or report its device limitation.
pub(in crate::wgpu_backend) fn build_spektra_grain_state(
    device: &ObservedDevice,
    width: u32,
    height: u32,
    density: &(impl RasterStorage + ?Sized),
    params: &GrainV1GpuParams,
    backend: &WgpuBackend,
) -> Result<SpektraGrainV1State, String> {
    build_spektra_grain_state_with_limit(
        device,
        width,
        height,
        density,
        params,
        backend,
        super::super::bounded::storage_binding_limit(backend)?,
    )
}

fn build_spektra_grain_state_with_limit(
    device: &ObservedDevice,
    width: u32,
    height: u32,
    density: &(impl RasterStorage + ?Sized),
    params: &GrainV1GpuParams,
    backend: &WgpuBackend,
    limit: u64,
) -> Result<SpektraGrainV1State, String> {
    if !device
        .features()
        .contains(wgpu::Features::SHADER_F64 | wgpu::Features::SHADER_INT64)
    {
        return Err("Spektra grain requires device binary64 arithmetic and 64-bit integers".into());
    }
    // The curve tables are the request's own slices, so an empty table is a
    // request the host would have asserted on.
    let knots = params.curve_x.len() / 3;
    if knots < 2 || params.curve_layers.len() != knots * 9 {
        return Err("Spektra grain requires matching composite and sublayer curve tables".into());
    }
    let image_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(12))
        .ok_or("Spektra grain image byte size overflows")?;
    let n_layers = if params.sublayers_active {
        3
    } else {
        params.n_sub_layers
    };
    let layer_bytes = image_bytes
        .checked_mul(u64::from(n_layers))
        .ok_or("Spektra grain layer size overflows")?;
    let n_pixels = width
        .checked_mul(height)
        .filter(|&n| n > 0 && n <= u32::MAX / 3)
        .ok_or("Spektra grain indexing overflows")?;
    let chunk = (limit / (12 * u64::from(n_layers.max(1)))).min(u64::from(n_pixels)) as u32;
    if chunk == 0 {
        return Err("Spektra grain minimum layer window exceeds storage binding limit".into());
    }
    // The WGSL uniform rows are 4 lanes wide, so each 3-value triple and each
    // 9-value table is padded to that stride; the shader reads lanes 0..2.
    fn pad3(v: [f64; 3]) -> [f64; 4] {
        [v[0], v[1], v[2], 0.0]
    }
    fn pad9(v: [f64; 9]) -> [[f64; 4]; 3] {
        std::array::from_fn(|row| {
            let base = row * 3;
            [v[base], v[base + 1], v[base + 2], 0.0]
        })
    }
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct Params {
        width: u32,
        height: u32,
        knots: u32,
        pad0: u32,
        base_seed: u64,
        pixel_size_um: f64,
        // Binary64 vec3 alignment is 32 bytes; rows include a padding lane.
        particle_area: [f64; 4],
        particle_scale_layers: [f64; 4],
        density_min: [f64; 4],
        uniformity: [f64; 4],
        density_min_layers: [[f64; 4]; 3],
        density_max_adj: [[f64; 4]; 3],
        n_particles: [[f64; 4]; 3],
        micro_sigma: f64,
        micro_blur_px: f64,
        grain_blur: f32,
        usm_sigma: f32,
        usm_amount: f32,
        lookup_sign: f32,
        flags: [u32; 4],
        tail_padding: [u32; 4],
    }
    let pipe = backend.cached_pipeline(
        concat!(
            include_str!("../../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
            include_str!("grain_v1.wgsl")
        ),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let make_params = |first, count| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("spektra_grain_params"),
            contents: bytemuck::bytes_of(&Params {
                width,
                height,
                knots: knots as u32,
                pad0: 0,
                base_seed: params.base_seed,
                pixel_size_um: params.pixel_size_um,
                particle_area: pad3(params.particle_area),
                particle_scale_layers: pad3(params.particle_scale_layers),
                density_min: pad3(params.density_min),
                uniformity: pad3(params.uniformity),
                density_min_layers: pad9(params.density_min_layers),
                density_max_adj: pad9(params.density_max_adj),
                n_particles: pad9(params.n_particles),
                micro_sigma: params.micro_sigma,
                micro_blur_px: params.micro_blur_px,
                grain_blur: params.grain_blur,
                usm_sigma: params.usm_sigma,
                usm_amount: params.usm_amount,
                lookup_sign: params.lookup_sign,
                flags: [
                    params.reference_f64 as u32,
                    params.monochrome as u32,
                    params.mt19937_regime as u32,
                    params.sublayers_active as u32,
                ],
                tail_padding: [first, count, 0, 0],
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        })
    };
    let curve_x = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("spektra_grain_curve_x"),
        contents: bytemuck::cast_slice(&params.curve_x),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let curve_layers = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("spektra_grain_curve_layers"),
        contents: bytemuck::cast_slice(&params.curve_layers),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let layers = paged_transport(backend, layer_bytes, "grain_particle_fields")?;
    let clump = paged_transport(backend, image_bytes, "grain_clumping_field")?;
    let density_window = transport(backend, u64::from(chunk) * 12, "grain_density_window")?;
    let layers_window = transport(
        backend,
        u64::from(chunk) * 12 * u64::from(n_layers),
        "grain_layers_window",
    )?;
    let clump_window = transport(backend, u64::from(chunk) * 12, "grain_clump_window")?;
    let mut dispatch = RasterCommands::default();
    for first in (0..n_pixels).step_by(chunk as usize) {
        let count = chunk.min(n_pixels - first);
        let params_buf = make_params(first, count);
        dispatch.copy(
            density,
            u64::from(first) * 12,
            &density_window,
            0,
            u64::from(count) * 12,
        );
        dispatch.dispatch(
            &pipe,
            &[
                &params_buf,
                &density_window,
                &curve_x,
                &curve_layers,
                &layers_window,
                &clump_window,
            ],
            count,
            false,
            backend,
        );
        if !params.mt19937_regime {
            for sl in 0..n_layers {
                dispatch.copy(
                    &layers_window,
                    u64::from(sl * count) * 12,
                    &layers,
                    (u64::from(sl) * u64::from(n_pixels) + u64::from(first)) * 12,
                    u64::from(count) * 12,
                );
            }
        }
        dispatch.copy(
            &clump_window,
            0,
            &clump,
            u64::from(first) * 12,
            u64::from(count) * 12,
        );
    }
    let filtered_layers = (params.sublayers_active && params.blur_dye_clouds > 0.0)
        .then(|| paged_transport(backend, layer_bytes, "grain_filtered_particle_fields"))
        .transpose()?;
    let mut layer_blurs = Vec::new();
    if let Some(filtered) = &filtered_layers {
        let layer_stride = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(3))
            .ok_or("Spektra grain layer indexing overflows")?;
        for sl in 0..3 {
            let sigma = std::array::from_fn(|ch| {
                f64::from(params.blur_dye_clouds)
                    * (params.density_max_adj[sl * 3 + ch] / params.n_particles[sl * 3 + ch]).sqrt()
            });
            let offset = sl as u64 * u64::from(layer_stride);
            layer_blurs.push(build_grain_blur_paged(
                backend, width, height, &layers, filtered, sigma, offset, offset,
            )?);
        }
    }
    let filtered_clump =
        (params.sublayers_active && params.micro_sigma > 0.05 && params.micro_blur_px > 0.4)
            .then(|| paged_transport(backend, image_bytes, "grain_filtered_clumping"))
            .transpose()?;
    let clump_blur = filtered_clump
        .as_ref()
        .map(|filtered| {
            build_grain_blur_paged(
                backend,
                width,
                height,
                &clump,
                filtered,
                [params.micro_blur_px; 3],
                0,
                0,
            )
        })
        .transpose()?;
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct ComposeParams {
        dimensions: [u32; 4],
        scale: f32,
        pad: [u32; 3],
    }
    let compose_pipe = backend.cached_pipeline(
        include_str!("compose.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let compose_params = |count| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grain_compose_params"),
            contents: bytemuck::bytes_of(&ComposeParams {
                dimensions: [
                    count,
                    1,
                    n_layers,
                    (params.sublayers_active && params.micro_sigma > 0.05) as u32,
                ],
                scale: if params.sublayers_active {
                    1.0
                } else {
                    (1.0 / f64::from(n_layers)) as f32
                },
                pad: [0; 3],
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        })
    };
    let mut compose = RasterCommands::default();
    for first in (0..n_pixels).step_by(chunk as usize) {
        let count = chunk.min(n_pixels - first);
        for sl in 0..n_layers {
            compose.copy(
                filtered_layers.as_ref().unwrap_or(&layers),
                (u64::from(sl) * u64::from(n_pixels) + u64::from(first)) * 12,
                &layers_window,
                u64::from(sl * count) * 12,
                u64::from(count) * 12,
            );
        }
        compose.copy(
            filtered_clump.as_ref().unwrap_or(&clump),
            u64::from(first) * 12,
            &clump_window,
            0,
            u64::from(count) * 12,
        );
        let params_buf = compose_params(count);
        compose.dispatch(
            &compose_pipe,
            &[&params_buf, &layers_window, &clump_window, &density_window],
            count,
            false,
            backend,
        );
        compose.copy(
            &density_window,
            0,
            density,
            u64::from(first) * 12,
            u64::from(count) * 12,
        );
    }
    let post = build_grain_post(backend, width, height, density, params)?;
    let mt19937 = if params.mt19937_regime {
        Some(build_spektra_mt19937_state(
            device, width, height, density, &layers, params, backend,
        )?)
    } else {
        None
    };
    Ok(SpektraGrainV1State {
        dispatch,
        layer_blurs,
        clump_blur,
        compose,
        post,
        mt19937,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod math_tests;
