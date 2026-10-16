use super::{
    ForwardCapture,
    coupled::{CoupledBounds, CoupledInverse},
    developed_density_curves,
};
use crate::WorkflowRoute;
use crate::{params::RuntimeParams, profile::Profile};

struct DensityAxis {
    density: Vec<f64>,
    exposure: Vec<f64>,
}

impl DensityAxis {
    fn prepare(log_exposure: &[f64], curves: &[[f64; 3]], channel: usize) -> Result<Self, String> {
        if log_exposure.len() < 2
            || log_exposure.len() != curves.len()
            || log_exposure
                .windows(2)
                .any(|w| !w[0].is_finite() || !w[1].is_finite() || w[0] >= w[1])
        {
            return Err("coupled inverse requires finite increasing film exposure knots and matching density samples".into());
        }
        let mut blocks: Vec<(f64, f64, usize)> = Vec::new();
        for (&e, row) in log_exposure.iter().zip(curves) {
            let d = row[channel];
            if !d.is_finite() || !e.is_finite() {
                continue;
            }
            blocks.push((d, e, 1));
            while blocks.len() > 1 {
                let n = blocks.len();
                let a = blocks[n - 2];
                let b = blocks[n - 1];
                if a.0 / (a.2 as f64) < b.0 / (b.2 as f64) {
                    break;
                }
                blocks.pop();
                let a = blocks.last_mut().unwrap();
                a.0 += b.0;
                a.1 += b.1;
                a.2 += b.2;
            }
        }
        let mut density = Vec::new();
        let mut exposure = Vec::new();
        for (d, e, n) in blocks {
            let d = d / n as f64;
            if density.last().is_some_and(|last| d - *last <= 1e-10) {
                continue;
            }
            density.push(d);
            exposure.push(e / n as f64);
        }
        if density.len() < 2 {
            return Err(format!(
                "coupled inverse requires distinct film densities in channel {channel}"
            ));
        }
        if density.windows(2).zip(exposure.windows(2)).any(|(d, e)| {
            let slope = (e[1] - e[0]) / (d[1] - d[0]);
            !slope.is_finite() || slope <= 0.0
        }) {
            return Err(format!(
                "coupled inverse has an unrepresentable film endpoint slope in channel {channel}"
            ));
        }
        Ok(Self { density, exposure })
    }

    fn estimate(&self, density: f64) -> f64 {
        let upper = self
            .density
            .partition_point(|&v| v < density)
            .clamp(1, self.density.len() - 1);
        let lower = upper - 1;
        self.exposure[lower]
            + (density - self.density[lower]) / (self.density[upper] - self.density[lower])
                * (self.exposure[upper] - self.exposure[lower])
    }

    fn slope_at(&self, density: f64) -> f64 {
        let upper = self
            .density
            .partition_point(|&v| v < density)
            .clamp(1, self.density.len() - 1);
        (self.exposure[upper] - self.exposure[upper - 1])
            / (self.density[upper] - self.density[upper - 1])
    }
}

/// The three record axes, read from the developed curves: the film stage
/// emits densities over each record's minimum, so the inverse uses the same
/// origin.
fn prepare_axes(profile: &Profile) -> Result<[DensityAxis; 3], String> {
    let curves = developed_density_curves(profile);
    let log_exposure = &profile.data.log_exposure;
    (0..3)
        .map(|channel| DensityAxis::prepare(log_exposure, &curves, channel))
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| "three film axes required".into())
}

/// The coupled receiver inverse recovers dye densities; the film's monotone
/// characteristic curves then recover relative exposure without toe singularities.
pub(super) struct CoupledCalibration {
    inverse: CoupledInverse,
    axes: [DensityAxis; 3],
}

impl CoupledCalibration {
    pub(super) fn prepare(profile: &Profile, forward: &ForwardCapture<'_>) -> Result<Self, String> {
        let axes = prepare_axes(profile)?;
        let minimum = std::array::from_fn(|c| axes[c].density[0]);
        let maximum = std::array::from_fn(|c| *axes[c].density.last().unwrap());
        let inverse = CoupledInverse::prepare(
            CoupledBounds {
                domain_min: minimum,
                domain_max: maximum,
            },
            33,
            |density| forward.capture(density),
        )?;
        Ok(Self { inverse, axes })
    }

    pub(super) fn range(&self) -> [f64; 2] {
        [
            self.axes
                .iter()
                .map(|c| c.exposure[0])
                .fold(f64::NEG_INFINITY, f64::max),
            self.axes
                .iter()
                .map(|c| *c.exposure.last().unwrap())
                .fold(f64::INFINITY, f64::min),
        ]
    }

