/// Filming stage: expose the digital image onto virtual film.
///
/// Full Hanatos2025 path: RGB → XYZ → xy chromaticity → tc coordinates →
/// 2D LUT lookup (spectra × sensitivity) → per-channel film raw exposure.
use rayon::prelude::*;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_backend::timing::{print_stage_timing, stage_timings_enabled};
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f32, from_f64, to_f64};
use spektrafilm_math::spectral::TcLut;
use std::time::Instant;

use crate::params::RuntimeParams;
use crate::profile::Profile;

mod metering;
pub use metering::{MAX_SIZE, MID_GREY, measure_autoexposure_ev, meter_autoexposure_ev};

/// Per-pixel `raw = m · rgb` for the Mallett2019 path (f64 matmul, rayon-parallel).
fn apply_mallett_matrix(image: &ImageBuf, m: &[[f64; 3]; 3]) -> ImageBuf {
    let mut out = image.clone();
    out.data
        .par_chunks_exact_mut(3)
        .zip(image.data.par_chunks_exact(3))
        .for_each(|(dst, src)| {
            let rgb = [src[0] as f64, src[1] as f64, src[2] as f64];
            let v = crate::mallett::apply(m, rgb);
            dst[0] = from_f64(v[0]);
            dst[1] = from_f64(v[1]);
            dst[2] = from_f64(v[2]);
        });
    out
}

/// The reference host diffusion filter, with its telemetry marker.
///
/// Both the per-stage route and the resident chain's fence call this, so
/// the published `optical_diffusion` CPU reason and the observed executor
/// stay identical whichever route ran the convolution on the host.
pub(crate) fn host_diffusion(
    image: &ImageBuf,
    filter: &spektrafilm_model::diffusion::DiffusionFilter<'_>,
    pixel_size_um: f64,
    backend: &dyn spektrafilm_backend::ComputeBackend,
) -> ImageBuf {
    let _diffusion = super::StageObservation::cpu(
        backend,
        "optical_diffusion",
        spektrafilm_backend::telemetry::CpuReason::OpticalDiffusion,
    );
    // An invalid family is rejected by `RuntimeParams::validate` /
    // `Pipeline::new_with_spectral` before any stage runs; reaching this
    // point with an unknown family is a programming error.
    spektrafilm_model::diffusion::apply_diffusion_filter_um(image, filter, pixel_size_um)
        .expect("diffusion filter family validated at pipeline entry")
}

