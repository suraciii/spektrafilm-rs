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
pub(super) const READOUT_WGSL: &str = include_str!("dye_law_readout.wgsl");

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

/// Per-octave gains and the band-limited scale.
///
/// The host law standardises every octave and then the band-limited field, so the
/// device must land on the same amplitude mix. A Gaussian discretised below one pixel
/// passes only a fraction of the variance, and that fraction differs per octave, so a
/// single scale cannot reproduce the host spectrum. Filtering a white-noise patch here
/// with the very taps the shader uses — same clamping, same discretisation — measures
/// each fraction and the final scale without any device-side reduction.
pub(super) struct LawNorm {
    pub(super) gains: [f64; 3],
    pub(super) scale: f64,
}

fn law_norm(taps: [&[f32]; 3], hp: &[f32]) -> LawNorm {
    const N: usize = 128;
    let norm = OCTAVES.iter().map(|(_, w)| w * w).sum::<f64>().sqrt();
    let mut state = 0x5eed_1234_9abc_def0u64;
    // One independent draw per octave, because that is what the shader has: reusing one
    // draw would leave the octaves correlated and misprice the combined amplitude.
    let mut noise: Vec<Vec<f32>> = Vec::with_capacity(OCTAVES.len());
    for _ in 0..OCTAVES.len() {
        let mut v: Vec<f32> = Vec::with_capacity(N * N);
        for _ in 0..(N * N) {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u1 = ((state >> 11) as f64 + 0.5) / 9007199254740992.0;
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u2 = ((state >> 11) as f64) / 9007199254740992.0;
            v.push(((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32);
        }
        noise.push(v);
    }
    // Periodic filtering: the quantity being measured is the interior amplitude of a
    // stationary field, and a clamped border would let the border dominate exactly when
    // the filter is narrow enough that the interior answer is a small residual.
    let blur = |field: &[f32], taps: &[f32]| -> Vec<f32> {
        let radius = (taps.len() / 2) as i64;
        let wrap = |i: i64| i.rem_euclid(N as i64) as usize;
        let mut tmp = vec![0.0f32; N * N];
        for y in 0..N {
            for x in 0..N {
                let mut acc = 0.0f32;
                for (t, &k) in taps.iter().enumerate() {
                    let sx = wrap(x as i64 + t as i64 - radius);
                    acc += k * field[y * N + sx];
                }
                tmp[y * N + x] = acc;
            }
        }
        let mut out = vec![0.0f32; N * N];
        for y in 0..N {
            for x in 0..N {
                let mut acc = 0.0f32;
                for (t, &k) in taps.iter().enumerate() {
                    let sy = wrap(y as i64 + t as i64 - radius);
                    acc += k * tmp[sy * N + x];
                }
                out[y * N + x] = acc;
            }
        }
        out
    };
    // Interior only: the clamp at the patch border leaves the field weaker there, so
    // averaging the border in would understate the amplitude a large render actually
    // gets and would make the scale depend on the patch size.
    let margin = 8usize;
    let sd = |v: &[f32]| {
        let mut sum = 0.0f64;
        let mut count = 0.0f64;
        for y in margin..(N - margin) {
            for x in margin..(N - margin) {
                sum += v[y * N + x] as f64;
                count += 1.0;
            }
        }
        let mean = sum / count;
        let mut var = 0.0f64;
        for y in margin..(N - margin) {
            for x in margin..(N - margin) {
                var += (v[y * N + x] as f64 - mean).powi(2);
            }
        }
        (var / count).sqrt().max(1e-9)
    };
    // The shader blurs raw noise and never standardises an octave on the way, so the
    // gain each octave needs is measured here rather than assumed.
    let mut gains = [1.0f64; 3];
    let mut field = vec![0.0f32; N * N];
    for (i, (_, weight)) in OCTAVES.iter().enumerate() {
        let v = blur(&noise[i], taps[i]);
        gains[i] = 1.0 / sd(&v);
        let k = ((weight / norm) * gains[i]) as f32;
        for (a, b) in field.iter_mut().zip(v.iter()) {
            *a += k * b;
        }
    }
    let big = blur(&field, hp);
    for (a, b) in field.iter_mut().zip(big.iter()) {
        *a -= *b;
    }
    LawNorm {
        gains,
        scale: 1.0 / sd(&field),
    }
}

/// Readout of the developed density at the grid's film-coordinate windows.
///
/// This is the same readout the host model uses: a cell finer than the lattice
/// interpolates its centre, a coarser one integrates the lattice cells it covers, so
/// a window and a full-frame read agree where they overlap.
fn resample_readout(
    film: &ImageBuf,
    source_pitch_um: f64,
    grid: &spektrafilm_backend::dye_params::DyeReadoutGrid,
) -> ImageBuf {
    let sw = film.width as usize;
    let sh = film.height as usize;
    let dw = grid.width as usize;
    let dh = grid.height as usize;
    let mut out = vec![0.0f32; dw * dh * 3];
    for y in 0..dh {
        let fy0 = (grid.origin_um[1] + y as f64 * grid.pitch_um[1]) / source_pitch_um;
        let fy1 = fy0 + grid.pitch_um[1] / source_pitch_um;
        for x in 0..dw {
            let fx0 = (grid.origin_um[0] + x as f64 * grid.pitch_um[0]) / source_pitch_um;
            let fx1 = fx0 + grid.pitch_um[0] / source_pitch_um;
            if fx1 - fx0 >= 1.0 || fy1 - fy0 >= 1.0 {
                let mut acc = [0.0f64; 3];
                let mut weight = 0.0f64;
                let mut sy = fy0.floor() as i64;
                while (sy as f64) < fy1 {
                    let wy = (fy1.min(sy as f64 + 1.0) - fy0.max(sy as f64)).max(0.0);
                    let mut sx = fx0.floor() as i64;
                    while (sx as f64) < fx1 {
                        let wx = (fx1.min(sx as f64 + 1.0) - fx0.max(sx as f64)).max(0.0);
                        let w = wx * wy;
                        if w > 0.0 {
                            let syc = sy.clamp(0, sh as i64 - 1) as usize;
                            let sxc = sx.clamp(0, sw as i64 - 1) as usize;
                            let base = (syc * sw + sxc) * 3;
                            for (c, slot) in acc.iter_mut().enumerate() {
                                *slot += w * film.data[base + c] as f64;
                            }
                            weight += w;
                        }
                        sx += 1;
                    }
                    sy += 1;
                }
                let norm = if weight > 0.0 { weight } else { 1.0 };
                for c in 0..3 {
                    out[(y * dw + x) * 3 + c] = (acc[c] / norm) as f32;
                }
            } else {
                let sx = ((fx0 + fx1) * 0.5 - 0.5).clamp(0.0, (sw - 1) as f64);
                let sy = ((fy0 + fy1) * 0.5 - 0.5).clamp(0.0, (sh - 1) as f64);
                let x0 = sx.floor() as usize;
                let y0 = sy.floor() as usize;
                let x1 = (x0 + 1).min(sw - 1);
                let y1 = (y0 + 1).min(sh - 1);
                let wx = (sx - x0 as f64) as f32;
                let wy = (sy - y0 as f64) as f32;
                for c in 0..3 {
                    let p00 = film.data[(y0 * sw + x0) * 3 + c];
                    let p10 = film.data[(y0 * sw + x1) * 3 + c];
                    let p01 = film.data[(y1 * sw + x0) * 3 + c];
                    let p11 = film.data[(y1 * sw + x1) * 3 + c];
                    let top = p00 + (p10 - p00) * wx;
                    let bottom = p01 + (p11 - p01) * wx;
                    out[(y * dw + x) * 3 + c] = top + (bottom - top) * wy;
                }
            }
        }
    }
    ImageBuf {
        data: out,
        width: grid.width,
        height: grid.height,
    }
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
            if index == 0
                && let Some(data) = contents
            {
                buffers.push(backend.device.create_buffer_init(
                    &wgpu::util::BufferInitDescriptor {
                        label: Some(label),
                        contents: bytemuck::cast_slice(data),
                        usage,
                    },
                ));
                continue;
            }
            buffers.push(backend.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            }));
        }
        LawBand {
            buffers,
            cols,
            rows,
        }
    }

    /// The interleaved density window, also the copy destination of the band.
    pub(super) fn buffer(&self) -> &wgpu::Buffer {
        &self.buffers[0]
    }

    /// The interleaved output window.
    pub(super) fn out_buffer(&self) -> &wgpu::Buffer {
        &self.buffers[1]
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
    pub(super) norm: LawNorm,
}

