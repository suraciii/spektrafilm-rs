use super::blur::{GaussianBuffers, GaussianScratch};
use super::grain::raster::RasterStorage;
use super::grain::v1::blur::build_grain_blur_paged_with_scratch;
use super::page_views::{Access, ViewPlan};
use super::paged::ImagePage;
use super::*;

/// DIR keeps the reference Gaussian and ordered three-fit exponential mixture.
/// Only the active spatial component owns a blur state; pointwise passes use
/// resident storage views, with bounded adapters for incompatible bindings.
pub(super) struct DirState {
    density: Vec<ImagePage>,
    log_raw: Vec<ImagePage>,
    width: u32,
    height: u32,
    n_pixels: u32,
    page_pixels: u32,
    gaussian: GaussianBuffers,
    matmul: MatmulParams,
    subtract: SubtractParams,
    diffusion_size: f32,
    diffusion_tail: f32,
    diffusion_weight: f32,
    curves: Vec<CurvePage>,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct MatmulParams {
    n_pixels: u32,
    positive: u32,
    source_base: u32,
    destination_base: u32,
    density_max: [f32; 4],
    donor_k: [f32; 4],
    donor_ref: [f32; 4],
    donor_mask: [f32; 4],
    m_row0: [f32; 4],
    m_row1: [f32; 4],
    m_row2: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct SubtractParams {
    n_pixels: u32,
    source_base: u32,
    destination_base: u32,
    _pad: u32,
    receiver_k: [f32; 4],
    receiver_ref: [f32; 4],
    receiver_mask: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ScaledParams {
    n_pixels: u32,
    scale: f32,
    clear_first: u32,
    _pad: u32,
    source_base: u32,
    destination_base: u32,
    _pad_bases: [u32; 2],
}

struct CurvePage {
    params: CurveParams,
    exposure: wgpu::Buffer,
    density: wgpu::Buffer,
}

fn pointwise<P: bytemuck::Pod>(
    backend: &WgpuBackend,
    n_pixels: u32,
    shader: &'static str,
    buffers: &[(&dyn RasterStorage, Access)],
    params: impl Fn(u32, &[u32]) -> P,
) -> Result<(), String> {
    let plan = ViewPlan::new(backend, u64::from(n_pixels), 12, buffers, "dir_pointwise")?;
    let mut layout = vec![wgpu::BufferBindingType::Uniform];
    layout.extend(
        buffers
            .iter()
            .map(|(_, access)| wgpu::BufferBindingType::Storage {
                read_only: matches!(access, Access::Read | Access::Fixed),
            }),
    );
    let pipeline = backend.cached_pipeline(shader, &layout);
    plan.for_each(backend, |encoder, chunk| {
        let mut bases = [0; 4];
        for (base, view) in bases.iter_mut().zip(&chunk.views) {
            *base = view.base;
        }
        let uniform = backend
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dir_view_params"),
                contents: bytemuck::bytes_of(&params(chunk.count, &bases)),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }];
        entries.extend(
            chunk
                .views
                .iter()
                .enumerate()
                .map(|(index, view)| wgpu::BindGroupEntry {
                    binding: index as u32 + 1,
                    resource: view.binding(),
                }),
        );
        let bg = backend
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dir_view_bg"),
                layout: &pipeline.layout,
                entries: &entries,
            });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("dir_pointwise"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &bg, &[]);
        dispatch_linear(&mut pass, chunk.count);
        Ok(())
    })
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
    _device: &ObservedDevice,
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
    // Validate the complete mutable image relationship before any DIR work.
    // The common planner checks page coverage, usages and whole-pass aliases.
    ViewPlan::new(
        backend,
        count,
        12,
        &[
            (&density, Access::WritePartial),
            (&log_raw, Access::ReadWrite),
        ],
        "dir_images",
    )?;
    let page_pixels = (super::bounded::chunk_elements(limit, 12)? as u32).min(n_pixels);
    let curves = density_pages(backend, dp, limit)?;
    let gaussian = if dp.diffusion_size_px > 0.0 {
        gaussian_scratch.buffers(backend, width, height)?
    } else {
        GaussianBuffers::default()
    };
    let m = &dp.couplers_matrix_scaled;
    let (donor_k, donor_mask) = langmuir_uniforms(dp.donor_k);
    let matmul = MatmulParams {
        n_pixels,
        positive: u32::from(dp.is_positive),
        source_base: 0,
        destination_base: 0,
        density_max: [dp.density_max[0], dp.density_max[1], dp.density_max[2], 0.0],
        donor_k,
        donor_mask,
        donor_ref: [dp.donor_ref[0], dp.donor_ref[1], dp.donor_ref[2], 0.0],
        m_row0: [m[0][0], m[0][1], m[0][2], 0.0],
        m_row1: [m[1][0], m[1][1], m[1][2], 0.0],
        m_row2: [m[2][0], m[2][1], m[2][2], 0.0],
    };
    let (receiver_k, receiver_mask) = langmuir_uniforms(dp.receiver_knee);
    let subtract = SubtractParams {
        n_pixels,
        source_base: 0,
        destination_base: 0,
        _pad: 0,
        receiver_k,
        receiver_mask,
        receiver_ref: [
            dp.receiver_ref[0],
            dp.receiver_ref[1],
            dp.receiver_ref[2],
            0.0,
        ],
    };
    Ok(DirState {
        density,
        log_raw,
        width,
        height,
        n_pixels,
        page_pixels,
        gaussian,
        matmul,
        subtract,
        diffusion_size: dp.diffusion_size_px,
        diffusion_tail: dp.diffusion_tail_px,
        diffusion_weight: dp.diffusion_tail_weight,
        curves,
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
    bases: [u32; 4],
}

/// Adjacent coefficient pages overlap one row: intervals have one owner,
/// while interpolation at a page boundary still sees both real endpoints.
fn density_pages(
    backend: &WgpuBackend,
    dp: &spektrafilm_backend::DirCouplersGpuParams<'_>,
    limit: u64,
) -> Result<Vec<CurvePage>, String> {
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
    let uniform_grid = u32::from(is_uniform_grid_endpoint(dp.log_exposure));
    let mut pages = Vec::new();
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
        pages.push(CurvePage {
            params: CurveParams {
                n_pixels: 0,
                total_rows: rows,
                first_row: first as u32,
                page_rows: (end - first) as u32,
                uniform_grid,
                gamma_inv: (1.0 / dp.gamma_factor) as f32,
                first_x: x[0],
                last_x: x[x.len() - 1],
                bases: [0; 4],
            },
            exposure: upload(&x[first..end]),
            density: upload(&y[first * 3..end * 3]),
        });
        if end == x.len() {
            break;
        }
        first = end - 1;
    }
    Ok(pages)
}

