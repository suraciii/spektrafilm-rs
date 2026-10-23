//! Scanning stage: convert developed film/print density back to RGB through
//! the neutral scan model — one lamp, a colorimetric sensor, and the clear
//! film as the white reference.
//!
//! Dispatches the spectral integration to the GPU backend when available.
use rayon::prelude::*;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{from_f64, to_f64};

use crate::params::RuntimeParams;
use crate::profile::Profile;
mod interpretation;
mod spectral;
pub(crate) use spectral::scan_spectral_rgb;

fn negative_to_positive_requested(params: &RuntimeParams) -> bool {
    params.scanner.scan_output == "negative_to_positive"
}

/// The film profile curves in the density domain the film stage emits: each
/// record measured over its own minimum.
fn developed_density_curves(profile: &Profile) -> Vec<[f64; 3]> {
    spektrafilm_model::density_curves::normalize_density_curves_f64(&profile.density_curves_f64())
}

/// Fixed positive tone response of the negative interpretation: a logistic
/// density curve in the same family for every style; the parameters come
/// from the resolved [`style::ChannelTone`] triple (`style::NEUTRAL` is the
/// frozen historical reference).
pub(crate) mod style;
pub(crate) use style::ChannelTone;

fn positive_tone(log_e: f64, tone: &ChannelTone, params: &RuntimeParams) -> f64 {
    let denominator = -(-std::f64::consts::LN_10 * tone.dmax).exp_m1();
    let e =
        log_e / std::f64::consts::LOG10_2 - interpretation::camera_ev(params) + tone.brightness_ev;
    let z = tone.contrast * e;
    let t = if z >= 0.0 {
        1.0 / (1.0 + (-z).exp())
    } else {
        let a = z.exp();
        a / (1.0 + a)
    };
    let density = tone.dmax * (1.0 - t);
    let positive = (-std::f64::consts::LN_10 * density).exp_m1();
    (positive + denominator) / denominator
}

/// Record-density interpretation: the narrowband lab-scanner inverse of the
/// film's own characteristic curves. Scanner optics blur transmitted light
/// record by record, before the sensor reads density, so the blur runs here in
fn interpret_negative_density(
    density_cmy: &ImageBuf,
    calibration: &interpretation::DensityCalibration,
    tones: &[ChannelTone; 3],
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
) -> Result<ImageBuf, String> {
    let mut rgb = if params.scanner.lens_blur > 0.0 {
        let mut transmitted = density_cmy.clone();
        transmitted
            .data
            .par_iter_mut()
            .for_each(|value| *value = from_f64(10.0f64.powf(-to_f64(*value))));
        let mut blurred = backend.gaussian_blur(&transmitted, params.scanner.lens_blur);
        blurred
            .data
            .par_iter_mut()
            .for_each(|value| *value = from_f64(-to_f64(*value).log10()));
        blurred
    } else {
        density_cmy.clone()
    };
    rgb.data
        .par_chunks_exact_mut(3)
        .try_for_each(|px| -> Result<(), String> {
            let density = [to_f64(px[0]), to_f64(px[1]), to_f64(px[2])];
            let exposure = calibration.estimate(density)?;
            for channel in 0..3 {
                px[channel] = from_f64(positive_tone(exposure[channel], &tones[channel], params));
                if !px[channel].is_finite() {
                    return Err(
                        "density interpretation output is not representable in the selected precision"
                            .into(),
                    );
                }
            }
            Ok(())
        })?;
    Ok(rgb)
}

/// Record-density acquisition for the assembled scanner acquisition fence.
pub(crate) fn interpret_negative_density_stage(
    density_cmy: &ImageBuf,
    profile: &Profile,
    tones: &[ChannelTone; 3],
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
) -> Result<ImageBuf, String> {
    let calibration = interpretation::DensityCalibration::prepare(profile)?;
    interpret_negative_density(density_cmy, &calibration, tones, params, backend)
}

