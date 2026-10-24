// Device Spektra v1 sequential MT19937 regime.
//!
//! This module owns the device realization of the scipy/NumPy sampling
//! regime of `spektrafilm_model::grain::v1::layer_particle_model`
//! (`use_fast_stats = false`), which is what the composite path
//! (`apply_grain_to_density`) always uses and the layered path
//! (`apply_grain_to_density_layers`) uses whenever the request sets
//! `use_fast_stats = false`. The `fast_stats` regime lives in the sibling
//! [`super::v1`] module and keeps its own shader; the two never share a
//! sampler.
//!
//! ## Why the reference is sequential
//!
//! The reference advances one `rand_mt::Mt` across a whole `(channel,
//! sublayer)` plane in row-major order: every Poisson draw first, then
//! every binomial draw. Both laws reject and consume a variable number of
//! 32-bit words, so the state element `k` observes depends on every earlier
//! element's consumption. `specs/gpu-execution/design.md` §Stochastic
//! alignment forbids reseeding or reordering, so no per-pixel dispatch can
//! reproduce it and no jump-ahead can precompute an element's state:
//! element `k`'s next word is not at a fixed offset in the output stream.
//!
//! ## Device execution
//!
//! One invocation owns one plane's MT19937 slot. Each public encode step
//! gathers and dispatches a bounded segment for every plane, then scatters
//! its seeds or fields back to copy-only transport. The first complete-source
//! walk draws Poisson seeds; only after every Poisson segment does the second
//! walk draw binomial counts. Persistent per-plane slots retain rejection
//! consumption across storage windows and submission boundaries.
//!
//! ## Precision
//!
//! The reference conditions both count laws on f64 values it reads at full
//! precision, so the sampler runs the whole kernel in f64 and needs
//! `SHADER_F64`. It uses the shared explicit `f64_math.wgsl` helpers
//! rather than driver log/exp/pow builtins, which return wrong values on
//! some devices. Every discrete decision is made in f64; only the finished
//! particle field is narrowed to f32, matching the sibling sampler's
//! representation.
//!
//! The reference stores its intermediate density samples in `Scalar`, which
//! is f32 unless the pipeline runs in full f64 precision. The knot values,
//! the reciprocal, the interpolation, the sublayer split, and the floor add
//! all cross that boundary in the reference, so the device narrows exactly
//! those values with `sample_round` unless `reference_f64` is set. A
//! narrowed knot or reciprocal changes the bracketing decision, so this is
//! not a cosmetic difference.
//!
//! `lgamma` inside the PTRS acceptance test is the one transcendental the
//! reference takes from libm; the device uses a Lanczos approximation of
//! it, so a PTRS acceptance decision that lands within that approximation's
//! error of its boundary can disagree with the reference. This is a stated
//! approximation, not a verified equivalence; the owning stage's budget
//! must account for it.
//!
//! ## Output
//!
//! The pass writes the per-sublayer particle fields as f32 into
//! `[sl][pixel][ch]` interleaved layout: sublayer-major, then pixel, then
//! channel. The composite and layered paths share this layout, with the
//! plane count equal to `3 * n_sub_layers` on the composite path and 9 on
//! the layered path. This pass writes fields only; composition, clumping,
//! blur, and unsharp passes own the density buffer.

use super::super::*;
use super::raster::{RasterCommands, RasterStorage, paged_transport, transport, uniform};
use spektrafilm_backend::GrainV1GpuParams;

/// Elements one plane draws per dispatch. Bounded so a dispatch has a
/// watchdog boundary; the dispatch count is derived from it.
pub(in crate::wgpu_backend) const CHUNK_ELEMENTS: u32 = 65_536;

/// Device sequential sampler state.
///
/// The state owns the scratch buffers the two-pass walk needs — the Poisson
/// seeds and the per-plane generator slots — and the caller owns only the
/// density buffer it reads and the layer-field buffer it writes.
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

/// Build the sequential sampler state, or report the named device
/// limitation that keeps the host fence.
///
/// The caller owns `density` (the film density CMY buffer, read only) and
/// `layer_fields` (the `[sl][pixel][ch]` f32 output); the state allocates
/// the scratch it needs. Declines with an error when the device lacks
/// native f64, when the curve tables are inconsistent, when the geometry
/// cannot be indexed, or when the composite path has no positive density
/// maximum.
pub(in crate::wgpu_backend) fn build_spektra_mt19937_state(
    device: &ObservedDevice,
    width: u32,
    height: u32,
    density: &(impl RasterStorage + ?Sized),
    layer_fields: &(impl RasterStorage + ?Sized),
    params: &GrainV1GpuParams,
    backend: &WgpuBackend,
) -> Result<SpektraMt19937State, String> {
    build_spektra_mt19937_state_with_limit(
        device,
        width,
        height,
        density,
        layer_fields,
        params,
        backend,
        super::super::bounded::storage_binding_limit(backend)?,
    )
}