/// Expose: convert RGB to film raw exposure.
///
/// Dispatches on the configured upsampler: the Mallett2019 per-pixel matrix
/// (when `mallett_core` is set), the full Hanatos2025 spectral path (when a
/// TC LUT is provided), or a simplified RGB → log10 fallback.
///
/// `pixel_size_um` is the working-geometry pitch derived by
/// [`crate::resizing::crop_and_rescale`] from the full (pre-crop)
/// image; `ae_ev` is the auto-exposure EV metered on the full input
/// (Python meters before crop/resize and scales the encoded source in
/// `_preprocess`; `Pipeline::apply_autoexposure` owns that multiply, so
/// this in-stage scaling stays a no-op for pipeline callers).
#[allow(clippy::too_many_arguments)]
pub fn expose(
    image: &ImageBuf,
    _film: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    tc_lut: Option<&TcLut>,
    mallett_core: Option<&[[f64; 3]; 3]>,
    front_illuminant: &[f32],
    pixel_size_um: f64,
    ae_ev: f64,
) -> Result<ImageBuf, String> {
    let pix_um = pixel_size_um as f32;
    let input_space = colorspace::resolve(&params.io.input_color_space)
        .expect("input color space must be validated before filming");

    // Auto-exposure EV is metered upstream on the full input (the meter
    // previews/decodes internally in `small_preview` order).
    let mut rgb = image.clone();
    if params.io.input_cctf_decoding {
        let _decode = super::StageObservation::cpu(
            backend,
            "input_transfer",
            spektrafilm_backend::telemetry::CpuReason::InputTransferDecoding,
        );
        rgb.data.par_iter_mut().for_each(|v| {
            *v = from_f64(colorspace::cctf_decode(to_f64(*v), input_space.cctf));
        });
    }

    // Auto-exposure scales linear irradiance after metering decoded luminance.
    if params.camera.auto_exposure {
        let scale = from_f64(2.0f64.powf(ae_ev));
        rgb.data.par_iter_mut().for_each(|v| *v *= scale);
    }

    // RGB → film raw exposure
    let mut raw = if let Some(core) = mallett_core {
        let _mallett = super::StageObservation::cpu(
            backend,
            "rgb_to_raw",
            spektrafilm_backend::telemetry::CpuReason::Mallett,
        );
        // Mallett2019: per-pixel `raw = (core · M_cs) · rgb`, a single 3×3
        // matrix folding the input-colour-space → linear-sRGB conversion.
        let m = crate::mallett::film_matrix(core, &params.io.input_color_space);
        apply_mallett_matrix(&rgb, &m)
    } else if let Some(lut) = tc_lut {
        // Full Hanatos2025 spectral upsampling with CAT02 adaptation.
        backend.hanatos2025_rgb_to_raw(
            &rgb,
            lut,
            &params.io.input_color_space,
            front_illuminant,
            params.settings.use_cat16,
        )?
    } else {
        // Simplified fallback: treat RGB values as proportional to raw exposure
        rgb.clone()
    };

    // Python applies camera compensation to film raw, before optical effects.
    let exp_comp = from_f32(2.0f32.powf(params.camera.exposure_compensation_ev));
    raw.data.par_iter_mut().for_each(|v| *v *= exp_comp);

    // Order mirrors Python filming: boost → diffusion → lens_blur → halation.

    // Highlight boost: reconstruct pre-clip highlight irradiance before the
    // optical-scatter effects. No-op when boost_ev == 0.
    let hal_boost = &params.film_render.halation;
    if hal_boost.boost_ev != 0.0 {
        let _boost = super::StageObservation::cpu(
            backend,
            "highlight_boost",
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
        );
        raw = spektrafilm_model::optics::boost_highlights(
            &raw,
            hal_boost.boost_ev as f64,
            hal_boost.boost_range as f64,
            hal_boost.protect_ev as f64,
        );
    }

    // Diffusion filter (camera): lens diffusion-filter PSF on linear raw.
    // Preview and export use the exact sampled PSF convolution. A device
    // convolution is the same discrete PSF and the same reflect extension;
    // a backend without one keeps the reference host convolution.
    let df = &params.camera.diffusion_filter;
    if df.active {
        let dm = df.to_model();
        // A device convolution evaluates the same discrete PSF; the host
        // resolves the taps from the filter family so both routes use one
        // authoritative kernel. The family is validated at pipeline entry,
        // so a zero scatter fraction (an effective no-op) is the only
        // resolution failure left, and that is a pass-through on both routes.
        let resolved = spektrafilm_model::diffusion::resolve_psf(
            &dm,
            pixel_size_um,
            raw.width as usize,
            raw.height as usize,
        )
        .ok();
        raw = match resolved {
            Some((psf, radius, scatter_fraction)) => {
                let params = spektrafilm_backend::OpticalDiffusionGpuParams {
                    psf: &psf,
                    radius: radius as u32,
                    scatter_fraction,
                };
                match backend.optical_diffusion(&raw, &params) {
                    Ok(Some(diffused)) => {
                        let _device = super::StageObservation::new(backend, "optical_diffusion");
                        diffused
                    }
                    Ok(None) if backend.is_gpu() => {
                        return Err(
                            "optical_diffusion: selected GPU has no camera diffusion operation"
                                .into(),
                        );
                    }
                    Ok(None) => host_diffusion(&raw, &dm, pixel_size_um, backend),
                    Err(error) => return Err(error),
                }
            }
            None => raw,
        };
    }

    // Lens blur
    if params.camera.lens_blur_um > 0.0 {
        raw = spektrafilm_model::optics::apply_gaussian_blur_um(
            &raw,
            params.camera.lens_blur_um,
            pix_um,
            backend,
        )?;
    }

    // Halation (on linear raw)
    let halation = &params.film_render.halation;
    if halation.active {
        raw = spektrafilm_model::halation::apply_halation_um(
            &raw,
            pix_um,
            halation.scatter_amount,
            halation.scatter_spatial_scale,
            halation.scatter_core_um,
            halation.scatter_tail_um,
            halation.scatter_tail_weight,
            halation.halation_amount,
            halation.halation_spatial_scale,
            halation.source_compression,
            halation.halation_strength,
            halation.halation_first_sigma_um,
            halation.halation_n_bounces,
            halation.halation_bounce_decay,
            halation.halation_renormalize,
            backend,
        );
    }

    // Convert to log10 exposure. Mirror Python's
    // `np.log10(np.fmax(raw, 0.0) + 1e-10)` exactly — adding 1e-10
    // after the floor-at-zero shifts every value by a constant in
    // log-space and is *not* the same as `log10(max(raw, 1e-10))`.
    let zero = from_f64(0.0);
    let eps = from_f64(1e-10);
    raw.data.par_iter_mut().for_each(|v| {
        *v = ((*v).max(zero) + eps).log10();
    });

    Ok(raw)
}

