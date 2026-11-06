use super::blur::{GaussianBuffers, GaussianScratch};
use super::grain::raster::{RasterStorage, paged_transport};
use super::grain::spektra::blur::{GrainBlurState as BlurJob, build_grain_blur_paged_with_scratch};
use super::highlight::{PointwiseAccess, PointwiseJob, image_transport_bytes, pointwise_job};
use super::paged::ImagePage;
use super::*;

/// Device scatter and multi-bounce halation.
///
/// CPU equivalent: `spektrafilm_model::halation::apply_halation_um`. Every
/// width, tail weight, and amplitude keeps its independent RGB value: the
/// scatter core, each of the three exponential-fit components, and every
/// bounce blur run through the shared reference Gaussian with per-channel
/// sigmas, so no channel average enters the optics.
///
/// All passes work on the resident image (`buf_b`) with `buf_c` as the blur
/// destination, `buf_d` as the scatter and then halation accumulator, and an
/// optional `buf_e` holding the compressed halation source. One filter
/// scratch region is shared, because the passes encode sequentially.
pub(super) struct HalationState {
    _buf_c: Vec<ImagePage>,
    _buf_d: Vec<ImagePage>,
    _buf_e: Option<Vec<ImagePage>>,
    _blur_scratch: Vec<ImagePage>,
    scatter: Option<ScatterPasses>,
    halation_source: Option<PointwiseJob>,
    halation: Option<HalationPasses>,
}

/// Scatter: `(1-sa)·x + sa·((1-wt)·G_σ(x) + wt·Σ_k amp_k·G_{ratio_k·λ}(x))`.
struct ScatterPasses {
    core: BlurJob,
    /// `acc = (1-wt)·core`
    init: PointwiseJob,
    /// Per fitted component: tail blur, then its weighted accumulate.
    tails: Vec<(BlurJob, PointwiseJob)>,
    finish: PointwiseJob,
}

/// Halation: `x += a_tot·Σ_k w_k·G_{σ₁√(k+1)}(source)`, then renormalize.
struct HalationPasses {
    blurs: Vec<BlurJob>,
    accumulate: Vec<PointwiseJob>,
    final_add: PointwiseJob,
    renormalize: Option<PointwiseJob>,
}

/// Per-channel `dst[c] = clear ? scale[c]·src[c] : dst[c] + scale[c]·src[c]`.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ChannelScaleParams {
    n_pixels: u32,
    clear_first: u32,
    _pad1: u32,
    _pad2: u32,
    scale: [f32; 4],
}

fn per_channel_job(
    backend: &WgpuBackend,
    n_pixels: u32,
    src: &dyn RasterStorage,
    dst: &dyn RasterStorage,
    scale: [f32; 3],
    clear_first: bool,
    label: &str,
) -> Result<PointwiseJob, String> {
    pointwise_job(
        backend,
        u64::from(n_pixels),
        12,
        include_str!("add_scaled_per_channel.wgsl"),
        &ChannelScaleParams {
            n_pixels,
            clear_first: u32::from(clear_first),
            _pad1: 0,
            _pad2: 0,
            scale: [scale[0], scale[1], scale[2], 0.0],
        },
        &[
            (src, PointwiseAccess::Read),
            (
                dst,
                if clear_first {
                    PointwiseAccess::Write
                } else {
                    PointwiseAccess::ReadWrite
                },
            ),
        ],
        None,
        label,
    )
}

/// Whether scatter or an effective bounce actually constructs a Gaussian job.
pub(super) fn uses_gaussian(hp: &spektrafilm_backend::HalationGpuParams) -> bool {
    hp.scatter_amount > 0.0
        || (hp.halation_n_bounces >= 1
            && hp.halation_a_tot.iter().any(|&value| value > 0.0)
            && (0..3).any(|c| hp.halation_a_tot[c] != 0.0 && hp.halation_first_sigma_px[c] > 0.0))
}