    pub(super) fn estimate(&self, capture: [f64; 3]) -> Result<[f64; 3], String> {
        self.estimate_with_anchor(capture).map(|result| result.0)
    }

    pub(super) fn estimate_with_anchor(
        &self,
        capture: [f64; 3],
    ) -> Result<([f64; 3], Option<[f64; 3]>), String> {
        let (density, anchor) = self.inverse.estimate_density_with_anchor(capture)?;
        let anchor_exposure = anchor.map(|d| std::array::from_fn(|c| self.axes[c].estimate(d[c])));
        let exposure = std::array::from_fn(|c| match (anchor, anchor_exposure) {
            (Some(d), Some(e)) => e[c] + self.axes[c].slope_at(d[c]) * (density[c] - d[c]),
            _ => self.axes[c].estimate(density[c]),
        });
        if exposure.iter().any(|v| !v.is_finite()) {
            return Err("coupled film inverse exceeds finite relative exposure range".into());
        }
        Ok((exposure, anchor_exposure))
    }
}

pub(super) fn camera_ev(params: &RuntimeParams) -> f64 {
    if params.workflow.route_enum() == WorkflowRoute::FilmScan {
        params.camera.exposure_compensation_ev as f64
    } else {
        0.0
    }
}

/// The record-density reading: the film's own monotone characteristic curves,
/// with no synthetic capture in the loop. A narrowband lab scanner reads each
/// dye record through its own band, so one record's estimate must not depend on
/// another record's density.
pub(super) struct DensityCalibration {
    axes: [DensityAxis; 3],
}

impl DensityCalibration {
    pub(super) fn prepare(profile: &Profile) -> Result<Self, String> {
        Ok(Self {
            axes: prepare_axes(profile)?,
        })
    }

    /// Relative log-exposure range that all three record inverses cover.
    pub(super) fn range(&self) -> [f64; 2] {
        [
            self.axes
                .iter()
                .map(|axis| axis.exposure[0])
                .fold(f64::NEG_INFINITY, f64::max),
            self.axes
                .iter()
                .map(|axis| *axis.exposure.last().unwrap())
                .fold(f64::INFINITY, f64::min),
        ]
    }

