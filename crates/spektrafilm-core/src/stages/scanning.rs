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
mod setup;
mod spectral;
pub(crate) use spectral::{scan_spectral_rgb, scanner_lut_bounds};

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
/// from the resolved [`style::ChannelTone`] triple (`style::STANDARD` is the
/// frozen historical reference).
pub(crate) mod style;
pub(crate) use style::ChannelTone;

/// Installed fixed styles plus the automatic built-in (legacy aliases omitted).
pub fn conversion_styles(data_dir: &std::path::Path) -> Vec<String> {
    style::available(Some(data_dir))
}

fn positive_tone(log_e: f64, tone: &ChannelTone, params: &RuntimeParams) -> f64 {
    let denominator = -(-std::f64::consts::LN_10 * tone.dmax).exp_m1();
    let e = log_e / std::f64::consts::LOG10_2
        - if params.scanner.uses_standard() {
            0.0
        } else {
            interpretation::camera_ev(params)
        }
        + tone.brightness_ev;
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

pub(crate) fn density_reading_gpu_params(
    profile: &Profile,
    tones: &[ChannelTone; 3],
    params: &RuntimeParams,
) -> Result<spektrafilm_backend::ScannerDensityGpuParams, String> {
    let calibration = interpretation::DensityCalibration::prepare(profile)?;
    let (density, exposure, offsets, counts) = calibration.gpu_axes();
    let operator = PreparedOperator::new(&params.scanner.operator);
    let standard_setup = params.scanner.uses_standard();
    let setup_aim_density = calibration.neutral_density(operator.pivot).iter().sum::<f64>() / 3.0;
    let setup_gamma = calibration.neutral_gamma(operator.pivot);
    if standard_setup && (!setup_aim_density.is_finite() || !setup_gamma.is_finite() || setup_gamma <= 0.0) {
        return Err("standard setup requires a finite positive neutral-curve slope at 16%".into());
    }
    let output = colorspace::resolve(&params.io.output_color_space)?;
    let tone_to_output = if std::ptr::eq(output, &colorspace::SRGB) {
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    } else {
        colorspace::conversion_matrix(&colorspace::SRGB, output)
    };
    Ok(spektrafilm_backend::ScannerDensityGpuParams {
        density,
        exposure,
        offsets,
        counts,
        contrast: tones.map(|tone| tone.contrast),
        dmax: tones.map(|tone| tone.dmax),
        brightness_ev: tones.map(|tone| tone.brightness_ev),
        denominator: tones.map(|tone| -(-std::f64::consts::LN_10 * tone.dmax).exp_m1()),
        camera_ev: if standard_setup { 0.0 } else { interpretation::camera_ev(params) },
        lens_blur: params.scanner.lens_blur,
        standard_setup,
        setup_aim_density,
        setup_gamma,
        operator_pivot: operator.pivot,
        operator_contrast_delta: operator.contrast_delta,
        operator_exposure_ev: operator.exposure_ev,
        operator_color: operator.color,
        tone_to_output,
    })
}

/// Immutable reconstructed exposure after the full-frame setup. Deliberately
/// f64 even in f32 builds: tone is evaluated only after operator correction.
pub(crate) struct PreparedNegative {
    width: u32,
    height: u32,
    exposure: Vec<[f64; 3]>,
}

impl PreparedNegative {
    pub(crate) fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub(crate) fn cached_bytes(&self) -> usize {
        self.exposure.capacity() * std::mem::size_of::<[f64; 3]>()
    }
}

/// Reconstruct the complete working frame once, before any operator controls.
pub(crate) fn prepare_negative(
    density: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
) -> Result<PreparedNegative, String> {
    if backend.is_gpu() {
        return Err("selected GPU negative reconstruction requires the retained scanner acquisition".into());
    }
    let observation = super::StageObservation::cpu(
        backend,
        "negative_reconstruction",
        spektrafilm_backend::telemetry::CpuReason::NegativeToPositiveOutput,
    );
    let backend = observation.backend(backend);
    params.validate()?;
    crate::pipeline_calibration::validate_scan_output(params, profile)?;
    let calibration = interpretation::DensityCalibration::prepare(profile)?;
    // Blur transmitted light, never density. The unblurred path borrows the
    // input instead of allocating a second density frame.
    let density = if params.scanner.lens_blur > 0.0 {
        let mut transmitted = density.clone();
        transmitted
            .data
            .par_iter_mut()
            .for_each(|value| *value = from_f64(10.0f64.powf(-to_f64(*value))));
        let mut blurred = backend.gaussian_blur(&transmitted, params.scanner.lens_blur)?;
        blurred
            .data
            .par_iter_mut()
            .for_each(|value| *value = from_f64(-to_f64(*value).log10()));
        std::borrow::Cow::Owned(blurred)
    } else {
        std::borrow::Cow::Borrowed(density)
    };
    let offset = if params.scanner.uses_standard() {
        let _setup = super::StageObservation::new(backend, "standard_setup");
        setup::FrameSetup::measure(&density, &calibration)?.offset
    } else {
        [0.0; 3]
    };
    let mut exposure = Vec::new();
    let pixels = density.data.len() / 3;
    exposure
        .try_reserve_exact(pixels)
        .map_err(|error| format!("cannot allocate full-frame negative reconstruction: {error}"))?;
    exposure.resize(pixels, [0.0; 3]);
    exposure
        .par_iter_mut()
        .zip(density.data.par_chunks_exact(3))
        .try_for_each(|(dst, px)| -> Result<(), String> {
            let reconstructed =
                calibration.estimate([to_f64(px[0]), to_f64(px[1]), to_f64(px[2])])?;
            for channel in 0..3 {
                dst[channel] = reconstructed[channel] + offset[channel];
                if !dst[channel].is_finite() {
                    return Err("negative reconstruction coordinate must be finite".into());
                }
            }
            Ok(())
        })?;
    Ok(PreparedNegative {
        width: density.width,
        height: density.height,
        exposure,
    })
}

/// Frame-invariant operator terms, prepared once rather than per pixel.
struct PreparedOperator {
    no_op: bool,
    pivot: f64,
    contrast_delta: f64,
    exposure_ev: f64,
    color: [f64; 3],
}

impl PreparedOperator {
    fn new(operator: &crate::params::ScanOperatorParams) -> Self {
        let tint = operator.tint_ev / 3.0;
        Self {
            no_op: operator.is_default(),
            pivot: (0.16_f64 / 0.184).log10(),
            contrast_delta: operator.contrast - 1.0,
            exposure_ev: operator.exposure_ev,
            color: [
                operator.warmth_ev / 2.0 + tint,
                -2.0 * tint,
                -operator.warmth_ev / 2.0 + tint,
            ],
        }
    }

    /// Correct common contrast without scaling record color differences.
    /// The exact default branch avoids a coordinate round trip.
    fn apply(&self, baseline: [f64; 3]) -> Result<[f64; 3], String> {
        if self.no_op {
            return Ok(baseline);
        }
        let stops = baseline.map(|value| (value - self.pivot) / std::f64::consts::LOG10_2);
        let mean = (stops[0] + stops[1] + stops[2]) / 3.0;
        let common = self.contrast_delta * mean + self.exposure_ev;
        let corrected = std::array::from_fn(|channel| {
            self.pivot + std::f64::consts::LOG10_2 * (stops[channel] + common + self.color[channel])
        });
        if !mean.is_finite()
            || !common.is_finite()
            || corrected.iter().any(|value| !value.is_finite())
        {
            return Err(
                "scanner.operator: combined exposure coordinate overflow; corrections are not clamped"
                    .into(),
            );
        }
        Ok(corrected)
    }
}

#[cfg(test)]
fn operator_exposure(
    baseline: [f64; 3],
    operator: &crate::params::ScanOperatorParams,
) -> Result<[f64; 3], String> {
    PreparedOperator::new(operator).apply(baseline)
}

/// Render output-space linear RGB, before the ordinary scan tail. Both CPU
/// and the resident acquisition fence use this same reconstruction and tone.
pub(crate) fn render_prepared_negative(
    prepared: &PreparedNegative,
    tones: &[ChannelTone; 3],
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
) -> Result<ImageBuf, String> {
    let _observation = super::StageObservation::cpu(
        backend,
        "negative_to_positive_tone",
        spektrafilm_backend::telemetry::CpuReason::NegativeToPositiveOutput,
    );
    params.validate()?;
    let output = colorspace::resolve(&params.io.output_color_space)?;
    // The fixed tone defines linear sRGB, not arbitrary destination RGB.
    // Bypass the rounded sRGB matrix product for an exact identity.
    let matrix = (!std::ptr::eq(output, &colorspace::SRGB))
        .then(|| colorspace::conversion_matrix(&colorspace::SRGB, output));
    let operator = PreparedOperator::new(&params.scanner.operator);
    let samples = prepared
        .exposure
        .len()
        .checked_mul(3)
        .ok_or("negative output allocation size overflow")?;
    let mut data = Vec::new();
    data.try_reserve_exact(samples)
        .map_err(|error| format!("cannot allocate negative output: {error}"))?;
    data.resize(samples, from_f64(0.0));
    data.par_chunks_exact_mut(3)
        .zip(prepared.exposure.par_iter())
        .try_for_each(|(dst, &baseline)| -> Result<(), String> {
            let exposure = operator.apply(baseline)?;
            let mut linear_srgb = [0.0; 3];
            for channel in 0..3 {
                let tone = &tones[channel];
                let ev = exposure[channel] / std::f64::consts::LOG10_2
                    - if params.scanner.uses_standard() { 0.0 } else { interpretation::camera_ev(params) }
                    + tone.brightness_ev;
                if !ev.is_finite() || !(tone.contrast * ev).is_finite() {
                    return Err("negative conversion combined tone coordinate overflow".into());
                }
                linear_srgb[channel] = positive_tone(exposure[channel], tone, params);
            }
            let rgb = matrix.as_ref().map_or(linear_srgb, |matrix| {
                std::array::from_fn(|channel| {
                    matrix[channel][0] * linear_srgb[0]
                        + matrix[channel][1] * linear_srgb[1]
                        + matrix[channel][2] * linear_srgb[2]
                })
            });
            for channel in 0..3 {
                dst[channel] = from_f64(rgb[channel]);
                if !dst[channel].is_finite() {
                    return Err("density interpretation output is not representable in the selected precision".into());
                }
            }
            Ok(())
        })?;
    Ok(ImageBuf::from_data(prepared.width, prepared.height, data))
}

/// Record-density acquisition for the assembled scanner acquisition fence.
pub(crate) fn interpret_negative_density_stage(
    density: &ImageBuf,
    profile: &Profile,
    tones: &[ChannelTone; 3],
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
) -> Result<ImageBuf, String> {
    let prepared = prepare_negative(density, profile, params, backend)?;
    render_prepared_negative(&prepared, tones, params, backend)
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
        if backend.is_gpu() {
            return Err("selected GPU negative conversion requires the retained scanner acquisition; retry with an explicit CPU request for host stage execution".into());
        }
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
        interpret_negative_density_stage(density, profile, tones, params, backend)?
    } else {
        match capture {
            Some(capture) => capture,
            None => scan_spectral_rgb(
                density,
                profile,
                params,
                backend,
                &channel_density,
                base_density,
                scan_context,
            )?,
        }
    };
    // Scanner readout optics operate on the linear acquisition before glare,
    // interpretation, or output gamut correction. The density interpretation
    // keeps its dedicated transmission-domain blur above.
    if params.scanner.lens_blur > 0.0 && !negative_to_positive {
        rgb = backend.gaussian_blur(&rgb, params.scanner.lens_blur)?;
    }
    finish_tail(
        rgb,
        params,
        backend,
        gamut,
        scan_context,
        output_space,
    )
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
) -> Result<ImageBuf, String> {
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
        rgb = match backend.grain_v2(&rgb, &gpu_params)? {
            Some(image) => image,
            None if backend.is_gpu() => return Err("grain_v2: selected GPU has no device operation".into()),
            None => {
                let _cpu = super::StageObservation::cpu(
                    backend,
                    "grain_v2_cpu",
                    spektrafilm_backend::telemetry::CpuReason::GrainV2Unsupported,
                );
                spektrafilm_model::grain::v2::apply_cpu(&rgb, grain)
            }
        };
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

    Ok(rgb)
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
    rgb = finish_tail(rgb, params, backend, gamut, &prepared.scan, output_space)?;
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
    scan_spectral_rgb(
        density_cmy,
        profile,
        params,
        backend,
        &crate::chain_prep::channel_density(profile),
        &profile.data.base_density,
        &prepared.scan,
    )
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
            let rgb = scan(&[0.0; 3], &film, &style::STANDARD, &params);
            for channel in &rgb {
                assert!((channel - 1.0).abs() < 5e-3, "{space}: clear film {rgb:?}");
            }
            // The dense end reads dark and finite in every channel.
            let dense = scan(&[3.0, 3.0, 3.0], &film, &style::STANDARD, &params);
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
            &style::STANDARD,
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
    fn fixed_style_source_srgb_reads_only_its_own_record() {
        let data = data_dir();
        let film = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let mut params = film_scan_params();
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.conversion_style = "fixed-probe".into();
        params.io.output_color_space = "sRGB".into();
        let base = scan(&[0.4, 0.4, 0.4], &film, &style::STANDARD, &params);
        for channel in 0..3 {
            let mut density = [0.4; 3];
            density[channel] += 0.4;
            let moved = scan(&density, &film, &style::STANDARD, &params);
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

    /// Fixed tones preserve per-record independence in their source sRGB;
    /// destination RGB conversion may mix these channels.
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
        params.scanner.conversion_style = "fixed-probe".into();
        params.io.output_color_space = "sRGB".into();

        let density = [0.55, 0.45, 0.35];
        let neutral = scan(&density, &film, &style::STANDARD, &params);
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

    #[test]
    fn operator_axes_preserve_color_differences_and_declared_stops() {
        use crate::params::ScanOperatorParams;
        let pivot = (0.16_f64 / 0.184).log10();
        let baseline = [-1.25, 0.5, 2.25].map(|ev| pivot + ev * std::f64::consts::LOG10_2);
        assert_eq!(
            operator_exposure(baseline, &ScanOperatorParams::default()).unwrap(),
            baseline
        );
        for contrast in [0.5, 1.0, 2.0] {
            let operator = ScanOperatorParams {
                exposure_ev: 0.7,
                warmth_ev: 0.4,
                tint_ev: -0.3,
                contrast,
            };
            let corrected = operator_exposure(baseline, &operator)
                .unwrap()
                .map(|x| (x - pivot) / std::f64::consts::LOG10_2);
            let mean = corrected.iter().sum::<f64>() / 3.0;
            assert!((mean - (contrast * 0.5 + 0.7)).abs() < 1e-14);
            assert!((corrected[0] - corrected[2] - (-3.5 + 0.4)).abs() < 1e-14);
            assert!(((corrected[0] + corrected[2]) / 2.0 - corrected[1] - (-0.3)).abs() < 1e-14);
        }
        // A balanced pixel remains neutral under common exposure and contrast.
        let operator = ScanOperatorParams {
            exposure_ev: -0.6,
            contrast: 1.7,
            ..ScanOperatorParams::default()
        };
        let corrected = operator_exposure([pivot + 0.4; 3], &operator).unwrap();
        assert_eq!(corrected, [corrected[0]; 3]);
    }

    #[test]
    fn combined_finite_operator_values_reject_overflow_instead_of_clamping() {
        let operator = crate::params::ScanOperatorParams {
            exposure_ev: f64::MAX,
            contrast: f64::MAX,
            ..crate::params::ScanOperatorParams::default()
        };
        assert!(operator.validate().is_ok());
        assert!(
            operator_exposure([1.0; 3], &operator)
                .unwrap_err()
                .contains("overflow")
        );
    }

    #[test]
    fn prepared_render_keeps_f64_baseline_and_default_tone_exact() {
        let film = crate::profile::load_profile_by_name(&data_dir(), "kodak_portra_400").unwrap();
        let mut params = film_scan_params();
        params.scanner.scan_output = "negative_to_positive".into();
        params.io.output_color_space = "sRGB".into();
        let density = ImageBuf::from_data(
            2,
            1,
            vec![
                from_f64(0.3),
                from_f64(0.5),
                from_f64(0.7),
                from_f64(1.2),
                from_f64(1.0),
                from_f64(0.8),
            ],
        );
        let original_density = density.data.clone();
        let backend = &spektrafilm_backend::cpu_backend::CpuBackend;
        let prepared = prepare_negative(&density, &film, &params, backend).unwrap();
        let calibration = interpretation::DensityCalibration::prepare(&film).unwrap();
        let offset = setup::FrameSetup::measure(&density, &calibration)
            .unwrap()
            .offset;
        for (baseline, px) in prepared
            .exposure
            .iter()
            .zip(density.data.as_chunks::<3>().0)
        {
            let inverse = calibration
                .estimate([to_f64(px[0]), to_f64(px[1]), to_f64(px[2])])
                .unwrap();
            assert_eq!(*baseline, std::array::from_fn(|c| inverse[c] + offset[c]));
        }
        let baseline = prepared.exposure.clone();
        let original =
            render_prepared_negative(&prepared, &style::STANDARD, &params, backend).unwrap();
        let expected: Vec<_> = baseline
            .iter()
            .flat_map(|px| {
                std::array::from_fn::<_, 3, _>(|c| {
                    from_f64(positive_tone(px[c], &style::STANDARD[c], &params))
                })
            })
            .collect();
        assert_eq!(original.data, expected);
        params.scanner.operator.exposure_ev = 0.5;
        let moved =
            render_prepared_negative(&prepared, &style::STANDARD, &params, backend).unwrap();
        assert!(moved.data.iter().zip(&original.data).all(|(a, b)| a > b));
        params.scanner.operator = crate::params::ScanOperatorParams::default();
        assert_eq!(
            render_prepared_negative(&prepared, &style::STANDARD, &params, backend)
                .unwrap()
                .data,
            original.data
        );
        assert_eq!(prepared.exposure, baseline);
        assert_eq!(density.data, original_density);
    }

    #[test]
    fn standard_and_fixed_tone_are_converted_from_linear_srgb_before_tail() {
        let backend = &spektrafilm_backend::cpu_backend::CpuBackend;
        // Deliberately non-f32 coordinates catch an intermediate tone cast.
        let prepared = PreparedNegative {
            width: 1,
            height: 1,
            exposure: vec![[-0.173456789123, 0.287654321987, 0.731234567891]],
        };
        for style in ["standard", "fixed-probe"] {
            let mut params = film_scan_params();
            params.scanner.scan_output = "negative_to_positive".into();
            params.scanner.conversion_style = style.into();
            let tone_rgb = std::array::from_fn::<_, 3, _>(|c| {
                positive_tone(prepared.exposure[0][c], &super::style::STANDARD[c], &params)
            });
            for space in ["sRGB", "Display P3", "ProPhoto RGB"] {
                params.io.output_color_space = space.into();
                let rendered =
                    render_prepared_negative(&prepared, &super::style::STANDARD, &params, backend)
                        .unwrap();
                let expected = if space == "sRGB" {
                    tone_rgb
                } else {
                    let matrix = colorspace::conversion_matrix(
                        &colorspace::SRGB,
                        colorspace::resolve(space).unwrap(),
                    );
                    std::array::from_fn(|c| {
                        matrix[c][0] * tone_rgb[0]
                            + matrix[c][1] * tone_rgb[1]
                            + matrix[c][2] * tone_rgb[2]
                    })
                };
                assert_eq!(rendered.data, expected.map(from_f64), "{style} -> {space}");
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
        let a = scan(&density, &film, &style::STANDARD, &params);
        let b = scan(&density, &film, &warm, &params);
        assert_eq!(a, b);
    }
}
