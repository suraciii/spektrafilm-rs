//! Resident Spektra grain formation and density finishing.
//!
//! Profile coefficients and curve tables are resolved without image sampling.
//! Device stages interpolate layers, sample counts, filter particle and clump
//! fields, compose density, and apply final blur and mass-conserving USM.
//! Count conditioning is f32; discrete samplers require binary64 and 64-bit words.
//! Unsupported arithmetic and unrepresentable working sets return errors.

pub(in crate::wgpu_backend) mod blur;
mod post;

use super::super::*;
use super::mt19937::{SpektraMt19937State, build_spektra_mt19937_state_with_limit};
use super::raster::{RasterCommands, RasterStorage, paged_transport, transport};
use blur::{GrainBlurState, build_grain_blur_f64_paged, build_grain_blur_paged};
use post::{GrainPostState, build_grain_post};
use spektrafilm_backend::SpektraGrainGpuParams;

/// Pass name in the resident chain's stage list (the density-fence slot).
pub(in crate::wgpu_backend) const GRAIN_SPEKTRA_STAGE_NAME: &str = "grain_spektra";

/// Bound transport scratch independently of image paging and coefficient
/// bindings. This changes only execution partitions, never global RNG indices.
pub(in crate::wgpu_backend) fn transport_window_limit(binding_limit: u64) -> u64 {
    binding_limit.min(super::super::resources::WORKSPACE_BINDING_BYTES)
}

/// Resident grain formation, layer optics, clumping, and density finishing.
pub(in crate::wgpu_backend) struct SpektraGrainState {
    dispatch: RasterCommands,
    layer_blurs: Vec<GrainBlurState>,
    clump_blur: Option<GrainBlurState>,
    compose: RasterCommands,
    post: GrainPostState,
    mt19937: Option<SpektraMt19937State>,
}

impl SpektraGrainState {
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
    params: &SpektraGrainGpuParams,
    backend: &WgpuBackend,
) -> Result<SpektraGrainState, String> {
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
    params: &SpektraGrainGpuParams,
    backend: &WgpuBackend,
    limit: u64,
) -> Result<SpektraGrainState, String> {
    let window_limit = transport_window_limit(limit);
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
    if !(1..=3).contains(&params.layer_count) {
        return Err("Spektra grain layer_count must be in 1..=3".into());
    }
    let n_layers = params.layer_count;
    let layer_bytes = image_bytes
        .checked_mul(u64::from(n_layers))
        .ok_or("Spektra grain layer size overflows")?;
    let n_pixels = width
        .checked_mul(height)
        .filter(|&n| n > 0 && n <= u32::MAX / 3)
        .ok_or("Spektra grain indexing overflows")?;
    let chunk = (window_limit / (12 * u64::from(n_layers).max(2))).min(u64::from(n_pixels)) as u32;
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
        layers: u32,
        base_seed: u64,
        pad_seed: u64,
        uniformity: [f64; 4],
        density_min_layers: [[f64; 4]; 3],
        density_max_adj: [[f64; 4]; 3],
        n_particles: [[f64; 4]; 3],
        lookup_sign: f32,
        pad_flags: [u32; 3],
        flags: [u32; 4],
        tail_padding: [u32; 4],
        pad_struct: [u32; 4],
    }
    let pipe = backend.cached_pipeline(
        concat!(
            include_str!("../../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
            include_str!("../work_f32.wgsl"),
            include_str!("spektra.wgsl")
        ),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
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
                layers: n_layers,
                base_seed: params.base_seed,
                pad_seed: 0,
                uniformity: pad3(params.uniformity),
                density_min_layers: pad9(params.density_min_layers),
                density_max_adj: pad9(params.density_max_adj),
                n_particles: pad9(params.n_particles),
                lookup_sign: params.lookup_sign,
                pad_flags: [0; 3],
                flags: [
                    params.reference_f64 as u32,
                    params.monochrome as u32,
                    params.mt19937_regime as u32,
                    0,
                ],
                tail_padding: [first, count, 0, 0],
                pad_struct: [0; 4],
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
    let micro_active = params.micro_sigma > 0.05;
    let clump = paged_transport(
        backend,
        if micro_active { image_bytes * 2 } else { 24 },
        "grain_clumping_field",
    )?;
    let density_window = transport(backend, u64::from(chunk) * 12, "grain_density_window")?;
    let layers_window = transport(
        backend,
        u64::from(chunk) * 12 * u64::from(n_layers),
        "grain_layers_window",
    )?;
    let clump_window = transport(
        backend,
        if micro_active {
            u64::from(chunk) * 24
        } else {
            24
        },
        "grain_clump_window",
    )?;
    let mut dispatch = RasterCommands::default();
    for first in (0..n_pixels).step_by(chunk as usize) {
        if params.mt19937_regime {
            break;
        }
        let count = chunk.min(n_pixels - first);
        let params_buf = make_params(first, count);
        dispatch.copy(
            density,
            u64::from(first) * 12,
            &density_window,
            0,
            u64::from(count) * 12,
        );
        dispatch.dispatch_named(
            &pipe,
            &[
                &params_buf,
                &density_window,
                &curve_x,
                &curve_layers,
                &layers_window,
            ],
            count,
            false,
            backend,
            Some("spektra_particle_formation"),
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
    }
    let filtered_layers = (params.blur_dye_clouds > 0.0)
        .then(|| paged_transport(backend, layer_bytes, "grain_filtered_particle_fields"))
        .transpose()?;
    let mut layer_blurs = Vec::new();
    if let Some(filtered) = &filtered_layers {
        let layer_stride = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(3))
            .ok_or("Spektra grain layer indexing overflows")?;
        for sl in 0..n_layers as usize {
            let sigma = std::array::from_fn(|ch| {
                let ch = if params.monochrome { 0 } else { ch };
                params.blur_dye_clouds
                    * (params.density_max_adj[sl * 3 + ch] / params.n_particles[sl * 3 + ch]).sqrt()
            });
            let offset = sl as u64 * u64::from(layer_stride);
            layer_blurs.push(build_grain_blur_paged(
                backend, width, height, &layers, filtered, sigma, offset, offset,
            )?);
        }
    }
    let filtered_clump = (params.micro_sigma > 0.05 && params.micro_blur_px > 0.4)
        .then(|| paged_transport(backend, image_bytes * 2, "grain_filtered_clumping"))
        .transpose()?;
    let clump_blur = filtered_clump
        .as_ref()
        .map(|filtered| {
            build_grain_blur_f64_paged(
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
                dimensions: [count, 1, n_layers, (params.micro_sigma > 0.05) as u32],
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
        if micro_active {
            compose.copy(
                filtered_clump.as_ref().unwrap_or(&clump),
                u64::from(first) * 24,
                &clump_window,
                0,
                u64::from(count) * 24,
            );
        }
        let params_buf = compose_params(count);
        compose.dispatch_named(
            &compose_pipe,
            &[&params_buf, &layers_window, &clump_window, &density_window],
            count,
            false,
            backend,
            Some("spektra_compose"),
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
    let mt19937 = if params.mt19937_regime || params.micro_sigma > 0.05 {
        Some(build_spektra_mt19937_state_with_limit(
            device,
            width,
            height,
            density,
            &layers,
            &clump,
            params,
            backend,
            window_limit,
        )?)
    } else {
        None
    };
    Ok(SpektraGrainState {
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
