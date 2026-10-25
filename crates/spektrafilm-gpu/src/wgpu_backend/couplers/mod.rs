use super::blur::{DispatchJob, GaussianBuffers, GaussianScratch};
use super::grain::raster::RasterStorage;
use super::grain::v1::blur::{GrainBlurState, build_grain_blur_paged_with_scratch};
use super::paged::ImagePage;
use super::params::AddScaledParams;
use super::*;

/// DIR retains physically paged full-source rasters, never full-image bindings.
/// Pointwise work copies bounded windows; each spatial component uses the exact
/// reference FIR/IIR with recurrence carried across storage boundaries and true
/// image endpoints. The exponential law is the CPU's ordered three-fit mixture,
/// accumulated before its final Gaussian blend, not a single Gaussian surrogate.
/// Coefficient pages overlap one row without changing global interpolation.
pub(super) struct DirState {
    correction: Vec<ImagePage>,
    gaussian: Vec<ImagePage>,
    tail: Vec<ImagePage>,
    mix: Vec<ImagePage>,
    _blur_scratch: Vec<ImagePage>,
    density: Vec<ImagePage>,
    log_raw: Vec<ImagePage>,
    n_pixels: u32,
    matmul: WindowPass,
    blur_gaussian: Option<GrainBlurState>,
    tail_ops: Vec<(Option<GrainBlurState>, WindowPass)>,
    blend: Option<WindowPass>,
    subtract: WindowPass,
    density_curve_0: WindowPass,
}

/// The same storage windows are reused for every image partition. Separate
/// uniforms for the final short partition avoid mutable queue-side uniforms.
struct WindowPass {
    source: wgpu::Buffer,
    destination: wgpu::Buffer,
    pixels: u32,
    full: Vec<DispatchJob>,
    remainder: Vec<DispatchJob>,
    _coefficients: Vec<wgpu::Buffer>,
}

impl WindowPass {
    fn encode_window(
        &self,
        encoder: &mut ObservedEncoder,
        source: &[ImagePage],
        destination: &[ImagePage],
        n_pixels: u32,
        load_destination: bool,
        first: u32,
    ) {
        let count = self.pixels.min(n_pixels - first);
        let samples = u64::from(count) * 3;
        let first_sample = u64::from(first) * 3;
        source.read(encoder, first_sample, samples, &self.source, 0);
        if load_destination {
            destination.read(encoder, first_sample, samples, &self.destination, 0);
        }
        let jobs = if count == self.pixels {
            &self.full
        } else {
            &self.remainder
        };
        for job in jobs {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("dir_window"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&job.pipeline.pipeline);
            pass.set_bind_group(0, &job.bg, &[]);
            dispatch_linear(&mut pass, count);
        }
        destination.write(encoder, &self.destination, 0, first_sample, samples);
    }

    fn submit(
        &self,
        backend: &WgpuBackend,
        source: &[ImagePage],
        destination: &[ImagePage],
        n_pixels: u32,
        load_destination: bool,
    ) -> Result<(), String> {
        for first in (0..n_pixels).step_by(self.pixels as usize) {
            backend.check_cancelled()?;
            let mut encoder = backend.device.create_command_encoder(&Default::default());
            self.encode_window(
                &mut encoder,
                source,
                destination,
                n_pixels,
                load_destination,
                first,
            );
            backend.check_cancelled()?;
            backend.queue.submit(Some(encoder.finish()));
        }
        backend.check_cancelled()
    }
}

fn window_buffer(device: &ObservedDevice, bytes: u64, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn window_job<P: bytemuck::Pod>(
    device: &ObservedDevice,
    pipeline: &CachedPipelineRef,
    params: &P,
    source: &wgpu::Buffer,
    destination: &wgpu::Buffer,
    coefficients: &[&wgpu::Buffer],
) -> DispatchJob {
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dir_window_params"),
        contents: bytemuck::bytes_of(params),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let mut entries = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: source.as_entire_binding(),
        },
    ];
    for (index, buffer) in coefficients.iter().enumerate() {
        entries.push(wgpu::BindGroupEntry {
            binding: index as u32 + 2,
            resource: buffer.as_entire_binding(),
        });
    }
    entries.push(wgpu::BindGroupEntry {
        binding: coefficients.len() as u32 + 2,
        resource: destination.as_entire_binding(),
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("dir_window_bg"),
        layout: &pipeline.layout,
        entries: &entries,
    });
    DispatchJob {
        _params_buf: uniform,
        pipeline: pipeline.clone(),
        bg,
    }
}

