use super::developed_density_curves;
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
            return Err("density inverse requires finite increasing film exposure knots and matching density samples".into());
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
                "density inverse requires distinct film densities in channel {channel}"
            ));
        }
        if density.windows(2).zip(exposure.windows(2)).any(|(d, e)| {
            let slope = (e[1] - e[0]) / (d[1] - d[0]);
            !slope.is_finite() || slope <= 0.0
        }) {
            return Err(format!(
                "density inverse has an unrepresentable film endpoint slope in channel {channel}"
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

    #[test]
    fn density_inverse_prepares_monotone_axes_from_the_profile() {
        let film = crate::profile::load_profile_by_name(
            &crate::test_support::data_dir(),
            "kodak_portra_400",
        )
        .unwrap();
        let calibration = DensityCalibration::prepare(&film).unwrap();
        for density in [0.1, 0.5, 1.0] {
            let exposure = calibration.estimate([density; 3]).unwrap();
            assert!(exposure.iter().all(|value| value.is_finite()));
        }
        assert!(DensityCalibration::estimate(&calibration, [f64::NAN; 3]).is_err());
    }
}