/// Develop: log_raw → density_cmy via density curves + DIR couplers + grain.
///
/// `pixel_size_um` is the working-geometry pitch from
/// [`crate::resizing::crop_and_rescale`] (Python's
/// `ResizingService.pixel_size_um`).
pub fn develop(
    log_raw: &ImageBuf,
    film: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    pixel_size_um: f64,
) -> Result<ImageBuf, String> {
    let stage_timings = stage_timings_enabled();
    let pix_um = pixel_size_um as f32;
    let curves = crate::chain_prep::FilmCurves::prepare(film);
    let log_exposure_f64 = curves.log_exposure;
    let density_curves_f64 = &curves.raw;
    let norm_curves_f64 = &curves.normalized;
    let gamma = params.film_render.density_curve_gamma;
    let t = Instant::now();
    let mut density_cmy =
        backend.density_curve_interp(log_raw, log_exposure_f64, norm_curves_f64, gamma as f64)?;
    print_stage_timing(stage_timings, "filming_develop.density_interp", t);

    // DIR couplers
    let dir = &params.film_render.dir_couplers;
    if dir.active {
        let t = Instant::now();
        let matrix = crate::chain_prep::dir_matrix(dir);
        density_cmy = spektrafilm_model::couplers::apply_density_correction(
            &density_cmy,
            log_raw,
            pix_um,
            log_exposure_f64,
            density_curves_f64,
            &matrix,
            dir.amount,
            dir.diffusion_size_um,
            dir.diffusion_tail_um,
            dir.diffusion_tail_weight,
            film.is_positive(),
            gamma,
            dir.langmuir_donor_k_rgb,
            dir.langmuir_receiver_k_rgb,
            backend,
        )?;
        print_stage_timing(stage_timings, "filming_develop.dir_couplers", t);
    }

    // Grain samples the developed density through the upstream sublayer model.
    let grain = &params.film_render.grain;
    if grain.active && matches!(grain.engine, crate::params::grain::GrainEngine::Spektra) {
        let t = Instant::now();
        let grain_observation = super::StageObservation::new(backend, "grain_v1");
        let backend = grain_observation.backend(backend);
        density_cmy = apply_spektra_grain(
            density_cmy,
            film,
            grain,
            norm_curves_f64,
            params.settings.use_fast_stats,
            params.random_seed,
            pixel_size_um,
            backend,
        )?;
        print_stage_timing(stage_timings, "filming_develop.grain", t);
    }

    Ok(density_cmy)
}

struct PreparedSpektra {
    layers_tensor: Vec<[[f64; 3]; 3]>,
    grain: spektrafilm_model::grain::v1::PreparedGrain,
}

fn prepare_spektra(
    film: &Profile,
    grain: &crate::params::grain::GrainParams,
    norm_curves_f64: &[[f64; 3]],
    pixel_size_um: f64,
) -> Result<PreparedSpektra, String> {
    let source_layers = &film.data.density_curves_layers;
    let (layers_tensor, layer_count) = if source_layers.is_empty() {
        (
            norm_curves_f64
                .iter()
                .map(|&curve| [curve, [0.0; 3], [0.0; 3]])
                .collect(),
            1,
        )
    } else {
        let count = source_layers[0].len();
        if !(1..=3).contains(&count) || source_layers.iter().any(|row| row.len() != count) {
            return Err("Spektra grain requires one to three consistent sublayers".into());
        }
        (film.density_curves_layers_f64(), count as u32)
    };
    if layers_tensor.len() != norm_curves_f64.len() || norm_curves_f64.len() < 2 {
        return Err("Spektra grain requires matching composite and sublayer curve tables".into());
    }
    let prepared = spektrafilm_model::grain::v1::prepare_grain(
        &layers_tensor,
        layer_count,
        pixel_size_um,
        grain.rms_granularity,
        grain.particle_scale_sublayers,
        grain.density_min,
        grain.uniformity,
    )?;
    Ok(PreparedSpektra {
        layers_tensor,
        grain: prepared,
    })
}