pub(super) fn build_halation_state(
    hp: &spektrafilm_backend::HalationGpuParams,
    width: u32,
    height: u32,
    buf_b: &(impl RasterStorage + ?Sized),
    gaussian_scratch: &mut GaussianScratch,
    backend: &WgpuBackend,
) -> Result<HalationState, String> {
    let n_pixels = (width as usize) * (height as usize);
    let img_bytes = image_transport_bytes(backend, width, height, "halation")?;
    let source_pages = buf_b.pages();
    let buf_b = &source_pages;
    let GaussianBuffers {
        intermediate: blur_scratch,
        output: buf_c,
    } = if uses_gaussian(hp) {
        gaussian_scratch.buffers(backend, width, height)?
    } else {
        GaussianBuffers::default()
    };
    let buf_d = paged_transport(backend, img_bytes, "halation_d")?;

    // ── Scatter ────────────────────────────────────────────────────────
    // The reference skips a channel only when its core and tail widths are
    // both non-positive; such a channel keeps its input unchanged, which the
    // zero blend weight reproduces exactly.
    let scatter = if hp.scatter_amount > 0.0 {
        let active: [bool; 3] =
            std::array::from_fn(|c| hp.scatter_core_px[c] > 0.0 || hp.scatter_tail_px[c] > 0.0);
        let core_sigma: [f64; 3] = std::array::from_fn(|c| {
            if hp.scatter_core_px[c] > 0.0 {
                f64::from(hp.scatter_core_px[c])
            } else {
                0.0
            }
        });
        let tail_lambda: [f64; 3] = std::array::from_fn(|c| {
            if hp.scatter_tail_px[c] > 0.0 {
                f64::from(hp.scatter_tail_px[c])
            } else {
                0.0
            }
        });
        let core = build_grain_blur_paged_with_scratch(
            backend,
            width,
            height,
            buf_b,
            &buf_c,
            Some(&blur_scratch),
            core_sigma,
            0,
            0,
        )
        .map_err(|e| format!("halation scatter core Gaussian: {e}"))?;
        // `acc = (1-wt)·core`
        let init = per_channel_job(
            backend,
            n_pixels as u32,
            &buf_c,
            &buf_d,
            std::array::from_fn(|c| {
                if active[c] {
                    1.0 - hp.scatter_tail_weight[c]
                } else {
                    0.0
                }
            }),
            true,
            "halation_scatter_init",
        )?;
        let mut tails = Vec::with_capacity(3);
        for (amp, ratio) in spektrafilm_math::gaussian::EXPONENTIAL_GAUSSIAN_FITS {
            let sigma: [f64; 3] = std::array::from_fn(|c| tail_lambda[c] * ratio);
            let blur = build_grain_blur_paged_with_scratch(
                backend,
                width,
                height,
                buf_b,
                &buf_c,
                Some(&blur_scratch),
                sigma,
                0,
                0,
            )
            .map_err(|e| format!("halation scatter tail Gaussian: {e}"))?;
            let accumulate = per_channel_job(
                backend,
                n_pixels as u32,
                &buf_c,
                &buf_d,
                std::array::from_fn(|c| {
                    if active[c] {
                        amp as f32 * hp.scatter_tail_weight[c]
                    } else {
                        0.0
                    }
                }),
                false,
                "halation_scatter_tail_acc",
            )?;
            tails.push((blur, accumulate));
        }
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct ScatterFinishParams {
            n_pixels: u32,
            _pad0: u32,
            _pad1: u32,
            _pad2: u32,
            amount: [f32; 4],
        }
        let finish = pointwise_job(
            backend,
            n_pixels as u64,
            12,
            include_str!("scatter_finish.wgsl"),
            &ScatterFinishParams {
                n_pixels: n_pixels as u32,
                _pad0: 0,
                _pad1: 0,
                _pad2: 0,
                amount: std::array::from_fn(|c| {
                    // The shader binds `vec4<f32>`, so this is four wide; only
                    // the first three channels have an entry in `active`.
                    if c < 3 && active[c] {
                        hp.scatter_amount
                    } else {
                        0.0
                    }
                }),
            },
            &[
                (&buf_d, PointwiseAccess::Read),
                (buf_b, PointwiseAccess::ReadWrite),
            ],
            None,
            "halation_scatter_finish",
        )?;
        Some(ScatterPasses {
            core,
            init,
            tails,
            finish,
        })
    } else {
        None
    };

    // ── Optional compressed halation source ───────────────────────────
    // Computed once from the post-scatter image and shared by every bounce.
    let halation_source_active = hp.source_compression > 0.0
        && hp.halation_n_bounces > 0
        && hp.halation_a_tot.iter().any(|&value| value > 0.0);
    let buf_e = halation_source_active
        .then(|| paged_transport(backend, img_bytes, "halation_e"))
        .transpose()?;
    let halation_source = if halation_source_active {
        #[repr(C)]
        #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
        struct SourceParams {
            n_pixels: u32,
            compression: f32,
            _pad0: u32,
            _pad1: u32,
        }
        Some(pointwise_job(
            backend,
            n_pixels as u64,
            12,
            include_str!("source_response.wgsl"),
            &SourceParams {
                n_pixels: n_pixels as u32,
                compression: hp.source_compression,
                _pad0: 0,
                _pad1: 0,
            },
            &[
                (buf_b, PointwiseAccess::Read),
                (
                    buf_e.as_ref().expect("active halation source"),
                    PointwiseAccess::Write,
                ),
            ],
            None,
            "halation_source_response",
        )?)
    } else {
        None
    };
    let halation_src = buf_e.as_ref().unwrap_or(buf_b);

    // ── Multi-bounce halation ─────────────────────────────────────────
    // The reference runs the block when at least one channel amplitude is
    // positive, then adds `a_tot[c]·hb` per channel and renormalizes by
    // `1 + a_tot[c]`. A channel whose first sigma is non-positive is skipped
    // for the add but still participates in renormalization; both effects
    // are reproduced here.
    let halation_runs =
        hp.halation_n_bounces >= 1 && hp.halation_a_tot.iter().any(|&value| value > 0.0);
    let a_eff: [f32; 3] = std::array::from_fn(|c| {
        if hp.halation_first_sigma_px[c] > 0.0 {
            hp.halation_a_tot[c]
        } else {
            0.0
        }
    });
    let halation = if halation_runs {
        let n = hp.halation_n_bounces as usize;
        // Decay weights in f64, normalized the way the reference does.
        let mut decay = vec![0.0f64; n];
        for (k, slot) in decay.iter_mut().enumerate() {
            *slot = f64::from(hp.halation_bounce_decay).powi(k as i32);
        }
        let decay_sum: f64 = decay.iter().sum();
        if decay_sum > 0.0 {
            for d in &mut decay {
                *d /= decay_sum;
            }
        }
        let mut blurs = Vec::new();
        let mut accumulate = Vec::new();
        if a_eff.iter().any(|&value| value != 0.0) {
            for (k, &wk) in decay.iter().enumerate() {
                // A channel with zero effective amplitude contributes nothing:
                // its accumulate weight is zero and renormalization uses
                // `a_tot`, which stays unchanged. A zero sigma copies the
                // channel instead of running a Gaussian whose result is
                // multiplied by zero.
                let sigma: [f64; 3] = std::array::from_fn(|c| {
                    if a_eff[c] == 0.0 {
                        0.0
                    } else {
                        (f64::from(hp.halation_first_sigma_px[c]) * ((k as f64) + 1.0).sqrt())
                            .max(1e-6)
                    }
                });
                blurs.push(
                    build_grain_blur_paged_with_scratch(
                        backend,
                        width,
                        height,
                        halation_src,
                        &buf_c,
                        Some(&blur_scratch),
                        sigma,
                        0,
                        0,
                    )
                    .map_err(|e| format!("halation bounce {k} Gaussian: {e}"))?,
                );
                accumulate.push(per_channel_job(
                    backend,
                    n_pixels as u32,
                    &buf_c,
                    &buf_d,
                    // The bounce weight is scalar; the final add carries the
                    // per-channel amplitude.
                    [wk as f32; 3],
                    k == 0,
                    &format!("halation_bounce_acc_{k}"),
                )?);
            }
        }
        let final_add = per_channel_job(
            backend,
            n_pixels as u32,
            &buf_d,
            buf_b,
            a_eff,
            false,
            "halation_final_add",
        )?;
        let renormalize = if hp.halation_renormalize {
            #[repr(C)]
            #[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
            struct RenormParams {
                n_pixels: u32,
                _pad: u32,
                _gap: [u32; 2],
                inv_factor: [f32; 4],
            }
            Some(pointwise_job(
                backend,
                n_pixels as u64,
                12,
                include_str!("halation_renormalize.wgsl"),
                &RenormParams {
                    n_pixels: n_pixels as u32,
                    _pad: 0,
                    _gap: [0, 0],
                    inv_factor: [
                        1.0 / (1.0 + hp.halation_a_tot[0]),
                        1.0 / (1.0 + hp.halation_a_tot[1]),
                        1.0 / (1.0 + hp.halation_a_tot[2]),
                        0.0,
                    ],
                },
                &[(buf_b, PointwiseAccess::ReadWrite)],
                None,
                "halation_renormalize",
            )?)
        } else {
            None
        };
        Some(HalationPasses {
            blurs,
            accumulate,
            final_add,
            renormalize,
        })
    } else {
        None
    };

    Ok(HalationState {
        _buf_c: buf_c,
        _buf_d: buf_d,
        _buf_e: buf_e,
        _blur_scratch: blur_scratch,
        scatter,
        halation_source,
        halation,
    })
}

impl HalationState {
    pub(super) fn submit(&self, backend: &WgpuBackend) -> Result<(), String> {
        if let Some(scatter) = &self.scatter {
            scatter.core.submit(backend)?;
            scatter.init.execute(backend)?;
            for (blur, accumulate) in &scatter.tails {
                blur.submit(backend)?;
                accumulate.execute(backend)?;
            }
            scatter.finish.execute(backend)?;
        }
        if let Some(source) = &self.halation_source {
            source.execute(backend)?;
        }
        if let Some(halation) = &self.halation {
            for (blur, accumulate) in halation.blurs.iter().zip(&halation.accumulate) {
                blur.submit(backend)?;
                accumulate.execute(backend)?;
            }
            if !halation.blurs.is_empty() {
                halation.final_add.execute(backend)?;
            }
            if let Some(renormalize) = &halation.renormalize {
                renormalize.execute(backend)?;
            }
        }
        Ok(())
    }
}
