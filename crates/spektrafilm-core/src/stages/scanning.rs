/// Scanning stage: convert print/film density back to RGB.
///
/// Dispatches the spectral integration to the GPU backend when available.
use rayon::prelude::*;
use spektrafilm_gpu::ComputeBackend;
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::pchip3d::{pchip_interp, prepare_pchip_3d};
use spektrafilm_math::precision::{Scalar, from_f64, to_f64};

use super::build_lut_grid;
use crate::params::RuntimeParams;
use crate::profile::Profile;
mod coupled;
mod finish;
mod interpretation;
mod setup;
pub use setup::ScannerSetup;

/// Scanner LUT bounds. When scan_film=true: `-grain.density_min` to
/// `nanmax(film.density_curves)`. Else: `nanmin..nanmax` of
/// `print.density_curves`. Mirrors Python `ScanningStage._density_to_rgb`.
fn scanner_lut_bounds(profile: &Profile, params: &RuntimeParams) -> ([f64; 3], [f64; 3]) {
    let curves = profile.density_curves_f64();
    let mut dmin_curves = [f64::INFINITY; 3];
    let mut dmax_curves = [f64::NEG_INFINITY; 3];
    for row in &curves {
        for c in 0..3 {
            if !row[c].is_nan() {
                if row[c] < dmin_curves[c] {
                    dmin_curves[c] = row[c];
                }
                if row[c] > dmax_curves[c] {
                    dmax_curves[c] = row[c];
                }
            }
        }
    }
    let (data_min, data_max) = if params.io.scan_film {
        let gmin = params.film_render.grain.density_min;
        ([-gmin[0], -gmin[1], -gmin[2]], dmax_curves)
    } else {
        (dmin_curves, dmax_curves)
    };
    let mut dm = data_min;
    let mut dx = data_max;
    for c in 0..3 {
        if !dm[c].is_finite() {
            dm[c] = 0.0;
        }
        if !dx[c].is_finite() {
            dx[c] = 1.0;
        }
        if dx[c] <= dm[c] {
            dx[c] = dm[c] + 1.0;
        }
    }
    (dm, dx)
}

struct PreparedScannerLut {
    pchip: spektrafilm_math::pchip3d::PreparedPchip3d,
    minimum: [f64; 3],
    inverse_step: [f64; 3],
}

fn prepare_scanner_lut(
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    color: &crate::chain_prep::ScanColorContext,
    minimum: [f64; 3],
    maximum: [f64; 3],
    steps: usize,
) -> PreparedScannerLut {
    let grid = build_lut_grid(steps, minimum, maximum);
    // log_xyz LUT — same function Python LUTs.
    let log_xyz = spektrafilm_gpu::cpu_backend::scan_log_xyz_cpu(
        &grid,
        channel_density,
        base_density,
        &color.illuminant,
        color.normalization,
    );
    let pchip = prepare_pchip_3d(log_xyz, steps);
    let scale = (steps - 1) as f64;
    PreparedScannerLut {
        pchip,
        minimum,
        inverse_step: std::array::from_fn(|c| scale / (maximum[c] - minimum[c])),
    }
}

/// Build a 17³ log_xyz LUT (matching Python's `cmy_to_log_xyz`),
/// PCHIP-interpolate per pixel, then apply `10^log_xyz → CAT →
/// xyz_to_rgb` to get RGB. Glare is added after this returns in RGB
/// space — algebraically equivalent to Python's add_glare-in-XYZ
/// because the matrices are linear (see the glare note in
/// `scan_with_options` below).
///
/// This LUTs the *same intermediate quantity* Python LUTs
/// (`cmy_to_log_xyz`), not the post-`10^x`/post-matrix RGB output —
/// keeps the PCHIP interpolating the smoother log10-domain surface
/// for bit-identical parity with Python's LUT path.
#[allow(clippy::too_many_arguments)]
fn scan_spectral_via_lut(
    density_cmy: &ImageBuf,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    color: &crate::chain_prep::ScanColorContext,
    _backend: &dyn ComputeBackend,
    data_min: [f64; 3],
    data_max: [f64; 3],
    steps: usize,
    color_ref: &crate::color_reference::ColorReference,
) -> ImageBuf {
    let _lut = super::StageObservation::cpu(
        _backend,
        "scanner_lut",
        spektrafilm_gpu::telemetry::CpuReason::SpectralLut,
    );
    let prepared = prepare_scanner_lut(
        channel_density,
        base_density,
        color,
        data_min,
        data_max,
        steps,
    );
    let cat = &color.adapt;
    let xyz_to_rgb = &color.base_xyz_to_rgb;
    let mut out = density_cmy.clone();
    out.data
        .par_chunks_exact_mut(3)
        .zip(density_cmy.data.par_chunks_exact(3))
        .for_each(|(dst, src)| {
            let r = (src[0] as f64 - prepared.minimum[0]) * prepared.inverse_step[0];
            let g = (src[1] as f64 - prepared.minimum[1]) * prepared.inverse_step[1];
            let b = (src[2] as f64 - prepared.minimum[2]) * prepared.inverse_step[2];
            let log_xyz = pchip_interp(&prepared.pchip, r, g, b);
            // Post-LUT: 10^log_xyz → CAT → xyz_to_rgb, mirroring Python's
            // `_density_to_rgb` AFTER the LUT call. bw_correction and
            // glare run after this returns (RGB space).
            let mut xyz = [
                10.0f64.powf(log_xyz[0]),
                10.0f64.powf(log_xyz[1]),
                10.0f64.powf(log_xyz[2]),
            ];
            // B&W/slide luminance remap on the pre-CAT scan XYZ (Python's
            // black_white_xyz_correction). Scalar scale from the Y channel;
            // no-op when color_ref has no remap.
            let scale = color_ref.xyz_scale(xyz[1]);
            if scale != 1.0 {
                xyz[0] *= scale;
                xyz[1] *= scale;
                xyz[2] *= scale;
            }
            let xa = cat[0][0] * xyz[0] + cat[0][1] * xyz[1] + cat[0][2] * xyz[2];
            let ya = cat[1][0] * xyz[0] + cat[1][1] * xyz[1] + cat[1][2] * xyz[2];
            let za = cat[2][0] * xyz[0] + cat[2][1] * xyz[1] + cat[2][2] * xyz[2];
            dst[0] =
                from_f64(xyz_to_rgb[0][0] * xa + xyz_to_rgb[0][1] * ya + xyz_to_rgb[0][2] * za);
            dst[1] =
                from_f64(xyz_to_rgb[1][0] * xa + xyz_to_rgb[1][1] * ya + xyz_to_rgb[1][2] * za);
            dst[2] =
                from_f64(xyz_to_rgb[2][0] * xa + xyz_to_rgb[2][1] * ya + xyz_to_rgb[2][2] * za);
        });
    out
}

