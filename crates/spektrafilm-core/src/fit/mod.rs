//! Profile fitting: derive profile and look parameter values from reference
//! patch data.
//!
//! The fit renders a chart through the calibrated pipeline and minimizes the
//! CIELAB difference between the rendered patches and a reference patch table.
//! See `specs/profile-fit/`.

pub mod chart;
mod color;
mod evaluate;
mod optimizer;

pub use chart::{ChartLayout, IndexRange, PatchTable};
pub use color::PatchDifferences;

use crate::params::RuntimeParams;
use crate::profile::Profile;
use evaluate::Evaluator;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A parameter group a fit owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FitGroup {
    /// The leading terms of the film profile's `hanatos2025_adaptation_surface_params`.
    Surface,
    /// The negative-conversion `positive_density_ev` and `positive_balance_ev`.
    Tone,
}

#[derive(Debug, Clone)]
pub struct FitOptions {
    pub groups: Vec<FitGroup>,
    pub surface_terms: usize,
    pub lambda: f64,
    pub max_iters: usize,
    /// Cross-validate the color patches with the even and odd folds.
    pub cv: bool,
}

impl Default for FitOptions {
    fn default() -> Self {
        Self {
            groups: vec![FitGroup::Surface],
            surface_terms: 9,
            lambda: 1.0,
            max_iters: 15,
            cv: false,
        }
    }
}

/// One fit run: the resolved chain, the chart data, and the fit options.
pub struct FitRequest {
    pub params: RuntimeParams,
    pub film: Profile,
    pub print: Profile,
    pub data_dir: PathBuf,
    pub layout: ChartLayout,
    pub scene: PatchTable,
    pub reference: PatchTable,
    pub options: FitOptions,
}

