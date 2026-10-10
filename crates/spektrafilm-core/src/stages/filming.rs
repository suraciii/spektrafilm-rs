/// Filming stage: expose the digital image onto virtual film.
///
/// Full Hanatos2025 path: RGB → XYZ → xy chromaticity → tc coordinates →
/// 2D LUT lookup (spectra × sensitivity) → per-channel film raw exposure.
use rayon::prelude::*;
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f32, from_f64, to_f64};
use spektrafilm_math::spectral::TcLut;
use std::time::Instant;

use crate::params::RuntimeParams;
use crate::profile::Profile;

mod metering;
pub use metering::{measure_autoexposure_ev, meter_autoexposure_ev};

fn stage_timings_enabled() -> bool {
    std::env::var_os("SPEKTRAFILM_STAGE_TIMINGS").is_some()
}

fn print_stage_timing(enabled: bool, stage: &str, start: Instant) {
    if enabled {
        eprintln!("stage {stage}: {} ms", start.elapsed().as_millis());
    }
}

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
    bw_filming_correction: f64,
    pixel_size_um: f64,
    ae_ev: f64,
) -> ImageBuf {
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
            spektrafilm_gpu::telemetry::CpuReason::InputTransferDecoding,
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
            spektrafilm_gpu::telemetry::CpuReason::Mallett,
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
        )
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
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        raw = spektrafilm_model::optics::boost_highlights(
            &raw,
            hal_boost.boost_ev as f64,
            hal_boost.boost_range as f64,
            hal_boost.protect_ev as f64,
        );
    }

    // Diffusion filter (camera): lens diffusion-filter PSF on linear raw.
    // Preview and export use the exact sampled PSF convolution.
    let df = &params.camera.diffusion_filter;
    if df.active {
        let _diffusion = super::StageObservation::cpu(
            backend,
            "optical_diffusion",
            spektrafilm_gpu::telemetry::CpuReason::OpticalDiffusion,
        );
        let dm = df.to_model();
        // An invalid family is rejected by `RuntimeParams::validate` /
        // `Pipeline::new_with_spectral` before any stage runs; reaching
        // this point with an unknown family is a programming error.
        raw = if backend.is_gpu() {
            spektrafilm_model::diffusion::apply_diffusion_filter_blur(
                &raw,
                &dm,
                pixel_size_um,
                backend,
            )
            .expect("camera diffusion filter family validated at pipeline entry")
        } else {
            spektrafilm_model::diffusion::apply_diffusion_filter_um(&raw, &dm, pixel_size_um)
                .expect("camera diffusion filter family validated at pipeline entry")
        };
    }

    // Lens blur
    if params.camera.lens_blur_um > 0.0 {
        raw = spektrafilm_model::optics::apply_gaussian_blur_um(
            &raw,
            params.camera.lens_blur_um,
            pix_um,
            backend,
        );
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
            halation.halation_strength,
            halation.halation_first_sigma_um,
            halation.halation_n_bounces,
            halation.halation_bounce_decay,
            halation.halation_renormalize,
            backend,
        );
    }

    // B&W / slide scanner exposure correction (Python applies this last,
    // before the log10). No-op at factor 1.0.
    if bw_filming_correction != 1.0 {
        let f = from_f64(bw_filming_correction);
        raw.data.par_iter_mut().for_each(|v| *v *= f);
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

    raw
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
) -> ImageBuf {
    let stage_timings = stage_timings_enabled();
    let pix_um = pixel_size_um as f32;
    let curves = crate::chain_prep::FilmCurves::prepare(film);
    let log_exposure_f64 = curves.log_exposure;
    let density_curves_f64 = &curves.raw;
    let norm_curves_f64 = &curves.normalized;
    let gamma = params.film_render.density_curve_gamma;
    let t = Instant::now();
    let mut density_cmy =
        backend.density_curve_interp(log_raw, &log_exposure_f64, &norm_curves_f64, gamma as f64);
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
            &log_exposure_f64,
            &density_curves_f64,
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
        );
        print_stage_timing(stage_timings, "filming_develop.dir_couplers", t);
    }

    // Grain — Python `apply_grain` dispatch (model/grain.py):
    // `sublayers_active == false` keeps the composite-density sampler,
    // `true` runs the layered model on the interpolated sublayer densities.
    // `n_sub_layers` is only consumed by the composite path (the layered
    // model always samples the profile's 3 emulsion sublayers) and
    // `use_fast_stats` only by the layered path, exactly like upstream.
    let grain = &params.film_render.grain;
    if grain.active && matches!(grain.engine, crate::params::grain::GrainEngine::V1) {
        let t = Instant::now();
        let grain_observation = super::StageObservation::new(backend, "grain_v1");
        let backend = grain_observation.backend(backend);
        // Use f64 throughout — Python reads these from JSON as f64; the
        // f32 storage in `GrainParams` would otherwise truncate to ~7
        // decimals and shift every Poisson lambda by ~5e-8, producing a
        // visibly different grain pattern.
        // The upstream grain sampler uses `particle_area_um2` directly.
        // `rms_granularity` is a profile/UI control only; it does not feed
        // `apply_grain` in the Python runtime.
        let particle_area_um2 = grain.particle_area_um2;
        if grain.sublayers_active {
            // Python `apply_grain_to_density_layers`: sublayer densities
            // come from interpolating the composite density against the
            // (normalized) composite curve, the layer maxima from the RAW
            // `density_curves_layers` tensor — upstream normalizes only
            // the composite curves before this call.
            let layers_tensor = film.density_curves_layers_f64();
            assert!(
                !layers_tensor.is_empty(),
                "grain.sublayers_active requires the film profile to provide \
                 density_curves_layers ([n][3 sublayers][3 channels]); profile '{}' \
                 has none — disable sublayers_active or fix the profile",
                film.info.stock.as_deref().unwrap_or("<unnamed>"),
            );
            let density_cmy_layers = spektrafilm_model::density_curves::interp_density_cmy_layers(
                &density_cmy,
                &norm_curves_f64,
                &layers_tensor,
                film.is_positive(),
            );
            let density_max_layers =
                spektrafilm_model::density_curves::density_max_layers_f64(&layers_tensor);
            assert!(
                density_max_layers
                    .iter()
                    .all(|row| row.iter().all(|&v| v > 0.0)),
                "grain.sublayers_active requires positive per-sublayer density \
                 maxima; profile '{}' yields zero maxima (empty or all-zero \
                 density_curves_layers)",
                film.info.stock.as_deref().unwrap_or("<unnamed>"),
            );
            // Python multiplies the per-channel particle area by
            // `particle_scale` before applying the sublayer scale.
            density_cmy = spektrafilm_model::grain::v1::apply_grain_to_density_layers(
                &density_cmy_layers,
                &density_max_layers,
                density_cmy.width,
                density_cmy.height,
                pixel_size_um,
                [
                    particle_area_um2 * grain.particle_scale[0],
                    particle_area_um2 * grain.particle_scale[1],
                    particle_area_um2 * grain.particle_scale[2],
                ],
                grain.particle_scale_sublayers,
                grain.density_min,
                grain.uniformity,
                grain.blur,
                grain.blur_dye_clouds_um,
                grain.micro_structure,
                grain.mult_usm_sigma,
                grain.mult_usm_amount,
                grain.monochrome,
                params.settings.use_fast_stats,
                params.random_seed,
                backend,
            );
        } else {
            let density_max = spektrafilm_model::density_curves::max_density_f64(&norm_curves_f64);
            density_cmy = spektrafilm_model::grain::v1::apply_grain_to_density(
                &density_cmy,
                pixel_size_um,
                particle_area_um2,
                grain.particle_scale,
                grain.density_min,
                density_max,
                grain.uniformity,
                grain.blur,
                grain.n_sub_layers,
                grain.monochrome,
                params.random_seed,
                backend,
            );
        }
        print_stage_timing(stage_timings, "filming_develop.grain", t);
    }

    density_cmy
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
) -> ImageBuf {
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
        1.0,
        pixel_size_um,
        ae_ev,
    );
    develop(&log_raw, film, params, backend, pixel_size_um)
}

