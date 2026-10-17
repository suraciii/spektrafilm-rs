//! Per-sample roundtrip metrics and per-group summaries.

use std::collections::BTreeMap;

use serde::Serialize;

/// ECDF thresholds reported per group (the frozen evaluation's grid).
pub const ECDF_THRESHOLDS: [f64; 4] = [0.035, 0.07, 0.105, 0.28];

/// One sample's relative roundtrip error. All fields are `NaN` when invalid.
#[derive(Clone, Copy, Debug)]
pub struct SampleMetrics {
    pub valid: bool,
    /// `max_c |response/truth − 1|`, dimensionless.
    pub rho: f64,
    /// `max_c |log2(response/truth)|`, EV.
    pub max_abs_ev: f64,
    /// Mean of `log2(response/truth)` over channels, EV.
    pub common_ev: f64,
    /// RMS over channels of `log2(response/truth) − common_ev`, EV.
    pub balance_rms_ev: f64,
    /// `log2(response/truth)` per channel, EV.
    pub delta_ev_rgb: [f64; 3],
    pub rb_ev: f64,
    pub gb_ev: f64,
}

impl SampleMetrics {
    /// An invalid sample: the response violated the evaluation rules.
    pub fn invalid() -> Self {
        Self {
            valid: false,
            rho: f64::NAN,
            max_abs_ev: f64::NAN,
            common_ev: f64::NAN,
            balance_rms_ev: f64::NAN,
            delta_ev_rgb: [f64::NAN; 3],
            rb_ev: f64::NAN,
            gb_ev: f64::NAN,
        }
    }

    /// The frozen evaluation's validity rule: every channel needs a positive
    /// truth and a positive, finite relative response.
    pub fn from_relative(truth: [f64; 3], response: [f64; 3]) -> Self {
        let valid = (0..3).all(|c| truth[c] > 0.0 && response[c] > 0.0 && response[c].is_finite());
        if !valid {
            return Self::invalid();
        }
        let ev = [
            (response[0] / truth[0]).log2(),
            (response[1] / truth[1]).log2(),
            (response[2] / truth[2]).log2(),
        ];
        let common = (ev[0] + ev[1] + ev[2]) / 3.0;
        let balance_rms =
            (((ev[0] - common).powi(2) + (ev[1] - common).powi(2) + (ev[2] - common).powi(2))
                / 3.0)
                .sqrt();
        let rho = (0..3)
            .map(|c| (response[c] / truth[c] - 1.0).abs())
            .fold(0.0f64, f64::max);
        let max_abs_ev = ev
            .iter()
            .fold(0.0f64, |worst, value| worst.max(value.abs()));
        Self {
            valid: true,
            rho,
            max_abs_ev,
            common_ev: common,
            balance_rms_ev: balance_rms,
            delta_ev_rgb: ev,
            rb_ev: ev[0] - ev[2],
            gb_ev: ev[1] - ev[2],
        }
    }
}

#[derive(Serialize)]
pub struct MetricTriple {
    pub median: f64,
    pub p90: f64,
    pub max: f64,
}

#[derive(Serialize)]
pub struct CommonEv {
    pub mean: f64,
    pub rmse: f64,
    pub p90_abs: f64,
}

#[derive(Serialize)]
pub struct Balance {
    pub rms_global: f64,
    pub p90_per_sample_rms: f64,
}

#[derive(Serialize)]
pub struct SignedEv {
    pub mean: f64,
    pub rmse: f64,
    pub p90_abs: f64,
}

/// Group-level summary over the group's samples. Invalid samples count in
/// `count` and in the ECDF denominator; statistics use the valid rows only.
#[derive(Serialize)]
pub struct GroupSummary {
    pub count: usize,
    pub valid: usize,
    pub invalid: usize,
    pub rho: Option<MetricTriple>,
    pub max_abs_ev: Option<MetricTriple>,
    pub ev_bias_rgb: Option<[f64; 3]>,
    pub ev_rmse_rgb: Option<[f64; 3]>,
    pub common_ev: Option<CommonEv>,
    pub balance: Option<Balance>,
    pub rb_ev: Option<SignedEv>,
    pub gb_ev: Option<SignedEv>,
    pub rho_ecdf: Option<BTreeMap<String, f64>>,
}