impl DirState {
    fn transport(&self, backend: &WgpuBackend, label: &str) -> Vec<ImagePage> {
        (0..self.n_pixels)
            .step_by(self.page_pixels as usize)
            .map(|first_pixel| {
                let n_pixels = self.page_pixels.min(self.n_pixels - first_pixel);
                ImagePage {
                    first_pixel,
                    n_pixels,
                    buffer: backend.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(label),
                        size: u64::from(n_pixels) * 12,
                        usage: wgpu::BufferUsages::STORAGE
                            | wgpu::BufferUsages::COPY_SRC
                            | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                }
            })
            .collect()
    }

    fn matmul(&self, backend: &WgpuBackend, target: &dyn RasterStorage) -> Result<(), String> {
        pointwise(
            backend,
            self.n_pixels,
            include_str!("dir_matmul.wgsl"),
            &[(&self.density, Access::Read), (target, Access::Write)],
            |n, bases| MatmulParams {
                n_pixels: n,
                source_base: bases[0],
                destination_base: bases[1],
                ..self.matmul
            },
        )
    }

    fn blur(
        &self,
        backend: &WgpuBackend,
        source: &[ImagePage],
        target: &[ImagePage],
        sigma: f64,
    ) -> Result<(), String> {
        // Drop each component's bind groups and recurrence state at its last
        // submission; the shared queue owns submitted resources until retired.
        build_grain_blur_paged_with_scratch(
            backend,
            self.width,
            self.height,
            source,
            target,
            Some(&self.gaussian.intermediate),
            [sigma; 3],
            0,
            0,
        )?
        .submit(backend)
    }