/// Spectral integration of one density image under the scan colour context.
///
/// The faithful B&W/slide luminance remap (`color_ref.has_remap`) only exists
/// on the LUT path, so it forces that path; `settings.use_scanner_lut` selects
/// it otherwise. The bulk scan and the one-pixel reference samples share this
/// switch, so a new path condition cannot desynchronize them.
// Both scan paths share the complete spectral context.
#[allow(clippy::too_many_arguments)]
fn scan_spectral_rgb(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    color: &crate::chain_prep::ScanColorContext,
    defer_correction: bool,
) -> ImageBuf {
    if let Some(receiver) = &color.receiver {
        let mut capture = receiver.capture(density_cmy, channel_density, base_density);
        if color_ref.has_remap() && !defer_correction {
            let to_y = receiver.luminance_weights();
            capture.data.par_chunks_exact_mut(3).for_each(|px| {
                let y = (0..3).map(|c| to_y[c] * to_f64(px[c])).sum();
                let scale = color_ref.xyz_scale(y);
                if scale != 1.0 {
                    for value in px {
                        *value = from_f64(to_f64(*value) * scale);
                    }
                }
            });
        }
        return capture;
    }
    if color_ref.has_remap() || params.settings.use_scanner_lut {
        let (data_min, data_max) = scanner_lut_bounds(profile, params);
        let identity = crate::color_reference::ColorReference::identity();
        scan_spectral_via_lut(
            density_cmy,
            channel_density,
            base_density,
            color,
            backend,
            data_min,
            data_max,
            params.settings.lut_resolution as usize,
            if defer_correction {
                &identity
            } else {
                color_ref
            },
        )
    } else {
        backend.scan_spectral(
            density_cmy,
            channel_density,
            base_density,
            &color.illuminant,
            color.normalization,
            &color.adapt,
            &color.base_xyz_to_rgb,
        )
    }
}
/// Prepared CPU f64 forward capture for inverse-table calibration.
pub(crate) struct ForwardCapture<'a> {
    channel_density: &'a [[f64; 3]],
    base_density: &'a [f64],
    color: &'a crate::chain_prep::ScanColorContext,
    color_ref: &'a crate::color_reference::ColorReference,
    lut: Option<PreparedScannerLut>,
}

impl<'a> ForwardCapture<'a> {
    pub(crate) fn prepare(
        profile: &Profile,
        params: &RuntimeParams,
        channel_density: &'a [[f64; 3]],
        base_density: &'a [f64],
        color: &'a crate::chain_prep::ScanColorContext,
        color_ref: &'a crate::color_reference::ColorReference,
    ) -> Self {
        let (minimum, maximum) = scanner_lut_bounds(profile, params);
        let steps = params.settings.lut_resolution as usize;
        let lut = if color.receiver.is_none()
            && (params.settings.use_scanner_lut || color_ref.has_remap())
        {
            Some(prepare_scanner_lut(
                channel_density,
                base_density,
                color,
                minimum,
                maximum,
                steps,
            ))
        } else {
            None
        };
        Self {
            channel_density,
            base_density,
            color,
            color_ref,
            lut,
        }
    }

    pub(crate) fn capture(&self, density: [f64; 3]) -> [f64; 3] {
        if let Some(receiver) = &self.color.receiver {
            let mut rgb = receiver.capture_f64(density, self.channel_density, self.base_density);
            if self.color_ref.has_remap() {
                let to_y = receiver.luminance_weights();
                let y = (0..3).map(|c| to_y[c] * rgb[c]).sum();
                let scale = self.color_ref.xyz_scale(y);
                for value in &mut rgb {
                    *value *= scale;
                }
            }
            return rgb;
        }
        let mut xyz = if let Some(lut) = &self.lut {
            let position: [f64; 3] =
                std::array::from_fn(|c| (density[c] - lut.minimum[c]) * lut.inverse_step[c]);
            pchip_interp(&lut.pchip, position[0], position[1], position[2]).map(|v| 10.0f64.powf(v))
        } else {
            let mut xyz = [0.0; 3];
            for i in 0..self.color.n_wl {
                let dye = self.channel_density[i];
                let d = density[0] * dye[0]
                    + density[1] * dye[1]
                    + density[2] * dye[2]
                    + if self.base_density.len() >= self.color.n_wl {
                        self.base_density[i]
                    } else {
                        0.0
                    };
                let light =
                    if d.is_nan() { 0.0 } else { 10.0f64.powf(-d) } * self.color.illuminant[i];
                xyz[0] += light * spektrafilm_math::spectral::CMF_X_F64[i];
                xyz[1] += light * spektrafilm_math::spectral::CMF_Y_F64[i];
                xyz[2] += light * spektrafilm_math::spectral::CMF_Z_F64[i];
            }
            xyz.map(|v| 10.0f64.powf(((v / self.color.normalization).max(0.0) + 1e-10).log10()))
        };
        let scale = self.color_ref.xyz_scale(xyz[1]);
        if scale != 1.0 {
            for value in &mut xyz {
                *value *= scale;
            }
        }
        let adapted: [f64; 3] = std::array::from_fn(|c| {
            self.color.adapt[c][0] * xyz[0]
                + self.color.adapt[c][1] * xyz[1]
                + self.color.adapt[c][2] * xyz[2]
        });
        std::array::from_fn(|c| {
            self.color.base_xyz_to_rgb[c][0] * adapted[0]
                + self.color.base_xyz_to_rgb[c][1] * adapted[1]
                + self.color.base_xyz_to_rgb[c][2] * adapted[2]
        })
    }
}

fn positive_scan_requested(params: &RuntimeParams) -> bool {
    params.scanner.scan_output == "positive_scan"
}

fn points_active(params: &RuntimeParams) -> bool {
    params.scanner.black_point != [0.0; 3] || params.scanner.white_point != [1.0; 3]
}

/// The film profile curves in the density domain the film stage emits: each
/// record measured over its own minimum.
fn developed_density_curves(profile: &Profile) -> Vec<[f64; 3]> {
    spektrafilm_model::density_curves::normalize_density_curves_f64(&profile.density_curves_f64())
}

/// One shared axis, with independent channel windows. Keep identity arithmetic
/// out of the historical path, including channels left at their defaults.
struct PointMap {
    start: f64,
    span: f64,
    black: [f64; 3],
    scale: [f64; 3],
}

impl PointMap {
    fn new(start: f64, end: f64, params: &RuntimeParams) -> Result<Self, String> {
        let scale = std::array::from_fn(|c| {
            1.0 / (params.scanner.white_point[c] - params.scanner.black_point[c])
        });
        if scale
            .iter()
            .any(|v: &f64| !v.is_finite() || !from_f64(*v).is_finite())
        {
            return Err("film-scan point window reciprocal is not representable; widen the black/white point window".into());
        }
        Ok(Self {
            start,
            span: end - start,
            black: params.scanner.black_point,
            scale,
        })
    }

    fn apply(&self, value: f64, channel: usize) -> f64 {
        if self.black[channel] == 0.0 && self.scale[channel] == 1.0 {
            value
        } else {
            self.start
                + (value - self.start - self.black[channel] * self.span) * self.scale[channel]
        }
    }
}

struct PositiveChannel {
    density: Vec<f64>,
    log_exposure: Vec<f64>,
    clear: f64,
}

struct PositiveCalibration {
    channels: [PositiveChannel; 3],
}

