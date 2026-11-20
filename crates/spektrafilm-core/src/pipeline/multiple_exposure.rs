//! Multiple-exposure composition at the film log-exposure boundary.
//!
//! A real multiple exposure adds light on one emulsion and develops once. The
//! film chain exposes each shot into per-record `log10` exposure (`log_e_film`)
//! and applies the density curve only in `develop`; composing at that boundary
//! therefore adds exposure before the nonlinear density curve, DIR couplers and
//! grain run exactly once.
//!
//! This is a proof-of-concept core API. It deliberately keeps the public
//! surface small: each shot carries only its physical exposure offset; all
//! shots share the pipeline's film/print/route parameters and working geometry,
//! and each shot gets its own auto-exposure decision when
//! `camera.auto_exposure` is enabled.

use super::*;
use spektrafilm_backend::{DeviceImage, Precision, RetainedImageMeta, SampleDomain};
use spektrafilm_math::colorspace;

#[cfg(feature = "precision-f64")]
type AccumulatorSample = f64;
#[cfg(not(feature = "precision-f64"))]
type AccumulatorSample = f32;

/// Metering policy for a multiple-exposure stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotMetering {
    /// Meter every shot independently on its full input.
    Each,
    /// Meter the first shot and reuse that AE value for the stack.
    LockToFirst,
    /// Do not auto-meter; apply `exposure_ev` to the input as exposed.
    Manual,
}

/// One exposure in a multiple-exposure stack.
#[derive(Debug, Clone, Copy)]
pub struct ExposureShot<'a> {
    /// Source image, before input decoding, crop and resize.
    pub image: &'a ImageBuf,
    /// Physical per-shot exposure offset in stops, applied inside `expose`
    /// after AE and before optical effects. Negative = underexpose.
    pub exposure_ev: f64,
    /// Metering policy for this stack. All shots must agree.
    pub metering: ShotMetering,
}

/// One RGB multiple-exposure shot. The shot is decoded, offset, and passed
/// through the active pre-LUT effects before linear-light composition.
#[derive(Debug, Clone, Copy)]
pub struct RgbExposureShot<'a> {
    pub image: &'a ImageBuf,
    pub exposure_ev: f64,
}

/// A streaming multiple-exposure stack. Create it with
/// [`Pipeline::begin_exposures`], add one shot at a time, then call `finish`.
///
/// CPU stacks keep one accumulator (`f64` in the `precision-f64` reference
/// build, `f32` in the default product build). GPU stacks keep one device-side
/// accumulator; the caller can drop each host image immediately after
/// `add_shot`. This is the product streaming path; the slice APIs below are
/// thin wrappers over it.
pub struct ExposureAccumulator<'a> {
    inner: AccumulatorKind<'a>,
}

enum AccumulatorKind<'a> {
    Cpu(CpuAccumulator<'a>),
    Gpu(GpuAccumulator<'a>),
}

impl<'a> ExposureAccumulator<'a> {
    /// Add one shot. The image is borrowed only for this call; the caller may
    /// drop it before loading the next shot.
    pub fn add_shot(&mut self, image: &ImageBuf, exposure_ev: f64) -> Result<(), String> {
        match &mut self.inner {
            AccumulatorKind::Cpu(stack) => stack.add_shot(image, exposure_ev),
            AccumulatorKind::Gpu(stack) => stack.add_shot(image, exposure_ev),
        }
    }

    /// Finalize the stack and run develop/print/scan once.
    pub fn finish(self) -> Result<(ImageBuf, MultipleExposureReport), String> {
        match self.inner {
            AccumulatorKind::Cpu(stack) => stack.finish(),
            AccumulatorKind::Gpu(stack) => stack.finish(),
        }
    }
}

struct CpuAccumulator<'a> {
    pipeline: &'a Pipeline,
    backend: &'a dyn ComputeBackend,
    metering: ShotMetering,
    collect: Tap,
    width: u32,
    height: u32,
    dims_set: bool,
    pitch: Option<f64>,
    reports: Vec<ShotReport>,
    locked_ae: Option<f64>,
    shift: Option<f64>,
    values: Vec<AccumulatorSample>,
}

struct GpuAccumulator<'a> {
    pipeline: &'a Pipeline,
    backend: &'a dyn ComputeBackend,
    metering: ShotMetering,
    collect: Tap,
    width: u32,
    height: u32,
    dims_set: bool,
    reports: Vec<ShotReport>,
    locked_ae: Option<f64>,
    accumulator: Option<DeviceImage>,
}