    /// Queue order protects shared scratch. Replaying requires restoring the
    /// immutable density/log inputs; partially modified outputs are not inputs.
    pub(super) fn submit_passes(&self, backend: &WgpuBackend) -> Result<(), String> {
        backend.check_cancelled()?;
        let mix = self.transport(backend, "dir_mix");
        if self.diffusion_size > 0.0 {
            let correction = self.transport(backend, "dir_correction");
            self.matmul(backend, &correction)?;
            self.blur(backend, &correction, &mix, f64::from(self.diffusion_size))?;
            let tail = self.transport(backend, "dir_exponential_tail");
            for (index, (amp, ratio)) in spektrafilm_math::gaussian::EXPONENTIAL_GAUSSIAN_FITS
                .into_iter()
                .enumerate()
            {
                let sigma = ratio * f64::from(self.diffusion_tail);
                let source = if sigma > 0.0 {
                    self.blur(backend, &correction, &self.gaussian.output, sigma)?;
                    &self.gaussian.output
                } else {
                    &correction
                };
                pointwise(
                    backend,
                    self.n_pixels,
                    include_str!("dir_accumulate.wgsl"),
                    &[
                        (source, Access::Read),
                        (
                            &tail,
                            if index == 0 {
                                Access::Write
                            } else {
                                Access::ReadWrite
                            },
                        ),
                    ],
                    |n, bases| ScaledParams {
                        n_pixels: n,
                        scale: amp as f32,
                        clear_first: u32::from(index == 0),
                        _pad: 0,
                        source_base: bases[0],
                        destination_base: bases[1],
                        _pad_bases: [0; 2],
                    },
                )?;
            }
            drop(correction);
            // Blend once after the ordered three terms; distributing this
            // weight into their amplitudes would change reference rounding.
            pointwise(
                backend,
                self.n_pixels,
                include_str!("dir_blend.wgsl"),
                &[(&tail, Access::Read), (&mix, Access::ReadWrite)],
                |n, bases| ScaledParams {
                    n_pixels: n,
                    scale: self.diffusion_weight,
                    clear_first: 0,
                    _pad: 0,
                    source_base: bases[0],
                    destination_base: bases[1],
                    _pad_bases: [0; 2],
                },
            )?;
        } else {
            self.matmul(backend, &mix)?;
        }
        pointwise(
            backend,
            self.n_pixels,
            include_str!("dir_subtract.wgsl"),
            &[(&mix, Access::Read), (&self.log_raw, Access::ReadWrite)],
            |n, bases| SubtractParams {
                n_pixels: n,
                source_base: bases[0],
                destination_base: bases[1],
                ..self.subtract
            },
        )?;
        drop(mix);
        for page in &self.curves {
            // A coefficient page owns only its interpolation intervals. The
            // adapter must retain values written by earlier coefficient pages.
            pointwise(
                backend,
                self.n_pixels,
                include_str!("dir_curve.wgsl"),
                &[
                    (&self.log_raw, Access::Read),
                    (&page.exposure, Access::Fixed),
                    (&page.density, Access::Fixed),
                    (&self.density, Access::WritePartial),
                ],
                |n, bases| CurveParams {
                    n_pixels: n,
                    bases: [bases[0], bases[1], bases[2], bases[3]],
                    ..page.params
                },
            )?;
        }
        backend.check_cancelled()
    }
}

#[cfg(all(test, feature = "wgpu-backend"))]
mod tests {
    use super::*;