pub(super) fn law_taps(pitch_um: f64) -> LawTaps {
    let octaves: [(Vec<f32>, u32); 3] = std::array::from_fn(|i| {
        let sigma_px = OCTAVES[i].0 / 2.0 / pitch_um;
        taps_for(sigma_px)
    });
    let highpass = taps_for(HIGHPASS_UM / pitch_um);
    let norm = law_norm([&octaves[0].0, &octaves[1].0, &octaves[2].0], &highpass.0);
    LawTaps {
        octaves,
        highpass,
        norm,
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

    /// Read a band's developed density out to the readout grid.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn encode_law_readout(
        &self,
        encoder: &mut ObservedEncoder,
        pipe: &CachedPipelineRef,
        source: &wgpu::Buffer,
        destination: &wgpu::Buffer,
        window: [u32; 4],
        band: [u32; 2],
        grid_origin: [u32; 2],
        grid_dims: [u32; 2],
        source_pitch: f64,
        grid_pitch: [f64; 2],
        film_origin: [f64; 2],
    ) {
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct P {
            cols: u32,
            rows: u32,
            window_x: u32,
            window_y: u32,
            window_w: u32,
            window_h: u32,
            grid_x: u32,
            grid_y: u32,
            source_pitch: f32,
            pitch_x: f32,
            pitch_y: f32,
            origin_x: f32,
            origin_y: f32,
            pad0: f32,
            pad1: f32,
            pad2: f32,
        }
        let params = P {
            cols: band[0],
            rows: band[1],
            window_x: window[0],
            window_y: window[1],
            window_w: window[2],
            window_h: window[3],
            grid_x: grid_origin[0],
            grid_y: grid_origin[1],
            source_pitch: source_pitch as f32,
            pitch_x: grid_pitch[0] as f32,
            pitch_y: grid_pitch[1] as f32,
            origin_x: film_origin[0] as f32,
            origin_y: film_origin[1] as f32,
            pad0: 0.0,
            pad1: 0.0,
            pad2: 0.0,
        };
        let _ = grid_dims;
        let u = self.uniform("dye_law_readout", bytemuck::bytes_of(&params));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dye_law_readout"),
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
            label: Some("dye_law_readout"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipe.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(band[0].div_ceil(64), band[1], 1);
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
                w0: ((OCTAVES[0].1 / norm) * taps.norm.gains[0]) as f32,
                w1: ((OCTAVES[1].1 / norm) * taps.norm.gains[1]) as f32,
                w2: ((OCTAVES[2].1 / norm) * taps.norm.gains[2]) as f32,
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
                // The vertical pass must read the horizontal result, plane 3, or the
                // band limit would only be one-dimensional.
                let target = if axis == 0 {
                    band.plane(3)
                } else {
                    band.plane(10)
                };
                let source = if axis == 0 {
                    band.plane(9)
                } else {
                    band.plane(3)
                };
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
                            resource: source,
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
                scale: taps.norm.scale as f32,
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
    ///
    /// The law is evaluated on the film lattice, exactly as the host model does, and
    /// the readout resamples the developed density afterwards. Realizing the field on
    /// the readout grid would make the draw depend on the crop or the preview scale.
    pub(in crate::wgpu_backend) fn dye_law_field(
        &self,
        image: &ImageBuf,
        pitch_um: f64,
        grid: &spektrafilm_backend::dye_params::DyeReadoutGrid,
        seed: u64,
        expectation: bool,
    ) -> Result<ImageBuf, String> {
        let (width, height) = (image.width, image.height);
        let pixels = u64::from(width) * u64::from(height);
        if pixels == 0 {
            return Err("Dye grain law: the source density is empty".into());
        }
        if !(pitch_um.is_finite() && pitch_um > 0.0) {
            return Err("Dye grain law: the source pitch must be positive".into());
        }
        let band = LawBand::new(self, width, height, "dye_law_field", Some(&image.data));
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
        let film = ImageBuf {
            data,
            width,
            height,
        };
        let whole = (grid.width, grid.height) == (width, height)
            && grid.origin_um == [0.0, 0.0]
            && grid.pitch_um == [pitch_um, pitch_um];
        if whole {
            return Ok(film);
        }
        Ok(resample_readout(&film, pitch_um, grid))
    }
}