fn validate_stack(pipeline: &Pipeline, collect: Tap) -> Result<(), String> {
    let route = pipeline.params.workflow.route_enum();
    if !route.exposes_film() {
        return Err("multiple exposure requires a film-forming route".into());
    }
    if route.artifact_input().is_some() || route.artifact_output().is_some() {
        return Err(
            "multiple exposure is an image-entry route; stage artifacts are not inputs".into(),
        );
    }
    if pipeline.params.taps.inject.is_some() || pipeline.params.taps.collect.is_some() {
        return Err(
            "multiple exposure owns the tap boundaries; clear taps.inject and taps.collect".into(),
        );
    }
    let order = pipeline.tap_order();
    if !order.contains(&collect) {
        return Err(format!(
            "tap {collect:?} is not part of this topology (scan_film = {})",
            pipeline.params.io.scan_film
        ));
    }
    if collect.position() < Tap::LogEFilm.position() {
        return Err("multiple exposure collects at log_e_film or later".into());
    }
    if collect == Tap::RgbOut && pipeline.params.output_mix.amount < 1.0 {
        return Err(
            "output_mix.amount below 1 requires rgb_in injection; apply output mix after multiple exposure"
                .into(),
        );
    }
    if pipeline.params.camera.exposure_compensation_ev != 0.0 {
        return Err(
            "multiple exposure requires camera.exposure_compensation_ev=0; use per-shot exposure_ev"
                .into(),
        );
    }
    Ok(())
}

fn check_shot(
    width: &mut u32,
    height: &mut u32,
    dims_set: &mut bool,
    image: &ImageBuf,
    exposure_ev: f64,
) -> Result<(), String> {
    if image.width == 0 || image.height == 0 {
        return Err("multiple exposure requires non-empty images".into());
    }
    if !*dims_set {
        *width = image.width;
        *height = image.height;
        *dims_set = true;
    } else if image.width != *width || image.height != *height {
        return Err(format!(
            "multiple exposure shot is {}x{}, expected {}x{}",
            image.width, image.height, *width, *height
        ));
    }
    if !exposure_ev.is_finite() || !(-10.0..=10.0).contains(&exposure_ev) {
        return Err("multiple exposure exposure_ev must be finite and in [-10, 10]".into());
    }
    Ok(())
}

fn report_for(
    pipeline: &Pipeline,
    width: u32,
    height: u32,
    reports: &[ShotReport],
    combined: &ImageBuf,
) -> MultipleExposureReport {
    let log_e_min = combined
        .data
        .iter()
        .map(|&value| to_f64(value))
        .fold(f64::INFINITY, f64::min);
    let log_e_max = combined
        .data
        .iter()
        .map(|&value| to_f64(value))
        .fold(f64::NEG_INFINITY, f64::max);
    let total_exposure: f64 = reports.iter().map(|shot| 2.0_f64.powf(shot.final_ev)).sum();
    let highlight_log_e = film_shoulder_log_e(&pipeline.film);
    let mut highlight_pixels = 0usize;
    for pixel in combined.data.as_chunks::<3>().0 {
        let peak = pixel
            .iter()
            .map(|&value| to_f64(value))
            .fold(f64::NEG_INFINITY, f64::max);
        if peak > highlight_log_e {
            highlight_pixels += 1;
        }
    }
    let total_pixels = (width as usize) * (height as usize);
    MultipleExposureReport {
        shots: reports.to_vec(),
        total_ev: total_exposure.log2(),
        log_e_range: (log_e_min, log_e_max),
        highlight_log_e,
        highlight_fraction: if total_pixels == 0 {
            0.0
        } else {
            highlight_pixels as f64 / total_pixels as f64
        },
    }
}

