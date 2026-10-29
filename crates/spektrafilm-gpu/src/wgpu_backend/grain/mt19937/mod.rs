//! Bounded resident NumPy MT19937 particle sampling and microstructure.
//!
//! Each active plane draws its complete Poisson field before any binomial
//! draw. Persistent device state crosses all transport/submission boundaries.
//! The final plane continues into row-major HWC NumPy normals; fast-statistics
//! requests instead seed that normal stream afresh, as upstream does.

use super::super::*;
use super::raster::{RasterCommands, RasterStorage, paged_transport, transport, uniform};
use spektrafilm_backend::SpektraGrainGpuParams;

pub(in crate::wgpu_backend) const CHUNK_ELEMENTS: u32 = 65_536;
const MT_STATE_BYTES: u64 = 2520;

pub(in crate::wgpu_backend) struct SpektraMt19937State {
    steps: Vec<RasterCommands>,
}

impl SpektraMt19937State {
    pub(in crate::wgpu_backend) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        for step in &self.steps {
            step.submit(backend)?;
        }
        backend.check_cancelled()
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(in crate::wgpu_backend) fn build_spektra_mt19937_state(
    device: &ObservedDevice,
    width: u32,
    height: u32,
    density: &(impl RasterStorage + ?Sized),
    layer_fields: &(impl RasterStorage + ?Sized),
    clump: &(impl RasterStorage + ?Sized),
    params: &SpektraGrainGpuParams,
    backend: &WgpuBackend,
) -> Result<SpektraMt19937State, String> {
    build_spektra_mt19937_state_with_limit(
        device,
        width,
        height,
        density,
        layer_fields,
        clump,
        params,
        backend,
        super::super::bounded::storage_binding_limit(backend)?,
    )
}

fn validate_inputs(
    width: u32,
    height: u32,
    params: &SpektraGrainGpuParams,
) -> Result<(u32, usize), String> {
    let n_pixels = width
        .checked_mul(height)
        .filter(|&n| n > 0 && n <= u32::MAX / 3)
        .ok_or("Spektra grain pixel count overflows or is zero")?;
    let knots = params.curve_x.len() / 3;
    if knots < 2 || params.curve_layers.len() != knots * 9 {
        return Err("Spektra grain requires matching composite and sublayer curve tables".into());
    }
    if !(1..=3).contains(&params.layer_count) {
        return Err("Spektra grain layer_count must be in 1..=3".into());
    }
    Ok((n_pixels, knots))
}

#[allow(clippy::too_many_arguments)]
pub(in crate::wgpu_backend) fn build_spektra_mt19937_state_with_limit(
    device: &ObservedDevice,
    width: u32,
    height: u32,
    density: &(impl RasterStorage + ?Sized),
    layer_fields: &(impl RasterStorage + ?Sized),
    clump: &(impl RasterStorage + ?Sized),
    params: &SpektraGrainGpuParams,
    backend: &WgpuBackend,
    limit: u64,
) -> Result<SpektraMt19937State, String> {
    if !device
        .features()
        .contains(wgpu::Features::SHADER_F64 | wgpu::Features::SHADER_INT64)
    {
        return Err(
            "Spektra grain MT19937 sampling needs native f64 and 64-bit integer shader support"
                .into(),
        );
    }
    let (n_pixels, knots) = validate_inputs(width, height, params)?;
    if limit < MT_STATE_BYTES {
        return Err("Spektra MT19937 minimum generator binding needs 2520 bytes".into());
    }
    let channels = if params.monochrome { 1 } else { 3 };
    let planes = params.layer_count * channels;
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
        n_pixels: u32,
        chunk_elements: u32,
        pad_flags: [u32; 2],
        // x: 0 particle, 1 continued normals, 2 fresh-seed normals;
        // y: monochrome; z: host representation; w: layer*3+channel.
        flags: [u32; 4],
        base_seed: u64,
        micro_sigma: f64,
        lookup_sign: f32,
        pad0: u32,
        pad_vec: [u32; 6],
        uniformity: [f64; 4],
        n_particles_layers: [[f64; 4]; 3],
        density_max_adj: [[f64; 4]; 3],
        density_min_layers: [[f64; 4]; 3],
    }
    let pipe = backend.cached_pipeline(
        concat!(
            include_str!("../../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
            include_str!("../work_f32.wgsl"),
            include_str!("../../../../../spektrafilm-shaders/wgsl/mt19937.wgsl"),
            include_str!("mt19937.wgsl")
        ),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: false },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let chunk = (limit / 24)
        .min(u64::from(CHUNK_ELEMENTS))
        .min(u64::from(n_pixels)) as u32;
    let micro_active = params.micro_sigma > 0.05;
    let sampling_chunk = if params.mt19937_regime { chunk } else { 1 };
    let clump_chunk = if micro_active { chunk } else { 1 };
    let density_window = transport(backend, u64::from(sampling_chunk) * 12, "mt_density_window")?;
    let field_window = transport(backend, u64::from(sampling_chunk) * 12, "mt_field_window")?;
    let seed_window = transport(backend, u64::from(sampling_chunk) * 8, "mt_seed_window")?;
    let clump_window = transport(backend, u64::from(clump_chunk) * 24, "mt_clump_window")?;
    let segments = n_pixels.div_ceil(chunk);
    let particle_steps = if params.mt19937_regime {
        segments * 2
    } else {
        0
    };
    let mut steps: Vec<RasterCommands> = (0..particle_steps
        + if micro_active { segments } else { 0 })
        .map(|_| RasterCommands::default())
        .collect();
    let make_params = |plane, mode| Params {
        width,
        height,
        knots: knots as u32,
        layers: params.layer_count,
        n_pixels,
        chunk_elements: chunk,
        pad_flags: [0; 2],
        flags: [
            mode,
            u32::from(params.monochrome),
            u32::from(params.reference_f64),
            plane,
        ],
        base_seed: params.base_seed,
        micro_sigma: params.micro_sigma,
        lookup_sign: params.lookup_sign,
        pad0: 0,
        pad_vec: [0; 6],
        uniformity: pad3(params.uniformity),
        n_particles_layers: pad9(params.n_particles),
        density_max_adj: pad9(params.density_max_adj),
        density_min_layers: pad9(params.density_min_layers),
    };
    let curve_x = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("spektra_mt19937_curve_x"),
        contents: bytemuck::cast_slice(&params.curve_x),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let curve_layers = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("spektra_mt19937_curve_layers"),
        contents: bytemuck::cast_slice(&params.curve_layers),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let final_state = transport(backend, MT_STATE_BYTES, "mt_final_generator")?;
    if params.mt19937_regime {
        for plane in 0..planes {
            let sl = plane / channels;
            let ch = plane % channels;
            let seeds = paged_transport(backend, u64::from(n_pixels) * 8, "mt_poisson_transport")?;
            let own_state = if plane + 1 == planes {
                None
            } else {
                Some(transport(
                    backend,
                    MT_STATE_BYTES,
                    "mt_persistent_generator",
                )?)
            };
            let state = own_state.as_ref().unwrap_or(&final_state);
            let params_buf = uniform(backend, &make_params(sl * 3 + ch, 0));
            for phase in 0..2 {
                for segment in 0..segments {
                    let first = segment * chunk;
                    let count = chunk.min(n_pixels - first);
                    let commands = &mut steps[(phase * segments + segment) as usize];
                    commands.copy(
                        density,
                        u64::from(first) * 12,
                        &density_window,
                        0,
                        u64::from(count) * 12,
                    );
                    if phase != 0 {
                        commands.copy(
                            layer_fields,
                            (u64::from(sl) * u64::from(n_pixels) + u64::from(first)) * 12,
                            &field_window,
                            0,
                            u64::from(count) * 12,
                        );
                        commands.copy(
                            &seeds,
                            u64::from(first) * 8,
                            &seed_window,
                            0,
                            u64::from(count) * 8,
                        );
                    }
                    commands.dispatch_named(
                        &pipe,
                        &[
                            &params_buf,
                            &density_window,
                            &curve_x,
                            &curve_layers,
                            &field_window,
                            &seed_window,
                            state,
                            &clump_window,
                        ],
                        1,
                        true,
                        backend,
                        Some(if phase == 0 {
                            "spektra_seed_formation"
                        } else {
                            "spektra_particle_formation"
                        }),
                    );
                    if phase == 0 {
                        commands.copy(
                            &seed_window,
                            0,
                            &seeds,
                            u64::from(first) * 8,
                            u64::from(count) * 8,
                        );
                    } else {
                        commands.copy(
                            &field_window,
                            0,
                            layer_fields,
                            (u64::from(sl) * u64::from(n_pixels) + u64::from(first)) * 12,
                            u64::from(count) * 12,
                        );
                    }
                }
            }
        }
    }
    if micro_active {
        let last_plane = (params.layer_count - 1) * 3 + channels - 1;
        let mode = if params.mt19937_regime { 1 } else { 2 };
        let params_buf = uniform(backend, &make_params(last_plane, mode));
        for segment in 0..segments {
            let first = segment * chunk;
            let count = chunk.min(n_pixels - first);
            let commands = &mut steps[(particle_steps + segment) as usize];
            commands.dispatch_named(
                &pipe,
                &[
                    &params_buf,
                    &density_window,
                    &curve_x,
                    &curve_layers,
                    &field_window,
                    &seed_window,
                    &final_state,
                    &clump_window,
                ],
                1,
                true,
                backend,
                Some("spektra_clump_formation"),
            );
            commands.copy(
                &clump_window,
                0,
                clump,
                u64::from(first) * 24,
                u64::from(count) * 24,
            );
        }
    }
    Ok(SpektraMt19937State { steps })
}

#[cfg(test)]
mod tests;