impl PositiveCalibration {
    #[allow(clippy::too_many_arguments)]
    fn from_profile(
        profile: &Profile,
        params: &RuntimeParams,
        backend: &dyn ComputeBackend,
        color_ref: &crate::color_reference::ColorReference,
        channel_density: &[[f64; 3]],
        base_density: &[f64],
        color: &crate::chain_prep::ScanColorContext,
    ) -> Result<Self, String> {
        let axis = &profile.data.log_exposure;
        if axis.len() < 2 || profile.data.density_curves.len() != axis.len() {
            return Err("positive_scan requires matching profile calibration curves with at least two samples".into());
        }
        let rows = ImageBuf::from_data(
            axis.len() as u32,
            1,
            developed_density_curves(profile)
                .iter()
                .flat_map(|row| row.iter().copied().map(from_f64))
                .collect(),
        );
        let captured = scan_spectral_rgb(
            &rows,
            profile,
            params,
            backend,
            color_ref,
            channel_density,
            base_density,
            color,
            false,
        );
        let mut channels = Vec::with_capacity(3);
        for channel in 0..3 {
            let mut density: Vec<f64> = Vec::with_capacity(axis.len());
            let mut exposure = Vec::with_capacity(axis.len());
            let mut clear = None;
            let mut ended = false;
            for (i, &log_e) in axis.iter().enumerate() {
                let value = to_f64(captured.data[i * 3 + channel]);
                if !log_e.is_finite() || !value.is_finite() || value <= 0.0 {
                    ended |= !density.is_empty();
                    continue;
                }
                if ended {
                    return Err(format!(
                        "positive_scan calibration has an invalid interior sample in channel {channel}"
                    ));
                }
                let reference = *clear.get_or_insert(value);
                let d = -(value / reference).log10();
                density.push(d);
                exposure.push(log_e);
            }
            // Pool adjacent violations to regularize measurement noise in the
            // inverse calibration without changing the forward film curves.
            let mut blocks: Vec<(f64, f64, usize)> = Vec::with_capacity(density.len());
            for (&d, &e) in density.iter().zip(&exposure) {
                blocks.push((d, e, 1));
                while blocks.len() >= 2 {
                    let last = blocks.len() - 1;
                    let (left_sum, _, left_count) = blocks[last - 1];
                    let (right_sum, _, right_count) = blocks[last];
                    if left_sum / (left_count as f64) < right_sum / (right_count as f64) {
                        break;
                    }
                    let right = blocks.pop().unwrap();
                    let left = blocks.last_mut().unwrap();
                    left.0 += right.0;
                    left.1 += right.1;
                    left.2 += right.2;
                }
            }
            density.clear();
            exposure.clear();
            let tolerance = if std::mem::size_of::<Scalar>() == 4 {
                2e-6
            } else {
                1e-12
            };
            for (density_sum, exposure_sum, count) in blocks {
                let d = density_sum / count as f64;
                if density.last().is_some_and(|last| d - *last <= tolerance) {
                    continue;
                }
                density.push(d);
                exposure.push(exposure_sum / count as f64);
            }
            if density.len() < 2 {
                return Err(format!(
                    "positive_scan calibration has fewer than two positive distinct samples in channel {channel}"
                ));
            }
            channels.push(PositiveChannel {
                density,
                log_exposure: exposure,
                clear: clear.unwrap(),
            });
        }
        if points_active(params) {
            let start = channels
                .iter()
                .map(|c| c.log_exposure[0])
                .fold(f64::NEG_INFINITY, f64::max);
            let end = channels
                .iter()
                .map(|c| *c.log_exposure.last().unwrap())
                .fold(f64::INFINITY, f64::min);
            if !start.is_finite() || !end.is_finite() || start >= end {
                return Err(
                    "positive_scan points require a finite shared log-exposure interval".into(),
                );
            }
        }
        Ok(Self {
            channels: channels
                .try_into()
                .map_err(|_| "positive_scan calibration requires three channels")?,
        })
    }
}

impl PositiveChannel {
    fn estimate(&self, value: f64) -> f64 {
        let d = -(value / self.clear).log10();
        let upper = self
            .density
            .partition_point(|x| *x < d)
            .clamp(1, self.density.len() - 1);
        let lower = upper - 1;
        let fraction = (d - self.density[lower]) / (self.density[upper] - self.density[lower]);
        self.log_exposure[lower] + fraction * (self.log_exposure[upper] - self.log_exposure[lower])
    }
}

fn positive_tone(
    log_e: f64,
    channel: usize,
    points: &PointMap,
    params: &RuntimeParams,
) -> (f64, f64) {
    let contrast = params.scanner.positive_contrast;
    let dmax = params.scanner.positive_dmax;
    let denominator = -(-std::f64::consts::LN_10 * dmax).exp_m1();
    let log_e = points.apply(log_e, channel);
    let e = log_e / std::f64::consts::LOG10_2 - interpretation::camera_ev(params)
        + params.scanner.positive_density_ev
        + params.scanner.positive_balance_ev[channel];
    let z = contrast * e;
    let t = if z >= 0.0 {
        1.0 / (1.0 + (-z).exp())
    } else {
        let a = z.exp();
        a / (1.0 + a)
    };
    let density = dmax * (1.0 - t);
    let positive = (-std::f64::consts::LN_10 * density).exp_m1();
    let output = (positive + denominator) / denominator;
    let derivative = std::f64::consts::LN_10
        * dmax
        * contrast
        * t
        * (1.0 - t)
        * (-std::f64::consts::LN_10 * density).exp()
        / (denominator * std::f64::consts::LOG10_2)
        * points.scale[channel];
    (output, derivative)
}

