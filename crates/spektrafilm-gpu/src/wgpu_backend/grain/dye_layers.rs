//! G2 of the Dye device path: per-pixel density to per-pixel layer targets.
//!
//! This is the device half of `spektrafilm_model::grain::v3::Model::layer_targets`.
//! The device is not asked to reproduce the CPU's draws, only the same
//! conditioning, so the two are compared to a tolerance in `spektrafilm-core`
//! where both the model and this backend are visible.

use super::*;

/// One channel's device-side tables.
///
/// The layout matches `Table` in `dye_tables.wgsl`, where the three `vec4`
/// members force sixteen-byte alignment, so the padding is explicit rather than
/// implied: a `#[repr(C)]` struct of four-byte members would otherwise put
/// `cap` eight bytes earlier than the shader reads it.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DyeChannelTableGpu {
    pub min: f32,
    pub cap_total: f32,
    pub pad_a: [u32; 2],
    pub cap: [f32; 4],
    pub frac: [f32; 4],
    pub area: [f32; 4],
    pub uniformity: f32,
    pub knot_count: u32,
    pub offset: u32,
    pub pad_b: u32,
}

/// All three channels: the fixed tables plus the packed knot and curve data.
///
/// `curves` holds each channel's block in the shader's order: `[knots K]
/// [inv_dx K-1][layer 0 K][layer 1 K][layer 2 K]`. `K` comes from the film
/// profile's curve resolution, so the offsets are data rather than a compiled
/// maximum.
#[derive(Clone, Debug, Default)]
pub struct DyeChannelTablesGpu {
    pub tables: [DyeChannelTableGpu; 3],
    pub curves: Vec<f32>,
}

/// One channel's tables as plain slices.
///
/// The device side has no model dependency, so the owner of the model hands
/// its tables over in this shape and this crate decides where they land in the
/// curve buffer.
#[derive(Clone, Debug)]
pub struct DyeChannelParts<'a> {
    pub min: f64,
    pub cap_total: f64,
    pub cap: [f64; 3],
    pub frac: [f64; 3],
    pub area: [f64; 3],
    pub uniformity: f64,
    pub knots: &'a [f64],
    pub inv_dx: &'a [f64],
    pub layers: [&'a [f64]; 3],
}

