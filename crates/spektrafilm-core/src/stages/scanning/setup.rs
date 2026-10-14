//! Scanner timing measured in relative film log exposure, before raster geometry.
//!
//! This value owns only additions to scanner brightness/balance and a contrast
//! multiplier. Film response, receiver calibration, point maps and camera exposure
//! remain owned by their existing stages.

use crate::params::{RuntimeParams, ScannerParams};

const CONTRAST_MIN: f64 = 0.5;
const CONTRAST_MAX: f64 = 2.0;
const SHADOW_OUTPUT: f64 = 0.02;
const MID_OUTPUT: f64 = 0.18;
const HIGHLIGHT_OUTPUT: f64 = 0.90;

/// Immutable frame measurements, reusable across Preview, Scan and Export.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScannerSetup {
    /// Additive scanner brightness, in stops.
    pub density_ev: f64,
    /// Additive scanner channel balance, in stops, with zero sum.
    pub balance_ev: [f64; 3],
    /// Multiplier for the user's conversion contrast.
    pub contrast_scale: f64,
}

impl ScannerSetup {
    /// Neutral setup for routes or parameter revisions with auto setup off.
    pub const IDENTITY: Self = Self {
        density_ev: 0.0,
        balance_ev: [0.0; 3],
        contrast_scale: 1.0,
    };

    /// Measure complete finite RGB samples of relative base-10 log exposure.
    ///
    /// `calibration_range` is the shared inverse calibration interval in that
    /// same domain. `camera_ev` is the simulated camera compensation, or zero
    /// for a real negative capture. User tone controls remain separate from
    /// the derived corrections, so changing them does not cancel their effect.
    pub fn from_samples(
        samples: &[[f64; 3]],
        camera_ev: f64,
        calibration_range: [f64; 2],
        scanner: &ScannerParams,
    ) -> Result<Self, String> {
        let [start, end] = calibration_range;
        let calibration_span = (end - start) / std::f64::consts::LOG10_2;
        if !camera_ev.is_finite()
            || !start.is_finite()
            || !end.is_finite()
            || start >= end
            || !calibration_span.is_finite()
        {
            return Err(
                "scanner auto setup requires finite camera EV and calibration range".into(),
            );
        }
        if !scanner.conversion_brightness_ev.is_finite()
            || !scanner.conversion_balance_ev.iter().all(|v| v.is_finite())
            || !scanner.conversion_contrast.is_finite()
            || scanner.conversion_contrast <= 0.0
            || !scanner.conversion_dmax.is_finite()
            || scanner.conversion_dmax <= 0.0
        {
            return Err(
                "scanner auto setup requires finite negative conversion tone controls".into(),
            );
        }

        let to_ev = |sample: &[f64; 3]| sample.map(|v| v / std::f64::consts::LOG10_2 - camera_ev);
        let valid = |sample: &&[f64; 3]| to_ev(sample).iter().all(|v| v.is_finite());
        let count = samples.iter().filter(valid).count();
        if count == 0 {
            return Err("scanner auto setup has no finite full-frame RGB measurements".into());
        }
        // Reuse one sorting buffer for every channel and the balanced scene.
        let mut values = Vec::with_capacity(count);
        let mut medians = [0.0; 3];
        for (channel, median) in medians.iter_mut().enumerate() {
            values.clear();
            values.extend(samples.iter().filter(valid).map(|s| to_ev(s)[channel]));
            values.sort_unstable_by(f64::total_cmp);
            *median = quantile(&values, 0.5);
        }
        let neutral_median = medians[0] / 3.0 + medians[1] / 3.0 + medians[2] / 3.0;
        let mut balance_ev = medians.map(|v| neutral_median - v);
        // Make the balancing correction zero-sum, including roundoff.
        balance_ev[2] = -(balance_ev[0] + balance_ev[1]);
        values.clear();
        values.extend(samples.iter().filter(valid).map(|sample| {
            let ev = to_ev(sample);
            (ev[0] + balance_ev[0]) / 3.0
                + (ev[1] + balance_ev[1]) / 3.0
                + (ev[2] + balance_ev[2]) / 3.0
        }));
        if !values.iter().all(|v| v.is_finite()) || !balance_ev.iter().all(|v| v.is_finite()) {
            return Err("scanner auto setup measurements exceed the finite exposure range".into());
        }
        values.sort_unstable_by(f64::total_cmp);
        let shadow = quantile(&values, 0.05);
        let median = quantile(&values, 0.5);
        let highlight = quantile(&values, 0.95);

        // Invert the *existing* positive density tone, rather than borrowing
        // linear-RGB stretch or the constants of a different film renderer.
        let reference = ScannerParams::default();
        let targets = [SHADOW_OUTPUT, MID_OUTPUT, HIGHLIGHT_OUTPUT]
            .map(|output| tone_coordinate(output, scanner.conversion_dmax));
        if !targets.iter().all(|v| v.is_finite()) {
            return Err(
                "scanner auto setup cannot represent the selected positive density tone".into(),
            );
        }
        let observed_span = highlight - shadow;
        let flat = observed_span <= calibration_span * 1.0e-6;
        let contrast_scale = if flat {
            1.0
        } else {
            ((targets[2] - targets[0]) / reference.conversion_contrast / observed_span)
                .clamp(CONTRAST_MIN, CONTRAST_MAX)
        };
        let contrast = reference.conversion_contrast * contrast_scale;
        let placement = |target: f64, measured: f64| {
            target / contrast - reference.conversion_brightness_ev - measured
        };
        // Median drives exposure; robust tails gently bias placement. Flat
        // scenes carry no usable contrast information, so use only the median.
        let density_ev = if flat {
            placement(targets[1], median)
        } else {
            0.15 * placement(targets[0], shadow)
                + 0.70 * placement(targets[1], median)
                + 0.15 * placement(targets[2], highlight)
        };
        if !density_ev.is_finite()
            || !contrast_scale.is_finite()
            || !observed_span.is_finite()
            || !(scanner.conversion_contrast * contrast_scale).is_finite()
            || !(scanner.conversion_brightness_ev + density_ev).is_finite()
            || !(0..3).all(|c| (scanner.conversion_balance_ev[c] + balance_ev[c]).is_finite())
        {
            return Err("scanner auto setup cannot represent the measured tone corrections".into());
        }
        Ok(Self {
            density_ev,
            balance_ev,
            contrast_scale,
        })
    }