use crate::spectral_service::select_illuminant;

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    mod grain_dispatch {
        use super::super::*;
        use crate::params::RuntimeParams;
        use crate::profile::{self, Profile};
        use spektrafilm_gpu::cpu_backend::CpuBackend;
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

        fn portra() -> Option<Profile> {
            let path = data_dir().join("profiles/kodak_portra_400.json");
            if !path.exists() {
                eprintln!("Skipping test — profile not found at {}", path.display());
                return None;
            }
            Some(profile::load_profile(&path).unwrap())
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
            let Some(film) = portra() else { return };
            let backend = CpuBackend;
            let mut params = base_params();
            params.film_render.grain.active = false;
            let log_raw = log_raw();

            let out = develop(&log_raw, &film, &params, &backend, 12.0);

            let norm = spektrafilm_model::density_curves::normalize_density_curves_f64(
                &film.density_curves_f64(),
            );
            let expected = backend.density_curve_interp(
                &log_raw,
                &film.log_exposure_f64(),
                &norm,
                params.film_render.density_curve_gamma as f64,
            );
            assert_eq!(out.data, expected.data);
        }

        /// The sublayer toggle must switch models: layered and composite
        /// outputs differ, and each is deterministic across runs.
        #[test]
        fn sublayers_toggle_switches_models() {
            let Some(film) = portra() else { return };
            let backend = CpuBackend;
            let log_raw = log_raw();
            let mut params = base_params();
            params.film_render.grain.sublayers_active = true;
            let layered = develop(&log_raw, &film, &params, &backend, 12.0);
            let layered_again = develop(&log_raw, &film, &params, &backend, 12.0);
            assert_eq!(layered.data, layered_again.data);

            params.film_render.grain.sublayers_active = false;
            let composite = develop(&log_raw, &film, &params, &backend, 12.0);
            let composite_again = develop(&log_raw, &film, &params, &backend, 12.0);
            assert_eq!(composite.data, composite_again.data);
            assert_ne!(layered.data, composite.data);
        }

        /// Layered grain must honor the per-channel particle scale. Larger
        /// particles reduce the particle count and increase that channel's
        /// density variance, matching Python's channel-wise area scaling.
        #[test]
        fn layered_grain_particle_scale_changes_channel_variance() {
            let Some(film) = portra() else { return };
            let backend = CpuBackend;
            let log_raw = log_raw();
            let mut params = base_params();
            params.film_render.grain.blur = 0.0;
            params.film_render.grain.blur_dye_clouds_um = 0.0;
            params.film_render.grain.mult_usm_amount = 0.0;
            params.film_render.grain.particle_scale = [1.0, 1.0, 1.0];
            let small_particles = develop(&log_raw, &film, &params, &backend, 12.0);
            params.film_render.grain.particle_scale[0] = 8.0;
            let large_red_particles = develop(&log_raw, &film, &params, &backend, 12.0);

            let small_mean = ch_mean(&small_particles, 0);
            let large_mean = ch_mean(&large_red_particles, 0);
            let small_variance = ch_variance(&small_particles, 0, small_mean);
            let large_variance = ch_variance(&large_red_particles, 0, large_mean);
            assert!(
                large_variance > small_variance,
                "larger particles should increase red grain variance: {large_variance} <= {small_variance}"
            );
            assert!(
                (large_mean - small_mean).abs() < 0.15,
                "particle scale must preserve expected density: {large_mean} vs {small_mean}"
            );
        }

        /// `n_sub_layers` is a composite-path control (upstream consumes it
        /// only in `apply_grain_to_density`): 1 vs 3 sub-layers must differ
        /// there, and must not touch the layered path.
        #[test]
        fn n_sub_layers_is_composite_only() {
            let Some(film) = portra() else { return };
            let backend = CpuBackend;
            let log_raw = log_raw();

            let mut params = base_params();
            params.film_render.grain.sublayers_active = false;
            params.film_render.grain.n_sub_layers = 1;
            let one = develop(&log_raw, &film, &params, &backend, 12.0);
            params.film_render.grain.n_sub_layers = 3;
            let three = develop(&log_raw, &film, &params, &backend, 12.0);
            assert_ne!(one.data, three.data);

            params.film_render.grain.sublayers_active = true;
            params.film_render.grain.n_sub_layers = 1;
            let l1 = develop(&log_raw, &film, &params, &backend, 12.0);
            params.film_render.grain.n_sub_layers = 4;
            let l4 = develop(&log_raw, &film, &params, &backend, 12.0);
            assert_eq!(l1.data, l4.data);
        }

        /// `use_fast_stats` switches the layered sampler's RNG regime:
        /// different texture, same statistical center (bounded comparison
        /// — bit parity is impossible for upstream's numba kernels).
        #[test]
        fn use_fast_stats_switches_regime() {
            let Some(film) = portra() else { return };
            let backend = CpuBackend;
            let log_raw = log_raw();
            let mut params = base_params();
            params.film_render.grain.blur = 0.0; // raw particle field
            params.film_render.grain.blur_dye_clouds_um = 0.0;

            params.settings.use_fast_stats = false;
            let scipy = develop(&log_raw, &film, &params, &backend, 12.0);
            params.settings.use_fast_stats = true;
            let fast = develop(&log_raw, &film, &params, &backend, 12.0);
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
            if !path.exists() {
                eprintln!("Skipping test — profile not found at {}", path.display());
                return;
            }
            let film = profile::load_profile(&path).unwrap();
            assert!(film.is_positive());
            let backend = CpuBackend;
            let params = base_params();
            let log_raw = log_raw();

            let out = develop(&log_raw, &film, &params, &backend, 12.0);
            let again = develop(&log_raw, &film, &params, &backend, 12.0);
            assert!(out.data.iter().all(|v| v.is_finite()));
            assert_eq!(out.data, again.data);
            // Densities came out of the curve tables (not the raw log
            // exposure), and grain actually fired.
            assert_ne!(out.data, log_raw.data);
        }

        /// Layered grain on a profile without layer curves fails with an
        /// actionable message instead of producing garbage — Python's
        /// `np.nanmax` on the empty tensor raises for the same condition.
        #[test]
        #[should_panic(expected = "grain.sublayers_active requires the film profile")]
        fn sublayers_without_layer_curves_fails() {
            let Some(mut film) = portra() else { return };
            film.data.density_curves_layers.clear();
            let backend = CpuBackend;
            let params = base_params();
            let _ = develop(&log_raw(), &film, &params, &backend, 12.0);
        }

        /// A monochrome (B&W) film runs the layered path with one shared
        /// noise field: identical per-channel input stays identical after
        /// grain (regression coverage for the Rust-only B&W addition; not
        /// part of the 0.3.4 baseline).
        #[test]
        fn layered_grain_monochrome_keeps_channels_equal() {
            let path = data_dir().join("profiles/kodak_doublex.json");
            if !path.exists() {
                eprintln!("Skipping test — profile not found at {}", path.display());
                return;
            }
            let film = profile::resolve_for_render(profile::load_profile(&path).unwrap(), None);
            let backend = CpuBackend;
            let mut params = base_params();
            // Mirror `Pipeline::apply_film_specific_params`: monochrome
            // flag + channel-0-flattened per-channel grain tuples, so the
            // broadcast 3-channel engine runs one shared noise field.
            params.film_render.grain.monochrome = true;
            let g = &mut params.film_render.grain;
            g.particle_scale = [g.particle_scale[0]; 3];
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

            let out = develop(&log_raw, &film, &params, &backend, 12.0);
            let c0: Vec<Scalar> = out.pixels().map(|px| px[0]).collect();
            let c1: Vec<Scalar> = out.pixels().map(|px| px[1]).collect();
            let c2: Vec<Scalar> = out.pixels().map(|px| px[2]).collect();
            assert_eq!(c0, c1);
            assert_eq!(c0, c2);
        }
    }
}
