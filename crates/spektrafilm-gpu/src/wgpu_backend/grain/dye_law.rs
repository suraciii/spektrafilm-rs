//! The delivered Dye grain law on the device.
//!
//! The law is a band-limited correlated field whose amplitude follows the record
//! density, plus an event count:
//!
//! ```text
//! D = t * (1 + sigma(t)/t * u) + count noise,      E[D] = t
//! u = normalise( sum_i w_i * G_sigma_i * white - G_hp * sum_i w_i * G_sigma_i * white )
//! ```
//!
//! Every pass is a separable Gaussian over white noise generated from the pixel
//! coordinates, so no noise buffer is stored. The normalisation is analytical:
//! the effective kernel is separable, so its L2 norm is computed on the host from
//! the one-dimensional taps and the device only applies a scalar.
//!
//! A band is a window of the image plus the halo the filter needs. The noise is
//! keyed on **global** coordinates, so overlapping halos reproduce the same field
//! and bands join without seams.

use super::*;

const BLUR_WGSL: &str = include_str!("dye_law_blur.wgsl");
const COMBINE_WGSL: &str = include_str!("dye_law_combine.wgsl");
const SUBTRACT_WGSL: &str = include_str!("dye_law_subtract.wgsl");
const APPLY_WGSL: &str = include_str!("dye_law_apply.wgsl");
pub(super) const RESAMPLE_WGSL: &str = include_str!("dye_law_resample.wgsl");

/// Octave diameters in micrometres and their relative weights.
const OCTAVES: [(f64, f64); 3] = [(4.5, 1.0), (9.0, 1.0), (18.0, 0.5)];
/// Band limit of the field, in micrometres.
const HIGHPASS_UM: f64 = 12.0;

/// Relative density amplitude per record.
pub(super) const STRENGTH: [f64; 3] = [1.0, 1.0, 1.18];
/// Shared fraction of the field between records.
pub(super) const RHO: f64 = 0.92;
/// Relative density amplitude at the taper peak.
pub(super) const WHITE: f64 = 0.035;
/// Event area of one developed silver cluster, in square micrometres.
pub(super) const Q_UM2: f64 = 0.0015;
/// Dark-end event baseline, in density units. Uncalibrated relative setting.
pub(super) const FOG: f64 = 0.15;
pub(super) const DMAX: f64 = 1.2;
pub(super) const UNIFORMITY: f64 = 0.97;
pub(super) const TAPER_FLOOR: f64 = 0.18;

/// Halo radius in micrometres that a band needs on each side.
pub(super) fn halo_um() -> f64 {
    let widest = OCTAVES
        .iter()
        .map(|(diameter, _)| diameter / 2.0)
        .fold(0.0f64, f64::max);
    3.0 * widest + 3.0 * HIGHPASS_UM
}

