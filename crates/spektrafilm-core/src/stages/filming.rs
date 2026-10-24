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
                    Ok(None) if backend.is_gpu() => return Err("optical_diffusion: selected GPU has no camera diffusion operation".into()),
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

    // Grain — Python `apply_grain` dispatch (model/grain.py):
    // `sublayers_active == false` keeps the composite-density sampler,
    // `true` runs the layered model on the interpolated sublayer densities.
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

/// Profile sublayer curves and their per-sublayer, per-channel maxima.
struct SpektraLayerTables {
    layers_tensor: Vec<[[f64; 3]; 3]>,
    density_max_layers: [[f64; 3]; 3],
}

/// The sublayer density tables the device pass reads, or the reason this film
/// profile cannot realize the request.
///
/// One rule for both routes: `spektra_grain_device_params` builds from the
/// returned tables, and the explicit-GPU preflight reports the reason, so the
/// preflight cannot contradict the executed route.
fn spektra_layer_tables(film: &Profile) -> Result<SpektraLayerTables, &'static str> {
    let layers_tensor = film.density_curves_layers_f64();
    if layers_tensor.is_empty() {
        return Err("the film profile supplies no sublayer density curves");
    }
    let density_max_layers =
        spektrafilm_model::density_curves::density_max_layers_f64(&layers_tensor);
    if density_max_layers
        .iter()
        .any(|row| {
            row.iter()
                .any(|v| v.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater))
        })
    {
        return Err("the film profile's per-sublayer density maxima are not positive");
    }
    Ok(SpektraLayerTables {
        layers_tensor,
        density_max_layers,
    })
}

/// Why the resident chain cannot resolve Spektra grain on the device for this
/// film profile, or `None` when it can realize the request.
pub(crate) fn spektra_device_gap(film: &Profile) -> Option<&'static str> {
    spektra_layer_tables(film).err()
}

/// Resolve the device Spektra v1 grain request for the resident chain, or
/// `None` when the film profile cannot supply the curves the pass reads.
///
/// The device realizes both sampling regimes. The layered `fast_stats` regime
/// (`spektrafilm_model::grain::v1::apply_grain_to_density_layers` with
/// `use_fast_stats`) is a per-pixel mapping over independent `FastStatsRng`
/// streams. The composite path hardcodes the scipy MT19937 regime upstream
/// regardless of the setting, and the layered path uses it whenever
/// `use_fast_stats` is false: one sequential generator advanced across the
/// whole array with variable-length rejection consumption. The device
/// reproduces that draw order exactly — all Poisson draws across the plane
/// first, then all binomial draws — so it never substitutes a different noise
/// law (`specs/gpu-execution/design.md` §Stochastic alignment).
///
/// Every f64 control is resolved here on the host from the profile and the
/// request, exactly as the host stage resolves them, so the device pass
/// performs no image-dependent arithmetic of its own.
pub(crate) fn spektra_grain_device_params(
    film: &Profile,
    grain: &crate::params::grain::GrainParams,
    norm_curves_f64: &[[f64; 3]],
    use_fast_stats: bool,
    random_seed: u64,
    pixel_size_um: f64,
) -> Option<spektrafilm_backend::GrainV1GpuParams> {
    let SpektraLayerTables {
        layers_tensor,
        density_max_layers,
    } = spektra_layer_tables(film).ok()?;
    // Python: density_max_total = sum(density_max_layers, axis=sublayer);
    //         fractions = density_max_layers / density_max_total;
    //         density_min_layers = fractions * density_min;
    //         density_max_layers += density_min_layers;
    //         n_particles = pixel_area * fraction / particle_area_layer.
    let pixel_area = pixel_size_um * pixel_size_um;
    let mut total = [0.0f64; 3];
    for layer in &density_max_layers {
        for ch in 0..3 {
            total[ch] += layer[ch];
        }
    }
    let mut density_min_layers = [0.0f64; 9];
    let mut density_max_adj = [0.0f64; 9];
    let mut n_particles = [0.0f64; 9];
    for (sl, layer) in density_max_layers.iter().enumerate() {
        for (ch, &density_max) in layer.iter().enumerate() {
            let slot = sl * 3 + ch;
            let fraction = density_max / total[ch];
            density_min_layers[slot] = fraction * grain.density_min[ch];
            density_max_adj[slot] = density_max + density_min_layers[slot];
            // Python multiplies the per-channel particle area by
            // `particle_scale` before applying the sublayer scale.
            let particle_area =
                grain.particle_area_um2 * grain.particle_scale[ch] * grain.particle_scale_sublayers[sl];
            n_particles[slot] = pixel_area * fraction / particle_area;
        }
    }
    // The composite curve x column per channel, negated for a positive film —
    // the same axis `interp_density_cmy_layers` looks up against.
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
            for &value in layer {
                curve_layers.push(value);
            }
        }
    }
    Some(spektrafilm_backend::GrainV1GpuParams {
        base_seed: random_seed,
        pixel_size_um,
        particle_area: [
            grain.particle_area_um2 * grain.particle_scale[0],
            grain.particle_area_um2 * grain.particle_scale[1],
            grain.particle_area_um2 * grain.particle_scale[2],
        ],
        particle_scale_layers: grain.particle_scale_sublayers,
        density_min: grain.density_min,
        uniformity: grain.uniformity,
        density_min_layers,
        density_max_adj,
        n_particles,
        monochrome: grain.monochrome,
        lookup_sign: sign as f32,
        blur_dye_clouds: grain.blur_dye_clouds_um,
        reference_f64: std::mem::size_of::<spektrafilm_math::precision::Scalar>() == 8,
        micro_sigma: grain.micro_structure[1] as f64 * 0.001 / pixel_size_um,
        micro_blur_px: grain.micro_structure[0] as f64 / pixel_size_um,
        grain_blur: grain.blur,
        usm_sigma: grain.mult_usm_sigma,
        usm_amount: grain.mult_usm_amount,
        curve_x,
        curve_layers,
        sublayers_active: grain.sublayers_active,
        n_sub_layers: grain.n_sub_layers,
        mt19937_regime: !grain.sublayers_active || !use_fast_stats,
        density_max_composite: spektrafilm_model::density_curves::max_density_f64(norm_curves_f64),
        n_particles_composite: std::array::from_fn(|ch| {
            let count = pixel_area / (grain.particle_area_um2 * grain.particle_scale[ch]);
            if grain.n_sub_layers > 1 { count / grain.n_sub_layers as f64 } else { count }
        }),
    })
}

