/// Scanning stage: convert print/film density back to RGB.
///
/// Dispatches the spectral integration to the GPU backend when available.
use rayon::prelude::*;
use spektrafilm_backend::ComputeBackend;
use spektrafilm_math::colorspace;
use spektrafilm_math::image::ImageBuf;
use spektrafilm_math::precision::{Scalar, from_f64, to_f64};

use crate::params::RuntimeParams;
use crate::profile::Profile;
mod coupled;
mod finish;
mod interpretation;
mod spectral;
pub use crate::scanner_setup::ScannerSetup;
use spectral::ForwardCapture;
pub(crate) use spectral::scan_spectral_rgb;

fn negative_to_positive_requested(params: &RuntimeParams) -> bool {
    params.scanner.scan_output == "negative_to_positive"
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
            return Err("negative_to_positive requires matching profile calibration curves with at least two samples".into());
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
                        "negative_to_positive calibration has an invalid interior sample in channel {channel}"
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
                    "negative_to_positive calibration has fewer than two positive distinct samples in channel {channel}"
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
                    "negative_to_positive points require a finite shared log-exposure interval"
                        .into(),
                );
            }
        }
        Ok(Self {
            channels: channels
                .try_into()
                .map_err(|_| "negative_to_positive calibration requires three channels")?,
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
    let contrast = params.scanner.conversion_contrast;
    let dmax = params.scanner.conversion_dmax;
    let denominator = -(-std::f64::consts::LN_10 * dmax).exp_m1();
    let log_e = points.apply(log_e, channel);
    let e = log_e / std::f64::consts::LOG10_2 - interpretation::camera_ev(params)
        + params.scanner.conversion_brightness_ev
        + params.scanner.conversion_balance_ev[channel];
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

/// Record-density acquisition for the assembled scanner acquisition fence:
/// the `density_reading` branch of `scan_with_reference`, with the same
/// calibration preparation.
pub(crate) fn interpret_negative_density_stage(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
) -> Result<ImageBuf, String> {
    let calibration = interpretation::DensityCalibration::prepare(profile)?;
    interpret_negative_density(density_cmy, &calibration, params, backend)
}

/// Record-density interpretation: the narrowband lab-scanner inverse of the
/// film's own characteristic curves. Scanner optics blur transmitted light
/// record by record, before the sensor reads density, so the blur runs here in
/// the density domain and the capture never enters the loop.
fn interpret_negative_density(
    density_cmy: &ImageBuf,
    calibration: &interpretation::DensityCalibration,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
) -> Result<ImageBuf, String> {
    let [start, end] = calibration.range();
    let points = PointMap::new(start, end, params)?;
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
                px[channel] = from_f64(positive_tone(exposure[channel], channel, &points, params).0);
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

enum ExposureCalibration {
    Density(interpretation::DensityCalibration),
    Separable(PositiveCalibration),
    Coupled(interpretation::CoupledCalibration),
    Monochrome(PositiveCalibration),
}

impl ExposureCalibration {
    fn range(&self) -> [f64; 2] {
        match self {
            Self::Density(c) => c.range(),
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
            Self::Density(_) => {
                return Err("density interpretation reads record densities, not a capture".into());
            }
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
    if params.scanner.negative_conversion_method == crate::params::NegativeConversionMethod::Density
    {
        return Ok(ExposureCalibration::Density(
            interpretation::DensityCalibration::prepare(profile)?,
        ));
    }
    let cpu = spektrafilm_backend::cpu_backend::CpuBackend;
    if params.scanner.negative_conversion_method == crate::params::NegativeConversionMethod::Coupled
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
                && params.scanner.negative_conversion_method
                    == crate::params::NegativeConversionMethod::Coupled
            {
                ExposureCalibration::Monochrome(c)
            } else {
                ExposureCalibration::Separable(c)
            },
        )
    }
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
                    spektrafilm_backend::telemetry::ObservationKind::Stage,
                    spektrafilm_backend::telemetry::Purpose::Calibration,
                )
            });
        let calibration_backend = calibration_scope
            .as_ref()
            .map(|scope| spektrafilm_backend::bind_backend(backend, scope.context().clone()));
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
    density: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    gamut: &crate::gamut_compression::OutputGamutCompress,
    illuminant: Option<&str>,
    include_base: bool,
    capture: Option<ImageBuf>,
) -> Result<ImageBuf, String> {
    scan_with_reference(
        density,
        profile,
        params,
        backend,
        color_ref,
        gamut,
        illuminant,
        include_base,
        capture,
        None,
    )
}