/// Bytes one band of `pixels` pixels needs: density and output in RGB plus ten
/// single-channel fields.
pub(super) fn band_bytes(pixels: u64) -> u64 {
    // Two interleaved windows and eleven single-channel planes.
    pixels * 68
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurParams {
    w: u32,
    h: u32,
    radius: u32,
    axis: u32,
    stream: u32,
    seed: u32,
    use_noise: u32,
    pad: u32,
    ox: u32,
    oy: u32,
    pad2: u32,
    pad3: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CombineParams {
    w: u32,
    h: u32,
    pad0: u32,
    pad1: u32,
    w0: f32,
    w1: f32,
    w2: f32,
    pad2: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SubtractParams {
    w: u32,
    h: u32,
    pad0: u32,
    pad1: u32,
    scale: f32,
    pad2: f32,
    pad3: f32,
    pad4: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ApplyParams {
    w: u32,
    h: u32,
    seed: u32,
    expectation: u32,
    white: f32,
    rho: f32,
    taper_peak: f32,
    taper_floor: f32,
    dmax: f32,
    uniformity: f32,
    q: f32,
    area: f32,
    fog: f32,
    ox: u32,
    oy: u32,
    pad2: f32,
    strength: [f32; 4],
}

/// Normalised Gaussian taps and their radius.
fn taps_for(sigma_px: f64) -> (Vec<f32>, u32) {
    let sigma = sigma_px.max(0.05);
    let radius = (3.0 * sigma).ceil().clamp(1.0, 48.0) as u32;
    let mut taps = Vec::with_capacity((2 * radius + 1) as usize);
    let mut sum = 0.0;
    for t in 0..=(2 * radius) {
        let x = t as f64 - radius as f64;
        let v = (-(x * x) / (2.0 * sigma * sigma)).exp();
        taps.push(v);
        sum += v;
    }
    for v in taps.iter_mut() {
        *v /= sum;
    }
    (taps.into_iter().map(|v| v as f32).collect(), radius)
}

fn l2(v: &[f32]) -> f64 {
    v.iter().map(|&x| (x as f64) * (x as f64)).sum()
}

/// Convolve two centred one-dimensional kernels.
fn convolve(a: &[f32], b: &[f32]) -> Vec<f32> {
    let out = a.len() + b.len() - 1;
    let mut v = vec![0.0f64; out];
    for (i, &x) in a.iter().enumerate() {
        for (j, &y) in b.iter().enumerate() {
            v[i + j] += x as f64 * y as f64;
        }
    }
    v.into_iter().map(|x| x as f32).collect()
}

/// Dot product of two centred kernels aligned on their centres.
fn dot(a: &[f32], b: &[f32]) -> f64 {
    let n = a.len().min(b.len());
    let off_a = (a.len() - n) / 2;
    let off_b = (b.len() - n) / 2;
    (0..n)
        .map(|i| a[off_a + i] as f64 * b[off_b + i] as f64)
        .sum()
}

/// Reciprocal L2 norm of the effective separable kernel of the whole field.
fn field_scale(taps: [&[f32]; 3], hp: &[f32]) -> f64 {
    let norm = OCTAVES.iter().map(|(_, w)| w * w).sum::<f64>().sqrt();
    let mut var = 0.0;
    for (i, (_, weight)) in OCTAVES.iter().enumerate() {
        let k = taps[i];
        let kh = convolve(k, hp);
        let a = l2(k); // <K*K, K*K>
        let b = dot(k, &kh); // <K*K, (K*K)*(H*H)>
        let c = l2(&kh); // <(K*K)*(H*H), (K*K)*(H*H)>
        let v = a * a - 2.0 * b * b + c * c;
        let w = weight / norm;
        var += w * w * v;
    }
    if var <= 0.0 { 1.0 } else { 1.0 / var.sqrt() }
}

/// Bilinear readout of the developed density at the grid's cell centres.
fn resample_density(src: &[f32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; dw * dh * 3];
    let fx = sw as f64 / dw as f64;
    let fy = sh as f64 / dh as f64;
    for y in 0..dh {
        let sy = ((y as f64 + 0.5) * fy - 0.5).clamp(0.0, (sh - 1) as f64);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let wy = (sy - y0 as f64) as f32;
        for x in 0..dw {
            let sx = ((x as f64 + 0.5) * fx - 0.5).clamp(0.0, (sw - 1) as f64);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let wx = (sx - x0 as f64) as f32;
            for c in 0..3 {
                let p00 = src[(y0 * sw + x0) * 3 + c];
                let p10 = src[(y0 * sw + x1) * 3 + c];
                let p01 = src[(y1 * sw + x0) * 3 + c];
                let p11 = src[(y1 * sw + x1) * 3 + c];
                let top = p00 + (p10 - p00) * wx;
                let bottom = p01 + (p11 - p01) * wx;
                out[(y * dw + x) * 3 + c] = top + (bottom - top) * wy;
            }
        }
    }
    out
}

/// Taper peak of the derived granularity shape, as the host model normalises by.
fn taper_peak() -> f64 {
    (0..=512)
        .map(|i| {
            let p = i as f64 / 512.0;
            let sat = 1.0 - p * UNIFORMITY;
            (sat * p * (1.0 - p)).max(0.0).sqrt()
        })
        .fold(0.0f64, f64::max)
        .max(1e-6)
}

pub(super) struct LawPipelines {
    pub(super) blur: CachedPipelineRef,
    combine: CachedPipelineRef,
    subtract: CachedPipelineRef,
    apply: CachedPipelineRef,
}

impl WgpuBackend {
    pub(super) fn dye_law_pipelines(&self) -> LawPipelines {
        LawPipelines {
            blur: self.cached_pipeline_at(
                BLUR_WGSL,
                &[
                    wgpu::BufferBindingType::Uniform,
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: false },
                ],
                "main",
            ),
            combine: self.cached_pipeline_at(
                COMBINE_WGSL,
                &[
                    wgpu::BufferBindingType::Uniform,
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: false },
                ],
                "main",
            ),
            subtract: self.cached_pipeline_at(
                SUBTRACT_WGSL,
                &[
                    wgpu::BufferBindingType::Uniform,
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: false },
                ],
                "main",
            ),
            apply: self.cached_pipeline_at(
                APPLY_WGSL,
                &[
                    wgpu::BufferBindingType::Uniform,
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: true },
                    wgpu::BufferBindingType::Storage { read_only: false },
                ],
                "main",
            ),
        }
    }
}