fn pointwise<P: bytemuck::Pod>(
    backend: &WgpuBackend,
    n_pixels: u32,
    pixels: u32,
    shader: &'static str,
    params: impl Fn(u32) -> P,
) -> WindowPass {
    let device = &backend.device;
    let source = window_buffer(device, u64::from(pixels) * 12, "dir_window_source");
    let destination = window_buffer(device, u64::from(pixels) * 12, "dir_window_destination");
    let pipeline = backend.cached_pipeline(
        shader,
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let make = |count| {
        vec![window_job(
            device,
            &pipeline,
            &params(count),
            &source,
            &destination,
            &[],
        )]
    };
    let full = make(pixels);
    let remainder = if !n_pixels.is_multiple_of(pixels) {
        make(n_pixels % pixels)
    } else {
        Vec::new()
    };
    WindowPass {
        source,
        destination,
        pixels,
        full,
        remainder,
        _coefficients: Vec::new(),
    }
}

/// Non-finite or absent Langmuir knees are linear on the reference path.
fn langmuir_uniforms(k: Option<[f32; 3]>) -> ([f32; 4], [f32; 4]) {
    let mut k_out = [1.0_f32; 4];
    let mut mask = [0.0_f32; 4];
    if let Some(k) = k {
        for c in 0..3 {
            if k[c].is_finite() {
                k_out[c] = k[c];
                mask[c] = 1.0;
            }
        }
    }
    (k_out, mask)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_dir_state<A: RasterStorage + ?Sized, B: RasterStorage + ?Sized>(
    device: &ObservedDevice,
    dp: &spektrafilm_backend::DirCouplersGpuParams<'_>,
    width: u32,
    height: u32,
    buf_a: &A,
    buf_b: &B,
    gaussian_scratch: &mut GaussianScratch,
    backend: &WgpuBackend,
) -> Result<DirState, String> {
    build_dir_state_with_limit(
        device,
        dp,
        width,
        height,
        buf_a,
        buf_b,
        gaussian_scratch,
        backend,
        super::bounded::storage_binding_limit(backend)?,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_dir_state_with_limit<A: RasterStorage + ?Sized, B: RasterStorage + ?Sized>(
    device: &ObservedDevice,
    dp: &spektrafilm_backend::DirCouplersGpuParams<'_>,
    width: u32,
    height: u32,
    buf_a: &A,
    buf_b: &B,
    gaussian_scratch: &mut GaussianScratch,
    backend: &WgpuBackend,
    limit: u64,
) -> Result<DirState, String> {
    let count = u64::from(width) * u64::from(height);
    let n_pixels = u32::try_from(count).map_err(|_| "DIR image exceeds u32 pixel indexing")?;
    if n_pixels == 0 {
        return Err("DIR requires a nonempty raster".into());
    }
    let density = buf_a.pages();
    let log_raw = buf_b.pages();
    for pages in [&density, &log_raw] {
        let mut expected = 0_u32;
        for page in pages {
            if page.first_pixel != expected
                || page.n_pixels == 0
                || page.buffer.size() < u64::from(page.n_pixels) * 12
                || !page
                    .buffer
                    .usage()
                    .contains(wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST)
            {
                return Err("DIR requires contiguous COPY_SRC | COPY_DST raster pages".into());
            }
            expected = expected
                .checked_add(page.n_pixels)
                .ok_or("DIR raster page indexing overflow")?;
        }
        if expected < n_pixels {
            return Err("DIR raster pages do not cover the image".into());
        }
    }
    let pixels = (super::bounded::chunk_elements(limit, 12)? as u32).min(n_pixels);
    let transport = |label| {
        (0..n_pixels)
            .step_by(pixels as usize)
            .map(|first_pixel| {
                let n_pixels = pixels.min(n_pixels - first_pixel);
                ImagePage {
                    first_pixel,
                    n_pixels,
                    buffer: window_buffer(device, u64::from(n_pixels) * 12, label),
                }
            })
            .collect::<Vec<_>>()
    };
    let diffuse = dp.diffusion_size_px > 0.0;
    let GaussianBuffers {
        intermediate: blur_scratch,
        output: gaussian,
    } = if diffuse {
        gaussian_scratch.buffers(backend, width, height)?
    } else {
        GaussianBuffers::default()
    };
    let correction = if diffuse {
        transport("dir_correction")
    } else {
        Vec::new()
    };
    let tail = if diffuse {
        transport("dir_exponential_tail")
    } else {
        Vec::new()
    };
    let mix = transport("dir_mix");

    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct MatmulParams {
        n_pixels: u32,
        positive: u32,
        _pad: [u32; 2],
        density_max: [f32; 4],
        donor_k: [f32; 4],
        donor_ref: [f32; 4],
        donor_mask: [f32; 4],
        m_row0: [f32; 4],
        m_row1: [f32; 4],
        m_row2: [f32; 4],
    }
    let m = &dp.couplers_matrix_scaled;
    let (donor_k, donor_mask) = langmuir_uniforms(dp.donor_k);
    let matmul = pointwise(
        backend,
        n_pixels,
        pixels,
        include_str!("dir_matmul.wgsl"),
        |n| MatmulParams {
            n_pixels: n,
            positive: u32::from(dp.is_positive),
            _pad: [0; 2],
            density_max: [dp.density_max[0], dp.density_max[1], dp.density_max[2], 0.0],
            donor_k,
            donor_mask,
            donor_ref: [dp.donor_ref[0], dp.donor_ref[1], dp.donor_ref[2], 0.0],
            m_row0: [m[0][0], m[0][1], m[0][2], 0.0],
            m_row1: [m[1][0], m[1][1], m[1][2], 0.0],
            m_row2: [m[2][0], m[2][1], m[2][2], 0.0],
        },
    );
    let make_blur = |sigma, out: &[ImagePage], _label: &str| {
        build_grain_blur_paged_with_scratch(
            backend,
            width,
            height,
            &correction,
            out,
            Some(&blur_scratch),
            [sigma; 3],
            0,
            0,
        )
    };
    let blur_gaussian = diffuse
        .then(|| make_blur(f64::from(dp.diffusion_size_px), &mix, "dir_gaussian"))
        .transpose()?;
    let mut tail_ops = Vec::new();
    if diffuse {
        for (index, (amp, ratio)) in spektrafilm_math::gaussian::EXPONENTIAL_GAUSSIAN_FITS
            .into_iter()
            .enumerate()
        {
            let sigma = ratio * f64::from(dp.diffusion_tail_px);
            let blur = (sigma > 0.0)
                .then(|| make_blur(sigma, &gaussian, "dir_exponential_component"))
                .transpose()?;
            let accum = pointwise(
                backend,
                n_pixels,
                pixels,
                include_str!("dir_accumulate.wgsl"),
                |n| AddScaledParams {
                    n_pixels: n,
                    scale: amp as f32,
                    clear_first: u32::from(index == 0),
                    _pad: 0,
                },
            );
            tail_ops.push((blur, accum));
        }
    }
    // CPU computes the fitted exponential first, then blends it once with
    // the Gaussian; folding weight into each amplitude changes rounding.
    let blend = diffuse.then(|| {
        pointwise(
            backend,
            n_pixels,
            pixels,
            include_str!("dir_blend.wgsl"),
            |n| AddScaledParams {
                n_pixels: n,
                scale: dp.diffusion_tail_weight,
                clear_first: 0,
                _pad: 0,
            },
        )
    });
    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct SubtractParams {
        n_pixels: u32,
        _pad: [u32; 3],
        receiver_k: [f32; 4],
        receiver_ref: [f32; 4],
        receiver_mask: [f32; 4],
    }
    let (receiver_k, receiver_mask) = langmuir_uniforms(dp.receiver_knee);
    let subtract = pointwise(
        backend,
        n_pixels,
        pixels,
        include_str!("dir_subtract.wgsl"),
        |n| SubtractParams {
            n_pixels: n,
            _pad: [0; 3],
            receiver_k,
            receiver_mask,
            receiver_ref: [
                dp.receiver_ref[0],
                dp.receiver_ref[1],
                dp.receiver_ref[2],
                0.0,
            ],
        },
    );
    let density_curve_0 = density_windows(backend, dp, n_pixels, pixels, limit)?;
    Ok(DirState {
        correction,
        gaussian,
        tail,
        mix,
        density,
        log_raw,
        n_pixels,
        _blur_scratch: blur_scratch,
        matmul,
        blur_gaussian,
        tail_ops,
        blend,
        subtract,
        density_curve_0,
    })
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CurveParams {
    n_pixels: u32,
    total_rows: u32,
    first_row: u32,
    page_rows: u32,
    uniform_grid: u32,
    gamma_inv: f32,
    first_x: f32,
    last_x: f32,
}

/// Adjacent coefficient pages overlap one row: intervals have one owner,
/// while interpolation at a page boundary still sees both real endpoints.
fn density_windows(
    backend: &WgpuBackend,
    dp: &spektrafilm_backend::DirCouplersGpuParams<'_>,
    n_pixels: u32,
    pixels: u32,
    limit: u64,
) -> Result<WindowPass, String> {
    let x: Vec<f32> = dp.log_exposure.iter().map(|&v| v as f32).collect();
    let y = nan_safe_rows_to_f32(dp.density_curves_0);
    let rows = u32::try_from(x.len()).map_err(|_| "DIR curve exceeds u32 indexing")?;
    if rows == 0 || y.len() != x.len() * 3 {
        return Err("DIR requires matching nonempty exposure and density curves".into());
    }
    let page_rows = (limit / 12).min(u64::from(rows)) as usize;
    if page_rows < 2 && rows > 1 {
        return Err(format!(
            "DIR curve interpolation requires two RGB coefficient rows (24 bytes); binding limit is {limit}"
        ));
    }
    let source = window_buffer(&backend.device, u64::from(pixels) * 12, "dir_curve_source");
    let destination = window_buffer(
        &backend.device,
        u64::from(pixels) * 12,
        "dir_curve_destination",
    );
    let pipeline = backend.cached_pipeline(
        include_str!("dir_curve.wgsl"),
        &[
            wgpu::BufferBindingType::Uniform,
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: true },
            wgpu::BufferBindingType::Storage { read_only: false },
        ],
    );
    let mut full = Vec::new();
    let mut remainder = Vec::new();
    let mut coefficients = Vec::new();
    let mut first = 0;
    loop {
        let end = (first + page_rows).min(x.len());
        let upload = |values: &[f32]| {
            backend
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("dir_curve_page"),
                    contents: bytemuck::cast_slice(values),
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let xb = upload(&x[first..end]);
        let yb = upload(&y[first * 3..end * 3]);
        let params = |n| CurveParams {
            n_pixels: n,
            total_rows: rows,
            first_row: first as u32,
            page_rows: (end - first) as u32,
            uniform_grid: u32::from(is_uniform_grid_endpoint(dp.log_exposure)),
            gamma_inv: (1.0 / dp.gamma_factor) as f32,
            first_x: x[0],
            last_x: x[x.len() - 1],
        };
        full.push(window_job(
            &backend.device,
            &pipeline,
            &params(pixels),
            &source,
            &destination,
            &[&xb, &yb],
        ));
        if !n_pixels.is_multiple_of(pixels) {
            remainder.push(window_job(
                &backend.device,
                &pipeline,
                &params(n_pixels % pixels),
                &source,
                &destination,
                &[&xb, &yb],
            ));
        }
        coefficients.extend([xb, yb]);
        if end == x.len() {
            break;
        }
        first = end - 1;
    }
    Ok(WindowPass {
        source,
        destination,
        pixels,
        full,
        remainder,
        _coefficients: coefficients,
    })
}

impl DirState {
    /// Submit one bounded pointwise window or spatial page at a time. Queue
    /// order retains shared scratch dependencies, with cancellation between
    /// submissions and no host inspection of image intermediates.
    pub(super) fn submit_passes(&self, backend: &WgpuBackend) -> Result<(), String> {
        let target = if self.blur_gaussian.is_some() {
            &self.correction
        } else {
            &self.mix
        };
        self.matmul
            .submit(backend, &self.density, target, self.n_pixels, false)?;
        if let Some(blur) = &self.blur_gaussian {
            blur.submit(backend)?;
        }
        for (index, (blur, accum)) in self.tail_ops.iter().enumerate() {
            if let Some(blur) = blur {
                blur.submit(backend)?;
            }
            let source = if blur.is_some() {
                &self.gaussian
            } else {
                &self.correction
            };
            accum.submit(backend, source, &self.tail, self.n_pixels, index != 0)?;
        }
        if let Some(blend) = &self.blend {
            blend.submit(backend, &self.tail, &self.mix, self.n_pixels, true)?;
        }
        self.subtract
            .submit(backend, &self.mix, &self.log_raw, self.n_pixels, true)?;
        self.density_curve_0
            .submit(backend, &self.log_raw, &self.density, self.n_pixels, false)
    }
}

#[cfg(all(test, feature = "wgpu-backend"))]
mod tests {
    use super::*;

    #[repr(C)]
    #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
    struct MatmulParams {
        n_pixels: u32,
        positive: u32,
        _pad0: u32,
        _pad1: u32,
        density_max: [f32; 4],
        donor_k: [f32; 4],
        donor_ref: [f32; 4],
        donor_mask: [f32; 4],
        m_row0: [f32; 4],
        m_row1: [f32; 4],
        m_row2: [f32; 4],
    }

    #[test]
    fn dir_matmul_shader_matches_reference_math() {
        let b = crate::test_support::adapter();
        let density: Vec<f32> = vec![2.511903, 2.7011967, 0.26266688, 0.34336, 0.07575, 0.05212];
        let params = MatmulParams {
            n_pixels: 2,
            positive: 1,
            _pad0: 0,
            _pad1: 0,
            density_max: [2.511998, 2.8034478, 2.810454, 0.0],
            donor_k: [1.0; 4],
            donor_ref: [0.0; 4],
            donor_mask: [0.0; 4],
            m_row0: [0.341, 0.355, 0.305, 0.0],
            m_row1: [0.154, 0.324, 0.358, 0.0],
            m_row2: [0.171, 0.225, 0.273, 0.0],
        };
        let input = b.pipe.upload_image(&density, 2);
        let out = b
            .pipe
            .dispatch_device(
                include_str!("dir_matmul.wgsl"),
                &input,
                &[GpuBuffer::uniform(bytemuck::bytes_of(&params))],
                2,
                "dir_matmul_test",
            )
            .expect("test bindings fit the device");
        let got = b.pipe.readback_image(&out);
        // silver = dmax - density; correction = M^T silver.
        let dmax = [2.511998f32, 2.8034478, 2.810454];
        let mut expected = Vec::new();
        for px in 0..2 {
            let s: Vec<f32> = (0..3).map(|c| dmax[c] - density[px * 3 + c]).collect();
            for m in 0..3 {
                expected.push(
                    s[0] * [0.341, 0.355, 0.305][m]
                        + s[1] * [0.154, 0.324, 0.358][m]
                        + s[2] * [0.171, 0.225, 0.273][m],
                );
            }
        }
        eprintln!("matmul got={got:?} expected={expected:?}");
        for (g, e) in got.iter().zip(expected.iter()) {
            assert!((g - e).abs() < 1e-5, "got {got:?} expected {expected:?}");
        }
    }

    #[test]
    fn langmuir_uniforms_masks_absent_and_non_finite_knees() {
        // No knee on this side (the caller passes `None`): the shader must
        // not apply the saturation, so every channel is masked off.
        let (k, mask) = langmuir_uniforms(None);
        assert_eq!(mask, [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(k, [1.0, 1.0, 1.0, 1.0]);
        // `K = inf` means "linear" on the CPU (`k.is_infinite()`); the
        // channel is masked off and the uniform stays finite so the
        // unconditionally-evaluated ratio cannot divide by infinity.
        let (k, mask) = langmuir_uniforms(Some([f64::INFINITY as f32, 1.5, 3.0]));
        assert_eq!(mask, [0.0, 1.0, 1.0, 0.0]);
        assert_eq!(k, [1.0, 1.5, 3.0, 1.0]);
        let (k, mask) = langmuir_uniforms(Some([0.0, 2.0, 4.0]));
        assert_eq!(mask, [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(k, [0.0, 2.0, 4.0, 1.0]);
    }

    #[test]
    fn dir_matmul_shader_applies_donor_langmuir_on_masked_channels() {
        let b = crate::test_support::adapter();
        // Negative film at the acceptor, where the donor langmuir runs.
        let density: Vec<f32> = vec![0.4, 1.6, 2.4, 0.9, 0.2, 1.1];
        let donor_k = [1.0f32, 2.0, 1.0, 1.0];
        let donor_ref = [0.5f32, 1.0, 0.5, 0.0];
        // Channel 1 sits behind a non-finite knee: linear there.
        let donor_mask = [1.0f32, 0.0, 1.0, 0.0];
        let params = MatmulParams {
            n_pixels: 2,
            positive: 0,
            _pad0: 0,
            _pad1: 0,
            density_max: [1.0; 4],
            donor_k,
            donor_ref,
            donor_mask,
            m_row0: [0.341, 0.355, 0.305, 0.0],
            m_row1: [0.154, 0.324, 0.358, 0.0],
            m_row2: [0.171, 0.225, 0.273, 0.0],
        };
        let input = b.pipe.upload_image(&density, 2);
        let out = b
            .pipe
            .dispatch_device(
                include_str!("dir_matmul.wgsl"),
                &input,
                &[GpuBuffer::uniform(bytemuck::bytes_of(&params))],
                2,
                "dir_matmul_donor_test",
            )
            .expect("test bindings fit the device");
        let got = b.pipe.readback_image(&out);
        let mut expected = Vec::new();
        for px in 0..2 {
            // silver = density, then g(v) = v*(k+ref)/(k+v) per masked channel.
            let s: Vec<f32> = (0..3)
                .map(|c| {
                    let v = density[px * 3 + c];
                    if donor_mask[c] > 0.5 {
                        v * (donor_k[c] + donor_ref[c]) / (donor_k[c] + v)
                    } else {
                        v
                    }
                })
                .collect();
            for m in 0..3 {
                expected.push(
                    s[0] * [0.341, 0.355, 0.305][m]
                        + s[1] * [0.154, 0.324, 0.358][m]
                        + s[2] * [0.171, 0.225, 0.273][m],
                );
            }
        }
        for (g, e) in got.iter().zip(expected.iter()) {
            assert!((g - e).abs() < 1e-5, "got {got:?} expected {expected:?}");
        }
    }
    #[cfg(not(feature = "precision-f64"))]
    #[test]
    fn dir_bounded_windows_preserve_reference_tail_and_true_endpoints() {
        use spektrafilm_math::gaussian::{exponential_filter_channel, gaussian_blur_channel};
        let gpu = crate::test_support::adapter();
        let width = 13;
        let height = 7;
        let count = (width * height) as usize;
        let density: Vec<f32> = (0..count * 3)
            .map(|i| 0.08 + ((i * 37 + 11) % 113) as f32 / 113.0)
            .collect();
        let log_raw: Vec<f32> = (0..count * 3)
            .map(|i| match i % 17 {
                0 => -2.0,
                1 => 2.0,
                _ => -0.4 + ((i * 19 + 7) % 107) as f32 / 107.0,
            })
            .collect();
        // Identity curves make the post-DIR interpolation directly observable.
        // Nonuniform x is exercised separately with matching identity y.
        for exposure in [
            vec![-0.6, -0.1, 0.4, 0.9, 1.4],
            vec![-0.6, -0.2, 0.15, 0.8, 1.4],
        ] {
            let identity: Vec<[f64; 3]> = exposure.iter().map(|&x| [x; 3]).collect();
            for (size, tail_size, weight, positive) in [
                (0.0, 0.0, 0.7, false),
                (0.7, 0.0, 1.0, true),
                (0.7, 2.2, 0.61, false),
                (3.0, 8.0, 0.77, true),
            ] {
                let dp = spektrafilm_backend::DirCouplersGpuParams {
                    couplers_matrix_scaled: [
                        [0.13, 0.04, 0.08],
                        [0.07, 0.19, 0.02],
                        [0.03, 0.06, 0.23],
                    ],
                    density_max: [1.2, 1.4, 1.6],
                    is_positive: positive,
                    diffusion_size_px: size,
                    diffusion_tail_px: tail_size,
                    diffusion_tail_weight: weight,
                    donor_k: Some([0.8, f32::INFINITY, 1.7]),
                    donor_ref: [0.4, 0.5, 0.6],
                    receiver_knee: positive.then_some([1.1, f32::INFINITY, 0.9]),
                    receiver_ref: [0.2, 0.3, 0.4],
                    density_curves_0: &identity,
                    log_exposure: &exposure,
                    gamma_factor: 1.0,
                };
                let upload = |data: &[f32], page_pixels: usize| {
                    data.chunks(page_pixels * 3)
                        .enumerate()
                        .map(|(i, values)| ImagePage {
                            first_pixel: (i * page_pixels) as u32,
                            n_pixels: values.len() as u32 / 3,
                            buffer: gpu.device.create_buffer_init(
                                &wgpu::util::BufferInitDescriptor {
                                    label: Some("dir_reference_transport"),
                                    contents: bytemuck::cast_slice(values),
                                    usage: wgpu::BufferUsages::COPY_SRC
                                        | wgpu::BufferUsages::COPY_DST,
                                },
                            ),
                        })
                        .collect::<Vec<_>>()
                };
                let a = upload(&density, 4);
                let b = upload(&log_raw, 7);
                // 60 bytes: five image pixels and five coefficient rows. A
                // 36-byte second run splits curves into overlapping pages too.
                let mut gaussian_scratch = GaussianScratch::new(width, height);
                for limit in [60, 36] {
                    for (pages, values) in [(&a, &density), (&b, &log_raw)] {
                        for page in pages {
                            let start = page.first_pixel as usize * 3;
                            let end = start + page.n_pixels as usize * 3;
                            gpu.queue.write_buffer(
                                &page.buffer,
                                0,
                                bytemuck::cast_slice(&values[start..end]),
                            );
                        }
                    }
                    let state = build_dir_state_with_limit(
                        &gpu.device,
                        &dp,
                        width,
                        height,
                        &a,
                        &b,
                        &mut gaussian_scratch,
                        &gpu,
                        limit,
                    )
                    .expect("bounded DIR state");
                    let bytes = count as u64 * 12;
                    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("dir_reference_readback"),
                        size: bytes * 2,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    state.submit_passes(&gpu).expect("DIR page submissions");
                    let mut encoder = gpu.device.create_command_encoder(&Default::default());
                    a.read(&mut encoder, 0, count as u64 * 3, &readback, 0);
                    b.read(
                        &mut encoder,
                        0,
                        count as u64 * 3,
                        &readback,
                        count as u64 * 3,
                    );
                    gpu.queue.submit(Some(encoder.finish()));
                    let actual = gpu.read_mapped_f32(&readback, bytes * 2);
                    let mut correction = vec![0.0_f32; density.len()];
                    for (pixel, input) in density.as_chunks::<3>().0.iter().enumerate() {
                        let mut silver = [0.0; 3];
                        for c in 0..3 {
                            let v = if positive {
                                dp.density_max[c] - input[c]
                            } else {
                                input[c]
                            };
                            let k = dp.donor_k.unwrap()[c];
                            silver[c] = if k.is_infinite() {
                                v
                            } else {
                                v * (k + dp.donor_ref[c]) / (k + v)
                            };
                        }
                        for c in 0..3 {
                            correction[pixel * 3 + c] = silver[0] * dp.couplers_matrix_scaled[0][c]
                                + silver[1] * dp.couplers_matrix_scaled[1][c]
                                + silver[2] * dp.couplers_matrix_scaled[2][c];
                        }
                    }
                    for c in 0..3 {
                        let channel: Vec<f32> =
                            correction.as_chunks::<3>().0.iter().map(|p| p[c]).collect();
                        let filtered = if size > 0.0 {
                            let g = gaussian_blur_channel(&channel, width, height, size);
                            let t = exponential_filter_channel(&channel, width, height, tail_size);
                            g.iter()
                                .zip(t)
                                .map(|(&g, t)| (1.0 - weight) * g + weight * t)
                                .collect::<Vec<_>>()
                        } else {
                            channel
                        };
                        for (pixel, &value) in filtered.iter().enumerate() {
                            let value = if let Some(k) = dp.receiver_knee {
                                if k[c].is_infinite() {
                                    value
                                } else {
                                    value * (k[c] + dp.receiver_ref[c]) / (k[c] + value)
                                }
                            } else {
                                value
                            };
                            let expected_log = log_raw[pixel * 3 + c] - value;
                            let expected_density = expected_log.clamp(-0.6, 1.4);
                            let index = pixel * 3 + c;
                            assert!(
                                (actual[index + count * 3] - expected_log).abs() < 5e-5,
                                "DIR raw-log: limit {limit}, sigma {size}/{tail_size}, pixel {pixel}, channel {c}"
                            );
                            assert!(
                                (actual[index] - expected_density).abs() < 5e-5,
                                "DIR curve/endpoints: limit {limit}, sigma {size}/{tail_size}, pixel {pixel}, channel {c}"
                            );
                        }
                    }
                }
            }
        }
    }
}