fn scan_with_reference(
    density_cmy: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    gamut: &crate::gamut_compression::OutputGamutCompress,
    scan_illuminant: Option<&str>,
    include_base: bool,
    captured_rgb: Option<ImageBuf>,
    reference: Option<&crate::digital_reference::DigitalReference>,
) -> Result<ImageBuf, String> {
    let negative_to_positive = negative_to_positive_requested(params);
    let reference_mode = params.scanner.negative_conversion_method
        == crate::params::NegativeConversionMethod::DigitalReference;
    if reference_mode {
        params.validate()?;
        crate::pipeline_calibration::validate_scan_output(params, profile)?;
        if captured_rgb.is_some() || !include_base || scan_illuminant.is_some() {
            return Err("Digital Reference requires developed densities with film base and its fixed illumination".into());
        }
        if reference.is_none() {
            return Err("Digital Reference requires runtime-prepared spectral assets from the configured data directory".into());
        }
    }
    // The density reading acquires no capture, so the spectral scan and the
    // receiver correction it feeds are skipped entirely. Where a capture does
    // arrive, `density` behaves as `separable`.
    let density_reading = negative_to_positive
        && params.scanner.negative_conversion_method
            == crate::params::NegativeConversionMethod::Density
        && captured_rgb.is_none();
    let slide_points = params.io.scan_film && profile.is_positive() && points_active(params);
    let _negative_to_positive = negative_to_positive.then(|| {
        super::StageObservation::cpu(
            backend,
            "negative_to_positive",
            spektrafilm_backend::telemetry::CpuReason::NegativeToPositiveOutput,
        )
    });
    let _slide_points = slide_points.then(|| {
        super::StageObservation::cpu(
            backend,
            "scanner_film_points",
            spektrafilm_backend::telemetry::CpuReason::ScannerFilmPoints,
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
    let prepared = crate::chain_prep::PreparedChain::for_scan(profile, params, scan_illuminant);
    let scan_context = &prepared.scan;
    let base_density = base_density.as_ref();
    let receiver_reference = if density_reading || reference_mode {
        None
    } else {
        scan_context.receiver.as_ref().map(|receiver| {
            receiver.correction(profile, params, color_ref, &channel_density, base_density)
        })
    };
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
    let mut rgb = if reference_mode {
        reference
            .expect("validated prepared reference")
            .convert(density_cmy, params, backend)?
    } else if density_reading {
        let calibration = interpretation::DensityCalibration::prepare(profile)?;
        interpret_negative_density(density_cmy, &calibration, params, backend)?
    } else {
        captured_rgb.unwrap_or_else(|| {
            scan_spectral_rgb(
                density_cmy,
                profile,
                params,
                backend,
                color_ref,
                &channel_density,
                base_density,
                scan_context,
                !negative_to_positive,
            )
        })
    };
    // Scanner readout optics operate on the linear acquisition before glare,
    // points, negative inversion, finish, or output gamut correction. Density
    // interpretation keeps its dedicated transmission-domain blur above.
    if params.scanner.lens_blur > 0.0 && !density_reading && !reference_mode {
        rgb = backend.gaussian_blur(&rgb, params.scanner.lens_blur);
    }

    // Direct acquisition defers white/black correction until after scanner
    // blur, print viewing glare, and slide points. Default points follow the
    // same acquisition order; correction is applied only when a remap exists.

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
            spektrafilm_backend::telemetry::CpuReason::BackendDefault,
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
    apply_scan_readout_interpretation(
        &mut rgb,
        profile,
        params,
        backend,
        color_ref,
        &channel_density,
        base_density,
        scan_context,
        output_space,
        negative_to_positive,
        reference_mode,
        density_reading,
    )?;

    // Output gamut compression — direct scans preserve the historical
    // capture order; converted negative images compress only after interpretation.
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

    // Scanner blur runs at the linear acquisition boundary above.

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

    Ok(rgb)
}

/// Scanner-domain readout interpretation shared by the per-stage route and
/// the resident GPU chain fence (pipe architecture, capability class):
/// slide points, the faithful B&W/slide luminance remap,
/// negative-to-positive interpretation, and the LabScan finish, in the CPU
/// scanning order — after optical blur and glare, before output gamut
/// compression. The per-stage route supplies its acquisition variants;
/// the resident fence always interprets a captured image, so it passes
/// `reference_mode = false, density_reading = false`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_scan_readout_interpretation(
    rgb: &mut ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    channel_density: &[[f64; 3]],
    base_density: &[f64],
    scan_context: &crate::chain_prep::ScanColorContext,
    output_space: &colorspace::RgbColorSpace,
    negative_to_positive: bool,
    reference_mode: bool,
    density_reading: bool,
) -> Result<(), String> {
    let slide_points = params.io.scan_film && profile.is_positive() && points_active(params);
    if slide_points {
        let points = SlidePoints::from_profile(
            profile,
            params,
            backend,
            color_ref,
            channel_density,
            base_density,
            scan_context,
        )?;
        points.apply(rgb);
    }
    if !negative_to_positive && color_ref.has_remap() {
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

    // Positive interpretation is scanner-domain for the capture methods:
    // optical capture and lens blur happen before the calibrated negative
    // endpoint inversion. The density reading ran above, where it blurred
    // transmitted light record by record instead.
    if negative_to_positive && !density_reading && !reference_mode {
        if params.scanner.negative_conversion_method
            != crate::params::NegativeConversionMethod::Coupled
        {
            // `separable`, and `density` where the incoming image is a
            // capture: the capture converts to density per channel before
            // the inverse.
            let calibration = PositiveCalibration::from_profile(
                profile,
                params,
                backend,
                color_ref,
                channel_density,
                base_density,
                scan_context,
            )?;
            // The capture is consumed by the inverse, so move the buffer out
            // instead of cloning the whole scan image.
            let capture = std::mem::replace(rgb, ImageBuf::new(0, 0));
            *rgb = interpret_negative_capture(capture, &calibration, params)?;
        } else {
            let forward = ForwardCapture::prepare(
                profile,
                params,
                channel_density,
                base_density,
                scan_context,
                color_ref,
            );
            let calibration = prepare_exposure_calibration(
                profile,
                params,
                color_ref,
                scan_context,
                channel_density,
                base_density,
                &forward,
            )?;
            interpret_coupled_capture(rgb, &calibration, params)?;
        }
    }
    if negative_to_positive && params.scanner.finish == crate::params::ScannerFinish::LabScan {
        let _finish = super::StageObservation::cpu(
            backend,
            "scanner_finish",
            spektrafilm_backend::telemetry::CpuReason::NegativeToPositiveOutput,
        );
        finish::apply(
            rgb,
            output_space,
            params.scanner.finish_strength,
            !profile.is_bw(),
        );
    }
    Ok(())
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
    scanner_setup: Option<&crate::scanner_setup::ScannerSetup>,
) -> Result<ImageBuf, String> {
    let scan_params = if let Some(setup) = scanner_setup {
        let mut params = params.clone();
        setup.apply(&mut params);
        params
    } else {
        params.clone()
    };
    scan_with_options(
        density_cmy,
        profile,
        &scan_params,
        backend,
        color_ref,
        gamut,
        None,
        true,
    )
}

/// Runtime scan with immutable reference assets resolved during preparation.
pub(crate) fn process_prepared(
    density: &ImageBuf,
    profile: &Profile,
    params: &RuntimeParams,
    backend: &dyn ComputeBackend,
    color_ref: &crate::color_reference::ColorReference,
    gamut: &crate::gamut_compression::OutputGamutCompress,
    reference: Option<&crate::digital_reference::DigitalReference>,
    scanner_setup: Option<&crate::scanner_setup::ScannerSetup>,
) -> Result<ImageBuf, String> {
    let scan_params = if let Some(setup) = scanner_setup {
        let mut params = params.clone();
        setup.apply(&mut params);
        params
    } else {
        params.clone()
    };
    scan_with_reference(
        density,
        profile,
        &scan_params,
        backend,
        color_ref,
        gamut,
        None,
        true,
        None,
        reference,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::data_dir;

    fn calibration() -> PositiveCalibration {
        PositiveCalibration {
            channels: std::array::from_fn(|_| PositiveChannel {
                density: vec![0.0, 1.0, 2.0],
                log_exposure: vec![-1.0, 0.0, 1.0],
                clear: 1.0,
            }),
        }
    }

    fn density_calibration() -> interpretation::DensityCalibration {
        let data = data_dir();
        let profile = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        interpretation::DensityCalibration::prepare(&profile).unwrap()
    }

    fn interpret_density(density: &[f64], params: &RuntimeParams) -> Vec<f64> {
        let input = ImageBuf::from_data(
            (density.len() / 3) as u32,
            1,
            density.iter().copied().map(from_f64).collect(),
        );
        let backend = spektrafilm_backend::cpu_backend::CpuBackend;
        interpret_negative_density(&input, &density_calibration(), params, &backend)
            .unwrap()
            .data
            .iter()
            .map(|value| to_f64(*value))
            .collect()
    }

    #[test]
    fn denser_negative_reads_brighter_and_stays_finite_beyond_the_curves() {
        let params = RuntimeParams::default();
        let values = interpret_density(
            &[-100.0, 0.01, 1.0, 1.9, 100.0]
                .into_iter()
                .flat_map(|v| [v; 3])
                .collect::<Vec<_>>(),
            &params,
        );
        let values: Vec<f64> = values.into_iter().step_by(3).collect();
        assert!(values.windows(2).all(|w| w[1] > w[0]));
        assert!(
            values
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        );
    }

    #[test]
    fn each_output_channel_reads_only_its_own_record() {
        let params = RuntimeParams::default();
        let neutral = interpret_density(&[1.0, 1.0, 1.0], &params);
        let low_magenta = interpret_density(&[1.0, 0.2, 1.0], &params);
        assert_eq!(low_magenta[0], neutral[0]);
        assert!(low_magenta[1] < neutral[1]);
        assert_eq!(low_magenta[2], neutral[2]);
    }

    #[test]
    fn density_reading_ignores_the_receiver_and_the_capture() {
        let data = data_dir();
        let profile = crate::profile::load_profile_by_name(&data, "kodak_portra_400").unwrap();
        let density = ImageBuf::from_data(
            3,
            1,
            [0.2, 0.4, 0.6, 0.9, 0.9, 0.9, 1.5, 1.2, 1.1]
                .into_iter()
                .map(from_f64)
                .collect(),
        );
        let backend = spektrafilm_backend::cpu_backend::CpuBackend;
        let color_ref = crate::color_reference::ColorReference::identity();
        let gamut = crate::gamut_compression::OutputGamutCompress::identity();
        let mut params = RuntimeParams::default();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.negative_conversion_method =
            crate::params::NegativeConversionMethod::Density;
        params.scanner.receiver_profile = crate::params::ReceiverProfile::LegacyColorimetric;
        let legacy = scan_with_capture(
            &density, &profile, &params, &backend, &color_ref, &gamut, None, true, None,
        )
        .unwrap();
        params.scanner.receiver_profile = crate::params::ReceiverProfile::LabScanApprox;
        let lab_scan = scan_with_capture(
            &density, &profile, &params, &backend, &color_ref, &gamut, None, true, None,
        )
        .unwrap();
        assert_eq!(legacy.data, lab_scan.data);
        assert!(legacy.data.iter().all(|value| value.is_finite()));
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
        params.scanner.conversion_brightness_ev += 1.0;
        let brighter = interpret_negative_capture(input.clone(), &calibration(), &params).unwrap();
        assert!(brighter.data.iter().zip(&baseline.data).all(|(a, b)| a > b));
        params.scanner.conversion_brightness_ev -= 1.0;
        params.scanner.conversion_balance_ev = [1.0, 0.0, -1.0];
        let balanced = interpret_negative_capture(input, &calibration(), &params).unwrap();
        assert!(balanced.data[0] > baseline.data[0]);
        assert_eq!(balanced.data[1], baseline.data[1]);
        assert!(balanced.data[2] < baseline.data[2]);
    }

    #[test]
    fn very_small_output_density_stays_finite_and_ordered() {
        let mut params = RuntimeParams::default();
        params.scanner.conversion_dmax = 1e-18;
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
        crate::profile::load_profile_by_name(&data_dir(), name).unwrap()
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
        let backend = spektrafilm_backend::cpu_backend::CpuBackend;
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
            None,
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
    fn default_slide_points_blur_acquisition_before_correction() {
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
        let backend = spektrafilm_backend::cpu_backend::CpuBackend;
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
            true,
        );
        let mut expected = backend.gaussian_blur(&historical_capture, params.scanner.lens_blur);
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
            None,
        )
        .unwrap();
        assert_eq!(actual.data, expected.data);
    }

    #[test]
    fn zero_scanner_blur_preserves_linear_capture() {
        let film = profile("kodak_portra_400");
        let mut params = RuntimeParams::default();
        params.io.scan_film = true;
        params.io.output_cctf_encoding = false;
        params.film_render.grain.active = false;
        params.scanner.lens_blur = 0.0;
        params.scanner.unsharp_mask = [0.0; 2];
        let backend = spektrafilm_backend::cpu_backend::CpuBackend;
        let correction = crate::color_reference::ColorReference::identity();
        let context = crate::chain_prep::PreparedChain::for_scan(&film, &params, None);
        let channel = crate::chain_prep::channel_density(&film);
        let input = ImageBuf::from_data(
            2,
            1,
            [0.1, 0.2, 0.3, 1.0, 1.2, 1.4]
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
            false,
        );
        let actual = process(
            &input,
            &film,
            &params,
            &backend,
            &correction,
            &crate::gamut_compression::OutputGamutCompress::identity(),
            None,
        )
        .unwrap();
        assert_eq!(actual.data, capture.data);
    }

    #[test]
    fn paper_and_direct_negative_ignore_stored_points() {
        let backend = spektrafilm_backend::cpu_backend::CpuBackend;
        let correction = crate::color_reference::ColorReference::identity();
        let gamut = crate::gamut_compression::OutputGamutCompress::identity();
        let input = ImageBuf::from_data(1, 1, vec![from_f64(0.5); 3]);
        for (name, scan_film) in [("kodak_portra_endura", false), ("kodak_portra_400", true)] {
            let profile = profile(name);
            let mut params = RuntimeParams::default();
            params.io.scan_film = scan_film;
            params.film_render.grain.active = false;
            let baseline = process(
                &input,
                &profile,
                &params,
                &backend,
                &correction,
                &gamut,
                None,
            )
            .unwrap();
            params.scanner.black_point = [0.2; 3];
            params.scanner.white_point = [0.8; 3];
            let active = process(
                &input,
                &profile,
                &params,
                &backend,
                &correction,
                &gamut,
                None,
            )
            .unwrap();
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
            &spektrafilm_backend::cpu_backend::CpuBackend,
            &crate::color_reference::ColorReference::identity(),
            &crate::gamut_compression::OutputGamutCompress::identity(),
            None,
        );
        assert!(
            result
                .unwrap_err()
                .contains("endpoint output or tangent is not representable")
        );
    }

    #[test]
    fn public_negative_to_positive_rejects_unrepresentable_window_reciprocal() {
        let film = profile("kodak_portra_400");
        let mut params = RuntimeParams::default();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
        params.film_render.grain.active = false;
        params.scanner.white_point = [1e-320; 3];
        let result = process(
            &ImageBuf::from_data(1, 1, vec![from_f64(0.5); 3]),
            &film,
            &params,
            &spektrafilm_backend::cpu_backend::CpuBackend,
            &crate::color_reference::ColorReference::identity(),
            &crate::gamut_compression::OutputGamutCompress::identity(),
            None,
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