/// Spektra composite/layered grain on the developed film density (Python
/// `apply_grain` dispatch). Extracted so the resident GPU chain can run it
/// as an assembled CPU fence on the density buffer (pipe architecture,
/// capability class).
///
/// Uses f64 control values throughout — Python reads these from JSON as
/// f64; the f32 storage in `GrainParams` would otherwise truncate to ~7
/// decimals and shift every Poisson lambda by ~5e-8, producing a visibly
/// different grain pattern. The upstream sampler uses `particle_area_um2`
/// directly; `rms_granularity` is a profile/UI control only.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_spektra_grain(
    mut density_cmy: ImageBuf,
    film: &Profile,
    grain: &crate::params::grain::GrainParams,
    norm_curves_f64: &[[f64; 3]],
    use_fast_stats: bool,
    random_seed: u64,
    pixel_size_um: f64,
    backend: &dyn ComputeBackend,
) -> Result<ImageBuf, String> {
    let particle_area_um2 = grain.particle_area_um2;
    if grain.sublayers_active {
        // Python `apply_grain_to_density_layers`: sublayer densities come
        // from interpolating the composite density against the
        // (normalized) composite curve, the layer maxima from the RAW
        // `density_curves_layers` tensor.
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
            norm_curves_f64,
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
            use_fast_stats,
            random_seed,
            backend,
        )?;
    } else {
        let density_max = spektrafilm_model::density_curves::max_density_f64(norm_curves_f64);
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
            random_seed,
            backend,
        )?;
    }
    Ok(density_cmy)
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
            let path = data_dir().join("profiles/kodak_portra_400.json");
            assert!(path.exists(), "test fixture required: {}", path.display());
            profile::load_profile(&path).unwrap()
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
            let expected = backend.density_curve_interp(
                &log_raw,
                &film.log_exposure_f64(),
                &norm,
                params.film_render.density_curve_gamma as f64,
            ).expect("reference density interpolation failed");
            assert_eq!(out.data, expected.data);
        }

        /// The sublayer toggle must switch models: layered and composite
        /// outputs differ, and each is deterministic across runs.
        #[test]
        fn sublayers_toggle_switches_models() {
            let film = portra();
            let backend = CpuBackend;
            let log_raw = log_raw();
            let mut params = base_params();
            params.film_render.grain.sublayers_active = true;
            let layered = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("layered film development failed");
            let layered_again = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("repeated layered film development failed");
            assert_eq!(layered.data, layered_again.data);

            params.film_render.grain.sublayers_active = false;
            let composite = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("composite film development failed");
            let composite_again = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("repeated composite film development failed");
            assert_eq!(composite.data, composite_again.data);
            assert_ne!(layered.data, composite.data);
        }

        /// Layered grain must honor the per-channel particle scale. Larger
        /// particles reduce the particle count and increase that channel's
        /// density variance, matching Python's channel-wise area scaling.
        #[test]
        fn layered_grain_particle_scale_changes_channel_variance() {
            let film = portra();
            let backend = CpuBackend;
            let log_raw = log_raw();
            let mut params = base_params();
            params.film_render.grain.blur = 0.0;
            params.film_render.grain.blur_dye_clouds_um = 0.0;
            params.film_render.grain.mult_usm_amount = 0.0;
            params.film_render.grain.particle_scale = [1.0, 1.0, 1.0];
            let small_particles = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("small-particle film development failed");
            params.film_render.grain.particle_scale[0] = 8.0;
            let large_red_particles = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("large-red-particle film development failed");

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
            let film = portra();
            let backend = CpuBackend;
            let log_raw = log_raw();

            let mut params = base_params();
            params.film_render.grain.sublayers_active = false;
            params.film_render.grain.n_sub_layers = 1;
            let one = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("single-sublayer composite film development failed");
            params.film_render.grain.n_sub_layers = 3;
            let three = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("three-sublayer composite film development failed");
            assert_ne!(one.data, three.data);

            params.film_render.grain.sublayers_active = true;
            params.film_render.grain.n_sub_layers = 1;
            let l1 = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("layered film development with n_sub_layers=1 failed");
            params.film_render.grain.n_sub_layers = 4;
            let l4 = develop(&log_raw, &film, &params, &backend, 12.0)
                .expect("layered film development with n_sub_layers=4 failed");
            assert_eq!(l1.data, l4.data);
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

        /// Layered grain on a profile without layer curves fails with an
        /// actionable message instead of producing garbage — Python's
        /// `np.nanmax` on the empty tensor raises for the same condition.
        #[test]
        #[should_panic(expected = "grain.sublayers_active requires the film profile")]
        fn sublayers_without_layer_curves_fails() {
            let mut film = portra();
            film.data.density_curves_layers.clear();
            let backend = CpuBackend;
            let params = base_params();
            develop(&log_raw(), &film, &params, &backend, 12.0)
                .expect("film development should panic for missing layer curves");
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