    #[test]
    fn dir_storage_copy_only_and_replay_preserve_raw_outputs() {
        // One adapter guard is shared by every layout and replay below.
        let gpu = crate::test_support::adapter();
        let width = 13;
        let height = 7;
        let count = (width * height) as usize;
        let density: Vec<f32> = (0..count * 3)
            .map(|i| 0.1 + ((i * 37 + 11) % 113) as f32 / 113.0)
            .collect();
        let log_raw: Vec<f32> = (0..count * 3)
            .map(|i| match i % 11 {
                0 => -2.0,
                1 => 2.0,
                2 => f32::NAN,
                _ => -0.5 + ((i * 19 + 7) % 107) as f32 / 107.0,
            })
            .collect();
        for exposure in [
            vec![-0.6, -0.1, 0.4, 0.9, 1.4],
            vec![-0.6, -0.2, 0.15, 0.8, 1.4],
        ] {
            let curves: Vec<[f64; 3]> = exposure
                .iter()
                .map(|&x| [x * 0.7 + 0.2, x * 0.4 + 0.1, x * 0.8 - 0.1])
                .collect();
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
                    density_curves_0: &curves,
                    log_exposure: &exposure,
                    gamma_factor: 1.0,
                };
                let mut expected = None;
                for (a_pixels, b_pixels, storage) in [(4, 7, true), (7, 4, true), (5, 9, false)] {
                    let upload = |values: &[f32], pixels: usize| {
                        values
                            .chunks(pixels * 3)
                            .enumerate()
                            .map(|(i, samples)| ImagePage {
                                first_pixel: (i * pixels) as u32,
                                n_pixels: samples.len() as u32 / 3,
                                buffer: gpu.device.create_buffer_init(
                                    &wgpu::util::BufferInitDescriptor {
                                        label: Some("dir_replay_page"),
                                        contents: bytemuck::cast_slice(samples),
                                        usage: wgpu::BufferUsages::COPY_SRC
                                            | wgpu::BufferUsages::COPY_DST
                                            | if storage {
                                                wgpu::BufferUsages::STORAGE
                                            } else {
                                                wgpu::BufferUsages::empty()
                                            },
                                    },
                                ),
                            })
                            .collect::<Vec<_>>()
                    };
                    let a = upload(&density, a_pixels);
                    let b = upload(&log_raw, b_pixels);
                    let mut scratch = GaussianScratch::new(width, height);
                    // Three-row coefficient pages force overlapping ownership
                    // with both earlier and later output intervals in one view.
                    let state = build_dir_state_with_limit(
                        &gpu.device,
                        &dp,
                        width,
                        height,
                        &a,
                        &b,
                        &mut scratch,
                        &gpu,
                        36,
                    )
                    .expect("DIR replay state");
                    for replay in 0..2 {
                        // Never replay DIR over its partially modified inputs.
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
                        state.submit_passes(&gpu).expect("DIR replay submission");
                        let bytes = count as u64 * 12;
                        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("dir_replay_readback"),
                            size: bytes * 2,
                            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        });
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
                        let actual = gpu.read_mapped_u32(&readback, bytes * 2);
                        if let Some(expected) = &expected {
                            assert_eq!(
                                &actual, expected,
                                "DIR raw bits: pages {a_pixels}/{b_pixels}, storage {storage}, replay {replay}, sigma {size}/{tail_size}, positive {positive}"
                            );
                        } else {
                            expected = Some(actual.clone());
                        }
                        for (i, &value) in log_raw.iter().enumerate() {
                            if value.is_nan() {
                                assert!(
                                    f32::from_bits(actual[i]).is_nan(),
                                    "curve must own NaN output"
                                );
                                assert!(f32::from_bits(actual[i + count * 3]).is_nan());
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn dir_matmul_shader_matches_reference_math() {
        let b = crate::test_support::adapter();
        let density: Vec<f32> = vec![2.511903, 2.7011967, 0.26266688, 0.34336, 0.07575, 0.05212];
        let params = MatmulParams {
            n_pixels: 2,
            positive: 1,
            source_base: 0,
            destination_base: 0,
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
            source_base: 0,
            destination_base: 0,
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