/// Resolve the same curve-based RMS preparation used by the CPU sampler.
pub(crate) fn spektra_grain_device_params(
    film: &Profile,
    grain: &crate::params::grain::GrainParams,
    norm_curves_f64: &[[f64; 3]],
    use_fast_stats: bool,
    random_seed: u64,
    pixel_size_um: f64,
) -> Result<spektrafilm_backend::GrainV1GpuParams, String> {
    let PreparedSpektra {
        layers_tensor,
        grain: prepared,
    } = prepare_spektra(film, grain, norm_curves_f64, pixel_size_um)?;
    let sign = if film.is_positive() { -1.0 } else { 1.0 };
    let mut curve_x = Vec::with_capacity(norm_curves_f64.len() * 3);
    for row in norm_curves_f64 {
        for &value in row {
            curve_x.push(value * sign);
        }
    }
    let mut curve_layers = Vec::with_capacity(layers_tensor.len() * 9);
    for row in &layers_tensor {
        for layer in row {
            curve_layers.extend_from_slice(layer);
        }
    }
    Ok(spektrafilm_backend::GrainV1GpuParams {
        base_seed: random_seed,
        layer_count: prepared.layer_count,
        density_min: prepared.density_min,
        uniformity: prepared.uniformity,
        density_min_layers: std::array::from_fn(|i| prepared.density_min_layers[i / 3][i % 3]),
        density_max_adj: std::array::from_fn(|i| prepared.density_max_adj[i / 3][i % 3]),
        n_particles: std::array::from_fn(|i| prepared.n_particles[i / 3][i % 3]),
        monochrome: grain.monochrome,
        lookup_sign: sign as f32,
        blur_dye_clouds: grain.blur_dye_clouds_um,
        reference_f64: std::mem::size_of::<spektrafilm_math::precision::Scalar>() == 8,
        micro_sigma: grain.micro_structure[1] * 0.001 / pixel_size_um,
        micro_blur_px: grain.micro_structure[0] / pixel_size_um,
        grain_blur: grain.blur,
        usm_sigma: grain.mult_usm_sigma,
        usm_amount: grain.mult_usm_amount,
        curve_x,
        curve_layers,
        mt19937_regime: !use_fast_stats,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_spektra_grain(
    density_cmy: ImageBuf,
    film: &Profile,
    grain: &crate::params::grain::GrainParams,
    norm_curves_f64: &[[f64; 3]],
    use_fast_stats: bool,
    random_seed: u64,
    pixel_size_um: f64,
    backend: &dyn ComputeBackend,
) -> Result<ImageBuf, String> {
    let prepared = prepare_spektra(film, grain, norm_curves_f64, pixel_size_um)?;
    spektrafilm_model::grain::v1::apply_grain(
        &density_cmy,
        norm_curves_f64,
        &prepared.layers_tensor,
        &prepared.grain,
        pixel_size_um,
        grain.blur,
        grain.blur_dye_clouds_um,
        grain.micro_structure,
        grain.mult_usm_sigma,
        grain.mult_usm_amount,
        film.is_positive(),
        grain.monochrome,
        use_fast_stats,
        random_seed,
        backend,
    )
}

/// Full filming stage: expose + develop on an already-prepared working
/// image. Geometry (crop/upscale) is the caller's responsibility — see
/// [`crate::resizing::crop_and_rescale`]; the pixel pitch passed here
/// must be the one derived from the full input.
pub fn process(
    image: &ImageBuf,
    film: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    tc_lut: Option<&TcLut>,
    pixel_size_um: f64,
) -> Result<ImageBuf, String> {
    let ref_illuminant = select_illuminant(&film.info.reference_illuminant);
    let ae_ev = meter_autoexposure_ev(image, params);
    let log_raw = expose(
        image,
        film,
        params,
        backend,
        tc_lut,
        None,
        &ref_illuminant,
        pixel_size_um,
        ae_ev,
    )?;
    develop(&log_raw, film, params, backend, pixel_size_um)
}

use crate::spectral_service::select_illuminant;

#[cfg(test)]
mod tests {
    #[cfg(test)]
    mod grain_dispatch {
        use super::super::*;
        use crate::params::RuntimeParams;
        use crate::profile::{self, Profile};
        use spektrafilm_backend::cpu_backend::CpuBackend;
        use spektrafilm_math::image::ImageBuf;
        use spektrafilm_math::precision::{Scalar, from_f64, to_f64};

        fn data_dir() -> std::path::PathBuf {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("data")
        }

        fn portra() -> Profile {
            static FILM: std::sync::LazyLock<Profile> = std::sync::LazyLock::new(|| {
                let path = data_dir().join("profiles/kodak_portra_400.json");
                profile::load_profile(&path).expect("Portra test fixture required")
            });
            FILM.clone()
        }

        /// Small log-exposure image spread across the curve axis.
        fn log_raw() -> ImageBuf {
            let (w, h) = (16usize, 12usize);
            let mut data = Vec::with_capacity(w * h * 3);
            for y in 0..h {
                for x in 0..w {
                    let t = (x + y * w) as f64 / (w * h) as f64;
                    data.push(from_f64(-3.5 + 3.0 * t));
                    data.push(from_f64(-3.2 + 3.0 * t));
                    data.push(from_f64(-2.9 + 3.0 * t));
                }
            }
            ImageBuf {
                width: w as u32,
                height: h as u32,
                data,
            }
        }

        fn base_params() -> RuntimeParams {
            let mut params = RuntimeParams::default();
            params.film_render.grain.engine = crate::params::GrainEngine::Spektra;
            params.film_render.grain.rms_granularity = [6.0, 8.0, 10.0];
            params.film_render.dir_couplers.active = false;
            params.film_render.halation.active = false;
            // The small test images would otherwise imply ~2 mm pixels and
            // wash the particle noise out; 0.192 mm over the 16 px long
            // edge gives a realistic 12 µm pixel.
            params.camera.film_format_mm = 0.192;
            params
        }

        fn ch_mean(img: &ImageBuf, ch: usize) -> f64 {
            img.pixels().map(|px| to_f64(px[ch])).sum::<f64>() / img.pixel_count() as f64
        }

        fn ch_variance(img: &ImageBuf, ch: usize, mean: f64) -> f64 {
            img.pixels()
                .map(|px| {
                    let delta = to_f64(px[ch]) - mean;
                    delta * delta
                })
                .sum::<f64>()
                / img.pixel_count() as f64
        }

        /// Grain off: develop must return the plain density interpolation
        /// (deterministic LUT-mode bypass — Python's `lut_mode` forces
        /// `grain.active = False`, and the non-spatial film behavior is
        /// untouched).
        #[test]
        fn grain_disabled_is_density_identity() {
            let film = portra();
            let backend = CpuBackend;
            let mut params = base_params();
            params.film_render.grain.active = false;
            let log_raw = log_raw();

            let out = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("grain-disabled film development failed");

            let norm = spektrafilm_model::density_curves::normalize_density_curves_f64(
                &film.density_curves_f64(),
            );
            let expected = backend
                .density_curve_interp(
                    &log_raw,
                    &film.log_exposure_f64(),
                    &norm,
                    params.film_render.density_curve_gamma as f64,
                )
                .expect("reference density interpolation failed");
            assert_eq!(out.data, expected.data);
        }

        #[test]
        fn single_curve_grain_tracks_requested_rms() {
            let mut film = portra();
            film.data.density_curves_layers.clear();
            let curves: Vec<[f64; 3]> = (0..=128).map(|i| [2.2 * i as f64 / 128.0; 3]).collect();
            let mut params = base_params();
            let g = &mut params.film_render.grain;
            g.rms_granularity = [6.0, 8.0, 10.0];
            g.blur = 0.0;
            g.blur_dye_clouds_um = 0.0;
            g.micro_structure = [0.0; 2];
            g.mult_usm_amount = 0.0;
            let image = ImageBuf::from_data(128, 128, vec![from_f64(1.1); 128 * 128 * 3]);
            let aperture_area = std::f64::consts::PI * 24.0_f64.powi(2);
            let output = apply_spektra_grain(
                image,
                &film,
                g,
                &curves,
                false,
                0,
                aperture_area.sqrt(),
                &CpuBackend,
            )
            .unwrap();
            for ch in 0..3 {
                let mean = ch_mean(&output, ch);
                let sigma = ch_variance(&output, ch, mean).sqrt();
                assert!((mean - 1.1).abs() < 0.001, "channel {ch}: mean {mean}");
                let expected = g.rms_granularity[ch] / 1000.0;
                assert!(
                    (sigma - expected).abs() < expected * 0.05,
                    "channel {ch}: sigma {sigma}, target {expected}"
                );
            }
        }

        /// `use_fast_stats` switches the layered sampler's RNG regime:
        /// different texture, same statistical center (bounded comparison
        /// — bit parity is impossible for upstream's numba kernels).
        #[test]
        fn use_fast_stats_switches_regime() {
            let film = portra();
            let backend = CpuBackend;
            let log_raw = log_raw();
            let mut params = base_params();
            params.film_render.grain.blur = 0.0; // raw particle field
            params.film_render.grain.blur_dye_clouds_um = 0.0;

            params.settings.use_fast_stats = false;
            let scipy = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("SciPy-statistics film development failed");
            params.settings.use_fast_stats = true;
            let fast = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("fast-statistics film development failed");
            assert_ne!(scipy.data, fast.data);
            for ch in 0..3 {
                assert!(
                    (ch_mean(&scipy, ch) - ch_mean(&fast, ch)).abs() < 0.1,
                    "ch {ch}: {} vs {}",
                    ch_mean(&scipy, ch),
                    ch_mean(&fast, ch)
                );
            }
        }

        /// Positive colour profiles reach the layered path through the
        /// negated-axis branch of `interp_density_cmy_layers` (upstream
        /// `-density_cmy` / `-density_curves` lookup): output stays finite
        /// and the negated lookup decorrelates it from the negative-profile
        /// run of the same input.
        #[test]
        fn layered_grain_positive_profile_runs_negated_axis() {
            let path = data_dir().join("profiles/fujifilm_provia_100f.json");
            assert!(path.exists(), "test fixture required: {}", path.display());
            let film = profile::load_profile(&path).unwrap();
            assert!(film.is_positive());
            let backend = CpuBackend;
            let params = base_params();
            let log_raw = log_raw();

            let out = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("positive-profile film development failed");
            let again = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("repeated positive-profile film development failed");
            assert!(out.data.iter().all(|v| v.is_finite()));
            assert_eq!(out.data, again.data);
            // Densities came out of the curve tables (not the raw log
            // exposure), and grain actually fired.
            assert_ne!(out.data, log_raw.data);
        }

        /// A monochrome (B&W) film runs the layered path with one shared
        /// noise field: identical per-channel input stays identical after
        /// grain (regression coverage for the Rust-only B&W addition; not
        /// part of the 0.3.4 baseline).
        #[test]
        fn layered_grain_monochrome_keeps_channels_equal() {
            let path = data_dir().join("profiles/kodak_doublex.json");
            assert!(path.exists(), "test fixture required: {}", path.display());
            let film = profile::resolve_for_render(profile::load_profile(&path).unwrap(), None);
            let backend = CpuBackend;
            let mut params = base_params();
            // Mirror `Pipeline::apply_film_specific_params`: monochrome
            // flag + channel-0-flattened per-channel grain tuples, so the
            // broadcast 3-channel engine runs one shared noise field.
            params.film_render.grain.monochrome = true;
            let g = &mut params.film_render.grain;
            g.rms_granularity = [g.rms_granularity[0]; 3];
            g.density_min = [g.density_min[0]; 3];
            g.uniformity = [g.uniformity[0]; 3];
            g.blur = 0.0;
            g.blur_dye_clouds_um = 0.0;
            // 8 px long edge at 0.096 mm → 12 µm pixels.
            params.camera.film_format_mm = 0.096;

            // Identical per-channel log exposure → identical densities.
            let (w, h) = (8usize, 8usize);
            let mut data = Vec::with_capacity(w * h * 3);
            for i in 0..w * h {
                let t = i as f64 / (w * h) as f64;
                let v = from_f64(-3.0 + 2.5 * t);
                data.extend_from_slice(&[v, v, v]);
            }
            let log_raw = ImageBuf {
                width: w as u32,
                height: h as u32,
                data,
            };

            let out = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("monochrome film development failed");
            let c0: Vec<Scalar> = out.pixels().map(|px| px[0]).collect();
            let c1: Vec<Scalar> = out.pixels().map(|px| px[1]).collect();
            let c2: Vec<Scalar> = out.pixels().map(|px| px[2]).collect();
            assert_eq!(c0, c1);
            assert_eq!(c0, c2);
        }
    }
}
