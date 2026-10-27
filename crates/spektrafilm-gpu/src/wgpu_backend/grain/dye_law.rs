//! The delivered Dye grain law on the device.
//!
//! The law is a continuous, band-limited correlated field whose amplitude follows
//! the record density, plus an event count:
//!
//! ```text
//! D = t * (1 + sigma(t)/t * u) + count noise,      E[D] = t
//! u = band-limit( sum_i w_i * G_sigma_i * white )
//! ```
//!
//! Every pass is a separable Gaussian over white noise that is generated from the
//! pixel coordinates, so nothing is stored between the noise and the filter. The
//! normalisation is analytical: the effective kernel is separable, so its L2 norm
//! is computed on the host from the one-dimensional taps and the device only
//! applies a scalar. No global reduction, and therefore no synchronisation point,
//! is introduced.

use super::*;

const BLUR_WGSL: &str = include_str!("dye_law_blur.wgsl");
const COMBINE_WGSL: &str = include_str!("dye_law_combine.wgsl");
const SUBTRACT_WGSL: &str = include_str!("dye_law_subtract.wgsl");
const APPLY_WGSL: &str = include_str!("dye_law_apply.wgsl");

/// Octave lengths in micrometres and their relative weights.
const OCTAVES: [(f64, f64); 3] = [(4.5, 1.0), (9.0, 1.0), (18.0, 0.5)];
/// Band limit of the field, in micrometres.
const HIGHPASS_UM: f64 = 12.0;

const STRENGTH: [f64; 3] = [1.0, 1.0, 1.18];
const RHO: f64 = 0.92;
const WHITE: f64 = 0.035;
const Q_UM2: f64 = 0.0015;
const FOG: f64 = 0.15;
const DMAX: f64 = 1.2;
const UNIFORMITY: f64 = 0.97;
const TAPER_FLOOR: f64 = 0.18;

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
    pad0: f32,
    pad1: f32,
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
        let c = l2(&kh); // <(K*K)*(H*H), ...>
        let v = a * a - 2.0 * b * b + c * c;
        let w = weight / norm;
        var += w * w * v;
    }
    if var <= 0.0 { 1.0 } else { 1.0 / var.sqrt() }
}