/// The effective settings a fit forces on every evaluation.
#[derive(Debug, Clone, Serialize)]
pub struct FitForcedSettings {
    pub halation_active: bool,
    pub grain_active: bool,
    /// The scan output domain; always false, because the reference is linear.
    pub output_cctf_encoding: bool,
    /// The scan output color space; always sRGB, the primaries of the reference.
    pub output_color_space: &'static str,
    /// The surface toggle the fit enabled; absent when the surface is not fitted.
    pub surface_toggle: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToneFit {
    pub positive_density_ev: f64,
    pub positive_balance_ev: [f64; 3],
}

/// The redundancy a tone fit pins, so that a fitted tone is unique.
///
/// A common offset added to `positive_density_ev` and subtracted from every
/// `positive_balance_ev` channel leaves the negative conversion unchanged. The fit
/// pins `positive_balance_ev[pinned_channel]` to `value` and searches the rest.
#[derive(Debug, Clone, Serialize)]
pub struct ToneGauge {
    pub pinned_channel: usize,
    pub value: f64,
}

/// One cross-validation fold: the same fit, scored on its own half of the
/// color patches and measured on the held-out half.
#[derive(Debug, Clone, Serialize)]
pub struct CvFold {
    /// The color patches this fold held out, in layout order.
    pub held_out: Vec<usize>,
    /// Median difference over the fold's training color patches.
    pub train_de_median: f64,
    /// Median difference over the held-out color patches.
    pub test_de_median: f64,
    /// Median difference of the unfitted configuration over the held-out
    /// color patches.
    pub test_base_de_median: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FitReport {
    pub groups: Vec<FitGroup>,
    pub surface_terms: usize,
    pub lambda: f64,
    pub iterations: usize,
    pub evaluations: usize,
    pub base_color_de_median: f64,
    pub color_de_median: f64,
    pub base_gray_de_median: f64,
    pub gray_de_median: f64,
    pub differences: PatchDifferences,
    /// The 3x15 surface the fitted profile stores; absent without the surface group.
    pub surface: Option<Vec<Vec<f64>>>,
    /// The fitted tone values; absent without the tone group.
    pub tone: Option<ToneFit>,
    /// The pinned tone redundancy; absent without the tone group.
    pub tone_gauge: Option<ToneGauge>,
    /// The cross-validation folds; absent without `cv`.
    pub cv: Option<Vec<CvFold>>,
    pub forced: FitForcedSettings,
}

/// One solved fit: the unfitted and fitted evaluations and the solver effort.
struct FitRun {
    base: evaluate::Evaluation,
    fitted: evaluate::Evaluation,
    x: Vec<f64>,
    iterations: usize,
    evaluations: usize,
}

/// Minimize one evaluator's objective and require an improvement over the
/// unfitted configuration.
fn solve_fit(evaluator: &Evaluator, options: &FitOptions) -> Result<FitRun, String> {
    let initial = evaluator.initial();
    let base = evaluator.evaluate(&initial)?;
    let objective = |x: &[f64]| evaluator.evaluate(x).map(|evaluation| evaluation.residual);
    let solution = optimizer::solve(&optimizer::Problem {
        initial,
        lower: evaluator.lower(),
        upper: evaluator.upper(),
        steps: evaluator.steps(),
        max_iters: options.max_iters,
        objective: &objective,
    })?;
    let fitted = evaluator.evaluate(&solution.x)?;
    let base_cost: f64 = base.residual.iter().map(|value| value * value).sum();
    // A strict improvement, with a non-finite cost treated as no improvement.
    if solution.cost.partial_cmp(&base_cost) != Some(std::cmp::Ordering::Less) {
        return Err(format!(
            "fit did not improve the objective: base cost {base_cost}, final cost {}",
            solution.cost
        ));
    }
    Ok(FitRun {
        base,
        fitted,
        x: solution.x,
        iterations: solution.iterations,
        evaluations: solution.evaluations + 2,
    })
}

/// A training set and its held-out counterpart.
type FoldSplit = (Vec<usize>, Vec<usize>);

/// Split the color patches into the even and odd folds. Every fold trains on
/// one position parity and is measured on the other.
fn fold_splits(color: &[usize]) -> Result<Vec<FoldSplit>, String> {
    if color.len() < 4 {
        return Err(format!(
            "cross validation needs at least four color patches, got {}",
            color.len()
        ));
    }
    let half = |first| {
        color
            .iter()
            .copied()
            .skip(first)
            .step_by(2)
            .collect::<Vec<_>>()
    };
    Ok(vec![(half(0), half(1)), (half(1), half(0))])
}

/// Fit every fold on its own half of the color patches.
fn cross_validate(evaluator: &Evaluator, options: &FitOptions) -> Result<Vec<CvFold>, String> {
    fold_splits(evaluator.color_indices())?
        .into_iter()
        .map(|(train, held_out)| {
            let fold = evaluator.scoring_color(train);
            let run = solve_fit(&fold, options)?;
            Ok(CvFold {
                train_de_median: run.fitted.color_de,
                test_de_median: fold.delta_median(&run.fitted.labs, &held_out),
                test_base_de_median: fold.delta_median(&run.base.labs, &held_out),
                held_out,
            })
        })
        .collect()
}

/// Fit the requested parameter groups and report the measured result.
pub fn fit(request: FitRequest) -> Result<FitReport, String> {
    let FitRequest {
        params,
        film,
        print,
        data_dir,
        layout,
        scene,
        reference,
        options,
    } = request;
    if options.groups.is_empty() {
        return Err("fit requires at least one parameter group".into());
    }
    let surface = options.groups.contains(&FitGroup::Surface);
    let tone = options.groups.contains(&FitGroup::Tone);
    let evaluator = Evaluator::new(
        film,
        print,
        params,
        data_dir,
        layout,
        &scene,
        &reference,
        surface,
        tone,
        options.surface_terms,
        options.lambda,
    )?;

    let run = solve_fit(&evaluator, &options)?;
    // A lower objective can trade one metric for the other; the fit only
    // publishes values that reduce both.
    for (metric, base, fitted) in [
        ("color", run.base.color_de, run.fitted.color_de),
        ("gray", run.base.gray_de, run.fitted.gray_de),
    ] {
        if fitted.partial_cmp(&base) != Some(std::cmp::Ordering::Less) {
            return Err(format!(
                "fit did not reduce the {metric} ΔE00 median: unfitted {base:.3}, fitted {fitted:.3}"
            ));
        }
    }
    let cv = if options.cv {
        Some(cross_validate(&evaluator, &options)?)
    } else {
        None
    };

    let surface_parameters = if surface {
        3 * options.surface_terms
    } else {
        0
    };
    let differences = evaluator.differences(&run.fitted.labs);
    Ok(FitReport {
        groups: options.groups,
        surface_terms: options.surface_terms,
        lambda: options.lambda,
        iterations: run.iterations,
        evaluations: run.evaluations,
        base_color_de_median: run.base.color_de,
        color_de_median: run.fitted.color_de,
        base_gray_de_median: run.base.gray_de,
        gray_de_median: run.fitted.gray_de,
        differences,
        surface: surface.then(|| evaluator.surface_matrix(&run.x[..surface_parameters])),
        tone: tone.then(|| {
            let (positive_density_ev, positive_balance_ev) = evaluator.tone_values(&run.x);
            ToneFit {
                positive_density_ev,
                positive_balance_ev,
            }
        }),
        tone_gauge: tone.then(|| {
            let (pinned_channel, value) = evaluator.tone_pin();
            ToneGauge {
                pinned_channel,
                value,
            }
        }),
        cv,
        forced: FitForcedSettings {
            halation_active: false,
            grain_active: false,
            output_cctf_encoding: false,
            output_color_space: evaluate::FIT_OUTPUT_COLOR_SPACE,
            surface_toggle: surface.then_some(true),
        },
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_split_the_color_patches_by_position() {
        // Positions, not absolute indices: the odd starting index stays in the
        // even fold.
        let color = vec![5, 9, 14, 21, 33, 40];
        let folds = fold_splits(&color).expect("folds");
        assert_eq!(folds.len(), 2);
        assert_eq!(folds[0].0, vec![5, 14, 33]);
        assert_eq!(folds[0].1, vec![9, 21, 40]);
        assert_eq!(folds[1].0, folds[0].1);
        assert_eq!(folds[1].1, folds[0].0);
        for (train, held_out) in &folds {
            assert_eq!(train.len() + held_out.len(), color.len());
            assert!(train.iter().all(|index| !held_out.contains(index)));
        }
    }

    #[test]
    fn folds_need_four_color_patches() {
        let error = fold_splits(&[0, 1, 2]).expect_err("too few patches");
        assert!(error.contains("at least four"), "{error}");
    }
}