// Scan options preserve the existing stage interface.
#[allow(clippy::too_many_arguments)]
pub fn scan_with_options(
    density_cmy: &ImageBuf,
    profile: &Profile,
    tones: &[ChannelTone; 3],
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    gamut: &crate::gamut_compression::OutputGamutCompress,
    include_base: bool,
) -> Result<ImageBuf, String> {
    scan_with_capture(
        density_cmy,
        profile,
        tones,
        params,
        backend,
        gamut,
        include_base,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn scan_with_capture(
    density: &ImageBuf,
    profile: &Profile,
    tones: &[ChannelTone; 3],
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    gamut: &crate::gamut_compression::OutputGamutCompress,
    include_base: bool,
    capture: Option<ImageBuf>,
) -> Result<ImageBuf, String> {
    let negative_to_positive = negative_to_positive_requested(params);
    if negative_to_positive {
        params.validate()?;
        crate::pipeline_calibration::validate_scan_output(params, profile)?;
    }
    let _negative_to_positive = negative_to_positive.then(|| {
        super::StageObservation::cpu(
            backend,
            "negative_to_positive",
            spektrafilm_backend::telemetry::CpuReason::NegativeToPositiveOutput,
        )
    });
    let cpu_fallback = negative_to_positive
        .then(|| {
            backend
                .observation_context()
                .filter(|context| context.enabled())
                .map(|context| {
                    spektrafilm_backend::bind_cpu_fallback(
                        context.clone(),
                        spektrafilm_backend::telemetry::CpuReason::NegativeToPositiveOutput,
                    )
                })
        })
        .flatten();
    let backend = if negative_to_positive {
        static CPU_BACKEND: spektrafilm_backend::cpu_backend::CpuBackend =
            spektrafilm_backend::cpu_backend::CpuBackend;
        cpu_fallback
            .as_deref()
            .unwrap_or(&CPU_BACKEND as &dyn ComputeBackend)
    } else {
        backend
    };
    let channel_density = crate::chain_prep::channel_density(profile);
    let base_density = if include_base {
        std::borrow::Cow::Borrowed(profile.data.base_density.as_slice())
    } else {
        std::borrow::Cow::Owned(vec![0.0; profile.data.base_density.len()])
    };

    let output_space = colorspace::resolve(&params.io.output_color_space)
        .expect("output color space must be validated before scanning");
    let prepared = crate::chain_prep::PreparedChain::for_scan(profile, params, include_base);
    let scan_context = &prepared.scan;
    let base_density = base_density.as_ref();

    // Dispatch spectral integration to backend (GPU or CPU). The backend
    // applies the two matrices in sequence — we pass them separately. The
    // negative density reading acquires no capture: it reads the record
    // densities through the film's own curves.
    let mut rgb = if negative_to_positive {
        let calibration = interpretation::DensityCalibration::prepare(profile)?;
        interpret_negative_density(density, &calibration, tones, params, backend)?
    } else {
        capture.unwrap_or_else(|| {
            scan_spectral_rgb(
                density,
                profile,
                params,
                backend,
                &channel_density,
                base_density,
                scan_context,
            )
        })
    };
    // Scanner readout optics operate on the linear acquisition before glare,
    // interpretation, or output gamut correction. The density interpretation
    // keeps its dedicated transmission-domain blur above.
    if params.scanner.lens_blur > 0.0 && !negative_to_positive {
        rgb = backend.gaussian_blur(&rgb, params.scanner.lens_blur);
    }
    Ok(finish_tail(
        rgb,
        params,
        backend,
        gamut,
        scan_context,
        output_space,
    ))
}

/// The post-capture tail shared by the full scan and a pre-captured readout:
/// viewing glare, output gamut compression, unsharp, then the encoded-domain
/// passes. Takes the scan context for the glare offset colour.
#[allow(clippy::too_many_arguments)]
fn finish_tail(
    mut rgb: ImageBuf,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    gamut: &crate::gamut_compression::OutputGamutCompress,
    scan_context: &crate::chain_prep::ScanColorContext,
    output_space: &spektrafilm_math::colorspace::RgbColorSpace,
) -> ImageBuf {
    // Viewing glare. Since CAT+matrix is linear, we equivalently add glare in
    // RGB space by pre-multiplying the lamp XYZ through the same matrix.
    // Direct-film scans get no viewing glare (`io.scan_film`); only the print
    // path consumes `print_render.glare`.
    let glare = (!params.io.scan_film)
        .then_some(&params.print_render.glare)
        .filter(|g| g.active && g.percent > 0.0);
    if let Some(glare) = glare {
        let _glare = super::StageObservation::cpu(
            backend,
            "glare",
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
        );
        // Shared scan context keeps the CPU two-step CAT→RGB operation
        // order used by the reference path.
        let glare_rgb_offset_f64 = crate::chain_prep::glare_rgb_offset_f64(scan_context);
        let glare_rgb_offset = glare_rgb_offset_f64.map(from_f64);
        let glare_amount = spektrafilm_model::glare::compute_random_glare_amount(
            rgb.width,
            rgb.height,
            glare.percent,
            glare.roughness,
            glare.blur,
            params.random_seed,
        );
        spektrafilm_model::glare::add_glare_with_amount(&mut rgb, &glare_amount, glare_rgb_offset);
    }

    // Output gamut compression.
    if gamut.is_active() {
        let _gamut = super::StageObservation::cpu(
            backend,
            "output_gamut",
            spektrafilm_backend::telemetry::CpuReason::UnsupportedOutputGamut,
        );
        rgb.data.par_chunks_exact_mut(3).for_each(|px| {
            let out = gamut.compress([px[0] as f64, px[1] as f64, px[2] as f64]);
            px[0] = from_f64(out[0]);
            px[1] = from_f64(out[1]);
            px[2] = from_f64(out[2]);
        });
    }

    // Unsharp mask
    let [usm_sigma, usm_amount] = params.scanner.unsharp_mask;
    if usm_sigma > 0.0 && usm_amount > 0.0 {
        rgb = spektrafilm_model::optics::apply_unsharp_mask(&rgb, usm_sigma, usm_amount, backend);
    }

    // Grain consumes native display-encoded RGB, with no internal transfer
    // or primaries conversion. The scanned image supplies its output space.
    let grain_v2_active = params.film_render.grain.active
        && params.settings.rgb_to_raw_method != "mallett2019"
        && matches!(
            params.film_render.grain.engine,
            crate::params::grain::GrainEngine::Procedural
        );
    // CMYK ink style is optional finishing, not a route: it applies to
    // any scanned base workflow after the scanner-domain effects.
    let cmyk_active = params.cmyk.active && params.cmyk.strength > 0.0;
    let encoded_domain = params.io.output_cctf_encoding || grain_v2_active;
    if encoded_domain {
        let _encoding = super::StageObservation::cpu(
            backend,
            "output_transfer",
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
        );
        rgb.data.par_chunks_exact_mut(3).for_each(|px| {
            let encoded =
                colorspace::encode_rgb([px[0] as f64, px[1] as f64, px[2] as f64], output_space);
            px[0] = from_f64(encoded[0]);
            px[1] = from_f64(encoded[1]);
            px[2] = from_f64(encoded[2]);
        });
    }
    if grain_v2_active {
        let grain_observation = super::StageObservation::new(backend, "grain_v2");
        let backend = grain_observation.backend(backend);
        let mut grain = params.film_render.grain.resolved_grain_v2();
        if params.debug.deactivate_spatial_effects {
            grain.resolution_factor = 100.0;
        }
        grain.seed = params.random_seed as u32;
        let gpu_params = params
            .film_render
            .grain
            .gpu_params(params.random_seed, params.debug.deactivate_spatial_effects);
        rgb = backend.grain_v2(&rgb, &gpu_params).unwrap_or_else(|| {
            let _cpu = super::StageObservation::cpu(
                backend,
                "grain_v2_cpu",
                spektrafilm_backend::telemetry::CpuReason::GrainV2Unsupported,
            );
            spektrafilm_model::grain::v2::apply_cpu(&rgb, grain)
        });
    }
    // CMYK ink style is the final post-scan appearance, after scanner-domain
    // grain. A linear export uses the helper's finite transfer round-trip and
    // remains linear at the API boundary.
    if cmyk_active {
        let _cmyk = super::StageObservation::cpu(
            backend,
            "cmyk",
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
        );
        crate::cmyk::apply(&mut rgb, &params.cmyk, output_space, encoded_domain);
    }
    // Linear exports retain the same grain realization as encoded exports.
    // Decode only the transfer; the stored same-space matrix ran above once.
    if grain_v2_active && !params.io.output_cctf_encoding {
        let _encoding = super::StageObservation::cpu(
            backend,
            "output_transfer",
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
        );
        rgb.data.par_iter_mut().for_each(|value| {
            *value = from_f64(colorspace::cctf_decode(*value as f64, output_space.cctf));
        });
    }

    rgb
}

/// Finish an already-captured readout in the scanner's stage order:
/// glare, gamut compression, unsharp, then encoding. Never re-acquires or
/// re-interprets; the density-reading route finishes its own output.
pub fn finish_capture(
    mut rgb: ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    gamut: &crate::gamut_compression::OutputGamutCompress,
) -> Result<ImageBuf, String> {
    let output_space = colorspace::resolve(&params.io.output_color_space)
        .expect("output color space must be validated before scanning");
    let prepared = crate::chain_prep::PreparedChain::for_scan(profile, params, true);
    rgb = finish_tail(rgb, params, backend, gamut, &prepared.scan, output_space);
    Ok(rgb)
}

/// Linear optical capture before inversion, gamut compression and encoding.
pub fn capture_linear(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
) -> Result<ImageBuf, String> {
    if params.settings.use_scanner_lut {
        return Err(
            "exact-cell scanning requires direct spectral capture without the scanner LUT"
                .to_owned(),
        );
    }
    let prepared = crate::chain_prep::PreparedChain::for_scan(profile, params, true);
    Ok(scan_spectral_rgb(
        density_cmy,
        profile,
        params,
        backend,
        &crate::chain_prep::channel_density(profile),
        &profile.data.base_density,
        &prepared.scan,
    ))
}
/// Scan with the film base included — the per-stage topology entry point.
pub fn process(
    density_cmy: &ImageBuf,
    profile: &Profile,
    tones: &[ChannelTone; 3],
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    gamut: &crate::gamut_compression::OutputGamutCompress,
) -> Result<ImageBuf, String> {
    scan_with_options(density_cmy, profile, tones, params, backend, gamut, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::data_dir;

    fn params() -> RuntimeParams {
        let mut params = RuntimeParams::default();
        params.scanner.unsharp_mask = [0.0; 2];
        params.print_render.glare.active = false;
        params.io.output_cctf_encoding = false;
        params.settings.use_scanner_lut = false;
        params
    }

    fn film_scan_params() -> RuntimeParams {
        let mut params = params();
        params.workflow.route = "input > film > scan".into();
        params.io.scan_film = true;
        params
    }

    fn scan(
        densities: &[f64],
        profile: &Profile,
        tones: &[ChannelTone; 3],
        params: &RuntimeParams,
    ) -> Vec<f64> {
        let image = ImageBuf::from_data(
            (densities.len() / 3) as u32,
            1,
            densities.iter().copied().map(from_f64).collect(),
        );
        let rgb = process(
            &image,
            profile,
            tones,
            params,
            &spektrafilm_backend::cpu_backend::CpuBackend,
            &crate::gamut_compression::OutputGamutCompress::identity(),
        )
        .unwrap();
        rgb.data.iter().map(|v| to_f64(*v)).collect()
    }

    /// Neutral scan model self-checks, mirroring the research reference
    /// implementation (`k64-neutral-scan-v2`): a clear film reads the output
    /// white, so the base tint is removed rather than baked in. The residual
    /// from exact white is the shared colour tables' rounding (the 4-digit
    /// `XYZ_TO_SRGB_F64` maps the D65 white to ≈(0.9998, 1.0001, 1.0001)),
    /// which every colour conversion in the product shares.
    #[test]
    fn clear_film_reads_output_white_in_any_output_space() {
        let data = data_dir();
        let film = crate::profile::load_profile_by_name(&data, "kodak_kodachrome_64").unwrap();
        for space in ["sRGB", "ProPhoto RGB", "Display P3"] {
            let mut params = params();
            params.io.output_color_space = space.into();
            let rgb = scan(&[0.0; 3], &film, &style::NEUTRAL, &params);
            for channel in &rgb {
                assert!((channel - 1.0).abs() < 5e-3, "{space}: clear film {rgb:?}");
            }
            // The dense end reads dark and finite in every channel.
            let dense = scan(&[3.0, 3.0, 3.0], &film, &style::NEUTRAL, &params);
            assert!(dense.iter().all(|v| v.is_finite() && *v >= 0.0 && *v < 1.0));
        }
    }

    /// The readout light is a product constant selected by the material
    /// category, not a runtime knob.
    #[test]
    fn readout_illuminant_follows_the_material_category() {
        let data = data_dir();
        let negative = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let cine_print = crate::profile::load_profile_by_name(&data, "kodak_2383").unwrap();
        let bw_print = crate::profile::load_profile_by_name(&data, "kodak_2302").unwrap();
        assert_eq!(negative.readout_illuminant(), "D50");
        assert_eq!(cine_print.readout_illuminant(), "BB5800");
        assert_eq!(bw_print.readout_illuminant(), "D50");
    }

    #[test]
    fn denser_negative_reads_brighter_through_the_film_curves() {
        let data = data_dir();
        let film = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let mut params = film_scan_params();
        params.scanner.scan_output = "negative_to_positive".into();
        let values = scan(
            &[0.0, 0.0, 0.0, 1.2, 1.2, 1.2],
            &film,
            &style::NEUTRAL,
            &params,
        );
        for channel in 0..3 {
            let clear = values[channel];
            let dense = values[3 + channel];
            assert!(
                clear < dense,
                "channel {channel}: clear {clear} must read darker than dense {dense}"
            );
        }
    }

    #[test]
    fn each_output_channel_reads_only_its_own_record() {
        let data = data_dir();
        let film = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let mut params = film_scan_params();
        params.scanner.scan_output = "negative_to_positive".into();
        let base = scan(&[0.4, 0.4, 0.4], &film, &style::NEUTRAL, &params);
        for channel in 0..3 {
            let mut density = [0.4; 3];
            density[channel] += 0.4;
            let moved = scan(&density, &film, &style::NEUTRAL, &params);
            for other in 0..3 {
                if other != channel {
                    assert!(
                        (moved[other] - base[other]).abs() < 1e-9,
                        "record {channel} leaked into channel {other}"
                    );
                }
            }
        }
    }

    /// A declared style is an interpretation-side tone: it changes the
    /// negative-to-positive render without touching the scanner readout, and
    /// keeps per-record independence and density monotonicity.
    #[test]
    fn conversion_style_tones_the_positive_without_breaking_invariants() {
        let data = data_dir();
        let film = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        // A user-authored warm style: red lifted, blue dropped, one EV step
        // per channel against the frozen neutral constants.
        let warm = [
            ChannelTone {
                brightness_ev: 0.9281766618,
                contrast: 0.73551400058216643,
                dmax: 1.5625841465572565,
            },
            ChannelTone {
                brightness_ev: 0.8781766618,
                contrast: 0.73551400058216643,
                dmax: 1.5625841465572565,
            },
            ChannelTone {
                brightness_ev: 0.8281766618,
                contrast: 0.73551400058216643,
                dmax: 1.5625841465572565,
            },
        ];
        let mut params = film_scan_params();
        params.scanner.scan_output = "negative_to_positive".into();

        let density = [0.55, 0.45, 0.35];
        let neutral = scan(&density, &film, &style::NEUTRAL, &params);
        let styled = scan(&density, &film, &warm, &params);
        assert!(
            neutral
                .iter()
                .zip(&styled)
                .any(|(a, b)| (a - b).abs() > 1e-4),
            "the style must move the tone: {neutral:?} vs {styled:?}"
        );
        // The warm style lifts red and drops blue relative to neutral.
        assert!(styled[0] > neutral[0], "red {styled:?} vs {neutral:?}");
        assert!(styled[2] < neutral[2], "blue {styled:?} vs {neutral:?}");

        // Monotonicity and per-record independence hold under the style too.
        let dense = scan(&[1.2; 3], &film, &warm, &params);
        let clear = scan(&[0.0; 3], &film, &warm, &params);
        for channel in 0..3 {
            assert!(clear[channel] < dense[channel], "channel {channel}");
        }
        let base = scan(&[0.4, 0.4, 0.4], &film, &warm, &params);
        for channel in 0..3 {
            let mut moved = [0.4; 3];
            moved[channel] += 0.4;
            let read = scan(&moved, &film, &warm, &params);
            for other in 0..3 {
                if other != channel {
                    assert!(
                        (read[other] - base[other]).abs() < 1e-9,
                        "record {channel} leaked into channel {other}"
                    );
                }
            }
        }
    }

    /// Direct scan ignores the conversion tone entirely: the style slot only
    /// exists on the negative-to-positive interpretation.
    #[test]
    fn conversion_style_is_inert_on_direct_scan() {
        let data = data_dir();
        let film = crate::profile::load_profile_by_name(&data, "kodak_kodachrome_64").unwrap();
        let warm = [
            ChannelTone {
                brightness_ev: 0.9281766618,
                contrast: 0.73551400058216643,
                dmax: 1.5625841465572565,
            },
            ChannelTone {
                brightness_ev: 0.8781766618,
                contrast: 0.73551400058216643,
                dmax: 1.5625841465572565,
            },
            ChannelTone {
                brightness_ev: 0.8281766618,
                contrast: 0.73551400058216643,
                dmax: 1.5625841465572565,
            },
        ];
        let params = film_scan_params();
        let density = [0.512, 0.357, 0.095];
        let a = scan(&density, &film, &style::NEUTRAL, &params);
        let b = scan(&density, &film, &warm, &params);
        assert_eq!(a, b);
    }
}
