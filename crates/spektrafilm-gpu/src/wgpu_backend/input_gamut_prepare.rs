use super::bounded::storage_binding_limit;
use super::preparation::PreparationTable;
use super::*;
use spektrafilm_backend::{InputGamutTableParams, PreparedTable};
use std::num::NonZeroU64;
use std::sync::Arc;

const SHADER: &str = concat!(
    include_str!("../../../spektrafilm-shaders/wgsl/f64_math.wgsl"),
    include_str!("gamut/jzazbz.wgsl"),
    include_str!("input_gamut_prepare.wgsl"),
);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    counts: [u32; 4],
    flags: [u32; 4],
    white: [f64; 2],
    white_pad: [f64; 2],
    knee: [f64; 4],
    neutral: [f64; 2],
    detail: f64,
    pad: f64,
}

impl WgpuBackend {
    pub(super) fn remap_input_gamut_table_gpu(
        &self,
        table: &PreparedTable,
        p: &InputGamutTableParams<'_>,
    ) -> Result<PreparedTable, String> {
        if !self.device.features().contains(wgpu::Features::SHADER_F64) {
            return Err("input gamut preparation requires device f64".to_owned());
        }
        if table.values.backend_id() != self.backend_id {
            return Err("input gamut preparation received a foreign device table".to_owned());
        }
        let table = table
            .values
            .as_any()
            .downcast_ref::<PreparationTable>()
            .ok_or("input gamut preparation requires a spectral device table")?;
        let cells = p
            .size
            .checked_mul(p.size)
            .filter(|_| p.size >= 2)
            .ok_or("input gamut table size is not device indexable")?;
        let samples = cells
            .checked_mul(3)
            .ok_or("input gamut table size overflows")?;
        if p.algorithm > 1 || p.boundary > 1 || p.locus.len() != 66 {
            return Err("input gamut preparation received unsupported coefficients".to_owned());
        }
        let bytes = u64::from(samples) * 8;
        if table.f64_buffer.size() < bytes || bytes > self.device.limits().max_buffer_size {
            return Err("input gamut source table does not match its declared size".to_owned());
        }
        let limit = storage_binding_limit(self)?;
        // The fixed FFT working set is 1024 complex f64 values. The CMF locus
        // and the final 16 RGB calibration taps are smaller bounded resources.
        let minimum = if p.algorithm == 0 && p.boundary == 1 {
            16384
        } else {
            1056
        };
        if limit < minimum {
            return Err(format!(
                "input gamut preparation needs a {minimum}-byte minimum binding; limit is {limit}"
            ));
        }
        let alignment = u64::from(self.device.limits().min_storage_buffer_offset_alignment).max(8);
        let page_bytes = limit / alignment * alignment;
        let page_samples = u32::try_from((page_bytes / 8).min(u64::from(u32::MAX)))
            .map_err(|_| "input gamut source page overflows")?;
        if page_samples == 0 {
            return Err("input gamut source page is empty".to_owned());
        }
        // Twelve f64 corner samples dominate each remap cell's scratch binding.
        let batch_cells = (limit / 96).min(u64::from(cells)) as u32;
        let mut params = Params {
            counts: [0, 0, p.size, 0],
            flags: [p.algorithm, p.boundary, 0, 0],
            white: p.white_xy,
            white_pad: [0.0; 2],
            knee: [p.knee[0], p.knee[1], p.knee[2], 0.0],
            neutral: p.neutral_tc,
            detail: p.hull_detail,
            pad: 0.0,
        };
        let locus = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("input_gamut_locus"),
                contents: bytemuck::cast_slice(p.locus),
                usage: wgpu::BufferUsages::STORAGE,
            });
        // Distinct dummies keep unused binding slots from introducing aliases.
        let dummy_source = self.input_gamut_buffer(8, "input_gamut_unused_source");
        let dummy_scratch = self.input_gamut_buffer(8, "input_gamut_unused_scratch");
        let dummy_narrow = self.input_gamut_buffer(4, "input_gamut_unused_narrow");
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let hull = if p.algorithm == 0 && p.boundary == 1 {
            let a = self.input_gamut_buffer(16384, "input_gamut_fft_a");
            let b = self.input_gamut_buffer(16384, "input_gamut_fft_b");
            let reach = self.input_gamut_buffer(8192, "input_gamut_reach");
            self.input_gamut_pass(
                &mut encoder,
                "reach",
                &params,
                &locus,
                [&dummy_source, &a, &reach, &dummy_narrow]
                    .map(wgpu::Buffer::as_entire_buffer_binding),
                1024,
            );
            let mut current = a;
            let mut next = b;
            for inverse in [false, true] {
                params.flags[0] = u32::from(inverse);
                for stage in 0..10 {
                    params.counts[3] = stage;
                    self.input_gamut_pass(
                        &mut encoder,
                        "fft",
                        &params,
                        &locus,
                        [&current, &next, &dummy_scratch, &dummy_narrow]
                            .map(wgpu::Buffer::as_entire_buffer_binding),
                        512,
                    );
                    std::mem::swap(&mut current, &mut next);
                }
                if !inverse {
                    self.input_gamut_pass(
                        &mut encoder,
                        "frequency_filter",
                        &params,
                        &locus,
                        [&current, &next, &dummy_scratch, &dummy_narrow]
                            .map(wgpu::Buffer::as_entire_buffer_binding),
                        1024,
                    );
                    std::mem::swap(&mut current, &mut next);
                }
            }
            let scale = self.input_gamut_buffer(8, "input_gamut_hull_scale");
            self.input_gamut_pass(
                &mut encoder,
                "hull_scale",
                &params,
                &locus,
                [&current, &scale, &reach, &dummy_narrow]
                    .map(wgpu::Buffer::as_entire_buffer_binding),
                1,
            );
            let hull = self.input_gamut_buffer(16384, "input_gamut_hull");
            self.input_gamut_pass(
                &mut encoder,
                "hull",
                &params,
                &locus,
                [&current, &hull, &scale, &dummy_narrow]
                    .map(wgpu::Buffer::as_entire_buffer_binding),
                1024,
            );
            Some(hull)
        } else {
            None
        };
        params.flags[0] = p.algorithm;
        let cmax = if p.algorithm == 1 {
            let table = self.input_gamut_buffer(64 * 720 * 8, "input_gamut_cmax");
            for first in (0u32..64 * 720).step_by(page_samples as usize) {
                let count = page_samples.min(64 * 720 - first);
                params.counts[0] = first;
                params.counts[1] = count;
                self.input_gamut_pass(
                    &mut encoder,
                    "cmax",
                    &params,
                    &locus,
                    [
                        dummy_source.as_entire_buffer_binding(),
                        Self::input_gamut_binding(
                            &table,
                            u64::from(first) * 8,
                            u64::from(count) * 8,
                        ),
                        dummy_scratch.as_entire_buffer_binding(),
                        dummy_narrow.as_entire_buffer_binding(),
                    ],
                    count,
                );
            }
            Some(table)
        } else {
            None
        };
        let output = self.input_gamut_buffer(u64::from(samples) * 4, "input_gamut_table_f32");
        let output_f64 = self.input_gamut_buffer(bytes, "input_gamut_table_f64");
        let coords =
            self.input_gamut_buffer(u64::from(batch_cells) * 32, "input_gamut_coordinates");
        let corners = self.input_gamut_buffer(u64::from(batch_cells) * 96, "input_gamut_corners");
        let cmax_corners = cmax.as_ref().map(|_| {
            self.input_gamut_buffer(u64::from(batch_cells) * 32, "input_gamut_cmax_corners")
        });
        let oklch_coords = cmax.as_ref().map(|_| {
            self.input_gamut_buffer(u64::from(batch_cells) * 32, "input_gamut_oklch_coordinates")
        });
        let batch_f64 =
            self.input_gamut_buffer(u64::from(batch_cells) * 24, "input_gamut_batch_f64");
        let batch_f32 =
            self.input_gamut_buffer(u64::from(batch_cells) * 12, "input_gamut_batch_f32");
        for first in (0..cells).step_by(batch_cells as usize) {
            let count = batch_cells.min(cells - first);
            params.counts[0] = first;
            params.counts[1] = count;
            self.input_gamut_pass(
                &mut encoder,
                "coordinates",
                &params,
                &locus,
                [
                    hull.as_ref().unwrap_or(&dummy_source),
                    &coords,
                    &dummy_scratch,
                    &dummy_narrow,
                ]
                .map(wgpu::Buffer::as_entire_buffer_binding),
                count,
            );
            let coordinates = if let Some(cmax) = &cmax {
                let cmax_corners = cmax_corners
                    .as_ref()
                    .expect("Oklch scratch is allocated with Cmax");
                let oklch_coords = oklch_coords
                    .as_ref()
                    .expect("Oklch scratch is allocated with Cmax");
                for page in (0u32..64 * 720).step_by(page_samples as usize) {
                    let len = page_samples.min(64 * 720 - page);
                    params.flags[2] = page;
                    params.flags[3] = len;
                    self.input_gamut_pass(
                        &mut encoder,
                        "gather_cmax",
                        &params,
                        &locus,
                        [
                            Self::input_gamut_binding(
                                cmax,
                                u64::from(page) * 8,
                                u64::from(len) * 8,
                            ),
                            cmax_corners.as_entire_buffer_binding(),
                            coords.as_entire_buffer_binding(),
                            dummy_narrow.as_entire_buffer_binding(),
                        ],
                        count,
                    );
                }
                self.input_gamut_pass(
                    &mut encoder,
                    "oklch_coordinates",
                    &params,
                    &locus,
                    [cmax_corners, oklch_coords, &coords, &dummy_narrow]
                        .map(wgpu::Buffer::as_entire_buffer_binding),
                    count,
                );
                oklch_coords
            } else {
                &coords
            };
            for page in (0..samples).step_by(page_samples as usize) {
                let len = page_samples.min(samples - page);
                params.flags[2] = page;
                params.flags[3] = len;
                self.input_gamut_pass(
                    &mut encoder,
                    "gather_rgb",
                    &params,
                    &locus,
                    [
                        Self::input_gamut_binding(
                            &table.f64_buffer,
                            u64::from(page) * 8,
                            u64::from(len) * 8,
                        ),
                        corners.as_entire_buffer_binding(),
                        coordinates.as_entire_buffer_binding(),
                        dummy_narrow.as_entire_buffer_binding(),
                    ],
                    count,
                );
            }
            self.input_gamut_pass(
                &mut encoder,
                "interpolate",
                &params,
                &locus,
                [&corners, &batch_f64, coordinates, &batch_f32]
                    .map(wgpu::Buffer::as_entire_buffer_binding),
                count,
            );
            encoder.copy_buffer_to_buffer(
                &batch_f64,
                0,
                &output_f64,
                u64::from(first) * 24,
                u64::from(count) * 24,
            );
            encoder.copy_buffer_to_buffer(
                &batch_f32,
                0,
                &output,
                u64::from(first) * 12,
                u64::from(count) * 12,
            );
        }
        // Only sixteen calibration taps are gathered; their Mitchell reduction
        // remains on-device and only its final three binary64 values are read.
        let neutral_taps = self.input_gamut_buffer(48 * 8, "input_gamut_neutral_taps");
        let neutral_result = self.input_gamut_buffer(24, "input_gamut_neutral");
        for page in (0..samples).step_by(page_samples as usize) {
            let len = page_samples.min(samples - page);
            params.flags[2] = page;
            params.flags[3] = len;
            self.input_gamut_pass(
                &mut encoder,
                "neutral_gather",
                &params,
                &locus,
                [
                    Self::input_gamut_binding(&output_f64, u64::from(page) * 8, u64::from(len) * 8),
                    neutral_taps.as_entire_buffer_binding(),
                    dummy_scratch.as_entire_buffer_binding(),
                    dummy_narrow.as_entire_buffer_binding(),
                ],
                48,
            );
        }
        self.input_gamut_pass(
            &mut encoder,
            "neutral_reduce",
            &params,
            &locus,
            [
                &neutral_taps,
                &neutral_result,
                &dummy_scratch,
                &dummy_narrow,
            ]
            .map(wgpu::Buffer::as_entire_buffer_binding),
            3,
        );
        self.submit_commands(Some(encoder.finish()))?;
        let neutral: Vec<f64> = self.pipe.readback_buffer_cancellable(&neutral_result, 24)?;
        self.device.context.record_executor(Executor::Gpu, None);
        Ok(PreparedTable {
            values: Arc::new(PreparationTable {
                backend_id: self.backend_id,
                buffer: output,
                f64_buffer: output_f64,
            }),
            neutral: [neutral[0], neutral[1], neutral[2]],
        })
    }

    fn input_gamut_buffer(&self, bytes: u64, label: &str) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn input_gamut_binding(
        buffer: &wgpu::Buffer,
        offset: u64,
        bytes: u64,
    ) -> wgpu::BufferBinding<'_> {
        wgpu::BufferBinding {
            buffer,
            offset,
            size: NonZeroU64::new(bytes),
        }
    }

    fn input_gamut_pass(
        &self,
        encoder: &mut ObservedEncoder,
        entry: &'static str,
        p: &Params,
        locus: &wgpu::Buffer,
        buffers: [wgpu::BufferBinding<'_>; 4],
        n: u32,
    ) {
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("input_gamut_params"),
                contents: bytemuck::bytes_of(p),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let read = wgpu::BufferBindingType::Storage { read_only: true };
        let write = wgpu::BufferBindingType::Storage { read_only: false };
        let pipeline = self.cached_pipeline_at(
            SHADER,
            &[
                wgpu::BufferBindingType::Uniform,
                read,
                read,
                write,
                write,
                write,
            ],
            entry,
        );
        let entries = [
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: locus.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Buffer(buffers[0].clone()),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Buffer(buffers[1].clone()),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Buffer(buffers[2].clone()),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Buffer(buffers[3].clone()),
            },
        ];
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(entry),
            layout: &pipeline.layout,
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(match entry {
                "reach" => "input_gamut_reach",
                "fft" => "input_gamut_fft",
                "frequency_filter" => "input_gamut_frequency_filter",
                "hull_scale" => "input_gamut_hull_scale",
                "hull" => "input_gamut_hull",
                "cmax" => "input_gamut_cmax",
                "coordinates" => "input_gamut_coordinates",
                "gather_cmax" => "input_gamut_gather_cmax",
                "oklch_coordinates" => "input_gamut_oklch_coordinates",
                "gather_rgb" => "input_gamut_gather_rgb",
                "interpolate" => "input_gamut_interpolate",
                "neutral_gather" => "input_gamut_neutral_gather",
                "neutral_reduce" => "input_gamut_neutral_reduce",
                _ => "input_gamut_other",
            }),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &group, &[]);
        dispatch_linear(&mut pass, n);
    }
}