impl WgpuBackend {
    /// Run the Dye grain law on the device for a developed density image.
    pub(in crate::wgpu_backend) fn dye_law_field(
        &self,
        image: &ImageBuf,
        pitch_um: f64,
        width: u32,
        height: u32,
        seed: u64,
        expectation: bool,
    ) -> Result<ImageBuf, String> {
        let pixels = (width as usize) * (height as usize);
        if pixels == 0 || image.data.len() != pixels * 3 {
            return Err("Dye grain law: the field must match the density image".into());
        }
        let pitch = pitch_um;
        let oct_taps: Vec<(Vec<f32>, u32)> = OCTAVES
            .iter()
            .map(|(len, _)| taps_for(len / pitch / 2.0))
            .collect();
        let (hp_taps, hp_radius) = taps_for(HIGHPASS_UM / pitch);
        let scale = field_scale(
            [&oct_taps[0].0, &oct_taps[1].0, &oct_taps[2].0],
            &hp_taps,
        );

        let plane_bytes = (pixels * 4) as u64;
        let make_plane = |label: &str| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: plane_bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let density = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("dye_law_density"),
            contents: bytemuck::cast_slice(&image.data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let out = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dye_law_out"),
            size: (pixels * 12) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        // Scratch: three octaves, the combined field, one horizontal target and the
        // band-limit blur.
        let octave: Vec<wgpu::Buffer> = (0..3).map(|i| make_plane(&format!("dye_law_oct{i}"))).collect();
        let combined = make_plane("dye_law_combined");
        let tmp = make_plane("dye_law_tmp");
        let blurred = make_plane("dye_law_blurred");
        let fields: Vec<wgpu::Buffer> = (0..4).map(|i| make_plane(&format!("dye_law_field{i}"))).collect();

        let blur_pipe = self.cached_pipeline_at(
            BLUR_WGSL,
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            "main",
        );
        let combine_pipe = self.cached_pipeline_at(
            COMBINE_WGSL,
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            "main",
        );
        let subtract_pipe = self.cached_pipeline_at(
            SUBTRACT_WGSL,
            &[
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferBindingType::Storage { read_only: false },
            ],
            "main",
        );
        let apply_pipe = self.cached_pipeline_at(
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
        );

        let uniform = |bytes: &[u8]| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dye_law_uniform"),
                contents: bytes,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        };
        let tap_buffer = |taps: &[f32]| {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dye_law_taps"),
                contents: bytemuck::cast_slice(taps),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
        };

        let mut encoder = self.device.create_command_encoder(&Default::default());
        let groups = (width.div_ceil(64), height, 1);

        // One field = shared field at 0, the three records at 1..=3.
        for field in 0..4u32 {
            for (i, (taps, radius)) in oct_taps.iter().enumerate() {
                for axis in 0..2u32 {
                    let params = BlurParams {
                        w: width,
                        h: height,
                        radius: *radius,
                        axis,
                        stream: field * 8 + i as u32,
                        seed: seed as u32,
                        use_noise: u32::from(axis == 0),
                        pad: 0,
                    };
                    let u = uniform(bytemuck::bytes_of(&params));
                    let t = tap_buffer(taps);
                    let target = if axis == 0 { &tmp } else { &octave[i] };
                    let source = if axis == 0 { &tmp } else { &tmp };
                    let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("dye_law_blur"),
                        layout: &blur_pipe.layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: t.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 2, resource: source.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 3, resource: target.as_entire_binding() },
                        ],
                    });
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("dye_law_blur"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&blur_pipe.pipeline);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.dispatch_workgroups(groups.0, groups.1, 1);
                }
            }
            let norm = OCTAVES.iter().map(|(_, w)| w * w).sum::<f64>().sqrt();
            let params = CombineParams {
                w: width,
                h: height,
                pad0: 0,
                pad1: 0,
                w0: (OCTAVES[0].1 / norm) as f32,
                w1: (OCTAVES[1].1 / norm) as f32,
                w2: (OCTAVES[2].1 / norm) as f32,
                pad2: 0.0,
            };
            let u = uniform(bytemuck::bytes_of(&params));
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dye_law_combine"),
                layout: &combine_pipe.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: octave[0].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: octave[1].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: octave[2].as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: combined.as_entire_binding() },
                ],
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("dye_law_combine"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&combine_pipe.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
            // Band limit: blur the combined field, then subtract in its own pass.
            let hp_taps_buf = tap_buffer(&hp_taps);
            for axis in 0..2u32 {
                let params = BlurParams {
                    w: width,
                    h: height,
                    radius: hp_radius,
                    axis,
                    stream: 0,
                    seed: seed as u32,
                    use_noise: 0,
                    pad: 0,
                };
                let u = uniform(bytemuck::bytes_of(&params));
                let target = if axis == 0 { &tmp } else { &blurred };
                let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("dye_law_hp"),
                    layout: &blur_pipe.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: hp_taps_buf.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: combined.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 3, resource: target.as_entire_binding() },
                    ],
                });
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("dye_law_hp"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&blur_pipe.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
            let params = SubtractParams {
                w: width,
                h: height,
                pad0: 0,
                pad1: 0,
                scale: scale as f32,
                pad2: 0.0,
                pad3: 0.0,
                pad4: 0.0,
            };
            let u = uniform(bytemuck::bytes_of(&params));
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dye_law_subtract"),
                layout: &subtract_pipe.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: combined.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: blurred.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: fields[field as usize].as_entire_binding() },
                ],
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("dye_law_subtract"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&subtract_pipe.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
        }

        // taper peak: the same derived shape the host model normalises by.
        let grid: Vec<f64> = (0..=512).map(|i| i as f64 / 512.0).collect();
        let peak = grid
            .iter()
            .map(|&p| {
                let sat = 1.0 - p * UNIFORMITY;
                (sat * p * (1.0 - p)).max(0.0).sqrt()
            })
            .fold(0.0f64, f64::max)
            .max(1e-6);
        let params = ApplyParams {
            w: width,
            h: height,
            seed: seed as u32,
            expectation: u32::from(expectation),
            white: WHITE as f32,
            rho: RHO as f32,
            taper_peak: peak as f32,
            taper_floor: TAPER_FLOOR as f32,
            dmax: DMAX as f32,
            uniformity: UNIFORMITY as f32,
            q: Q_UM2 as f32,
            area: (pitch * pitch) as f32,
            fog: FOG as f32,
            pad0: 0.0,
            pad1: 0.0,
            pad2: 0.0,
            strength: [STRENGTH[0] as f32, STRENGTH[1] as f32, STRENGTH[2] as f32, 0.0],
        };
        let u = uniform(bytemuck::bytes_of(&params));
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dye_law_apply"),
            layout: &apply_pipe.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: density.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: fields[0].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: fields[1].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: fields[2].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: fields[3].as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: out.as_entire_binding() },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dye_law_apply"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&apply_pipe.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        self.queue.submit(Some(encoder.finish()));
        let data: Vec<f32> = self.pipe.read_mapped(&out, (pixels * 12) as u64);
        let mut result = ImageBuf::new(width, height);
        result.data.copy_from_slice(&data);
        Ok(result)
    }
}