/// Summarize one group's samples (including invalid rows).
pub fn summarize(metrics: &[SampleMetrics]) -> GroupSummary {
    let count = metrics.len();
    let valid: Vec<&SampleMetrics> = metrics.iter().filter(|m| m.valid).collect();
    let valid_count = valid.len();
    let invalid = count - valid_count;
    if valid.is_empty() {
        return GroupSummary {
            count,
            valid: valid_count,
            invalid,
            rho: None,
            max_abs_ev: None,
            ev_bias_rgb: None,
            ev_rmse_rgb: None,
            common_ev: None,
            balance: None,
            rb_ev: None,
            gb_ev: None,
            rho_ecdf: None,
        };
    }

    let n = valid_count as f64;
    let rho_values: Vec<f64> = valid.iter().map(|m| m.rho).collect();
    let worst_values: Vec<f64> = valid.iter().map(|m| m.max_abs_ev).collect();
    let common_values: Vec<f64> = valid.iter().map(|m| m.common_ev).collect();
    let balance_values: Vec<f64> = valid.iter().map(|m| m.balance_rms_ev).collect();
    let rb_values: Vec<f64> = valid.iter().map(|m| m.rb_ev).collect();
    let gb_values: Vec<f64> = valid.iter().map(|m| m.gb_ev).collect();

    let mut bias = [0.0f64; 3];
    let mut squares = [0.0f64; 3];
    for m in &valid {
        for c in 0..3 {
            bias[c] += m.delta_ev_rgb[c];
            squares[c] += m.delta_ev_rgb[c] * m.delta_ev_rgb[c];
        }
    }
    let ev_bias_rgb = bias.map(|value| value / n);
    let ev_rmse_rgb = squares.map(|value| (value / n).sqrt());

    let rho_ecdf = ECDF_THRESHOLDS
        .iter()
        .map(|threshold| {
            let within = rho_values
                .iter()
                .filter(|value| **value <= *threshold)
                .count();
            (format!("{threshold}"), within as f64 / count as f64)
        })
        .collect();

    GroupSummary {
        count,
        valid: valid_count,
        invalid,
        rho: Some(MetricTriple {
            median: quantile(&rho_values, 0.5),
            p90: quantile(&rho_values, 0.9),
            max: rho_values.iter().fold(f64::NEG_INFINITY, |m, v| m.max(*v)),
        }),
        max_abs_ev: Some(MetricTriple {
            median: quantile(&worst_values, 0.5),
            p90: quantile(&worst_values, 0.9),
            max: worst_values
                .iter()
                .fold(f64::NEG_INFINITY, |m, v| m.max(*v)),
        }),
        ev_bias_rgb: Some(ev_bias_rgb),
        ev_rmse_rgb: Some(ev_rmse_rgb),
        common_ev: Some(CommonEv {
            mean: mean(&common_values),
            rmse: rms(&common_values),
            p90_abs: quantile_abs(&common_values, 0.9),
        }),
        balance: Some(Balance {
            rms_global: rms(&balance_values),
            p90_per_sample_rms: quantile(&balance_values, 0.9),
        }),
        rb_ev: Some(SignedEv {
            mean: mean(&rb_values),
            rmse: rms(&rb_values),
            p90_abs: quantile_abs(&rb_values, 0.9),
        }),
        gb_ev: Some(SignedEv {
            mean: mean(&gb_values),
            rmse: rms(&gb_values),
            p90_abs: quantile_abs(&gb_values, 0.9),
        }),
        rho_ecdf: Some(rho_ecdf),
    }
}

/// The shared interpolated percentile estimator, on a sorted copy. The
/// evaluation library owns the estimator; this module reports values, it does
/// not restate the convention.
fn quantile(values: &[f64], q: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    spektrafilm_benchmark::metrics::quantile(&sorted, q).unwrap_or(f64::NAN)
}

fn quantile_abs(values: &[f64], q: f64) -> f64 {
    let absolute: Vec<f64> = values.iter().map(|value| value.abs()).collect();
    quantile(&absolute, q)
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn rms(values: &[f64]) -> f64 {
    (values.iter().map(|value| value * value).sum::<f64>() / values.len() as f64).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_metrics_follow_the_frozen_decomposition() {
        let truth = [0.5, 0.25, 1.0];
        let response = [0.5 * 2.0, 0.25 * 2.0, 1.0 * 2.0];
        let m = SampleMetrics::from_relative(truth, response);
        assert!(m.valid);
        assert!((m.rho - 1.0).abs() < 1e-15);
        assert!((m.common_ev - 1.0).abs() < 1e-15);
        assert!(m.balance_rms_ev.abs() < 1e-15);
        assert!(m.rb_ev.abs() < 1e-15);
        assert!((m.max_abs_ev - 1.0).abs() < 1e-15);
    }

    #[test]
    fn non_positive_response_is_invalid() {
        let m = SampleMetrics::from_relative([1.0, 1.0, 1.0], [1.0, -0.1, 1.0]);
        assert!(!m.valid);
        let m = SampleMetrics::from_relative([1.0, 1.0, 1.0], [1.0, f64::NAN, 1.0]);
        assert!(!m.valid);
        let m = SampleMetrics::from_relative([1.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        assert!(!m.valid);
    }

    #[test]
    fn quantile_matches_numpy_linear() {
        assert!((quantile(&[1.0, 2.0, 3.0, 4.0], 0.5) - 2.5).abs() < 1e-15);
        assert!((quantile(&[1.0, 2.0, 3.0, 4.0], 0.9) - 3.7).abs() < 1e-15);
        assert_eq!(quantile(&[5.0], 0.9), 5.0);
    }

    #[test]
    fn ecdf_denominator_includes_invalid_rows() {
        let rows = vec![
            SampleMetrics::from_relative([1.0, 1.0, 1.0], [1.0, 1.0, 1.0]),
            SampleMetrics::invalid(),
        ];
        let summary = summarize(&rows);
        assert_eq!(summary.count, 2);
        assert_eq!(summary.valid, 1);
        let ecdf = summary.rho_ecdf.expect("valid rows exist");
        assert_eq!(ecdf["0.035"], 0.5);
    }
}