    pub(super) fn estimate(&self, density: [f64; 3]) -> Result<[f64; 3], String> {
        let exposure = std::array::from_fn(|channel| self.axes[channel].estimate(density[channel]));
        if exposure.iter().any(|value| !value.is_finite()) {
            return Err("density inverse exceeds finite relative exposure range".into());
        }
        Ok(exposure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::data_dir;

    fn profile(name: &str) -> Profile {
        crate::profile::load_profile_by_name(&data_dir(), name).unwrap()
    }
    #[test]
    fn coupled_real_receivers_recover_chromatic_density_and_exposure() {
        let film = profile("kodak_portra_400");
        let dyes = crate::chain_prep::channel_density(&film);
        let mut params = RuntimeParams::default();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.negative_conversion_method =
            crate::params::NegativeConversionMethod::Coupled;
        for receiver in [
            crate::params::ReceiverProfile::LegacyColorimetric,
            crate::params::ReceiverProfile::LabScanApprox,
        ] {
            params.scanner.receiver_profile = receiver;
            let prepared = crate::chain_prep::PreparedChain::for_scan(&film, &params, None);
            let correction = crate::color_reference::ColorReference::identity();
            let forward = ForwardCapture::prepare(
                &film,
                &params,
                &dyes,
                &film.data.base_density,
                &prepared.scan,
                &correction,
            );
            let calibration = CoupledCalibration::prepare(&film, &forward).unwrap();
            let mut probes = vec![[0.13, 0.39, 0.67], [0.51, 0.27, 0.74], [0.82, 0.64, 0.43]];
            for f in [0.0, 0.001, 0.02, 0.1, 0.5, 0.9, 0.98, 0.999, 1.0] {
                probes.push([f; 3]);
            }
            for bits in 0..8 {
                probes.push(std::array::from_fn(|c| ((bits >> c) & 1) as f64));
            }
            let mut worst_density = 0.0f64;
            let mut worst_exposure = 0.0f64;
            for fraction in probes {
                let density = std::array::from_fn(|c| {
                    let knots = &calibration.axes[c].density;
                    knots[0] + fraction[c] * (knots.last().unwrap() - knots[0])
                });
                let recovered = calibration
                    .inverse
                    .estimate_density(forward.capture(density))
                    .unwrap();
                let error = (0..3)
                    .map(|c| (recovered[c] - density[c]).abs())
                    .fold(0.0, f64::max);
                worst_density = worst_density.max(error);
                assert!(
                    error < 0.015,
                    "{receiver:?}: density {density:?} recovered {recovered:?}, error={error}"
                );
                let expected =
                    std::array::from_fn::<_, 3, _>(|c| calibration.axes[c].estimate(density[c]));
                let exposure = calibration.estimate(forward.capture(density)).unwrap();
                worst_exposure = worst_exposure.max(
                    (0..3)
                        .map(|c| (exposure[c] - expected[c]).abs())
                        .fold(0.0, f64::max),
                );
                assert!(
                    (0..3).all(|c| (exposure[c] - expected[c]).abs() < 0.08),
                    "{receiver:?}: expected {expected:?}, recovered {exposure:?}"
                );
            }
            eprintln!(
                "coupled numerical evidence {receiver:?}: density_error={worst_density}, log_exposure_error={worst_exposure}"
            );
        }
    }

    #[test]
    fn public_coupled_scan_preserves_endpoint_order_and_reports_invalid_capture() {
        let film = profile("kodak_portra_400");
        let dyes = crate::chain_prep::channel_density(&film);
        let mut params = RuntimeParams::default();
        params.io.scan_film = true;
        params.scanner.scan_output = "negative_to_positive".into();
        params.scanner.negative_conversion_method =
            crate::params::NegativeConversionMethod::Coupled;
        params.scanner.unsharp_mask = [0.0; 2];
        params.io.output_cctf_encoding = false;
        params.settings.use_scanner_lut = false;
        for receiver in [
            crate::params::ReceiverProfile::LegacyColorimetric,
            crate::params::ReceiverProfile::LabScanApprox,
        ] {
            params.scanner.receiver_profile = receiver;
            let prepared = crate::chain_prep::PreparedChain::for_scan(&film, &params, None);
            let correction = crate::color_reference::ColorReference::identity();
            let forward = ForwardCapture::prepare(
                &film,
                &params,
                &dyes,
                &film.data.base_density,
                &prepared.scan,
                &correction,
            );
            let calibration = CoupledCalibration::prepare(&film, &forward).unwrap();
            let count = calibration.axes[0].exposure.len();
            let samples: Vec<_> = [0, 1, count / 2, count - 2, count - 1]
                .into_iter()
                .map(|i| {
                    let e = calibration.axes[0].exposure[i];
                    std::array::from_fn::<_, 3, _>(|c| {
                        let axis = &calibration.axes[c];
                        let upper = axis
                            .exposure
                            .partition_point(|v| *v < e)
                            .clamp(1, axis.exposure.len() - 1);
                        let weight = (e - axis.exposure[upper - 1])
                            / (axis.exposure[upper] - axis.exposure[upper - 1]);
                        axis.density[upper - 1]
                            + weight * (axis.density[upper] - axis.density[upper - 1])
                    })
                })
                .collect();
            let image = spektrafilm_math::image::ImageBuf::from_data(
                samples.len() as u32,
                1,
                samples
                    .iter()
                    .flatten()
                    .map(|v| spektrafilm_math::precision::from_f64(*v))
                    .collect(),
            );
            let actual = super::super::process(
                &image,
                &film,
                &params,
                &spektrafilm_backend::cpu_backend::CpuBackend,
                &correction,
                &crate::gamut_compression::OutputGamutCompress::identity(),
            )
            .unwrap();
            for channel in 0..3 {
                let values: Vec<_> = actual
                    .data
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|p| spektrafilm_math::precision::to_f64(p[channel]))
                    .collect();
                assert!(values.iter().all(|v| v.is_finite()));
                assert!(
                    values.windows(2).all(|w| w[0] < w[1]),
                    "{receiver:?}: {values:?}"
                );
            }
            let exposure_calibration = super::super::ExposureCalibration::Coupled(calibration);
            for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                let mut capture = spektrafilm_math::image::ImageBuf::from_data(
                    1,
                    1,
                    vec![spektrafilm_math::precision::from_f64(invalid); 3],
                );
                assert!(
                    super::super::interpret_coupled_capture(
                        &mut capture,
                        &exposure_calibration,
                        &params
                    )
                    .is_err()
                );
            }
        }
    }
}