impl CpuAccumulator<'_> {
    fn add_shot(&mut self, image: &ImageBuf, exposure_ev: f64) -> Result<(), String> {
        check_shot(
            &mut self.width,
            &mut self.height,
            &mut self.dims_set,
            image,
            exposure_ev,
        )?;
        let index = self.reports.len();
        if self.values.is_empty() {
            self.values =
                vec![0.0 as AccumulatorSample; (self.width as usize) * (self.height as usize) * 3];
        }
        let mut shot_pipeline = self.pipeline.clone();
        shot_pipeline.params.camera.exposure_compensation_ev = exposure_ev as f32;
        let ae_ev = match self.metering {
            ShotMetering::Each => shot_pipeline.meter_autoexposure(image, self.backend)?,
            ShotMetering::LockToFirst => {
                if self.locked_ae.is_none() {
                    self.locked_ae = Some(self.pipeline.meter_autoexposure(image, self.backend)?);
                }
                self.locked_ae.expect("locked above")
            }
            ShotMetering::Manual => {
                shot_pipeline.params.camera.auto_exposure = false;
                0.0
            }
        };
        let ae_applies = shot_pipeline.params.camera.auto_exposure;
        let final_ev = if ae_applies {
            ae_ev + exposure_ev
        } else {
            exposure_ev
        };
        let (working, shot_pitch) = shot_pipeline.rescale_working(image, self.backend)?;
        if let Some(expected) = self.pitch {
            let scale = expected.abs().max(1.0);
            if (expected - shot_pitch).abs() > 1e-9 * scale {
                return Err(format!(
                    "multiple exposure shot {index} resolves pitch {shot_pitch} µm, expected {expected} µm; use identical crop/upscale settings"
                ));
            }
        } else {
            self.pitch = Some(shot_pitch);
        }
        let log_e = shot_pipeline.run_from(
            working.into_owned(),
            Tap::RgbPre,
            Tap::LogEFilm,
            self.backend,
            Some((shot_pitch, ae_ev)),
            None,
            None,
            None,
        )?;
        let shot_max = log_e
            .data
            .iter()
            .map(|&value| to_f64(value))
            .fold(f64::NEG_INFINITY, f64::max);
        let effective_shift = match self.shift {
            None => {
                self.shift = Some(shot_max);
                shot_max
            }
            Some(current) if shot_max > current => {
                let delta = 10.0_f64.powf(current - shot_max);
                self.values
                    .iter_mut()
                    .for_each(|value| *value = (*value as f64 * delta) as AccumulatorSample);
                self.shift = Some(shot_max);
                shot_max
            }
            Some(current) => current,
        };
        for (acc, &value) in self.values.iter_mut().zip(&log_e.data) {
            *acc += (10.0_f64.powf(to_f64(value) - effective_shift)) as AccumulatorSample;
        }
        self.reports.push(ShotReport {
            index,
            metered_ev: ae_ev,
            exposure_ev,
            final_ev,
        });
        Ok(())
    }

    fn finish(self) -> Result<(ImageBuf, MultipleExposureReport), String> {
        if !self.dims_set || self.reports.is_empty() {
            return Err("multiple exposure requires at least one shot".into());
        }
        let shift = self.shift.ok_or("multiple exposure has no logE shift")?;
        let mut combined = ImageBuf::from_data(
            self.width,
            self.height,
            vec![from_f64(0.0); self.values.len()],
        );
        for (slot, &sum) in combined.data.iter_mut().zip(&self.values) {
            *slot = from_f64(shift + (sum as f64).log10());
        }
        let report = report_for(
            self.pipeline,
            self.width,
            self.height,
            &self.reports,
            &combined,
        );
        if self.collect == Tap::LogEFilm {
            return Ok((combined, report));
        }
        let pitch = self.pitch.ok_or("multiple exposure has no pitch")?;
        let mut final_pipeline = self.pipeline.clone();
        final_pipeline.params.camera.exposure_compensation_ev = 0.0;
        let image = final_pipeline.run_from(
            combined,
            Tap::LogEFilm,
            self.collect,
            self.backend,
            Some((pitch, 0.0)),
            None,
            None,
            None,
        )?;
        Ok((image, report))
    }
}