impl DyeChannelTablesGpu {
    /// Pack three channels into the device layout.
    ///
    /// The curve block layout `[knots K][inv_dx K-1][layer 0 K][layer 1 K]
    /// [layer 2 K]` and the resulting offsets are this crate's rule, so callers
    /// hand over values and not addresses. `inv_dx` carries one spacing per
    /// bracket and a bracket is chosen with `low <= K-2`, so its block is one
    /// shorter than the knot block. Every channel must come from one model,
    /// whose tables are already length-checked.
    pub fn pack(channels: [DyeChannelParts<'_>; 3]) -> Self {
        let size = channels.iter().map(|c| c.knots.len() * 5 - 1).sum();
        let mut curves = Vec::with_capacity(size);
        let tables = std::array::from_fn(|c| {
            let channel = &channels[c];
            debug_assert_eq!(channel.inv_dx.len() + 1, channel.knots.len());
            debug_assert!(channel.knots.len() >= 2);
            debug_assert!(
                channel
                    .layers
                    .iter()
                    .all(|l| l.len() == channel.knots.len())
            );
            let offset = curves.len() as u32;
            curves.extend(channel.knots.iter().map(|&v| v as f32));
            curves.extend(channel.inv_dx.iter().map(|&v| v as f32));
            for layer in channel.layers {
                curves.extend(layer.iter().map(|&v| v as f32));
            }
            DyeChannelTableGpu {
                min: channel.min as f32,
                cap_total: channel.cap_total as f32,
                pad_a: [0; 2],
                cap: [
                    channel.cap[0] as f32,
                    channel.cap[1] as f32,
                    channel.cap[2] as f32,
                    0.0,
                ],
                frac: [
                    channel.frac[0] as f32,
                    channel.frac[1] as f32,
                    channel.frac[2] as f32,
                    0.0,
                ],
                area: [
                    channel.area[0] as f32,
                    channel.area[1] as f32,
                    channel.area[2] as f32,
                    0.0,
                ],
                uniformity: channel.uniformity as f32,
                knot_count: channel.knots.len() as u32,
                offset,
                pad_b: 0,
            }
        });
        Self { tables, curves }
    }
}

const SHADER: &str = concat!(
    include_str!("dye_tables.wgsl"),
    include_str!("dye_layers.wgsl")
);

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct LayerParams {
    source: [u32; 4],
    region: [u32; 4],
}

pub(super) struct LayerPass {
    pipeline: CachedPipelineRef,
    group: wgpu::BindGroup,
    pixels: u32,
}

impl LayerPass {
    pub(super) fn encode(&self, encoder: &mut ObservedEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("dye_layer_targets"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        dispatch_linear(&mut pass, self.pixels);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_layers(
    backend: &WgpuBackend,
    source: [u32; 4],
    region: [u32; 2],
    tables: &wgpu::Buffer,
    curves: &wgpu::Buffer,
    input: &wgpu::Buffer,
    targets: &wgpu::Buffer,
    status: &wgpu::Buffer,
) -> LayerPass {
    build_layers_with_source(
        backend, source, region, tables, curves, input, targets, status, SHADER,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_layers_with_source(
    backend: &WgpuBackend,
    source: [u32; 4],
    region: [u32; 2],
    tables: &wgpu::Buffer,
    curves: &wgpu::Buffer,
    input: &wgpu::Buffer,
    targets: &wgpu::Buffer,
    status: &wgpu::Buffer,
    shader: &'static str,
) -> LayerPass {
    let uniform = backend.pipe.static_buffer(
        "dye_layer_params",
        bytemuck::bytes_of(&LayerParams {
            source,
            region: [region[0], region[1], 0, 0],
        }),
        wgpu::BufferUsages::UNIFORM,
    );
    let storage = wgpu::BufferBindingType::Storage { read_only: true };
    let writable = wgpu::BufferBindingType::Storage { read_only: false };
    let pipeline = backend.cached_pipeline(
        shader,
        &[
            wgpu::BufferBindingType::Uniform,
            storage,
            storage,
            storage,
            writable,
            writable,
        ],
    );
    let entries: Vec<_> = [&uniform, tables, curves, input, targets, status]
        .into_iter()
        .enumerate()
        .map(|(i, buffer)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: buffer.as_entire_binding(),
        })
        .collect();
    let group = backend
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dye_layer_targets"),
            layout: &pipeline.layout,
            entries: &entries,
        });
    LayerPass {
        pipeline,
        group,
        pixels: region[0] * region[1],
    }
}

impl WgpuBackend {
    /// Diagnostic materialization of G2, in `[channel][layer]` order.
    /// Production uses the same pass with resident input and band scratch.
    pub fn dye_layer_targets(
        &self,
        densities: &[[f32; 3]],
        tables: &DyeChannelTablesGpu,
    ) -> Vec<[f32; 9]> {
        self.dye_layer_targets_with(densities, tables, SHADER)
    }

    pub(super) fn certify_dye_layers(&self) -> Result<(), String> {
        let knots = [0.0, 6.0];
        let inv_dx = [1.0 / 6.0];
        let dominant = [10.0; 2];
        let secondary = [1.0; 2];
        let zero = [0.0; 2];
        let inputs = [[0.0_f32; 3], [0.55; 3], [3.0; 3], [3.5; 3], [6.0; 3]];
        for fallback in [false, true] {
            let tables = DyeChannelTablesGpu::pack(std::array::from_fn(|_| DyeChannelParts {
                min: 0.0,
                cap_total: 6.0,
                cap: [1.0, 2.0, 3.0],
                frac: [1.0 / 6.0, 1.0 / 3.0, 0.5],
                area: [1.0; 3],
                uniformity: 0.0,
                knots: &knots,
                inv_dx: &inv_dx,
                layers: if fallback {
                    [&zero; 3]
                } else {
                    [&dominant, &secondary, &zero]
                },
            }));
            for (input, output) in inputs.iter().zip(self.dye_layer_targets(&inputs, &tables)) {
                let value = input[0];
                let expected = if fallback {
                    [value / 6.0, value / 3.0, value / 2.0]
                } else if value <= 1.1 {
                    [10.0 * value / 11.0, value / 11.0, 0.0]
                } else if value <= 3.0 {
                    [1.0, value - 1.0, 0.0]
                } else {
                    [1.0, 2.0, value - 3.0]
                };
                if output
                    .iter()
                    .zip(expected.into_iter().cycle())
                    .any(|(actual, expected)| {
                        !actual.is_finite() || (actual - expected).abs() > 2e-6
                    })
                {
                    return Err("film grain Dye: device layer-allocation self-check failed".into());
                }
            }
        }
        Ok(())
    }

    fn dye_layer_targets_with(
        &self,
        densities: &[[f32; 3]],
        tables: &DyeChannelTablesGpu,
        shader: &'static str,
    ) -> Vec<[f32; 9]> {
        if densities.is_empty() {
            return Vec::new();
        }
        let input = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dye_layer_densities"),
                contents: bytemuck::cast_slice(densities),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let table_buffer = self.pipe.static_buffer(
            "dye_layer_tables",
            bytemuck::cast_slice(&tables.tables),
            wgpu::BufferUsages::STORAGE,
        );
        let curve_buffer = self.pipe.static_buffer(
            "dye_layer_curves",
            bytemuck::cast_slice(&tables.curves),
            wgpu::BufferUsages::STORAGE,
        );
        let bytes = densities.len() as u64 * 9 * 4;
        let targets = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_layer_targets"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let status = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_status"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let layer_pass = build_layers_with_source(
            self,
            [densities.len() as u32, 1, 0, 0],
            [densities.len() as u32, 1],
            &table_buffer,
            &curve_buffer,
            &input,
            &targets,
            &status,
            shader,
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        layer_pass.encode(&mut encoder);
        self.queue.submit(Some(encoder.finish()));
        // This diagnostic also exercises the endpoint clamp outside the
        // production envelope; production checks the status before publication.
        self.pipe
            .readback_buffer(&targets, bytes)
            .as_chunks::<9>()
            .0
            .to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturation_retains_retired_layers_and_zero_weights_use_capacity() {
        let _guard = crate::wgpu_backend::observation::TEST_DEVICE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let backend = crate::test_support::adapter();
        eprintln!("G2 boundary sweep adapter: {}", backend.name());
        let knots = [0.0, 6.0];
        let spacing = [1.0 / 6.0];
        let dominant = [10.0; 2];
        let secondary = [1.0; 2];
        let zero = [0.0; 2];
        let make = |weights| {
            DyeChannelTablesGpu::pack(std::array::from_fn(|_| DyeChannelParts {
                min: 0.0,
                cap_total: 6.0,
                cap: [1.0, 2.0, 3.0],
                frac: [1.0 / 6.0, 2.0 / 6.0, 0.5],
                area: [1.0; 3],
                uniformity: 0.0,
                knots: &knots,
                inv_dx: &spacing,
                layers: weights,
            }))
        };
        let mut inputs = vec![[0.0; 3], [0.5; 3], [3.5; 3], [6.0; 3]];
        for boundary in [1.1_f32, 3.0, 6.0] {
            for bits in 5..=23 {
                let delta = 2.0_f32.powi(-bits);
                inputs.extend([[boundary - delta; 3], [boundary; 3], [boundary + delta; 3]]);
            }
        }
        let broken = Box::leak(
            SHADER
                .replace(
                    "out = select(out, vec3<f32>(0.0), unsaturated);",
                    "out = select(vec3<f32>(0.0), out, unsaturated);",
                )
                .into_boxed_str(),
        );
        let control = backend.dye_layer_targets_with(
            &[[3.5; 3]],
            &make([&dominant, &secondary, &zero]),
            broken,
        );
        let control_error = control[0]
            .iter()
            .zip([1.0, 2.0, 0.5].into_iter().cycle())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            control_error > 0.1,
            "allocation oracle accepted the retired-capacity defect"
        );
        eprintln!("G2 broken-control maximum error: {control_error}");
        for (tables, fallback) in [
            (make([&dominant, &secondary, &zero]), false),
            (make([&zero; 3]), true),
        ] {
            let output = backend.dye_layer_targets(&inputs, &tables);
            let mut maximum_error = 0.0_f64;
            for (input, layers) in inputs.iter().zip(output) {
                let total = f64::from(input[0]).clamp(0.0, 6.0);
                let expected = if fallback {
                    [total / 6.0, total / 3.0, total / 2.0]
                } else if total <= 1.1 {
                    [10.0 * total / 11.0, total / 11.0, 0.0]
                } else if total <= 3.0 {
                    [1.0, total - 1.0, 0.0]
                } else {
                    [1.0, 2.0, total - 3.0]
                };
                for channel in layers.as_chunks::<3>().0 {
                    for (actual, expected) in channel.iter().zip(expected) {
                        maximum_error = maximum_error.max((f64::from(*actual) - expected).abs());
                        assert!(
                            (f64::from(*actual) - expected).abs() < 2e-6,
                            "density {input:?}, actual {channel:?}, expected {expected}"
                        );
                    }
                }
            }
            eprintln!(
                "G2 placed boundaries delta=2^-5..2^-23 fallback={fallback}: max error {maximum_error}"
            );
        }
        for exponent in [-70, 70] {
            let scale = 2.0_f64.powi(exponent);
            let knots = [0.0, 3.0 * scale];
            let inv_dx = [1.0 / (3.0 * scale)];
            let weights = [scale; 2];
            let tables = DyeChannelTablesGpu::pack(std::array::from_fn(|_| DyeChannelParts {
                min: 0.0,
                cap_total: 3.0 * scale,
                cap: [scale; 3],
                frac: [1.0 / 3.0; 3],
                area: [1.0; 3],
                uniformity: 0.0,
                knots: &knots,
                inv_dx: &inv_dx,
                layers: [&weights; 3],
            }));
            let output = backend.dye_layer_targets(&[[scale as f32; 3]], &tables);
            for value in output[0] {
                assert!(
                    (f64::from(value) / (scale / 3.0) - 1.0).abs() < 2e-6,
                    "allocation lost a finite quotient at 2^{exponent}: {value}"
                );
            }
        }
    }
}