fn interpret_negative_capture(
    mut rgb: ImageBuf,
    calibration: &PositiveCalibration,
    params: &RuntimeParams,
) -> Result<ImageBuf, String> {
    let start = calibration
        .channels
        .iter()
        .map(|c| c.log_exposure[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let end = calibration
        .channels
        .iter()
        .map(|c| *c.log_exposure.last().unwrap())
        .fold(f64::INFINITY, f64::min);
    let points = PointMap::new(start, end, params)?;
    let tone = |log_e: f64, channel: usize| positive_tone(log_e, channel, &points, params);
    let endpoints: [[(f64, f64, f64); 2]; 3] = std::array::from_fn(|c| {
        let k = &calibration.channels[c];
        let last = k.density.len() - 1;
        std::array::from_fn(|side| {
            let i = if side == 0 { 0 } else { last };
            let (a, b) = if side == 0 { (0, 1) } else { (last - 1, last) };
            let capture = k.clear * 10.0f64.powf(-k.density[i]);
            let (output, derivative) = tone(k.log_exposure[i], c);
            let slope = -derivative * (k.log_exposure[b] - k.log_exposure[a])
                / ((k.density[b] - k.density[a]) * std::f64::consts::LN_10 * capture);
            (capture, output, slope)
        })
    });
    rgb.data.par_chunks_exact_mut(3).for_each(|px| {
        for channel in 0..3 {
            let value = to_f64(px[channel]);
            let (clear, low, low_slope) = endpoints[channel][0];
            let (dense, high, high_slope) = endpoints[channel][1];
            let output = if value >= clear {
                low + (value - clear) * low_slope
            } else if value <= dense {
                high + (value - dense) * high_slope
            } else {
                tone(calibration.channels[channel].estimate(value), channel).0
            };
            px[channel] = from_f64(output);
        }
    });
    Ok(rgb)
}

enum ExposureCalibration {
    Separable(PositiveCalibration),
    Coupled(interpretation::CoupledCalibration),
    Monochrome(PositiveCalibration),
}

impl ExposureCalibration {
    fn range(&self) -> [f64; 2] {
        match self {
            Self::Coupled(c) => c.range(),
            Self::Separable(c) | Self::Monochrome(c) => [
                c.channels
                    .iter()
                    .map(|k| k.log_exposure[0])
                    .fold(f64::NEG_INFINITY, f64::max),
                c.channels
                    .iter()
                    .map(|k| *k.log_exposure.last().unwrap())
                    .fold(f64::INFINITY, f64::min),
            ],
        }
    }

    fn estimate(&self, capture: [f64; 3]) -> Result<[f64; 3], String> {
        let estimate = |k: &PositiveChannel, value: f64| {
            let last = k.density.len() - 1;
            let clear = k.clear * 10.0f64.powf(-k.density[0]);
            let dense = k.clear * 10.0f64.powf(-k.density[last]);
            let endpoint = if value >= clear {
                Some((0, 1, clear))
            } else if value <= dense {
                Some((last, last - 1, dense))
            } else {
                None
            };
            match endpoint {
                Some((i, j, reference)) => {
                    k.log_exposure[i]
                        - (value - reference) * (k.log_exposure[j] - k.log_exposure[i])
                            / ((k.density[j] - k.density[i]) * std::f64::consts::LN_10 * reference)
                }
                None => k.estimate(value),
            }
        };
        let values = match self {
            Self::Coupled(c) => c.estimate(capture)?,
            Self::Separable(c) => std::array::from_fn(|i| estimate(&c.channels[i], capture[i])),
            Self::Monochrome(c) => [estimate(&c.channels[0], capture[0]); 3],
        };
        if values.iter().any(|v| !v.is_finite()) {
            return Err("scanner inverse produced nonfinite relative exposure".into());
        }
        Ok(values)
    }
}

fn prepare_exposure_calibration(
    profile: &Profile,
    params: &RuntimeParams,
    color_ref: &crate::color_reference::ColorReference,
    color: &crate::chain_prep::ScanColorContext,
    channel_density: &[[f64; 3]],
    base: &[f64],
    forward: &ForwardCapture<'_>,
) -> Result<ExposureCalibration, String> {
    let cpu = spektrafilm_gpu::cpu_backend::CpuBackend;
    if params.scanner.positive_interpretation == crate::params::PositiveInterpretation::Coupled
        && !profile.is_bw()
    {
        Ok(ExposureCalibration::Coupled(
            interpretation::CoupledCalibration::prepare(profile, forward)?,
        ))
    } else {
        let c = PositiveCalibration::from_profile(
            profile,
            params,
            &cpu,
            color_ref,
            channel_density,
            base,
            color,
        )?;
        Ok(
            if profile.is_bw()
                && params.scanner.positive_interpretation
                    == crate::params::PositiveInterpretation::Coupled
            {
                ExposureCalibration::Monochrome(c)
            } else {
                ExposureCalibration::Separable(c)
            },
        )
    }
}

pub(crate) fn prepare_setup(
    density: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    color_ref: &crate::color_reference::ColorReference,
    illuminant: Option<&str>,
) -> Result<ScannerSetup, String> {
    let mut params = params.clone();
    params.io.scan_film = true;
    let prepared = crate::chain_prep::PreparedChain::for_scan(profile, &params, illuminant);
    let dyes = crate::chain_prep::channel_density(profile);
    let base = profile.data.base_density.as_slice();
    let forward = ForwardCapture::prepare(profile, &params, &dyes, base, &prepared.scan, color_ref);
    let calibration = prepare_exposure_calibration(
        profile,
        &params,
        color_ref,
        &prepared.scan,
        &dyes,
        base,
        &forward,
    )?;
    let samples: Vec<[f64; 3]> = density
        .data
        .par_chunks_exact(3)
        .map(|p| calibration.estimate(forward.capture([to_f64(p[0]), to_f64(p[1]), to_f64(p[2])])))
        .collect::<Result<Vec<_>, _>>()?;
    ScannerSetup::from_samples(
        &samples,
        interpretation::camera_ev(&params),
        calibration.range(),
        &params.scanner,
    )
}

fn interpret_coupled_capture(
    rgb: &mut ImageBuf,
    calibration: &ExposureCalibration,
    params: &RuntimeParams,
) -> Result<(), String> {
    let [start, end] = calibration.range();
    let points = PointMap::new(start, end, params)?;
    rgb.data.par_chunks_exact_mut(3).try_for_each(|px| -> Result<(),String> {
        let capture = [to_f64(px[0]), to_f64(px[1]), to_f64(px[2])];
        let (e, anchor) = match calibration {
            ExposureCalibration::Coupled(c) => c.estimate_with_anchor(capture)?,
            ExposureCalibration::Monochrome(c) => {
                let k = &c.channels[0];
                let last = k.density.len()-1;
                let clear = k.clear*10.0f64.powf(-k.density[0]);
                let dense = k.clear*10.0f64.powf(-k.density[last]);
                let anchor = if capture[0] >= clear { Some([k.log_exposure[0];3]) }
                    else if capture[0] <= dense { Some([k.log_exposure[last];3]) }
                    else { None };
                (calibration.estimate(capture)?,anchor)
            }
            _ => (calibration.estimate(capture)?, None),
        };
        for c in 0..3 {
            let output = if let Some(anchor) = anchor {
                let (value, slope) = positive_tone(anchor[c], c, &points, params);
                value + slope * (e[c] - anchor[c])
            } else {
                positive_tone(e[c], c, &points, params).0
            };
            px[c] = from_f64(output);
            if !px[c].is_finite() {
                return Err("coupled scan endpoint output is not representable in the selected precision".into());
            }
        }
        Ok(())
    })?;
    Ok(())
}

/// Slide points use effective output-RGB density, not the dye-density axes.
struct SlidePoints {
    clear: [f64; 3],
    dense: [f64; 3],
    points: PointMap,
    endpoints: [[(f64, f64); 2]; 3],
}

impl SlidePoints {
    #[allow(clippy::too_many_arguments)]
    fn from_profile(
        profile: &Profile,
        params: &RuntimeParams,
        backend: &dyn ComputeBackend,
        color_ref: &crate::color_reference::ColorReference,
        channel_density: &[[f64; 3]],
        base_density: &[f64],
        color: &crate::chain_prep::ScanColorContext,
    ) -> Result<Self, String> {
        let calibration_scope = backend
            .observation_context()
            .filter(|c| c.enabled())
            .map(|c| {
                c.scope(
                    "scanner_film_points",
                    spektrafilm_gpu::telemetry::ObservationKind::Stage,
                    spektrafilm_gpu::telemetry::Purpose::Calibration,
                )
            });
        let calibration_backend = calibration_scope
            .as_ref()
            .map(|scope| spektrafilm_gpu::bind_backend(backend, scope.context().clone()));
        let backend = calibration_backend.as_deref().unwrap_or(backend);
        let mut maximum = [f64::NEG_INFINITY; 3];
        for row in &developed_density_curves(profile) {
            for c in 0..3 {
                if row[c].is_finite() {
                    maximum[c] = maximum[c].max(row[c]);
                }
            }
        }
        let references = ImageBuf::from_data(
            2,
            1,
            [0.0, 0.0, 0.0, maximum[0], maximum[1], maximum[2]]
                .into_iter()
                .map(from_f64)
                .collect(),
        );
        let captured = scan_spectral_rgb(
            &references,
            profile,
            params,
            backend,
            color_ref,
            channel_density,
            base_density,
            color,
            true,
        );
        let clear = std::array::from_fn(|c| to_f64(captured.data[c]));
        let maximum_capture: [f64; 3] = std::array::from_fn(|c| to_f64(captured.data[3 + c]));
        if clear
            .iter()
            .chain(&maximum_capture)
            .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("slide points require finite positive zero and maximum dye captures in every output RGB channel".into());
        }
        let density = (0..3)
            .map(|c| -(maximum_capture[c] / clear[c]).log10())
            .fold(f64::NEG_INFINITY, f64::max);
        if !density.is_finite() || density <= 0.0 {
            return Err(
                "slide points require a finite positive shared scanned-density interval".into(),
            );
        }
        let dense = clear.map(|v| v * 10.0f64.powf(-density));
        if dense.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err(
                "slide points require representable positive shared-density endpoints".into(),
            );
        }
        Self::new(clear, dense, PointMap::new(-density, 0.0, params)?)
    }

    fn new(clear: [f64; 3], dense: [f64; 3], points: PointMap) -> Result<Self, String> {
        let endpoints: [[(f64, f64); 2]; 3] = std::array::from_fn(|c| {
            std::array::from_fn(|side| {
                let capture = if side == 0 { clear[c] } else { dense[c] };
                let mapped = clear[c] * 10.0f64.powf(points.apply((capture / clear[c]).log10(), c));
                let slope = points.scale[c] * mapped / capture;
                (mapped, slope)
            })
        });
        if endpoints.iter().flatten().any(|&(output, slope)| {
            !output.is_finite()
                || !slope.is_finite()
                || !from_f64(output).is_finite()
                || !from_f64(slope).is_finite()
                || output <= 0.0
                || slope <= 0.0
                || to_f64(from_f64(output)) <= 0.0
                || to_f64(from_f64(slope)) <= 0.0
        }) {
            return Err("slide point endpoint output or tangent is not representable; widen the black/white point window".into());
        }
        Ok(Self {
            clear,
            dense,
            points,
            endpoints,
        })
    }

    fn map(&self, value: f64, c: usize) -> f64 {
        if self.points.black[c] == 0.0 && self.points.scale[c] == 1.0 {
            return value;
        }
        if value >= self.clear[c] {
            let (output, slope) = self.endpoints[c][0];
            return output + (value - self.clear[c]) * slope;
        }
        if value <= self.dense[c] {
            let (output, slope) = self.endpoints[c][1];
            return output + (value - self.dense[c]) * slope;
        }
        self.clear[c] * 10.0f64.powf(self.points.apply((value / self.clear[c]).log10(), c))
    }

    fn apply(&self, rgb: &mut ImageBuf) {
        rgb.data.par_chunks_exact_mut(3).for_each(|px| {
            for (c, value) in px.iter_mut().enumerate() {
                *value = from_f64(self.map(to_f64(*value), c));
            }
        });
    }
}