/// One window of the field: interleaved density and output, four field planes and
/// six scratch planes. Each plane is its own buffer, because one dispatch binds
/// some planes read-only and others read-write and a single buffer may not be both.
pub(super) struct LawBand {
    buffers: Vec<wgpu::Buffer>,
    cols: u32,
    rows: u32,
}

impl LawBand {
    pub(super) fn new(
        backend: &WgpuBackend,
        cols: u32,
        rows: u32,
        label: &str,
        contents: Option<&[f32]>,
    ) -> LawBand {
        let pixels = u64::from(cols) * u64::from(rows);
        let usage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST;
        let mut buffers = Vec::with_capacity(13);
        for index in 0..13u64 {
            let size = if index < 2 { pixels * 12 } else { pixels * 4 };
            if index == 0 {
                if let Some(data) = contents {
                    buffers.push(
                        backend
                            .device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some(label),
                                contents: bytemuck::cast_slice(data),
                                usage,
                            }),
                    );
                    continue;
                }
            }
            buffers.push(backend.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            }));
        }
        LawBand { buffers, cols, rows }
    }

    /// The interleaved density window, also the copy destination of the band.
    pub(super) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffers[0]
    }

    /// The interleaved output window.
    pub(super) fn out_buffer(&self) -> &wgpu::Buffer {
        &self.buffers[1]
    }

    pub(super) fn cols(&self) -> u32 {
        self.cols
    }

    pub(super) fn rows(&self) -> u32 {
        self.rows
    }

    fn plane(&self, index: u64) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.buffers[2 + index as usize],
            offset: 0,
            size: None,
        })
    }

    fn rgb(&self, which: u64) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.buffers[which as usize],
            offset: 0,
            size: None,
        })
    }
}

/// Where a band sits in the image, and the sampling pitch of its field.
pub(super) struct LawGeom {
    pub origin_x: u32,
    pub origin_y: u32,
    pub pitch_um: f64,
    pub seed: u64,
    pub expectation: bool,
}

pub(super) struct LawTaps {
    pub(super) octaves: [(Vec<f32>, u32); 3],
    pub(super) highpass: (Vec<f32>, u32),
    pub(super) scale: f64,
}

pub(super) fn law_taps(pitch_um: f64) -> LawTaps {
    let octaves: [(Vec<f32>, u32); 3] = std::array::from_fn(|i| {
        let sigma_px = OCTAVES[i].0 / 2.0 / pitch_um;
        taps_for(sigma_px)
    });
    let highpass = taps_for(HIGHPASS_UM / pitch_um);
    let scale = field_scale(
        [&octaves[0].0, &octaves[1].0, &octaves[2].0],
        &highpass.0,
    );
    LawTaps {
        octaves,
        highpass,
        scale,
    }
}

impl WgpuBackend {
    fn uniform(&self, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytes,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
    }

