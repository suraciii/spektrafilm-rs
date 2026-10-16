//! Bounded resident Dye execution. Only profile coefficients and scalar
//! geometry cross from the host; every image-dependent stage stays on device.
//!
//! `DyeGpuParams` and `DyeReadoutGrid` live in `spektrafilm-backend`, because
//! `ComputeBackend::dye_field` and `FilmChainParams::grain_dye` reference them
//! across the crate boundary.

use super::dye_decisions::DYE_MAX_RATE;
use super::dye_layers::{self, LayerPass};
use super::dye_spatial::{self, BAND_BUFFERS, DyeBand, DyePlan, DyeSpatialState};
use super::*;
use spektrafilm_backend::dye_params::{DyeGpuParams, DyeReadoutGrid};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct AccumulateParams {
    geometry: [i32; 4],
    seed: [u32; 2],
    cell_area: f32,
    correlation: f32,
    expectation: u32,
    padding: [u32; 3],
}

struct AccumulatePass {
    pipeline: CachedPipelineRef,
    group: wgpu::BindGroup,
    cells: u32,
}

impl AccumulatePass {
    fn encode(&self, encoder: &mut ObservedEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("dye_accumulate"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        dispatch_linear(&mut pass, self.cells);
    }
}

struct BandState {
    layers: LayerPass,
    accumulate: AccumulatePass,
    spatial: DyeSpatialState,
}

impl BandState {
    fn encode(&self, encoder: &mut ObservedEncoder) {
        self.layers.encode(encoder);
        self.spatial.encode_remap(encoder);
        self.accumulate.encode(encoder);
        self.spatial.encode_formation_readout(encoder);
    }
}

pub(in crate::wgpu_backend) struct DyeState {
    bands: Vec<BandState>,
    output: wgpu::Buffer,
    status: wgpu::Buffer,
    bytes: u64,
}

impl DyeState {
    pub(in crate::wgpu_backend) fn status_buffer(&self) -> wgpu::Buffer {
        self.status.clone()
    }

    fn submit_bands(&self, backend: &WgpuBackend) {
        for band in &self.bands {
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            band.encode(&mut encoder);
            backend.queue.submit(Some(encoder.finish()));
        }
    }

    /// Source reads finish before this final copy. The resident destination
    /// can therefore be the developed density buffer without corrupting halos.
    pub(in crate::wgpu_backend) fn append_stages<'a>(
        self,
        stages: &mut Vec<crate::wgpu_backend::pipe::ChainStage<'a>>,
        destination: &wgpu::Buffer,
    ) {
        use crate::wgpu_backend::pipe::ChainStage;
        for band in self.bands {
            stages.push(ChainStage::Gpu {
                name: "grain_dye",
                encode: Box::new(move |encoder| band.encode(encoder)),
            });
        }
        let destination = destination.clone();
        stages.push(ChainStage::Gpu {
            name: "grain_dye_readout",
            encode: Box::new(move |encoder| {
                encoder.copy_buffer_to_buffer(&self.output, 0, &destination, 0, self.bytes);
            }),
        });
    }
}

impl WgpuBackend {
    /// Standalone materialization uses exactly the resident kernels. The full
    /// source remains available when a crop or fractional output grid is read.
    pub fn dye_field(&self, image: &ImageBuf, params: &DyeGpuParams) -> Result<ImageBuf, String> {
        self.validate_dye_resources(image.width, image.height, params)?;
        let input = self.pipe.upload_image(
            scalars_to_f32(&image.data).as_ref(),
            image.pixel_count() as u32,
        );
        let state = self.prepare_dye(image.width, image.height, &input.buf, params)?;
        state.submit_bands(self);
        self.check_dye_status(&state.status)?;
        let samples = self.pipe.readback_buffer(&state.output, state.bytes);
        Ok(ImageBuf::from_data(
            params.grid.width,
            params.grid.height,
            f32_to_scalars(samples),
        ))
    }

    /// Preflight before chain allocations or sample upload.
    pub fn validate_dye_resources(
        &self,
        width: u32,
        height: u32,
        params: &DyeGpuParams,
    ) -> Result<(), String> {
        self.dye_plan(width, height, params).map(|_| ())
    }