    /// Apply once to a render-local clone, leaving the original recipe intact.
    pub fn apply(&self, params: &mut RuntimeParams) {
        params.scanner.conversion_brightness_ev += self.density_ev;
        for channel in 0..3 {
            params.scanner.conversion_balance_ev[channel] += self.balance_ev[channel];
        }
        params.scanner.conversion_contrast *= self.contrast_scale;
    }
}

fn quantile(sorted: &[f64], fraction: f64) -> f64 {
    let position = fraction * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    let weight = position - lower as f64;
    sorted[lower] * (1.0 - weight) + sorted[upper] * weight
}

/// Logistic coordinate `contrast * exposure_ev` for the density tone output.
fn tone_coordinate(output: f64, dmax: f64) -> f64 {
    let denominator = -(-std::f64::consts::LN_10 * dmax).exp_m1();
    let density = -(-denominator * (1.0 - output)).ln_1p() / std::f64::consts::LN_10;
    let t = 1.0 - density / dmax;
    (t / (1.0 - t)).ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(samples: &[[f64; 3]]) -> ScannerSetup {
        ScannerSetup::from_samples(samples, 0.0, [-2.0, 2.0], &ScannerParams::default()).unwrap()
    }

    fn ramp() -> Vec<[f64; 3]> {
        (0..101).map(|i| [(i as f64 - 50.0) * 0.02; 3]).collect()
    }

    #[test]
    fn small_positive_density_places_flat_frame_at_mid_output() {
        let mut params = RuntimeParams::default();
        params.scanner.conversion_dmax = 1e-18;
        let samples = [[0.4; 3]; 16];
        let measured =
            ScannerSetup::from_samples(&samples, 0.0, [-2.0, 2.0], &params.scanner).unwrap();
        measured.apply(&mut params);
        let exposure = 0.4 / std::f64::consts::LOG10_2 + params.scanner.conversion_brightness_ev;
        let coordinate = params.scanner.conversion_contrast * exposure;
        let t = 1.0 / (1.0 + (-coordinate).exp());
        let dmax = params.scanner.conversion_dmax;
        let denominator = -(-std::f64::consts::LN_10 * dmax).exp_m1();
        let positive = (-std::f64::consts::LN_10 * dmax * (1.0 - t)).exp_m1();
        let output = (positive + denominator) / denominator;
        assert!((output - MID_OUTPUT).abs() < 1e-12);
        assert_eq!(measured.contrast_scale, 1.0);
        assert_eq!(measured.balance_ev, [0.0; 3]);
    }

    #[test]
    fn dim_and_bright_frames_shift_only_scanner_brightness() {
        let samples = ramp();
        let original = setup(&samples);
        for shift_ev in [-3.0, 3.0] {
            let shifted: Vec<_> = samples
                .iter()
                .map(|s| s.map(|v| v + shift_ev * std::f64::consts::LOG10_2))
                .collect();
            let measured = setup(&shifted);
            assert!((measured.density_ev - original.density_ev + shift_ev).abs() < 1e-12);
            assert!((measured.contrast_scale - original.contrast_scale).abs() < 1e-12);
            assert_eq!(measured.balance_ev, original.balance_ev);
        }
    }

    #[test]
    fn isolated_extreme_pixels_do_not_drive_frame_timing() {
        let mut samples = ramp();
        let original = setup(&samples);
        samples.extend([[-1000.0; 3], [1000.0; 3]]);
        let measured = setup(&samples);
        assert!((measured.density_ev - original.density_ev).abs() < 0.05);
        assert!((measured.contrast_scale - original.contrast_scale).abs() < 0.05);
        assert_eq!(measured.balance_ev, original.balance_ev);
    }

    #[test]
    fn channel_cast_is_balanced_without_changing_mean_exposure() {
        let samples: Vec<_> = ramp()
            .iter()
            .map(|s| [s[0] + 0.3, s[1] - 0.1, s[2] - 0.2])
            .collect();
        let measured = setup(&samples);
        let expected = [-0.3, 0.1, 0.2].map(|v| v / std::f64::consts::LOG10_2);
        for (c, expected) in expected.iter().enumerate() {
            assert!((measured.balance_ev[c] - expected).abs() < 1e-12);
        }
        assert_eq!(measured.balance_ev.iter().sum::<f64>(), 0.0);
        assert!((measured.density_ev - setup(&ramp()).density_ev).abs() < 1e-12);
    }

    #[test]
    fn flat_and_nearly_flat_frames_keep_finite_bounded_contrast() {
        for samples in [vec![[0.4; 3]; 100], vec![[0.4; 3], [0.4 + 1e-12; 3]]] {
            let measured = setup(&samples);
            assert!(measured.density_ev.is_finite());
            assert!(measured.balance_ev.iter().all(|v| v.is_finite()));
            assert_eq!(measured.contrast_scale, 1.0);
        }
    }

    #[test]
    fn measurements_are_independent_of_sample_order_and_ignore_invalid_rows() {
        let mut samples = ramp();
        samples.extend([[f64::NAN, 0.0, 0.0], [0.0, f64::INFINITY, 0.0]]);
        let forward = setup(&samples);
        samples.reverse();
        assert_eq!(forward, setup(&samples));
        assert_eq!(forward, setup(&ramp()));
        assert!(
            ScannerSetup::from_samples(&[], 0.0, [-2.0, 2.0], &ScannerParams::default()).is_err()
        );
        assert!(
            ScannerSetup::from_samples(
                &[[f64::NAN; 3]],
                0.0,
                [-2.0, 2.0],
                &ScannerParams::default()
            )
            .is_err()
        );
    }

    #[test]
    fn simulated_camera_compensation_is_only_a_measurement_coordinate() {
        let samples = ramp();
        let original = setup(&samples);
        let camera_ev = 2.0;
        let exposed: Vec<_> = samples
            .iter()
            .map(|s| s.map(|v| v + camera_ev * std::f64::consts::LOG10_2))
            .collect();
        let measured =
            ScannerSetup::from_samples(&exposed, camera_ev, [-2.0, 2.0], &ScannerParams::default())
                .unwrap();
        assert!((original.density_ev - measured.density_ev).abs() < 1e-12);
        assert!((original.contrast_scale - measured.contrast_scale).abs() < 1e-12);
    }

    #[test]
    fn apply_preserves_user_adjustments_and_has_no_saturation_or_point_correction() {
        let original = RuntimeParams::default();
        let measured = setup(&ramp());
        let mut adjusted = original.clone();
        adjusted.scanner.conversion_brightness_ev += 0.7;
        adjusted.scanner.conversion_balance_ev = [0.1, -0.3, 0.2];
        adjusted.scanner.conversion_contrast *= 1.2;
        let before = adjusted.clone();
        measured.apply(&mut adjusted);
        assert_eq!(
            adjusted.scanner.conversion_brightness_ev,
            before.scanner.conversion_brightness_ev + measured.density_ev
        );
        assert_eq!(
            adjusted.scanner.conversion_contrast,
            before.scanner.conversion_contrast * measured.contrast_scale
        );
        assert_eq!(adjusted.scanner.black_point, before.scanner.black_point);
        assert_eq!(adjusted.scanner.white_point, before.scanner.white_point);
        assert_eq!(
            adjusted.scanner.conversion_dmax,
            before.scanner.conversion_dmax
        );
        assert_eq!(
            adjusted.camera.exposure_compensation_ev,
            before.camera.exposure_compensation_ev
        );
        for c in 0..3 {
            assert_eq!(
                adjusted.scanner.conversion_balance_ev[c],
                before.scanner.conversion_balance_ev[c] + measured.balance_ev[c]
            );
        }
        // No cross-channel mixer or content-derived saturation operation is
        // represented: pixels of equal luminance only enter neutral timing.
        let neutral = vec![[0.0; 3]; 100];
        let chromatic: Vec<_> = (0..100)
            .map(|i| {
                let a = if i % 2 == 0 { 0.2 } else { -0.2 };
                [a, -a, 0.0]
            })
            .collect();
        assert_eq!(setup(&neutral), setup(&chromatic));
        assert_eq!(
            original.scanner.conversion_brightness_ev,
            ScannerParams::default().conversion_brightness_ev
        );
    }
}