/// Recover pre-adaptation XYZ Y with the exact inverse of the actual scan
/// matrix. Stored RGB-to-XYZ matrices are rounded and are not its inverse.
fn scan_rgb_to_y(color: &crate::chain_prep::ScanColorContext) -> Result<[f64; 3], String> {
    let m: [[f64; 3]; 3] = std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            (0..3)
                .map(|k| color.base_xyz_to_rgb[i][k] * color.adapt[k][j])
                .sum()
        })
    });
    let cofactors = [
        m[1][2] * m[2][0] - m[1][0] * m[2][2],
        m[0][0] * m[2][2] - m[0][2] * m[2][0],
        m[0][2] * m[1][0] - m[0][0] * m[1][2],
    ];
    let determinant = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        + m[0][1] * cofactors[0]
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if !determinant.is_finite() || determinant == 0.0 {
        return Err("slide points require an invertible scan color transform".into());
    }
    Ok(cofactors.map(|v| v / determinant))
}

/// Linear optical capture before inversion, gamut compression and encoding.
pub fn capture_linear(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
) -> Result<ImageBuf, String> {
    if color_ref.has_remap() || params.settings.use_scanner_lut {
        return Err(
            "exact-cell scanning requires direct spectral capture without luminance remap or LUT"
                .to_owned(),
        );
    }
    let prepared = crate::chain_prep::PreparedChain::for_scan(profile, params, None);
    Ok(scan_spectral_rgb(
        density_cmy,
        profile,
        params,
        backend,
        color_ref,
        &crate::chain_prep::channel_density(profile),
        &profile.data.base_density,
        &prepared.scan,
        false,
    ))
}