    fn dye_plan(&self, width: u32, height: u32, params: &DyeGpuParams) -> Result<DyePlan, String> {
        let limits = self.device.raw.limits();
        if limits.max_storage_buffers_per_shader_stage < 5 {
            return Err(
                "film grain Dye: device needs five storage bindings per shader stage".into(),
            );
        }
        let plan = DyePlan::new(width, height, params, &limits)?;
        let bound = limits
            .max_buffer_size
            .min(u64::from(limits.max_storage_buffer_binding_size));
        // The band scratch sizes come from the one rule that bounds them, in
        // the order `band_fit` checks them; the plan-level buffers flank it.
        for (name, bytes) in [
            (
                "source",
                u64::from(width)
                    .saturating_mul(u64::from(height))
                    .saturating_mul(12),
            ),
            (
                "output",
                u64::from(params.grid.width)
                    .saturating_mul(u64::from(params.grid.height))
                    .saturating_mul(12),
            ),
        ]
        .into_iter()
        .chain(BAND_BUFFERS.iter().copied().zip(plan.max_band_bytes))
        .chain([
            ("channel curves", params.tables.curves.len() as u64 * 4),
            ("formation taps", params.taps.len() as u64 * 4),
        ]) {
            if bytes == 0 || bytes > bound {
                return Err(format!(
                    "film grain Dye: {name} needs {bytes} bytes; device storage limit is {bound} bytes"
                ));
            }
        }
        if !params.channel_correlation.is_finite()
            || !(0.0..1.0).contains(&params.channel_correlation)
        {
            return Err("film grain Dye: device channel correlation must be in [0, 1)".into());
        }
        let area = (params.cell_um * params.cell_um) as f32;
        if !area.is_finite() || area <= 0.0 {
            return Err("film grain Dye: field cell area is not representable in f32".into());
        }
        if params
            .taps
            .iter()
            .any(|&w| !w.is_finite() || w < 0.0 || !(w as f32).is_finite())
        {
            return Err("film grain Dye: formation taps must be finite and nonnegative".into());
        }
        for (c, table) in params.tables.tables.iter().enumerate() {
            let k = table.knot_count as usize;
            let end = table.offset as usize + k * 5;
            if k < 2 || end == 0 || end - 1 > params.tables.curves.len() {
                return Err(format!(
                    "film grain Dye: channel {c} packed curves do not match their knot count"
                ));
            }
            if !table.min.is_finite()
                || table.min < 0.0
                || !table.cap_total.is_finite()
                || table.cap_total <= 0.0
                || !table.uniformity.is_finite()
                || !(0.0..=1.0).contains(&table.uniformity)
            {
                return Err(format!(
                    "film grain Dye: channel {c} coefficients are not representable in f32"
                ));
            }
            for l in 0..3 {
                if table.cap[l] == 0.0 {
                    continue;
                }
                let s_min = (1.0 - table.uniformity) + table.uniformity * 1e-6;
                let max_rate = area * table.frac[l] / (table.area[l] * s_min);
                let mass_product = table.cap[l] * table.area[l];
                let mass = mass_product / table.frac[l];
                let max_density = mass * 16777216.0 / area;
                if !table.cap[l].is_finite()
                    || table.cap[l] < 0.0
                    || !table.frac[l].is_finite()
                    || table.frac[l] <= 0.0
                    || !table.area[l].is_finite()
                    || table.area[l] <= 0.0
                    || !max_rate.is_finite()
                    || max_rate <= 0.0
                    || max_rate > DYE_MAX_RATE as f32
                    || !mass_product.is_finite()
                    || mass_product * s_min <= 0.0
                    || !mass.is_finite()
                    || mass <= 0.0
                    || !max_density.is_finite()
                    || max_density > f32::MAX / 3.0
                {
                    return Err(format!(
                        "film grain Dye: channel {c} layer {l} has unrepresentable device rate/mass (maximum rate {max_rate}); device rate limit is {DYE_MAX_RATE}; use the CPU reference for this configuration"
                    ));
                }
            }
        }
        if params.tables.curves.iter().any(|v| !v.is_finite()) {
            return Err("film grain Dye: channel curves are not representable in f32".into());
        }
        Ok(plan)
    }