// The bounded builder keeps raster endpoints, sampling parameters, and the
// explicit binding limit separate; bundling them would only wrap transport.
#[allow(clippy::too_many_arguments)]
fn build_spektra_mt19937_state_with_limit(
    device: &ObservedDevice,
    width: u32,
    height: u32,
    density: &(impl RasterStorage + ?Sized),
    layer_fields: &(impl RasterStorage + ?Sized),
    params: &GrainV1GpuParams,
    backend: &WgpuBackend,
    limit: u64,
) -> Result<SpektraMt19937State, String> {
    // The reference conditions its count laws on f64, so a device without
    // native f64 cannot run them; the caller keeps the host fence rather
    // than narrowing the conditioning values.
    if !device.features().contains(wgpu::Features::SHADER_F64) {
        return Err("Spektra grain MT19937 sampling needs native f64 shader support".into());
    }
    let n_pixels = width
        .checked_mul(height)
        .ok_or("Spektra grain pixel count overflows")?;
    if n_pixels == 0 {
        return Err("Spektra grain needs a nonzero image".into());
    }
    let knots = params.curve_x.len() / 3;
    if knots < 2 || params.curve_layers.len() != knots * 9 {
        return Err("Spektra grain requires matching composite and sublayer curve tables".into());
    }
    // The layered model always forms three sublayers; the composite path
    // draws `n_sub_layers` planes per channel. The plane count is whatever
    // the request configures, so no fixed cap applies.
    let layers = if params.sublayers_active {
        3
    } else {
        params.n_sub_layers.max(1)
    };
    let planes = layers
        .checked_mul(3)
        .ok_or("Spektra grain plane count overflows")?;
    if !params.sublayers_active
        && params
            .density_max_composite
            .iter()
            .any(|v| v.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater))
    {
        return Err("Spektra grain composite sampling needs a positive density maximum".into());
    }
    // 4-lane uniform rows: each 3-value triple and 9-value table is padded
    // to that stride and the shader reads lanes 0..2.
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
        // `flags: vec4<u32>` is 16-byte aligned in the uniform address
        // space, so it begins at offset 32 rather than 24.
        pad_flags: [u32; 2],
        // Bit 0: composite (not layered) path. Bit 1: monochrome, which
        // shares channel 0's stream for every channel like the reference.
        // Bit 2: the reference keeps f64 samples, so the device's
        // `sample_round` is the identity.
        flags: [u32; 4],
        base_seed: u64,
        pixel_size_um: f64,
        lookup_sign: f32,
        pad0: u32,
        // The `vec4<f64>` triples are 32-byte aligned, so the first one
        // begins at offset 96.
        pad_vec: [u32; 6],
        particle_area: [f64; 4],
        density_min: [f64; 4],
        uniformity: [f64; 4],
        density_max_composite: [f64; 4],
        n_particles_composite: [f64; 4],
        n_particles_layers: [[f64; 4]; 3],
        density_max_adj: [[f64; 4]; 3],
        density_min_layers: [[f64; 4]; 3],
    }
    let pipe = backend.cached_pipeline(
        concat!(
            include_str!("../../../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
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
        ],
    );
    let chunk = (limit / 12).min(u64::from(CHUNK_ELEMENTS)).max(1) as u32;
    if limit < 628 * 4 {
        return Err("Spektra MT19937 minimum generator binding needs 2512 bytes".into());
    }
    let density_window = transport(backend, u64::from(chunk) * 12, "mt_density_window")?;
    let field_window = transport(backend, u64::from(chunk) * 12, "mt_field_window")?;
    let seed_window = transport(backend, u64::from(chunk) * 8, "mt_seed_window")?;
    let segments = n_pixels.div_ceil(chunk);
    let mut steps: Vec<RasterCommands> = (0..segments * 2)
        .map(|_| RasterCommands::default())
        .collect();
    let make_params = |plane| Params {
        width,
        height,
        knots: knots as u32,
        layers,
        n_pixels,
        chunk_elements: chunk,
        pad_flags: [0; 2],
        flags: [
            u32::from(!params.sublayers_active),
            u32::from(params.monochrome),
            u32::from(params.reference_f64),
            plane,
        ],
        base_seed: params.base_seed,
        pixel_size_um: params.pixel_size_um,
        lookup_sign: params.lookup_sign,
        pad0: 0,
        pad_vec: [0; 6],
        particle_area: pad3(params.particle_area),
        density_min: pad3(params.density_min),
        uniformity: pad3(params.uniformity),
        density_max_composite: pad3(params.density_max_composite),
        n_particles_composite: pad3(params.n_particles_composite),
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
    for plane in 0..planes {
        let seeds = paged_transport(backend, u64::from(n_pixels) * 8, "mt_poisson_transport")?;
        let state = transport(backend, 628 * 4, "mt_persistent_generator")?;
        let params_buf = uniform(backend, &make_params(plane));
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
                        (u64::from(plane / 3) * u64::from(n_pixels) + u64::from(first)) * 12,
                        &field_window,
                        0,
                        u64::from(count) * 12,
                    );
                }
                if phase != 0 {
                    commands.copy(
                        &seeds,
                        u64::from(first) * 8,
                        &seed_window,
                        0,
                        u64::from(count) * 8,
                    );
                }
                commands.dispatch(
                    &pipe,
                    &[
                        &params_buf,
                        &density_window,
                        &curve_x,
                        &curve_layers,
                        &field_window,
                        &seed_window,
                        &state,
                    ],
                    1,
                    true,
                    backend,
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
                        (u64::from(plane / 3) * u64::from(n_pixels) + u64::from(first)) * 12,
                        u64::from(count) * 12,
                    );
                }
            }
        }
    }
    Ok(SpektraMt19937State { steps })
}

#[cfg(test)]
mod tests;