    fn tap_buffer(&self, taps: &[f32]) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dye_law_taps"),
                contents: bytemuck::cast_slice(taps),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
    }

    /// Fill a band's density window from the flat source at the grid's mapping.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn encode_law_resample(
        &self,
        encoder: &mut ObservedEncoder,
        pipe: &CachedPipelineRef,
        source: &wgpu::Buffer,
        destination: &wgpu::Buffer,
        cols: u32,
        rows: u32,
        origin: [u32; 2],
        source_dims: [u32; 2],
        grid_dims: [u32; 2],
    ) {
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct P {
            cols: u32,
            rows: u32,
            origin_x: u32,
            origin_y: u32,
            source_w: u32,
            source_h: u32,
            grid_w: u32,
            grid_h: u32,
            fx: f32,
            fy: f32,
            pad0: f32,
            pad1: f32,
        }
        let params = P {
            cols,
            rows,
            origin_x: origin[0],
            origin_y: origin[1],
            source_w: source_dims[0],
            source_h: source_dims[1],
            grid_w: grid_dims[0],
            grid_h: grid_dims[1],
            fx: source_dims[0] as f32 / grid_dims[0] as f32,
            fy: source_dims[1] as f32 / grid_dims[1] as f32,
            pad0: 0.0,
            pad1: 0.0,
        };
        let u = self.uniform("dye_law_resample", bytemuck::bytes_of(&params));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dye_law_resample"),
            layout: &pipe.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: u.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: source.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: destination.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("dye_law_resample"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipe.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(cols.div_ceil(64), rows, 1);
    }

    /// Encode the whole law for one band.
    pub(super) fn encode_law_band(
        &self,
        encoder: &mut ObservedEncoder,
        band: &LawBand,
        geom: &LawGeom,
        pipes: &LawPipelines,
        taps: &LawTaps,
    ) {
        let (w, h) = (band.cols, band.rows);
        let groups = (w.div_ceil(64), h, 1);
        let norm = OCTAVES.iter().map(|(_, w)| w * w).sum::<f64>().sqrt();
        for field in 0..4u64 {
            for (i, (taps, radius)) in taps.octaves.iter().enumerate() {
                for axis in 0..2u32 {
                    let params = BlurParams {
                        w,
                        h,
                        radius: *radius,
                        axis,
                        stream: field as u32 * 8 + i as u32,
                        seed: geom.seed as u32,
                        use_noise: u32::from(axis == 0),
                        pad: 0,
                        ox: geom.origin_x,
                        oy: geom.origin_y,
                        pad2: 0,
                        pad3: 0,
                    };
                    let u = self.uniform("dye_law_blur", bytemuck::bytes_of(&params));
                    let t = self.tap_buffer(taps);
                    // Octave planes 0..2 and the horizontal target, plane 3.
                    let (source, target) = if axis == 0 {
                        (band.plane(0), band.plane(3))
                    } else {
                        (band.plane(3), band.plane(i as u64))
                    };
                    let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("dye_law_blur"),
                        layout: &pipes.blur.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: u.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: t.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: source,
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: target,
                            },
                        ],
                    });
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("dye_law_blur"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&pipes.blur.pipeline);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.dispatch_workgroups(groups.0, groups.1, 1);
                }
            }
            let params = CombineParams {
                w,
                h,
                pad0: 0,
                pad1: 0,
                w0: (OCTAVES[0].1 / norm) as f32,
                w1: (OCTAVES[1].1 / norm) as f32,
                w2: (OCTAVES[2].1 / norm) as f32,
                pad2: 0.0,
            };
            let u = self.uniform("dye_law_combine", bytemuck::bytes_of(&params));
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dye_law_combine"),
                layout: &pipes.combine.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: u.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: band.plane(0),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: band.plane(1),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: band.plane(2),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: band.plane(9),
                    },
                ],
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("dye_law_combine"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipes.combine.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
            // Band limit: blur the combined field, plane 9, then subtract.
            let hp_taps = self.tap_buffer(&taps.highpass.0);
            for axis in 0..2u32 {
                let params = BlurParams {
                    w,
                    h,
                    radius: taps.highpass.1,
                    axis,
                    stream: 0,
                    seed: geom.seed as u32,
                    use_noise: 0,
                    pad: 0,
                    ox: geom.origin_x,
                    oy: geom.origin_y,
                    pad2: 0,
                    pad3: 0,
                };
                let u = self.uniform("dye_law_highpass", bytemuck::bytes_of(&params));
                let target = if axis == 0 { band.plane(3) } else { band.plane(10) };
                let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("dye_law_highpass"),
                    layout: &pipes.blur.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: u.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: hp_taps.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: band.plane(9),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: target,
                        },
                    ],
                });
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("dye_law_highpass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipes.blur.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
            let params = SubtractParams {
                w,
                h,
                pad0: 0,
                pad1: 0,
                scale: taps.scale as f32,
                pad2: 0.0,
                pad3: 0.0,
                pad4: 0.0,
            };
            let u = self.uniform("dye_law_subtract", bytemuck::bytes_of(&params));
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dye_law_subtract"),
                layout: &pipes.subtract.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: u.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: band.plane(9),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: band.plane(10),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: band.plane(4 + field),
                    },
                ],
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("dye_law_subtract"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipes.subtract.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
        }
        let params = ApplyParams {
            w,
            h,
            seed: geom.seed as u32,
            expectation: u32::from(geom.expectation),
            white: WHITE as f32,
            rho: RHO as f32,
            taper_peak: taper_peak() as f32,
            taper_floor: TAPER_FLOOR as f32,
            dmax: DMAX as f32,
            uniformity: UNIFORMITY as f32,
            q: Q_UM2 as f32,
            area: (geom.pitch_um * geom.pitch_um) as f32,
            fog: FOG as f32,
            ox: geom.origin_x,
            oy: geom.origin_y,
            pad2: 0.0,
            strength: [
                STRENGTH[0] as f32,
                STRENGTH[1] as f32,
                STRENGTH[2] as f32,
                0.0,
            ],
        };
        let u = self.uniform("dye_law_apply", bytemuck::bytes_of(&params));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dye_law_apply"),
            layout: &pipes.apply.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: u.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: band.rgb(0),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: band.plane(4),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: band.plane(5),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: band.plane(6),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: band.plane(7),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: band.rgb(1),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dye_law_apply"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipes.apply.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
    }

    /// The whole law as one band, for the standalone `dye_field` entry point.
    pub(in crate::wgpu_backend) fn dye_law_field(
        &self,
        image: &ImageBuf,
        pitch_um: f64,
        grid_width: u32,
        grid_height: u32,
        seed: u64,
        expectation: bool,
    ) -> Result<ImageBuf, String> {
        let pixels = (grid_width as u64) * (grid_height as u64);
        if pixels == 0 {
            return Err("Dye grain law: the readout grid is empty".into());
        }
        if image.width == 0 || image.height == 0 {
            return Err("Dye grain law: the source density is empty".into());
        }
        if !(pitch_um.is_finite() && pitch_um > 0.0) {
            return Err("Dye grain law: the source pitch must be positive".into());
        }
        // Readout geometry independent of the source grid, exactly as the host law
        // resamples: bilinear at the cell centre, so a preview scale or a fractional
        // cell is defined rather than only the 1:1 case.
        let density = if (grid_width, grid_height) == (image.width, image.height) {
            image.data.clone()
        } else {
            resample_density(
                &image.data,
                image.width as usize,
                image.height as usize,
                grid_width as usize,
                grid_height as usize,
            )
        };
        let band = LawBand::new(
            self,
            grid_width,
            grid_height,
            "dye_law_field",
            Some(&density),
        );
        let geom = LawGeom {
            origin_x: 0,
            origin_y: 0,
            pitch_um,
            seed,
            expectation,
        };
        let pipes = self.dye_law_pipelines();
        let taps = law_taps(pitch_um);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.encode_law_band(&mut encoder, &band, &geom, &pipes, &taps);
        self.queue.submit(Some(encoder.finish()));
        let bytes = pixels * 12;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_law_readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(band.out_buffer(), 0, &staging, 0, bytes);
        self.queue.submit(Some(encoder.finish()));
        let data: Vec<f32> = self.pipe.read_mapped(&staging, bytes);
        Ok(ImageBuf {
            data,
            width: grid_width,
            height: grid_height,
        })
    }
}