    pub(in crate::wgpu_backend) fn prepare_dye(
        &self,
        width: u32,
        height: u32,
        input: &wgpu::Buffer,
        params: &DyeGpuParams,
    ) -> Result<DyeState, String> {
        let plan = self.dye_plan(width, height, params)?;
        // Certification needs this runtime device, so this cell cannot use a
        // declaration-time initializer. Observed clones share the result.
        self.pipe
            .dye_certification
            .get_or_init(|| {
                self.certify_dye_counts()?;
                self.certify_dye_layers()?;
                self.certify_dye_accumulation()?;
                self.certify_dye_spatial()
            })
            .clone()?;
        let tables = self.pipe.static_buffer(
            "dye_channel_tables",
            bytemuck::cast_slice(&params.tables.tables),
            wgpu::BufferUsages::STORAGE,
        );
        let curves = self.pipe.static_buffer(
            "dye_channel_curves",
            bytemuck::cast_slice(&params.tables.curves),
            wgpu::BufferUsages::STORAGE,
        );
        let scratch = |label, bytes| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let status = scratch("dye_status", 4);
        let [
            source_layers_bytes,
            cell_layers_bytes,
            cells_bytes,
            horizontal_bytes,
            formed_bytes,
        ] = plan.max_band_bytes;
        let source_layers = scratch("dye_source_layers", source_layers_bytes);
        let cell_layers = scratch("dye_cell_layers", cell_layers_bytes);
        let cells = scratch("dye_cell_density", cells_bytes);
        let horizontal = scratch("dye_horizontal", horizontal_bytes);
        let formed = scratch("dye_formed", formed_bytes);
        let bytes = u64::from(params.grid.width) * u64::from(params.grid.height) * 12;
        let output = scratch("dye_readout", bytes);
        let bands = plan
            .bands
            .iter()
            .map(|band| {
                let layers = dye_layers::build_layers(
                    self,
                    [width, height, band.source_x, band.source_y],
                    [band.source_cols, band.source_rows],
                    &tables,
                    &curves,
                    input,
                    &source_layers,
                    &status,
                );
                let accumulate =
                    self.build_dye_accumulate(params, band, &tables, &cell_layers, &cells, &status);
                let spatial = dye_spatial::build_spatial(
                    self,
                    params,
                    band,
                    width,
                    height,
                    &source_layers,
                    &cell_layers,
                    &cells,
                    &horizontal,
                    &formed,
                    &output,
                );
                BandState {
                    layers,
                    accumulate,
                    spatial,
                }
            })
            .collect();
        Ok(DyeState {
            bands,
            output,
            status,
            bytes,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn build_dye_accumulate(
        &self,
        params: &DyeGpuParams,
        band: &DyeBand,
        tables: &wgpu::Buffer,
        layers: &wgpu::Buffer,
        cells: &wgpu::Buffer,
        status: &wgpu::Buffer,
    ) -> AccumulatePass {
        let uniforms = self.pipe.static_buffer(
            "dye_accumulate_params",
            bytemuck::bytes_of(&AccumulateParams {
                geometry: [
                    band.cell_x,
                    band.cell_y,
                    band.ext_cols as i32,
                    band.ext_rows as i32,
                ],
                seed: [params.seed as u32, (params.seed >> 32) as u32],
                cell_area: (params.cell_um * params.cell_um) as f32,
                correlation: params.channel_correlation,
                expectation: u32::from(params.expectation),
                padding: [0; 3],
            }),
            wgpu::BufferUsages::UNIFORM,
        );
        let storage = wgpu::BufferBindingType::Storage { read_only: true };
        let writable = wgpu::BufferBindingType::Storage { read_only: false };
        let pipeline = self.cached_pipeline(
            concat!(
                include_str!("dye_count.wgsl"),
                include_str!("dye_tables.wgsl"),
                include_str!("dye_accumulate.wgsl")
            ),
            &[
                wgpu::BufferBindingType::Uniform,
                storage,
                storage,
                writable,
                writable,
            ],
        );
        let entries: Vec<_> = [&uniforms, tables, layers, cells, status]
            .into_iter()
            .enumerate()
            .map(|(i, buffer)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dye_accumulate"),
            layout: &pipeline.layout,
            entries: &entries,
        });
        AccumulatePass {
            pipeline,
            group,
            cells: band.ext_cols * band.ext_rows,
        }
    }

    fn certify_dye_accumulation(&self) -> Result<(), String> {
        use super::dye_decisions::DyeDecisionCase;
        use spektrafilm_backend::dye_params::DyeChannelParts;
        let knots = [0.0, 1.0];
        let inverse = [1.0];
        let zero = [0.0; 2];
        let tables = DyeChannelTablesGpu::pack(std::array::from_fn(|_| DyeChannelParts {
            min: 0.0,
            cap_total: 1.0,
            cap: [1.0, 0.0, 0.0],
            frac: [1.0, 0.0, 0.0],
            area: [0.2; 3],
            uniformity: 0.0,
            knots: &knots,
            inv_dx: &inverse,
            layers: [&knots, &zero, &zero],
        }));
        let mut params = DyeGpuParams {
            tables,
            cell_um: 1.0,
            taps: vec![1.0],
            source_pitch_um: 1.0,
            grid: DyeReadoutGrid {
                origin_um: [0.0; 2],
                pitch_um: [1.0; 2],
                width: 2,
                height: 1,
            },
            seed: 0xFEDC_BA98_7654_3210,
            channel_correlation: 0.45,
            expectation: false,
        };
        let band = DyeBand {
            output_y: 0,
            output_rows: 1,
            source_x: 0,
            source_y: 0,
            source_cols: 2,
            source_rows: 1,
            cell_x: -3,
            cell_y: -5,
            cols: 2,
            rows: 1,
            ext_cols: 2,
            ext_rows: 1,
        };
        let table_buffer = self.pipe.static_buffer(
            "dye_check_tables",
            bytemuck::cast_slice(&params.tables.tables),
            wgpu::BufferUsages::STORAGE,
        );
        let inputs = [
            [1.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        ];
        let layers = self.pipe.static_buffer(
            "dye_check_layers",
            bytemuck::cast_slice(&inputs),
            wgpu::BufferUsages::STORAGE,
        );
        let buffer = |label, size| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let output = buffer("dye_check_accumulation", 24);
        let status = buffer("dye_check_status", 4);
        let cases = [
            DyeDecisionCase::new(2.25, params.seed, 9, (-3, -5)),
            DyeDecisionCase::new(2.75, params.seed, 0, (-3, -5)),
            DyeDecisionCase::new(2.75, params.seed, 3, (-3, -5)),
            DyeDecisionCase::new(5.0, params.seed, 6, (-2, -5)),
        ];
        let (counts, _) = self.dye_decision_counts(&cases)?;
        for expectation in [false, true] {
            params.expectation = expectation;
            let pass =
                self.build_dye_accumulate(&params, &band, &table_buffer, &layers, &output, &status);
            let mut encoder = self.device.create_command_encoder(&Default::default());
            pass.encode(&mut encoder);
            self.queue.submit(Some(encoder.finish()));
            self.check_dye_status(&status)?;
            let values = self.pipe.readback_buffer(&output, 24);
            let expected = if expectation {
                [1.0, 1.0, 0.0, 0.0, 0.0, 1.0]
            } else {
                [
                    0.2 * (counts[0] + counts[1]) as f32,
                    0.2 * (counts[0] + counts[2]) as f32,
                    0.0,
                    0.0,
                    0.0,
                    0.2 * counts[3] as f32,
                ]
            };
            if values
                .iter()
                .zip(expected)
                .any(|(a, b)| !a.is_finite() || (a - b).abs() > 2e-6)
            {
                return Err(
                    "film grain Dye: device correlated-accumulation self-check failed".into(),
                );
            }
        }
        Ok(())
    }

    /// Only this bounded validation word returns before final publication.
    pub(in crate::wgpu_backend) fn check_dye_status(
        &self,
        status: &wgpu::Buffer,
    ) -> Result<(), String> {
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_status_readback"),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(status, 0, &readback, 0, 4);
        self.queue.submit(Some(encoder.finish()));
        let code = self.read_mapped_u32(&readback, 4)[0];
        if code == 0 {
            return Ok(());
        }
        let reason = if code & 1 != 0 {
            "developed density is outside the finite channel capacity envelope"
        } else if code & 2 != 0 {
            "conditional dye mass or rate is not representable"
        } else if code & 4 != 0 {
            "a conditional count exceeded its exact-integer or draw budget"
        } else {
            "unknown field validation failure"
        };
        Err(format!(
            "film grain Dye: device field failed ({reason}; flags {code:#x})"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::super::dye_decisions::DyeDecisionCase;
    use spektrafilm_backend::dye_params::DyeChannelParts;
    use super::*;

    #[test]
    fn accumulation_uses_the_certified_streams_and_shared_layer_counts() {
        let _guard = crate::wgpu_backend::observation::TEST_DEVICE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let backend = crate::test_support::adapter();
        let knots = [0.0, 1.0];
        let inverse = [1.0];
        let zero = [0.0; 2];
        let band = DyeBand {
            output_y: 0,
            output_rows: 16,
            source_x: 0,
            source_y: 0,
            source_cols: 64,
            source_rows: 16,
            cell_x: -13,
            cell_y: -7,
            cols: 64,
            rows: 16,
            ext_cols: 64,
            ext_rows: 16,
        };
        let count = (band.ext_cols * band.ext_rows) as usize;
        let broken: &'static str = Box::leak(
            concat!(
                include_str!("dye_count.wgsl"),
                include_str!("dye_tables.wgsl"),
                include_str!("dye_accumulate.wgsl")
            )
            .replace(
                "shared_rate = params.correlation * smallest;",
                "shared_rate = 0.0;",
            )
            .into_boxed_str(),
        );
        // Rotate one/two/three active channels independently in each layer.
        // Distinct channel rates and layer masses expose min-rate and tag bugs.
        let caps = [1.0_f32, 2.0, 3.0];
        let fractions = [0.25_f32, 0.25, 0.5];
        let targets: Vec<[f32; 9]> = (0..count)
            .map(|index| {
                std::array::from_fn(|lane| {
                    if lane / 3 <= (index + lane % 3) % 3 {
                        caps[lane % 3]
                    } else {
                        0.0
                    }
                })
            })
            .collect();
        let layers = backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("accumulation_test_targets"),
                contents: bytemuck::cast_slice(&targets),
                usage: wgpu::BufferUsages::STORAGE,
            });
        for requested_rate in [0.03, 29.999998, 30.0, 30.000002, 160.0] {
            for correlation in [0.0_f32, 0.45] {
                let area = 1.0_f32 / requested_rate;
                let tables = DyeChannelTablesGpu::pack(std::array::from_fn(|c| DyeChannelParts {
                    min: 0.0,
                    cap_total: 6.0,
                    cap: caps.map(f64::from),
                    frac: fractions.map(f64::from),
                    area: fractions.map(|f| f64::from(area * f * (1u32 << c) as f32)),
                    uniformity: 0.0,
                    knots: &knots,
                    inv_dx: &inverse,
                    layers: [&knots, &zero, &zero],
                }));
                let params = DyeGpuParams {
                    tables,
                    cell_um: 1.0,
                    taps: vec![1.0],
                    source_pitch_um: 1.0,
                    grid: DyeReadoutGrid {
                        origin_um: [0.0; 2],
                        pitch_um: [1.0; 2],
                        width: 64,
                        height: 16,
                    },
                    seed: 0xFEDC_BA98_7654_3210,
                    channel_correlation: correlation,
                    expectation: false,
                };
                let table_buffer = backend.pipe.static_buffer(
                    "accumulation_test_tables",
                    bytemuck::cast_slice(&params.tables.tables),
                    wgpu::BufferUsages::STORAGE,
                );
                let output = backend.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("accumulation_test_output"),
                    size: count as u64 * 12,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                let status = backend.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("accumulation_test_status"),
                    size: 4,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                let pass = backend.build_dye_accumulate(
                    &params,
                    &band,
                    &table_buffer,
                    &layers,
                    &output,
                    &status,
                );
                let mut encoder = backend.device.create_command_encoder(&Default::default());
                pass.encode(&mut encoder);
                backend.queue.submit(Some(encoder.finish()));
                backend.check_dye_status(&status).unwrap();
                let values = backend.pipe.readback_buffer(&output, count as u64 * 12);
                let cases: Vec<_> = (0..count)
                    .flat_map(|index| {
                        let cell = (
                            band.cell_x + (index as u32 % band.ext_cols) as i32,
                            band.cell_y + (index as u32 / band.ext_cols) as i32,
                        );
                        (0..3).flat_map(move |layer| {
                            let active = (index + layer) % 3 + 1;
                            let rate = 1.0_f32 / area;
                            let shared = if active >= 2 {
                                correlation * rate / (1u32 << (active - 1)) as f32
                            } else {
                                0.0
                            };
                            std::iter::once(DyeDecisionCase::new(
                                f64::from(shared),
                                params.seed,
                                9 + layer as u64,
                                cell,
                            ))
                            .chain((0..3).map(move |c| {
                                DyeDecisionCase::new(
                                    f64::from(if c < active {
                                        rate / (1u32 << c) as f32 - shared
                                    } else {
                                        0.0
                                    }),
                                    params.seed,
                                    (c * 3 + layer) as u64,
                                    cell,
                                )
                            }))
                        })
                    })
                    .collect();
                let (counts, _) = backend.dye_decision_counts(&cases).unwrap();
                let expected: Vec<[f32; 3]> = counts
                    .as_chunks::<12>()
                    .0
                    .iter()
                    .enumerate()
                    .map(|(index, counts)| {
                        std::array::from_fn(|c| {
                            (0..3)
                                .filter(|&l| c <= (index + l) % 3)
                                .map(|l| {
                                    let table = &params.tables.tables[c];
                                    let mass = table.cap[l] * table.area[l] / table.frac[l];
                                    mass * (counts[l * 4] + counts[l * 4 + c + 1]) as f32
                                })
                                .sum()
                        })
                    })
                    .collect();
                let violations = |actual: &[f32]| {
                    actual
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .zip(&expected)
                        .filter(|(pixel, want)| {
                            (0..3).any(|c| (pixel[c] - want[c]).abs() > 2e-6 * want[c].max(1.0))
                        })
                        .count()
                };
                assert_eq!(
                    violations(&values),
                    0,
                    "lambda {requested_rate}, corr {correlation}"
                );
                if correlation > 0.0 {
                    // Same marginals but no shared stream: this deliberately
                    // broken G4 must fail even though its G1 count law is sound.
                    let mut control = backend.build_dye_accumulate(
                        &params,
                        &band,
                        &table_buffer,
                        &layers,
                        &output,
                        &status,
                    );
                    let read = wgpu::BufferBindingType::Storage { read_only: true };
                    let write = wgpu::BufferBindingType::Storage { read_only: false };
                    control.pipeline = backend.cached_pipeline(
                        broken,
                        &[wgpu::BufferBindingType::Uniform, read, read, write, write],
                    );
                    let mut encoder = backend.device.create_command_encoder(&Default::default());
                    control.encode(&mut encoder);
                    backend.queue.submit(Some(encoder.finish()));
                    let bad = backend.pipe.readback_buffer(&output, count as u64 * 12);
                    let rejected = violations(&bad);
                    assert!(rejected > 0, "G4 check did not reject missing correlation");
                    eprintln!(
                        "G4 {}: lambda {requested_rate}, corr {correlation}, production 0/{count} mismatches, control {rejected}/{count}",
                        backend.name()
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod range_tests {
    use spektrafilm_backend::dye_params::DyeChannelParts;
    use super::*;

    #[test]
    fn unrepresentable_mass_and_positive_rate_fail_instead_of_borrowing_shared_counts() {
        let _guard = crate::wgpu_backend::observation::TEST_DEVICE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let backend = crate::test_support::adapter();
        let knots = [0.0, 1.0];
        let inverse = [1.0];
        let zero = [0.0; 2];
        let tables = DyeChannelTablesGpu::pack(std::array::from_fn(|c| DyeChannelParts {
            min: 0.0,
            cap_total: 1.0,
            cap: [1.0, 0.0, 0.0],
            frac: [1.0, 0.0, 0.0],
            area: [if c == 2 { 2.0_f64.powi(70) } else { 1.0 }; 3],
            uniformity: 0.0,
            knots: &knots,
            inv_dx: &inverse,
            layers: [&knots, &zero, &zero],
        }));
        let mut params = DyeGpuParams {
            tables,
            cell_um: 1.0,
            taps: vec![1.0],
            source_pitch_um: 1.0,
            grid: DyeReadoutGrid {
                origin_um: [0.0; 2],
                pitch_um: [1.0; 2],
                width: 2,
                height: 1,
            },
            seed: 7,
            channel_correlation: 0.45,
            expectation: false,
        };
        let input = ImageBuf::from_data(
            2,
            1,
            [1.0, 1.0, 2.0_f64.powi(-90)]
                .into_iter()
                .cycle()
                .take(6)
                .map(spektrafilm_math::precision::from_f64)
                .collect(),
        );
        let error = backend.dye_field(&input, &params).unwrap_err();
        assert!(error.contains("conditional dye mass or rate"), "{error}");

        let table = &mut params.tables.tables[0];
        table.cap = [2.0_f32.powi(-81), 1.0, 0.0, 0.0];
        table.frac = table.cap;
        table.area = [2.0_f32.powi(-50), 1.0, 1.0, 0.0];
        table.uniformity = 1.0;
        let error = backend.validate_dye_resources(2, 1, &params).unwrap_err();
        assert!(
            error.contains("channel 0 layer 0") && error.contains("rate/mass"),
            "{error}"
        );
    }
}