impl GpuAccumulator<'_> {
    fn add_shot(&mut self, image: &ImageBuf, exposure_ev: f64) -> Result<(), String> {
        check_shot(
            &mut self.width,
            &mut self.height,
            &mut self.dims_set,
            image,
            exposure_ev,
        )?;
        let index = self.reports.len();
        let input_space =
            spektrafilm_math::colorspace::resolve(&self.pipeline.params.io.input_color_space)?;
        let carrier = self.backend.retain_image(
            image,
            &RetainedImageMeta {
                width: image.width,
                height: image.height,
                domain: if self.pipeline.params.io.input_cctf_decoding {
                    SampleDomain::Encoded
                } else {
                    SampleDomain::Linear
                },
                precision: Precision::host(),
                space: input_space,
                pixel_pitch_um: None,
            },
        )?;
        let ae_ev = if !self.pipeline.params.camera.auto_exposure {
            None
        } else {
            match self.metering {
                ShotMetering::Each => Some(self.pipeline.meter_retained(&carrier, self.backend)?),
                ShotMetering::LockToFirst => {
                    if self.locked_ae.is_none() {
                        self.locked_ae =
                            Some(self.pipeline.meter_retained(&carrier, self.backend)?);
                    }
                    self.locked_ae
                }
                ShotMetering::Manual => None,
            }
        };
        let mut shot_pipeline = self.pipeline.clone();
        shot_pipeline.params.camera.auto_exposure = false;
        shot_pipeline.params.camera.exposure_compensation_ev =
            (ae_ev.unwrap_or(0.0) + exposure_ev) as f32;
        let retained = shot_pipeline.process_device_retained_with_taps(
            &carrier,
            RetainedRender::FULL,
            self.backend,
            Some(Tap::RgbIn),
            Some(Tap::LogEFilm),
        )?;
        self.accumulator = Some(
            self.backend
                .log_e_accumulate(self.accumulator.as_ref(), &retained)?
                .ok_or_else(|| {
                    "selected GPU has no device logE composition; use --backend cpu".to_owned()
                })?,
        );
        let metered_ev = ae_ev.unwrap_or(0.0);
        let final_ev = if ae_ev.is_some() {
            metered_ev + exposure_ev
        } else {
            exposure_ev
        };
        self.reports.push(ShotReport {
            index,
            metered_ev,
            exposure_ev,
            final_ev,
        });
        Ok(())
    }

    fn finish(self) -> Result<(ImageBuf, MultipleExposureReport), String> {
        if !self.dims_set || self.reports.is_empty() {
            return Err("multiple exposure requires at least one shot".into());
        }
        let accumulator = self
            .accumulator
            .ok_or("multiple exposure has no GPU accumulator")?;
        let composite_device = self
            .backend
            .log_e_finalize(&accumulator)?
            .ok_or_else(|| "selected GPU cannot finalize logE composition".to_owned())?;
        let composite = composite_device.to_host(self.backend)?;
        let report = report_for(
            self.pipeline,
            self.width,
            self.height,
            &self.reports,
            &composite,
        );
        if self.collect == Tap::LogEFilm {
            return Ok((composite, report));
        }
        let rendered = self.pipeline.process_device_retained_with_taps(
            &composite_device,
            RetainedRender::FULL,
            self.backend,
            Some(Tap::LogEFilm),
            Some(self.collect),
        )?;
        let image = rendered.to_host(self.backend)?;
        Ok((image, report))
    }
}

/// Diagnostics for one shot in the stack.
#[derive(Debug, Clone)]
pub struct ShotReport {
    pub index: usize,
    pub metered_ev: f64,
    pub exposure_ev: f64,
    /// Final relative exposure: `metered_ev + exposure_ev` when AE is active,
    /// otherwise `exposure_ev`.
    pub final_ev: f64,
}

/// Diagnostics for the composite stack.
#[derive(Debug, Clone)]
pub struct MultipleExposureReport {
    pub shots: Vec<ShotReport>,
    /// Relative total exposure under the equal-luminance assumption.
    pub total_ev: f64,
    /// Observed composite log10 exposure range.
    pub log_e_range: (f64, f64),
    /// Profile-derived shoulder reference (log10 exposure).
    pub highlight_log_e: f64,
    /// Fraction of composite pixels whose brightest record exceeds the
    /// shoulder reference.
    pub highlight_fraction: f64,
}