/// Finish an area-averaged capture in the scanner's original stage order.
pub fn finish_capture(
    rgb: ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    gamut: &crate::gamut_compression::OutputGamutCompress,
) -> Result<ImageBuf, String> {
    let unused_density = ImageBuf::new(0, 0);
    scan_with_capture(
        &unused_density,
        profile,
        params,
        backend,
        color_ref,
        gamut,
        None,
        true,
        Some(rgb),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn scan_with_options(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    gamut: &crate::gamut_compression::OutputGamutCompress,
    scan_illuminant: Option<&str>,
    include_base: bool,
) -> Result<ImageBuf, String> {
    scan_with_capture(
        density_cmy,
        profile,
        params,
        backend,
        color_ref,
        gamut,
        scan_illuminant,
        include_base,
        None,
    )
}

// Scan options preserve the existing stage interface.
#[allow(clippy::too_many_arguments)]
fn scan_with_capture(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    gamut: &crate::gamut_compression::OutputGamutCompress,
    scan_illuminant: Option<&str>,
    include_base: bool,
    captured_rgb: Option<ImageBuf>,
) -> Result<ImageBuf, String> {
    let positive_scan = positive_scan_requested(params);
    let slide_points = params.io.scan_film && profile.is_positive() && points_active(params);
    let _positive_scan = positive_scan.then(|| {
        super::StageObservation::cpu(
            backend,
            "positive_scan",
            spektrafilm_gpu::telemetry::CpuReason::PositiveScanOutput,
        )
    });
    let _slide_points = slide_points.then(|| {
        super::StageObservation::cpu(
            backend,
            "scanner_film_points",
            spektrafilm_gpu::telemetry::CpuReason::ScannerFilmPoints,
        )
    });
    let cpu_fallback = positive_scan
        .then(|| {
            backend
                .observation_context()
                .filter(|context| context.enabled())
                .map(|context| {
                    spektrafilm_gpu::bind_cpu_fallback(
                        context.clone(),
                        spektrafilm_gpu::telemetry::CpuReason::PositiveScanOutput,
                    )
                })
        })
        .flatten();
    let backend = if positive_scan {
        static CPU_BACKEND: spektrafilm_gpu::cpu_backend::CpuBackend =
            spektrafilm_gpu::cpu_backend::CpuBackend;
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
    let prepared = crate::chain_prep::PreparedChain::for_scan(profile, params, scan_illuminant);
    let scan_context = &prepared.scan;
    let base_density = base_density.as_ref();
    let receiver_reference = scan_context.receiver.as_ref().map(|receiver| {
        receiver.correction(profile, params, color_ref, &channel_density, base_density)
    });
    let color_ref = receiver_reference.as_ref().unwrap_or(color_ref);

    // Dispatch spectral integration to backend (GPU or CPU). The backend
    // applies the two matrices in sequence — we pass them separately.
    //
    // `use_scanner_lut` path: evaluate `scan_spectral` on a 17³ CMY
    // grid and PCHIP-interpolate per pixel. Glare is added in RGB
    // space after this returns (the existing post-step at the bottom
    // of this function), so swapping to the LUT path is glare-safe.
    // The path selection (including the faithful B&W/slide luminance
    // remap) lives in `scan_spectral_rgb` so the reference samples
    // below cannot diverge from this bulk scan.
    let mut rgb = captured_rgb.unwrap_or_else(|| {
        scan_spectral_rgb(
            density_cmy,
            profile,
            params,
            backend,
            color_ref,
            &channel_density,
            base_density,
            scan_context,
            slide_points,
        )
    });

    // Historical white/black correction stays inside the scan LUT. Active
    // slide points defer only that remap until after capture blur and points.
    // When no remap is built we apply nothing: scan_film+negative because upstream
    // Python explicitly skips it (`black_white_xyz_correction`: "do not
    // correct negative film scans"), and a positive print paper because
    // upstream crashes there (its printing exposure correction returns None)
    // — unsupported, and unrepresentable with the shipped profiles anyway.

    // Viewing glare (Python: `add_glare(xyz, illuminant_xyz, glare)` between scan_spectral
    // and chromatic adapt + matrix). Since CAT+matrix is linear, we equivalently add
    // glare in RGB space by pre-multiplying the illuminant XYZ through the same matrix.
    //   Python: rgb = M @ (xyz + g*illu) = M@xyz + g*(M@illu)
    //   Rust:   rgb = M @ xyz; then rgb += g * (M@illu)
    //
    // Python 0.3.4 `ScanningStage._density_to_rgb` sets `glare = None` on the
    // `io.scan_film` path — direct-film scans get no viewing glare, and
    // `film_render.glare` is never read anywhere upstream. Only the print
    // path consumes `print_render.glare` (scan_film=false).
    let glare = (!params.io.scan_film)
        .then_some(&params.print_render.glare)
        .filter(|g| g.active && g.percent > 0.0);
    if let Some(glare) = glare {
        let _glare = super::StageObservation::cpu(
            backend,
            "glare",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        // Shared scan context keeps the CPU two-step CAT→RGB operation
        // order used by the reference path.
        let glare_rgb_offset_f64 = crate::chain_prep::glare_rgb_offset_f64(scan_context);
        let glare_rgb_offset: [Scalar; 3] = glare_rgb_offset_f64.map(from_f64);
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
    if slide_points {
        let points = SlidePoints::from_profile(
            profile,
            params,
            backend,
            color_ref,
            &channel_density,
            base_density,
            scan_context,
        )?;
        if params.scanner.lens_blur > 0.0 {
            rgb = backend.gaussian_blur(&rgb, params.scanner.lens_blur);
        }
        points.apply(&mut rgb);
        if color_ref.has_remap() {
            let to_y = match &scan_context.receiver {
                Some(receiver) => receiver.luminance_weights(),
                None => scan_rgb_to_y(scan_context)?,
            };
            rgb.data.par_chunks_exact_mut(3).for_each(|px| {
                let y = (0..3).map(|c| to_y[c] * to_f64(px[c])).sum();
                let scale = color_ref.xyz_scale(y);
                if scale != 1.0 {
                    for value in px {
                        *value = from_f64(to_f64(*value) * scale);
                    }
                }
            });
        }
    }

    // Positive interpretation is scanner-domain: optical capture and lens
    // blur happen before the calibrated negative endpoint inversion.
    if positive_scan {
        if params.scanner.lens_blur > 0.0 {
            rgb = backend.gaussian_blur(&rgb, params.scanner.lens_blur);
        }
        if params.scanner.positive_interpretation
            == crate::params::PositiveInterpretation::Separable
        {
            let calibration = PositiveCalibration::from_profile(
                profile,
                params,
                backend,
                color_ref,
                &channel_density,
                base_density,
                scan_context,
            )?;
            rgb = interpret_negative_capture(rgb, &calibration, params)?;
        } else {
            let forward = ForwardCapture::prepare(
                profile,
                params,
                &channel_density,
                base_density,
                scan_context,
                color_ref,
            );
            let calibration = prepare_exposure_calibration(
                profile,
                params,
                color_ref,
                scan_context,
                &channel_density,
                base_density,
                &forward,
            )?;
            interpret_coupled_capture(&mut rgb, &calibration, params)?;
        }
    }
    if positive_scan && params.scanner.finish == crate::params::ScannerFinish::LabScan {
        let _finish = super::StageObservation::cpu(
            backend,
            "scanner_finish",
            spektrafilm_gpu::telemetry::CpuReason::PositiveScanOutput,
        );
        finish::apply(
            &mut rgb,
            output_space,
            params.scanner.finish_strength,
            !profile.is_bw(),
        );
    }

    // Output gamut compression — direct scans preserve the historical
    // capture order; positive scans compress only after interpretation.
    if gamut.is_active() {
        let _gamut = super::StageObservation::cpu(
            backend,
            "output_gamut",
            spektrafilm_gpu::telemetry::CpuReason::UnsupportedOutputGamut,
        );
        rgb.data.par_chunks_exact_mut(3).for_each(|px| {
            let out = gamut.compress([px[0] as f64, px[1] as f64, px[2] as f64]);
            px[0] = from_f64(out[0]);
            px[1] = from_f64(out[1]);
            px[2] = from_f64(out[2]);
        });
    }

    // Direct scans apply optical blur after gamut compression, preserving
    // the established direct-scan behavior.
    if !positive_scan && !slide_points && params.scanner.lens_blur > 0.0 {
        rgb = backend.gaussian_blur(&rgb, params.scanner.lens_blur);
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
    // Magazine appearance is optional finishing, not a route: it applies to
    // any scanned base workflow after the scanner-domain effects.
    let magazine_active =
        params.magazine_print_color.active && params.magazine_print_color.strength > 0.0;
    let encoded_domain = params.io.output_cctf_encoding || grain_v2_active;
    if encoded_domain {
        let _encoding = super::StageObservation::cpu(
            backend,
            "output_transfer",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
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
                spektrafilm_gpu::telemetry::CpuReason::GrainV2Unsupported,
            );
            spektrafilm_model::grain::v2::apply_cpu(&rgb, grain)
        });
    }
    // Magazine color is the final post-scan appearance, after scanner-domain
    // grain. A linear export uses the helper's finite transfer round-trip and
    // remains linear at the API boundary.
    if magazine_active {
        let _magazine = super::StageObservation::cpu(
            backend,
            "magazine_print_color",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        crate::magazine_print_color::apply(
            &mut rgb,
            &params.magazine_print_color,
            output_space,
            encoded_domain,
        );
    }
    // Linear exports retain the same grain realization as encoded exports.
    // Decode only the transfer; the stored same-space matrix ran above once.
    if grain_v2_active && !params.io.output_cctf_encoding {
        let _encoding = super::StageObservation::cpu(
            backend,
            "output_transfer",
            spektrafilm_gpu::telemetry::CpuReason::BackendDefault,
        );
        rgb.data.par_iter_mut().for_each(|value| {
            *value = from_f64(colorspace::cctf_decode(*value as f64, output_space.cctf));
        });
    }

    Ok(rgb)
}

/// Scan with the profile's own viewing illuminant and the film base included
/// — the per-stage topology entry point.
pub fn process(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    gamut: &crate::gamut_compression::OutputGamutCompress,
) -> Result<ImageBuf, String> {
    scan_with_options(
        density_cmy,
        profile,
        params,
        backend,
        color_ref,
        gamut,
        None,
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calibration() -> PositiveCalibration {
        PositiveCalibration {
            channels: std::array::from_fn(|_| PositiveChannel {
                density: vec![0.0, 1.0, 2.0],
                log_exposure: vec![-1.0, 0.0, 1.0],
                clear: 1.0,
            }),
        }
    }

    #[test]
    fn calibrated_response_preserves_polarity_and_unbounded_headroom() {
        let params = RuntimeParams::default();
        let input = ImageBuf::from_data(
            5,
            1,
            [100.0, 1.0, 0.1, 0.01, -100.0]
                .into_iter()
                .flat_map(|v| [from_f64(v); 3])
                .collect(),
        );
        let output = interpret_negative_capture(input, &calibration(), &params).unwrap();
        let values: Vec<f64> = output.data.iter().step_by(3).map(|v| to_f64(*v)).collect();
        assert!(values.windows(2).all(|w| w[1] > w[0]));
        assert!(values[0] < 0.0 && values[4] > 1.0);
    }

    #[test]
    fn density_and_balance_controls_change_only_requested_channels() {
        let input = ImageBuf::from_data(1, 1, vec![from_f64(0.1); 3]);
        let mut params = RuntimeParams::default();
        let baseline = interpret_negative_capture(input.clone(), &calibration(), &params).unwrap();
        params.scanner.positive_density_ev += 1.0;
        let brighter = interpret_negative_capture(input.clone(), &calibration(), &params).unwrap();
        assert!(brighter.data.iter().zip(&baseline.data).all(|(a, b)| a > b));
        params.scanner.positive_density_ev -= 1.0;
        params.scanner.positive_balance_ev = [1.0, 0.0, -1.0];
        let balanced = interpret_negative_capture(input, &calibration(), &params).unwrap();
        assert!(balanced.data[0] > baseline.data[0]);
        assert_eq!(balanced.data[1], baseline.data[1]);
        assert!(balanced.data[2] < baseline.data[2]);
    }

    #[test]
    fn very_small_output_density_stays_finite_and_ordered() {
        let mut params = RuntimeParams::default();
        params.scanner.positive_dmax = 1e-18;
        let input = ImageBuf::from_data(
            2,
            1,
            [0.5, 0.5, 0.5, 0.1, 0.1, 0.1]
                .into_iter()
                .map(from_f64)
                .collect(),
        );
        let output = interpret_negative_capture(input, &calibration(), &params).unwrap();
        assert!(output.data.iter().all(|v| v.is_finite()));
        assert!(output.data[3] > output.data[0]);
    }

    #[test]
    fn negative_points_share_axis_and_preserve_neutrality() {
        let mut params = RuntimeParams::default();
        params.scanner.black_point = [0.2; 3];
        params.scanner.white_point = [0.8; 3];
        let mut calibration = calibration();
        calibration.channels[1].density = vec![0.0, 0.5, 1.0];
        calibration.channels[2].density = vec![0.0, 1.5, 3.0];
        let input = ImageBuf::from_data(
            3,
            1,
            [-0.7, 0.0, 0.5]
                .into_iter()
                .flat_map(|e| {
                    [
                        10.0f64.powf(-(e + 1.0)),
                        10.0f64.powf(-0.5 * (e + 1.0)),
                        10.0f64.powf(-1.5 * (e + 1.0)),
                    ]
                    .map(from_f64)
                })
                .collect(),
        );
        let equal = interpret_negative_capture(input.clone(), &calibration, &params).unwrap();
        for px in equal.data.as_chunks::<3>().0 {
            assert!((to_f64(px[0]) - to_f64(px[1])).abs() < 2e-6);
            assert!((to_f64(px[0]) - to_f64(px[2])).abs() < 2e-6);
        }
        params.scanner.black_point[0] = 0.1;
        let independent = interpret_negative_capture(input, &calibration, &params).unwrap();
        for (a, b) in independent
            .data
            .as_chunks::<3>()
            .0
            .iter()
            .zip(equal.data.as_chunks::<3>().0)
        {
            assert_ne!(a[0], b[0]);
            assert_eq!(&a[1..], &b[1..]);
        }
    }

    #[test]
    fn active_negative_endpoint_tangents_chain_point_derivative() {
        let mut params = RuntimeParams::default();
        params.scanner.black_point = [0.15; 3];
        params.scanner.white_point = [0.85; 3];
        for endpoint in [1.0, 0.01] {
            let h = endpoint * 1e-3;
            let input = ImageBuf::from_data(
                3,
                1,
                [endpoint - h, endpoint, endpoint + h]
                    .into_iter()
                    .flat_map(|v| [from_f64(v); 3])
                    .collect(),
            );
            let output = interpret_negative_capture(input, &calibration(), &params).unwrap();
            let left = (to_f64(output.data[3]) - to_f64(output.data[0])) / h;
            let right = (to_f64(output.data[6]) - to_f64(output.data[3])) / h;
            assert!((left - right).abs() < 0.02 * left.abs());
        }
        let signed = ImageBuf::from_data(
            2,
            1,
            [-1.0, -1.0, -1.0, 2.0, 2.0, 2.0]
                .into_iter()
                .map(from_f64)
                .collect(),
        );
        assert!(
            interpret_negative_capture(signed, &calibration(), &params)
                .unwrap()
                .data
                .iter()
                .all(|v| v.is_finite())
        );
    }

    #[test]
    fn slide_points_share_density_map_and_extend_signed_headroom() {
        let mut params = RuntimeParams::default();
        params.scanner.black_point = [0.2; 3];
        params.scanner.white_point = [0.8; 3];
        let mut points = SlidePoints::new(
            [1.0, 2.0, 0.5],
            [0.01, 0.02, 0.005],
            PointMap::new(-2.0, 0.0, &params).unwrap(),
        )
        .unwrap();
        for fraction in [0.1, 0.4, 0.7] {
            let mapped = std::array::from_fn::<_, 3, _>(|c| {
                points.map(points.clear[c] * fraction, c) / points.clear[c]
            });
            assert!((mapped[0] - mapped[1]).abs() < 1e-12);
            assert!((mapped[0] - mapped[2]).abs() < 1e-12);
        }
        for endpoint in [0.01, 1.0] {
            let h = endpoint * 1e-5;
            let left = (points.map(endpoint, 0) - points.map(endpoint - h, 0)) / h;
            let right = (points.map(endpoint + h, 0) - points.map(endpoint, 0)) / h;
            assert!((left - right).abs() < 1e-3 * left.abs());
        }
        assert!(points.map(-1.0, 0).is_finite());
        assert!(points.map(-1.0, 0) < 0.0);
        assert!(points.map(10.0, 0) > points.clear[0]);
        let unchanged = points.map(0.2, 1);
        points.points.black[0] = 0.1;
        assert_eq!(points.map(0.2, 1), unchanged);
        let identity = PointMap::new(-2.0, 0.0, &RuntimeParams::default()).unwrap();
        points.points = identity;
        for value in [-100.0, -0.0, 0.2, 1.0, 100.0] {
            assert_eq!(points.map(value, 0).to_bits(), value.to_bits());
        }
    }

    fn profile(name: &str) -> Profile {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        crate::profile::load_profile_by_name(&data, name).unwrap()
    }

    #[test]
    fn slide_points_run_after_blur_before_original_correction() {
        let film = profile("kodak_ektachrome_100");
        let print = profile("kodak_portra_endura");
        let mut params = RuntimeParams::default();
        params.io.scan_film = true;
        params.io.output_cctf_encoding = false;
        params.film_render.grain.active = false;
        params.scanner.unsharp_mask = [0.0; 2];
        params.scanner.lens_blur = 0.7;
        params.scanner.white_correction = true;
        params.scanner.black_correction = true;
        params.scanner.black_point = [0.1, 0.2, 0.15];
        params.scanner.white_point = [0.9, 0.85, 0.95];
        let correction = crate::color_reference::ColorReference::compute(
            &film,
            &print,
            &params,
            &[],
            1.0,
            [0.0; 3],
        );
        let context = crate::chain_prep::PreparedChain::for_scan(&film, &params, None);
        let channel = crate::chain_prep::channel_density(&film);
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let input = ImageBuf::from_data(
            3,
            1,
            [0.1, 0.2, 0.3, 1.0, 1.2, 1.4, 2.0, 1.8, 1.6]
                .into_iter()
                .map(from_f64)
                .collect(),
        );
        let capture = scan_spectral_rgb(
            &input,
            &film,
            &params,
            &backend,
            &correction,
            &channel,
            &film.data.base_density,
            &context.scan,
            true,
        );
        let points = SlidePoints::from_profile(
            &film,
            &params,
            &backend,
            &correction,
            &channel,
            &film.data.base_density,
            &context.scan,
        )
        .unwrap();
        let mut expected = backend.gaussian_blur(&capture, params.scanner.lens_blur);
        points.apply(&mut expected);
        let y = scan_rgb_to_y(&context.scan).unwrap();
        for px in expected.data.as_chunks_mut::<3>().0 {
            let scale = correction.xyz_scale((0..3).map(|c| y[c] * to_f64(px[c])).sum());
            for value in px {
                *value = from_f64(to_f64(*value) * scale);
            }
        }
        let actual = process(
            &input,
            &film,
            &params,
            &backend,
            &correction,
            &crate::gamut_compression::OutputGamutCompress::identity(),
        )
        .unwrap();
        assert_eq!(actual.data, expected.data);
        let mut wrong_order = capture;
        points.apply(&mut wrong_order);
        wrong_order = backend.gaussian_blur(&wrong_order, params.scanner.lens_blur);
        assert!(
            wrong_order
                .data
                .iter()
                .zip(&actual.data)
                .any(|(a, b)| (to_f64(*a) - to_f64(*b)).abs() > 1e-5)
        );
    }

    #[test]
    fn default_slide_points_preserve_historical_capture_and_blur() {
        let film = profile("kodak_ektachrome_100");
        let print = profile("kodak_portra_endura");
        let mut params = RuntimeParams::default();
        params.io.scan_film = true;
        params.io.output_cctf_encoding = false;
        params.film_render.grain.active = false;
        params.scanner.unsharp_mask = [0.0; 2];
        params.scanner.lens_blur = 0.7;
        params.scanner.white_correction = true;
        params.scanner.black_correction = true;
        let correction = crate::color_reference::ColorReference::compute(
            &film,
            &print,
            &params,
            &[],
            1.0,
            [0.0; 3],
        );
        let context = crate::chain_prep::PreparedChain::for_scan(&film, &params, None);
        let channel = crate::chain_prep::channel_density(&film);
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let input = ImageBuf::from_data(
            3,
            1,
            [0.1, 0.2, 0.3, 1.0, 1.2, 1.4, 2.0, 1.8, 1.6]
                .into_iter()
                .map(from_f64)
                .collect(),
        );
        let historical_capture = scan_spectral_rgb(
            &input,
            &film,
            &params,
            &backend,
            &correction,
            &channel,
            &film.data.base_density,
            &context.scan,
            false,
        );
        let expected = backend.gaussian_blur(&historical_capture, params.scanner.lens_blur);
        let actual = process(
            &input,
            &film,
            &params,
            &backend,
            &correction,
            &crate::gamut_compression::OutputGamutCompress::identity(),
        )
        .unwrap();
        assert_eq!(actual.data, expected.data);
    }

    #[test]
    fn paper_and_direct_negative_ignore_stored_points() {
        let backend = spektrafilm_gpu::cpu_backend::CpuBackend;
        let correction = crate::color_reference::ColorReference::identity();
        let gamut = crate::gamut_compression::OutputGamutCompress::identity();
        let input = ImageBuf::from_data(1, 1, vec![from_f64(0.5); 3]);
        for (name, scan_film) in [("kodak_portra_endura", false), ("kodak_portra_400", true)] {
            let profile = profile(name);
            let mut params = RuntimeParams::default();
            params.io.scan_film = scan_film;
            params.film_render.grain.active = false;
            let baseline =
                process(&input, &profile, &params, &backend, &correction, &gamut).unwrap();
            params.scanner.black_point = [0.2; 3];
            params.scanner.white_point = [0.8; 3];
            let active = process(&input, &profile, &params, &backend, &correction, &gamut).unwrap();
            assert_eq!(active.data, baseline.data);
        }
    }

    #[test]
    fn public_slide_scan_rejects_unrepresentable_endpoint_window() {
        let film = profile("kodak_ektachrome_100");
        let mut params = RuntimeParams::default();
        params.io.scan_film = true;
        params.film_render.grain.active = false;
        params.scanner.white_point = [0.001; 3];
        let result = process(
            &ImageBuf::from_data(1, 1, vec![from_f64(0.5); 3]),
            &film,
            &params,
            &spektrafilm_gpu::cpu_backend::CpuBackend,
            &crate::color_reference::ColorReference::identity(),
            &crate::gamut_compression::OutputGamutCompress::identity(),
        );
        assert!(
            result
                .unwrap_err()
                .contains("endpoint output or tangent is not representable")
        );
    }

    #[test]
    fn public_positive_scan_rejects_unrepresentable_window_reciprocal() {
        let film = profile("kodak_portra_400");
        let mut params = RuntimeParams::default();
        params.io.scan_film = true;
        params.scanner.scan_output = "positive_scan".into();
        params.film_render.grain.active = false;
        params.scanner.white_point = [1e-320; 3];
        let result = process(
            &ImageBuf::from_data(1, 1, vec![from_f64(0.5); 3]),
            &film,
            &params,
            &spektrafilm_gpu::cpu_backend::CpuBackend,
            &crate::color_reference::ColorReference::identity(),
            &crate::gamut_compression::OutputGamutCompress::identity(),
        );
        assert!(
            result
                .unwrap_err()
                .contains("point window reciprocal is not representable")
        );
    }

    #[test]
    fn monochrome_coupled_preserves_signed_endpoint_headroom() {
        let params = RuntimeParams::default();
        let calibration = ExposureCalibration::Monochrome(calibration());
        let mut image = ImageBuf::from_data(
            5,
            1,
            [100.0, 1.0, 0.1, 0.01, -100.0]
                .into_iter()
                .flat_map(|v| [from_f64(v); 3])
                .collect(),
        );
        interpret_coupled_capture(&mut image, &calibration, &params).unwrap();
        let values: Vec<_> = image
            .data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| to_f64(p[0]))
            .collect();
        assert!(values.windows(2).all(|w| w[0] < w[1]));
        assert!(values[0] < 0.0 && values[4] > 1.0);
        assert!(
            image
                .data
                .as_chunks::<3>()
                .0
                .iter()
                .all(|p| p[0] == p[1] && p[1] == p[2])
        );
    }
}
