//! The Dye grain field on the device.
//!
//! The field is the same three-octave value-noise process as the host
//! reference: global-coordinate hashing, smoothstep interpolation and a fixed
//! unit-variance scale. The device writes the shared and record fields, then
//! the existing apply pass mixes them and draws the conditional count. No
//! Gaussian taps, halo, or device-side reduction are needed.

use super::*;
use crate::gpu_helpers::{f32_to_scalars, scalars_to_f32};
use spektrafilm_backend::dye_params::law;
use spektrafilm_math::precision::{ZERO, from_f64, to_f64};

const VALUE_WGSL: &str = include_str!("dye_value.wgsl");
const APPLY_WGSL: &str = include_str!("dye_law_apply.wgsl");
pub(super) const READOUT_WGSL: &str = include_str!("dye_law_readout.wgsl");

/// Bytes one band needs: density and output RGB plus four single-channel
/// fields.
pub(super) fn band_bytes(pixels: u64) -> u64 {
    pixels * 40
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ValueParams {
    w: u32,
    h: u32,
    seed: u32,
    ox: u32,
    oy: u32,
    monochrome: u32,
    pad0: u32,
    pad1: u32,
    base_px: f32,
    field_scale: f32,
    pad2: f32,
    pad3: f32,
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
    monochrome: u32,
    strength: [f32; 4],
}

fn resample_readout(
    film: &ImageBuf,
    source_pitch_um: f64,
    grid: &spektrafilm_backend::dye_params::DyeReadoutGrid,
) -> ImageBuf {
    let sw = film.width as usize;
    let sh = film.height as usize;
    let dw = grid.width as usize;
    let dh = grid.height as usize;
    let mut out = vec![ZERO; dw * dh * 3];
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
                                *slot += w * to_f64(film.data[base + c]);
                            }
                            weight += w;
                        }
                        sx += 1;
                    }
                    sy += 1;
                }
                let norm = if weight > 0.0 { weight } else { 1.0 };
                for c in 0..3 {
                    out[(y * dw + x) * 3 + c] = from_f64(acc[c] / norm);
                }
            } else {
                let sx = ((fx0 + fx1) * 0.5 - 0.5).clamp(0.0, (sw - 1) as f64);
                let sy = ((fy0 + fy1) * 0.5 - 0.5).clamp(0.0, (sh - 1) as f64);
                let x0 = sx.floor() as usize;
                let y0 = sy.floor() as usize;
                let x1 = (x0 + 1).min(sw - 1);
                let y1 = (y0 + 1).min(sh - 1);
                let wx = from_f64(sx - x0 as f64);
                let wy = from_f64(sy - y0 as f64);
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
pub(super) struct LawPipelines {
    value: CachedPipelineRef,
    apply: CachedPipelineRef,
}

impl WgpuBackend {
    pub(super) fn dye_law_pipelines(&self) -> LawPipelines {
        LawPipelines {
            value: self.cached_pipeline_at(
                VALUE_WGSL,
                &[
                    wgpu::BufferBindingType::Uniform,
                    wgpu::BufferBindingType::Storage { read_only: false },
                    wgpu::BufferBindingType::Storage { read_only: false },
                    wgpu::BufferBindingType::Storage { read_only: false },
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

/// One window of the field: interleaved density and output, plus the shared
/// field and three record fields. Each plane has its own buffer because one
/// dispatch binds some planes read-only and others read-write.
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
        let mut buffers = Vec::with_capacity(6);
        for index in 0..6u64 {
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

/// Where a band sits in the image, and the value-noise sampling pitch.
pub(super) struct LawGeom {
    pub origin_x: u32,
    pub origin_y: u32,
    pub seed: u64,
    pub expectation: bool,
    pub monochrome: bool,
    pub white: f64,
    pub event_area_um2: f64,
    pub pitch_um: f64,
    pub base_px: f64,
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

    /// Encode the value-noise field and the density law for one band.
    pub(super) fn encode_law_band(
        &self,
        encoder: &mut ObservedEncoder,
        band: &LawBand,
        geom: &LawGeom,
        pipes: &LawPipelines,
    ) {
        let (w, h) = (band.cols, band.rows);
        let groups = (w.div_ceil(64), h, 1);
        let params = ValueParams {
            w,
            h,
            seed: geom.seed as u32,
            ox: geom.origin_x,
            oy: geom.origin_y,
            monochrome: u32::from(geom.monochrome),
            pad0: 0,
            pad1: 0,
            base_px: geom.base_px as f32,
            field_scale: law::VALUE_FIELD_SCALE as f32,
            pad2: 0.0,
            pad3: 0.0,
        };
        let u = self.uniform("dye_value", bytemuck::bytes_of(&params));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dye_value"),
            layout: &pipes.value.layout,
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
                    resource: band.plane(3),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dye_value"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipes.value.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }

        let params = ApplyParams {
            w,
            h,
            seed: geom.seed as u32,
            expectation: u32::from(geom.expectation),
            white: geom.white as f32,
            rho: law::RHO as f32,
            taper_peak: law::taper_peak() as f32,
            taper_floor: law::TAPER_FLOOR as f32,
            dmax: law::DMAX as f32,
            uniformity: law::UNIFORMITY as f32,
            q: geom.event_area_um2 as f32,
            area: (geom.pitch_um * geom.pitch_um) as f32,
            fog: law::FOG as f32,
            ox: geom.origin_x,
            oy: geom.origin_y,
            monochrome: u32::from(geom.monochrome),
            strength: [
                law::STRENGTH[0] as f32,
                law::STRENGTH[1] as f32,
                law::STRENGTH[2] as f32,
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
                    resource: band.plane(0),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: band.plane(1),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: band.plane(2),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: band.plane(3),
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
        params: &DyeGpuParams,
    ) -> Result<ImageBuf, String> {
        let (width, height) = (image.width, image.height);
        let pixels = u64::from(width) * u64::from(height);
        if pixels == 0 {
            return Err("Dye grain law: the source density is empty".into());
        }
        let pitch_um = params.source_pitch_um;
        if !(pitch_um.is_finite() && pitch_um > 0.0) {
            return Err("Dye grain law: the source pitch must be positive".into());
        }
        if !(params.length_scale.is_finite() && params.length_scale > 0.0) {
            return Err("Dye grain law: the correlation-length scale must be positive".into());
        }
        let grid = &params.grid;
        let base_px = law::VALUE_BASE_UM * params.length_scale / pitch_um;
        let upload = scalars_to_f32(&image.data);
        let band = LawBand::new(self, width, height, "dye_law_field", Some(&upload));
        let geom = LawGeom {
            origin_x: 0,
            origin_y: 0,
            seed: params.seed,
            expectation: params.expectation,
            monochrome: params.monochrome,
            white: params.white,
            event_area_um2: params.event_area_um2,
            pitch_um,
            base_px,
        };
        let pipes = self.dye_law_pipelines();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.encode_law_band(&mut encoder, &band, &geom, &pipes);
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
        let data: Vec<f32> = self.pipe.read_mapped(&staging, bytes)?;
        let film = ImageBuf {
            data: f32_to_scalars(data),
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