impl Pipeline {
    /// Begin a streaming multiple-exposure stack. The backend is bound here
    /// so CPU and GPU state cannot be mixed.
    pub fn begin_exposures<'a>(
        &'a self,
        backend: &'a dyn ComputeBackend,
        metering: ShotMetering,
        collect: Tap,
    ) -> Result<ExposureAccumulator<'a>, String> {
        validate_stack(self, collect)?;
        if backend.is_gpu() {
            Ok(ExposureAccumulator {
                inner: AccumulatorKind::Gpu(GpuAccumulator {
                    pipeline: self,
                    backend,
                    metering,
                    collect,
                    width: 0,
                    height: 0,
                    dims_set: false,
                    reports: Vec::new(),
                    locked_ae: None,
                    accumulator: None,
                }),
            })
        } else {
            Ok(ExposureAccumulator {
                inner: AccumulatorKind::Cpu(CpuAccumulator {
                    pipeline: self,
                    backend,
                    metering,
                    collect,
                    width: 0,
                    height: 0,
                    dims_set: false,
                    pitch: None,
                    reports: Vec::new(),
                    locked_ae: None,
                    shift: None,
                    values: Vec::new(),
                }),
            })
        }
    }

    /// Compose N exposures in the film log-exposure domain, then run the
    /// remaining chain once.
    ///
    /// This is a thin wrapper over [`Pipeline::begin_exposures`]. Each shot
    /// runs its own `expose` (input decoding, spectral RGB→raw, auto-exposure,
    /// camera compensation, diffusion, lens blur, light leak, halation) and is
    /// collected at [`Tap::LogEFilm`]. The collected log10 exposures are added
    /// as linear exposure:
    ///
    /// `L_c = log10(sum_i 10^(L_i))`
    ///
    /// using a per-pixel max shift for numerical stability.
    pub fn process_exposures(
        &self,
        shots: &[ExposureShot<'_>],
        backend: &dyn ComputeBackend,
        collect: Tap,
    ) -> Result<ImageBuf, String> {
        self.process_exposures_with_report(shots, backend, collect)
            .map(|(image, _report)| image)
    }

    /// Compose N exposures and return per-shot diagnostics alongside the image.
    pub fn process_exposures_with_report(
        &self,
        shots: &[ExposureShot<'_>],
        backend: &dyn ComputeBackend,
        collect: Tap,
    ) -> Result<(ImageBuf, MultipleExposureReport), String> {
        if shots.is_empty() {
            return Err("multiple exposure requires at least one shot".into());
        }
        let metering = shots[0].metering;
        if shots.iter().any(|shot| shot.metering != metering) {
            return Err("all multiple-exposure shots must share one metering mode".into());
        }
        let mut stack = self.begin_exposures(backend, metering, collect)?;
        for shot in shots {
            stack.add_shot(shot.image, shot.exposure_ev)?;
        }
        stack.finish()
    }

    /// Compose an RGB stack in linear light, then run the active LUT,
    /// LUT Grade, gamut, output encoding, and Grain once.
    ///
    /// Each shot is decoded, offset, clamped to nonnegative light, and passed
    /// through the active pre-LUT effects. The input is already a raster; the
    /// caller owns the GUI/CLI source-loading and preview resize policy.
    pub fn process_rgb_exposures(
        &self,
        shots: &[RgbExposureShot<'_>],
        backend: &dyn ComputeBackend,
    ) -> Result<ImageBuf, String> {
        if self.params.workflow.mode.is_film() {
            return Err("RGB multiple exposure requires the LUT or Off workflow".into());
        }
        if shots.len() < 2 || shots.len() > 8 {
            return Err("RGB multiple exposure requires two to eight shots".into());
        }
        if let Some(effects) = self.params.effects.as_ref() {
            effects.validate()?;
        }
        let look = if self.params.workflow.mode.is_lut() {
            self.lut_look
                .as_deref()
                .ok_or_else(|| "LUT mode has no resolved LUT look".to_owned())?
        } else {
            self.off_look
                .as_deref()
                .ok_or_else(|| "Off mode has no resolved Effects look".to_owned())?
        };
        let effects_look = look.effects_only();
        let final_look = look.look_only();
        let linear_srgb = crate::lut_look::LutSpace::SrgbLinear.color_space();
        let source_space = colorspace::resolve(&self.params.io.input_color_space)?;
        let output_space = colorspace::resolve(&self.params.io.output_color_space)?;
        let output_encoded = self.params.io.output_cctf_encoding;
        let matrix = colorspace::conversion_matrix(source_space, &linear_srgb);
        let gamut_off = crate::gamut_compression::OutputGamutCompress::build(
            &crate::params::OutputGamutCompressParams {
                algorithm: "off".into(),
                ..Default::default()
            },
            "sRGB",
        )?;
        let mut composite: Option<ImageBuf> = None;
        let mut pitch = 0.0;
        for (index, shot) in shots.iter().enumerate() {
            if !shot.exposure_ev.is_finite() || !(-10.0..=10.0).contains(&shot.exposure_ev) {
                return Err(format!(
                    "RGB multiple exposure shot {index}: exposure_ev must be finite and in [-10, 10]"
                ));
            }
            let (working, shot_pitch) =
                crate::resizing::crop_and_rescale(shot.image, &self.params.io, 35.0)?;
            if index == 0 {
                pitch = shot_pitch;
            } else if (pitch - shot_pitch).abs() > f64::EPSILON {
                return Err(format!(
                    "RGB multiple exposure shot {index}: shots do not share one working pitch"
                ));
            }
            let mut linear = working.into_owned();
            for pixel in linear.data.as_chunks_mut::<3>().0 {
                let rgb = [to_f64(pixel[0]), to_f64(pixel[1]), to_f64(pixel[2])];
                let converted = colorspace::convert_rgb(
                    rgb,
                    source_space,
                    self.params.io.input_cctf_decoding,
                    &linear_srgb,
                    false,
                    &matrix,
                );
                let gain = shot.exposure_ev.exp2();
                for channel in 0..3 {
                    pixel[channel] = from_f64((converted[channel] * gain).max(0.0));
                }
            }
            let affected = effects_look.render(
                &linear,
                &linear_srgb,
                false,
                &linear_srgb,
                false,
                pitch,
                &gamut_off,
                backend,
            )?;
            match &mut composite {
                Some(sum) => {
                    if sum.width != affected.width || sum.height != affected.height {
                        return Err(format!(
                            "RGB multiple exposure shot {index}: shots do not share working geometry"
                        ));
                    }
                    for (sum, value) in sum.data.iter_mut().zip(affected.data) {
                        *sum += value;
                    }
                }
                None => composite = Some(affected),
            }
        }
        let composite =
            composite.ok_or_else(|| "RGB multiple exposure produced no composition".to_owned())?;
        final_look.render(
            &composite,
            &linear_srgb,
            false,
            output_space,
            output_encoded,
            pitch,
            &self.output_gamut,
            backend,
        )
    }

    /// GPU path: expose each shot to retained `log_e_film`, compose on the
    /// device, then run the remaining chain once.
    ///
    /// All metering modes are supported: the retained meter value is folded
    /// into the per-shot camera compensation before the retained expose, so
    /// auto-exposure is not run twice.
    pub fn process_exposures_gpu(
        &self,
        shots: &[ExposureShot<'_>],
        backend: &dyn ComputeBackend,
        collect: Tap,
    ) -> Result<(ImageBuf, MultipleExposureReport), String> {
        if !backend.is_gpu() {
            return Err("process_exposures_gpu requires a GPU backend".into());
        }
        self.process_exposures_with_report(shots, backend, collect)
    }
}

/// Estimate the film shoulder from the profile's density curves.
///
/// For each record, find the log exposure where density reaches 90% of its
/// range (the crossing nearest the high-exposure end), then take the most
/// conservative (smallest) shoulder across records. Reversal materials fall
/// with exposure; the same high-end crossing works for both polarities.
fn film_shoulder_log_e(film: &crate::profile::Profile) -> f64 {
    const SHOULDER_FRACTION: f64 = 0.9;
    let log_e = &film.data.log_exposure;
    let curves = &film.data.density_curves;
    if log_e.len() < 2 || curves.len() != log_e.len() {
        return f64::INFINITY;
    }
    let mut shoulder = f64::INFINITY;
    for channel in 0..3 {
        let values: Vec<f64> = curves
            .iter()
            .map(|row| row.get(channel).copied().unwrap_or(0.0))
            .collect();
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if !min.is_finite() || !max.is_finite() || max <= min {
            continue;
        }
        let threshold = min + SHOULDER_FRACTION * (max - min);
        for index in (1..values.len()).rev() {
            let (a, b) = (values[index - 1], values[index]);
            if (a - threshold) * (b - threshold) <= 0.0 && a != b {
                shoulder = shoulder.min(log_e[index]);
                break;
            }
        }
    }
    shoulder
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile;
    use crate::test_support::data_dir;
    use spektrafilm_backend::cpu_backend::CpuBackend;

    fn pipeline() -> Pipeline {
        let dir = data_dir();
        let film = profile::load_profile_by_name(&dir, "kodak_portra_400").unwrap();
        let print = profile::load_profile_by_name(&dir, "kodak_portra_endura").unwrap();
        let mut params = RuntimeParams::default();
        params.io.input_color_space = "sRGB".into();
        params.io.input_cctf_decoding = false;
        params.camera.auto_exposure = false;
        // 32 px over 0.32 mm keeps the working pitch at 10 µm, like the other
        // small-image fixtures, so halation/DIR kernels stay bounded.
        params.camera.film_format_mm = 0.32;
        params.film_render.grain.active = false;
        Pipeline::new_with_spectral(film, print, params, &dir).unwrap()
    }

    fn image(offset: f64) -> ImageBuf {
        let (w, h) = (32u32, 32u32);
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let ramp = (x + y * w) as f64 / (w * h) as f64;
                data.push(from_f64((0.02 + offset + 0.7 * ramp).clamp(0.001, 1.5)));
                data.push(from_f64((0.03 + offset + 0.5 * ramp).clamp(0.001, 1.5)));
                data.push(from_f64((0.04 + offset + 0.3 * ramp).clamp(0.001, 1.5)));
            }
        }
        ImageBuf::from_data(w, h, data)
    }

    fn max_abs(a: &ImageBuf, b: &ImageBuf) -> f64 {
        a.data
            .iter()
            .zip(&b.data)
            .map(|(&x, &y)| (to_f64(x) - to_f64(y)).abs())
            .fold(0.0, f64::max)
    }

    #[test]
    fn single_shot_matches_the_existing_tap_route() {
        let pipeline = pipeline();
        let source = image(0.0);
        let expected = pipeline
            .process_with_taps(
                source.clone(),
                &CpuBackend,
                Some(Tap::RgbIn),
                Some(Tap::CmyFilm),
            )
            .unwrap();
        let actual = pipeline
            .process_exposures(
                &[ExposureShot {
                    image: &source,
                    exposure_ev: 0.0,
                    metering: ShotMetering::Each,
                }],
                &CpuBackend,
                Tap::CmyFilm,
            )
            .unwrap();
        assert!(max_abs(&expected, &actual) < 1e-6);
    }

    #[test]
    fn two_minus_one_exposures_add_to_one_full_exposure() {
        let pipeline = pipeline();
        let source = image(0.1);
        let full = pipeline
            .process_with_taps(
                source.clone(),
                &CpuBackend,
                Some(Tap::RgbIn),
                Some(Tap::LogEFilm),
            )
            .unwrap();
        let half = ExposureShot {
            image: &source,
            exposure_ev: -1.0,
            metering: ShotMetering::Each,
        };
        let combined = pipeline
            .process_exposures(&[half, half], &CpuBackend, Tap::LogEFilm)
            .unwrap();
        for (i, (&full, &combined)) in full.data.iter().zip(&combined.data).enumerate() {
            let (full, combined) = (to_f64(full), to_f64(combined));
            // The -10 black floor is the only place where the 1e-10 epsilon
            // changes the sum; ignore those pixels.
            if full <= -8.0 {
                continue;
            }
            assert!(
                (full - combined).abs() < 1e-7,
                "pixel {i}: full {full}, combined {combined}"
            );
        }
    }

    #[test]
    fn bad_shots_fail() {
        let pipeline = pipeline();
        let source = image(0.0);
        assert!(
            pipeline
                .process_exposures(
                    &[ExposureShot {
                        image: &source,
                        exposure_ev: 11.0,
                        metering: ShotMetering::Each
                    }],
                    &CpuBackend,
                    Tap::CmyFilm
                )
                .is_err()
        );
        assert!(
            pipeline
                .process_exposures(
                    &[ExposureShot {
                        image: &source,
                        exposure_ev: f64::NAN,
                        metering: ShotMetering::Each
                    }],
                    &CpuBackend,
                    Tap::CmyFilm
                )
                .is_err()
        );
        assert!(
            pipeline
                .process_exposures(&[], &CpuBackend, Tap::CmyFilm)
                .is_err()
        );
    }

    #[test]
    fn mismatched_dimensions_and_early_collection_fail() {
        let pipeline = pipeline();
        let a = image(0.0);
        let b = ImageBuf::from_data(16, 16, vec![from_f64(0.2); 16 * 16 * 3]);
        assert!(
            pipeline
                .process_exposures(
                    &[
                        ExposureShot {
                            image: &a,
                            exposure_ev: 0.0,
                            metering: ShotMetering::Each
                        },
                        ExposureShot {
                            image: &b,
                            exposure_ev: 0.0,
                            metering: ShotMetering::Each
                        }
                    ],
                    &CpuBackend,
                    Tap::CmyFilm
                )
                .is_err()
        );
        assert!(
            pipeline
                .process_exposures(
                    &[ExposureShot {
                        image: &a,
                        exposure_ev: 0.0,
                        metering: ShotMetering::Each
                    }],
                    &CpuBackend,
                    Tap::RgbPre
                )
                .is_err()
        );
    }
    #[test]
    fn streaming_stack_matches_batch() {
        let pipeline = pipeline();
        let a = image(0.0);
        let b = image(0.1);
        let shots = [
            ExposureShot {
                image: &a,
                exposure_ev: -0.5,
                metering: ShotMetering::Manual,
            },
            ExposureShot {
                image: &b,
                exposure_ev: -1.5,
                metering: ShotMetering::Manual,
            },
        ];
        let batch = pipeline
            .process_exposures(&shots, &CpuBackend, Tap::CmyFilm)
            .unwrap();
        let mut stack = pipeline
            .begin_exposures(&CpuBackend, ShotMetering::Manual, Tap::CmyFilm)
            .unwrap();
        for shot in &shots {
            stack.add_shot(shot.image, shot.exposure_ev).unwrap();
        }
        let (streamed, _) = stack.finish().unwrap();
        assert_eq!(batch.width, streamed.width);
        assert_eq!(batch.height, streamed.height);
        assert_eq!(batch.data, streamed.data);
    }

    #[test]
    fn exposure_ev_is_applied_before_collection() {
        let pipeline = pipeline();
        let source = image(0.2);
        let base = ExposureShot {
            image: &source,
            exposure_ev: 0.0,
            metering: ShotMetering::Manual,
        };
        let minus_one = ExposureShot {
            exposure_ev: -1.0,
            ..base
        };
        let zero = pipeline
            .process_exposures(&[base], &CpuBackend, Tap::LogEFilm)
            .unwrap();
        let down = pipeline
            .process_exposures(&[minus_one], &CpuBackend, Tap::LogEFilm)
            .unwrap();
        let expected = 2.0_f64.log10();
        for (index, (&zero, &down)) in zero.data.iter().zip(&down.data).enumerate() {
            let (zero, down) = (to_f64(zero), to_f64(down));
            if zero <= -8.0 {
                continue;
            }
            assert!(
                ((zero - down) - expected).abs() < 1e-6,
                "pixel {index}: zero {zero}, down {down}, expected shift {expected}"
            );
        }
    }

    #[test]
    fn manual_metering_reports_ev_and_total() {
        let pipeline = pipeline();
        let source = image(0.0);
        let shot = ExposureShot {
            image: &source,
            exposure_ev: -1.0,
            metering: ShotMetering::Manual,
        };
        let (_, report) = pipeline
            .process_exposures_with_report(&[shot, shot], &CpuBackend, Tap::LogEFilm)
            .unwrap();
        assert_eq!(report.shots.len(), 2);
        for shot_report in &report.shots {
            assert_eq!(shot_report.metered_ev, 0.0);
            assert_eq!(shot_report.exposure_ev, -1.0);
            assert_eq!(shot_report.final_ev, -1.0);
        }
        assert!((report.total_ev - 0.0).abs() < 1e-12);
        assert!(report.log_e_range.0 <= report.log_e_range.1);
    }

    #[test]
    fn highlight_report_is_bounded() {
        let pipeline = pipeline();
        let source = image(0.0);
        let (_, report) = pipeline
            .process_exposures_with_report(
                &[ExposureShot {
                    image: &source,
                    exposure_ev: 0.0,
                    metering: ShotMetering::Manual,
                }],
                &CpuBackend,
                Tap::LogEFilm,
            )
            .unwrap();
        assert!(report.highlight_log_e.is_finite());
        assert!((0.0..=1.0).contains(&report.highlight_fraction));
    }

    #[test]
    fn mixed_metering_modes_fail() {
        let pipeline = pipeline();
        let source = image(0.0);
        assert!(
            pipeline
                .process_exposures(
                    &[
                        ExposureShot {
                            image: &source,
                            exposure_ev: 0.0,
                            metering: ShotMetering::Each,
                        },
                        ExposureShot {
                            image: &source,
                            exposure_ev: 0.0,
                            metering: ShotMetering::Manual,
                        },
                    ],
                    &CpuBackend,
                    Tap::CmyFilm,
                )
                .is_err()
        );
    }

    #[test]
    fn global_camera_compensation_must_be_zero() {
        let mut pipeline = pipeline();
        pipeline.params.camera.exposure_compensation_ev = -1.0;
        let source = image(0.0);
        assert!(
            pipeline
                .process_exposures(
                    &[ExposureShot {
                        image: &source,
                        exposure_ev: 0.0,
                        metering: ShotMetering::Manual,
                    }],
                    &CpuBackend,
                    Tap::CmyFilm,
                )
                .is_err()
        );
    }
}
